//! Accounts as other people see them: profile, avatar, status, time zone, the people directory,
//! and the rules an administrator sets for AI agent accounts.
//!
//! Every account is either a person (`kind = human`) or an AI agent (`kind = agent`), and every
//! API response that shows an account carries that kind (see the SQL function `person_json`),
//! so the client can label agents wherever they appear.

use crate::auth::{AuthMethod, Ctx};
use crate::authz::{self, AgentPolicy};
use crate::error::{bad, AppError, AppResult};
use crate::http::{Api, Paging};
use crate::{audit, crypto, settings, St};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, put};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::Row;
use std::fmt::Write as _;
use uuid::Uuid;

pub const PRESENCE: &[&str] = &["available", "away", "busy", "out_of_office"];
/// The size people are advised to upload: square, and enough for every place avatars appear.
pub const AVATAR_SUGGESTED_PX: u32 = 256;
const AVATAR_MIN_PX: usize = 32;
const AVATAR_TYPES: &[(&str, imagesize::ImageType)] = &[("image/png", imagesize::ImageType::Png), ("image/jpeg", imagesize::ImageType::Jpeg), ("image/webp", imagesize::ImageType::Webp), ("image/gif", imagesize::ImageType::Gif)];

pub fn valid_email(e: &str) -> bool {
    let Some((local, domain)) = e.split_once('@') else { return false };
    !local.is_empty() && domain.contains('.') && !domain.starts_with('.') && !domain.ends_with('.') && e.len() <= 320 && !e.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';' | '"'))
}

/// Handles are what people type after `@`: lowercase letters, digits, dot, dash, underscore.
pub fn valid_handle(h: &str) -> bool {
    let b = h.as_bytes();
    (2..=39).contains(&b.len()) && (b[0].is_ascii_lowercase() || b[0].is_ascii_digit()) && b.iter().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-'))
}

/// `auto` (follow the device) or an IANA time zone name.
pub fn valid_timezone(tz: &str) -> bool {
    tz == "auto" || settings::valid_timezone(tz)
}

// ───────────────────────── identicons ─────────────────────────

/// A geometric identicon as SVG, determined entirely by `seed`. Accounts without an uploaded
/// picture show one; the seed is the account id unless the person asked for another pattern.
pub fn identicon_svg(seed: &str) -> String {
    let h = crypto::sha256(seed.as_bytes());
    let hue = u16::from_be_bytes([h[0], h[1]]) % 360;
    let hue2 = (hue + 35 + (h[2] as u16 % 90)) % 360;
    let (bg, fg) = (format!("hsl({hue} 42% 93%)"), [format!("hsl({hue} 58% 40%)"), format!("hsl({hue2} 62% 52%)")]);
    const CELL: i32 = 12;
    const PAD: i32 = 4;
    let mut out = format!("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 80 80\" width=\"80\" height=\"80\" role=\"img\" aria-label=\"Generated avatar\"><rect width=\"80\" height=\"80\" fill=\"{bg}\"/>");
    let mut drawn = 0;
    for row in 0..6i32 {
        for col in 0..3i32 {
            let b = h[3 + (row * 3 + col) as usize];
            let shape = b % 8;
            if shape < 2 {
                continue;
            }
            drawn += 1;
            let color = &fg[(b >> 7) as usize];
            for mirrored in [false, true] {
                let x = PAD + if mirrored { 5 - col } else { col } * CELL;
                let y = PAD + row * CELL;
                let (x1, y1, cx, cy) = (x + CELL, y + CELL, x + CELL / 2, y + CELL / 2);
                // Triangles lean toward the outer edge on both halves, so the figure is symmetric.
                let tri = |a: (i32, i32), b: (i32, i32), c: (i32, i32)| format!("<path d=\"M{} {}L{} {}L{} {}Z\" fill=\"{color}\"/>", a.0, a.1, b.0, b.1, c.0, c.1);
                let piece = match (shape, mirrored) {
                    (2, _) => format!("<rect x=\"{x}\" y=\"{y}\" width=\"{CELL}\" height=\"{CELL}\" fill=\"{color}\"/>"),
                    (3, false) | (7, true) => tri((x, y), (x1, y), (x, y1)),
                    (3, true) | (7, false) => tri((x, y), (x1, y), (x1, y1)),
                    (4, false) | (6, true) => tri((x1, y), (x1, y1), (x, y1)),
                    (4, true) | (6, false) => tri((x, y), (x, y1), (x1, y1)),
                    _ => format!("<path d=\"M{cx} {y}L{x1} {cy}L{cx} {y1}L{x} {cy}Z\" fill=\"{color}\"/>"),
                };
                out.push_str(&piece);
            }
        }
    }
    // A hash that selects almost no cells would look like a blank tile; give it a center mark.
    if drawn < 3 {
        let _ = write!(out, "<circle cx=\"40\" cy=\"40\" r=\"18\" fill=\"{}\"/>", fg[0]);
    }
    out.push_str("</svg>");
    out
}

// ───────────────────────── own profile ─────────────────────────

const PROFILE_SQL: &str = "SELECT person_json(u) || jsonb_build_object('email', u.email, 'theme', u.theme, 'timezone', u.timezone,
    'org_name', u.org_name, 'org_code', u.org_code, 'status_until', u.status_until, 'description', u.description,
    'has_avatar', u.avatar_blob IS NOT NULL, 'has_password', u.password_hash IS NOT NULL, 'created_at', u.created_at) FROM users u WHERE u.id = $1";

async fn me(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    let user: Value = sqlx::query_scalar(PROFILE_SQL).bind(ctx.user_id).fetch_one(&st.db).await?;
    let unread: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id = $1 AND read_at IS NULL").bind(ctx.user_id).fetch_one(&st.db).await?;
    let (method, csrf) = match &ctx.auth {
        AuthMethod::Session { csrf, .. } => ("session", Some(csrf.clone())),
        AuthMethod::Token { .. } => ("token", None),
    };
    let perms: Vec<String> = ctx.global.list().into_iter().filter(|p| ctx.token_allows(p)).collect();
    let site = settings::get(&st, "site").await;
    let profile = settings::get(&st, "profile").await;
    Ok(Json(json!({
        "user": user,
        "auth_method": method,
        "csrf": csrf,
        "permissions": perms,
        "unread_notifications": unread,
        "features": {
            "semantic_search": st.stores.embedder.is_some(),
            "local_login": st.cfg.auth.allow_local_login,
            "email": st.cfg.email.enabled(),
        },
        "site": {"name": site["name"], "announcement": site["announcement"], "timezone": site["timezone"], "support": site["support"]},
        "profile_rules": {"user_editable": profile["user_editable"], "avatar_max_kb": profile["avatar_max_kb"], "avatar_max_px": profile["avatar_max_px"], "avatar_suggested_px": AVATAR_SUGGESTED_PX},
        "agent_policy": ctx.policy.as_ref().map(AgentPolicy::to_json),
        "version": crate::VERSION,
    })))
}

/// Characters that have no place in a name or a file name: control characters (line breaks
/// among them) and the bidirectional overrides and isolates, which make text display in a
/// different order than it is stored (a file name that reads as a PDF and runs as a program).
pub fn deceptive(c: char) -> bool {
    c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn short_text(v: &Value, name: &str, max: usize) -> AppResult<String> {
    let s = v.as_str().map(str::trim).filter(|s| s.chars().count() <= max && !s.chars().any(deceptive)).ok_or_else(|| bad(format!("{name} must be text of at most {max} characters")))?;
    Ok(s.to_string())
}

fn parse_until(v: &Value) -> AppResult<Option<DateTime<Utc>>> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => {
            let t = DateTime::parse_from_rfc3339(s).map_err(|_| bad("status_until must be a date and time like 2026-03-31T17:00:00Z"))?.with_timezone(&Utc);
            if t <= Utc::now() {
                return Err(bad("status_until must be in the future"));
            }
            Ok(Some(t))
        }
        _ => Err(bad("status_until must be a date and time, or null")),
    }
}

/// Apply profile changes to `target`. `as_admin` lifts the per-field locks and allows the
/// fields only an administrator may set.
async fn apply_profile(st: &St, target: Uuid, changes: &Map<String, Value>, as_admin: bool) -> AppResult<Vec<String>> {
    let rules = settings::get(st, "profile").await;
    let editable = |field: &str| as_admin || rules["user_editable"][field].as_bool().unwrap_or(false);
    let locked = |field: &str| AppError::Forbidden(format!("{field} is managed by your administrator"));
    let mut changed = vec![];
    for (k, v) in changes {
        match k.as_str() {
            "name" => {
                if !editable("name") {
                    return Err(locked("name"));
                }
                let n = short_text(v, "name", 200)?;
                if n.is_empty() {
                    return Err(bad("name must be 1 to 200 characters"));
                }
                sqlx::query("UPDATE users SET name = $2 WHERE id = $1").bind(target).bind(n).execute(&st.db).await?;
            }
            "theme" => {
                let t = v.as_str().filter(|t| ["system", "light", "dark"].contains(t)).ok_or_else(|| bad("theme must be system, light or dark"))?;
                sqlx::query("UPDATE users SET theme = $2 WHERE id = $1").bind(target).bind(t).execute(&st.db).await?;
            }
            "timezone" => {
                let t = v.as_str().filter(|t| valid_timezone(t)).ok_or_else(|| bad("timezone must be auto or an IANA time zone name such as America/Chicago"))?;
                sqlx::query("UPDATE users SET timezone = $2 WHERE id = $1").bind(target).bind(t).execute(&st.db).await?;
            }
            "org_name" | "org_code" | "job_title" => {
                if !editable(k) {
                    return Err(locked(match k.as_str() {
                        "org_name" => "organization name",
                        "org_code" => "organization code",
                        _ => "job title",
                    }));
                }
                let val = short_text(v, k, if k == "org_code" { 40 } else { 200 })?;
                let sql = match k.as_str() {
                    "org_name" => "UPDATE users SET org_name = $2 WHERE id = $1",
                    "org_code" => "UPDATE users SET org_code = $2 WHERE id = $1",
                    _ => "UPDATE users SET job_title = $2 WHERE id = $1",
                };
                sqlx::query(sql).bind(target).bind(val).execute(&st.db).await?;
            }
            "presence" => {
                let p = v.as_str().filter(|p| PRESENCE.contains(p)).ok_or_else(|| bad("presence must be available, away, busy or out_of_office"))?;
                sqlx::query("UPDATE users SET presence = $2 WHERE id = $1").bind(target).bind(p).execute(&st.db).await?;
            }
            "status_message" => {
                let m = short_text(v, "status_message", 140)?;
                sqlx::query("UPDATE users SET status_message = $2 WHERE id = $1").bind(target).bind(m).execute(&st.db).await?;
            }
            "status_until" => {
                sqlx::query("UPDATE users SET status_until = $2 WHERE id = $1").bind(target).bind(parse_until(v)?).execute(&st.db).await?;
            }
            "handle" if as_admin => {
                let h = v.as_str().map(|h| h.trim().to_lowercase()).filter(|h| valid_handle(h)).ok_or_else(|| bad("handle must be 2 to 39 lowercase letters, digits, dots, dashes or underscores"))?;
                let taken: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE handle = $1 AND id <> $2)").bind(&h).bind(target).fetch_one(&st.db).await?;
                if taken {
                    return Err(AppError::Conflict(format!("the handle {h:?} is already taken")));
                }
                sqlx::query("UPDATE users SET handle = $2 WHERE id = $1").bind(target).bind(h).execute(&st.db).await?;
            }
            "description" if as_admin => {
                let d = v.as_str().filter(|d| d.chars().count() <= 2000).ok_or_else(|| bad("description must be text of at most 2000 characters"))?;
                sqlx::query("UPDATE users SET description = $2 WHERE id = $1").bind(target).bind(d).execute(&st.db).await?;
            }
            "owner_id" if as_admin => {
                let owner: Option<Uuid> = match v {
                    Value::Null => None,
                    Value::String(s) => Some(s.parse().map_err(|_| bad("owner_id must be a user id"))?),
                    _ => return Err(bad("owner_id must be a user id or null")),
                };
                if let Some(o) = owner {
                    let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND kind = 'human' AND active)").bind(o).fetch_one(&st.db).await?;
                    if !ok {
                        return Err(bad("the owner must be an active person"));
                    }
                }
                sqlx::query("UPDATE users SET owner_id = $2 WHERE id = $1").bind(target).bind(owner).execute(&st.db).await?;
            }
            other => return Err(bad(format!("unknown field {other:?}"))),
        }
        changed.push(k.clone());
    }
    if !changed.is_empty() {
        sqlx::query("UPDATE users SET updated_at = now() WHERE id = $1").bind(target).execute(&st.db).await?;
    }
    Ok(changed)
}

/// Profile fields an administrator may set on any account (used by `admin.rs`).
pub const ADMIN_PROFILE_FIELDS: &[&str] = &["org_name", "org_code", "job_title", "timezone", "handle", "description", "owner_id"];

pub async fn admin_apply_profile(st: &St, target: Uuid, changes: &Map<String, Value>) -> AppResult<Vec<String>> {
    apply_profile(st, target, changes, true).await
}

async fn update_profile(State(st): State<St>, ctx: Ctx, Json(b): Json<Map<String, Value>>) -> AppResult<Json<Value>> {
    let changed = apply_profile(&st, ctx.user_id, &b, false).await?;
    // Names of fields only: a status message is personal and does not belong in the audit log.
    audit::log(&st, &ctx, "user.profile_update", "user", &ctx.user_id.to_string(), json!({"changed": changed})).await;
    if changed.iter().any(|c| matches!(c.as_str(), "presence" | "status_message" | "status_until" | "name")) {
        crate::realtime::publish(&st, "directory", json!({"t": "event", "room": "directory", "kind": "user.updated", "data": {"id": ctx.user_id}})).await;
    }
    Ok(Json(sqlx::query_scalar(PROFILE_SQL).bind(ctx.user_id).fetch_one(&st.db).await?))
}

// ───────────────────────── avatars ─────────────────────────

/// Check an uploaded picture: an allowed type whose bytes really are that type, within the
/// size and dimension limits. Returns the content type to store.
pub fn check_avatar(content_type: &str, data: &[u8], max_kb: usize, max_px: usize) -> AppResult<&'static str> {
    let base = content_type.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let Some((ct, expected)) = AVATAR_TYPES.iter().find(|(t, _)| *t == base) else {
        return Err(bad("the picture must be a PNG, JPEG, WebP or GIF file"));
    };
    if data.is_empty() {
        return Err(bad("the file is empty"));
    }
    if data.len() > max_kb * 1024 {
        return Err(bad(format!("the picture is larger than the {max_kb} KB limit; a {AVATAR_SUGGESTED_PX}×{AVATAR_SUGGESTED_PX} pixel picture is plenty")));
    }
    let kind = imagesize::image_type(data).map_err(|_| bad("the file is not a readable picture"))?;
    if kind != *expected {
        return Err(bad("the file's content does not match its type"));
    }
    let size = imagesize::blob_size(data).map_err(|_| bad("the file is not a readable picture"))?;
    if size.width < AVATAR_MIN_PX || size.height < AVATAR_MIN_PX {
        return Err(bad(format!("the picture is too small; use at least {AVATAR_MIN_PX}×{AVATAR_MIN_PX} pixels ({AVATAR_SUGGESTED_PX}×{AVATAR_SUGGESTED_PX} is recommended)")));
    }
    if size.width > max_px || size.height > max_px {
        return Err(bad(format!("the picture is {}×{} pixels; the limit is {max_px}×{max_px} ({AVATAR_SUGGESTED_PX}×{AVATAR_SUGGESTED_PX} is recommended)", size.width, size.height)));
    }
    Ok(ct)
}

async fn put_avatar(State(st): State<St>, ctx: Ctx, headers: HeaderMap, body: Bytes) -> AppResult<Json<Value>> {
    let rules = settings::get(&st, "profile").await;
    let ct = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or_default();
    let ct = check_avatar(ct, &body, rules["avatar_max_kb"].as_u64().unwrap_or(512) as usize, rules["avatar_max_px"].as_u64().unwrap_or(1024) as usize)?;
    // Stored by content hash, like every other file.
    let key = crypto::sha256_hex(&body);
    st.stores.blob.put(&key, body, ct).await.map_err(AppError::Other)?;
    sqlx::query("UPDATE users SET avatar_blob = $2, avatar_type = $3, avatar_updated_at = now(), updated_at = now() WHERE id = $1").bind(ctx.user_id).bind(&key).bind(ct).execute(&st.db).await?;
    audit::log(&st, &ctx, "user.avatar_update", "user", &ctx.user_id.to_string(), json!({"content_type": ct})).await;
    Ok(Json(sqlx::query_scalar(PROFILE_SQL).bind(ctx.user_id).fetch_one(&st.db).await?))
}

#[derive(Deserialize)]
struct AvatarReset {
    /// Draw a different generated pattern instead of the one derived from the account.
    #[serde(default)]
    shuffle: bool,
}

/// Remove the uploaded picture and go back to a generated one.
async fn delete_avatar(State(st): State<St>, ctx: Ctx, Query(q): Query<AvatarReset>) -> AppResult<Json<Value>> {
    let seed = q.shuffle.then(|| format!("identicon:{}", crypto::random_hex(8)));
    sqlx::query("UPDATE users SET avatar_blob = NULL, avatar_type = $2, avatar_updated_at = now(), updated_at = now() WHERE id = $1").bind(ctx.user_id).bind(seed).execute(&st.db).await?;
    audit::log(&st, &ctx, "user.avatar_remove", "user", &ctx.user_id.to_string(), json!({"shuffle": q.shuffle})).await;
    Ok(Json(sqlx::query_scalar(PROFILE_SQL).bind(ctx.user_id).fetch_one(&st.db).await?))
}

/// Administrators can remove an unsuitable picture from any account.
pub async fn admin_remove_avatar(st: &St, target: Uuid) -> AppResult<()> {
    sqlx::query("UPDATE users SET avatar_blob = NULL, avatar_type = NULL, avatar_updated_at = now() WHERE id = $1").bind(target).execute(&st.db).await?;
    Ok(())
}

async fn get_avatar(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Response> {
    // Pictures are shown next to names everywhere, including to portal customers, so any
    // signed-in account may load them. A confined agent needs the directory permission.
    if ctx.policy.as_ref().is_some_and(|p| !p.allows("workspace.access")) {
        return Err(authz::hide(&st, &ctx, "workspace.access", &format!("avatar:{id}")));
    }
    let row = sqlx::query("SELECT avatar_blob, avatar_type FROM users WHERE id = $1").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let (blob, kind): (Option<String>, Option<String>) = (row.get("avatar_blob"), row.get("avatar_type"));
    let mut resp = match blob {
        Some(key) => {
            let data = st.stores.blob.get(&key).await.map_err(AppError::Other)?.ok_or(AppError::NotFound)?;
            let ct = AVATAR_TYPES.iter().map(|t| t.0).find(|t| Some(*t) == kind.as_deref()).unwrap_or("application/octet-stream");
            ([(header::CONTENT_TYPE, ct)], data).into_response()
        }
        None => {
            let seed = kind.as_deref().and_then(|k| k.strip_prefix("identicon:")).map(str::to_string).unwrap_or_else(|| id.to_string());
            ([(header::CONTENT_TYPE, "image/svg+xml")], identicon_svg(&seed)).into_response()
        }
    };
    let h = resp.headers_mut();
    // The address changes (`?v=`) whenever the picture does, so the browser may keep it.
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=86400"));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("default-src 'none'; style-src 'unsafe-inline'; sandbox"));
    Ok(resp)
}

// ───────────────────────── directory and profiles ─────────────────────────

#[derive(Deserialize)]
struct DirectoryQuery {
    q: Option<String>,
    kind: Option<String>,
    #[serde(flatten)]
    paging: Paging,
}

/// People directory for pickers and mentions. Requires workspace access.
async fn directory(State(st): State<St>, ctx: Ctx, Query(q): Query<DirectoryQuery>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    let like = q.q.as_deref().map(str::trim).filter(|s| !s.is_empty()).map(|s| format!("%{}%", crate::storage::like_escape(s)));
    let limit = q.paging.limit.unwrap_or(1000).clamp(1, 2000);
    Ok(Json(
        sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(t.p ORDER BY t.name), '[]'::jsonb) FROM (
               SELECT person_json(u) || jsonb_build_object('email', u.email, 'org_name', u.org_name, 'org_code', u.org_code) AS p, u.name FROM users u
               WHERE u.active AND ($1::text IS NULL OR u.name ILIKE $1 OR u.handle ILIKE $1 OR u.email ILIKE $1) AND ($2::text IS NULL OR u.kind = $2)
               ORDER BY u.name LIMIT $3) t",
        )
        .bind(&like)
        .bind(&q.kind)
        .bind(limit)
        .fetch_one(&st.db)
        .await?,
    ))
}

/// One account's public profile, with the team positions it holds and has held.
async fn user_profile(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    if id != ctx.user_id {
        authz::require_global(&st, &ctx, "workspace.access")?;
    }
    let mut user: Value = sqlx::query_scalar(
        "SELECT person_json(u) || jsonb_build_object('email', u.email, 'org_name', u.org_name, 'org_code', u.org_code, 'timezone', u.timezone,
            'status_until', u.status_until, 'description', u.description, 'created_at', u.created_at,
            'owner', (SELECT person_json(o) FROM users o WHERE o.id = u.owner_id)) FROM users u WHERE u.id = $1",
    )
    .bind(id)
    .fetch_optional(&st.db)
    .await?
    .ok_or(AppError::NotFound)?;
    user["positions"] = crate::teams::positions_of(&st, id).await?;
    Ok(Json(user))
}

// ───────────────────────── AI agent rules ─────────────────────────

async fn agent_row(st: &St, id: Uuid) -> AppResult<sqlx::postgres::PgRow> {
    let row = sqlx::query("SELECT kind, name, agent_policy FROM users WHERE id = $1").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    if row.get::<String, _>("kind") != "agent" {
        return Err(bad("this account is a person; rules apply to AI agent accounts only"));
    }
    Ok(row)
}

/// The rules for one agent, with what they add up to: the permissions of the agent's roles
/// that the rules let through.
async fn get_agent_policy(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.agents")?;
    let row = agent_row(&st, id).await?;
    let policy = AgentPolicy::from_json(row.get::<Option<Value>, _>("agent_policy").as_ref());
    let role_perms = authz::load_perms(&st.db, id, None).await?;
    let scoped: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(jsonb_build_object('scope_type', b.scope_type, 'scope_id', b.scope_id, 'role', r.key, 'role_name', r.name, 'permissions', r.permissions,
            'label', CASE b.scope_type WHEN 'project' THEN (SELECT key || ' · ' || name FROM projects WHERE id = b.scope_id) WHEN 'space' THEN (SELECT key || ' · ' || name FROM spaces WHERE id = b.scope_id) ELSE 'Everywhere' END)), '[]'::jsonb)
         FROM role_bindings b JOIN roles r ON r.id = b.role_id
         WHERE b.user_id = $1 OR b.group_id IN (SELECT group_id FROM group_members WHERE user_id = $1)",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await?;
    Ok(Json(json!({
        "policy": policy.to_json(),
        "effective_global": policy.narrow(role_perms.clone()).list(),
        "role_global": role_perms.list(),
        "role_bindings": scoped,
        "catalog": authz::PERMISSIONS.iter().map(|(k, s, d)| json!({"key": k, "scope": s, "description": d})).collect::<Vec<_>>(),
    })))
}

async fn put_agent_policy(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Value>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.agents")?;
    let row = agent_row(&st, id).await?;
    let o = b.as_object().ok_or_else(|| bad("the policy must be an object"))?;
    if let Some(k) = o.keys().find(|k| !["mode", "permissions", "all_projects", "projects", "all_spaces", "spaces", "rate_limit_per_minute"].contains(&k.as_str())) {
        return Err(bad(format!("unknown policy field {k:?}")));
    }
    let mode = o.get("mode").and_then(Value::as_str).unwrap_or("restricted");
    if !["restricted", "unrestricted"].contains(&mode) {
        return Err(bad("mode must be restricted or unrestricted"));
    }
    let wanted: Vec<String> = match o.get("permissions") {
        None => vec![],
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| bad("permissions must be a list of permission keys"))?,
    };
    for p in &wanted {
        if p == "*" || !authz::is_known_permission(p) {
            return Err(bad(format!("unknown permission {p:?}; list each permission by its key")));
        }
        // An administrator cannot let an agent do, anywhere, what the administrator may not do.
        // Permissions inside a project or space still need a role there, which is granted separately.
        let global = authz::PERMISSIONS.iter().any(|(k, s, _)| k == p && *s == "global");
        if global && !ctx.has(p) {
            return Err(AppError::Forbidden(format!("you cannot allow {p:?} because you do not hold it yourself")));
        }
    }
    if mode == "unrestricted" && !ctx.has("admin.security") {
        return Err(AppError::Forbidden("only an administrator who manages sign-in security can lift all rules from an agent".into()));
    }
    let ids = |k: &str| -> AppResult<Vec<Uuid>> {
        match o.get(k) {
            None => Ok(vec![]),
            Some(v) => serde_json::from_value(v.clone()).map_err(|_| bad(format!("{k} must be a list of ids"))),
        }
    };
    let (projects, spaces) = (ids("projects")?, ids("spaces")?);
    let known_p: i64 = sqlx::query_scalar("SELECT count(*) FROM projects WHERE id = ANY($1)").bind(&projects).fetch_one(&st.db).await?;
    let known_s: i64 = sqlx::query_scalar("SELECT count(*) FROM spaces WHERE id = ANY($1)").bind(&spaces).fetch_one(&st.db).await?;
    let distinct = |v: &[Uuid]| v.iter().collect::<std::collections::HashSet<_>>().len() as i64;
    if known_p != distinct(&projects) || known_s != distinct(&spaces) {
        return Err(bad("the policy names a project or space that does not exist"));
    }
    let flag = |k: &str| -> AppResult<bool> {
        match o.get(k) {
            None => Ok(false),
            Some(Value::Bool(b)) => Ok(*b),
            Some(_) => Err(bad(format!("{k} must be true or false"))),
        }
    };
    let rate = match o.get("rate_limit_per_minute") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().filter(|n| (1..=100_000).contains(n)).ok_or_else(|| bad("rate_limit_per_minute must be between 1 and 100000"))? as u32),
    };
    let policy = AgentPolicy { unrestricted: mode == "unrestricted", permissions: wanted.into_iter().collect(), all_projects: flag("all_projects")?, projects: projects.into_iter().collect(), all_spaces: flag("all_spaces")?, spaces: spaces.into_iter().collect(), rate_limit: rate };
    let before = AgentPolicy::from_json(row.get::<Option<Value>, _>("agent_policy").as_ref());
    sqlx::query("UPDATE users SET agent_policy = $2, updated_at = now() WHERE id = $1").bind(id).bind(policy.to_json()).execute(&st.db).await?;
    audit::log(&st, &ctx, "agent.policy_update", "user", &id.to_string(), json!({"agent": row.get::<String, _>("name"), "before": before.to_json(), "after": policy.to_json()})).await;
    Ok(Json(json!({"policy": policy.to_json()})))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/auth/me", "Auth", "Current account, permissions, site settings and CSRF token", get(me))
        .route("PATCH", "/api/v1/auth/me", "Auth", "Update own profile: name, theme, time zone, organization, job title, status", patch(update_profile))
        .route("PUT", "/api/v1/auth/me/avatar", "Auth", "Upload a profile picture (raw image body)", put(put_avatar))
        .route("DELETE", "/api/v1/auth/me/avatar", "Auth", "Remove the profile picture and use a generated one (?shuffle=true for a new pattern)", delete(delete_avatar))
        .route("GET", "/api/v1/users", "Directory", "People and AI agents, for pickers and mentions", get(directory))
        .route("GET", "/api/v1/users/{id}", "Directory", "An account's profile and team positions", get(user_profile))
        .route("GET", "/api/v1/users/{id}/avatar", "Directory", "An account's picture, or its generated identicon", get(get_avatar))
        .route("GET", "/api/v1/admin/users/{id}/agent-policy", "Admin: agents", "What an AI agent account is allowed to do", get(get_agent_policy))
        .route("PUT", "/api/v1/admin/users/{id}/agent-policy", "Admin: agents", "Set what an AI agent account is allowed to do", put(put_agent_policy))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_and_time_zones() {
        for ok in ["ada", "ada.lovelace", "a1", "build-bot_2", "9lives"] {
            assert!(valid_handle(ok), "{ok}");
        }
        for bad in ["", "a", "Ada", "-ada", ".ada", "ada lovelace", "ada@x", "a".repeat(40).as_str(), "ada/../x", "ädä"] {
            assert!(!valid_handle(bad), "{bad}");
        }
        assert!(valid_timezone("auto") && valid_timezone("UTC") && valid_timezone("America/Los_Angeles") && valid_timezone("Asia/Kolkata"));
        assert!(!valid_timezone("") && !valid_timezone("Central") && !valid_timezone("America/Atlantis") && !valid_timezone("../../etc/passwd"));
    }

    #[test]
    fn email_addresses() {
        assert!(valid_email("ada@example.org") && valid_email("a.b+c@sub.example.org"));
        for bad in ["", "ada", "@example.org", "ada@localhost", "ada@.org", "ada@example.", "a b@example.org", "ada@example.org\r\nBcc: x@y.z", "<ada@example.org>", "a@b.c,d@e.f"] {
            assert!(!valid_email(bad), "{bad:?}");
        }
    }

    #[test]
    fn identicons_are_deterministic_distinct_and_plain_svg() {
        let a = identicon_svg("5f0c1a52-3a8e-4b7e-9d0a-111111111111");
        assert_eq!(a, identicon_svg("5f0c1a52-3a8e-4b7e-9d0a-111111111111"));
        assert_ne!(a, identicon_svg("5f0c1a52-3a8e-4b7e-9d0a-111111111112"));
        assert!(a.starts_with("<svg ") && a.ends_with("</svg>"));
        for forbidden in ["<script", "href", "onload", "<foreignObject", "<image", "url("] {
            assert!(!a.contains(forbidden), "{forbidden}");
        }
        // Seeds are hashed, so hostile text cannot reach the markup.
        let hostile = identicon_svg("\"><script>alert(1)</script>");
        assert!(!hostile.contains("script") && !hostile.contains("alert"));
        // Every seed yields a visible figure.
        for i in 0..500 {
            let svg = identicon_svg(&format!("seed-{i}"));
            assert!(svg.matches("<path").count() + svg.matches("<rect").count() + svg.matches("<circle").count() >= 2, "seed-{i} is blank");
        }
    }

    fn png(width: u32, height: u32) -> Vec<u8> {
        let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
        v.extend_from_slice(&width.to_be_bytes());
        v.extend_from_slice(&height.to_be_bytes());
        v.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        v
    }

    #[test]
    fn avatar_limits() {
        assert_eq!(check_avatar("image/png", &png(256, 256), 512, 1024).unwrap(), "image/png");
        assert_eq!(check_avatar("image/png; charset=binary", &png(1024, 1024), 512, 1024).unwrap(), "image/png");
        let msg = |r: AppResult<&str>| r.unwrap_err().public_message();
        assert!(msg(check_avatar("image/png", &png(1025, 100), 512, 1024)).contains("limit is 1024×1024"));
        assert!(msg(check_avatar("image/png", &png(16, 16), 512, 1024)).contains("too small"));
        assert!(msg(check_avatar("image/svg+xml", b"<svg/>", 512, 1024)).contains("PNG, JPEG, WebP or GIF"));
        assert!(msg(check_avatar("text/html", b"<html>", 512, 1024)).contains("PNG, JPEG, WebP or GIF"));
        assert!(msg(check_avatar("image/jpeg", &png(100, 100), 512, 1024)).contains("does not match"), "a PNG labeled as JPEG is refused");
        assert!(msg(check_avatar("image/png", b"GIF89a not a png at all.....", 512, 1024)).contains("does not match"));
        assert!(msg(check_avatar("image/png", b"<script>alert(1)</script>", 512, 1024)).contains("not a readable picture"));
        let mut big = png(100, 100);
        big.resize(600 * 1024, 0);
        assert!(msg(check_avatar("image/png", &big, 512, 1024)).contains("512 KB limit"));
        assert!(msg(check_avatar("image/png", b"", 512, 1024)).contains("empty"));
    }
}
