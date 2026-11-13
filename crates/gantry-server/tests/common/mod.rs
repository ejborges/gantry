//! Test harness: each test gets its own PostgreSQL database and a real server on a loopback port.
//!
//! Set `GANTRY_TEST_DATABASE_URL` to a superuser connection to any PostgreSQL 15+ server
//! (default `postgres://postgres@127.0.0.1:5433/postgres`; `dev/test-db.sh` starts one).
#![allow(dead_code)]

use gantry::{config::Config, crypto, AppState, St};
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use std::net::SocketAddr;
use uuid::Uuid;

pub const PASSWORD: &str = "correct horse battery staple";

/// Everything a project member and space editor does, for agents in tests that are about
/// something other than agent rules. Which projects the agent reaches is still decided by its roles.
pub fn working_agent() -> Value {
    json!({"mode": "restricted", "all_projects": true, "all_spaces": true, "permissions": [
        "workspace.access", "project.view", "item.create", "item.edit", "item.transition", "item.assign", "item.comment", "item.link",
        "space.view", "page.edit", "desk.request"]})
}

pub fn admin_url() -> String {
    std::env::var("GANTRY_TEST_DATABASE_URL").unwrap_or_else(|_| "postgres://postgres@127.0.0.1:5433/postgres".into())
}

pub struct TestApp {
    pub st: St,
    pub base: String,
    pub db_name: String,
    pub http: reqwest::Client,
    tmp: std::path::PathBuf,
}

impl Drop for TestApp {
    fn drop(&mut self) {
        let (name, tmp) = (self.db_name.clone(), self.tmp.clone());
        // Best effort: drop the per-test database from a separate runtime.
        let _ = std::thread::spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
            rt.block_on(async {
                if let Ok(pool) = sqlx::postgres::PgPoolOptions::new().max_connections(1).connect(&admin_url()).await {
                    let _ = sqlx::query(&format!("DROP DATABASE IF EXISTS {name} WITH (FORCE)")).execute(&pool).await;
                }
            });
            let _ = std::fs::remove_dir_all(tmp);
        })
        .join();
    }
}

pub async fn spawn() -> TestApp {
    spawn_with(|_| {}).await
}

pub async fn spawn_with(tweak: impl FnOnce(&mut Config)) -> TestApp {
    let db_name = format!("gantry_test_{}", crypto::random_hex(8));
    let admin = sqlx::postgres::PgPoolOptions::new().max_connections(1).connect(&admin_url()).await.expect("connect to the test PostgreSQL server (see tests/common/mod.rs)");
    sqlx::query(&format!("CREATE DATABASE {db_name}")).execute(&admin).await.expect("create test database");
    admin.close().await;
    let tmp = std::env::temp_dir().join(&db_name);
    std::fs::create_dir_all(tmp.join("web")).unwrap();
    std::fs::write(tmp.join("web/index.html"), "<!doctype html><title>Gantry</title><div id=root></div>").unwrap();

    let mut cfg = Config::default();
    let base_db = admin_url();
    cfg.database.url = format!("{}/{db_name}", base_db.rsplit_once('/').unwrap().0);
    cfg.database.max_connections = 8;
    cfg.security.secret_key = crypto::SecretBox::generate_key();
    cfg.security.allow_insecure_outbound = true;
    cfg.server.web_root = tmp.join("web");
    cfg.server.manual_root = tmp.join("manual");
    cfg.server.rate_limit_per_minute = 100_000;
    cfg.storage.blob.path = tmp.join("blobs");
    cfg.ai.embeddings.provider = "hash".into();
    cfg.addons.dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../addons");
    cfg.worker.embedded = false;
    cfg.auth.login_attempts_per_ip = 1000;
    tweak(&mut cfg);

    let db = gantry::connect_db(&cfg).await.unwrap();
    gantry::migrate(&db).await.unwrap();
    let st = AppState::init(cfg, db).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    let router = gantry::http::build_router(st.clone());
    tokio::spawn(async move {
        axum::serve(listener, router.into_make_service_with_connect_info::<SocketAddr>()).await.unwrap();
    });
    let http = reqwest::Client::builder().redirect(reqwest::redirect::Policy::none()).build().unwrap();
    TestApp { st, base: format!("http://{addr}"), db_name, http, tmp }
}

/// An authenticated caller: a browser session (cookie + CSRF) or an API token.
#[derive(Clone)]
pub struct Caller {
    pub base: String,
    pub http: reqwest::Client,
    pub cookie: Option<String>,
    pub csrf: Option<String>,
    pub token: Option<String>,
    pub user_id: Uuid,
}

impl Caller {
    pub fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let mut r = self.http.request(method.clone(), format!("{}{path}", self.base));
        if let Some(t) = &self.token {
            r = r.bearer_auth(t);
        }
        if let Some(c) = &self.cookie {
            r = r.header("cookie", c);
            if method != Method::GET {
                if let Some(csrf) = &self.csrf {
                    r = r.header("x-gantry-csrf", csrf);
                }
            }
        }
        r
    }

    pub async fn send(&self, method: Method, path: &str, body: Option<Value>) -> (StatusCode, Value) {
        let mut r = self.request(method, path);
        if let Some(b) = body {
            r = r.json(&b);
        }
        let resp = r.send().await.expect("request");
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }
    pub async fn get(&self, path: &str) -> (StatusCode, Value) {
        self.send(Method::GET, path, None).await
    }
    pub async fn post(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(Method::POST, path, Some(body)).await
    }
    pub async fn patch(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(Method::PATCH, path, Some(body)).await
    }
    pub async fn put(&self, path: &str, body: Value) -> (StatusCode, Value) {
        self.send(Method::PUT, path, Some(body)).await
    }
    pub async fn delete(&self, path: &str) -> (StatusCode, Value) {
        self.send(Method::DELETE, path, None).await
    }
    /// GET that must succeed.
    pub async fn ok(&self, path: &str) -> Value {
        let (s, v) = self.get(path).await;
        assert_eq!(s, StatusCode::OK, "GET {path}: {v}");
        v
    }
    /// POST that must create.
    pub async fn created(&self, path: &str, body: Value) -> Value {
        let (s, v) = self.post(path, body).await;
        assert_eq!(s, StatusCode::CREATED, "POST {path}: {v}");
        v
    }
    /// Write the rules for an AI agent account (the caller must be an administrator).
    /// New agents may do nothing until this is called.
    pub async fn agent_rules(&self, agent_id: &str, policy: Value) {
        let (status, body) = self.put(&format!("/api/v1/admin/users/{agent_id}/agent-policy"), policy).await;
        assert_eq!(status, StatusCode::OK, "agent policy: {body}");
    }

    /// Send a raw body (a file upload).
    pub async fn upload(&self, method: Method, path: &str, content_type: &str, body: Vec<u8>) -> (StatusCode, Value) {
        let resp = self.request(method, path).header("content-type", content_type).body(body).send().await.expect("request");
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        (status, serde_json::from_str(&text).unwrap_or(Value::String(text)))
    }

    /// Same account, authenticated with an API token of the given scopes.
    pub async fn with_token(&self, scopes: &[&str]) -> Caller {
        let v = self.created("/api/v1/tokens", json!({"name": "test", "scopes": scopes})).await;
        Caller { cookie: None, csrf: None, token: Some(v["token"].as_str().unwrap().to_string()), ..self.clone() }
    }
}

impl TestApp {
    pub fn anon(&self) -> Caller {
        Caller { base: self.base.clone(), http: self.http.clone(), cookie: None, csrf: None, token: None, user_id: Uuid::nil() }
    }

    /// Insert a user with a known password and a global role, directly in the database.
    pub async fn make_user(&self, email: &str, role: &str) -> Uuid {
        static HASH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
        let hash = HASH.get_or_init(|| crypto::hash_password(PASSWORD));
        let name = email.split('@').next().unwrap();
        let id: Uuid = sqlx::query_scalar("INSERT INTO users (email, name, password_hash) VALUES ($1, $2, $3) RETURNING id").bind(email).bind(name).bind(hash).fetch_one(&self.st.db).await.unwrap();
        sqlx::query("INSERT INTO role_bindings (role_id, user_id, scope_type) SELECT id, $1, 'global' FROM roles WHERE key = $2").bind(id).bind(role).execute(&self.st.db).await.unwrap();
        id
    }

    pub async fn login(&self, email: &str) -> Caller {
        self.login_with(email, PASSWORD).await.expect("login")
    }

    pub async fn login_with(&self, email: &str, password: &str) -> Result<Caller, StatusCode> {
        let resp = self.http.post(format!("{}/api/v1/auth/login", self.base)).json(&json!({"email": email, "password": password})).send().await.unwrap();
        if resp.status() != StatusCode::OK {
            return Err(resp.status());
        }
        let cookie = resp.headers().get("set-cookie").unwrap().to_str().unwrap().split(';').next().unwrap().to_string();
        let mut c = Caller { base: self.base.clone(), http: self.http.clone(), cookie: Some(cookie), csrf: None, token: None, user_id: Uuid::nil() };
        let me = c.ok("/api/v1/auth/me").await;
        c.csrf = Some(me["csrf"].as_str().unwrap().to_string());
        c.user_id = me["user"]["id"].as_str().unwrap().parse().unwrap();
        Ok(c)
    }

    /// Create a user with a global role and sign them in.
    pub async fn user(&self, email: &str, role: &str) -> Caller {
        self.make_user(email, role).await;
        self.login(email).await
    }

    pub async fn admin(&self) -> Caller {
        self.user("root@example.org", "system_admin").await
    }

    pub async fn drain_jobs(&self) -> u32 {
        gantry::jobs::drain(&self.st).await.unwrap()
    }

    pub async fn audit_actions(&self) -> Vec<String> {
        sqlx::query_scalar("SELECT action FROM audit_log ORDER BY id").fetch_all(&self.st.db).await.unwrap()
    }

    pub async fn security_kinds(&self) -> Vec<String> {
        // Denials are recorded from a spawned task; give it a moment.
        tokio::time::sleep(std::time::Duration::from_millis(60)).await;
        sqlx::query_scalar("SELECT kind FROM security_events ORDER BY id").fetch_all(&self.st.db).await.unwrap()
    }
}

/// Start a throwaway HTTP server that records every request it receives.
pub async fn recorder() -> (String, std::sync::Arc<tokio::sync::Mutex<Vec<(axum::http::HeaderMap, Vec<u8>)>>>) {
    use axum::routing::any;
    let seen = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let s2 = seen.clone();
    let app = axum::Router::new().fallback(any(move |headers: axum::http::HeaderMap, body: axum::body::Bytes| {
        let s = s2.clone();
        async move {
            s.lock().await.push((headers, body.to_vec()));
            "ok"
        }
    }));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    (format!("http://{addr}"), seen)
}

/// A mail server for tests: accepts every message without TLS or sign-in and keeps what it
/// was given (envelope lines, then headers and body). Returns its port.
pub async fn mail_sink() -> (u16, std::sync::Arc<tokio::sync::Mutex<Vec<String>>>) {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let store = std::sync::Arc::new(tokio::sync::Mutex::new(Vec::<String>::new()));
    let kept = store.clone();
    tokio::spawn(async move {
        while let Ok((sock, _)) = listener.accept().await {
            let kept = kept.clone();
            tokio::spawn(async move {
                let (r, mut w) = sock.into_split();
                let mut lines = BufReader::new(r).lines();
                let _ = w.write_all(b"220 sink ESMTP\r\n").await;
                let (mut envelope, mut data, mut in_data) = (String::new(), String::new(), false);
                while let Ok(Some(line)) = lines.next_line().await {
                    if in_data {
                        if line == "." {
                            in_data = false;
                            kept.lock().await.push(format!("{envelope}\n{data}"));
                            envelope.clear();
                            data.clear();
                            let _ = w.write_all(b"250 queued\r\n").await;
                        } else {
                            data.push_str(line.strip_prefix('.').filter(|_| line.starts_with("..")).unwrap_or(&line));
                            data.push('\n');
                        }
                        continue;
                    }
                    let upper = line.to_ascii_uppercase();
                    let reply: &[u8] = if upper.starts_with("EHLO") || upper.starts_with("HELO") {
                        b"250-sink\r\n250 8BITMIME\r\n"
                    } else if upper.starts_with("MAIL FROM") || upper.starts_with("RCPT TO") {
                        envelope.push_str(&line);
                        envelope.push('\n');
                        b"250 ok\r\n"
                    } else if upper == "DATA" {
                        in_data = true;
                        b"354 go ahead\r\n"
                    } else if upper == "QUIT" {
                        let _ = w.write_all(b"221 bye\r\n").await;
                        break;
                    } else {
                        b"250 ok\r\n"
                    };
                    let _ = w.write_all(reply).await;
                }
            });
        }
    });
    (port, store)
}

/// Point a test server at a [`mail_sink`].
pub fn use_mail(cfg: &mut Config, port: u16) {
    cfg.email.enabled = true;
    cfg.email.host = "127.0.0.1".into();
    cfg.email.port = port;
    cfg.email.encryption = "none".into();
    cfg.email.from = "Gantry <gantry@example.org>".into();
    cfg.server.public_url = "http://gantry.test".into();
    // No embedding jobs, so the queue holds only what the test is about.
    cfg.ai.embeddings.provider = "none".into();
}

/// A real-time connection as a browser tab (or an API client) would hold it.
pub struct Ws {
    stream: tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    pub conn: String,
}

impl Ws {
    /// Open the socket. `origin` is what a browser would send; `None` sends no Origin header.
    pub async fn try_connect(app: &TestApp, caller: &Caller, origin: Option<&str>) -> Result<Ws, u16> {
        Ws::try_connect_at(&app.base, caller, origin).await
    }

    /// The same against a given node of the installation.
    pub async fn try_connect_at(base: &str, caller: &Caller, origin: Option<&str>) -> Result<Ws, u16> {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        let mut req = format!("{}/api/v1/ws", base.replace("http://", "ws://")).into_client_request().unwrap();
        let h = req.headers_mut();
        if let Some(c) = &caller.cookie {
            h.insert("cookie", c.parse().unwrap());
        }
        if let Some(t) = &caller.token {
            h.insert("authorization", format!("Bearer {t}").parse().unwrap());
        }
        if let Some(o) = origin {
            h.insert("origin", o.parse().unwrap());
        }
        match tokio_tungstenite::connect_async(req).await {
            Ok((stream, _)) => {
                let mut ws = Ws { stream, conn: String::new() };
                let hello = ws.until(|m| m["t"] == "hello").await;
                ws.conn = hello["conn"].as_str().unwrap().to_string();
                Ok(ws)
            }
            Err(tokio_tungstenite::tungstenite::Error::Http(resp)) => Err(resp.status().as_u16()),
            Err(e) => panic!("websocket: {e}"),
        }
    }

    /// Connect the way the application's own page does.
    pub async fn connect(app: &TestApp, caller: &Caller) -> Ws {
        Ws::try_connect(app, caller, caller.cookie.as_ref().map(|_| app.base.as_str())).await.expect("websocket connects")
    }

    pub async fn send(&mut self, v: Value) {
        use futures_util::SinkExt;
        self.stream.send(tokio_tungstenite::tungstenite::Message::Text(v.to_string().into())).await.unwrap();
    }

    /// Send, reporting failure instead of panicking (the server may have closed the socket).
    pub async fn try_send(&mut self, v: Value) -> Result<(), String> {
        use futures_util::SinkExt;
        self.stream.send(tokio_tungstenite::tungstenite::Message::Text(v.to_string().into())).await.map_err(|e| e.to_string())
    }

    /// Send a frame exactly as given.
    pub async fn send_raw(&mut self, m: tokio_tungstenite::tungstenite::Message) {
        use futures_util::SinkExt;
        let _ = self.stream.send(m).await;
    }

    /// Whether the server closes the connection within `ms` milliseconds.
    pub async fn closed(&mut self, ms: u64) -> bool {
        use futures_util::StreamExt;
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_millis(ms);
        loop {
            match tokio::time::timeout_at(deadline, self.stream.next()).await {
                Ok(None) | Ok(Some(Err(_))) | Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))) => return true,
                Ok(Some(Ok(_))) => continue,
                Err(_) => return false,
            }
        }
    }

    /// The next JSON message, or `None` if nothing arrives within `ms`.
    pub async fn next(&mut self, ms: u64) -> Option<Value> {
        use futures_util::StreamExt;
        loop {
            match tokio::time::timeout(std::time::Duration::from_millis(ms), self.stream.next()).await {
                Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Text(t)))) => return serde_json::from_str(t.as_str()).ok(),
                Ok(Some(Ok(tokio_tungstenite::tungstenite::Message::Close(_)))) | Ok(None) | Ok(Some(Err(_))) => return None,
                Ok(Some(Ok(_))) => continue,
                Err(_) => return None,
            }
        }
    }

    /// Read until a message satisfies `want`; panic if none does within three seconds.
    pub async fn until(&mut self, want: impl Fn(&Value) -> bool) -> Value {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        let mut seen = vec![];
        while std::time::Instant::now() < deadline {
            match self.next(300).await {
                Some(m) if want(&m) => return m,
                Some(m) => seen.push(m),
                None => {}
            }
        }
        panic!("the expected message did not arrive; saw {seen:?}");
    }

    /// Everything that arrives in the next `ms` milliseconds.
    pub async fn drain(&mut self, ms: u64) -> Vec<Value> {
        let mut out = vec![];
        while let Some(m) = self.next(ms).await {
            out.push(m);
        }
        out
    }

    pub async fn subscribe(&mut self, room: &str) -> Value {
        self.send(json!({"t": "sub", "room": room})).await;
        let r = room.to_string();
        self.until(move |m| (m["t"] == "joined" || m["t"] == "denied") && m["room"] == r.as_str()).await
    }
}
