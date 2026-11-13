//! Administration API behind the administrator console. Each handler names the permission it
//! needs; `system_admin` holds all of them, `app_admin` and `auditor` hold subsets (see `authz.rs`).

use crate::auth::{self, Ctx, TokenBody};
use crate::authz;
use crate::error::{bad, AppError, AppResult};
use crate::http::{Api, Paging};
use crate::packs::{self, WorkflowDef};
use crate::accounts::{self, valid_email};
use crate::{audit, oidc, saml, settings, St};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post, put};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::{Postgres, QueryBuilder, Row};
use uuid::Uuid;

// ───────────────────────── overview and health ─────────────────────────

async fn overview(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.access")?;
    let counts: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object(
            'users', (SELECT count(*) FROM users WHERE kind = 'human' AND active),
            'agents', (SELECT count(*) FROM users WHERE kind = 'agent' AND active),
            'projects', (SELECT count(*) FROM projects WHERE NOT archived),
            'spaces', (SELECT count(*) FROM spaces WHERE NOT archived),
            'items', (SELECT count(*) FROM items WHERE NOT archived),
            'pages', (SELECT count(*) FROM pages WHERE NOT archived),
            'sessions', (SELECT count(*) FROM sessions WHERE expires_at > now()),
            'jobs_queued', (SELECT count(*) FROM jobs WHERE state = 'queued'),
            'jobs_failed', (SELECT count(*) FROM jobs WHERE state = 'failed'),
            'security_events_24h', (SELECT count(*) FROM security_events WHERE at > now() - interval '24 hours'),
            'failed_logins_24h', (SELECT count(*) FROM security_events WHERE kind = 'auth.login_failed' AND at > now() - interval '24 hours'),
            'database_bytes', pg_database_size(current_database()))",
    )
    .fetch_one(&st.db)
    .await?;
    let check = |r: anyhow::Result<()>| match r {
        Ok(()) => json!({"ok": true}),
        Err(e) => json!({"ok": false, "error": e.to_string()}),
    };
    let pg_version: String = sqlx::query_scalar("SHOW server_version").fetch_one(&st.db).await?;
    let components = json!([
        {"name": "Database", "backend": format!("PostgreSQL {pg_version}"), "status": {"ok": true}},
        {"name": "Schema", "backend": "migrations", "status": if crate::schema_is_current(&st.db).await { json!({"ok": true}) } else { json!({"ok": false, "error": "out of date: run gantry migrate"}) }},
        {"name": "Files", "backend": st.stores.blob.name(), "status": check(st.stores.blob.health().await)},
        {"name": "Search", "backend": st.stores.search.name(), "status": check(st.stores.search.health().await)},
        {"name": "Vectors", "backend": st.stores.vector.name(), "status": {"ok": true}},
        {"name": "Embeddings", "backend": st.stores.embedder.as_ref().map(|e| e.model()).unwrap_or_else(|| "off".into()), "status": {"ok": true}},
        {"name": "Cache", "backend": st.stores.kv.name(), "status": check(st.stores.kv.set("health:probe", "1", 30).await)},
        {"name": "Graph", "backend": st.stores.graph.name(), "status": {"ok": true}},
        {"name": "Worker", "backend": if st.cfg.worker.embedded { "embedded in the server" } else { "separate gantry-worker service" }, "status": {"ok": true}},
        {"name": "Email", "backend": if st.cfg.email.enabled() { format!("SMTP at {}", st.cfg.email.host) } else { "off: notifications stay in the application".into() },
         "status": if st.cfg.email.enabled() && st.cfg.server.public_url.is_empty() { json!({"ok": false, "error": "server.public_url is not set, so no email is sent"}) } else { json!({"ok": true}) }},
        {"name": "Live updates", "backend": format!("{} open on this node", match st.hub.connections() { 1 => "1 connection".to_string(), n => format!("{n} connections") }), "status": {"ok": true}},
    ]);
    // Every server and worker process that has reported in lately. One that has gone quiet is
    // listed for a while, so a node that dropped out is noticed.
    let nodes: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(jsonb_build_object('node', node, 'role', role, 'version', version, 'started_at', started_at, 'seen_at', seen_at,
                 'alive', seen_at > now() - interval '3 minutes') ORDER BY role, node), '[]'::jsonb) FROM cluster_nodes WHERE seen_at > now() - interval '1 day'",
    )
    .fetch_one(&st.db)
    .await?;
    Ok(Json(json!({"version": crate::VERSION, "node": st.node_id, "uptime_seconds": st.started.elapsed().as_secs(), "public_url": st.cfg.server.public_url, "counts": counts, "components": components, "nodes": nodes})))
}

async fn jobs(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.access")?;
    Ok(Json(
        sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(j) ORDER BY j.id DESC), '[]'::jsonb) FROM (
               SELECT id, kind, state, attempts, max_attempts, run_at, last_error, created_at, finished_at FROM jobs
               WHERE state IN ('failed', 'queued', 'running') OR finished_at > now() - interval '1 hour' ORDER BY id DESC LIMIT 200) j",
        )
        .fetch_one(&st.db)
        .await?,
    ))
}

async fn retry_job(State(st): State<St>, ctx: Ctx, Path(id): Path<i64>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.settings")?;
    let n = sqlx::query("UPDATE jobs SET state = 'queued', attempts = 0, run_at = now(), last_error = NULL WHERE id = $1 AND state = 'failed'").bind(id).execute(&st.db).await?.rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    audit::log(&st, &ctx, "job.retry", "job", &id.to_string(), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── users ─────────────────────────

#[derive(Deserialize)]
struct UserQuery {
    q: Option<String>,
    kind: Option<String>,
    #[serde(flatten)]
    paging: Paging,
}

async fn list_users(State(st): State<St>, ctx: Ctx, Query(q): Query<UserQuery>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.users")?;
    let (limit, offset) = q.paging.get();
    let like = q.q.as_deref().map(|s| format!("%{}%", crate::storage::like_escape(s.trim())));
    let users: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.name), '[]'::jsonb) FROM (
           SELECT u.id, u.kind, u.email, u.name, u.handle, u.active, u.last_login_at, u.created_at, u.locked_until > now() AS locked, u.password_hash IS NOT NULL AS has_password,
                  u.org_name, u.org_code, u.job_title, u.timezone, u.description, u.avatar_blob IS NOT NULL AS has_avatar, person_json(u) AS person,
                  (SELECT person_json(o) FROM users o WHERE o.id = u.owner_id) AS owner,
                  CASE WHEN u.kind = 'agent' THEN coalesce(u.agent_policy->>'mode', 'restricted') END AS agent_mode,
                  (SELECT coalesce(jsonb_agg(jsonb_build_object('binding_id', b.id, 'key', r.key, 'name', r.name)), '[]'::jsonb) FROM role_bindings b JOIN roles r ON r.id = b.role_id WHERE b.user_id = u.id AND b.scope_type = 'global') AS roles,
                  (SELECT coalesce(jsonb_agg(jsonb_build_object('id', g.id, 'name', g.name)), '[]'::jsonb) FROM group_members m JOIN groups g ON g.id = m.group_id WHERE m.user_id = u.id) AS groups,
                  (SELECT count(*) FROM api_tokens t WHERE t.user_id = u.id AND t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > now())) AS active_tokens
           FROM users u WHERE ($1::text IS NULL OR u.name ILIKE $1 OR u.email ILIKE $1 OR u.handle ILIKE $1 OR u.org_code ILIKE $1) AND ($2::text IS NULL OR u.kind = $2)
           ORDER BY u.name LIMIT $3 OFFSET $4) t",
    )
    .bind(&like)
    .bind(&q.kind)
    .bind(limit)
    .bind(offset)
    .fetch_one(&st.db)
    .await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM users u WHERE ($1::text IS NULL OR u.name ILIKE $1 OR u.email ILIKE $1 OR u.handle ILIKE $1 OR u.org_code ILIKE $1) AND ($2::text IS NULL OR u.kind = $2)").bind(&like).bind(&q.kind).fetch_one(&st.db).await?;
    Ok(Json(json!({"users": users, "total": total})))
}

#[derive(Deserialize)]
struct NewUser {
    email: Option<String>,
    name: String,
    #[serde(default)]
    kind: Option<String>,
    password: Option<String>,
    role: Option<String>,
    /// AI agents: what the agent is for, and the person answerable for it.
    description: Option<String>,
    owner_id: Option<Uuid>,
}

/// Only someone who may change sign-in security can hand out roles that carry it.
async fn check_role_grant(st: &St, ctx: &Ctx, role_key: &str, scope: &str) -> AppResult<Uuid> {
    let row = sqlx::query("SELECT id, permissions FROM roles WHERE key = $1 AND scope = $2").bind(role_key).bind(scope).fetch_optional(&st.db).await?.ok_or_else(|| bad(format!("unknown {scope} role {role_key:?}")))?;
    let perms: Vec<String> = row.get("permissions");
    // A role can be handed out (or taken away) only by someone who holds everything in it.
    let holds = |p: &String| if p == "*" { ctx.global.0.contains("*") && ctx.token_level() >= 3 } else { ctx.has(p) };
    if !perms.iter().all(holds) {
        let _ = authz::deny(st, ctx, "role.grant", &format!("role:{role_key}"));
        return Err(AppError::Forbidden(format!("you cannot grant or revoke the role {role_key:?} because it includes permissions you do not hold")));
    }
    Ok(row.get("id"))
}

async fn create_user(State(st): State<St>, ctx: Ctx, Json(b): Json<NewUser>) -> AppResult<(StatusCode, Json<Value>)> {
    let kind = b.kind.as_deref().unwrap_or("human");
    if !["human", "agent"].contains(&kind) {
        return Err(bad("kind must be human or agent"));
    }
    authz::require_global(&st, &ctx, if kind == "agent" { "admin.agents" } else { "admin.users" })?;
    let name = b.name.trim();
    if name.is_empty() || name.len() > 200 || name.chars().any(accounts::deceptive) {
        return Err(bad("name must be 1 to 200 characters, on one line"));
    }
    // Agents do not sign in, so they get a placeholder address that cannot receive mail.
    let email = match (kind, b.email.as_deref().map(|e| e.trim().to_lowercase())) {
        (_, Some(e)) if valid_email(&e) => e,
        ("agent", None) => format!("{}-{}@agents.invalid", name.to_lowercase().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect::<String>(), crate::crypto::random_hex(3)),
        _ => return Err(bad("a valid email address is required")),
    };
    let hash = match (kind, &b.password) {
        ("human", Some(p)) => {
            auth::validate_password(&st, p)?;
            Some(auth::hash_password_async(p.clone()).await?)
        }
        ("agent", Some(_)) => return Err(bad("agents authenticate with API tokens, not passwords")),
        _ => None,
    };
    let role = b.role.as_deref().unwrap_or("member");
    let role_id = check_role_grant(&st, &ctx, role, "global").await?;
    // A new agent may do nothing until an administrator writes its rules.
    let policy = (kind == "agent").then(|| authz::AgentPolicy::default().to_json());
    let id: Uuid = sqlx::query_scalar("INSERT INTO users (kind, email, name, password_hash, agent_policy) VALUES ($1,$2,$3,$4,$5) RETURNING id").bind(kind).bind(&email).bind(name).bind(hash).bind(policy).fetch_one(&st.db).await.map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => AppError::Conflict("an account with this email address already exists".into()),
        e => e.into(),
    })?;
    sqlx::query("INSERT INTO role_bindings (role_id, user_id, scope_type) VALUES ($1, $2, 'global')").bind(role_id).bind(id).execute(&st.db).await?;
    if kind == "agent" {
        let mut extra = Map::new();
        if let Some(d) = &b.description {
            extra.insert("description".into(), json!(d));
        }
        // Whoever creates an agent answers for it unless someone else is named.
        extra.insert("owner_id".into(), json!(b.owner_id.unwrap_or(ctx.user_id)));
        accounts::admin_apply_profile(&st, id, &extra).await?;
    }
    audit::log(&st, &ctx, "user.create", "user", &id.to_string(), json!({"email": email, "kind": kind, "role": role})).await;
    Ok((StatusCode::CREATED, Json(json!({"id": id, "email": email}))))
}

/// The last active system administrator cannot be removed, deactivated or demoted.
async fn other_system_admins(st: &St, excluding: Uuid) -> AppResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(DISTINCT u.id) FROM users u
         WHERE u.active AND u.kind = 'human' AND u.id <> $1 AND EXISTS (
           SELECT 1 FROM role_bindings b JOIN roles r ON r.id = b.role_id
           WHERE r.permissions @> ARRAY['*'] AND b.scope_type = 'global'
             AND (b.user_id = u.id OR b.group_id IN (SELECT group_id FROM group_members WHERE user_id = u.id)))",
    )
    .bind(excluding)
    .fetch_one(&st.db)
    .await?)
}

async fn update_user(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Map<String, Value>>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.users")?;
    let cur = sqlx::query("SELECT kind, active FROM users WHERE id = $1").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    for (k, v) in &b {
        match k.as_str() {
            "name" => {
                let n = v.as_str().map(str::trim).filter(|n| !n.is_empty() && n.len() <= 200).ok_or_else(|| bad("name must be 1 to 200 characters"))?;
                sqlx::query("UPDATE users SET name = $2, updated_at = now() WHERE id = $1").bind(id).bind(n).execute(&st.db).await?;
            }
            "email" => {
                let e = v.as_str().map(|e| e.trim().to_lowercase()).filter(|e| valid_email(e)).ok_or_else(|| bad("a valid email address is required"))?;
                sqlx::query("UPDATE users SET email = $2, updated_at = now() WHERE id = $1").bind(id).bind(e).execute(&st.db).await?;
            }
            "active" => {
                let active = v.as_bool().ok_or_else(|| bad("active must be true or false"))?;
                if !active {
                    if id == ctx.user_id {
                        return Err(bad("you cannot deactivate your own account"));
                    }
                    if other_system_admins(&st, id).await? == 0 {
                        return Err(bad("this is the last active system administrator; promote someone else first"));
                    }
                    sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(id).execute(&st.db).await?;
                    sqlx::query("UPDATE api_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL").bind(id).execute(&st.db).await?;
                }
                sqlx::query("UPDATE users SET active = $2, updated_at = now() WHERE id = $1").bind(id).bind(active).execute(&st.db).await?;
            }
            "unlock" if v == &json!(true) => {
                sqlx::query("UPDATE users SET locked_until = NULL, failed_logins = 0 WHERE id = $1").bind(id).execute(&st.db).await?;
            }
            "remove_avatar" if v == &json!(true) => accounts::admin_remove_avatar(&st, id).await?,
            field if accounts::ADMIN_PROFILE_FIELDS.contains(&field) => {
                let mut one = Map::new();
                one.insert(k.clone(), v.clone());
                accounts::admin_apply_profile(&st, id, &one).await?;
            }
            "password" => {
                authz::require_global(&st, &ctx, "admin.security")?;
                if cur.get::<String, _>("kind") != "human" {
                    return Err(bad("agents authenticate with API tokens, not passwords"));
                }
                let p = v.as_str().ok_or_else(|| bad("password must be text"))?;
                auth::validate_password(&st, p)?;
                let hash = auth::hash_password_async(p.to_string()).await?;
                sqlx::query("UPDATE users SET password_hash = $2, failed_logins = 0, locked_until = NULL, updated_at = now() WHERE id = $1").bind(id).bind(hash).execute(&st.db).await?;
                sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(id).execute(&st.db).await?;
            }
            other => return Err(bad(format!("unknown field {other:?}"))),
        }
    }
    // Field names only: the audit log must never contain a password.
    audit::log(&st, &ctx, "user.update", "user", &id.to_string(), json!({"changed": b.keys().collect::<Vec<_>>(), "active": b.get("active")})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_sessions(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.users")?;
    let n = sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(id).execute(&st.db).await?.rows_affected();
    audit::log(&st, &ctx, "user.sessions_revoke", "user", &id.to_string(), json!({"sessions": n})).await;
    Ok(Json(json!({"revoked": n})))
}

#[derive(Deserialize)]
struct RoleGrant {
    role: String,
}

async fn grant_role(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<RoleGrant>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.users")?;
    let role_id = check_role_grant(&st, &ctx, &b.role, "global").await?;
    sqlx::query("INSERT INTO role_bindings (role_id, user_id, scope_type) VALUES ($1, $2, 'global') ON CONFLICT DO NOTHING").bind(role_id).bind(id).execute(&st.db).await?;
    audit::log(&st, &ctx, "role.grant", "user", &id.to_string(), json!({"role": b.role, "scope": "global"})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_role(State(st): State<St>, ctx: Ctx, Path((id, role)): Path<(Uuid, String)>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.users")?;
    check_role_grant(&st, &ctx, &role, "global").await?;
    if role == "system_admin" && other_system_admins(&st, id).await? == 0 {
        return Err(bad("this is the last active system administrator; promote someone else first"));
    }
    let n = sqlx::query("DELETE FROM role_bindings b USING roles r WHERE r.id = b.role_id AND r.key = $2 AND b.user_id = $1 AND b.scope_type = 'global'").bind(id).bind(&role).execute(&st.db).await?.rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    audit::log(&st, &ctx, "role.revoke", "user", &id.to_string(), json!({"role": role, "scope": "global"})).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Tokens of people are a sign-in security matter; tokens of agents belong to whoever runs the agents.
async fn token_admin(st: &St, ctx: &Ctx, owner: Uuid) -> AppResult<String> {
    let kind: String = sqlx::query_scalar("SELECT kind FROM users WHERE id = $1").bind(owner).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    if kind == "agent" && ctx.has("admin.agents") {
        return Ok(kind);
    }
    authz::require_global(st, ctx, "admin.security")?;
    Ok(kind)
}

async fn user_tokens(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    token_admin(&st, &ctx, id).await?;
    Ok(Json(sqlx::query_scalar(auth::TOKEN_LIST_SQL).bind(id).fetch_one(&st.db).await?))
}

/// Mint a token for another account. Intended for agents and service accounts.
async fn create_user_token(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<TokenBody>) -> AppResult<(StatusCode, Json<Value>)> {
    let kind = token_admin(&st, &ctx, id).await?;
    if !ctx.is_session() {
        return Err(AppError::Forbidden("tokens can only be created from a signed-in browser session".into()));
    }
    let active: bool = sqlx::query_scalar("SELECT active FROM users WHERE id = $1").bind(id).fetch_one(&st.db).await?;
    if kind != "agent" || !active {
        return Err(bad("administrators can create tokens only for active agent accounts; people create their own under Profile"));
    }
    if b.scopes.iter().any(|s| s == "admin") && !ctx.has("admin.security") {
        return Err(AppError::Forbidden("only an administrator who manages sign-in security can give an agent a token with the admin scope".into()));
    }
    Ok((StatusCode::CREATED, Json(auth::mint_token(&st, &ctx, id, &b.name, &b.scopes, b.expires_in_days).await?)))
}

// ───────────────────────── groups ─────────────────────────

async fn list_groups(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.users")?;
    Ok(Json(
        sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.name), '[]'::jsonb) FROM (
               SELECT g.id, g.name, g.description, g.external_id IS NOT NULL AS provisioned,
                 (SELECT coalesce(jsonb_agg(jsonb_build_object('id', u.id, 'name', u.name, 'email', u.email) ORDER BY u.name), '[]'::jsonb) FROM group_members m JOIN users u ON u.id = m.user_id WHERE m.group_id = g.id) AS members,
                 (SELECT coalesce(jsonb_agg(jsonb_build_object('key', r.key, 'name', r.name)), '[]'::jsonb) FROM role_bindings b JOIN roles r ON r.id = b.role_id WHERE b.group_id = g.id AND b.scope_type = 'global') AS roles
               FROM groups g) t",
        )
        .fetch_one(&st.db)
        .await?,
    ))
}

#[derive(Deserialize)]
struct GroupBody {
    name: String,
    #[serde(default)]
    description: String,
}

async fn create_group(State(st): State<St>, ctx: Ctx, Json(b): Json<GroupBody>) -> AppResult<(StatusCode, Json<Value>)> {
    authz::require_global(&st, &ctx, "admin.users")?;
    if b.name.trim().is_empty() || b.name.len() > 200 {
        return Err(bad("name must be 1 to 200 characters"));
    }
    let id: Uuid = sqlx::query_scalar("INSERT INTO groups (name, description) VALUES ($1, $2) RETURNING id").bind(b.name.trim()).bind(&b.description).fetch_one(&st.db).await?;
    audit::log(&st, &ctx, "group.create", "group", &id.to_string(), json!({"name": b.name.trim()})).await;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

async fn delete_group(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.users")?;
    let name: String = sqlx::query_scalar("DELETE FROM groups WHERE id = $1 RETURNING name").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    audit::log(&st, &ctx, "group.delete", "group", &id.to_string(), json!({"name": name})).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct MemberRef {
    user_id: Uuid,
}

/// Roles a group holds pass to its members, so adding a member is a grant of those roles.
async fn check_group_grant(st: &St, ctx: &Ctx, group: Uuid) -> AppResult<()> {
    let roles: Vec<(String, String)> = sqlx::query_as("SELECT r.key, r.scope FROM role_bindings b JOIN roles r ON r.id = b.role_id WHERE b.group_id = $1 AND b.scope_type = 'global'").bind(group).fetch_all(&st.db).await?;
    for (key, scope) in roles {
        check_role_grant(st, ctx, &key, &scope).await?;
    }
    Ok(())
}

async fn add_group_member(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<MemberRef>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.users")?;
    check_group_grant(&st, &ctx, id).await?;
    sqlx::query("INSERT INTO group_members (group_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(id).bind(b.user_id).execute(&st.db).await?;
    audit::log(&st, &ctx, "group.member_add", "group", &id.to_string(), json!({"user_id": b.user_id})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove_group_member(State(st): State<St>, ctx: Ctx, Path((id, user)): Path<(Uuid, Uuid)>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.users")?;
    sqlx::query("DELETE FROM group_members WHERE group_id = $1 AND user_id = $2").bind(id).bind(user).execute(&st.db).await?;
    audit::log(&st, &ctx, "group.member_remove", "group", &id.to_string(), json!({"user_id": user})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn grant_group_role(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<RoleGrant>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.users")?;
    let role_id = check_role_grant(&st, &ctx, &b.role, "global").await?;
    sqlx::query("INSERT INTO role_bindings (role_id, group_id, scope_type) VALUES ($1, $2, 'global') ON CONFLICT DO NOTHING").bind(role_id).bind(id).execute(&st.db).await?;
    audit::log(&st, &ctx, "role.grant", "group", &id.to_string(), json!({"role": b.role, "scope": "global"})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn revoke_group_role(State(st): State<St>, ctx: Ctx, Path((id, role)): Path<(Uuid, String)>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.users")?;
    check_role_grant(&st, &ctx, &role, "global").await?;
    sqlx::query("DELETE FROM role_bindings b USING roles r WHERE r.id = b.role_id AND r.key = $2 AND b.group_id = $1 AND b.scope_type = 'global'").bind(id).bind(&role).execute(&st.db).await?;
    audit::log(&st, &ctx, "role.revoke", "group", &id.to_string(), json!({"role": role, "scope": "global"})).await;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── roles ─────────────────────────

async fn list_roles(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.access")?;
    let roles: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('key', key, 'name', name, 'scope', scope, 'permissions', permissions, 'builtin', builtin, 'description', description) ORDER BY scope, builtin DESC, name), '[]'::jsonb) FROM roles").fetch_one(&st.db).await?;
    let perms: Vec<Value> = authz::PERMISSIONS.iter().map(|(k, s, d)| json!({"key": k, "scope": s, "description": d})).collect();
    Ok(Json(json!({"roles": roles, "permissions": perms})))
}

#[derive(Deserialize)]
struct RoleBody {
    key: Option<String>,
    name: String,
    scope: Option<String>,
    permissions: Vec<String>,
    #[serde(default)]
    description: String,
}

fn check_role_perms(ctx: &Ctx, scope: &str, perms: &[String]) -> AppResult<()> {
    for p in perms {
        let Some((_, pscope, _)) = authz::PERMISSIONS.iter().find(|(k, _, _)| k == p) else { return Err(bad(format!("unknown permission {p:?}"))) };
        if scope != "global" && *pscope != scope {
            return Err(bad(format!("{p:?} is a {pscope} permission and cannot be part of a {scope} role")));
        }
        if !ctx.has(p) {
            return Err(AppError::Forbidden(format!("you cannot put {p:?} in a role because you do not hold it")));
        }
    }
    Ok(())
}

async fn create_role(State(st): State<St>, ctx: Ctx, Json(b): Json<RoleBody>) -> AppResult<(StatusCode, Json<Value>)> {
    authz::require_global(&st, &ctx, "admin.roles")?;
    let key = b.key.as_deref().unwrap_or_default();
    if !packs::valid_key(key) || key.len() < 2 {
        return Err(bad("key must be lowercase letters, digits and underscores, starting with a letter"));
    }
    let scope = b.scope.as_deref().unwrap_or("project");
    if !["global", "project", "space"].contains(&scope) {
        return Err(bad("scope must be global, project or space"));
    }
    check_role_perms(&ctx, scope, &b.permissions)?;
    sqlx::query("INSERT INTO roles (key, name, scope, permissions, description) VALUES ($1,$2,$3,$4,$5)").bind(key).bind(b.name.trim()).bind(scope).bind(&b.permissions).bind(&b.description).execute(&st.db).await?;
    audit::log(&st, &ctx, "role.create", "role", key, json!({"scope": scope, "permissions": b.permissions})).await;
    Ok((StatusCode::CREATED, Json(json!({"key": key}))))
}

async fn update_role(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<RoleBody>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.roles")?;
    let row = sqlx::query("SELECT scope, builtin FROM roles WHERE key = $1").bind(&key).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    if row.get::<bool, _>("builtin") {
        return Err(bad("built-in roles cannot be changed; create a custom role instead"));
    }
    check_role_perms(&ctx, &row.get::<String, _>("scope"), &b.permissions)?;
    sqlx::query("UPDATE roles SET name = $2, permissions = $3, description = $4 WHERE key = $1").bind(&key).bind(b.name.trim()).bind(&b.permissions).bind(&b.description).execute(&st.db).await?;
    audit::log(&st, &ctx, "role.update", "role", &key, json!({"permissions": b.permissions})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_role(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.roles")?;
    let n = sqlx::query("DELETE FROM roles WHERE key = $1 AND NOT builtin").bind(&key).execute(&st.db).await?.rows_affected();
    if n == 0 {
        return Err(bad("no such custom role (built-in roles cannot be deleted)"));
    }
    audit::log(&st, &ctx, "role.delete", "role", &key, json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── sign-in providers ─────────────────────────

async fn list_providers(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.security")?;
    let base = st.cfg.server.public_url.trim_end_matches('/').to_string();
    let mut list: Vec<Value> = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', id, 'kind', kind, 'name', name, 'issuer', issuer, 'client_id', client_id, 'has_secret', client_secret_enc <> '', 'scopes', scopes,
            'enabled', enabled, 'auto_provision', auto_provision, 'link_by_email', link_by_email, 'allowed_domains', allowed_domains, 'default_role', default_role, 'config', config,
            'redirect_uri', $1 || '/api/v1/auth/oidc/' || id || '/callback') FROM auth_providers ORDER BY name",
    )
    .bind(base)
    .fetch_all(&st.db)
    .await?;
    for p in &mut list {
        if p["kind"] == "saml" {
            let id: Uuid = p["id"].as_str().and_then(|s| s.parse().ok()).unwrap_or_default();
            p["saml"] = saml::describe(&st, id, &p["config"]);
            p.as_object_mut().expect("provider is an object").remove("redirect_uri");
        }
    }
    Ok(Json(json!(list)))
}

#[derive(Deserialize)]
struct ProviderBody {
    /// `oidc` (default) or `saml`. Fixed once the provider exists.
    kind: Option<String>,
    name: Option<String>,
    /// OpenID Connect: the issuer URL. SAML: the identity provider's entity ID.
    issuer: Option<String>,
    client_id: Option<String>,
    client_secret: Option<String>,
    scopes: Option<String>,
    enabled: Option<bool>,
    auto_provision: Option<bool>,
    link_by_email: Option<bool>,
    allowed_domains: Option<Vec<String>>,
    default_role: Option<String>,
    /// SAML only: sso_url, certificates, sp_entity_id, email_attribute, name_attribute, allow_idp_initiated.
    config: Option<Value>,
}

async fn check_default_role(st: &St, role: &Option<String>) -> AppResult<()> {
    if let Some(r) = role {
        // New sign-ups must never land in a role that carries administration.
        let perms: Vec<String> = sqlx::query_scalar("SELECT permissions FROM roles WHERE key = $1 AND scope = 'global'").bind(r).fetch_optional(&st.db).await?.ok_or_else(|| bad("default_role must be a global role"))?;
        if perms.iter().any(|p| p == "*" || p.starts_with("admin.") || p.starts_with("scim.")) {
            return Err(bad("default_role cannot be an administrative role"));
        }
    }
    Ok(())
}

fn domains(d: &[String]) -> Vec<String> {
    d.iter().map(|d| d.trim().to_lowercase()).filter(|d| !d.is_empty()).collect()
}

async fn create_provider(State(st): State<St>, ctx: Ctx, Json(b): Json<ProviderBody>) -> AppResult<(StatusCode, Json<Value>)> {
    authz::require_global(&st, &ctx, "admin.security")?;
    let kind = b.kind.as_deref().unwrap_or("oidc");
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty() && n.chars().count() <= 100).ok_or_else(|| bad("name is required (at most 100 characters)"))?;
    check_default_role(&st, &b.default_role).await?;
    let base = st.cfg.server.public_url.trim_end_matches('/');
    let (issuer, client_id, config) = match kind {
        "oidc" => {
            let issuer = b.issuer.as_deref().map(|i| i.trim().trim_end_matches('/')).filter(|i| !i.is_empty()).ok_or_else(|| bad("issuer is required"))?;
            let client_id = b.client_id.as_deref().map(str::trim).filter(|c| !c.is_empty()).ok_or_else(|| bad("client_id is required"))?;
            oidc::check_provider(&st, issuer).await?;
            (issuer.to_string(), client_id.to_string(), json!({}))
        }
        "saml" => {
            let issuer = b.issuer.as_deref().map(str::trim).unwrap_or_default();
            (issuer.to_string(), String::new(), saml::check_config(&st, issuer, b.config.as_ref().unwrap_or(&json!({})))?)
        }
        _ => return Err(bad("kind must be oidc or saml")),
    };
    let id: Uuid = sqlx::query_scalar("INSERT INTO auth_providers (kind, name, issuer, client_id, client_secret_enc, scopes, enabled, auto_provision, link_by_email, allowed_domains, default_role, config) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12) RETURNING id")
        .bind(kind)
        .bind(name)
        .bind(&issuer)
        .bind(&client_id)
        .bind(st.secrets.seal(b.client_secret.as_deref().unwrap_or_default()))
        .bind(b.scopes.as_deref().unwrap_or("openid email profile"))
        .bind(b.enabled.unwrap_or(true))
        .bind(b.auto_provision.unwrap_or(true))
        .bind(b.link_by_email.unwrap_or(true))
        .bind(domains(&b.allowed_domains.unwrap_or_default()))
        .bind(b.default_role.as_deref().unwrap_or("member"))
        .bind(&config)
        .fetch_one(&st.db)
        .await?;
    audit::log(&st, &ctx, "auth_provider.create", "auth_provider", &id.to_string(), json!({"kind": kind, "name": name, "issuer": issuer})).await;
    Ok((StatusCode::CREATED, Json(if kind == "saml" { json!({"id": id, "saml": saml::describe(&st, id, &config)}) } else { json!({"id": id, "redirect_uri": format!("{base}/api/v1/auth/oidc/{id}/callback")}) })))
}

async fn update_provider(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<ProviderBody>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.security")?;
    check_default_role(&st, &b.default_role).await?;
    let cur = sqlx::query("SELECT kind, issuer, config FROM auth_providers WHERE id = $1").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let kind: String = cur.get("kind");
    if b.kind.as_deref().is_some_and(|k| k != kind) {
        return Err(bad("the kind of a sign-in provider cannot change; add another provider instead"));
    }
    let issuer = b.issuer.as_deref().map(|i| if kind == "oidc" { i.trim().trim_end_matches('/').to_string() } else { i.trim().to_string() });
    let config = if kind == "saml" {
        // Settings left out keep their stored values.
        let mut merged: Value = cur.get("config");
        if let Some(c) = &b.config {
            settings::merge(&mut merged, c);
        }
        Some(saml::check_config(&st, issuer.as_deref().unwrap_or(&cur.get::<String, _>("issuer")), &merged)?)
    } else {
        if let Some(i) = &issuer {
            oidc::check_provider(&st, i).await?;
        }
        None
    };
    sqlx::query(
        "UPDATE auth_providers SET name = coalesce($2, name), issuer = coalesce($3, issuer), client_id = coalesce($4, client_id), client_secret_enc = coalesce($5, client_secret_enc),
            scopes = coalesce($6, scopes), enabled = coalesce($7, enabled), auto_provision = coalesce($8, auto_provision), link_by_email = coalesce($9, link_by_email),
            allowed_domains = coalesce($10, allowed_domains), default_role = coalesce($11, default_role), config = coalesce($12, config) WHERE id = $1",
    )
    .bind(id)
    .bind(b.name.as_deref().map(str::trim).filter(|n| !n.is_empty() && n.chars().count() <= 100))
    .bind(&issuer)
    .bind(b.client_id.as_deref().map(str::trim).filter(|c| !c.is_empty()))
    .bind(b.client_secret.as_deref().filter(|s| !s.is_empty()).map(|s| st.secrets.seal(s)))
    .bind(&b.scopes)
    .bind(b.enabled)
    .bind(b.auto_provision)
    .bind(b.link_by_email)
    .bind(b.allowed_domains.as_deref().map(domains))
    .bind(&b.default_role)
    .bind(&config)
    .execute(&st.db)
    .await?;
    audit::log(&st, &ctx, "auth_provider.update", "auth_provider", &id.to_string(), json!({"kind": kind, "enabled": b.enabled, "secret_changed": b.client_secret.is_some(), "settings_changed": b.config.is_some()})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_provider(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.security")?;
    let name: String = sqlx::query_scalar("DELETE FROM auth_providers WHERE id = $1 RETURNING name").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    audit::log(&st, &ctx, "auth_provider.delete", "auth_provider", &id.to_string(), json!({"name": name})).await;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── audit log and security events ─────────────────────────

#[derive(Deserialize)]
struct AuditQuery {
    actor: Option<String>,
    action: Option<String>,
    target: Option<String>,
    outcome: Option<String>,
    from: Option<chrono::DateTime<chrono::Utc>>,
    to: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(flatten)]
    paging: Paging,
}

async fn audit_log(State(st): State<St>, ctx: Ctx, Query(q): Query<AuditQuery>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.audit")?;
    let (limit, offset) = q.paging.get();
    let filters = |qb: &mut QueryBuilder<'_, Postgres>| {
        qb.push(" WHERE true");
        if let Some(a) = q.actor.as_deref().filter(|a| !a.is_empty()) {
            // The label is what was recorded (an email address; for an AI agent its internal
            // address). People look for agents by name, so the account's name matches too.
            let pattern = format!("%{}%", crate::storage::like_escape(a));
            qb.push(" AND (actor_label ILIKE ").push_bind(pattern.clone()).push(" OR actor_id IN (SELECT id FROM users WHERE name ILIKE ").push_bind(pattern).push("))");
        }
        if let Some(a) = q.action.as_deref().filter(|a| !a.is_empty()) {
            qb.push(" AND action LIKE ").push_bind(format!("{}%", crate::storage::like_escape(a)));
        }
        if let Some(t) = q.target.as_deref().filter(|t| !t.is_empty()) {
            qb.push(" AND target_id ILIKE ").push_bind(format!("%{}%", crate::storage::like_escape(t)));
        }
        if let Some(o) = q.outcome.as_deref().filter(|o| !o.is_empty()) {
            qb.push(" AND outcome = ").push_bind(o.to_string());
        }
        if let Some(f) = q.from {
            qb.push(" AND at >= ").push_bind(f);
        }
        if let Some(t) = q.to {
            qb.push(" AND at <= ").push_bind(t);
        }
    };
    let mut qb = QueryBuilder::<Postgres>::new("SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.id DESC), '[]'::jsonb) FROM (SELECT id, at, actor_id, actor_kind, actor_label, (SELECT u.name FROM users u WHERE u.id = audit_log.actor_id) AS actor_name, auth_method, action, target_type, target_id, ip, request_id, outcome, detail FROM audit_log");
    filters(&mut qb);
    qb.push(" ORDER BY id DESC LIMIT ").push_bind(limit).push(" OFFSET ").push_bind(offset).push(") t");
    let entries: Value = qb.build_query_scalar().fetch_one(&st.db).await?;
    let mut cq = QueryBuilder::<Postgres>::new("SELECT count(*) FROM audit_log");
    filters(&mut cq);
    let total: i64 = cq.build_query_scalar().fetch_one(&st.db).await?;
    audit::log(&st, &ctx, "audit.read", "audit_log", "", json!({"filters": {"actor": q.actor, "action": q.action, "target": q.target}})).await;
    Ok(Json(json!({"entries": entries, "total": total})))
}

async fn audit_verify(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.audit")?;
    let report = audit::verify_chain(&st.db).await?;
    if !report.intact {
        audit::security(&st, "audit.chain_broken", "critical", Some((ctx.user_id, &ctx.email)), &ctx.req, json!({"first_broken_id": report.first_broken_id})).await;
    }
    Ok(Json(json!(report)))
}

#[derive(Deserialize)]
struct SecurityQuery {
    kind: Option<String>,
    severity: Option<String>,
    #[serde(flatten)]
    paging: Paging,
}

async fn security_events(State(st): State<St>, ctx: Ctx, Query(q): Query<SecurityQuery>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.security")?;
    let (limit, offset) = q.paging.get();
    let kind = q.kind.as_deref().filter(|k| !k.is_empty()).map(|k| format!("{}%", crate::storage::like_escape(k)));
    let severity = q.severity.filter(|s| !s.is_empty());
    let events: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.id DESC), '[]'::jsonb) FROM (
           SELECT e.id, e.at, e.kind, e.severity, e.actor_label, e.ip, e.request_id, e.detail, u.name AS actor_name, u.kind AS actor_kind FROM security_events e
           LEFT JOIN users u ON u.id = e.actor_id
           WHERE ($1::text IS NULL OR e.kind LIKE $1) AND ($2::text IS NULL OR e.severity = $2) ORDER BY e.id DESC LIMIT $3 OFFSET $4) t",
    )
    .bind(&kind)
    .bind(&severity)
    .bind(limit)
    .bind(offset)
    .fetch_one(&st.db)
    .await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM security_events WHERE ($1::text IS NULL OR kind LIKE $1) AND ($2::text IS NULL OR severity = $2)").bind(&kind).bind(&severity).fetch_one(&st.db).await?;
    let summary: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('kind', kind, 'count', n) ORDER BY n DESC), '[]'::jsonb) FROM (SELECT kind, count(*) AS n FROM security_events WHERE at > now() - interval '24 hours' GROUP BY kind) s").fetch_one(&st.db).await?;
    Ok(Json(json!({"events": events, "total": total, "last_24h": summary})))
}

// ───────────────────────── schemes: fields, workflows, types, packs ─────────────────────────

async fn schemes(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.schemes")?;
    let fields: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(f) ORDER BY f.name), '[]'::jsonb) FROM fields f").fetch_one(&st.db).await?;
    let workflows: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('key', key, 'name', name, 'source', source, 'definition', definition, 'used_by', (SELECT count(*) FROM item_types t WHERE t.workflow_key = w.key)) ORDER BY name), '[]'::jsonb) FROM workflows w").fetch_one(&st.db).await?;
    let types: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('key', key, 'name', name, 'icon', icon, 'color', color, 'description', description, 'workflow', workflow_key, 'fields', fields, 'source', source, 'active', active,
            'projects', (SELECT count(*) FROM project_types pt WHERE pt.type_key = t.key)) ORDER BY name), '[]'::jsonb) FROM item_types t").fetch_one(&st.db).await?;
    Ok(Json(json!({"fields": fields, "workflows": workflows, "item_types": types})))
}

#[derive(Deserialize)]
struct FieldBody {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    #[serde(default)]
    options: Vec<String>,
    #[serde(default)]
    description: String,
}

async fn put_field(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<FieldBody>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.schemes")?;
    if !packs::valid_key(&key) {
        return Err(bad("key must be lowercase letters, digits and underscores, starting with a letter"));
    }
    if !["text", "textarea", "number", "date", "select", "multiselect", "user", "url", "checkbox"].contains(&b.kind.as_str()) {
        return Err(bad("unknown field type"));
    }
    if ["select", "multiselect"].contains(&b.kind.as_str()) && b.options.iter().all(|o| o.trim().is_empty()) {
        return Err(bad("a choice field needs at least one option"));
    }
    // Changing the type of a field that holds data would invalidate that data.
    let existing: Option<String> = sqlx::query_scalar("SELECT type FROM fields WHERE key = $1").bind(&key).fetch_optional(&st.db).await?;
    if let Some(t) = &existing {
        let in_use: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM items WHERE custom ? $1)").bind(&key).fetch_one(&st.db).await?;
        if *t != b.kind && in_use {
            return Err(bad("this field already holds values; its type cannot be changed"));
        }
    }
    let options: Vec<String> = b.options.iter().map(|o| o.trim().to_string()).filter(|o| !o.is_empty()).collect();
    sqlx::query("INSERT INTO fields (key, name, type, options, description) VALUES ($1,$2,$3,$4,$5) ON CONFLICT (key) DO UPDATE SET name = EXCLUDED.name, type = EXCLUDED.type, options = EXCLUDED.options, description = EXCLUDED.description, source = 'custom'")
        .bind(&key).bind(b.name.trim()).bind(&b.kind).bind(json!(options)).bind(&b.description).execute(&st.db).await?;
    audit::log(&st, &ctx, if existing.is_some() { "field.update" } else { "field.create" }, "field", &key, json!({"type": b.kind})).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct WorkflowBody {
    name: String,
    definition: Value,
}

async fn put_workflow(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<WorkflowBody>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.schemes")?;
    if !packs::valid_key(&key) {
        return Err(bad("key must be lowercase letters, digits and underscores, starting with a letter"));
    }
    let wf: WorkflowDef = serde_json::from_value(b.definition).map_err(|e| bad(format!("workflow definition is not valid: {e}")))?;
    wf.validate().map_err(bad)?;
    let known: Vec<String> = sqlx::query_scalar("SELECT key FROM fields").fetch_all(&st.db).await?;
    if let Some(missing) = wf.transitions.iter().flat_map(|t| t.requires.iter()).find(|r| !known.contains(r)) {
        return Err(bad(format!("a transition requires the unknown field {missing:?}")));
    }
    // Items must not be stranded in a status the new definition no longer has.
    let stranded: Option<String> = sqlx::query_scalar("SELECT i.status FROM items i JOIN item_types t ON t.key = i.type_key WHERE t.workflow_key = $1 AND NOT (i.status = ANY($2)) LIMIT 1")
        .bind(&key).bind(wf.statuses.iter().map(|s| s.key.clone()).collect::<Vec<_>>()).fetch_optional(&st.db).await?;
    if let Some(s) = stranded {
        return Err(bad(format!("work items are still in status {s:?}; move them before removing that status")));
    }
    let mut tx = st.db.begin().await?;
    let existed: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM workflows WHERE key = $1)").bind(&key).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO workflows (key, name, definition) VALUES ($1,$2,$3) ON CONFLICT (key) DO UPDATE SET name = EXCLUDED.name, definition = EXCLUDED.definition, source = 'custom', updated_at = now()").bind(&key).bind(b.name.trim()).bind(wf.definition()).execute(&mut *tx).await?;
    // Keep the denormalized category on items in step with the definition.
    for s in &wf.statuses {
        sqlx::query("UPDATE items i SET status_category = $3 FROM item_types t WHERE t.key = i.type_key AND t.workflow_key = $1 AND i.status = $2 AND i.status_category <> $3").bind(&key).bind(&s.key).bind(&s.category).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    audit::log(&st, &ctx, if existed { "workflow.update" } else { "workflow.create" }, "workflow", &key, json!({"statuses": wf.statuses.len(), "transitions": wf.transitions.len()})).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct TypeBody {
    name: String,
    #[serde(default)]
    icon: Option<String>,
    #[serde(default)]
    color: Option<String>,
    #[serde(default)]
    description: String,
    workflow: String,
    #[serde(default)]
    fields: Vec<packs::TypeField>,
    #[serde(default)]
    active: Option<bool>,
}

async fn put_type(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<TypeBody>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.schemes")?;
    if !packs::valid_key(&key) {
        return Err(bad("key must be lowercase letters, digits and underscores, starting with a letter"));
    }
    let known: Vec<String> = sqlx::query_scalar("SELECT key FROM fields").fetch_all(&st.db).await?;
    if let Some(f) = b.fields.iter().find(|f| !known.contains(&f.key)) {
        return Err(bad(format!("unknown field {:?}", f.key)));
    }
    let def: Value = sqlx::query_scalar("SELECT definition FROM workflows WHERE key = $1").bind(&b.workflow).fetch_optional(&st.db).await?.ok_or_else(|| bad("unknown workflow"))?;
    let existing: Option<String> = sqlx::query_scalar("SELECT workflow_key FROM item_types WHERE key = $1").bind(&key).fetch_optional(&st.db).await?;
    if existing.as_deref().map(|w| w != b.workflow).unwrap_or(false) {
        let wf: WorkflowDef = serde_json::from_value(def).map_err(|e| AppError::Other(e.into()))?;
        let stranded: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM items WHERE type_key = $1 AND NOT (status = ANY($2)))").bind(&key).bind(wf.statuses.iter().map(|s| s.key.clone()).collect::<Vec<_>>()).fetch_one(&st.db).await?;
        if stranded {
            return Err(bad("existing items of this type are in statuses the new workflow does not have"));
        }
    }
    sqlx::query("INSERT INTO item_types (key, name, icon, color, description, workflow_key, fields, active) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
         ON CONFLICT (key) DO UPDATE SET name = EXCLUDED.name, icon = EXCLUDED.icon, color = EXCLUDED.color, description = EXCLUDED.description, workflow_key = EXCLUDED.workflow_key, fields = EXCLUDED.fields, active = EXCLUDED.active, source = 'custom'")
        .bind(&key).bind(b.name.trim()).bind(b.icon.as_deref().unwrap_or("circle")).bind(b.color.as_deref().unwrap_or("neutral")).bind(&b.description).bind(&b.workflow).bind(json!(b.fields)).bind(b.active.unwrap_or(true))
        .execute(&st.db).await?;
    audit::log(&st, &ctx, if existing.is_some() { "item_type.update" } else { "item_type.create" }, "item_type", &key, json!({"workflow": b.workflow, "fields": b.fields.len()})).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct ProjectTypesBody {
    types: Vec<String>,
}

/// Choose which work item types a project uses.
async fn put_project_types(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<ProjectTypesBody>) -> AppResult<StatusCode> {
    let (project, perms) = crate::projects::load_project(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "project.admin", &format!("project:{}", project.key))?;
    if !b.types.contains(&project.default_type) {
        return Err(bad("the project's default type must stay enabled"));
    }
    let known: Vec<String> = sqlx::query_scalar("SELECT key FROM item_types WHERE active").fetch_all(&st.db).await?;
    if let Some(t) = b.types.iter().find(|t| !known.contains(t)) {
        return Err(bad(format!("unknown work item type {t:?}")));
    }
    let in_use: Option<String> = sqlx::query_scalar("SELECT type_key FROM items WHERE project_id = $1 AND NOT archived AND NOT (type_key = ANY($2)) LIMIT 1").bind(project.id).bind(&b.types).fetch_optional(&st.db).await?;
    if let Some(t) = in_use {
        return Err(bad(format!("the project still has {t:?} items; a type in use cannot be removed")));
    }
    let mut tx = st.db.begin().await?;
    sqlx::query("DELETE FROM project_types WHERE project_id = $1").bind(project.id).execute(&mut *tx).await?;
    for t in &b.types {
        sqlx::query("INSERT INTO project_types (project_id, type_key) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(project.id).bind(t).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    audit::log(&st, &ctx, "project.types_update", "project", &project.key, json!({"types": b.types})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn list_packs(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.schemes")?;
    let installed: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'name', name, 'version', version, 'description', description, 'contents', manifest, 'installed_at', installed_at) ORDER BY id), '[]'::jsonb) FROM packs").fetch_one(&st.db).await?;
    let on_disk: Vec<Value> = packs::discover(&st.cfg.addons.dir)
        .into_iter()
        .map(|d| match d.pack {
            Ok(p) => json!({"id": p.id, "name": p.name, "version": p.version, "description": p.description, "valid": true}),
            Err(e) => json!({"id": d.id, "valid": false, "error": e}),
        })
        .collect();
    Ok(Json(json!({"installed": installed, "available": on_disk, "addons_dir": st.cfg.addons.dir})))
}

async fn install_pack(State(st): State<St>, ctx: Ctx, Path(id): Path<String>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.schemes")?;
    let pack = packs::install_addon(&st.db, &st.cfg.addons.dir, &id).await?;
    audit::log(&st, &ctx, "pack.install", "pack", &pack.id, json!({"version": pack.version})).await;
    Ok(Json(json!({"id": pack.id, "version": pack.version})))
}

// ───────────────────────── settings ─────────────────────────

/// Stored settings the caller may change, each with its effective value, and (for system
/// administrators) the configuration file as loaded, without secrets.
async fn get_settings(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.access")?;
    let mut stored = Map::new();
    let mut keys = vec![];
    for k in settings::KEYS.iter().filter(|k| ctx.has(k.permission)) {
        stored.insert(k.key.into(), settings::get(&st, k.key).await);
        keys.push(json!({"key": k.key, "summary": k.summary, "permission": k.permission}));
    }
    let mut out = json!({"stored": stored, "keys": keys, "notification_categories": settings::CATEGORIES.iter().map(|c| json!({"key": c.0, "label": c.1, "description": c.2, "permission": c.5})).collect::<Vec<_>>(),
        "legal_documents": settings::LEGAL_DOCS.iter().map(|d| json!({"key": d.0, "title": d.1})).collect::<Vec<_>>()});
    if ctx.has("admin.settings") {
        // The configuration file is shown read-only; secret values are never serialized.
        out["config_file"] = serde_json::to_value(&st.cfg).unwrap_or_default();
        out["secret_key_set"] = json!(!st.cfg.security.secret_key.is_empty());
    }
    Ok(Json(out))
}

/// Change a stored setting. Fields that are left out keep their current value.
async fn put_setting(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(v): Json<Value>) -> AppResult<StatusCode> {
    let def = settings::KEYS.iter().find(|k| k.key == key).ok_or_else(|| bad(format!("unknown setting; available: {}", settings::KEYS.iter().map(|k| k.key).collect::<Vec<_>>().join(", "))))?;
    authz::require_global(&st, &ctx, def.permission)?;
    settings::validate(&key, &v)?;
    let mut merged: Value = sqlx::query_scalar("SELECT value FROM settings WHERE key = $1").bind(&key).fetch_optional(&st.db).await?.unwrap_or_else(|| json!({}));
    settings::merge(&mut merged, &v);
    sqlx::query("INSERT INTO settings (key, value, updated_by) VALUES ($1, $2, $3) ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value, updated_at = now(), updated_by = EXCLUDED.updated_by").bind(&key).bind(&merged).bind(ctx.user_id).execute(&st.db).await?;
    // Legal texts can be long; the log records that they changed and how much, not the text.
    let detail = if key == "legal" { json!({"changed": v.as_object().map(|o| o.iter().map(|(k, t)| (k.clone(), json!(t.as_str().map(|s| s.chars().count())))).collect::<Map<_, _>>())}) } else { v };
    audit::log(&st, &ctx, "settings.update", "setting", &key, detail).await;
    Ok(StatusCode::NO_CONTENT)
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/admin/overview", "Admin", "System status: counts, components and their health", get(overview))
        .route("GET", "/api/v1/admin/jobs", "Admin", "Background jobs: queued, running, failed and recent", get(jobs))
        .route("POST", "/api/v1/admin/jobs/{id}/retry", "Admin", "Retry a failed job", post(retry_job))
        .route("GET", "/api/v1/admin/users", "Admin: users", "List users and agents", get(list_users))
        .route("POST", "/api/v1/admin/users", "Admin: users", "Create a user or an agent account", post(create_user))
        .route("PATCH", "/api/v1/admin/users/{id}", "Admin: users", "Update an account: name, email, organization, job title, handle, time zone, active, unlock, password, picture", patch(update_user))
        .route("POST", "/api/v1/admin/users/{id}/sessions/revoke", "Admin: users", "Sign a user out everywhere", post(revoke_sessions))
        .route("POST", "/api/v1/admin/users/{id}/roles", "Admin: users", "Grant a global role", post(grant_role))
        .route("DELETE", "/api/v1/admin/users/{id}/roles/{role}", "Admin: users", "Revoke a global role", delete(revoke_role))
        .route("GET", "/api/v1/admin/users/{id}/tokens", "Admin: users", "List a user's API tokens", get(user_tokens))
        .route("POST", "/api/v1/admin/users/{id}/tokens", "Admin: users", "Create an API token for an AI agent", post(create_user_token))
        .route("GET", "/api/v1/admin/groups", "Admin: groups", "List groups with members and roles", get(list_groups))
        .route("POST", "/api/v1/admin/groups", "Admin: groups", "Create a group", post(create_group))
        .route("DELETE", "/api/v1/admin/groups/{id}", "Admin: groups", "Delete a group", delete(delete_group))
        .route("POST", "/api/v1/admin/groups/{id}/members", "Admin: groups", "Add a member", post(add_group_member))
        .route("DELETE", "/api/v1/admin/groups/{id}/members/{user}", "Admin: groups", "Remove a member", delete(remove_group_member))
        .route("POST", "/api/v1/admin/groups/{id}/roles", "Admin: groups", "Grant a global role to a group", post(grant_group_role))
        .route("DELETE", "/api/v1/admin/groups/{id}/roles/{role}", "Admin: groups", "Revoke a global role from a group", delete(revoke_group_role))
        .route("GET", "/api/v1/admin/roles", "Admin: roles", "List roles and the permission catalog", get(list_roles))
        .route("POST", "/api/v1/admin/roles", "Admin: roles", "Create a custom role", post(create_role))
        .route("PUT", "/api/v1/admin/roles/{key}", "Admin: roles", "Update a custom role", put(update_role))
        .route("DELETE", "/api/v1/admin/roles/{key}", "Admin: roles", "Delete a custom role", delete(delete_role))
        .route("GET", "/api/v1/admin/auth-providers", "Admin: security", "List single sign-on providers", get(list_providers))
        .route("POST", "/api/v1/admin/auth-providers", "Admin: security", "Add a single sign-on provider", post(create_provider))
        .route("PATCH", "/api/v1/admin/auth-providers/{id}", "Admin: security", "Update a single sign-on provider", patch(update_provider))
        .route("DELETE", "/api/v1/admin/auth-providers/{id}", "Admin: security", "Remove a single sign-on provider", delete(delete_provider))
        .route("GET", "/api/v1/admin/audit", "Admin: audit", "Search the audit log", get(audit_log))
        .route("POST", "/api/v1/admin/audit/verify", "Admin: audit", "Verify the audit log hash chain", post(audit_verify))
        .route("GET", "/api/v1/admin/security-events", "Admin: security", "Security events with a 24-hour summary", get(security_events))
        .route("GET", "/api/v1/admin/schemes", "Admin: schemes", "Fields, workflows and work item types", get(schemes))
        .route("PUT", "/api/v1/admin/fields/{key}", "Admin: schemes", "Create or update a custom field", put(put_field))
        .route("PUT", "/api/v1/admin/workflows/{key}", "Admin: schemes", "Create or update a workflow", put(put_workflow))
        .route("PUT", "/api/v1/admin/item-types/{key}", "Admin: schemes", "Create or update a work item type", put(put_type))
        .route("PUT", "/api/v1/projects/{key}/types", "Projects", "Choose the work item types a project uses", put(put_project_types))
        .route("GET", "/api/v1/admin/packs", "Admin: schemes", "Installed and available add-on packs", get(list_packs))
        .route("POST", "/api/v1/admin/packs/{id}/install", "Admin: schemes", "Install or update an add-on pack", post(install_pack))
        .route("GET", "/api/v1/admin/settings", "Admin: settings", "Stored settings the caller may change, and the effective configuration (secrets omitted)", get(get_settings))
        .route("PUT", "/api/v1/admin/settings/{key}", "Admin: settings", "Change a stored setting: site, legal, profile, notifications, hierarchy", put(put_setting))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn email_shapes() {
        for ok in ["ada@example.org", "a.b+c@sub.example.co.uk"] {
            assert!(valid_email(ok), "{ok}");
        }
        for no in ["", "ada", "@example.org", "ada@", "ada@localhost", "a b@example.org", "ada@.org", "ada@example.", "ada\n@example.org"] {
            assert!(!valid_email(no), "{no:?}");
        }
    }
}
