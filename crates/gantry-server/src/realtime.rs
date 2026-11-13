//! Live updates, presence and concurrent editing over one WebSocket per browser tab.
//!
//! A client subscribes to *rooms* (`item:OPS-12`, `project:OPS`, `page:<id>`, …). Subscribing is
//! authorized like reading the object, and is checked again every minute, so a revoked
//! permission stops the stream. Rooms carry three kinds of traffic:
//!
//! * **events** — "this changed" hints. They name the object and never contain its content;
//!   the client refetches through the ordinary API, so the stream cannot reveal anything the
//!   API would not.
//! * **presence** — who else has the object open and which field or line they are on.
//! * **shared documents** — Yjs updates for text that several people edit at once. The server
//!   stores and forwards the updates without interpreting them; sending one needs the same
//!   permission as saving the object.
//!
//! With several server nodes, every message is also written to `rt_messages` and announced with
//! PostgreSQL `NOTIFY`; each node delivers the rows other nodes wrote. A single node skips this.

use crate::auth::{AuthMethod, Ctx};
use crate::error::{AppError, AppResult};
use crate::http::Api;
use crate::{audit, authz, items, projects, wiki, AppState, St};
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::State;
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use sqlx::Row;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::{mpsc, watch};
use uuid::Uuid;

const MAX_ROOMS_PER_CONNECTION: usize = 64;
const MAX_UPDATE_BYTES: usize = 1024 * 1024;
const MAX_DOC_BYTES: i64 = 16 * 1024 * 1024;
const MAX_PRESENCE_BYTES: usize = 2048;
/// How long the connection chosen to load saved content into an empty shared document has to do it.
const SEED_SECONDS: f64 = 10.0;

struct Conn {
    user_id: Uuid,
    tx: mpsc::Sender<Arc<str>>,
    /// Projects this connection may see, for rooms that mix many projects (`work`).
    projects: Arc<Mutex<HashSet<Uuid>>>,
}

#[derive(Default)]
struct Inner {
    conns: HashMap<String, Conn>,
    /// room → connection id → presence state.
    rooms: HashMap<String, HashMap<String, Value>>,
}

pub struct Hub {
    inner: Mutex<Inner>,
    next: AtomicU64,
    started: AtomicBool,
    /// True when another server node has reported in recently.
    pub peers: AtomicBool,
    /// True in a process that accepts browser connections.
    pub serving: AtomicBool,
    stop: watch::Sender<bool>,
}

impl Default for Hub {
    fn default() -> Self {
        Hub { inner: Mutex::default(), next: AtomicU64::new(1), started: AtomicBool::new(false), peers: AtomicBool::new(false), serving: AtomicBool::new(false), stop: watch::channel(false).0 }
    }
}

impl Hub {
    /// Close every connection (graceful shutdown).
    pub fn close_all(&self) {
        let _ = self.stop.send(true);
    }

    pub fn connections(&self) -> usize {
        self.inner.lock().expect("hub lock").conns.len()
    }

    fn connections_of(&self, user: Uuid) -> usize {
        self.inner.lock().expect("hub lock").conns.values().filter(|c| c.user_id == user).count()
    }

    /// Hand `text` to every local subscriber of `room`, except `skip`. When `project` is given,
    /// only to connections that may see that project.
    fn fan_out(&self, room: &str, text: &Arc<str>, skip: Option<&str>, project: Option<Uuid>) {
        let inner = self.inner.lock().expect("hub lock");
        let Some(members) = inner.rooms.get(room) else { return };
        for id in members.keys().filter(|id| Some(id.as_str()) != skip) {
            if let Some(c) = inner.conns.get(id) {
                if project.is_some_and(|p| !c.projects.lock().expect("projects lock").contains(&p)) {
                    continue;
                }
                // A client that cannot keep up loses messages rather than stalling the room;
                // it resynchronizes when it next refetches.
                let _ = c.tx.try_send(text.clone());
            }
        }
    }
}

/// Send `payload` to everyone subscribed to `room`, on this node and the others.
pub async fn publish(st: &AppState, room: &str, payload: Value) {
    send(st, room, payload, None).await;
}

/// Tell one account's open tabs something (for example that a notification arrived).
pub async fn publish_user(st: &AppState, user: Uuid, payload: Value) {
    send(st, &format!("user:{user}"), payload, None).await;
}

/// A "this changed" hint about something in a project, for the room of the object itself and
/// for the catch-all `work` room (filtered to people who can see the project).
pub async fn publish_work(st: &AppState, project_id: Uuid, project_key: &str, item_key: Option<&str>, kind: &str, data: Value) {
    let mut payload = json!({"t": "event", "kind": kind, "data": data});
    if let Some(k) = item_key {
        payload["room"] = json!(format!("item:{k}"));
        publish(st, &format!("item:{k}"), payload.clone()).await;
    }
    payload["room"] = json!(format!("project:{project_key}"));
    publish(st, &format!("project:{project_key}"), payload.clone()).await;
    payload["room"] = json!("work");
    payload["project_id"] = json!(project_id);
    publish(st, "work", payload).await;
}

async fn send(st: &AppState, room: &str, payload: Value, skip: Option<&str>) {
    let project = payload.get("project_id").and_then(Value::as_str).and_then(|s| s.parse().ok());
    let text: Arc<str> = Arc::from(payload.to_string());
    st.hub.fan_out(room, &text, skip, project);
    let relay = !st.hub.serving.load(Ordering::Relaxed) || st.hub.peers.load(Ordering::Relaxed);
    if relay {
        let r = sqlx::query("WITH m AS (INSERT INTO rt_messages (node, room, payload) VALUES ($1, $2, $3) RETURNING id) SELECT pg_notify('gantry_rt', id::text) FROM m").bind(&st.node_id).bind(room).bind(&payload).execute(&st.db).await;
        if let Err(e) = r {
            tracing::warn!(target: "realtime", error = %e, room, "could not relay a message to other nodes");
        }
    }
}

/// Start the per-process background duties once: the cross-node listener, the node heartbeat
/// and the metric samples that system alerts are computed from.
pub fn start_background(st: &St) {
    st.hub.serving.store(true, Ordering::Relaxed);
    if st.hub.started.swap(true, Ordering::SeqCst) {
        return;
    }
    let listener_st = st.clone();
    tokio::spawn(async move {
        let st = listener_st;
        let mut stop = st.hub.stop.subscribe();
        loop {
            let run = async {
                let mut l = sqlx::postgres::PgListener::connect_with(&st.db).await?;
                l.listen("gantry_rt").await?;
                loop {
                    let n = l.recv().await?;
                    let Ok(id) = n.payload().parse::<i64>() else { continue };
                    if let Some(row) = sqlx::query("SELECT room, payload FROM rt_messages WHERE id = $1 AND node <> $2").bind(id).bind(&st.node_id).fetch_optional(&st.db).await? {
                        let payload: Value = row.get("payload");
                        let project = payload.get("project_id").and_then(Value::as_str).and_then(|s| s.parse().ok());
                        st.hub.fan_out(&row.get::<String, _>("room"), &Arc::from(payload.to_string()), None, project);
                    }
                }
                #[allow(unreachable_code)]
                Ok::<(), sqlx::Error>(())
            };
            tokio::select! {
                r = run => {
                    if let Err(e) = r {
                        tracing::warn!(target: "realtime", error = %e, "lost the connection that listens for other nodes; reconnecting");
                    }
                }
                _ = stop.changed() => break,
            }
            tokio::time::sleep(Duration::from_secs(2)).await;
        }
    });
    let beat_st = st.clone();
    tokio::spawn(async move {
        let st = beat_st;
        let mut stop = st.hub.stop.subscribe();
        let mut tick = tokio::time::interval(Duration::from_secs(60));
        let mut last = Sample::default();
        loop {
            tokio::select! {
                _ = tick.tick() => {
                    if let Err(e) = heartbeat(&st, "server", &mut last).await {
                        tracing::warn!(target: "realtime", error = %e, "node heartbeat failed");
                    }
                }
                _ = stop.changed() => break,
            }
        }
        let _ = sqlx::query("DELETE FROM cluster_nodes WHERE node = $1").bind(&st.node_id).execute(&st.db).await;
    });
}

#[derive(Default)]
pub struct Sample {
    requests: u64,
    server_errors: u64,
    client_errors: u64,
    duration_micros: u64,
    rate_limited: u64,
    audit_failures: u64,
}

/// Report this node as alive, learn whether others are, and store the last minute's counters.
pub async fn heartbeat(st: &AppState, role: &str, last: &mut Sample) -> sqlx::Result<()> {
    sqlx::query("INSERT INTO cluster_nodes (node, role, version) VALUES ($1, $2, $3) ON CONFLICT (node) DO UPDATE SET seen_at = now(), role = EXCLUDED.role, version = EXCLUDED.version").bind(&st.node_id).bind(role).bind(crate::VERSION).execute(&st.db).await?;
    let others: i64 = sqlx::query_scalar("SELECT count(*) FROM cluster_nodes WHERE node <> $1 AND role = 'server' AND seen_at > now() - interval '3 minutes'").bind(&st.node_id).fetch_one(&st.db).await?;
    st.hub.peers.store(others > 0, Ordering::Relaxed);
    if role != "server" {
        return Ok(());
    }
    let m = &st.metrics;
    let now = Sample {
        requests: m.requests.load(Ordering::Relaxed),
        server_errors: m.server_errors.load(Ordering::Relaxed),
        client_errors: m.client_errors.load(Ordering::Relaxed),
        duration_micros: m.duration_micros.load(Ordering::Relaxed),
        rate_limited: m.rate_limited.load(Ordering::Relaxed),
        audit_failures: m.audit_failures.load(Ordering::Relaxed),
    };
    let pool_percent = 100.0 * (st.db.size() as f64 - st.db.num_idle() as f64).max(0.0) / st.cfg.database.max_connections.max(1) as f64;
    let rows: [(&str, f64); 8] = [
        ("requests", now.requests.saturating_sub(last.requests) as f64),
        ("server_errors", now.server_errors.saturating_sub(last.server_errors) as f64),
        ("client_errors", now.client_errors.saturating_sub(last.client_errors) as f64),
        ("duration_micros", now.duration_micros.saturating_sub(last.duration_micros) as f64),
        ("rate_limited", now.rate_limited.saturating_sub(last.rate_limited) as f64),
        ("audit_failures", now.audit_failures.saturating_sub(last.audit_failures) as f64),
        ("db_pool_percent", pool_percent),
        ("ws_connections", st.hub.connections() as f64),
    ];
    *last = now;
    let (names, values): (Vec<&str>, Vec<f64>) = rows.iter().cloned().unzip();
    sqlx::query("INSERT INTO metric_samples (at, node, name, value) SELECT date_trunc('minute', now()), $1, n, v FROM unnest($2::text[], $3::float8[]) AS t (n, v)").bind(&st.node_id).bind(&names).bind(&values).execute(&st.db).await?;
    Ok(())
}

// ───────────────────────── authorization ─────────────────────────

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Access {
    pub read: bool,
    pub write: bool,
}

const NONE: Access = Access { read: false, write: false };

/// What `ctx` may do in `room`. Unknown rooms and rooms about things the caller cannot see are
/// indistinguishable: both answer "no".
pub async fn authorize(st: &St, ctx: &Ctx, room: &str) -> AppResult<Access> {
    let (kind, id) = room.split_once(':').unwrap_or((room, ""));
    Ok(match kind {
        "user" => Access { read: id == ctx.user_id.to_string(), write: false },
        "work" | "directory" | "teams" | "planning" if id.is_empty() => Access { read: ctx.has("workspace.access"), write: false },
        "item" => {
            let Some(item) = items::find_item(st, id).await? else { return Ok(NONE) };
            let project = projects::project_by_id(st, item.project_id).await?;
            let perms = authz::project_perms(st, ctx, project.id, &project.access, project.portal_open()).await?;
            if perms.has("project.view") {
                Access { read: true, write: perms.has("item.edit") && ctx.token_allows("item.edit") }
            } else {
                // The person who raised a request follows it through the portal.
                Access { read: perms.has("desk.request") && item.reporter_id == Some(ctx.user_id), write: false }
            }
        }
        "project" => match projects::find_project(st, id).await? {
            Some(p) => {
                let perms = authz::project_perms(st, ctx, p.id, &p.access, p.portal_open()).await?;
                Access { read: perms.has("project.view"), write: perms.has("project.admin") && ctx.token_allows("project.admin") }
            }
            None => NONE,
        },
        "page" => match id.parse::<Uuid>() {
            Ok(pid) => match wiki::page_access(st, ctx, pid).await? {
                Some((view, edit)) => Access { read: view, write: view && edit && ctx.token_allows("page.edit") },
                None => NONE,
            },
            Err(_) => NONE,
        },
        "space" => {
            let row = sqlx::query("SELECT id, access FROM spaces WHERE key = $1 AND NOT archived").bind(id.to_uppercase()).fetch_optional(&st.db).await?;
            match row {
                Some(r) => {
                    let perms = authz::space_perms(st, ctx, r.get("id"), &r.get::<String, _>("access")).await?;
                    Access { read: perms.has("space.view"), write: perms.has("space.admin") }
                }
                None => NONE,
            }
        }
        "dashboard" | "viz" | "gantt" => match id.parse::<Uuid>() {
            Ok(oid) => crate::analytics::room_access(st, ctx, kind, oid).await?,
            Err(_) => NONE,
        },
        "team" => Access { read: ctx.has("workspace.access") && id.parse::<Uuid>().is_ok(), write: false },
        _ => NONE,
    })
}

// ───────────────────────── shared documents ─────────────────────────

fn valid_doc(doc: &str) -> bool {
    !doc.is_empty() && doc.len() <= 40 && doc.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

/// Discard the working copy of a shared document and tell everyone who has it open to reload.
/// Called when the saved content changes by another route (the API, a restore, an import).
pub async fn reset_doc(st: &AppState, room: &str, doc: &str) {
    match sqlx::query("DELETE FROM collab_docs WHERE room = $1 AND doc = $2").bind(room).bind(doc).execute(&st.db).await {
        Ok(r) if r.rows_affected() > 0 => publish(st, room, json!({"t": "yreset", "room": room, "doc": doc})).await,
        Ok(_) => {}
        Err(e) => tracing::error!(target: "realtime", error = %e, room, "could not reset a shared document"),
    }
}

/// Record that the working copy now matches saved version `base`.
pub async fn mark_saved(st: &AppState, room: &str, doc: &str, base: i64) {
    let _ = sqlx::query("UPDATE collab_docs SET base = $3, updated_at = now() WHERE room = $1 AND doc = $2").bind(room).bind(doc).bind(base).execute(&st.db).await;
}

async fn doc_state(st: &St, conn: &str, room: &str, doc: &str, mode: &str) -> AppResult<Value> {
    sqlx::query("INSERT INTO collab_docs (room, doc, mode) VALUES ($1, $2, $3) ON CONFLICT DO NOTHING").bind(room).bind(doc).bind(mode).execute(&st.db).await?;
    let updates: Vec<(i64, Vec<u8>)> = sqlx::query_as("SELECT id, data FROM collab_updates WHERE room = $1 AND doc = $2 ORDER BY id").bind(room).bind(doc).fetch_all(&st.db).await?;
    // An empty document is filled from the saved content by exactly one connection.
    let seed = if updates.is_empty() {
        sqlx::query("UPDATE collab_docs SET seed_conn = $3, seed_until = now() + make_interval(secs => $4), mode = $5 WHERE room = $1 AND doc = $2 AND NOT seeded AND (seed_conn IS NULL OR seed_conn = $3 OR seed_until < now())")
            .bind(room)
            .bind(doc)
            .bind(conn)
            .bind(SEED_SECONDS)
            .bind(mode)
            .execute(&st.db)
            .await?
            .rows_affected()
            > 0
    } else {
        false
    };
    let row = sqlx::query("SELECT mode, base FROM collab_docs WHERE room = $1 AND doc = $2").bind(room).bind(doc).fetch_one(&st.db).await?;
    Ok(json!({
        "t": "ystate", "room": room, "doc": doc, "mode": row.get::<String, _>("mode"), "base": row.get::<i64, _>("base"), "seed": seed,
        "last": updates.last().map(|u| u.0).unwrap_or(0),
        "updates": updates.iter().map(|u| B64.encode(&u.1)).collect::<Vec<_>>(),
    }))
}

async fn store_update(st: &St, ctx: &Ctx, conn: &str, room: &str, doc: &str, data: &[u8], seeding: bool) -> AppResult<i64> {
    let total: i64 = sqlx::query_scalar("SELECT coalesce(sum(length(data)), 0)::bigint FROM collab_updates WHERE room = $1 AND doc = $2").bind(room).bind(doc).fetch_one(&st.db).await?;
    if total + data.len() as i64 > MAX_DOC_BYTES {
        return Err(AppError::BadRequest("this document's edit history is full; save it and reopen the editor".into()));
    }
    if seeding {
        let ok = sqlx::query("UPDATE collab_docs SET seeded = true, seed_conn = NULL WHERE room = $1 AND doc = $2 AND NOT seeded AND seed_conn = $3").bind(room).bind(doc).bind(conn).execute(&st.db).await?.rows_affected() > 0;
        if !ok {
            return Err(AppError::Conflict("another editor already loaded this document".into()));
        }
    }
    let id: i64 = sqlx::query_scalar("INSERT INTO collab_updates (room, doc, data, author_id) VALUES ($1, $2, $3, $4) RETURNING id").bind(room).bind(doc).bind(data).bind(ctx.user_id).fetch_optional(&st.db).await.map_err(|_| AppError::Conflict("the shared document was reset; reload it".into()))?.ok_or(AppError::NotFound)?;
    sqlx::query("UPDATE collab_docs SET seeded = true, updated_at = now() WHERE room = $1 AND doc = $2 AND NOT seeded").bind(room).bind(doc).execute(&st.db).await?;
    Ok(id)
}

/// Replace every stored update up to `upto` with one merged update supplied by a client that
/// has applied them all.
async fn compact(st: &St, ctx: &Ctx, room: &str, doc: &str, upto: i64, data: &[u8]) -> AppResult<()> {
    let mut tx = st.db.begin().await?;
    let n = sqlx::query("DELETE FROM collab_updates WHERE room = $1 AND doc = $2 AND id <= $3").bind(room).bind(doc).bind(upto).execute(&mut *tx).await?.rows_affected();
    if n > 0 {
        sqlx::query("INSERT INTO collab_updates (room, doc, data, author_id) VALUES ($1, $2, $3, $4)").bind(room).bind(doc).bind(data).bind(ctx.user_id).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    Ok(())
}

// ───────────────────────── the connection ─────────────────────────

async fn person(st: &AppState, id: Uuid) -> Value {
    sqlx::query_scalar("SELECT person_json(u) FROM users u WHERE u.id = $1").bind(id).fetch_optional(&st.db).await.ok().flatten().unwrap_or(Value::Null)
}

/// True while the session or token the connection was opened with is still good.
async fn still_authenticated(st: &AppState, ctx: &Ctx) -> bool {
    let q = match &ctx.auth {
        AuthMethod::Session { id, .. } => sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM sessions s JOIN users u ON u.id = s.user_id WHERE s.id = $1 AND s.expires_at > now() AND u.active)").bind(*id),
        AuthMethod::Token { id, .. } => sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM api_tokens t JOIN users u ON u.id = t.user_id WHERE t.id = $1 AND t.revoked_at IS NULL AND (t.expires_at IS NULL OR t.expires_at > now()) AND u.active)").bind(*id),
    };
    q.fetch_one(&st.db).await.unwrap_or(false)
}

async fn upgrade(State(st): State<St>, ctx: Ctx, headers: HeaderMap, ws: WebSocketUpgrade) -> Response {
    if !st.cfg.realtime.enabled {
        return AppError::NotFound.into_response();
    }
    // A page on another site must not be able to open this socket with the visitor's cookie.
    // Browsers always send Origin on a WebSocket handshake; a session without one is refused.
    if ctx.is_session() && (headers.get(axum::http::header::ORIGIN).is_none() || !crate::http::origin_ok(&st, &headers)) {
        audit::security(&st, "ws.origin_rejected", "warning", Some((ctx.user_id, &ctx.email)), &ctx.req, json!({"origin": headers.get(axum::http::header::ORIGIN).and_then(|v| v.to_str().ok()).map(|s| s.chars().take(200).collect::<String>())})).await;
        return AppError::Forbidden("this connection must be opened from the application itself".into()).into_response();
    }
    if st.hub.connections_of(ctx.user_id) >= st.cfg.realtime.max_connections_per_user {
        return AppError::RateLimited.into_response();
    }
    ws.max_message_size(2 * MAX_UPDATE_BYTES).max_frame_size(2 * MAX_UPDATE_BYTES).on_upgrade(move |socket| run(st, ctx, socket))
}

struct Session {
    st: St,
    ctx: Ctx,
    id: String,
    me: Value,
    /// room → may write.
    rooms: HashMap<String, bool>,
    projects: Arc<Mutex<HashSet<Uuid>>>,
    tx: mpsc::Sender<Arc<str>>,
}

impl Session {
    fn reply(&self, v: Value) {
        let _ = self.tx.try_send(Arc::from(v.to_string()));
    }

    async fn refresh_projects(&self) {
        if let Ok(ids) = authz::visible_project_ids(&self.st, &self.ctx).await {
            *self.projects.lock().expect("projects lock") = ids.into_iter().collect();
        }
    }

    async fn leave(&mut self, room: &str) {
        if self.rooms.remove(room).is_none() {
            return;
        }
        {
            let mut inner = self.st.hub.inner.lock().expect("hub lock");
            if let Some(m) = inner.rooms.get_mut(room) {
                m.remove(&self.id);
                if m.is_empty() {
                    inner.rooms.remove(room);
                }
            }
        }
        // Give back a seeding duty this connection had not carried out.
        let _ = sqlx::query("UPDATE collab_docs SET seed_conn = NULL WHERE room = $1 AND seed_conn = $2 AND NOT seeded").bind(room).bind(&self.id).execute(&self.st.db).await;
        send(&self.st, room, json!({"t": "left", "room": room, "conn": self.id}), Some(&self.id)).await;
    }

    async fn handle(&mut self, msg: Value) -> AppResult<()> {
        let room = msg["room"].as_str().unwrap_or_default().to_string();
        if room.is_empty() || room.len() > 120 {
            return Err(AppError::BadRequest("room is required".into()));
        }
        match msg["t"].as_str().unwrap_or_default() {
            "sub" => {
                if self.rooms.len() >= MAX_ROOMS_PER_CONNECTION && !self.rooms.contains_key(&room) {
                    return Err(AppError::BadRequest("too many subscriptions on one connection".into()));
                }
                let access = authorize(&self.st, &self.ctx, &room).await?;
                if !access.read {
                    self.reply(json!({"t": "denied", "room": room}));
                    return Ok(());
                }
                if room == "work" {
                    self.refresh_projects().await;
                }
                self.rooms.insert(room.clone(), access.write);
                let peers: Vec<Value> = {
                    let mut inner = self.st.hub.inner.lock().expect("hub lock");
                    let members = inner.rooms.entry(room.clone()).or_default();
                    members.insert(self.id.clone(), json!({"user": self.me, "state": {}}));
                    members.iter().filter(|(id, _)| **id != self.id).map(|(id, v)| json!({"conn": id, "user": v["user"], "state": v["state"]})).collect()
                };
                self.reply(json!({"t": "joined", "room": room, "conn": self.id, "write": access.write, "peers": peers}));
                // Presence is only meaningful (and only shared) in rooms about one object.
                if room.contains(':') && !room.starts_with("user:") {
                    send(&self.st, &room, json!({"t": "peer", "room": room, "conn": self.id, "user": self.me, "state": {}}), Some(&self.id)).await;
                    if self.st.hub.peers.load(Ordering::Relaxed) {
                        send(&self.st, &room, json!({"t": "who", "room": room}), Some(&self.id)).await;
                    }
                }
            }
            "unsub" => self.leave(&room).await,
            other => {
                let Some(&can_write) = self.rooms.get(&room) else {
                    return Err(AppError::Forbidden("subscribe to the room first".into()));
                };
                match other {
                    "presence" => {
                        let state = msg.get("state").cloned().unwrap_or(json!({}));
                        if state.to_string().len() > MAX_PRESENCE_BYTES || !state.is_object() {
                            return Err(AppError::BadRequest("presence state is too large".into()));
                        }
                        if let Some(m) = self.st.hub.inner.lock().expect("hub lock").rooms.get_mut(&room).and_then(|r| r.get_mut(&self.id)) {
                            m["state"] = state.clone();
                        }
                        send(&self.st, &room, json!({"t": "peer", "room": room, "conn": self.id, "user": self.me, "state": state}), Some(&self.id)).await;
                    }
                    "ysync" => {
                        let doc = msg["doc"].as_str().filter(|d| valid_doc(d)).ok_or_else(|| AppError::BadRequest("doc is required".into()))?;
                        let mode = msg["mode"].as_str().filter(|m| ["text", "rich"].contains(m)).unwrap_or("text");
                        let state = doc_state(&self.st, &self.id, &room, doc, mode).await?;
                        // A viewer may follow a document being edited but is never asked to load it.
                        let mut state = state;
                        if !can_write {
                            state["seed"] = json!(false);
                            let _ = sqlx::query("UPDATE collab_docs SET seed_conn = NULL WHERE room = $1 AND doc = $2 AND seed_conn = $3 AND NOT seeded").bind(&room).bind(doc).bind(&self.id).execute(&self.st.db).await;
                        }
                        self.reply(state);
                    }
                    "y" | "ycompact" => {
                        if !can_write {
                            let _ = authz::deny(&self.st, &self.ctx, "edit", &format!("room:{room}"));
                            return Err(AppError::Forbidden("you can view this but not edit it".into()));
                        }
                        let doc = msg["doc"].as_str().filter(|d| valid_doc(d)).ok_or_else(|| AppError::BadRequest("doc is required".into()))?;
                        let data = msg["u"].as_str().and_then(|u| B64.decode(u).ok()).filter(|d| !d.is_empty() && d.len() <= MAX_UPDATE_BYTES).ok_or_else(|| AppError::BadRequest("update is missing or too large".into()))?;
                        if other == "y" {
                            let id = store_update(&self.st, &self.ctx, &self.id, &room, doc, &data, msg["seed"] == true).await?;
                            send(&self.st, &room, json!({"t": "y", "room": room, "doc": doc, "u": msg["u"], "id": id, "from": self.id}), Some(&self.id)).await;
                            self.reply(json!({"t": "yack", "room": room, "doc": doc, "id": id}));
                        } else {
                            let upto = msg["upto"].as_i64().ok_or_else(|| AppError::BadRequest("upto is required".into()))?;
                            compact(&self.st, &self.ctx, &room, doc, upto, &data).await?;
                        }
                    }
                    "ydiscard" => {
                        // Throw away what was typed and not saved, for everyone: the next editor
                        // starts again from the saved content. The text may be someone else's,
                        // so this needs the right to edit and is written to the audit log.
                        if !can_write {
                            let _ = authz::deny(&self.st, &self.ctx, "edit", &format!("room:{room}"));
                            return Err(AppError::Forbidden("you can view this but not edit it".into()));
                        }
                        let doc = msg["doc"].as_str().filter(|d| valid_doc(d)).ok_or_else(|| AppError::BadRequest("doc is required".into()))?;
                        let removed = sqlx::query("DELETE FROM collab_docs WHERE room = $1 AND doc = $2").bind(&room).bind(doc).execute(&self.st.db).await?.rows_affected();
                        if removed > 0 {
                            audit::log(&self.st, &self.ctx, "draft.discard", "shared_document", &format!("{room}/{doc}"), json!({})).await;
                            send(&self.st, &room, json!({"t": "yreset", "room": room, "doc": doc, "from": self.id}), None).await;
                        }
                        self.reply(json!({"t": "ydiscarded", "room": room, "doc": doc}));
                    }
                    "ya" => {
                        // Cursor positions and selections. Forwarded, never stored.
                        let doc = msg["doc"].as_str().filter(|d| valid_doc(d)).ok_or_else(|| AppError::BadRequest("doc is required".into()))?;
                        if msg["u"].as_str().is_none_or(|u| u.len() > 16 * 1024) {
                            return Err(AppError::BadRequest("awareness update is missing or too large".into()));
                        }
                        send(&self.st, &room, json!({"t": "ya", "room": room, "doc": doc, "u": msg["u"], "from": self.id}), Some(&self.id)).await;
                    }
                    _ => return Err(AppError::BadRequest("unknown message type".into())),
                }
            }
        }
        Ok(())
    }

    /// Drop subscriptions the account is no longer entitled to. False when the sign-in itself ended.
    async fn revalidate(&mut self) -> bool {
        if !still_authenticated(&self.st, &self.ctx).await {
            return false;
        }
        // Permissions are read again, not taken from when the socket was opened.
        if let Ok(Some(fresh)) = crate::auth::refresh_ctx(&self.st, &self.ctx).await {
            self.ctx = fresh;
        }
        for room in self.rooms.keys().cloned().collect::<Vec<_>>() {
            match authorize(&self.st, &self.ctx, &room).await {
                Ok(a) if a.read => {
                    self.rooms.insert(room, a.write);
                }
                _ => {
                    self.leave(&room).await;
                    self.reply(json!({"t": "denied", "room": room}));
                }
            }
        }
        if self.rooms.contains_key("work") {
            self.refresh_projects().await;
        }
        true
    }
}

async fn run(st: St, ctx: Ctx, socket: WebSocket) {
    let id = format!("{}-{}", st.node_id, st.hub.next.fetch_add(1, Ordering::Relaxed));
    let (tx, mut rx) = mpsc::channel::<Arc<str>>(256);
    let projects = Arc::new(Mutex::new(HashSet::new()));
    st.hub.inner.lock().expect("hub lock").conns.insert(id.clone(), Conn { user_id: ctx.user_id, tx: tx.clone(), projects: projects.clone() });
    tracing::info!(target: "realtime", conn = %id, user = %ctx.email, ip = %ctx.req.ip, "connection opened");
    let me = person(&st, ctx.user_id).await;
    let mut s = Session { st: st.clone(), ctx, id: id.clone(), me, rooms: HashMap::new(), projects, tx };
    let own = format!("user:{}", s.ctx.user_id);
    s.rooms.insert(own.clone(), false);
    st.hub.inner.lock().expect("hub lock").rooms.entry(own).or_default().insert(id.clone(), json!({}));
    s.reply(json!({"t": "hello", "conn": id, "user": s.me}));

    let (mut sink, mut stream) = socket.split();
    let mut stop = st.hub.stop.subscribe();
    let mut check = tokio::time::interval(Duration::from_secs(60));
    check.tick().await;
    let mut ping = tokio::time::interval(Duration::from_secs(25));
    let (mut window, mut count) = (Instant::now(), 0u32);
    let limit = st.cfg.realtime.max_messages_per_second * 10;
    let reason = loop {
        tokio::select! {
            out = rx.recv() => {
                let Some(text) = out else { break "closed" };
                if sink.send(Message::Text(text.as_ref().into())).await.is_err() {
                    break "send failed";
                }
            }
            incoming = stream.next() => {
                let Some(Ok(frame)) = incoming else { break "closed by the client" };
                let text = match frame {
                    Message::Text(t) => t,
                    Message::Close(_) => break "closed by the client",
                    _ => continue,
                };
                // A ten-second window keeps bursts (pasting, fast typing) from tripping the limit.
                if window.elapsed() > Duration::from_secs(10) {
                    (window, count) = (Instant::now(), 0);
                }
                count += 1;
                if count > limit {
                    audit::security(&st, "ws.flood", "warning", Some((s.ctx.user_id, &s.ctx.email)), &s.ctx.req, json!({"messages": count})).await;
                    break "too many messages";
                }
                let Ok(msg) = serde_json::from_str::<Value>(text.as_str()) else {
                    s.reply(json!({"t": "error", "detail": "messages must be JSON"}));
                    continue;
                };
                if let Err(e) = s.handle(msg.clone()).await {
                    if matches!(e, AppError::Db(_) | AppError::Other(_)) {
                        tracing::error!(target: "realtime", conn = %s.id, error = ?e, "message failed");
                    }
                    s.reply(json!({"t": "error", "room": msg["room"], "doc": msg["doc"], "of": msg["t"], "detail": e.public_message()}));
                }
            }
            _ = check.tick() => {
                if !s.revalidate().await {
                    break "sign-in ended";
                }
            }
            _ = ping.tick() => {
                if sink.send(Message::Ping(Default::default())).await.is_err() {
                    break "send failed";
                }
            }
            _ = stop.changed() => break "server shutting down",
        }
    };
    let _ = sink.send(Message::Close(None)).await;
    for room in s.rooms.keys().cloned().collect::<Vec<_>>() {
        s.leave(&room).await;
    }
    st.hub.inner.lock().expect("hub lock").conns.remove(&s.id);
    tracing::info!(target: "realtime", conn = %s.id, user = %s.ctx.email, reason, "connection closed");
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/ws", "Real time", "WebSocket for live updates, presence and concurrent editing", get(upgrade))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn document_names_are_plain_identifiers() {
        assert!(valid_doc("content") && valid_doc("description") && valid_doc("field_2"));
        for bad in ["", "Content", "a b", "a/b", "../x", "a;b", &"x".repeat(41)] {
            assert!(!valid_doc(bad), "{bad:?}");
        }
    }

    #[test]
    fn fan_out_reaches_subscribers_only_and_respects_project_visibility() {
        let hub = Hub::default();
        let mut receivers = vec![];
        let (seen, hidden) = (Uuid::new_v4(), Uuid::new_v4());
        {
            let mut inner = hub.inner.lock().unwrap();
            for (id, room, visible) in [("a", "work", vec![seen]), ("b", "work", vec![]), ("c", "item:OPS-1", vec![seen])] {
                let (tx, rx) = mpsc::channel(8);
                inner.conns.insert(id.into(), Conn { user_id: Uuid::nil(), tx, projects: Arc::new(Mutex::new(visible.into_iter().collect())) });
                inner.rooms.entry(room.into()).or_default().insert(id.into(), json!({}));
                receivers.push((id, rx));
            }
        }
        let text: Arc<str> = Arc::from("x");
        hub.fan_out("work", &text, None, Some(seen));
        hub.fan_out("work", &text, None, Some(hidden));
        hub.fan_out("item:OPS-1", &text, Some("c"), None);
        let counts: HashMap<&str, usize> = receivers.iter_mut().map(|(id, rx)| (*id, std::iter::from_fn(|| rx.try_recv().ok()).count())).collect();
        assert_eq!(counts["a"], 1, "sees the project, gets the event once");
        assert_eq!(counts["b"], 0, "cannot see either project");
        assert_eq!(counts["c"], 0, "the sender is skipped");
    }
}
