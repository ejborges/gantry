//! Audit trail and security events.
//!
//! * `audit_log` records who did what to which object. Rows are append-only (database trigger)
//!   and hash-chained, so removal or alteration of history is detectable with `gantry audit verify`.
//! * `security_events` records things a security reviewer wants to see: failed sign-ins, lockouts,
//!   denied permissions, bad tokens, CSRF failures, rejected webhook signatures, rate limiting.
//!
//! Both are also written to the structured application log, so they reach journald / a SIEM
//! even if the database write fails.

use crate::auth::{AuthMethod, Ctx};
use crate::crypto;
use crate::http::ReqInfo;
use crate::AppState;
use chrono::{DateTime, Utc};
use serde_json::Value;
use sqlx::Row;
use uuid::Uuid;

const CHAIN_LOCK: i64 = 0x47414e54; // "GANT"

struct Entry<'a> {
    actor_id: Option<Uuid>,
    actor_kind: &'a str,
    actor_label: &'a str,
    auth_method: &'a str,
    action: &'a str,
    target_type: &'a str,
    target_id: &'a str,
    ip: &'a str,
    request_id: &'a str,
    outcome: &'a str,
    detail: Value,
}

#[allow(clippy::too_many_arguments)]
fn chain_hash(prev: Option<&[u8]>, at_micros: i64, actor_id: Option<Uuid>, actor_kind: &str, actor_label: &str, auth_method: &str, action: &str, target_type: &str, target_id: &str, ip: &str, request_id: &str, outcome: &str, detail: &Value) -> Vec<u8> {
    let fields = [
        prev.map(hex::encode).unwrap_or_default(),
        at_micros.to_string(),
        actor_id.map(|u| u.to_string()).unwrap_or_default(),
        actor_kind.to_string(),
        actor_label.to_string(),
        auth_method.to_string(),
        action.to_string(),
        target_type.to_string(),
        target_id.to_string(),
        ip.to_string(),
        request_id.to_string(),
        outcome.to_string(),
        serde_json::to_string(detail).unwrap_or_default(),
    ];
    crypto::sha256(fields.join("\u{1f}").as_bytes())
}

async fn write(st: &AppState, e: Entry<'_>) {
    tracing::info!(
        target: "audit",
        action = e.action, actor = e.actor_label, actor_kind = e.actor_kind, auth = e.auth_method,
        target_type = e.target_type, target_id = e.target_id, outcome = e.outcome,
        ip = e.ip, request_id = e.request_id, detail = %e.detail, "audit"
    );
    let result: Result<(), sqlx::Error> = async {
        let mut tx = st.db.begin().await?;
        sqlx::query("SELECT pg_advisory_xact_lock($1)").bind(CHAIN_LOCK).execute(&mut *tx).await?;
        let prev: Option<Vec<u8>> = sqlx::query_scalar("SELECT hash FROM audit_log ORDER BY id DESC LIMIT 1").fetch_optional(&mut *tx).await?;
        let at = DateTime::<Utc>::from_timestamp_micros(Utc::now().timestamp_micros()).expect("current time is representable");
        let hash = chain_hash(prev.as_deref(), at.timestamp_micros(), e.actor_id, e.actor_kind, e.actor_label, e.auth_method, e.action, e.target_type, e.target_id, e.ip, e.request_id, e.outcome, &e.detail);
        sqlx::query(
            "INSERT INTO audit_log (at, actor_id, actor_kind, actor_label, auth_method, action, target_type, target_id, ip, request_id, outcome, detail, prev_hash, hash)
             VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14)",
        )
        .bind(at)
        .bind(e.actor_id)
        .bind(e.actor_kind)
        .bind(e.actor_label)
        .bind(e.auth_method)
        .bind(e.action)
        .bind(e.target_type)
        .bind(e.target_id)
        .bind(e.ip)
        .bind(e.request_id)
        .bind(e.outcome)
        .bind(&e.detail)
        .bind(prev)
        .bind(hash)
        .execute(&mut *tx)
        .await?;
        tx.commit().await
    }
    .await;
    if let Err(err) = result {
        st.metrics.audit_failures.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        tracing::error!(target: "audit", error = %err, action = e.action, "AUDIT WRITE FAILED; the event above exists only in the application log");
    }
}

/// Record an action performed by an authenticated principal.
pub async fn log(st: &AppState, ctx: &Ctx, action: &str, target_type: &str, target_id: &str, detail: Value) {
    let auth = match &ctx.auth {
        AuthMethod::Session { .. } => "session",
        AuthMethod::Token { .. } => "token",
    };
    write(st, Entry { actor_id: Some(ctx.user_id), actor_kind: &ctx.kind, actor_label: &ctx.email, auth_method: auth, action, target_type, target_id, ip: &ctx.req.ip, request_id: &ctx.req.id, outcome: "success", detail }).await;
}

/// Record an event with no authenticated principal (sign-in attempts, inbound webhooks, CLI, worker).
pub async fn log_system(st: &AppState, info: Option<&ReqInfo>, actor_label: &str, action: &str, target_type: &str, target_id: &str, outcome: &str, detail: Value) {
    let (ip, rid) = info.map(|i| (i.ip.as_str(), i.id.as_str())).unwrap_or(("", ""));
    write(st, Entry { actor_id: None, actor_kind: "system", actor_label, auth_method: "", action, target_type, target_id, ip, request_id: rid, outcome, detail }).await;
}

/// Record a security event (also logged at WARN).
pub async fn security(st: &AppState, kind: &str, severity: &str, actor: Option<(Uuid, &str)>, info: &ReqInfo, detail: Value) {
    let label = actor.map(|a| a.1).unwrap_or("");
    tracing::warn!(target: "security", kind, severity, actor = label, ip = %info.ip, request_id = %info.id, path = %info.path, detail = %detail, "security event");
    st.metrics.security_events.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let r = sqlx::query("INSERT INTO security_events (kind, severity, actor_id, actor_label, ip, request_id, detail) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(kind)
        .bind(severity)
        .bind(actor.map(|a| a.0))
        .bind(label)
        .bind(&info.ip)
        .bind(&info.id)
        .bind(detail)
        .execute(&st.db)
        .await;
    if let Err(e) = r {
        tracing::error!(target: "security", error = %e, kind, "failed to persist security event");
    }
}

#[derive(Debug, serde::Serialize)]
pub struct ChainReport {
    pub entries: i64,
    pub intact: bool,
    pub first_broken_id: Option<i64>,
}

/// Recompute the hash chain over the whole audit log.
pub async fn verify_chain(db: &sqlx::PgPool) -> Result<ChainReport, sqlx::Error> {
    let mut prev: Option<Vec<u8>> = None;
    let mut last_id = 0i64;
    let mut entries = 0i64;
    loop {
        let rows = sqlx::query(
            "SELECT id, at, actor_id, actor_kind, actor_label, auth_method, action, target_type, target_id, ip, request_id, outcome, detail, prev_hash, hash
             FROM audit_log WHERE id > $1 ORDER BY id LIMIT 2000",
        )
        .bind(last_id)
        .fetch_all(db)
        .await?;
        if rows.is_empty() {
            break;
        }
        for r in rows {
            let id: i64 = r.get("id");
            let at: DateTime<Utc> = r.get("at");
            let stored_prev: Option<Vec<u8>> = r.get("prev_hash");
            let stored: Vec<u8> = r.get("hash");
            let expect = chain_hash(
                prev.as_deref(),
                at.timestamp_micros(),
                r.get("actor_id"),
                r.get::<String, _>("actor_kind").as_str(),
                r.get::<String, _>("actor_label").as_str(),
                r.get::<String, _>("auth_method").as_str(),
                r.get::<String, _>("action").as_str(),
                r.get::<String, _>("target_type").as_str(),
                r.get::<String, _>("target_id").as_str(),
                r.get::<String, _>("ip").as_str(),
                r.get::<String, _>("request_id").as_str(),
                r.get::<String, _>("outcome").as_str(),
                &r.get::<Value, _>("detail"),
            );
            if stored_prev != prev || stored != expect {
                return Ok(ChainReport { entries, intact: false, first_broken_id: Some(id) });
            }
            prev = Some(stored);
            last_id = id;
            entries += 1;
        }
    }
    Ok(ChainReport { entries, intact: true, first_broken_id: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn hash_depends_on_every_field_and_on_the_previous_hash() {
        let base = |action: &str, prev: Option<&[u8]>, detail: &Value| chain_hash(prev, 1, None, "human", "a@x", "session", action, "item", "OPS-1", "10.0.0.1", "r1", "success", detail);
        let d = json!({"b": 1, "a": 2});
        let h1 = base("item.create", None, &d);
        assert_eq!(h1.len(), 32);
        assert_eq!(h1, base("item.create", None, &json!({"a": 2, "b": 1})), "key order must not matter");
        assert_ne!(h1, base("item.delete", None, &d));
        assert_ne!(h1, base("item.create", Some(&[1, 2, 3]), &d));
        assert_ne!(h1, base("item.create", None, &json!({"a": 2, "b": 2})));
    }

    #[test]
    fn field_boundaries_are_unambiguous() {
        let a = chain_hash(None, 1, None, "k", "ab", "c", "x", "", "", "", "", "ok", &Value::Null);
        let b = chain_hash(None, 1, None, "k", "a", "bc", "x", "", "", "", "", "ok", &Value::Null);
        assert_ne!(a, b);
    }
}
