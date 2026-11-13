//! Authentication: browser sessions, API tokens, local sign-in, profile and token management.
//!
//! * Sessions: 256-bit random token in an HttpOnly, SameSite=Lax cookie (`__Host-` prefixed and
//!   Secure when served over HTTPS). Only a SHA-256 of the token is stored. Idle and absolute timeouts.
//! * CSRF: cookie-authenticated unsafe requests must carry `X-Gantry-CSRF` (per-session token) and,
//!   when an `Origin` header is present, it must match the server's origin. Enforced in `http.rs`.
//! * API tokens: `Authorization: Bearer gty_<prefix>_<secret>`; SHA-256 of the secret stored;
//!   scopes `read` < `write` < `admin`; optional expiry; revocable. Used by scripts and AI agents.
//! * Local passwords: Argon2id, length-based policy, per-account lockout and per-address throttling.

use crate::authz::{self, Perms};
use crate::error::{bad, AppError, AppResult};
use crate::http::{Api, ReqInfo};
use crate::{audit, crypto, AppState, St};
use axum::extract::{FromRequestParts, Path, State};
use axum::http::{header, request::Parts, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::Json;
use chrono::{DateTime, Utc};
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

#[derive(Clone, Debug)]
pub enum AuthMethod {
    Session { id: Uuid, csrf: String },
    Token { id: Uuid, scopes: Vec<String> },
}

/// The authenticated principal for one request. Extracting it in a handler makes the route private.
#[derive(Clone, Debug)]
pub struct Ctx {
    pub user_id: Uuid,
    pub name: String,
    pub email: String,
    /// `human` or `agent`.
    pub kind: String,
    pub auth: AuthMethod,
    /// Global permissions, already narrowed by the agent policy when there is one.
    pub global: Perms,
    /// Present on AI agent accounts only: the administrator's rules for this agent.
    pub policy: Option<authz::AgentPolicy>,
    pub req: ReqInfo,
}

/// Global permissions and policy for an account row (`kind`, `agent_policy`).
async fn principal(st: &AppState, user_id: Uuid, kind: &str, policy: Option<Value>) -> AppResult<(Perms, Option<authz::AgentPolicy>)> {
    let perms = authz::load_perms(&st.db, user_id, None).await?;
    if kind == "agent" {
        let policy = authz::AgentPolicy::from_json(policy.as_ref());
        Ok((policy.narrow(perms), Some(policy)))
    } else {
        Ok((perms, None))
    }
}

const READ_PERMS: &[&str] = &["workspace.access", "project.view", "space.view"];

impl Ctx {
    /// 0 = none, 1 = read, 2 = write, 3 = admin. Sessions have full level.
    pub fn token_level(&self) -> u8 {
        match &self.auth {
            AuthMethod::Session { .. } => 3,
            AuthMethod::Token { scopes, .. } => {
                if scopes.iter().any(|s| s == "admin") {
                    3
                } else if scopes.iter().any(|s| s == "write") {
                    2
                } else if scopes.iter().any(|s| s == "read") {
                    1
                } else {
                    0
                }
            }
        }
    }

    /// Whether the token's scopes permit exercising `permission` (always true for sessions).
    pub fn token_allows(&self, permission: &str) -> bool {
        let need = if permission.starts_with("admin.") || permission.starts_with("scim.") {
            3
        } else if READ_PERMS.contains(&permission) {
            1
        } else {
            2
        };
        self.token_level() >= need
    }

    /// Global permission check, narrowed by token scopes.
    pub fn has(&self, permission: &str) -> bool {
        self.global.has(permission) && self.token_allows(permission)
    }

    pub fn can_write(&self) -> bool {
        self.token_level() >= 2
    }

    pub fn is_session(&self) -> bool {
        matches!(self.auth, AuthMethod::Session { .. })
    }

    pub fn is_agent(&self) -> bool {
        self.kind == "agent"
    }
}

impl<S: Send + Sync> FromRequestParts<S> for Ctx {
    type Rejection = AppError;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, AppError> {
        parts.extensions.get::<Ctx>().cloned().ok_or(AppError::Unauthorized)
    }
}

/// The principal if there is one. For handlers that must shape their own "unauthenticated" answer.
pub struct MaybeCtx(pub Option<Ctx>);

impl<S: Send + Sync> FromRequestParts<S> for MaybeCtx {
    type Rejection = std::convert::Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Self::Rejection> {
        Ok(MaybeCtx(parts.extensions.get::<Ctx>().cloned()))
    }
}

pub fn cookie_name(st: &AppState) -> &'static str {
    if st.cfg.secure_cookies() {
        "__Host-gantry_session"
    } else {
        "gantry_session"
    }
}

fn read_cookie<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers.get_all(header::COOKIE).iter().filter_map(|v| v.to_str().ok()).flat_map(|v| v.split(';')).find_map(|kv| {
        let (k, v) = kv.trim().split_once('=')?;
        (k == name).then_some(v)
    })
}

fn session_cookie(st: &AppState, value: &str, max_age_secs: i64) -> HeaderValue {
    let secure = if st.cfg.secure_cookies() { "; Secure" } else { "" };
    HeaderValue::from_str(&format!("{}={value}; Path=/; HttpOnly; SameSite=Lax; Max-Age={max_age_secs}{secure}", cookie_name(st))).expect("cookie is ASCII")
}

/// Resolve the caller. `Ok(None)` = anonymous. A presented but invalid bearer token is an error,
/// so a client with a bad token never silently falls back to anonymous access.
pub async fn authenticate(st: &AppState, headers: &HeaderMap, info: &ReqInfo) -> Result<Option<Ctx>, AppError> {
    if let Some(v) = headers.get(header::AUTHORIZATION) {
        let v = v.to_str().map_err(|_| AppError::Unauthorized)?;
        let token = v.strip_prefix("Bearer ").or_else(|| v.strip_prefix("bearer ")).ok_or(AppError::Unauthorized)?;
        return match token_ctx(st, token.trim(), info).await? {
            Some(ctx) => Ok(Some(ctx)),
            None => {
                audit::security(st, "auth.token_rejected", "warning", None, info, json!({"prefix": crypto::parse_api_token(token.trim()).map(|p| p.0)})).await;
                Err(AppError::Unauthorized)
            }
        };
    }
    let Some(raw) = read_cookie(headers, cookie_name(st)) else { return Ok(None) };
    let row = sqlx::query(
        "SELECT s.id, s.csrf, u.id AS user_id, u.name, u.email, u.kind, u.agent_policy FROM sessions s JOIN users u ON u.id = s.user_id
         WHERE s.token_hash = $1 AND s.expires_at > now() AND s.last_seen_at > now() - make_interval(mins => $2) AND u.active",
    )
    .bind(crypto::sha256(raw.as_bytes()))
    .bind(st.cfg.auth.session_idle_minutes as i32)
    .fetch_optional(&st.db)
    .await?;
    let Some(r) = row else { return Ok(None) };
    let sid: Uuid = r.get("id");
    sqlx::query("UPDATE sessions SET last_seen_at = now() WHERE id = $1 AND last_seen_at < now() - interval '1 minute'").bind(sid).execute(&st.db).await?;
    let user_id: Uuid = r.get("user_id");
    let kind: String = r.get("kind");
    let (global, policy) = principal(st, user_id, &kind, r.get("agent_policy")).await?;
    Ok(Some(Ctx { user_id, name: r.get("name"), email: r.get("email"), kind, auth: AuthMethod::Session { id: sid, csrf: r.get("csrf") }, global, policy, req: info.clone() }))
}

async fn token_ctx(st: &AppState, token: &str, info: &ReqInfo) -> Result<Option<Ctx>, AppError> {
    let Some((prefix, secret)) = crypto::parse_api_token(token) else { return Ok(None) };
    let row = sqlx::query(
        "SELECT t.id, t.secret_hash, t.scopes, u.id AS user_id, u.name, u.email, u.kind, u.agent_policy FROM api_tokens t JOIN users u ON u.id = t.user_id
         WHERE t.prefix = $1 AND t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > now()) AND u.active",
    )
    .bind(prefix)
    .fetch_optional(&st.db)
    .await?;
    let Some(r) = row else { return Ok(None) };
    if !crypto::ct_eq(&crypto::sha256(secret.as_bytes()), &r.get::<Vec<u8>, _>("secret_hash")) {
        return Ok(None);
    }
    let tid: Uuid = r.get("id");
    sqlx::query("UPDATE api_tokens SET last_used_at = now() WHERE id = $1 AND (last_used_at IS NULL OR last_used_at < now() - interval '1 minute')").bind(tid).execute(&st.db).await?;
    let user_id: Uuid = r.get("user_id");
    let kind: String = r.get("kind");
    let (global, policy) = principal(st, user_id, &kind, r.get("agent_policy")).await?;
    Ok(Some(Ctx { user_id, name: r.get("name"), email: r.get("email"), kind, auth: AuthMethod::Token { id: tid, scopes: r.get("scopes") }, global, policy, req: info.clone() }))
}

/// A principal for work done on behalf of an integration (for example a Slack slash command).
/// It has the user's own permissions, limited to read and write: never administration.
pub async fn service_ctx(st: &AppState, user_id: Uuid, info: &ReqInfo) -> AppResult<Option<Ctx>> {
    let Some(r) = sqlx::query("SELECT name, email, kind, agent_policy FROM users WHERE id = $1 AND active").bind(user_id).fetch_optional(&st.db).await? else { return Ok(None) };
    let kind: String = r.get("kind");
    let (global, policy) = principal(st, user_id, &kind, r.get("agent_policy")).await?;
    Ok(Some(Ctx { user_id, name: r.get("name"), email: r.get("email"), kind, auth: AuthMethod::Token { id: Uuid::nil(), scopes: vec!["read".into(), "write".into()] }, global, policy, req: info.clone() }))
}

/// Create a session row and return the `Set-Cookie` header value.
pub async fn start_session(st: &AppState, user_id: Uuid, method: &str, info: &ReqInfo) -> AppResult<HeaderValue> {
    let token = crypto::random_token(32);
    let abs_secs = st.cfg.auth.session_absolute_hours * 3600;
    sqlx::query("INSERT INTO sessions (token_hash, user_id, csrf, auth_method, ip, user_agent, expires_at) VALUES ($1,$2,$3,$4,$5,$6, now() + make_interval(secs => $7))")
        .bind(crypto::sha256(token.as_bytes()))
        .bind(user_id)
        .bind(crypto::random_token(24))
        .bind(method)
        .bind(&info.ip)
        .bind(info.user_agent.chars().take(300).collect::<String>())
        .bind(abs_secs as f64)
        .execute(&st.db)
        .await?;
    sqlx::query("UPDATE users SET last_login_at = now(), failed_logins = 0, locked_until = NULL WHERE id = $1").bind(user_id).execute(&st.db).await?;
    Ok(session_cookie(st, &token, abs_secs))
}

pub fn validate_password(st: &AppState, password: &str) -> AppResult<()> {
    let min = st.cfg.auth.password_min_length;
    if password.chars().count() < min {
        return Err(bad(format!("password must be at least {min} characters")));
    }
    if password.len() > 1024 {
        return Err(bad("password is too long"));
    }
    Ok(())
}

pub async fn hash_password_async(password: String) -> AppResult<String> {
    tokio::task::spawn_blocking(move || crypto::hash_password(&password)).await.map_err(|e| AppError::Other(e.into()))
}

#[derive(Deserialize)]
struct LoginBody {
    email: String,
    password: String,
}

async fn login(State(st): State<St>, info: ReqInfo, Json(b): Json<LoginBody>) -> AppResult<Response> {
    if !st.cfg.auth.allow_local_login {
        return Err(AppError::Forbidden("password sign-in is disabled; use single sign-on".into()));
    }
    // Only failed attempts count toward the per-address limit. A whole office usually shares
    // one address, and successful sign-ins at the start of a day must not lock everyone out.
    let throttle_key = format!("login-fail:{}", info.ip);
    let failures: i64 = st.stores.kv.get(&throttle_key).await.map_err(AppError::Other)?.and_then(|v| v.parse().ok()).unwrap_or(0);
    if failures >= st.cfg.auth.login_attempts_per_ip {
        audit::security(&st, "auth.login_throttled", "warning", None, &info, json!({"failed_attempts": failures})).await;
        return Err(AppError::RateLimited);
    }
    let email = b.email.trim().to_lowercase();
    let row = sqlx::query("SELECT id, password_hash, active, locked_until FROM users WHERE lower(email) = $1 AND kind = 'human'").bind(&email).fetch_optional(&st.db).await?;
    let (user_id, hash, usable) = match &row {
        Some(r) => {
            let locked: Option<DateTime<Utc>> = r.get("locked_until");
            let hash: Option<String> = r.get("password_hash");
            let usable = r.get::<bool, _>("active") && hash.is_some() && locked.map(|t| t <= Utc::now()).unwrap_or(true);
            (Some(r.get::<Uuid, _>("id")), hash.unwrap_or_else(|| crypto::dummy_hash().to_string()), usable)
        }
        None => (None, crypto::dummy_hash().to_string(), false),
    };
    let password = b.password;
    let verified = tokio::task::spawn_blocking(move || crypto::verify_password(&password, &hash)).await.map_err(|e| AppError::Other(e.into()))?;
    if !(verified && usable) {
        st.stores.kv.incr(&throttle_key, 300).await.map_err(AppError::Other)?;
        let mut locked_now = false;
        if let Some(uid) = user_id {
            let fails: i32 = sqlx::query_scalar("UPDATE users SET failed_logins = failed_logins + 1 WHERE id = $1 RETURNING failed_logins").bind(uid).fetch_one(&st.db).await?;
            if fails >= st.cfg.auth.lockout_threshold {
                sqlx::query("UPDATE users SET locked_until = now() + make_interval(mins => $2), failed_logins = 0 WHERE id = $1").bind(uid).bind(st.cfg.auth.lockout_minutes as i32).execute(&st.db).await?;
                locked_now = true;
            }
        }
        audit::security(&st, if locked_now { "auth.lockout" } else { "auth.login_failed" }, "warning", user_id.map(|u| (u, email.as_str())), &info, json!({"email": email})).await;
        audit::log_system(&st, Some(&info), &email, "auth.login", "user", &user_id.map(|u| u.to_string()).unwrap_or_default(), "failure", json!({"method": "password"})).await;
        return Err(AppError::Unauthorized);
    }
    let uid = user_id.expect("verified implies a user");
    let cookie = start_session(&st, uid, "password", &info).await?;
    audit::log_system(&st, Some(&info), &email, "auth.login", "user", &uid.to_string(), "success", json!({"method": "password"})).await;
    let mut resp = Json(json!({"ok": true})).into_response();
    resp.headers_mut().insert(header::SET_COOKIE, cookie);
    Ok(resp)
}

async fn logout(State(st): State<St>, ctx: Ctx) -> AppResult<Response> {
    if let AuthMethod::Session { id, .. } = &ctx.auth {
        sqlx::query("DELETE FROM sessions WHERE id = $1").bind(id).execute(&st.db).await?;
        audit::log(&st, &ctx, "auth.logout", "user", &ctx.user_id.to_string(), json!({})).await;
    }
    let mut resp = StatusCode::NO_CONTENT.into_response();
    resp.headers_mut().insert(header::SET_COOKIE, session_cookie(&st, "", 0));
    Ok(resp)
}

#[derive(Deserialize)]
struct PasswordBody {
    current: String,
    new: String,
}

async fn change_password(State(st): State<St>, ctx: Ctx, Json(b): Json<PasswordBody>) -> AppResult<StatusCode> {
    let AuthMethod::Session { id: sid, .. } = &ctx.auth else {
        return Err(AppError::Forbidden("passwords can only be changed from a signed-in browser session".into()));
    };
    validate_password(&st, &b.new)?;
    let hash: Option<String> = sqlx::query_scalar("SELECT password_hash FROM users WHERE id = $1").bind(ctx.user_id).fetch_one(&st.db).await?;
    let hash = hash.ok_or_else(|| bad("this account signs in through single sign-on and has no password"))?;
    let current = b.current;
    if !tokio::task::spawn_blocking(move || crypto::verify_password(&current, &hash)).await.map_err(|e| AppError::Other(e.into()))? {
        audit::security(&st, "auth.password_change_failed", "warning", Some((ctx.user_id, &ctx.email)), &ctx.req, json!({})).await;
        return Err(bad("current password is incorrect"));
    }
    let new_hash = hash_password_async(b.new).await?;
    sqlx::query("UPDATE users SET password_hash = $2, updated_at = now() WHERE id = $1").bind(ctx.user_id).bind(new_hash).execute(&st.db).await?;
    sqlx::query("DELETE FROM sessions WHERE user_id = $1 AND id <> $2").bind(ctx.user_id).bind(sid).execute(&st.db).await?;
    audit::log(&st, &ctx, "user.password_change", "user", &ctx.user_id.to_string(), json!({"other_sessions_revoked": true})).await;
    Ok(StatusCode::NO_CONTENT)
}

pub fn validate_scopes(scopes: &[String]) -> AppResult<()> {
    if scopes.is_empty() || scopes.iter().any(|s| !["read", "write", "admin"].contains(&s.as_str())) {
        return Err(bad("scopes must be a non-empty subset of read, write, admin"));
    }
    Ok(())
}

/// Mint an API token for `owner`. The full token is returned once and never stored.
pub async fn mint_token(st: &AppState, ctx: &Ctx, owner: Uuid, name: &str, scopes: &[String], expires_in_days: Option<i64>) -> AppResult<Value> {
    validate_scopes(scopes)?;
    if name.trim().is_empty() || name.len() > 200 {
        return Err(bad("name must be 1 to 200 characters"));
    }
    if let Some(d) = expires_in_days {
        if !(1..=3650).contains(&d) {
            return Err(bad("expires_in_days must be between 1 and 3650"));
        }
    }
    let t = crypto::new_api_token();
    let row = sqlx::query(
        "INSERT INTO api_tokens (user_id, name, prefix, secret_hash, scopes, created_by, expires_at)
         VALUES ($1,$2,$3,$4,$5,$6, CASE WHEN $7::int IS NULL THEN NULL ELSE now() + make_interval(days => $7) END) RETURNING id, expires_at",
    )
    .bind(owner)
    .bind(name.trim())
    .bind(&t.prefix)
    .bind(&t.secret_hash)
    .bind(scopes)
    .bind(ctx.user_id)
    .bind(expires_in_days.map(|d| d as i32))
    .fetch_one(&st.db)
    .await?;
    let id: Uuid = row.get("id");
    audit::log(st, ctx, "token.create", "api_token", &id.to_string(), json!({"owner": owner, "name": name.trim(), "scopes": scopes, "expires_in_days": expires_in_days})).await;
    Ok(json!({"id": id, "name": name.trim(), "prefix": t.prefix, "scopes": scopes, "expires_at": row.get::<Option<DateTime<Utc>>, _>("expires_at"), "token": t.full}))
}

pub const TOKEN_LIST_SQL: &str = "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.created_at DESC), '[]'::jsonb) FROM (
    SELECT id, name, prefix, scopes, created_at, expires_at, last_used_at, revoked_at FROM api_tokens WHERE user_id = $1) t";

async fn list_tokens(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    Ok(Json(sqlx::query_scalar(TOKEN_LIST_SQL).bind(ctx.user_id).fetch_one(&st.db).await?))
}

#[derive(Deserialize)]
pub struct TokenBody {
    pub name: String,
    pub scopes: Vec<String>,
    pub expires_in_days: Option<i64>,
}

async fn create_token(State(st): State<St>, ctx: Ctx, Json(b): Json<TokenBody>) -> AppResult<(StatusCode, Json<Value>)> {
    if !ctx.is_session() {
        return Err(AppError::Forbidden("tokens can only be created from a signed-in browser session".into()));
    }
    if b.scopes.iter().any(|s| s == "admin") && !ctx.global.has("admin.access") {
        return Err(AppError::Forbidden("only administrators can create tokens with the admin scope".into()));
    }
    Ok((StatusCode::CREATED, Json(mint_token(&st, &ctx, ctx.user_id, &b.name, &b.scopes, b.expires_in_days).await?)))
}

async fn revoke_token(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let owner: Uuid = sqlx::query_scalar("SELECT user_id FROM api_tokens WHERE id = $1").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    if owner != ctx.user_id {
        let agent: bool = sqlx::query_scalar("SELECT kind = 'agent' FROM users WHERE id = $1").bind(owner).fetch_one(&st.db).await?;
        if !(ctx.has("admin.security") || (agent && ctx.has("admin.agents"))) {
            return Err(authz::hide(&st, &ctx, "admin.security", &format!("api_token:{id}")));
        }
    } else if !ctx.can_write() {
        return Err(authz::deny(&st, &ctx, "token.revoke", &format!("api_token:{id}")));
    }
    sqlx::query("UPDATE api_tokens SET revoked_at = now() WHERE id = $1 AND revoked_at IS NULL").bind(id).execute(&st.db).await?;
    audit::log(&st, &ctx, "token.revoke", "api_token", &id.to_string(), json!({"owner": owner})).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The same sign-in with permissions read again from the database. `None` when the account
/// is gone or inactive. Used by long-lived connections, which must not keep stale permissions.
pub async fn refresh_ctx(st: &AppState, ctx: &Ctx) -> AppResult<Option<Ctx>> {
    let Some(r) = sqlx::query("SELECT name, email, kind, agent_policy FROM users WHERE id = $1 AND active").bind(ctx.user_id).fetch_optional(&st.db).await? else { return Ok(None) };
    let kind: String = r.get("kind");
    let (global, policy) = principal(st, ctx.user_id, &kind, r.get("agent_policy")).await?;
    Ok(Some(Ctx { user_id: ctx.user_id, name: r.get("name"), email: r.get("email"), kind, auth: ctx.auth.clone(), global, policy, req: ctx.req.clone() }))
}

pub fn routes(api: Api) -> Api {
    api.route("POST", "/api/v1/auth/login", "Auth", "Sign in with email and password", post(login))
        .public()
        .route("POST", "/api/v1/auth/logout", "Auth", "End the current session", post(logout))
        .route("POST", "/api/v1/auth/password", "Auth", "Change own password", post(change_password))
        .route("GET", "/api/v1/tokens", "Tokens", "List own API tokens", get(list_tokens))
        .route("POST", "/api/v1/tokens", "Tokens", "Create an API token (shown once)", post(create_token))
        .route("DELETE", "/api/v1/tokens/{id}", "Tokens", "Revoke an API token", delete(revoke_token))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(auth: AuthMethod, perms: &[&str]) -> Ctx {
        Ctx { user_id: Uuid::nil(), name: "n".into(), email: "e@x".into(), kind: "human".into(), auth, global: Perms(perms.iter().map(|s| s.to_string()).collect()), policy: None, req: ReqInfo::default() }
    }

    #[test]
    fn token_scopes_narrow_permissions() {
        let read = ctx(AuthMethod::Token { id: Uuid::nil(), scopes: vec!["read".into()] }, &["*"]);
        assert!(read.has("project.view"));
        assert!(!read.has("project.create"));
        assert!(!read.has("admin.users"));
        assert!(!read.can_write());

        let write = ctx(AuthMethod::Token { id: Uuid::nil(), scopes: vec!["read".into(), "write".into()] }, &["*"]);
        assert!(write.has("project.create") && write.can_write());
        assert!(!write.has("admin.users"), "write scope must not unlock administration");

        let admin = ctx(AuthMethod::Token { id: Uuid::nil(), scopes: vec!["admin".into()] }, &["*"]);
        assert!(admin.has("admin.users") && admin.has("scim.provision"));
    }

    #[test]
    fn scopes_never_add_permissions_the_user_lacks() {
        let t = ctx(AuthMethod::Token { id: Uuid::nil(), scopes: vec!["admin".into()] }, &["workspace.access"]);
        assert!(!t.has("admin.users"));
        let s = ctx(AuthMethod::Session { id: Uuid::nil(), csrf: "c".into() }, &["workspace.access"]);
        assert!(s.has("workspace.access") && !s.has("admin.access"));
    }

    #[test]
    fn cookie_parsing() {
        let mut h = HeaderMap::new();
        h.insert(header::COOKIE, HeaderValue::from_static("a=1; gantry_session=abc; b=2"));
        assert_eq!(read_cookie(&h, "gantry_session"), Some("abc"));
        assert_eq!(read_cookie(&h, "gantry_sessio"), None);
        assert_eq!(read_cookie(&HeaderMap::new(), "gantry_session"), None);
    }

    #[test]
    fn scope_validation() {
        assert!(validate_scopes(&["read".into(), "write".into()]).is_ok());
        assert!(validate_scopes(&[]).is_err());
        assert!(validate_scopes(&["root".into()]).is_err());
    }
}
