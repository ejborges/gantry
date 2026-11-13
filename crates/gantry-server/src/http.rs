//! HTTP plumbing: request context, authentication hook, CSRF, rate limiting, security headers,
//! structured request logging, health and metrics endpoints, static files, and the route table.

use crate::auth::{self, AuthMethod};
use crate::error::AppError;
use crate::{audit, crypto, St};
use axum::extract::{ConnectInfo, DefaultBodyLimit, FromRequestParts, Request, State};
use axum::http::{header, request::Parts, HeaderMap, HeaderValue, Method, StatusCode};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, MethodRouter};
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::convert::Infallible;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tower_http::catch_panic::CatchPanicLayer;
use tower_http::services::ServeDir;

/// Facts about the current request that handlers and logs need.
#[derive(Clone, Debug, Default)]
pub struct ReqInfo {
    pub id: String,
    pub ip: String,
    pub user_agent: String,
    pub method: String,
    pub path: String,
}

impl<S: Send + Sync> FromRequestParts<S> for ReqInfo {
    type Rejection = Infallible;
    async fn from_request_parts(parts: &mut Parts, _: &S) -> Result<Self, Infallible> {
        Ok(parts.extensions.get::<ReqInfo>().cloned().unwrap_or_default())
    }
}

#[derive(Default)]
pub struct Metrics {
    pub requests: AtomicU64,
    pub client_errors: AtomicU64,
    pub server_errors: AtomicU64,
    pub duration_micros: AtomicU64,
    pub rate_limited: AtomicU64,
    pub auth_failures: AtomicU64,
    pub audit_failures: AtomicU64,
    pub security_events: AtomicU64,
    pub jobs_done: AtomicU64,
    pub jobs_failed: AtomicU64,
}

/// Fixed-window limiter held in process memory. Each node enforces its own window; sign-in
/// throttling, which must be shared across nodes, uses the `Kv` port instead.
#[derive(Default)]
pub struct RateLimiter {
    windows: Mutex<HashMap<String, (u64, u32)>>,
}

impl RateLimiter {
    pub fn allow(&self, key: &str, limit: u32) -> bool {
        self.allow_at(key, limit, SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0) / 60)
    }
    pub fn allow_at(&self, key: &str, limit: u32, minute: u64) -> bool {
        let mut w = self.windows.lock().expect("rate limiter lock");
        if w.len() > 50_000 {
            w.retain(|_, (m, _)| *m == minute);
        }
        let e = w.entry(key.to_string()).or_insert((minute, 0));
        if e.0 != minute {
            *e = (minute, 0);
        }
        e.1 += 1;
        e.1 <= limit
    }
}

/// Offset pagination shared by list endpoints.
///
/// The fields accept a number or a numeric string. Query structs embed this with
/// `#[serde(flatten)]`, and a flattened struct receives every query value as text, so a
/// plain `Option<i64>` rejects `?limit=50`.
#[derive(Debug, Default, Deserialize)]
pub struct Paging {
    #[serde(default, deserialize_with = "lenient_i64")]
    pub limit: Option<i64>,
    #[serde(default, deserialize_with = "lenient_i64")]
    pub offset: Option<i64>,
}

fn lenient_i64<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<i64>, D::Error> {
    struct V;
    impl serde::de::Visitor<'_> for V {
        type Value = Option<i64>;
        fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
            f.write_str("a whole number")
        }
        fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Self::Value, E> {
            Ok(Some(v))
        }
        fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Self::Value, E> {
            Ok(Some(i64::try_from(v).unwrap_or(i64::MAX)))
        }
        fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Self::Value, E> {
            if v.is_empty() {
                return Ok(None);
            }
            v.trim().parse().map(Some).map_err(|_| E::custom(format!("'{}' is not a whole number", v.chars().take(24).collect::<String>())))
        }
        fn visit_none<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
        fn visit_unit<E: serde::de::Error>(self) -> Result<Self::Value, E> {
            Ok(None)
        }
    }
    d.deserialize_any(V)
}
impl Paging {
    pub fn get(&self) -> (i64, i64) {
        (self.limit.unwrap_or(50).clamp(1, 200), self.offset.unwrap_or(0).max(0))
    }
}

#[derive(Clone, Debug)]
pub struct RouteDoc {
    pub method: &'static str,
    pub path: &'static str,
    pub tag: &'static str,
    pub summary: &'static str,
    pub public: bool,
}

/// Route table builder. Every API route is registered through it, which gives one list that
/// drives the OpenAPI document and the "every private route rejects anonymous callers" test.
#[derive(Default)]
pub struct Api {
    pub router: Router<St>,
    pub docs: Vec<RouteDoc>,
}

impl Api {
    pub fn route(mut self, method: &'static str, path: &'static str, tag: &'static str, summary: &'static str, handler: MethodRouter<St>) -> Self {
        self.router = self.router.route(path, handler);
        self.docs.push(RouteDoc { method, path, tag, summary, public: false });
        self
    }
    /// Mark the most recently added route as reachable without authentication.
    pub fn public(mut self) -> Self {
        if let Some(d) = self.docs.last_mut() {
            d.public = true;
        }
        self
    }
}

pub fn all_routes() -> Api {
    let api = Api::default();
    let api = auth::routes(api);
    let api = crate::accounts::routes(api);
    let api = crate::oidc::routes(api);
    let api = crate::saml::routes(api);
    let api = crate::teams::routes(api);
    let api = crate::notify::routes(api);
    let api = crate::refs::routes(api);
    let api = crate::realtime::routes(api);
    let api = crate::analytics::routes(api);
    let api = crate::planning::routes(api);
    let api = crate::about::routes(api);
    let api = crate::pages::routes(api);
    let api = crate::projects::routes(api);
    let api = crate::items::routes(api);
    let api = crate::desk::routes(api);
    let api = crate::wiki::routes(api);
    let api = crate::search::routes(api);
    let api = crate::admin::routes(api);
    let api = crate::integrations::routes(api);
    let api = crate::scim::routes(api);
    crate::mcp::routes(api)
}

/// Whether `ip` is the address `entry`, or inside the network `entry` when it is written
/// as `address/prefix-length`. `None` when the entry is not an address or network at all.
pub fn net_contains(entry: &str, ip: IpAddr) -> Option<bool> {
    let (addr, prefix) = match entry.trim().split_once('/') {
        Some((a, p)) => (a.parse::<IpAddr>().ok()?, Some(p.parse::<u32>().ok()?)),
        None => (entry.trim().parse::<IpAddr>().ok()?, None),
    };
    // An IPv4 client reaching a dual-stack listener appears as ::ffff:a.b.c.d.
    let unmap = |i: IpAddr| match i {
        IpAddr::V6(v6) => v6.to_ipv4_mapped().map(IpAddr::V4).unwrap_or(IpAddr::V6(v6)),
        v4 => v4,
    };
    let (addr, ip) = (unmap(addr), unmap(ip));
    Some(match (addr, ip) {
        (IpAddr::V4(n), IpAddr::V4(i)) => {
            let bits = prefix.unwrap_or(32);
            if bits > 32 {
                return None;
            }
            let mask = if bits == 0 { 0 } else { u32::MAX << (32 - bits) };
            u32::from(n) & mask == u32::from(i) & mask
        }
        (IpAddr::V6(n), IpAddr::V6(i)) => {
            let bits = prefix.unwrap_or(128);
            if bits > 128 {
                return None;
            }
            let mask = if bits == 0 { 0 } else { u128::MAX << (128 - bits) };
            u128::from(n) & mask == u128::from(i) & mask
        }
        _ => false,
    })
}

fn client_ip(st: &St, peer: Option<IpAddr>, headers: &HeaderMap) -> String {
    let Some(peer) = peer else { return "unknown".into() };
    let trusted = st.cfg.server.trusted_proxies.iter().any(|entry| net_contains(entry, peer) == Some(true));
    if trusted {
        let forwarded = headers
            .get("x-real-ip")
            .and_then(|v| v.to_str().ok())
            .map(str::trim)
            .or_else(|| headers.get("x-forwarded-for").and_then(|v| v.to_str().ok()).and_then(|v| v.rsplit(',').next()).map(str::trim));
        if let Some(ip) = forwarded.and_then(|v| v.parse::<IpAddr>().ok()) {
            return ip.to_string();
        }
    }
    peer.to_string()
}

pub fn origin_ok(st: &St, headers: &HeaderMap) -> bool {
    let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) else { return true };
    if let Some(expected) = st.cfg.public_origin() {
        return origin == expected;
    }
    let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default();
    origin.split_once("://").map(|(_, h)| h == host).unwrap_or(false)
}

/// Marks a response that is meant to be shown inside a frame on the application's own pages
/// (the sandbox that runs custom content). Such a response sets its own, stricter policy.
#[derive(Clone)]
pub struct Frameable;

/// The policy for the application's own pages. Scripts come only from this server; frames may
/// show only this server's sandbox document, which is how custom HTML and visualization
/// scripts are kept away from the session; the WebSocket goes to this server only.
pub fn content_security_policy(cfg: &crate::config::Config) -> String {
    // `'self'` covers the WebSocket in current browsers; older Safari needs the host spelled out.
    let ws = url::Url::parse(&cfg.server.public_url).ok().and_then(|u| Some(format!(" {}://{}", if u.scheme() == "https" { "wss" } else { "ws" }, u.host_str().map(|h| match u.port() { Some(p) => format!("{h}:{p}"), None => h.to_string() })?))).unwrap_or_default();
    format!("default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; media-src 'self' blob:; font-src 'self' data:; connect-src 'self'{ws}; worker-src 'self' blob:; frame-src 'self'; frame-ancestors 'none'; base-uri 'self'; form-action 'self'; object-src 'none'")
}

/// The framework's own wording for a request it could not read, put the way the rest of the
/// API speaks: what was wrong with the request, without the names of the program's types.
pub fn plain_words(text: &str) -> String {
    if text.is_empty() {
        return "the request could not be read".into();
    }
    if text.starts_with("Invalid URL") || text.starts_with("Cannot parse") {
        return "the address contains an id or number that is not valid".into();
    }
    let mut out = text.to_string();
    for (from, to) in [
        ("Failed to deserialize the JSON body into the target type: ", "the request body is not what this address expects: "),
        ("Failed to parse the request body as JSON: ", "the request body is not valid JSON: "),
        ("Failed to deserialize query string: ", "the query string is not what this address expects: "),
        ("Failed to deserialize form body: ", "the form is not what this address expects: "),
        ("Failed to deserialize form: ", "the form is not what this address expects: "),
        ("Failed to buffer the request body: ", "the request body could not be read: "),
    ] {
        out = out.replace(from, to);
    }
    crate::error::reword(&out)
}

async fn context_mw(State(st): State<St>, mut req: Request, next: Next) -> Response {
    let started = Instant::now();
    let method = req.method().clone();
    let path = req.uri().path().to_string();
    let headers = req.headers();
    let id = headers
        .get("x-request-id")
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty() && v.len() <= 64 && v.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_'))
        .map(str::to_string)
        .unwrap_or_else(|| crypto::random_hex(8));
    let peer = req.extensions().get::<ConnectInfo<SocketAddr>>().map(|c| c.0.ip());
    let info = ReqInfo {
        id: id.clone(),
        ip: client_ip(&st, peer, headers),
        user_agent: headers.get(header::USER_AGENT).and_then(|v| v.to_str().ok()).unwrap_or_default().chars().take(300).collect(),
        method: method.to_string(),
        path: path.clone(),
    };
    let is_api = path.starts_with("/api/") || path == "/mcp" || path.starts_with("/scim/");
    let navigation = crate::pages::is_navigation(&method, headers);
    let unsafe_method = !matches!(method, Method::GET | Method::HEAD | Method::OPTIONS);
    let mut user = String::new();
    let mut early: Option<Response> = None;

    if is_api {
        match auth::authenticate(&st, headers, &info).await {
            Ok(Some(ctx)) => {
                user = ctx.email.clone();
                let reject = match &ctx.auth {
                    AuthMethod::Session { csrf, .. } if unsafe_method => {
                        let sent = headers.get("x-gantry-csrf").map(|v| v.as_bytes()).unwrap_or_default();
                        (!origin_ok(&st, headers) || !crypto::ct_eq(sent, csrf.as_bytes())).then_some(("csrf.rejected", "missing or invalid CSRF token"))
                    }
                    AuthMethod::Token { .. } if unsafe_method && path != "/mcp" && !ctx.can_write() => Some(("authz.read_only_token", "this token is read-only")),
                    _ => None,
                };
                // Requests made on behalf of a script in the sandbox are marked by the page that
                // makes them. They may read and nothing else, whatever the page was talked into.
                let reject = if unsafe_method && headers.contains_key("x-gantry-sandbox") { Some(("sandbox.write_refused", "custom content can read data but not change it")) } else { reject };
                if let Some((kind, msg)) = reject {
                    audit::security(&st, kind, "warning", Some((ctx.user_id, &ctx.email)), &info, json!({"method": method.as_str()})).await;
                    early = Some(AppError::Forbidden(msg.into()).into_response());
                } else if !st.limiter.allow(&format!("u:{}", ctx.user_id), ctx.policy.as_ref().and_then(|p| p.rate_limit).unwrap_or(st.cfg.server.rate_limit_per_minute)) {
                    early = Some(AppError::RateLimited.into_response());
                } else {
                    req.extensions_mut().insert(ctx);
                }
            }
            Ok(None) => {
                if !st.limiter.allow(&format!("ip:{}", info.ip), st.cfg.server.rate_limit_per_minute) {
                    early = Some(AppError::RateLimited.into_response());
                }
            }
            Err(e) => {
                st.metrics.auth_failures.fetch_add(1, Ordering::Relaxed);
                early = Some(e.into_response());
            }
        }
    }
    req.extensions_mut().insert(info.clone());

    let mut resp = match early {
        Some(r) => r,
        None => next.run(req).await,
    };

    let status = resp.status();
    // Rejections raised by the framework before a handler runs (a malformed query string or
    // body) arrive as plain text. API clients are promised problem documents for every error.
    if is_api && status.is_client_error() && resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/plain")) {
        let (parts, body) = resp.into_parts();
        let text = axum::body::to_bytes(body, 2048).await.map(|b| String::from_utf8_lossy(&b).into_owned()).unwrap_or_default();
        let mut problem = (
            parts.status,
            Json(json!({
                "type": "about:blank",
                "title": parts.status.canonical_reason().unwrap_or("Error"),
                "status": parts.status.as_u16(),
                "code": "bad_request",
                "detail": plain_words(&text),
            })),
        )
            .into_response();
        problem.headers_mut().insert(header::CONTENT_TYPE, HeaderValue::from_static("application/problem+json"));
        resp = problem;
    }
    // A person who opened the address in a browser gets a page they can read, with the same
    // status code: what happened, what to do, and how to reach support.
    if (status.is_client_error() || status.is_server_error()) && navigation && !resp.headers().get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).is_some_and(|v| v.starts_with("text/html")) {
        resp = crate::pages::page(&st, status, &id, !user.is_empty()).await;
    }
    if status == StatusCode::TOO_MANY_REQUESTS {
        st.metrics.rate_limited.fetch_add(1, Ordering::Relaxed);
        resp.headers_mut().insert(header::RETRY_AFTER, HeaderValue::from_static("60"));
        audit::security(&st, "rate_limit.exceeded", "info", None, &info, json!({"user": user})).await;
    }
    if let Ok(v) = HeaderValue::from_str(&id) {
        resp.headers_mut().insert("x-request-id", v);
    }
    resp.headers_mut().insert("x-content-type-options", HeaderValue::from_static("nosniff"));
    let frameable = resp.extensions().get::<Frameable>().is_some();
    let h = resp.headers_mut();
    if !frameable {
        h.insert("x-frame-options", HeaderValue::from_static("DENY"));
    }
    h.insert("referrer-policy", HeaderValue::from_static("same-origin"));
    h.insert("permissions-policy", HeaderValue::from_static("camera=(), microphone=(), geolocation=(), payment=(), usb=()"));
    if !h.contains_key(header::CONTENT_SECURITY_POLICY) {
        if let Ok(v) = HeaderValue::from_str(&st.csp) {
            h.insert(header::CONTENT_SECURITY_POLICY, v);
        }
    }
    if st.cfg.secure_cookies() {
        h.insert(header::STRICT_TRANSPORT_SECURITY, HeaderValue::from_static("max-age=31536000; includeSubDomains"));
    }
    if is_api {
        h.entry(header::CACHE_CONTROL).or_insert(HeaderValue::from_static("no-store"));
    }

    let micros = started.elapsed().as_micros() as u64;
    st.metrics.requests.fetch_add(1, Ordering::Relaxed);
    st.metrics.duration_micros.fetch_add(micros, Ordering::Relaxed);
    if status.is_client_error() {
        st.metrics.client_errors.fetch_add(1, Ordering::Relaxed);
    } else if status.is_server_error() {
        st.metrics.server_errors.fetch_add(1, Ordering::Relaxed);
    }
    // The query string is never logged: it can carry authorization codes and search text.
    if is_api {
        tracing::info!(target: "http", request_id = %id, method = %method, path = %path, status = status.as_u16(), duration_ms = micros as f64 / 1000.0, user = %user, ip = %info.ip, "request");
    } else {
        tracing::debug!(target: "http", request_id = %id, method = %method, path = %path, status = status.as_u16(), "request");
    }
    resp
}

async fn readyz(State(st): State<St>) -> Response {
    match sqlx::query("SELECT 1").execute(&st.db).await {
        Ok(_) if crate::schema_is_current(&st.db).await => (StatusCode::OK, "ready").into_response(),
        Ok(_) => (StatusCode::SERVICE_UNAVAILABLE, "schema out of date: run `gantry migrate`").into_response(),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "database unavailable").into_response(),
    }
}

async fn metrics(State(st): State<St>, ConnectInfo(peer): ConnectInfo<SocketAddr>, headers: HeaderMap) -> Response {
    let token = &st.cfg.server.metrics_token;
    let authorized = if token.is_empty() {
        peer.ip().is_loopback() && !headers.contains_key("x-real-ip") && !headers.contains_key("x-forwarded-for")
    } else {
        headers.get(header::AUTHORIZATION).and_then(|v| v.to_str().ok()).and_then(|v| v.strip_prefix("Bearer ")).map(|t| crypto::ct_eq(t.as_bytes(), token.as_bytes())).unwrap_or(false)
    };
    if !authorized {
        return StatusCode::NOT_FOUND.into_response();
    }
    let m = &st.metrics;
    let g = |a: &AtomicU64| a.load(Ordering::Relaxed);
    let queued: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE state = 'queued'").fetch_one(&st.db).await.unwrap_or(-1);
    let failed: i64 = sqlx::query_scalar("SELECT count(*) FROM jobs WHERE state = 'failed'").fetch_one(&st.db).await.unwrap_or(-1);
    let body = format!(
        "# TYPE gantry_http_requests_total counter\ngantry_http_requests_total {}\n\
         # TYPE gantry_http_client_errors_total counter\ngantry_http_client_errors_total {}\n\
         # TYPE gantry_http_server_errors_total counter\ngantry_http_server_errors_total {}\n\
         # TYPE gantry_http_request_duration_seconds_sum counter\ngantry_http_request_duration_seconds_sum {:.6}\n\
         # TYPE gantry_rate_limited_total counter\ngantry_rate_limited_total {}\n\
         # TYPE gantry_auth_failures_total counter\ngantry_auth_failures_total {}\n\
         # TYPE gantry_audit_write_failures_total counter\ngantry_audit_write_failures_total {}\n\
         # TYPE gantry_security_events_total counter\ngantry_security_events_total {}\n\
         # TYPE gantry_jobs_done_total counter\ngantry_jobs_done_total {}\n\
         # TYPE gantry_jobs_failed_total counter\ngantry_jobs_failed_total {}\n\
         # TYPE gantry_jobs_queued gauge\ngantry_jobs_queued {}\n\
         # TYPE gantry_jobs_dead gauge\ngantry_jobs_dead {}\n\
         # TYPE gantry_db_pool_connections gauge\ngantry_db_pool_connections {}\n\
         # TYPE gantry_uptime_seconds gauge\ngantry_uptime_seconds {}\n",
        g(&m.requests),
        g(&m.client_errors),
        g(&m.server_errors),
        g(&m.duration_micros) as f64 / 1e6,
        g(&m.rate_limited),
        g(&m.auth_failures),
        g(&m.audit_failures),
        g(&m.security_events),
        g(&m.jobs_done),
        g(&m.jobs_failed),
        queued,
        failed,
        st.db.size(),
        st.started.elapsed().as_secs()
    );
    ([(header::CONTENT_TYPE, "text/plain; version=0.0.4")], body).into_response()
}

/// OpenAPI 3.1 document generated from the route table.
pub fn openapi(docs: &[RouteDoc]) -> Value {
    let mut paths = serde_json::Map::new();
    for d in docs {
        let params: Vec<Value> = d
            .path
            .split('/')
            .filter_map(|s| s.strip_prefix('{').and_then(|s| s.strip_suffix('}')))
            .map(|name| json!({"name": name, "in": "path", "required": true, "schema": {"type": "string"}}))
            .collect();
        let mut op = json!({"summary": d.summary, "tags": [d.tag], "parameters": params, "responses": {"default": {"description": "JSON result, or an RFC 9457 problem document on error"}}});
        op["security"] = if d.public { json!([]) } else { json!([{"bearerToken": []}, {"sessionCookie": []}]) };
        paths.entry(d.path.to_string()).or_insert_with(|| json!({})).as_object_mut().expect("path item is an object").insert(d.method.to_lowercase(), op);
    }
    json!({
        "openapi": "3.1.0",
        "info": {"title": "Gantry API", "version": crate::VERSION, "description": "Request and response bodies are documented in the web manual under Reference → REST API."},
        "paths": paths,
        "components": {"securitySchemes": {
            "bearerToken": {"type": "http", "scheme": "bearer", "description": "API token: gty_<prefix>_<secret>"},
            "sessionCookie": {"type": "apiKey", "in": "cookie", "name": "gantry_session", "description": "Browser session; unsafe methods also need the X-Gantry-CSRF header"}
        }}
    })
}

async fn api_not_found() -> AppError {
    AppError::NotFound
}

/// The application's page, for every address the web application handles itself. When the
/// public address is configured, the link-preview image gets an absolute address, which the
/// sites that draw previews require.
async fn spa_index(State(st): State<St>) -> Response {
    let Ok(mut html) = tokio::fs::read_to_string(st.cfg.server.web_root.join("index.html")).await else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let base = crate::pages::escape(st.cfg.server.public_url.trim_end_matches('/'));
    if !base.is_empty() {
        html = html.replace("content=\"/og.png\"", &format!("content=\"{base}/og.png\"")).replace("<meta property=\"og:type\"", &format!("<meta property=\"og:url\" content=\"{base}/\" />\n    <meta property=\"og:type\""));
    }
    ([(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")], html).into_response()
}

pub fn build_router(st: St) -> Router {
    let api = all_routes();
    let docs = Arc::new(api.docs);
    let web_root = st.cfg.server.web_root.clone();
    let spa = ServeDir::new(&web_root).append_index_html_on_directories(false).fallback(get(spa_index).with_state(st.clone()));
    let manual = ServeDir::new(&st.cfg.server.manual_root).append_index_html_on_directories(true);
    let max_body = st.cfg.server.max_body_mb.max(1) * 1024 * 1024;
    crate::realtime::start_background(&st);
    api.router
        .route("/healthz", get(|| async { "ok" }))
        .route("/readyz", get(readyz))
        .route("/metrics", get(metrics))
        .route(
            "/api/v1/openapi.json",
            get({
                let docs = docs.clone();
                move || async move { Json(openapi(&docs)) }
            }),
        )
        .route("/api/{*rest}", any(api_not_found))
        .nest_service("/manual", manual)
        .fallback_service(spa)
        .layer(middleware::from_fn_with_state(st.clone(), context_mw))
        .layer(DefaultBodyLimit::max(max_body))
        .layer(CatchPanicLayer::new())
        .with_state(st)
}

#[cfg(test)]
mod tests {
    #[test]
    fn framework_messages_are_reworded_without_type_names() {
        assert_eq!(plain_words(""), "the request could not be read");
        assert_eq!(plain_words("Invalid URL: Cannot parse `id` with value `x`: UUID parsing failed: invalid character"), "the address contains an id or number that is not valid");
        assert_eq!(plain_words("Failed to deserialize the JSON body into the target type: invalid type: string \"x\", expected struct NewItem at line 1 column 3"), "the request body is not what this address expects: invalid type: string \"x\", expected an object at line 1 column 3");
        assert_eq!(plain_words("Failed to deserialize the JSON body into the target type: unknown field `a`, expected one of `b`, `c` at line 1 column 4"), "the request body is not what this address expects: unknown field `a`, expected one of `b`, `c` at line 1 column 4");
        assert_eq!(plain_words("Failed to deserialize query string: missing field `q`"), "the query string is not what this address expects: missing field `q`");
        assert_eq!(plain_words("Expected request with `Content-Type: application/json`"), "Expected request with `Content-Type: application/json`");
    }

    use super::*;

    #[test]
    fn trusted_proxies_may_be_addresses_or_networks() {
        let ip = |s: &str| s.parse::<IpAddr>().unwrap();
        assert_eq!(net_contains("127.0.0.1", ip("127.0.0.1")), Some(true));
        assert_eq!(net_contains("127.0.0.1", ip("127.0.0.2")), Some(false));
        assert_eq!(net_contains("10.0.0.0/8", ip("10.200.3.4")), Some(true));
        assert_eq!(net_contains("10.0.0.0/8", ip("11.0.0.1")), Some(false));
        assert_eq!(net_contains("192.168.4.0/22", ip("192.168.7.255")), Some(true));
        assert_eq!(net_contains("192.168.4.0/22", ip("192.168.8.0")), Some(false));
        assert_eq!(net_contains("0.0.0.0/0", ip("203.0.113.9")), Some(true));
        assert_eq!(net_contains("fd00::/8", ip("fd12:3456::1")), Some(true));
        assert_eq!(net_contains("fd00::/8", ip("2001:db8::1")), Some(false));
        assert_eq!(net_contains("::1", ip("::1")), Some(true));
        // An IPv4 peer seen through a dual-stack socket still matches an IPv4 entry.
        assert_eq!(net_contains("10.0.0.0/8", ip("::ffff:10.1.2.3")), Some(true));
        // An IPv4 network never matches an IPv6 peer, and nonsense is reported as such.
        assert_eq!(net_contains("10.0.0.0/8", ip("fd00::1")), Some(false));
        assert_eq!(net_contains("10.0.0.0/33", ip("10.0.0.1")), None);
        assert_eq!(net_contains("proxy.internal", ip("10.0.0.1")), None);
        assert_eq!(net_contains("", ip("10.0.0.1")), None);
    }

    #[test]
    fn paging_reads_numbers_sent_as_text() {
        #[derive(Deserialize)]
        struct Q {
            name: Option<String>,
            #[serde(flatten)]
            paging: Paging,
        }
        let q: Q = serde_urlencoded::from_str("name=a&limit=25&offset=50").unwrap();
        assert_eq!(q.name.as_deref(), Some("a"));
        assert_eq!(q.paging.get(), (25, 50));
        let q: Q = serde_urlencoded::from_str("").unwrap();
        assert_eq!(q.paging.get(), (50, 0));
        let q: Q = serde_urlencoded::from_str("limit=&offset=").unwrap();
        assert_eq!(q.paging.get(), (50, 0));
        let q: Q = serde_urlencoded::from_str("limit=100000&offset=-4").unwrap();
        assert_eq!(q.paging.get(), (200, 0));
        assert!(serde_urlencoded::from_str::<Q>("limit=ten").is_err());
        let p: Paging = serde_json::from_value(json!({"limit": 7})).unwrap();
        assert_eq!(p.get(), (7, 0));
    }

    #[test]
    fn rate_limiter_counts_per_key_and_resets_each_minute() {
        let rl = RateLimiter::default();
        assert!(rl.allow_at("a", 2, 10));
        assert!(rl.allow_at("a", 2, 10));
        assert!(!rl.allow_at("a", 2, 10));
        assert!(rl.allow_at("b", 2, 10), "other keys are unaffected");
        assert!(rl.allow_at("a", 2, 11), "a new minute starts a new window");
    }

    #[test]
    fn paging_is_clamped() {
        assert_eq!(Paging { limit: Some(100_000), offset: Some(-5) }.get(), (200, 0));
        assert_eq!(Paging::default().get(), (50, 0));
    }

    #[test]
    fn route_table_is_consistent() {
        let docs = all_routes().docs;
        assert!(docs.len() > 80, "expected a full route table, got {}", docs.len());
        let mut seen = std::collections::HashSet::new();
        for d in &docs {
            assert!(seen.insert((d.method, d.path)), "duplicate route {} {}", d.method, d.path);
            assert!(["GET", "POST", "PUT", "PATCH", "DELETE"].contains(&d.method), "bad method on {}", d.path);
            assert!(d.path.starts_with("/api/v1/") || d.path.starts_with("/scim/v2/") || d.path == "/mcp" || d.path.starts_with("/.well-known/"), "unexpected path {}", d.path);
            assert!(!d.summary.is_empty() && !d.tag.is_empty());
        }
    }

    #[test]
    fn openapi_lists_every_route_and_marks_public_ones() {
        let docs = all_routes().docs;
        let doc = openapi(&docs);
        for d in &docs {
            let op = &doc["paths"][d.path][d.method.to_lowercase()];
            assert_eq!(op["summary"], d.summary);
            assert_eq!(op["security"].as_array().unwrap().is_empty(), d.public, "{} {}", d.method, d.path);
        }
    }
}
