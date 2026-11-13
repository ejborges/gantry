//! Notifications: who is told what, and how.
//!
//! Three levels of configuration decide whether a notice reaches a person:
//!
//! * the **system administrator** connects a mail server (`[email]` in the configuration file);
//! * the **application administrator** sets the workspace default for each category, may lock
//!   a category so people cannot change it, and tunes reminders (setting `notifications`);
//! * each **person** chooses, per category, between in-app, email, both or neither.
//!
//! This module also runs the things that produce notices on a schedule: due-date reminders,
//! saved-search alerts and the administrators' alert rules.

use crate::auth::Ctx;
use crate::error::{bad, AppError, AppResult};
use crate::http::Api;
use crate::settings::{self, CATEGORIES};
use crate::{audit, authz, jobs, realtime, AppState, St};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post, put};
use axum::Json;
use chrono::{Datelike, NaiveDate, Timelike, Utc};
use lettre::message::header::{self, Header, HeaderName, HeaderValue};
use lettre::message::{Mailbox, MultiPart};
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::Row;
use std::time::Duration;
use uuid::Uuid;

// ───────────────────────── delivering a notice ─────────────────────────

#[derive(Clone, Debug, Default)]
pub struct Notice {
    pub category: &'static str,
    /// Event kind, e.g. `comment.created`.
    pub kind: String,
    /// One sentence: what happened.
    pub title: String,
    /// What it is about, e.g. `OPS-12 Replace pump seal`.
    pub body: String,
    /// Path inside the application.
    pub link: String,
    pub actor_id: Option<Uuid>,
    /// Longer text quoted in email (a message, a list of items). Plain text.
    pub excerpt: String,
    /// Messages with the same thread key are grouped by mail clients.
    pub thread: Option<String>,
}

fn category_known(c: &str) -> bool {
    CATEGORIES.iter().any(|k| k.0 == c)
}

/// (in_app, email, locked) for a category under the workspace settings.
fn workspace_rule(ws: &Value, category: &str) -> (bool, bool, bool) {
    let d = CATEGORIES.iter().find(|c| c.0 == category);
    let r = &ws["categories"][category];
    (r["in_app"].as_bool().unwrap_or(d.map(|d| d.3).unwrap_or(true)), r["email"].as_bool().unwrap_or(d.map(|d| d.4).unwrap_or(false)), r["locked"].as_bool().unwrap_or(false))
}

/// The channels a person receives a category on: their own choice unless the workspace locked it.
pub fn effective(ws: &Value, category: &str, own: Option<(bool, bool)>) -> (bool, bool) {
    let (in_app, email, locked) = workspace_rule(ws, category);
    match own {
        Some(choice) if !locked => choice,
        _ => (in_app, email),
    }
}

/// Addresses that can never receive mail: agents and placeholder domains.
fn deliverable(kind: &str, email: &str) -> bool {
    kind == "human" && crate::accounts::valid_email(email) && !email.ends_with(".invalid") && !email.ends_with(".localhost") && !email.ends_with("@localhost")
}

/// Tell `recipients` about something, on the channels each of them chose. The actor is never
/// told about their own action. Callers decide who may know; this function does not check access.
pub async fn deliver(st: &AppState, recipients: &[Uuid], n: &Notice) {
    debug_assert!(category_known(n.category), "unknown notification category {}", n.category);
    let mut ids: Vec<Uuid> = recipients.iter().copied().filter(|u| Some(*u) != n.actor_id).collect();
    ids.sort();
    ids.dedup();
    if ids.is_empty() {
        return;
    }
    let ws = settings::get(st, "notifications").await;
    let rows = sqlx::query(
        "SELECT u.id, u.kind, u.email, u.name, p.in_app, p.email AS by_email FROM users u
         LEFT JOIN notification_prefs p ON p.user_id = u.id AND p.category = $2 WHERE u.id = ANY($1) AND u.active",
    )
    .bind(&ids)
    .bind(n.category)
    .fetch_all(&st.db)
    .await;
    let rows = match rows {
        Ok(r) => r,
        Err(e) => {
            tracing::error!(error = %e, "could not load notification recipients");
            return;
        }
    };
    let mail_on = st.cfg.email.enabled() && ws["email_enabled"].as_bool().unwrap_or(true) && !st.cfg.server.public_url.is_empty();
    for r in rows {
        let user: Uuid = r.get("id");
        let own = r.get::<Option<bool>, _>("in_app").zip(r.get::<Option<bool>, _>("by_email"));
        let (in_app, email) = effective(&ws, n.category, own);
        if in_app {
            match sqlx::query("INSERT INTO notifications (user_id, kind, category, title, body, link, actor_id) VALUES ($1,$2,$3,$4,$5,$6,$7)").bind(user).bind(&n.kind).bind(n.category).bind(&n.title).bind(&n.body).bind(&n.link).bind(n.actor_id).execute(&st.db).await {
                // The title goes with it so the page can announce the notice as it arrives; this
                // room is the recipient's own, and the title is what their list will show.
                Ok(_) => realtime::publish_user(st, user, json!({"t": "notify", "room": format!("user:{user}"), "category": n.category, "title": n.title, "link": n.link})).await,
                Err(e) => tracing::error!(error = %e, "failed to store notification"),
            }
        }
        let to: String = r.get("email");
        if email && mail_on && deliverable(&r.get::<String, _>("kind"), &to) {
            let payload = json!({"user_id": user, "to": to, "name": r.get::<String, _>("name"), "category": n.category, "title": n.title, "body": n.body, "link": n.link, "excerpt": n.excerpt, "thread": n.thread});
            if let Err(e) = jobs::enqueue(st, "email", payload).await {
                tracing::error!(error = %e, "failed to queue notification email");
            }
        }
    }
}

// ───────────────────────── email ─────────────────────────

/// Marks a message as machine-generated so vacation responders leave it alone (RFC 3834).
#[derive(Clone)]
struct AutoSubmitted;
impl Header for AutoSubmitted {
    fn name() -> HeaderName {
        HeaderName::new_from_ascii_str("Auto-Submitted")
    }
    fn parse(_: &str) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        Ok(AutoSubmitted)
    }
    fn display(&self) -> HeaderValue {
        HeaderValue::new(Self::name(), "auto-generated".into())
    }
}

/// One line of header-safe text: control characters (including line breaks) removed.
pub fn one_line(s: &str, max: usize) -> String {
    s.chars().filter(|c| !crate::accounts::deceptive(*c)).take(max).collect::<String>().trim().to_string()
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

pub struct Mail {
    pub subject: String,
    pub text: String,
    pub html: String,
}

/// Compose the message for a notice. Everything that came from a person is escaped in the
/// HTML part and stripped of line breaks in the subject.
pub fn compose(site: &str, base_url: &str, category: &str, title: &str, body: &str, link: &str, excerpt: &str) -> Mail {
    let site = one_line(site, 60);
    let base = base_url.trim_end_matches('/');
    let url = if link.starts_with('/') { format!("{base}{link}") } else { base.to_string() };
    let label = CATEGORIES.iter().find(|c| c.0 == category).map(|c| c.1).unwrap_or("Notification");
    let subject = one_line(&format!("[{site}] {}", if body.is_empty() { title } else { body }), 200);
    let excerpt = excerpt.trim();
    let quoted = if excerpt.is_empty() { String::new() } else { format!("\n{}\n", excerpt.lines().map(|l| format!("> {l}")).collect::<Vec<_>>().join("\n")) };
    let text = format!("{title}\n{body}\n{quoted}\nOpen: {url}\n\n-- \nYou receive \"{label}\" notifications from {site} by email.\nChange what you receive: {base}/profile#notifications\n");
    let quoted_html = if excerpt.is_empty() { String::new() } else { format!("<blockquote style=\"margin:16px 0;padding:8px 14px;border-left:3px solid #c9ced6;color:#333;white-space:pre-wrap\">{}</blockquote>", html_escape(excerpt)) };
    let html = format!(
        "<!doctype html><html lang=\"en\"><body style=\"margin:0;padding:24px;font-family:system-ui,-apple-system,'Segoe UI',sans-serif;font-size:15px;line-height:1.5;color:#1b1f24;background:#f5f6f8\">\
         <div style=\"max-width:600px;margin:0 auto;background:#ffffff;border:1px solid #dfe3e8;border-radius:8px;padding:24px\">\
         <p style=\"margin:0 0 4px;font-size:12px;letter-spacing:.04em;text-transform:uppercase;color:#5c6670\">{}</p>\
         <p style=\"margin:0 0 4px;font-size:17px;font-weight:600\">{}</p><p style=\"margin:0;color:#444\">{}</p>{quoted_html}\
         <p style=\"margin:20px 0 0\"><a href=\"{}\" style=\"display:inline-block;padding:9px 16px;border-radius:6px;background:#1f5fbf;color:#ffffff;text-decoration:none;font-weight:600\">Open in {}</a></p></div>\
         <p style=\"max-width:600px;margin:14px auto 0;font-size:12px;color:#5c6670\">You receive “{}” notifications from {} by email. <a href=\"{}/profile#notifications\" style=\"color:#5c6670\">Change what you receive</a>.</p></body></html>",
        html_escape(label),
        html_escape(title),
        html_escape(body),
        html_escape(&url),
        html_escape(&site),
        html_escape(label),
        html_escape(&site),
        html_escape(base),
    );
    Mail { subject, text, html }
}

/// Connect to the mail server, greet it and sign in, without sending anything. For `gantry doctor`.
pub async fn check_mail_server(st: &AppState) -> anyhow::Result<String> {
    let e = &st.cfg.email;
    if !e.enabled() {
        return Ok("not configured; notifications stay in the application".into());
    }
    match transport(st)?.test_connection().await {
        Ok(true) => Ok(format!("{}:{} answers ({})", e.host, e.port, e.encryption)),
        Ok(false) => Err(anyhow::anyhow!("{}:{} did not answer as a mail server", e.host, e.port)),
        Err(err) => Err(anyhow::anyhow!("{}:{}: {err}", e.host, e.port)),
    }
}

fn transport(st: &AppState) -> anyhow::Result<AsyncSmtpTransport<Tokio1Executor>> {
    let e = &st.cfg.email;
    let builder = match e.encryption.as_str() {
        "tls" => AsyncSmtpTransport::<Tokio1Executor>::relay(&e.host)?,
        "none" => AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(&e.host),
        _ => AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&e.host)?,
    };
    let builder = builder.port(e.port).timeout(Some(Duration::from_secs(e.timeout_seconds.clamp(2, 120))));
    Ok(if e.username.is_empty() { builder } else { builder.credentials(Credentials::new(e.username.clone(), e.password.clone())) }.build())
}

async fn log_mail(st: &AppState, user: Option<Uuid>, to: &str, category: &str, subject: &str, outcome: &str, detail: &str) {
    let r = sqlx::query("INSERT INTO email_log (user_id, to_addr, category, subject, outcome, detail) VALUES ($1,$2,$3,$4,$5,$6)").bind(user).bind(to).bind(category).bind(subject.chars().take(300).collect::<String>()).bind(outcome).bind(detail.chars().take(500).collect::<String>()).execute(&st.db).await;
    if let Err(e) = r {
        tracing::error!(error = %e, "could not record an email in the log");
    }
}

/// Hand one message to the mail server and record the outcome. The recipient address is
/// logged; the content is not.
pub async fn send_mail(st: &AppState, user: Option<Uuid>, to: &str, to_name: &str, category: &str, mail: &Mail, thread: Option<&str>) -> anyhow::Result<()> {
    let e = &st.cfg.email;
    anyhow::ensure!(e.enabled(), "email is not configured");
    let address: lettre::Address = to.parse().map_err(|_| anyhow::anyhow!("not a valid recipient address"))?;
    let host = url::Url::parse(&st.cfg.server.public_url).ok().and_then(|u| u.host_str().map(str::to_string)).unwrap_or_else(|| "gantry.invalid".into());
    let mut b = Message::builder()
        .from(e.from.parse::<Mailbox>()?)
        .to(Mailbox::new(Some(one_line(to_name, 100)).filter(|n| !n.is_empty()), address))
        .subject(&mail.subject)
        .header(AutoSubmitted)
        .message_id(Some(format!("<{}@{host}>", crate::crypto::random_hex(16))));
    if !e.reply_to.is_empty() {
        b = b.reply_to(e.reply_to.parse::<Mailbox>()?);
    }
    if let Some(t) = thread.filter(|t| t.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')) {
        let root = format!("<thread-{t}@{host}>");
        b = b.header(header::InReplyTo::from(root.clone())).header(header::References::from(root));
    }
    let message = b.multipart(MultiPart::alternative_plain_html(mail.text.clone(), mail.html.clone()))?;
    let result = transport(st)?.send(message).await;
    match result {
        Ok(_) => {
            log_mail(st, user, to, category, &mail.subject, "sent", "").await;
            tracing::info!(target: "email", to, category, "email sent");
            Ok(())
        }
        Err(err) => {
            log_mail(st, user, to, category, &mail.subject, "failed", &err.to_string()).await;
            tracing::warn!(target: "email", to, category, error = %err, "email failed");
            Err(anyhow::anyhow!("the mail server did not accept the message: {err}"))
        }
    }
}

/// The `email` job.
pub async fn email_job(st: &St, p: &Value) -> anyhow::Result<()> {
    let s = |k: &str| p[k].as_str().unwrap_or_default();
    let user: Option<Uuid> = s("user_id").parse().ok();
    // The account may have been deactivated, or the mail server turned off, since this was queued.
    if !st.cfg.email.enabled() {
        log_mail(st, user, s("to"), s("category"), s("title"), "skipped", "email is turned off").await;
        return Ok(());
    }
    if let Some(u) = user {
        let active: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND active AND lower(email) = lower($2))").bind(u).bind(s("to")).fetch_one(&st.db).await?;
        if !active {
            log_mail(st, user, s("to"), s("category"), s("title"), "skipped", "the account is inactive or its address changed").await;
            return Ok(());
        }
    }
    let site = settings::get(st, "site").await;
    let mail = compose(site["name"].as_str().unwrap_or("Gantry"), &st.cfg.server.public_url, s("category"), s("title"), s("body"), s("link"), s("excerpt"));
    send_mail(st, user, s("to"), s("name"), s("category"), &mail, p["thread"].as_str()).await
}

// ───────────────────────── a person's own settings ─────────────────────────

async fn list_notifications(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    Ok(Json(
        sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(n) ORDER BY n.created_at DESC), '[]'::jsonb) FROM (
               SELECT n.id, n.kind, n.category, n.title, n.body, n.link, n.created_at, n.read_at, person_json(a) AS actor
               FROM notifications n LEFT JOIN users a ON a.id = n.actor_id WHERE n.user_id = $1 ORDER BY n.created_at DESC LIMIT 100) n",
        )
        .bind(ctx.user_id)
        .fetch_one(&st.db)
        .await?,
    ))
}

async fn read_all(State(st): State<St>, ctx: Ctx) -> AppResult<StatusCode> {
    sqlx::query("UPDATE notifications SET read_at = now() WHERE user_id = $1 AND read_at IS NULL").bind(ctx.user_id).execute(&st.db).await?;
    realtime::publish_user(&st, ctx.user_id, json!({"t": "notify", "room": format!("user:{}", ctx.user_id)})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn read_one(State(st): State<St>, ctx: Ctx, Path(id): Path<i64>) -> AppResult<StatusCode> {
    let n = sqlx::query("UPDATE notifications SET read_at = coalesce(read_at, now()) WHERE id = $1 AND user_id = $2").bind(id).bind(ctx.user_id).execute(&st.db).await?.rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    realtime::publish_user(&st, ctx.user_id, json!({"t": "notify", "room": format!("user:{}", ctx.user_id)})).await;
    Ok(StatusCode::NO_CONTENT)
}

/// The categories that apply to the caller, with the workspace rule and their own choice.
async fn get_prefs(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    let ws = settings::get(&st, "notifications").await;
    let own: Vec<(String, bool, bool)> = sqlx::query_as("SELECT category, in_app, email FROM notification_prefs WHERE user_id = $1").bind(ctx.user_id).fetch_all(&st.db).await?;
    let cats: Vec<Value> = CATEGORIES
        .iter()
        .filter(|c| c.5.is_empty() || ctx.global.has(c.5))
        .map(|c| {
            let (_, _, locked) = workspace_rule(&ws, c.0);
            let (in_app, email) = effective(&ws, c.0, own.iter().find(|o| o.0 == c.0).map(|o| (o.1, o.2)));
            json!({"key": c.0, "label": c.1, "description": c.2, "in_app": in_app, "email": email, "locked": locked})
        })
        .collect();
    let address: String = sqlx::query_scalar("SELECT email FROM users WHERE id = $1").bind(ctx.user_id).fetch_one(&st.db).await?;
    Ok(Json(json!({
        "categories": cats,
        "email_available": st.cfg.email.enabled() && ws["email_enabled"].as_bool().unwrap_or(true) && deliverable(&ctx.kind, &address),
        "email_address": address,
    })))
}

async fn put_prefs(State(st): State<St>, ctx: Ctx, Json(b): Json<Map<String, Value>>) -> AppResult<StatusCode> {
    let ws = settings::get(&st, "notifications").await;
    for (cat, v) in &b {
        let Some(def) = CATEGORIES.iter().find(|c| c.0 == cat) else { return Err(bad(format!("unknown notification category {cat:?}"))) };
        if !def.5.is_empty() && !ctx.global.has(def.5) {
            return Err(bad(format!("unknown notification category {cat:?}")));
        }
        let (Some(in_app), Some(email)) = (v["in_app"].as_bool(), v["email"].as_bool()) else { return Err(bad("each category needs in_app and email set to true or false")) };
        if workspace_rule(&ws, cat).2 {
            return Err(AppError::Forbidden(format!("\"{}\" is set by your administrator", def.1)));
        }
        sqlx::query("INSERT INTO notification_prefs (user_id, category, in_app, email) VALUES ($1,$2,$3,$4) ON CONFLICT (user_id, category) DO UPDATE SET in_app = EXCLUDED.in_app, email = EXCLUDED.email").bind(ctx.user_id).bind(cat).bind(in_app).bind(email).execute(&st.db).await?;
    }
    audit::log(&st, &ctx, "user.notification_prefs", "user", &ctx.user_id.to_string(), json!({"categories": b.keys().collect::<Vec<_>>()})).await;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── reminders ─────────────────────────

fn local_now(tz: &str, fallback: &str) -> chrono::DateTime<chrono_tz::Tz> {
    let zone: chrono_tz::Tz = tz.parse().or_else(|_| fallback.parse()).unwrap_or(chrono_tz::UTC);
    Utc::now().with_timezone(&zone)
}

/// Where a reminder leads: the caller's own unfinished work, soonest due date first. The
/// `find` parameter is the search page's own encoding of a query (see `web/src/lib/criteria.ts`).
pub const REMINDER_LINK: &str = "/search?find=%7B%22c%22%3A%5B%5B%22assignee%22%2C%22in%22%2C%5B%22me%22%5D%5D%2C%5B%22status_category%22%2C%22not_in%22%2C%5B%22done%22%5D%5D%2C%5B%22due%22%2C%22is_not_empty%22%5D%5D%2C%22s%22%3A%5B%5B%22due%22%2C%22asc%22%5D%5D%7D";

/// Which reminder, if any, an item with this due date deserves today.
pub fn reminder_kind(due: NaiveDate, today: NaiveDate, due_soon_days: i64, overdue_every_days: i64) -> Option<&'static str> {
    let days = (due - today).num_days();
    if days == 0 {
        Some("due_today")
    } else if days > 0 && days == due_soon_days {
        Some("due_soon")
    } else if days < 0 && overdue_every_days > 0 && (-days - 1) % overdue_every_days == 0 {
        Some("overdue")
    } else {
        None
    }
}

/// Once an hour: one digest per person whose local time is the configured reminder hour,
/// listing their open work that is due soon, due today or overdue.
pub async fn send_reminders(st: &AppState) -> anyhow::Result<u32> {
    let ws = settings::get(st, "notifications").await;
    let site = settings::get(st, "site").await;
    let (soon, every, hour) = (ws["reminders"]["due_soon_days"].as_i64().unwrap_or(2), ws["reminders"]["overdue_every_days"].as_i64().unwrap_or(3), ws["reminders"]["hour"].as_i64().unwrap_or(8) as u32);
    let rows = sqlx::query(
        "SELECT i.id, i.assignee_id, i.title, i.due_date, p.key || '-' || i.number AS key, u.timezone
         FROM items i JOIN projects p ON p.id = i.project_id JOIN users u ON u.id = i.assignee_id
         WHERE NOT i.archived AND NOT p.archived AND i.status_category <> 'done' AND i.due_date IS NOT NULL AND u.active
           AND i.due_date BETWEEN current_date - 366 AND current_date + 32
         ORDER BY i.assignee_id, i.due_date",
    )
    .fetch_all(&st.db)
    .await?;
    let mut by_user: std::collections::BTreeMap<Uuid, Vec<(Uuid, String, &'static str, NaiveDate)>> = Default::default();
    for r in &rows {
        let user: Uuid = r.get("assignee_id");
        let now = local_now(&r.get::<String, _>("timezone"), site["timezone"].as_str().unwrap_or("UTC"));
        if now.hour() != hour {
            continue;
        }
        let (due, today): (NaiveDate, NaiveDate) = (r.get("due_date"), now.date_naive());
        let Some(kind) = reminder_kind(due, today, soon, every) else { continue };
        // Claim the reminder; a second worker or a rerun in the same hour gets nothing.
        let fresh = sqlx::query("INSERT INTO reminders_sent (item_id, user_id, kind, for_date) VALUES ($1,$2,$3,$4) ON CONFLICT DO NOTHING").bind(r.get::<Uuid, _>("id")).bind(user).bind(kind).bind(today).execute(&st.db).await?.rows_affected() > 0;
        if fresh {
            by_user.entry(user).or_default().push((r.get("id"), format!("{} {}", r.get::<String, _>("key"), r.get::<String, _>("title")), kind, due));
        }
    }
    let mut sent = 0;
    for (user, list) in by_user {
        let count = |k: &str| list.iter().filter(|i| i.2 == k).count();
        let mut parts = vec![];
        for (k, label) in [("overdue", "overdue"), ("due_today", "due today"), ("due_soon", "due soon")] {
            if count(k) > 0 {
                parts.push(format!("{} {label}", count(k)));
            }
        }
        let lines: Vec<String> = list.iter().take(25).map(|i| format!("{} — due {}", i.1, i.3.format("%b %-d, %Y"))).collect();
        let n = Notice { category: "reminder", kind: "reminder.due".into(), title: format!("Your work: {}", parts.join(", ")), body: if list.len() == 1 { list[0].1.clone() } else { format!("{} work items need attention", list.len()) }, link: REMINDER_LINK.into(), actor_id: None, excerpt: lines.join("\n"), thread: None };
        deliver(st, &[user], &n).await;
        sent += 1;
    }
    sqlx::query("DELETE FROM reminders_sent WHERE for_date < current_date - 400").execute(&st.db).await?;
    Ok(sent)
}

// ───────────────────────── alert rules ─────────────────────────

pub struct AlertKind {
    pub kind: &'static str,
    pub scope: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    /// (parameter, label, default)
    pub params: &'static [(&'static str, &'static str, i64)],
}

pub const ALERT_KINDS: &[AlertKind] = &[
    AlertKind { kind: "http_error_rate", scope: "system", name: "Server error rate", description: "Share of requests that ended in a server error (5xx) over a period.", params: &[("window_minutes", "Period in minutes", 10), ("threshold_percent", "Alert above this percentage", 5), ("min_requests", "Ignore periods with fewer requests than", 50)] },
    AlertKind { kind: "http_latency", scope: "system", name: "Response time", description: "Average time to answer a request over a period.", params: &[("window_minutes", "Period in minutes", 10), ("threshold_ms", "Alert above this many milliseconds", 1500), ("min_requests", "Ignore periods with fewer requests than", 50)] },
    AlertKind { kind: "rate_limited", scope: "system", name: "Requests refused by the rate limit", description: "Requests answered 429 over a period: a client in a loop, or an attack.", params: &[("window_minutes", "Period in minutes", 10), ("threshold", "Alert above this many", 200)] },
    AlertKind { kind: "job_backlog", scope: "system", name: "Background job backlog", description: "Jobs waiting to run right now.", params: &[("threshold", "Alert above this many waiting jobs", 500)] },
    AlertKind { kind: "job_failures", scope: "system", name: "Background job failures", description: "Jobs that failed for good over a period.", params: &[("window_minutes", "Period in minutes", 60), ("threshold", "Alert at this many failures", 5)] },
    AlertKind { kind: "db_pool", scope: "system", name: "Database connections in use", description: "Highest share of the connection pool in use on any node in the last few minutes.", params: &[("threshold_percent", "Alert above this percentage", 90)] },
    AlertKind { kind: "component_down", scope: "system", name: "Component health", description: "File storage, search or cache does not answer its health check.", params: &[] },
    AlertKind { kind: "node_missing", scope: "system", name: "Server node stopped reporting", description: "A node that was running in the last day has not reported for a while.", params: &[("minutes", "Minutes without a report", 5)] },
    AlertKind { kind: "audit_failures", scope: "system", name: "Audit log write failures", description: "Audit entries that could not be stored over a period.", params: &[("window_minutes", "Period in minutes", 10), ("threshold", "Alert at this many", 1)] },
    AlertKind { kind: "security_events", scope: "system", name: "Security events", description: "Security events over a period, optionally only some kinds or one severity.", params: &[("window_minutes", "Period in minutes", 15), ("threshold", "Alert at this many events", 20)] },
    AlertKind { kind: "email_failures", scope: "system", name: "Email delivery failures", description: "Messages the mail server refused over a period.", params: &[("window_minutes", "Period in minutes", 60), ("threshold", "Alert at this many", 5)] },
    AlertKind { kind: "integration_failures", scope: "application", name: "Integration delivery failures", description: "Messages to Slack, Teams or webhooks that failed over a period.", params: &[("window_minutes", "Period in minutes", 60), ("threshold", "Alert at this many", 5)] },
    AlertKind { kind: "sla_breaches", scope: "application", name: "Missed service-level targets", description: "Requests that missed a response or resolution target over a period.", params: &[("window_minutes", "Period in minutes", 60), ("threshold", "Alert at this many", 3)] },
    AlertKind { kind: "unassigned_requests", scope: "application", name: "Unassigned requests", description: "Open service desk requests nobody has taken, older than a given age.", params: &[("older_than_minutes", "Older than this many minutes", 240), ("threshold", "Alert at this many", 1)] },
    AlertKind { kind: "overdue_items", scope: "application", name: "Overdue work", description: "Open work items past their due date, in one project or everywhere.", params: &[("threshold", "Alert at this many", 10)] },
    AlertKind { kind: "stale_items", scope: "application", name: "Stalled work", description: "Work items in progress that nobody has touched for a number of days.", params: &[("days", "Days without a change", 14), ("threshold", "Alert at this many", 5)] },
    AlertKind { kind: "new_accounts", scope: "application", name: "New accounts", description: "Accounts created over a period (by hand, single sign-on or provisioning).", params: &[("window_minutes", "Period in minutes", 60), ("threshold", "Alert at this many", 1)] },
];

fn param(p: &Value, k: &str, default: i64) -> i64 {
    p[k].as_i64().unwrap_or(default)
}

async fn metric_sum(st: &AppState, name: &str, minutes: i64) -> sqlx::Result<f64> {
    sqlx::query_scalar("SELECT coalesce(sum(value), 0)::float8 FROM metric_samples WHERE name = $1 AND at > now() - make_interval(mins => $2)").bind(name).bind(minutes.clamp(1, 10_080) as i32).fetch_one(&st.db).await
}

/// Measure one rule: (current value, whether it is over its threshold, sentence for people).
pub async fn evaluate(st: &AppState, kind: &str, p: &Value) -> anyhow::Result<(f64, bool, String)> {
    let window = param(p, "window_minutes", 10).clamp(1, 10_080);
    let threshold = param(p, "threshold", 1) as f64;
    let count = |sql: &'static str| sqlx::query_scalar::<_, i64>(sql).bind(window as i32);
    Ok(match kind {
        "http_error_rate" => {
            let (req, err) = (metric_sum(st, "requests", window).await?, metric_sum(st, "server_errors", window).await?);
            let rate = if req >= param(p, "min_requests", 50) as f64 && req > 0.0 { 100.0 * err / req } else { 0.0 };
            (rate, rate > param(p, "threshold_percent", 5) as f64, format!("{rate:.1}% of {req:.0} requests failed with a server error in the last {window} minutes"))
        }
        "http_latency" => {
            let (req, micros) = (metric_sum(st, "requests", window).await?, metric_sum(st, "duration_micros", window).await?);
            let ms = if req >= param(p, "min_requests", 50) as f64 && req > 0.0 { micros / req / 1000.0 } else { 0.0 };
            (ms, ms > param(p, "threshold_ms", 1500) as f64, format!("requests took {ms:.0} ms on average over the last {window} minutes"))
        }
        "rate_limited" => {
            let v = metric_sum(st, "rate_limited", window).await?;
            (v, v >= threshold, format!("{v:.0} requests were refused by the rate limit in the last {window} minutes"))
        }
        "audit_failures" => {
            let v = metric_sum(st, "audit_failures", window).await?;
            (v, v >= threshold, format!("{v:.0} audit entries could not be written in the last {window} minutes"))
        }
        "job_backlog" => {
            let v: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE state = 'queued' AND run_at <= now()").fetch_one(&st.db).await?;
            (v as f64, v as f64 > threshold, format!("{v} background jobs are waiting"))
        }
        "job_failures" => {
            let v = count("SELECT count(*) FROM jobs WHERE state = 'failed' AND finished_at > now() - make_interval(mins => $1)").fetch_one(&st.db).await?;
            (v as f64, v as f64 >= threshold, format!("{v} background jobs failed for good in the last {window} minutes"))
        }
        "db_pool" => {
            let v: f64 = sqlx::query_scalar("SELECT coalesce(max(value), 0)::float8 FROM metric_samples WHERE name = 'db_pool_percent' AND at > now() - interval '3 minutes'").fetch_one(&st.db).await?;
            (v, v > param(p, "threshold_percent", 90) as f64, format!("{v:.0}% of database connections are in use"))
        }
        "component_down" => {
            let mut down = vec![];
            if st.stores.blob.health().await.is_err() {
                down.push("file storage");
            }
            if st.stores.search.health().await.is_err() {
                down.push("search");
            }
            if st.stores.kv.set("health:probe", "1", 30).await.is_err() {
                down.push("cache");
            }
            (down.len() as f64, !down.is_empty(), if down.is_empty() { "all components answer their health checks".into() } else { format!("not answering: {}", down.join(", ")) })
        }
        "node_missing" => {
            let names: Vec<String> = sqlx::query_scalar("SELECT node FROM cluster_nodes WHERE seen_at < now() - make_interval(mins => $1) AND seen_at > now() - interval '1 day'").bind(param(p, "minutes", 5).clamp(2, 1440) as i32).fetch_all(&st.db).await?;
            (names.len() as f64, !names.is_empty(), if names.is_empty() { "every node is reporting".into() } else { format!("not reporting: {}", names.join(", ")) })
        }
        "security_events" => {
            let kinds: Vec<String> = p["kinds"].as_array().map(|a| a.iter().filter_map(|k| k.as_str().map(str::to_string)).collect()).unwrap_or_default();
            let severity = p["severity"].as_str().filter(|s| ["info", "warning", "critical"].contains(s));
            let v: i64 = sqlx::query_scalar("SELECT count(*) FROM security_events WHERE at > now() - make_interval(mins => $1) AND (cardinality($2::text[]) = 0 OR kind = ANY($2)) AND ($3::text IS NULL OR severity = $3)").bind(window as i32).bind(&kinds).bind(severity).fetch_one(&st.db).await?;
            (v as f64, v as f64 >= threshold, format!("{v} security events{} in the last {window} minutes", if kinds.is_empty() { String::new() } else { format!(" ({})", kinds.join(", ")) }))
        }
        "email_failures" => {
            let v = count("SELECT count(*) FROM email_log WHERE outcome = 'failed' AND at > now() - make_interval(mins => $1)").fetch_one(&st.db).await?;
            (v as f64, v as f64 >= threshold, format!("{v} emails were refused by the mail server in the last {window} minutes"))
        }
        "integration_failures" => {
            let v = count("SELECT count(*) FROM deliveries WHERE direction = 'out' AND outcome <> 'ok' AND at > now() - make_interval(mins => $1)").fetch_one(&st.db).await?;
            (v as f64, v as f64 >= threshold, format!("{v} integration deliveries failed in the last {window} minutes"))
        }
        "sla_breaches" => {
            let v = count("SELECT count(*) FROM item_history WHERE field = 'sla' AND at > now() - make_interval(mins => $1)").fetch_one(&st.db).await?;
            (v as f64, v as f64 >= threshold, format!("{v} requests missed a service-level target in the last {window} minutes"))
        }
        "unassigned_requests" => {
            let v: i64 = sqlx::query_scalar("SELECT count(*) FROM items i JOIN projects p ON p.id = i.project_id WHERE p.kind = 'service_desk' AND NOT i.archived AND NOT p.archived AND i.status_category <> 'done' AND i.assignee_id IS NULL AND i.created_at < now() - make_interval(mins => $1)").bind(param(p, "older_than_minutes", 240).clamp(1, 100_000) as i32).fetch_one(&st.db).await?;
            (v as f64, v as f64 >= threshold, format!("{v} open requests have no owner"))
        }
        "overdue_items" => {
            let project = p["project"].as_str().map(str::to_uppercase);
            let v: i64 = sqlx::query_scalar("SELECT count(*) FROM items i JOIN projects p ON p.id = i.project_id WHERE NOT i.archived AND NOT p.archived AND i.status_category <> 'done' AND i.due_date < current_date AND ($1::text IS NULL OR p.key = $1)").bind(&project).fetch_one(&st.db).await?;
            (v as f64, v as f64 >= threshold, format!("{v} open work items are overdue{}", project.map(|k| format!(" in {k}")).unwrap_or_default()))
        }
        "stale_items" => {
            let (days, project) = (param(p, "days", 14).clamp(1, 3650), p["project"].as_str().map(str::to_uppercase));
            let v: i64 = sqlx::query_scalar("SELECT count(*) FROM items i JOIN projects p ON p.id = i.project_id WHERE NOT i.archived AND NOT p.archived AND i.status_category = 'in_progress' AND i.updated_at < now() - make_interval(days => $1) AND ($2::text IS NULL OR p.key = $2)").bind(days as i32).bind(&project).fetch_one(&st.db).await?;
            (v as f64, v as f64 >= threshold, format!("{v} work items in progress have not changed in {days} days"))
        }
        "new_accounts" => {
            let v = count("SELECT count(*) FROM users WHERE created_at > now() - make_interval(mins => $1)").fetch_one(&st.db).await?;
            (v as f64, v as f64 >= threshold, format!("{v} accounts were created in the last {window} minutes"))
        }
        other => anyhow::bail!("unknown alert kind {other:?}"),
    })
}

/// Send one alert through the rule's channels: people holding the named roles (in-app and
/// email, as they chose), extra addresses, and Slack, Teams or webhook integrations.
async fn announce(st: &AppState, rule: &sqlx::postgres::PgRow, state: &str, message: &str) {
    let (name, scope, kind, severity): (String, String, String, String) = (rule.get("name"), rule.get("scope"), rule.get("kind"), rule.get("severity"));
    let channels: Value = rule.get("channels");
    let category: &'static str = if kind == "security_events" { "security" } else if scope == "system" { "admin_system" } else { "admin_app" };
    let title = if state == "firing" { format!("Alert: {name}") } else { format!("Resolved: {name}") };
    let link = if scope == "system" { "/admin/alerts?scope=system" } else { "/admin/alerts" };
    let notice = Notice { category, kind: format!("alert.{state}"), title: title.clone(), body: message.to_string(), link: link.into(), actor_id: None, excerpt: format!("Severity: {severity}\nRule: {name}"), thread: Some(rule.get::<Uuid, _>("id").to_string()) };
    let roles: Vec<String> = channels["roles"].as_array().map(|a| a.iter().filter_map(|r| r.as_str().map(str::to_string)).collect()).unwrap_or_default();
    if channels["in_app"] != false || channels["email"] != false {
        let people: Vec<Uuid> = sqlx::query_scalar(
            "SELECT DISTINCT u.id FROM users u JOIN role_bindings b ON (b.user_id = u.id OR b.group_id IN (SELECT group_id FROM group_members WHERE user_id = u.id))
             JOIN roles r ON r.id = b.role_id WHERE b.scope_type = 'global' AND r.key = ANY($1) AND u.active AND u.kind = 'human'",
        )
        .bind(&roles)
        .fetch_all(&st.db)
        .await
        .unwrap_or_default();
        deliver(st, &people, &notice).await;
    }
    if st.cfg.email.enabled() && channels["email"] != false {
        let site = settings::get(st, "site").await;
        for to in channels["addresses"].as_array().into_iter().flatten().filter_map(Value::as_str).filter(|a| crate::accounts::valid_email(a)) {
            let payload = json!({"to": to, "name": "", "category": category, "title": title, "body": message, "link": link, "excerpt": notice.excerpt, "thread": notice.thread, "site": site["name"]});
            if let Err(e) = jobs::enqueue(st, "email", payload).await {
                tracing::error!(error = %e, "failed to queue alert email");
            }
        }
    }
    let base = st.cfg.server.public_url.trim_end_matches('/');
    for id in channels["integrations"].as_array().into_iter().flatten().filter_map(|v| v.as_str()?.parse::<Uuid>().ok()) {
        let payload = json!({"integration_id": id, "event": {"id": crate::crypto::random_hex(12), "type": format!("alert.{state}"), "occurred_at": Utc::now(),
            "actor": {"id": null, "name": "Gantry"}, "subject": name, "summary": format!("{title} — {message}"), "url": format!("{base}{link}"), "data": {"severity": severity, "rule": name, "kind": kind}}});
        if let Err(e) = jobs::enqueue(st, "deliver", payload).await {
            tracing::error!(error = %e, "failed to queue alert delivery");
        }
    }
}

const RULE_COLS: &str = "id, scope, name, kind, params, severity, channels, enabled, cooldown_minutes, notify_resolved, state, last_value, last_checked_at, last_fired_at, builtin";

/// Once a minute on one node: measure every enabled rule and announce changes of state.
pub async fn check_alerts(st: &AppState) -> anyhow::Result<u32> {
    let rules = sqlx::query(&format!("SELECT {RULE_COLS}, (last_fired_at IS NULL OR last_fired_at < now() - make_interval(mins => cooldown_minutes)) AS cooled FROM alert_rules WHERE enabled")).fetch_all(&st.db).await?;
    let mut changed = 0;
    for rule in &rules {
        let (id, kind, was): (Uuid, String, String) = (rule.get("id"), rule.get("kind"), rule.get("state"));
        let (value, firing, message) = match evaluate(st, &kind, &rule.get::<Value, _>("params")).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(target: "alerts", rule = %id, error = %e, "alert rule could not be evaluated");
                continue;
            }
        };
        let now = if firing { "firing" } else { "ok" };
        sqlx::query("UPDATE alert_rules SET last_value = $2, last_checked_at = now(), state = $3 WHERE id = $1").bind(id).bind(value).bind(now).execute(&st.db).await?;
        if now == was {
            continue;
        }
        changed += 1;
        let event = if firing { "firing" } else { "resolved" };
        sqlx::query("INSERT INTO alert_events (rule_id, state, value, message) VALUES ($1,$2,$3,$4)").bind(id).bind(event).bind(value).bind(&message).execute(&st.db).await?;
        tracing::warn!(target: "alerts", rule = %rule.get::<String, _>("name"), state = event, value, "{message}");
        // A rule that flaps announces itself once per cooldown period; every change is still recorded above.
        if firing && rule.get::<bool, _>("cooled") {
            sqlx::query("UPDATE alert_rules SET last_fired_at = now() WHERE id = $1").bind(id).execute(&st.db).await?;
            announce(st, rule, "firing", &message).await;
        } else if !firing && rule.get::<bool, _>("notify_resolved") {
            announce(st, rule, "resolved", &message).await;
        }
    }
    Ok(changed)
}

fn alert_permission(scope: &str) -> &'static str {
    if scope == "system" {
        "admin.settings"
    } else {
        "admin.notifications"
    }
}

async fn list_alerts(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.access")?;
    let rules: Value = sqlx::query_scalar(&format!("SELECT coalesce(jsonb_agg(to_jsonb(r) ORDER BY r.scope DESC, r.name), '[]'::jsonb) FROM (SELECT {RULE_COLS} FROM alert_rules) r")).fetch_one(&st.db).await?;
    let events: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(e) ORDER BY e.at DESC), '[]'::jsonb) FROM (
           SELECT e.id, e.at, e.state, e.value, e.message, r.name AS rule, r.scope, r.severity FROM alert_events e JOIN alert_rules r ON r.id = e.rule_id ORDER BY e.at DESC LIMIT 100) e",
    )
    .fetch_one(&st.db)
    .await?;
    let kinds: Vec<Value> = ALERT_KINDS.iter().map(|k| json!({"kind": k.kind, "scope": k.scope, "name": k.name, "description": k.description, "params": k.params.iter().map(|(key, label, default)| json!({"key": key, "label": label, "default": default})).collect::<Vec<_>>()})).collect();
    let integrations: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'name', name, 'kind', kind) ORDER BY name), '[]'::jsonb) FROM integrations WHERE enabled AND kind IN ('slack', 'teams', 'webhook')").fetch_one(&st.db).await?;
    let roles: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('key', key, 'name', name) ORDER BY name), '[]'::jsonb) FROM roles WHERE scope = 'global'").fetch_one(&st.db).await?;
    Ok(Json(json!({"rules": rules, "events": events, "kinds": kinds, "integrations": integrations, "roles": roles,
        "can_edit": {"system": ctx.has("admin.settings"), "application": ctx.has("admin.notifications")}})))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AlertBody {
    scope: Option<String>,
    name: Option<String>,
    kind: Option<String>,
    params: Option<Value>,
    severity: Option<String>,
    channels: Option<Value>,
    enabled: Option<bool>,
    cooldown_minutes: Option<i32>,
    notify_resolved: Option<bool>,
}

async fn check_rule(st: &St, scope: &str, kind: &str, b: &AlertBody) -> AppResult<()> {
    let def = ALERT_KINDS.iter().find(|k| k.kind == kind).ok_or_else(|| bad(format!("unknown alert kind {kind:?}")))?;
    if def.scope != scope {
        return Err(bad(format!("{} is a {} alert", def.name, def.scope)));
    }
    if let Some(n) = &b.name {
        if n.trim().is_empty() || n.chars().count() > 200 {
            return Err(bad("name must be 1 to 200 characters"));
        }
    }
    if let Some(s) = &b.severity {
        if !["info", "warning", "critical"].contains(&s.as_str()) {
            return Err(bad("severity must be info, warning or critical"));
        }
    }
    if b.cooldown_minutes.is_some_and(|c| !(1..=10_080).contains(&c)) {
        return Err(bad("cooldown_minutes must be between 1 and 10080"));
    }
    if let Some(p) = &b.params {
        let o = p.as_object().ok_or_else(|| bad("params must be an object"))?;
        for (k, v) in o {
            let numeric = def.params.iter().any(|d| d.0 == k);
            let ok = match k.as_str() {
                _ if numeric => v.as_i64().is_some_and(|n| (0..=1_000_000).contains(&n)),
                "kinds" if kind == "security_events" => v.as_array().is_some_and(|a| a.len() <= 30 && a.iter().all(|x| x.as_str().is_some_and(|s| s.len() <= 60))),
                "severity" if kind == "security_events" => v.as_str().is_some_and(|s| ["info", "warning", "critical"].contains(&s)),
                "project" if ["overdue_items", "stale_items"].contains(&kind) => v.as_str().is_some_and(crate::projects::valid_project_key),
                _ => false,
            };
            if !ok {
                return Err(bad(format!("{k:?} is not a valid setting for this alert")));
            }
        }
    }
    if let Some(c) = &b.channels {
        let o = c.as_object().ok_or_else(|| bad("channels must be an object"))?;
        for (k, v) in o {
            let ok = match k.as_str() {
                "in_app" | "email" => v.is_boolean(),
                "roles" => {
                    let keys: Vec<String> = serde_json::from_value(v.clone()).map_err(|_| bad("roles must be a list of role keys"))?;
                    let known: i64 = sqlx::query_scalar("SELECT count(*) FROM roles WHERE scope = 'global' AND key = ANY($1)").bind(&keys).fetch_one(&st.db).await?;
                    known == keys.len() as i64
                }
                "addresses" => v.as_array().is_some_and(|a| a.len() <= 20 && a.iter().all(|x| x.as_str().is_some_and(crate::accounts::valid_email))),
                "integrations" => {
                    let ids: Vec<Uuid> = serde_json::from_value(v.clone()).map_err(|_| bad("integrations must be a list of ids"))?;
                    let known: i64 = sqlx::query_scalar("SELECT count(*) FROM integrations WHERE id = ANY($1) AND kind IN ('slack', 'teams', 'webhook')").bind(&ids).fetch_one(&st.db).await?;
                    known == ids.len() as i64
                }
                _ => false,
            };
            if !ok {
                return Err(bad(format!("channels.{k} is not valid")));
            }
        }
    }
    Ok(())
}

async fn create_alert(State(st): State<St>, ctx: Ctx, Json(b): Json<AlertBody>) -> AppResult<(StatusCode, Json<Value>)> {
    let kind = b.kind.clone().ok_or_else(|| bad("kind is required"))?;
    let scope = b.scope.clone().or_else(|| ALERT_KINDS.iter().find(|k| k.kind == kind).map(|k| k.scope.to_string())).unwrap_or_default();
    if !["system", "application"].contains(&scope.as_str()) {
        return Err(bad("scope must be system or application"));
    }
    authz::require_global(&st, &ctx, alert_permission(&scope))?;
    check_rule(&st, &scope, &kind, &b).await?;
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("name is required"))?;
    let default_channels = json!({"in_app": true, "email": true, "roles": [if scope == "system" { "system_admin" } else { "app_admin" }]});
    let id: Uuid = sqlx::query_scalar("INSERT INTO alert_rules (scope, name, kind, params, severity, channels, enabled, cooldown_minutes, notify_resolved, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10) RETURNING id")
        .bind(&scope)
        .bind(name)
        .bind(&kind)
        .bind(b.params.clone().unwrap_or(json!({})))
        .bind(b.severity.as_deref().unwrap_or("warning"))
        .bind(b.channels.clone().unwrap_or(default_channels))
        .bind(b.enabled.unwrap_or(true))
        .bind(b.cooldown_minutes.unwrap_or(60))
        .bind(b.notify_resolved.unwrap_or(true))
        .bind(ctx.user_id)
        .fetch_one(&st.db)
        .await?;
    audit::log(&st, &ctx, "alert.create", "alert_rule", &id.to_string(), json!({"scope": scope, "name": name, "kind": kind})).await;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

async fn load_rule(st: &St, ctx: &Ctx, id: Uuid) -> AppResult<sqlx::postgres::PgRow> {
    let row = sqlx::query(&format!("SELECT {RULE_COLS} FROM alert_rules WHERE id = $1")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    authz::require_global(st, ctx, alert_permission(&row.get::<String, _>("scope")))?;
    Ok(row)
}

async fn update_alert(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<AlertBody>) -> AppResult<StatusCode> {
    let row = load_rule(&st, &ctx, id).await?;
    if b.kind.is_some() || b.scope.is_some() {
        return Err(bad("the kind and scope of a rule cannot change; create another rule instead"));
    }
    check_rule(&st, &row.get::<String, _>("scope"), &row.get::<String, _>("kind"), &b).await?;
    sqlx::query(
        "UPDATE alert_rules SET name = coalesce($2, name), params = coalesce($3, params), severity = coalesce($4, severity), channels = coalesce($5, channels),
                enabled = coalesce($6, enabled), cooldown_minutes = coalesce($7, cooldown_minutes), notify_resolved = coalesce($8, notify_resolved),
                state = CASE WHEN $6 = false THEN 'ok' ELSE state END, updated_at = now() WHERE id = $1",
    )
    .bind(id)
    .bind(b.name.as_deref().map(str::trim))
    .bind(&b.params)
    .bind(&b.severity)
    .bind(&b.channels)
    .bind(b.enabled)
    .bind(b.cooldown_minutes)
    .bind(b.notify_resolved)
    .execute(&st.db)
    .await?;
    audit::log(&st, &ctx, "alert.update", "alert_rule", &id.to_string(), json!({"name": row.get::<String, _>("name"), "enabled": b.enabled, "params": b.params, "channels": b.channels})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_alert(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let row = load_rule(&st, &ctx, id).await?;
    if row.get::<bool, _>("builtin") {
        // They can be turned off and changed; keeping the row means a default is always there to return to.
        return Err(AppError::Conflict("a built-in alert cannot be deleted; turn it off instead".into()));
    }
    sqlx::query("DELETE FROM alert_rules WHERE id = $1").bind(id).execute(&st.db).await?;
    audit::log(&st, &ctx, "alert.delete", "alert_rule", &id.to_string(), json!({"name": row.get::<String, _>("name")})).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Measure the rule now and send its announcement through every channel, marked as a test.
async fn test_alert(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let row = load_rule(&st, &ctx, id).await?;
    let (value, firing, message) = evaluate(&st, &row.get::<String, _>("kind"), &row.get::<Value, _>("params")).await.map_err(AppError::Other)?;
    announce(&st, &row, "firing", &format!("Test of this alert. Right now: {message}.")).await;
    audit::log(&st, &ctx, "alert.test", "alert_rule", &id.to_string(), json!({"name": row.get::<String, _>("name")})).await;
    Ok(Json(json!({"value": value, "over_threshold": firing, "message": message})))
}

// ───────────────────────── email administration ─────────────────────────

async fn email_status(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.access")?;
    if !(ctx.has("admin.settings") || ctx.has("admin.notifications")) {
        return Err(authz::deny(&st, &ctx, "admin.notifications", "email"));
    }
    let e = &st.cfg.email;
    let log: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(l) ORDER BY l.at DESC), '[]'::jsonb) FROM (SELECT at, to_addr, category, subject, outcome, detail FROM email_log ORDER BY at DESC LIMIT 100) l").fetch_one(&st.db).await?;
    let day: Value = sqlx::query_scalar("SELECT jsonb_build_object('sent', count(*) FILTER (WHERE outcome = 'sent'), 'failed', count(*) FILTER (WHERE outcome = 'failed'), 'skipped', count(*) FILTER (WHERE outcome = 'skipped')) FROM email_log WHERE at > now() - interval '24 hours'").fetch_one(&st.db).await?;
    Ok(Json(json!({
        "configured": e.enabled(), "enabled": e.enabled, "host": e.host, "port": e.port, "encryption": e.encryption, "from": e.from, "reply_to": e.reply_to,
        "authenticated": !e.username.is_empty(), "public_url_set": !st.cfg.server.public_url.is_empty(), "last_24h": day, "log": log,
    })))
}

#[derive(Deserialize)]
struct TestMail {
    to: Option<String>,
}

/// Send a test message now and report what the mail server said.
async fn email_test(State(st): State<St>, ctx: Ctx, Json(b): Json<TestMail>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.settings")?;
    if !st.cfg.email.enabled() {
        return Err(bad("email is not configured: set [email] enabled, host and from in the configuration file and restart"));
    }
    let to = b.to.as_deref().map(str::trim).filter(|t| !t.is_empty()).unwrap_or(&ctx.email).to_string();
    if !crate::accounts::valid_email(&to) {
        return Err(bad("that is not a valid email address"));
    }
    let site = settings::get(&st, "site").await;
    let mail = compose(site["name"].as_str().unwrap_or("Gantry"), &st.cfg.server.public_url, "admin_system", "Test message", "Email from this server works.", "/admin/email", &format!("Sent by {} from the administration console.", ctx.name));
    let result = send_mail(&st, Some(ctx.user_id), &to, "", "admin_system", &mail, None).await;
    audit::log(&st, &ctx, "email.test", "email", &to, json!({"ok": result.is_ok()})).await;
    Ok(Json(match result {
        Ok(()) => json!({"ok": true, "to": to}),
        Err(e) => json!({"ok": false, "to": to, "error": e.to_string()}),
    }))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/notifications", "Notifications", "List own notifications", get(list_notifications))
        .route("POST", "/api/v1/notifications/read", "Notifications", "Mark all notifications read", post(read_all))
        .route("POST", "/api/v1/notifications/{id}/read", "Notifications", "Mark one notification read", post(read_one))
        .route("GET", "/api/v1/notifications/preferences", "Notifications", "Own notification choices per category", get(get_prefs))
        .route("PUT", "/api/v1/notifications/preferences", "Notifications", "Choose in-app and email delivery per category", put(put_prefs))
        .route("GET", "/api/v1/admin/alerts", "Admin: alerts", "Alert rules, the kinds available and recent alerts", get(list_alerts))
        .route("POST", "/api/v1/admin/alerts", "Admin: alerts", "Create an alert rule", post(create_alert))
        .route("PATCH", "/api/v1/admin/alerts/{id}", "Admin: alerts", "Change or turn off an alert rule", patch(update_alert))
        .route("DELETE", "/api/v1/admin/alerts/{id}", "Admin: alerts", "Delete an alert rule", delete(delete_alert))
        .route("POST", "/api/v1/admin/alerts/{id}/test", "Admin: alerts", "Measure a rule now and send a test alert through its channels", post(test_alert))
        .route("GET", "/api/v1/admin/email", "Admin: email", "Mail server settings in effect and the recent email log", get(email_status))
        .route("POST", "/api/v1/admin/email/test", "Admin: email", "Send a test email", post(email_test))
}

/// Calendar month name helper kept out of line so the date arithmetic above stays readable.
#[allow(dead_code)]
fn month_of(d: NaiveDate) -> u32 {
    d.month()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_reminder_link_is_a_query_the_search_page_and_the_server_both_accept() {
        let find = REMINDER_LINK.strip_prefix("/search?find=").expect("the link opens the search page");
        let json: String = url::form_urlencoded::parse(format!("x={find}").as_bytes()).next().unwrap().1.into_owned();
        let v: Value = serde_json::from_str(&json).expect("the parameter is JSON");
        assert_eq!(v["c"][0], json!(["assignee", "in", ["me"]]));
        assert_eq!(v["c"][1], json!(["status_category", "not_in", ["done"]]));
        assert_eq!(v["c"][2], json!(["due", "is_not_empty"]));
        assert_eq!(v["s"], json!([["due", "asc"]]));
    }

    #[test]
    fn reminders_fire_on_the_right_days_only() {
        let today = d("2026-03-10");
        assert_eq!(reminder_kind(d("2026-03-10"), today, 2, 3), Some("due_today"));
        assert_eq!(reminder_kind(d("2026-03-12"), today, 2, 3), Some("due_soon"));
        assert_eq!(reminder_kind(d("2026-03-11"), today, 2, 3), None, "one day ahead is not the configured lead time");
        assert_eq!(reminder_kind(d("2026-03-13"), today, 2, 3), None);
        assert_eq!(reminder_kind(d("2026-03-09"), today, 2, 3), Some("overdue"), "the day after it was due");
        assert_eq!(reminder_kind(d("2026-03-08"), today, 2, 3), None);
        assert_eq!(reminder_kind(d("2026-03-06"), today, 2, 3), Some("overdue"), "and every third day after");
        assert_eq!(reminder_kind(d("2026-03-09"), today, 2, 0), None, "overdue reminders can be turned off");
        assert_eq!(reminder_kind(d("2026-03-10"), today, 0, 3), Some("due_today"), "a lead time of zero still reminds on the day");
    }

    #[test]
    fn a_locked_category_overrides_personal_choices() {
        let ws = json!({"categories": {"mention": {"in_app": true, "email": true, "locked": true}, "status": {"in_app": true, "email": false, "locked": false}}});
        assert_eq!(effective(&ws, "mention", Some((false, false))), (true, true));
        assert_eq!(effective(&ws, "status", Some((false, true))), (false, true));
        assert_eq!(effective(&ws, "status", None), (true, false));
        // A category the stored settings do not mention falls back to its built-in default.
        assert_eq!(effective(&json!({}), "reminder", None), (true, true));
    }

    #[test]
    fn only_people_with_real_addresses_get_mail() {
        assert!(deliverable("human", "ada@example.org"));
        assert!(!deliverable("agent", "ada@example.org"));
        assert!(!deliverable("human", "build-bot-1a2b3c@agents.invalid"));
        assert!(!deliverable("human", "not-an-address"));
    }

    #[test]
    fn messages_escape_what_people_wrote_and_keep_headers_on_one_line() {
        let m = compose("Gan<try>", "https://gantry.example.org/", "comment", "Ada commented on OPS-1", "OPS-1 Fix <b>pump</b>\r\nBcc: evil@example.org", "/i/OPS-1", "line one\n<script>alert(1)</script>");
        assert!(!m.subject.contains('\r') && !m.subject.contains('\n'), "{:?}", m.subject);
        assert!(m.subject.starts_with("[Gan<try>] OPS-1 Fix"));
        assert!(m.html.contains("&lt;script&gt;alert(1)&lt;/script&gt;") && !m.html.contains("<script>"));
        assert!(m.html.contains("OPS-1 Fix &lt;b&gt;pump&lt;/b&gt;"));
        assert!(m.html.contains("href=\"https://gantry.example.org/i/OPS-1\""));
        assert!(m.text.contains("> line one\n> <script>alert(1)</script>"));
        assert!(m.text.contains("Open: https://gantry.example.org/i/OPS-1"));
        assert!(m.text.contains("https://gantry.example.org/profile#notifications"));
        // A link that is not a path inside the application is never followed.
        let m = compose("Gantry", "https://gantry.example.org", "comment", "t", "b", "https://evil.example/x", "");
        assert!(!m.html.contains("evil.example") && !m.text.contains("evil.example"));
    }

    #[test]
    fn alert_kinds_are_unique_and_scoped() {
        let mut seen = std::collections::HashSet::new();
        for k in ALERT_KINDS {
            assert!(seen.insert(k.kind), "duplicate {}", k.kind);
            assert!(["system", "application"].contains(&k.scope));
            assert!(!k.description.is_empty());
        }
    }
}
