//! Integrations with other systems.
//!
//! | Kind      | Outbound                              | Inbound                                   |
//! |-----------|---------------------------------------|-------------------------------------------|
//! | `slack`   | event messages to an incoming webhook | `/gantry` slash command (signed)          |
//! | `teams`   | Adaptive Cards to a Workflows webhook | —                                         |
//! | `webhook` | signed JSON events (Standard Webhooks)| —                                         |
//! | `github`  | —                                     | push and pull-request webhooks (signed)   |
//! | `gitlab`  | —                                     | push and merge-request webhooks (signed)  |
//! | `jira`    | —                                     | repeatable import of issues by JQL        |
//!
//! Adding a kind: add a `KindSpec`, then an arm in `render_outbound` (for outbound) or an inbound
//! handler and route. Secrets are encrypted at rest and never returned by the API.
//! Every outbound call goes through `guarded_client`, which pins DNS and refuses unsafe addresses.

use crate::auth::{self, Ctx};
use crate::authz;
use crate::error::{bad, AppError, AppResult};
use crate::events::EVENT_KINDS;
use crate::http::{Api, ReqInfo};
use crate::items::{self, NewItem};
use crate::projects;
use crate::{audit, crypto, jobs, AppState, St};
use axum::body::Bytes;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, patch, post};
use axum::Json;
use base64::engine::general_purpose::STANDARD;
use base64::Engine;
use chrono::Utc;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::Row;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::OnceLock;
use uuid::Uuid;

pub struct KindSpec {
    pub kind: &'static str,
    pub name: &'static str,
    pub direction: &'static str,
    pub description: &'static str,
    /// (key, label, required)
    pub config: &'static [(&'static str, &'static str, bool)],
    pub secrets: &'static [(&'static str, &'static str, bool)],
}

pub const KINDS: &[KindSpec] = &[
    KindSpec { kind: "slack", name: "Slack", direction: "outbound and inbound", description: "Post event messages to a channel and accept the /gantry slash command.",
        config: &[("acting_user_id", "User the slash command acts as (id)", false)],
        secrets: &[("webhook_url", "Incoming webhook URL", false), ("signing_secret", "App signing secret (for the slash command)", false)] },
    KindSpec { kind: "teams", name: "Microsoft Teams", direction: "outbound", description: "Post Adaptive Cards to a channel through a Workflows webhook.",
        config: &[], secrets: &[("workflow_url", "Workflows webhook URL", true)] },
    KindSpec { kind: "webhook", name: "Generic webhook", direction: "outbound", description: "POST signed JSON events to any HTTPS endpoint (Standard Webhooks signature).",
        config: &[("url", "Endpoint URL", true)], secrets: &[("signing_secret", "Signing secret (generated when left empty)", false)] },
    KindSpec { kind: "github", name: "GitHub", direction: "inbound", description: "Link commits and pull requests that mention a work item key; optionally complete items when a pull request merges.",
        config: &[("auto_transition", "Complete items on merge when the description says \"fixes KEY\" (true/false)", false)], secrets: &[("webhook_secret", "Webhook secret", true)] },
    KindSpec { kind: "gitlab", name: "GitLab", direction: "inbound", description: "Link commits and merge requests that mention a work item key; optionally complete items on merge.",
        config: &[("auto_transition", "Complete items on merge when the description says \"closes KEY\" (true/false)", false)],
        secrets: &[("signing_token", "Signing token (whsec_…, GitLab 19.0+)", false), ("secret_token", "Secret token (older GitLab)", false)] },
    KindSpec { kind: "jira", name: "Atlassian Jira", direction: "import", description: "Import issues from Jira Cloud or Data Center by JQL. Re-running an import updates items it created.",
        config: &[("base_url", "Jira base URL", true), ("deployment", "cloud or datacenter", true), ("email", "Account email (Cloud only)", false)],
        secrets: &[("api_token", "API token (Cloud) or personal access token (Data Center)", true)] },
];

pub fn kind_spec(kind: &str) -> Option<&'static KindSpec> {
    KINDS.iter().find(|k| k.kind == kind)
}

pub struct Integration {
    pub id: Uuid,
    pub kind: String,
    pub name: String,
    pub enabled: bool,
    pub config: Value,
    pub secrets: Map<String, Value>,
    pub project_id: Option<Uuid>,
}

impl Integration {
    fn secret(&self, key: &str) -> Option<&str> {
        self.secrets.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
    }
    fn cfg(&self, key: &str) -> Option<&str> {
        self.config.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
    }
    fn flag(&self, key: &str) -> bool {
        match self.config.get(key) {
            Some(Value::Bool(b)) => *b,
            Some(Value::String(s)) => s.eq_ignore_ascii_case("true"),
            _ => false,
        }
    }
}

pub async fn load(st: &AppState, id: Uuid) -> AppResult<Option<Integration>> {
    let Some(r) = sqlx::query("SELECT id, kind, name, enabled, config, secrets_enc, project_id FROM integrations WHERE id = $1").bind(id).fetch_optional(&st.db).await? else { return Ok(None) };
    let sealed: String = r.get("secrets_enc");
    let secrets = match st.secrets.open(&sealed) {
        Some(s) if s.is_empty() => Map::new(),
        Some(s) => serde_json::from_str(&s).unwrap_or_default(),
        None => {
            tracing::error!(integration = %id, "integration secrets cannot be decrypted; was security.secret_key changed?");
            Map::new()
        }
    };
    Ok(Some(Integration { id, kind: r.get("kind"), name: r.get("name"), enabled: r.get("enabled"), config: r.get("config"), secrets, project_id: r.get("project_id") }))
}

async fn record(st: &AppState, id: Uuid, direction: &str, event: &str, delivery_id: &str, status: Option<i32>, outcome: &str, detail: &str) {
    let r = sqlx::query("INSERT INTO deliveries (integration_id, direction, event, delivery_id, status, outcome, detail) VALUES ($1,$2,$3,$4,$5,$6,$7)")
        .bind(id).bind(direction).bind(event).bind(delivery_id).bind(status).bind(outcome).bind(detail.chars().take(500).collect::<String>())
        .execute(&st.db).await;
    if let Err(e) = r {
        tracing::error!(error = %e, "failed to record delivery");
    }
}

// ───────────────────────── outbound safety ─────────────────────────

/// Why an address must not be contacted, if it must not.
pub fn ip_blocked(ip: IpAddr, block_private: bool, allow_loopback: bool) -> Option<&'static str> {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            if v4.is_unspecified() || v4.is_broadcast() || v4.is_multicast() {
                Some("not a unicast address")
            } else if v4.is_link_local() {
                Some("link-local address (includes cloud metadata services)")
            } else if v4.is_loopback() {
                (!allow_loopback).then_some("loopback address")
            } else if v4.is_private() || (o[0] == 100 && (64..128).contains(&o[1])) {
                block_private.then_some("private network address")
            } else {
                None
            }
        }
        IpAddr::V6(v6) => {
            if let Some(v4) = v6.to_ipv4_mapped() {
                return ip_blocked(IpAddr::V4(v4), block_private, allow_loopback);
            }
            let first = v6.segments()[0];
            if v6.is_unspecified() || v6.is_multicast() {
                Some("not a unicast address")
            } else if (first & 0xffc0) == 0xfe80 {
                Some("link-local address")
            } else if v6.is_loopback() {
                (!allow_loopback).then_some("loopback address")
            } else if (first & 0xfe00) == 0xfc00 {
                block_private.then_some("private network address")
            } else {
                None
            }
        }
    }
}

/// An HTTP client that may only talk to the validated address of `raw_url`'s host.
pub async fn guarded_client(st: &AppState, raw_url: &str) -> anyhow::Result<(reqwest::Client, url::Url)> {
    let url = url::Url::parse(raw_url).map_err(|e| anyhow::anyhow!("invalid URL: {e}"))?;
    let insecure_ok = st.cfg.security.allow_insecure_outbound;
    anyhow::ensure!(url.scheme() == "https" || (insecure_ok && url.scheme() == "http"), "only https URLs are allowed");
    anyhow::ensure!(url.username().is_empty() && url.password().is_none(), "credentials in URLs are not allowed");
    let host = url.host_str().ok_or_else(|| anyhow::anyhow!("URL has no host"))?.to_string();
    let port = url.port_or_known_default().unwrap_or(443);
    let addrs: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.trim_matches(['[', ']']), port)).await.map_err(|e| anyhow::anyhow!("cannot resolve {host}: {e}"))?.collect();
    anyhow::ensure!(!addrs.is_empty(), "{host} does not resolve");
    for a in &addrs {
        if let Some(reason) = ip_blocked(a.ip(), st.cfg.security.block_private_networks, insecure_ok) {
            anyhow::bail!("refusing to contact {host}: {reason}");
        }
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(st.cfg.integrations.timeout_seconds))
        .connect_timeout(std::time::Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .user_agent(format!("Gantry/{}", crate::VERSION))
        .resolve(&host, addrs[0])
        .build()?;
    Ok((client, url))
}

// ───────────────────────── signatures ─────────────────────────

fn sw_key(secret: &str) -> Vec<u8> {
    secret.strip_prefix("whsec_").and_then(|b| STANDARD.decode(b).ok()).unwrap_or_else(|| secret.as_bytes().to_vec())
}

/// Standard Webhooks signature: `v1,` + base64(HMAC-SHA256(key, "{id}.{timestamp}.{body}")).
pub fn sw_sign(secret: &str, id: &str, timestamp: i64, body: &[u8]) -> String {
    let mut msg = format!("{id}.{timestamp}.").into_bytes();
    msg.extend_from_slice(body);
    format!("v1,{}", STANDARD.encode(crypto::hmac_sha256(&sw_key(secret), &msg)))
}

pub fn sw_verify(secret: &str, id: &str, timestamp: &str, body: &[u8], signatures: &str, now: i64) -> bool {
    let Ok(ts) = timestamp.parse::<i64>() else { return false };
    if (now - ts).abs() > 300 {
        return false;
    }
    let expected = sw_sign(secret, id, ts, body);
    signatures.split_whitespace().any(|s| crypto::ct_eq(s.as_bytes(), expected.as_bytes()))
}

/// GitHub: `X-Hub-Signature-256: sha256=<hex HMAC-SHA256 of the body>`.
pub fn github_verify(secret: &str, body: &[u8], header: &str) -> bool {
    let Some(hex_sig) = header.strip_prefix("sha256=") else { return false };
    let Ok(sig) = hex::decode(hex_sig) else { return false };
    crypto::ct_eq(&sig, &crypto::hmac_sha256(secret.as_bytes(), body))
}

/// Slack: `X-Slack-Signature: v0=<hex HMAC-SHA256 of "v0:{timestamp}:{body}">`, five-minute window.
pub fn slack_verify(secret: &str, timestamp: &str, body: &[u8], header: &str, now: i64) -> bool {
    let Ok(ts) = timestamp.parse::<i64>() else { return false };
    if (now - ts).abs() > 300 {
        return false;
    }
    let mut msg = format!("v0:{ts}:").into_bytes();
    msg.extend_from_slice(body);
    let expected = format!("v0={}", hex::encode(crypto::hmac_sha256(secret.as_bytes(), &msg)));
    crypto::ct_eq(header.as_bytes(), expected.as_bytes())
}

// ───────────────────────── outbound delivery ─────────────────────────

fn slack_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;")
}

pub fn slack_message(event: &Value) -> Value {
    let (subject, summary, url) = (event["subject"].as_str().unwrap_or_default(), event["summary"].as_str().unwrap_or_default(), event["url"].as_str().unwrap_or_default());
    let title = if url.starts_with("http") { format!("*<{url}|{}>*", slack_escape(subject)) } else { format!("*{}*", slack_escape(subject)) };
    json!({"text": summary, "blocks": [{"type": "section", "text": {"type": "mrkdwn", "text": format!("{title}\n{}", slack_escape(summary))}}]})
}

/// Message for a Teams Workflows webhook ("When a Teams webhook request is received").
pub fn teams_card(event: &Value) -> Value {
    let (subject, summary, url) = (event["subject"].as_str().unwrap_or_default(), event["summary"].as_str().unwrap_or_default(), event["url"].as_str().unwrap_or_default());
    let mut card = json!({
        "$schema": "http://adaptivecards.io/schemas/adaptive-card.json", "type": "AdaptiveCard", "version": "1.4",
        "body": [
            {"type": "TextBlock", "text": subject, "weight": "Bolder", "size": "Medium", "wrap": true},
            {"type": "TextBlock", "text": summary, "wrap": true}
        ]
    });
    if url.starts_with("http") {
        card["actions"] = json!([{"type": "Action.OpenUrl", "title": "Open in Gantry", "url": url}]);
    }
    json!({"type": "message", "attachments": [{"contentType": "application/vnd.microsoft.card.adaptive", "contentUrl": null, "content": card}]})
}

/// (target URL, extra headers, body) for one outbound message.
fn render_outbound(integ: &Integration, event: &Value) -> anyhow::Result<(String, Vec<(&'static str, String)>, Vec<u8>)> {
    match integ.kind.as_str() {
        "slack" => Ok((integ.secret("webhook_url").ok_or_else(|| anyhow::anyhow!("no incoming webhook URL is configured"))?.to_string(), vec![], serde_json::to_vec(&slack_message(event))?)),
        "teams" => Ok((integ.secret("workflow_url").ok_or_else(|| anyhow::anyhow!("no Workflows webhook URL is configured"))?.to_string(), vec![], serde_json::to_vec(&teams_card(event))?)),
        "webhook" => {
            let body = serde_json::to_vec(event)?;
            let id = format!("msg_{}", event["id"].as_str().unwrap_or("0"));
            let ts = Utc::now().timestamp();
            let mut headers = vec![("webhook-id", id.clone()), ("webhook-timestamp", ts.to_string())];
            if let Some(secret) = integ.secret("signing_secret") {
                headers.push(("webhook-signature", sw_sign(secret, &id, ts, &body)));
            }
            Ok((integ.cfg("url").ok_or_else(|| anyhow::anyhow!("no endpoint URL is configured"))?.to_string(), headers, body))
        }
        other => anyhow::bail!("{other} integrations do not send messages"),
    }
}

async fn send(st: &AppState, integ: &Integration, event: &Value) -> anyhow::Result<u16> {
    let (target, headers, body) = render_outbound(integ, event)?;
    let (client, url) = guarded_client(st, &target).await?;
    let mut req = client.post(url).header("content-type", "application/json").body(body);
    for (k, v) in headers {
        req = req.header(k, v);
    }
    let resp = req.send().await.map_err(|e| anyhow::anyhow!("request failed: {}", e.without_url()))?;
    let status = resp.status();
    anyhow::ensure!(status.is_success(), "endpoint answered {status}");
    Ok(status.as_u16())
}

/// Job handler for `deliver`.
pub async fn deliver(st: &St, payload: &Value) -> anyhow::Result<()> {
    let id: Uuid = payload["integration_id"].as_str().and_then(|s| s.parse().ok()).ok_or_else(|| anyhow::anyhow!("deliver job without integration id"))?;
    let Some(integ) = load(st, id).await.map_err(|e| anyhow::anyhow!("{e}"))?.filter(|i| i.enabled) else { return Ok(()) };
    let event = &payload["event"];
    let kind = event["type"].as_str().unwrap_or_default();
    match send(st, &integ, event).await {
        Ok(status) => {
            record(st, id, "out", kind, event["id"].as_str().unwrap_or_default(), Some(status as i32), "ok", "").await;
            Ok(())
        }
        Err(e) => {
            record(st, id, "out", kind, event["id"].as_str().unwrap_or_default(), None, "error", &e.to_string()).await;
            Err(e)
        }
    }
}

// ───────────────────────── inbound: Git hosting ─────────────────────────

fn key_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"\b([A-Z][A-Z0-9]{1,9}-[0-9]{1,9})\b").expect("valid regex"))
}

fn closing_regex() -> &'static regex::Regex {
    static RE: OnceLock<regex::Regex> = OnceLock::new();
    RE.get_or_init(|| regex::Regex::new(r"(?i)\b(?:fix(?:e[sd])?|close[sd]?|resolve[sd]?)\s*:?\s+([a-z][a-z0-9]{1,9}-[0-9]{1,9})\b").expect("valid regex"))
}

/// Work item keys mentioned in free text, in order of first appearance.
pub fn extract_keys(text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for m in key_regex().captures_iter(text) {
        let k = m[1].to_string();
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

/// Keys that follow a closing keyword ("fixes OPS-12", "Closes: ops-3").
pub fn extract_closing_keys(text: &str) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    for m in closing_regex().captures_iter(text) {
        let k = m[1].to_uppercase();
        if !out.contains(&k) {
            out.push(k);
        }
    }
    out
}

fn http_url(u: &str) -> &str {
    if u.starts_with("https://") || u.starts_with("http://") {
        u
    } else {
        ""
    }
}

async fn linkable_item(st: &AppState, integ: &Integration, key: &str) -> Option<items::Item> {
    let item = items::find_item(st, key).await.ok().flatten()?;
    (integ.project_id.is_none() || integ.project_id == Some(item.project_id)).then_some(item)
}

async fn link_external(st: &AppState, integ: &Integration, keys: &[String], rel: &str, node: (&str, &str), meta: Value) -> usize {
    let mut n = 0;
    for key in keys {
        let Some(item) = linkable_item(st, integ, key).await else { continue };
        match st.stores.graph.add_edge(("item", &item.id.to_string()), rel, node, meta.clone(), None).await {
            Ok(_) => n += 1,
            Err(e) => tracing::error!(error = %e, key, "failed to link external reference"),
        }
    }
    n
}

/// Move items named after a closing keyword to the first reachable "done" status.
async fn complete_items(st: &AppState, integ: &Integration, keys: &[String], source: &str) -> usize {
    let mut n = 0;
    for key in keys {
        let Some(item) = linkable_item(st, integ, key).await else { continue };
        let Ok(ti) = items::load_type(st, &item.type_key).await else { continue };
        let Some(to) = ti.workflow.available(&item.status).into_iter().find(|t| t.requires.is_empty() && ti.workflow.status(&t.to).map(|s| s.category == "done").unwrap_or(false)).map(|t| t.to.clone()) else { continue };
        if items::apply_status(st, None, &item, &ti.workflow, &to).await.is_ok() {
            audit::log_system(st, None, &format!("integration:{}", integ.name), "item.transition", "item", key, "success", json!({"from": item.status, "to": to, "source": source})).await;
            n += 1;
        }
    }
    n
}

struct Inbound {
    integ: Integration,
    payload: Value,
}

/// Common front door for signed inbound webhooks: load, verify, de-duplicate, parse.
async fn accept_inbound(st: &St, info: &ReqInfo, id: Uuid, kind: &str, event: &str, delivery_id: &str, body: &Bytes, verified: impl FnOnce(&Integration) -> bool) -> Result<Inbound, Response> {
    let integ = match load(st, id).await {
        Ok(Some(i)) if i.kind == kind && i.enabled => i,
        Ok(_) => return Err(AppError::NotFound.into_response()),
        Err(e) => return Err(e.into_response()),
    };
    if !verified(&integ) {
        audit::security(st, "webhook.signature_invalid", "warning", None, info, json!({"integration": integ.name, "kind": kind})).await;
        record(st, id, "in", event, "", Some(401), "rejected", "signature verification failed").await;
        return Err(AppError::Unauthorized.into_response());
    }
    if !delivery_id.is_empty() {
        let fresh = sqlx::query("INSERT INTO deliveries (integration_id, direction, event, delivery_id, status, outcome) VALUES ($1, 'in', $2, $3, 200, 'ok') ON CONFLICT DO NOTHING").bind(id).bind(event).bind(delivery_id.chars().take(200).collect::<String>()).execute(&st.db).await.map(|r| r.rows_affected() > 0).unwrap_or(true);
        if !fresh {
            return Err(Json(json!({"ok": true, "duplicate": true})).into_response());
        }
    } else {
        record(st, id, "in", event, "", Some(200), "ok", "").await;
    }
    match serde_json::from_slice(body) {
        Ok(payload) => Ok(Inbound { integ, payload }),
        Err(_) => Err(bad("body is not valid JSON").into_response()),
    }
}

fn header<'a>(h: &'a HeaderMap, name: &str) -> &'a str {
    h.get(name).and_then(|v| v.to_str().ok()).unwrap_or_default()
}

async fn github_hook(State(st): State<St>, info: ReqInfo, Path(id): Path<Uuid>, headers: HeaderMap, body: Bytes) -> Response {
    let event = header(&headers, "x-github-event").to_string();
    let sig = header(&headers, "x-hub-signature-256").to_string();
    let inb = match accept_inbound(&st, &info, id, "github", &event, header(&headers, "x-github-delivery"), &body, |i| i.secret("webhook_secret").map(|s| github_verify(s, &body, &sig)).unwrap_or(false)).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let p = &inb.payload;
    let repo = p["repository"]["full_name"].as_str().unwrap_or_default();
    let (mut linked, mut completed) = (0, 0);
    match event.as_str() {
        "push" => {
            for c in p["commits"].as_array().map(Vec::as_slice).unwrap_or_default() {
                let (sha, msg) = (c["id"].as_str().unwrap_or_default(), c["message"].as_str().unwrap_or_default());
                if sha.is_empty() {
                    continue;
                }
                let title = format!("{} {}", &sha[..sha.len().min(8)], msg.lines().next().unwrap_or_default());
                linked += link_external(&st, &inb.integ, &extract_keys(msg), "mentioned_in", ("commit", sha), json!({"url": http_url(c["url"].as_str().unwrap_or_default()), "title": title, "repo": repo})).await;
            }
        }
        "pull_request" => {
            let pr = &p["pull_request"];
            let text = format!("{}\n{}\n{}", pr["title"].as_str().unwrap_or_default(), pr["body"].as_str().unwrap_or_default(), pr["head"]["ref"].as_str().unwrap_or_default());
            let merged = pr["merged"].as_bool().unwrap_or(false);
            let node = format!("{repo}#{}", pr["number"]);
            let meta = json!({"url": http_url(pr["html_url"].as_str().unwrap_or_default()), "title": format!("{node} {}", pr["title"].as_str().unwrap_or_default()), "state": if merged { "merged" } else { pr["state"].as_str().unwrap_or_default() }});
            linked += link_external(&st, &inb.integ, &extract_keys(&text.to_uppercase()), "pull_request", ("change_request", &node), meta).await;
            if merged && p["action"] == "closed" && inb.integ.flag("auto_transition") {
                completed = complete_items(&st, &inb.integ, &extract_closing_keys(&text), &node).await;
            }
        }
        _ => {}
    }
    tracing::info!(target: "integrations", integration = %inb.integ.name, event = %event, linked, completed, "github webhook");
    Json(json!({"ok": true, "linked": linked, "completed": completed})).into_response()
}

async fn gitlab_hook(State(st): State<St>, info: ReqInfo, Path(id): Path<Uuid>, headers: HeaderMap, body: Bytes) -> Response {
    let event = header(&headers, "x-gitlab-event").to_string();
    let (wid, wts, wsig, token) = (header(&headers, "webhook-id").to_string(), header(&headers, "webhook-timestamp").to_string(), header(&headers, "webhook-signature").to_string(), header(&headers, "x-gitlab-token").to_string());
    let delivery = [header(&headers, "idempotency-key"), wid.as_str(), header(&headers, "x-gitlab-event-uuid")].into_iter().find(|s| !s.is_empty()).unwrap_or_default().to_string();
    let verify = |i: &Integration| match (i.secret("signing_token"), i.secret("secret_token")) {
        // When a signing token is configured the signature is mandatory; the plain token alone is not accepted.
        (Some(s), _) => !wsig.is_empty() && sw_verify(s, &wid, &wts, &body, &wsig, Utc::now().timestamp()),
        (None, Some(t)) => !token.is_empty() && crypto::ct_eq(token.as_bytes(), t.as_bytes()),
        (None, None) => false,
    };
    let inb = match accept_inbound(&st, &info, id, "gitlab", &event, &delivery, &body, verify).await {
        Ok(i) => i,
        Err(r) => return r,
    };
    let p = &inb.payload;
    let repo = p["project"]["path_with_namespace"].as_str().unwrap_or_default();
    let (mut linked, mut completed) = (0, 0);
    match p["object_kind"].as_str().unwrap_or_default() {
        "push" => {
            for c in p["commits"].as_array().map(Vec::as_slice).unwrap_or_default() {
                let (sha, msg) = (c["id"].as_str().unwrap_or_default(), c["message"].as_str().unwrap_or_default());
                if sha.is_empty() {
                    continue;
                }
                let title = format!("{} {}", &sha[..sha.len().min(8)], msg.lines().next().unwrap_or_default());
                linked += link_external(&st, &inb.integ, &extract_keys(msg), "mentioned_in", ("commit", sha), json!({"url": http_url(c["url"].as_str().unwrap_or_default()), "title": title, "repo": repo})).await;
            }
        }
        "merge_request" => {
            let mr = &p["object_attributes"];
            let text = format!("{}\n{}\n{}", mr["title"].as_str().unwrap_or_default(), mr["description"].as_str().unwrap_or_default(), mr["source_branch"].as_str().unwrap_or_default());
            let node = format!("{repo}!{}", mr["iid"]);
            let meta = json!({"url": http_url(mr["url"].as_str().unwrap_or_default()), "title": format!("{node} {}", mr["title"].as_str().unwrap_or_default()), "state": mr["state"].as_str().unwrap_or_default()});
            linked += link_external(&st, &inb.integ, &extract_keys(&text.to_uppercase()), "pull_request", ("change_request", &node), meta).await;
            if mr["action"] == "merge" && inb.integ.flag("auto_transition") {
                completed = complete_items(&st, &inb.integ, &extract_closing_keys(&text), &node).await;
            }
        }
        _ => {}
    }
    tracing::info!(target: "integrations", integration = %inb.integ.name, event = %event, linked, completed, "gitlab webhook");
    Json(json!({"ok": true, "linked": linked, "completed": completed})).into_response()
}

// ───────────────────────── inbound: Slack slash command ─────────────────────────

const SLACK_HELP: &str = "Usage:\n• `/gantry show KEY` — status of a work item\n• `/gantry create PROJECT title…` — create a work item\n• `/gantry find words…` — search";

async fn slack_command(State(st): State<St>, info: ReqInfo, Path(id): Path<Uuid>, headers: HeaderMap, body: Bytes) -> Response {
    let integ = match load(&st, id).await {
        Ok(Some(i)) if i.kind == "slack" && i.enabled => i,
        Ok(_) => return AppError::NotFound.into_response(),
        Err(e) => return e.into_response(),
    };
    let ok = integ.secret("signing_secret").map(|s| slack_verify(s, header(&headers, "x-slack-request-timestamp"), &body, header(&headers, "x-slack-signature"), Utc::now().timestamp())).unwrap_or(false);
    if !ok {
        audit::security(&st, "webhook.signature_invalid", "warning", None, &info, json!({"integration": integ.name, "kind": "slack"})).await;
        return AppError::Unauthorized.into_response();
    }
    let form: HashMap<String, String> = url::form_urlencoded::parse(&body).into_owned().collect();
    let text = form.get("text").map(|s| s.trim()).unwrap_or_default();
    let slack_user = form.get("user_name").cloned().unwrap_or_default();
    let reply = |t: String| Json(json!({"response_type": "ephemeral", "text": t})).into_response();
    let acting = match integ.cfg("acting_user_id").and_then(|s| s.parse::<Uuid>().ok()) {
        Some(u) => auth::service_ctx(&st, u, &info).await.ok().flatten(),
        None => None,
    };
    let Some(ctx) = acting else { return reply("This integration has no acting user configured. Ask a Gantry administrator to set one.".into()) };
    record(&st, id, "in", "slash_command", "", Some(200), "ok", text.split_whitespace().next().unwrap_or("help")).await;
    let (cmd, rest) = text.split_once(char::is_whitespace).unwrap_or((text, ""));
    let base = st.cfg.server.public_url.trim_end_matches('/').to_string();
    let out = match cmd {
        "show" => match items::load_item(&st, &ctx, rest.trim()).await {
            Ok((item, project, _)) => format!("*{}-{}* {}\nStatus: {} · Priority: {}\n{base}/i/{}-{}", project.key, item.number, slack_escape(&item.title), item.status, item.priority, project.key, item.number),
            Err(_) => "No such work item, or the integration user cannot see it.".into(),
        },
        "create" => {
            let (project, title) = rest.trim().split_once(char::is_whitespace).unwrap_or((rest.trim(), ""));
            if title.trim().is_empty() {
                SLACK_HELP.to_string()
            } else {
                let n = NewItem { title: title.trim().to_string(), description: Some(format!("Created from Slack by @{slack_user}.")), ..Default::default() };
                match items::create_item_checked(&st, &ctx, project, n, "slack").await {
                    Ok(v) => format!("Created *{}* {}\n{base}/i/{}", v["key"].as_str().unwrap_or_default(), slack_escape(title.trim()), v["key"].as_str().unwrap_or_default()),
                    Err(e) => format!("Could not create the item: {}", e.public_message()),
                }
            }
        }
        "find" => match crate::search::search(&st, &ctx, rest, vec![], Some("keyword"), 5).await {
            Ok(hits) if hits.is_empty() => "Nothing found.".into(),
            Ok(hits) => hits.iter().map(|h| format!("• *{}* {} — {base}{}", h.label, slack_escape(&h.title), h.url)).collect::<Vec<_>>().join("\n"),
            Err(e) => format!("Search failed: {}", e.public_message()),
        },
        _ => SLACK_HELP.to_string(),
    };
    reply(out)
}

// ───────────────────────── Jira import ─────────────────────────

fn adf_inline(nodes: &[Value]) -> String {
    let mut out = String::new();
    for n in nodes {
        match n["type"].as_str().unwrap_or_default() {
            "text" => {
                let mut t = n["text"].as_str().unwrap_or_default().to_string();
                for m in n["marks"].as_array().map(Vec::as_slice).unwrap_or_default() {
                    t = match m["type"].as_str().unwrap_or_default() {
                        "strong" => format!("**{t}**"),
                        "em" => format!("*{t}*"),
                        "code" => format!("`{t}`"),
                        "strike" => format!("~~{t}~~"),
                        "link" => format!("[{t}]({})", http_url(m["attrs"]["href"].as_str().unwrap_or_default())),
                        _ => t,
                    };
                }
                out.push_str(&t);
            }
            "hardBreak" => out.push('\n'),
            "mention" => out.push_str(n["attrs"]["text"].as_str().unwrap_or("@someone")),
            "emoji" => out.push_str(n["attrs"]["text"].as_str().or(n["attrs"]["shortName"].as_str()).unwrap_or_default()),
            "inlineCard" => out.push_str(http_url(n["attrs"]["url"].as_str().unwrap_or_default())),
            _ => out.push_str(&adf_inline(n["content"].as_array().map(Vec::as_slice).unwrap_or_default())),
        }
    }
    out
}

fn adf_blocks(nodes: &[Value], indent: &str, out: &mut String) {
    for n in nodes {
        let content = n["content"].as_array().map(Vec::as_slice).unwrap_or_default();
        match n["type"].as_str().unwrap_or_default() {
            "paragraph" => out.push_str(&format!("{indent}{}\n\n", adf_inline(content))),
            "heading" => out.push_str(&format!("{} {}\n\n", "#".repeat(n["attrs"]["level"].as_u64().unwrap_or(2).clamp(1, 6) as usize), adf_inline(content))),
            "bulletList" | "orderedList" | "taskList" => {
                let ordered = n["type"] == "orderedList";
                for (i, li) in content.iter().enumerate() {
                    let marker = if ordered { format!("{}. ", i + 1) } else if n["type"] == "taskList" { format!("- [{}] ", if li["attrs"]["state"] == "DONE" { "x" } else { " " }) } else { "- ".to_string() };
                    let mut inner = String::new();
                    let kids = li["content"].as_array().map(Vec::as_slice).unwrap_or_default();
                    if kids.iter().all(|k| k["type"] == "text" || k["type"].as_str().map(|t| !["paragraph", "bulletList", "orderedList", "codeBlock"].contains(&t)).unwrap_or(true)) {
                        inner.push_str(&adf_inline(kids));
                    } else {
                        adf_blocks(kids, &format!("{indent}  "), &mut inner);
                    }
                    out.push_str(&format!("{indent}{marker}{}\n", inner.trim()));
                }
                out.push('\n');
            }
            "codeBlock" => out.push_str(&format!("```{}\n{}\n```\n\n", n["attrs"]["language"].as_str().unwrap_or_default(), content.iter().filter_map(|c| c["text"].as_str()).collect::<String>())),
            "blockquote" => {
                let mut inner = String::new();
                adf_blocks(content, "", &mut inner);
                for line in inner.trim().lines() {
                    out.push_str(&format!("> {line}\n"));
                }
                out.push('\n');
            }
            "rule" => out.push_str("---\n\n"),
            _ => adf_blocks(content, indent, out),
        }
    }
}

/// Convert Atlassian Document Format (Jira Cloud descriptions and comments) to Markdown.
/// A plain string (Jira Data Center) is returned unchanged.
pub fn adf_to_markdown(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Object(_) => {
            let mut out = String::new();
            adf_blocks(v["content"].as_array().map(Vec::as_slice).unwrap_or_default(), "", &mut out);
            out.trim().to_string()
        }
        _ => String::new(),
    }
}

pub fn jira_priority(name: &str) -> i16 {
    match name.to_ascii_lowercase().as_str() {
        "highest" | "blocker" | "critical" | "p1" => 4,
        "high" | "major" | "p2" => 3,
        "medium" | "normal" | "p3" => 2,
        "low" | "lowest" | "minor" | "trivial" | "p4" | "p5" => 1,
        _ => 2,
    }
}

fn jira_category(key: &str) -> &'static str {
    match key {
        "done" => "done",
        "indeterminate" => "in_progress",
        _ => "todo",
    }
}

fn jira_time(s: &str) -> chrono::DateTime<Utc> {
    chrono::DateTime::parse_from_str(s, "%Y-%m-%dT%H:%M:%S%.f%z").map(|d| d.with_timezone(&Utc)).unwrap_or_else(|_| Utc::now())
}

/// Job handler for `jira_import`.
pub async fn jira_import(st: &St, payload: &Value) -> anyhow::Result<()> {
    let parse = |k: &str| payload[k].as_str().and_then(|s| s.parse::<Uuid>().ok()).ok_or_else(|| anyhow::anyhow!("jira_import job is missing {k}"));
    let integ = load(st, parse("integration_id")?).await.map_err(|e| anyhow::anyhow!("{e}"))?.filter(|i| i.kind == "jira").ok_or_else(|| anyhow::anyhow!("the Jira integration no longer exists"))?;
    let project = projects::project_by_id(st, parse("project_id")?).await.map_err(|e| anyhow::anyhow!("{e}"))?;
    let actor = parse("actor_id")?;
    let base = integ.cfg("base_url").ok_or_else(|| anyhow::anyhow!("base_url is not configured"))?.trim_end_matches('/').to_string();
    let cloud = integ.cfg("deployment") != Some("datacenter");
    let token = integ.secret("api_token").ok_or_else(|| anyhow::anyhow!("api_token is not configured"))?.to_string();
    let email = integ.cfg("email").unwrap_or_default().to_string();
    let jql = payload["jql"].as_str().unwrap_or_default().to_string();
    let max = payload["max"].as_i64().unwrap_or(1000).clamp(1, 20_000) as usize;
    let system = format!("jira:{}", integ.id);
    let (client, _) = guarded_client(st, &base).await?;
    let fields = ["summary", "description", "issuetype", "status", "priority", "labels", "duedate", "assignee", "reporter", "parent", "comment"];

    let types: Vec<(String, String)> = sqlx::query_as("SELECT t.key, t.name FROM project_types pt JOIN item_types t ON t.key = pt.type_key WHERE pt.project_id = $1").bind(project.id).fetch_all(&st.db).await?;
    let user_by_email = |e: String| {
        let db = st.db.clone();
        async move {
            if e.is_empty() {
                return None;
            }
            sqlx::query_scalar::<_, Uuid>("SELECT id FROM users WHERE lower(email) = lower($1) AND active").bind(e).fetch_optional(&db).await.ok().flatten()
        }
    };

    let (mut created, mut updated, mut fetched) = (0usize, 0usize, 0usize);
    let mut parents: Vec<(Uuid, String)> = vec![];
    let mut next_token: Option<String> = None;
    let mut start_at = 0usize;
    'pages: loop {
        let req = if cloud {
            let mut body = json!({"jql": jql, "maxResults": 100, "fields": fields});
            if let Some(t) = &next_token {
                body["nextPageToken"] = json!(t);
            }
            client.post(format!("{base}/rest/api/3/search/jql")).basic_auth(&email, Some(&token)).json(&body)
        } else {
            client.get(format!("{base}/rest/api/2/search")).query(&[("jql", jql.clone()), ("startAt", start_at.to_string()), ("maxResults", "100".into()), ("fields", fields.join(","))]).bearer_auth(&token)
        };
        let resp = req.send().await.map_err(|e| anyhow::anyhow!("Jira request failed: {}", e.without_url()))?;
        anyhow::ensure!(resp.status().is_success(), "Jira answered {} (check the URL, credentials and JQL)", resp.status());
        let page: Value = resp.json().await?;
        let issues = page["issues"].as_array().cloned().unwrap_or_default();
        for issue in &issues {
            if fetched >= max {
                break 'pages;
            }
            fetched += 1;
            let (jkey, f) = (issue["key"].as_str().unwrap_or_default(), &issue["fields"]);
            if jkey.is_empty() {
                continue;
            }
            let jtype = f["issuetype"]["name"].as_str().unwrap_or_default();
            let type_key = types.iter().find(|(_, n)| n.eq_ignore_ascii_case(jtype)).map(|(k, _)| k.clone()).unwrap_or_else(|| project.default_type.clone());
            let ti = items::load_type(st, &type_key).await.map_err(|e| anyhow::anyhow!("{e}"))?;
            let jstatus = f["status"]["name"].as_str().unwrap_or_default();
            let category = jira_category(f["status"]["statusCategory"]["key"].as_str().unwrap_or_default());
            let status = ti.workflow.statuses.iter().find(|s| s.name.eq_ignore_ascii_case(jstatus) && s.category == category).or_else(|| ti.workflow.first_in_category(category)).map(|s| s.key.clone()).unwrap_or_else(|| ti.workflow.initial.clone());
            let status_category = ti.workflow.status(&status).map(|s| s.category.clone()).unwrap_or_else(|| "todo".into());
            let title: String = f["summary"].as_str().unwrap_or("(no summary)").chars().take(500).collect();
            let description = adf_to_markdown(&f["description"]);
            let priority = jira_priority(f["priority"]["name"].as_str().unwrap_or_default());
            let labels = items::clean_labels(&f["labels"].as_array().map(|a| a.iter().filter_map(|l| l.as_str().map(|s| s.chars().take(50).collect::<String>())).take(30).collect::<Vec<_>>()).unwrap_or_default()).unwrap_or_default();
            let due = f["duedate"].as_str().and_then(|d| chrono::NaiveDate::parse_from_str(d, "%Y-%m-%d").ok());
            let assignee = user_by_email(f["assignee"]["emailAddress"].as_str().unwrap_or_default().to_string()).await;
            let existing: Option<Uuid> = sqlx::query_scalar("SELECT item_id FROM external_refs WHERE system = $1 AND external_id = $2").bind(&system).bind(jkey).fetch_optional(&st.db).await?;
            let item_id = match existing {
                Some(id) => {
                    sqlx::query("UPDATE items SET title=$2, description=$3, priority=$4, labels=$5, due_date=$6, status=$7, status_category=$8, assignee_id=$9, updated_at=now(), resolved_at = CASE WHEN $8 = 'done' THEN coalesce(resolved_at, now()) ELSE NULL END WHERE id=$1")
                        .bind(id).bind(&title).bind(&description).bind(priority).bind(&labels).bind(due).bind(&status).bind(&status_category).bind(assignee)
                        .execute(&st.db).await?;
                    sqlx::query("UPDATE external_refs SET synced_at = now() WHERE system = $1 AND external_id = $2").bind(&system).bind(jkey).execute(&st.db).await?;
                    items::index_item(st, id).await;
                    updated += 1;
                    id
                }
                None => {
                    let reporter = user_by_email(f["reporter"]["emailAddress"].as_str().unwrap_or_default().to_string()).await.unwrap_or(actor);
                    let n = NewItem { type_key: Some(type_key), title, description: Some(description), priority: Some(priority), assignee_id: assignee, labels: Some(labels), due_date: due, channel: Some("jira".into()), status: Some(status), lenient: true, ..Default::default() };
                    let (id, _) = items::create_item(st, reporter, &project, n).await.map_err(|e| anyhow::anyhow!("importing {jkey}: {e}"))?;
                    sqlx::query("INSERT INTO external_refs (system, external_id, item_id, url) VALUES ($1,$2,$3,$4)").bind(&system).bind(jkey).bind(id).bind(format!("{base}/browse/{jkey}")).execute(&st.db).await?;
                    for c in f["comment"]["comments"].as_array().map(Vec::as_slice).unwrap_or_default() {
                        let author = c["author"]["displayName"].as_str().unwrap_or("Unknown");
                        let text = adf_to_markdown(&c["body"]);
                        if text.trim().is_empty() {
                            continue;
                        }
                        let author_id = user_by_email(c["author"]["emailAddress"].as_str().unwrap_or_default().to_string()).await;
                        let body: String = if author_id.is_some() { text } else { format!("**{author}** (imported from Jira):\n\n{text}") };
                        sqlx::query("INSERT INTO comments (item_id, author_id, body, created_at) VALUES ($1,$2,$3,$4)").bind(id).bind(author_id).bind(body.chars().take(100_000).collect::<String>()).bind(jira_time(c["created"].as_str().unwrap_or_default())).execute(&st.db).await?;
                    }
                    items::index_item(st, id).await;
                    created += 1;
                    id
                }
            };
            if let Some(pk) = f["parent"]["key"].as_str() {
                parents.push((item_id, pk.to_string()));
            }
        }
        if cloud {
            next_token = page["nextPageToken"].as_str().map(str::to_string);
            if next_token.is_none() || issues.is_empty() {
                break;
            }
        } else {
            start_at += issues.len();
            if issues.is_empty() || start_at >= page["total"].as_u64().unwrap_or(0) as usize {
                break;
            }
        }
    }
    for (child, parent_key) in parents {
        sqlx::query("UPDATE items SET parent_id = (SELECT item_id FROM external_refs WHERE system = $2 AND external_id = $3) WHERE id = $1 AND EXISTS (SELECT 1 FROM external_refs WHERE system = $2 AND external_id = $3 AND item_id <> $1)")
            .bind(child).bind(&system).bind(&parent_key).execute(&st.db).await?;
    }
    record(st, integ.id, "in", "jira.import", "", Some(200), "ok", &format!("{created} created, {updated} updated")).await;
    audit::log_system(st, None, &format!("integration:{}", integ.name), "jira.import", "project", &project.key, "success", json!({"jql": jql, "created": created, "updated": updated, "requested_by": actor})).await;
    Ok(())
}

// ───────────────────────── administration ─────────────────────────

fn inbound_url(st: &AppState, kind: &str, id: Uuid) -> Option<String> {
    let base = st.cfg.server.public_url.trim_end_matches('/');
    match kind {
        "github" | "gitlab" => Some(format!("{base}/api/v1/hooks/{kind}/{id}")),
        "slack" => Some(format!("{base}/api/v1/hooks/slack/{id}/command")),
        _ => None,
    }
}

async fn list(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.integrations")?;
    let rows = sqlx::query("SELECT i.id, i.kind, i.name, i.enabled, i.config, i.secrets_enc, i.events, i.created_at, (SELECT key FROM projects p WHERE p.id = i.project_id) AS project,
            (SELECT jsonb_build_object('at', d.at, 'outcome', d.outcome, 'event', d.event) FROM deliveries d WHERE d.integration_id = i.id ORDER BY d.at DESC LIMIT 1) AS last_delivery
         FROM integrations i ORDER BY i.kind, i.name").fetch_all(&st.db).await?;
    let integrations: Vec<Value> = rows.iter().map(|r| {
        let id: Uuid = r.get("id");
        let kind: String = r.get("kind");
        let set: Vec<String> = st.secrets.open(&r.get::<String, _>("secrets_enc")).and_then(|s| serde_json::from_str::<Map<String, Value>>(&s).ok()).map(|m| m.keys().cloned().collect()).unwrap_or_default();
        json!({"id": id, "kind": kind, "name": r.get::<String, _>("name"), "enabled": r.get::<bool, _>("enabled"), "config": r.get::<Value, _>("config"), "secrets_set": set,
               "events": r.get::<Vec<String>, _>("events"), "project": r.get::<Option<String>, _>("project"), "created_at": r.get::<chrono::DateTime<Utc>, _>("created_at"),
               "last_delivery": r.get::<Option<Value>, _>("last_delivery"), "inbound_url": inbound_url(&st, &kind, id)})
    }).collect();
    let field = |f: &(&str, &str, bool)| json!({"key": f.0, "label": f.1, "required": f.2});
    let kinds: Vec<Value> = KINDS.iter().map(|k| json!({"kind": k.kind, "name": k.name, "direction": k.direction, "description": k.description, "config": k.config.iter().map(field).collect::<Vec<_>>(), "secrets": k.secrets.iter().map(field).collect::<Vec<_>>()})).collect();
    let events: Vec<Value> = EVENT_KINDS.iter().map(|(k, d)| json!({"key": k, "description": d})).collect();
    Ok(Json(json!({"integrations": integrations, "kinds": kinds, "events": events})))
}

#[derive(Deserialize)]
struct IntegrationBody {
    kind: Option<String>,
    name: Option<String>,
    enabled: Option<bool>,
    config: Option<Map<String, Value>>,
    secrets: Option<Map<String, Value>>,
    events: Option<Vec<String>>,
    /// Project key to scope the integration to; `""` clears the scope.
    project: Option<String>,
}

fn check_fields(spec: &KindSpec, config: &Map<String, Value>, secrets: &Map<String, Value>, events: &[String]) -> AppResult<()> {
    if let Some(k) = config.keys().find(|k| !spec.config.iter().any(|c| c.0 == k.as_str())) {
        return Err(bad(format!("{k:?} is not a setting of {} integrations", spec.name)));
    }
    if let Some(k) = secrets.keys().find(|k| !spec.secrets.iter().any(|c| c.0 == k.as_str())) {
        return Err(bad(format!("{k:?} is not a secret of {} integrations", spec.name)));
    }
    if let Some(e) = events.iter().find(|e| *e != "*" && !EVENT_KINDS.iter().any(|(k, _)| k == e)) {
        return Err(bad(format!("unknown event {e:?}")));
    }
    for key in ["url", "base_url"] {
        if let Some(u) = config.get(key).and_then(Value::as_str) {
            let parsed = url::Url::parse(u).map_err(|_| bad(format!("{key} is not a valid URL")))?;
            if !matches!(parsed.scheme(), "https" | "http") {
                return Err(bad(format!("{key} must be an http(s) URL")));
            }
        }
    }
    Ok(())
}

async fn project_scope(st: &AppState, key: &Option<String>) -> AppResult<Option<Option<Uuid>>> {
    match key.as_deref() {
        None => Ok(None),
        Some("") => Ok(Some(None)),
        Some(k) => Ok(Some(Some(projects::find_project(st, k).await?.ok_or_else(|| bad("unknown project"))?.id))),
    }
}

async fn create(State(st): State<St>, ctx: Ctx, Json(b): Json<IntegrationBody>) -> AppResult<(StatusCode, Json<Value>)> {
    authz::require_global(&st, &ctx, "admin.integrations")?;
    let kind = b.kind.as_deref().unwrap_or_default();
    let spec = kind_spec(kind).ok_or_else(|| bad("unknown integration kind"))?;
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty() && n.len() <= 200).ok_or_else(|| bad("name is required"))?;
    let config = b.config.unwrap_or_default();
    let mut secrets = b.secrets.unwrap_or_default();
    secrets.retain(|_, v| v.as_str().map(|s| !s.is_empty()).unwrap_or(false));
    let events = b.events.unwrap_or_default();
    check_fields(spec, &config, &secrets, &events)?;
    if let Some(missing) = spec.config.iter().find(|c| c.2 && !config.contains_key(c.0)) {
        return Err(bad(format!("{} is required", missing.1)));
    }
    let mut generated = None;
    if kind == "webhook" && !secrets.contains_key("signing_secret") {
        let mut key = [0u8; 32];
        rand::RngCore::fill_bytes(&mut rand::thread_rng(), &mut key);
        let s = format!("whsec_{}", STANDARD.encode(key));
        secrets.insert("signing_secret".into(), json!(s));
        generated = Some(s);
    }
    if let Some(missing) = spec.secrets.iter().find(|c| c.2 && !secrets.contains_key(c.0)) {
        return Err(bad(format!("{} is required", missing.1)));
    }
    let project_id = project_scope(&st, &b.project).await?.flatten();
    let id: Uuid = sqlx::query_scalar("INSERT INTO integrations (kind, name, enabled, config, secrets_enc, events, project_id, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8) RETURNING id")
        .bind(kind).bind(name).bind(b.enabled.unwrap_or(true)).bind(Value::Object(config)).bind(st.secrets.seal(&Value::Object(secrets).to_string())).bind(&events).bind(project_id).bind(ctx.user_id)
        .fetch_one(&st.db).await?;
    audit::log(&st, &ctx, "integration.create", "integration", &id.to_string(), json!({"kind": kind, "name": name, "events": events})).await;
    Ok((StatusCode::CREATED, Json(json!({"id": id, "inbound_url": inbound_url(&st, kind, id), "generated_signing_secret": generated}))))
}

async fn update(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<IntegrationBody>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.integrations")?;
    let cur = load(&st, id).await?.ok_or(AppError::NotFound)?;
    let spec = kind_spec(&cur.kind).ok_or(AppError::NotFound)?;
    let mut config = cur.config.as_object().cloned().unwrap_or_default();
    for (k, v) in b.config.clone().unwrap_or_default() {
        if v.is_null() || v == "" {
            config.remove(&k);
        } else {
            config.insert(k, v);
        }
    }
    let mut secrets = cur.secrets.clone();
    for (k, v) in b.secrets.clone().unwrap_or_default() {
        if v.is_null() {
            secrets.remove(&k);
        } else if v.as_str().map(|s| !s.is_empty()).unwrap_or(false) {
            secrets.insert(k, v);
        }
    }
    let events_in = b.events.clone();
    check_fields(spec, &config, &secrets, events_in.as_deref().unwrap_or_default())?;
    let scope = project_scope(&st, &b.project).await?;
    sqlx::query("UPDATE integrations SET name = coalesce($2, name), enabled = coalesce($3, enabled), config = $4, secrets_enc = $5, events = coalesce($6, events), project_id = CASE WHEN $7 THEN $8 ELSE project_id END, updated_at = now() WHERE id = $1")
        .bind(id).bind(b.name.as_deref().map(str::trim).filter(|n| !n.is_empty())).bind(b.enabled).bind(Value::Object(config)).bind(st.secrets.seal(&Value::Object(secrets).to_string())).bind(events_in).bind(scope.is_some()).bind(scope.flatten())
        .execute(&st.db).await?;
    audit::log(&st, &ctx, "integration.update", "integration", &id.to_string(), json!({"name": cur.name, "enabled": b.enabled, "secrets_changed": b.secrets.map(|s| s.keys().cloned().collect::<Vec<_>>())})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn remove(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "admin.integrations")?;
    let name: String = sqlx::query_scalar("DELETE FROM integrations WHERE id = $1 RETURNING name").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    audit::log(&st, &ctx, "integration.delete", "integration", &id.to_string(), json!({"name": name})).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Send a test message now and report what happened.
async fn test(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.integrations")?;
    let integ = load(&st, id).await?.ok_or(AppError::NotFound)?;
    let event = json!({"id": crypto::random_hex(12), "type": "test", "occurred_at": Utc::now(), "actor": {"id": ctx.user_id, "name": ctx.name},
        "subject": "Test message from Gantry", "summary": format!("{} sent a test message to check the \"{}\" integration.", ctx.name, integ.name), "url": st.cfg.server.public_url.clone(), "data": {}});
    let out = match send(&st, &integ, &event).await {
        Ok(status) => {
            record(&st, id, "out", "test", "", Some(status as i32), "ok", "").await;
            json!({"ok": true, "status": status})
        }
        Err(e) => {
            record(&st, id, "out", "test", "", None, "error", &e.to_string()).await;
            json!({"ok": false, "error": e.to_string()})
        }
    };
    audit::log(&st, &ctx, "integration.test", "integration", &id.to_string(), json!({"name": integ.name, "ok": out["ok"]})).await;
    Ok(Json(out))
}

async fn deliveries(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "admin.integrations")?;
    Ok(Json(sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(d) ORDER BY d.at DESC), '[]'::jsonb) FROM (SELECT direction, event, status, outcome, detail, at FROM deliveries WHERE integration_id = $1 ORDER BY at DESC LIMIT 50) d").bind(id).fetch_one(&st.db).await?))
}

#[derive(Deserialize)]
struct JiraImportBody {
    project: String,
    jql: String,
    max: Option<i64>,
}

async fn start_jira_import(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<JiraImportBody>) -> AppResult<(StatusCode, Json<Value>)> {
    authz::require_global(&st, &ctx, "admin.integrations")?;
    let integ = load(&st, id).await?.filter(|i| i.kind == "jira").ok_or(AppError::NotFound)?;
    let project = projects::find_project(&st, &b.project).await?.ok_or_else(|| bad("unknown project"))?;
    if b.jql.trim().is_empty() || b.jql.len() > 2000 {
        return Err(bad("jql is required (up to 2000 characters), for example: project = ABC ORDER BY created"));
    }
    let job = jobs::enqueue(&st, "jira_import", json!({"integration_id": id, "project_id": project.id, "actor_id": ctx.user_id, "jql": b.jql.trim(), "max": b.max.unwrap_or(1000)})).await.map_err(AppError::Other)?;
    audit::log(&st, &ctx, "jira.import_requested", "integration", &id.to_string(), json!({"name": integ.name, "project": project.key, "jql": b.jql.trim()})).await;
    Ok((StatusCode::ACCEPTED, Json(json!({"job_id": job}))))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/admin/integrations", "Integrations", "List integrations, available kinds and event types", get(list))
        .route("POST", "/api/v1/admin/integrations", "Integrations", "Create an integration", post(create))
        .route("PATCH", "/api/v1/admin/integrations/{id}", "Integrations", "Update an integration", patch(update))
        .route("DELETE", "/api/v1/admin/integrations/{id}", "Integrations", "Delete an integration", axum::routing::delete(remove))
        .route("POST", "/api/v1/admin/integrations/{id}/test", "Integrations", "Send a test message", post(test))
        .route("GET", "/api/v1/admin/integrations/{id}/deliveries", "Integrations", "Recent deliveries", get(deliveries))
        .route("POST", "/api/v1/admin/integrations/{id}/jira-import", "Integrations", "Start a Jira import", post(start_jira_import))
        .route("POST", "/api/v1/hooks/github/{id}", "Webhooks", "GitHub webhook receiver (signature required)", post(github_hook))
        .public()
        .route("POST", "/api/v1/hooks/gitlab/{id}", "Webhooks", "GitLab webhook receiver (signature required)", post(gitlab_hook))
        .public()
        .route("POST", "/api/v1/hooks/slack/{id}/command", "Webhooks", "Slack slash command receiver (signature required)", post(slack_command))
        .public()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_webhooks_known_answer() {
        // Test vector from the Standard Webhooks specification.
        let secret = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
        let body = br#"{"test": 2432232314}"#;
        assert_eq!(sw_sign(secret, "msg_p5jXN8AQM9LWM0D4loKWxJek", 1614265330, body), "v1,g0hM9SsE+OTPJTGt/tmIKtSyZlE3uFJELVlNIOLJ1OE=");
    }

    #[test]
    fn standard_webhooks_verification() {
        let secret = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
        let sig = sw_sign(secret, "id1", 1000, b"body");
        assert!(sw_verify(secret, "id1", "1000", b"body", &sig, 1100));
        assert!(sw_verify(secret, "id1", "1000", b"body", &format!("v1,AAAA {sig}"), 1100), "any listed signature may match");
        assert!(!sw_verify(secret, "id1", "1000", b"tampered", &sig, 1100));
        assert!(!sw_verify(secret, "id2", "1000", b"body", &sig, 1100));
        assert!(!sw_verify(secret, "id1", "1000", b"body", &sig, 1000 + 301), "stale timestamps are refused");
        assert!(!sw_verify("whsec_b3RoZXI=", "id1", "1000", b"body", &sig, 1100));
        assert!(!sw_verify(secret, "id1", "soon", b"body", &sig, 1100));
    }

    #[test]
    fn github_signature_known_answer() {
        // Example from GitHub's "Validating webhook deliveries" documentation.
        let sig = "sha256=757107ea0eb2509fc211221cce984b8a37570b6d7586c22c46f4379c8b043e17";
        assert!(github_verify("It's a Secret to Everybody", b"Hello, World!", sig));
        assert!(!github_verify("wrong", b"Hello, World!", sig));
        assert!(!github_verify("It's a Secret to Everybody", b"Hello, World?", sig));
        assert!(!github_verify("It's a Secret to Everybody", b"Hello, World!", "sha1=abc"));
    }

    #[test]
    fn slack_signature_and_replay_window() {
        let secret = "8f742231b10e8888abcd99yyyzzz85a5";
        let body = b"token=x&command=%2Fgantry&text=show+OPS-1";
        let mut msg = b"v0:1531420618:".to_vec();
        msg.extend_from_slice(body);
        let sig = format!("v0={}", hex::encode(crypto::hmac_sha256(secret.as_bytes(), &msg)));
        assert!(slack_verify(secret, "1531420618", body, &sig, 1531420618 + 30));
        assert!(!slack_verify(secret, "1531420618", body, &sig, 1531420618 + 400), "replayed request");
        assert!(!slack_verify(secret, "1531420618", b"token=x&text=other", &sig, 1531420618));
        assert!(!slack_verify("other", "1531420618", body, &sig, 1531420618));
    }

    #[test]
    fn unsafe_addresses_are_blocked() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert!(ip_blocked(ip("169.254.169.254"), false, true).is_some(), "metadata service is always blocked");
        assert!(ip_blocked(ip("fe80::1"), false, true).is_some());
        assert!(ip_blocked(ip("0.0.0.0"), false, true).is_some());
        assert!(ip_blocked(ip("127.0.0.1"), false, false).is_some());
        assert!(ip_blocked(ip("::1"), false, false).is_some());
        assert!(ip_blocked(ip("::ffff:127.0.0.1"), false, false).is_some(), "IPv4-mapped loopback");
        assert!(ip_blocked(ip("127.0.0.1"), false, true).is_none());
        assert!(ip_blocked(ip("10.1.2.3"), true, false).is_some());
        assert!(ip_blocked(ip("192.168.1.1"), true, false).is_some());
        assert!(ip_blocked(ip("100.64.0.1"), true, false).is_some());
        assert!(ip_blocked(ip("fd00::1"), true, false).is_some());
        assert!(ip_blocked(ip("10.1.2.3"), false, false).is_none(), "private networks are allowed unless blocked by policy");
        assert!(ip_blocked(ip("93.184.216.34"), true, false).is_none());
        assert!(ip_blocked(ip("2606:2800:220:1::1"), true, false).is_none());
    }

    #[test]
    fn keys_are_extracted_from_text() {
        assert_eq!(extract_keys("OPS-12: fix pump; see also ENG2-7 and OPS-12"), vec!["OPS-12", "ENG2-7"]);
        assert!(extract_keys("no keys here, not even ops-1 or A-1 or UTF-8x").is_empty());
        assert_eq!(extract_keys("feature/OPS-9-new-seal"), vec!["OPS-9"]);
    }

    #[test]
    fn closing_keywords() {
        assert_eq!(extract_closing_keys("Fixes OPS-12 and closes: eng-3. Related to OPS-99. resolved OPS-4"), vec!["OPS-12", "ENG-3", "OPS-4"]);
        assert!(extract_closing_keys("prefixes OPS-1, discloses OPS-2").is_empty(), "keywords must be whole words");
    }

    #[test]
    fn adf_converts_to_markdown() {
        let doc = json!({"type": "doc", "version": 1, "content": [
            {"type": "heading", "attrs": {"level": 2}, "content": [{"type": "text", "text": "Steps"}]},
            {"type": "paragraph", "content": [{"type": "text", "text": "Open the "}, {"type": "text", "text": "valve", "marks": [{"type": "strong"}]}, {"type": "text", "text": " and see "},
                {"type": "text", "text": "the manual", "marks": [{"type": "link", "attrs": {"href": "https://example.org/m"}}]}]},
            {"type": "bulletList", "content": [
                {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "one"}]}]},
                {"type": "listItem", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "two", "marks": [{"type": "code"}]}]}]}]},
            {"type": "codeBlock", "attrs": {"language": "sh"}, "content": [{"type": "text", "text": "systemctl status pump"}]}
        ]});
        assert_eq!(adf_to_markdown(&doc), "## Steps\n\nOpen the **valve** and see [the manual](https://example.org/m)\n\n- one\n- `two`\n\n```sh\nsystemctl status pump\n```");
        assert_eq!(adf_to_markdown(&json!("h2. wiki markup")), "h2. wiki markup");
        assert_eq!(adf_to_markdown(&Value::Null), "");
        let evil = json!({"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "x", "marks": [{"type": "link", "attrs": {"href": "javascript:alert(1)"}}]}]}]});
        assert_eq!(adf_to_markdown(&evil), "[x]()", "non-http links are dropped");
    }

    #[test]
    fn jira_field_mapping() {
        assert_eq!(jira_priority("Highest"), 4);
        assert_eq!(jira_priority("high"), 3);
        assert_eq!(jira_priority("Medium"), 2);
        assert_eq!(jira_priority("Lowest"), 1);
        assert_eq!(jira_priority("Custom"), 2);
        assert_eq!(jira_category("new"), "todo");
        assert_eq!(jira_category("indeterminate"), "in_progress");
        assert_eq!(jira_category("done"), "done");
        assert_eq!(jira_time("2024-03-05T10:20:30.000+0000").to_rfc3339(), "2024-03-05T10:20:30+00:00");
    }

    #[test]
    fn chat_payload_shapes() {
        let e = json!({"subject": "OPS-1 <b>Pump</b>", "summary": "Ada created OPS-1 & more", "url": "https://gantry.example.org/i/OPS-1"});
        let s = slack_message(&e);
        assert_eq!(s["text"], "Ada created OPS-1 & more");
        assert_eq!(s["blocks"][0]["text"]["text"], "*<https://gantry.example.org/i/OPS-1|OPS-1 &lt;b&gt;Pump&lt;/b&gt;>*\nAda created OPS-1 &amp; more");
        let t = teams_card(&e);
        assert_eq!(t["type"], "message");
        assert_eq!(t["attachments"][0]["contentType"], "application/vnd.microsoft.card.adaptive");
        assert_eq!(t["attachments"][0]["content"]["type"], "AdaptiveCard");
        assert_eq!(t["attachments"][0]["content"]["actions"][0]["url"], "https://gantry.example.org/i/OPS-1");
        let no_url = teams_card(&json!({"subject": "s", "summary": "t", "url": "/i/OPS-1"}));
        assert!(no_url["attachments"][0]["content"].get("actions").is_none(), "relative links are not valid card actions");
    }

    #[test]
    fn kind_catalog_is_consistent() {
        for k in KINDS {
            assert!(kind_spec(k.kind).is_some() && !k.description.is_empty());
        }
        assert!(kind_spec("irc").is_none());
        for required in ["slack", "teams", "github", "gitlab", "jira", "webhook"] {
            assert!(kind_spec(required).is_some(), "{required}");
        }
    }
}
