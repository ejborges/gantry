//! Accounts and the people side of the product: profiles and avatars, AI agent rules,
//! teams, notifications, email and alerts.

mod common;
use common::*;
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

fn png(width: u32, height: u32) -> Vec<u8> {
    let mut v = vec![0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0, 0, 0, 13, b'I', b'H', b'D', b'R'];
    v.extend_from_slice(&width.to_be_bytes());
    v.extend_from_slice(&height.to_be_bytes());
    v.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
    v
}

fn detail(v: &Value) -> &str {
    v["detail"].as_str().unwrap_or_default()
}

// ───────────────────────── profiles ─────────────────────────

#[tokio::test]
async fn profile_fields_follow_the_administrators_rules() {
    let app = spawn().await;
    let admin = app.admin().await;
    let ada = app.user("ada@example.org", "member").await;

    // Out of the box people may set their organization name and job title, not the organization code.
    let me = ada.ok("/api/v1/auth/me").await;
    assert_eq!(me["user"]["handle"], "ada");
    assert_eq!(me["user"]["kind"], "human");
    assert_eq!(me["user"]["timezone"], "auto");
    assert_eq!(me["profile_rules"]["user_editable"], json!({"name": true, "org_name": true, "org_code": false, "job_title": true}));
    let (s, v) = ada.patch("/api/v1/auth/me", json!({"org_name": "Propulsion Lab", "job_title": "Systems engineer", "timezone": "America/Chicago"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["org_name"].as_str(), v["job_title"].as_str(), v["timezone"].as_str()), (Some("Propulsion Lab"), Some("Systems engineer"), Some("America/Chicago")));
    let (s, v) = ada.patch("/api/v1/auth/me", json!({"org_code": "PL-7"})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(detail(&v).contains("managed by your administrator"));

    // The administrator sets it, and decides what people may change.
    assert_eq!(admin.patch(&format!("/api/v1/admin/users/{}", ada.user_id), json!({"org_code": "PL-7", "handle": "ada.l"})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(admin.put("/api/v1/admin/settings/profile", json!({"user_editable": {"job_title": false, "org_code": true}})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ada.patch("/api/v1/auth/me", json!({"job_title": "Chief everything"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(ada.patch("/api/v1/auth/me", json!({"org_code": "PL-8"})).await.0, StatusCode::OK);
    let me = ada.ok("/api/v1/auth/me").await;
    assert_eq!((me["user"]["handle"].as_str(), me["user"]["org_code"].as_str(), me["user"]["job_title"].as_str()), (Some("ada.l"), Some("PL-8"), Some("Systems engineer")));
    assert_eq!(me["user"]["org_name"], "Propulsion Lab", "a setting that was not mentioned keeps its value");

    // People cannot change what makes them who they are to the system.
    for field in [json!({"kind": "agent"}), json!({"active": false}), json!({"email": "x@y.zz"}), json!({"handle": "root"}), json!({"agent_policy": {"mode": "unrestricted"}}), json!({"owner_id": null}), json!({"password_hash": "x"})] {
        let (s, v) = ada.patch("/api/v1/auth/me", field.clone()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{field}: {v}");
    }
    for bad in [json!({"timezone": "Mars/Olympus"}), json!({"presence": "invisible"}), json!({"status_message": "x".repeat(141)}), json!({"theme": "neon"}), json!({"name": ""}), json!({"status_until": "2001-01-01T00:00:00Z"})] {
        assert_eq!(ada.patch("/api/v1/auth/me", bad.clone()).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    // Handles are unique.
    let ben = app.make_user("ben@example.org", "member").await;
    let (s, v) = admin.patch(&format!("/api/v1/admin/users/{ben}"), json!({"handle": "ada.l"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    // Two accounts whose addresses start the same get different handles.
    app.make_user("ben@other.example", "member").await;
    let handles: Vec<String> = sqlx::query_scalar("SELECT handle FROM users WHERE email LIKE 'ben@%' ORDER BY created_at").fetch_all(&app.st.db).await.unwrap();
    assert_eq!(handles, vec!["ben", "ben2"]);
    // A profile setting needs the right administrator.
    assert_eq!(ada.put("/api/v1/admin/settings/profile", json!({"user_editable": {"org_code": true}})).await.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn status_and_presence_are_shown_and_lapse() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    let ben = app.user("ben@example.org", "member").await;
    let until = (chrono::Utc::now() + chrono::Duration::hours(4)).to_rfc3339();
    let (s, _) = ada.patch("/api/v1/auth/me", json!({"presence": "out_of_office", "status_message": "At the test range", "status_until": until})).await;
    assert_eq!(s, StatusCode::OK);
    let find = |list: Value| list.as_array().unwrap().iter().find(|u| u["handle"] == "ada").cloned().unwrap();
    let seen = find(ben.ok("/api/v1/users").await);
    assert_eq!((seen["presence"].as_str(), seen["status_message"].as_str()), (Some("out_of_office"), Some("At the test range")));
    // Once the end time passes, everyone sees the account as available again without any job running.
    sqlx::query("UPDATE users SET status_until = now() - interval '1 minute' WHERE id = $1").bind(ada.user_id).execute(&app.st.db).await.unwrap();
    let seen = find(ben.ok("/api/v1/users").await);
    assert_eq!((seen["presence"].as_str(), seen["status_message"].as_str()), (Some("available"), Some("")));
    // The directory can be searched and filtered, and says who is a person and who is an agent.
    assert_eq!(ben.ok("/api/v1/users?q=ada").await.as_array().unwrap().len(), 1);
    assert_eq!(ben.ok("/api/v1/users?q=%25").await.as_array().unwrap().len(), 0, "a percent sign is text, not a wildcard");
    assert_eq!(ben.ok("/api/v1/users?kind=agent").await.as_array().unwrap().len(), 0);
    let profile = ben.ok(&format!("/api/v1/users/{}", ada.user_id)).await;
    assert_eq!(profile["kind"], "human");
    assert!(profile["positions"].as_array().unwrap().is_empty());
    // The audit log says that the profile changed, not what the status said.
    let logged: Value = sqlx::query_scalar("SELECT detail FROM audit_log WHERE action = 'user.profile_update' ORDER BY id DESC LIMIT 1").fetch_one(&app.st.db).await.unwrap();
    assert!(!logged.to_string().contains("test range"), "{logged}");
}

#[tokio::test]
async fn avatars_are_validated_and_accounts_without_one_get_an_identicon() {
    let app = spawn().await;
    let admin = app.admin().await;
    let ada = app.user("ada@example.org", "member").await;
    let ben = app.user("ben@example.org", "member").await;
    let avatar = |c: &Caller, id: uuid::Uuid| {
        let (c, path) = (c.clone(), format!("/api/v1/users/{id}/avatar"));
        async move {
            let r = c.request(Method::GET, &path).send().await.unwrap();
            (r.status(), r.headers().get("content-type").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default(), r.headers().get("content-security-policy").map(|v| v.to_str().unwrap().to_string()).unwrap_or_default(), r.bytes().await.unwrap().to_vec())
        }
    };
    // No picture yet: a generated one, different for each account, stable for the same one.
    let (s, ct, csp, a1) = avatar(&ben, ada.user_id).await;
    assert_eq!((s, ct.as_str()), (StatusCode::OK, "image/svg+xml"));
    assert!(csp.contains("sandbox") && csp.contains("default-src 'none'"));
    assert!(String::from_utf8_lossy(&a1).starts_with("<svg "));
    assert_eq!(avatar(&ada, ada.user_id).await.3, a1);
    assert_ne!(avatar(&ada, ben.user_id).await.3, a1);
    let me = ada.ok("/api/v1/auth/me").await;
    assert_eq!(me["profile_rules"]["avatar_suggested_px"], 256);
    assert_eq!(me["user"]["has_avatar"], false);

    // Upload: type, real content, size and dimensions are all checked.
    let (s, v) = ada.upload(Method::PUT, "/api/v1/auth/me/avatar", "image/png", png(256, 256)).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["has_avatar"], true);
    let (_, ct, _, body) = avatar(&ben, ada.user_id).await;
    assert_eq!(ct, "image/png");
    assert_eq!(body, png(256, 256));
    assert_ne!(v["avatar"], me["user"]["avatar"], "the address changes with the picture, so browsers do not show a stale one");
    for (ct, body, expect) in [
        ("image/png", png(5000, 5000), "limit is 1024×1024"),
        ("image/png", png(8, 8), "too small"),
        ("image/svg+xml", b"<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>".to_vec(), "PNG, JPEG, WebP or GIF"),
        ("text/html", b"<script>alert(1)</script>".to_vec(), "PNG, JPEG, WebP or GIF"),
        ("image/jpeg", png(100, 100), "does not match"),
        ("image/png", b"<html><script>alert(1)</script></html>".to_vec(), "not a readable picture"),
        ("image/png", vec![], "empty"),
    ] {
        let (s, v) = ada.upload(Method::PUT, "/api/v1/auth/me/avatar", ct, body).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{ct}");
        assert!(detail(&v).contains(expect), "{ct}: {v}");
    }
    let mut big = png(200, 200);
    big.resize(700 * 1024, 0);
    assert!(detail(&ada.upload(Method::PUT, "/api/v1/auth/me/avatar", "image/png", big).await.1).contains("512 KB"));
    // The administrator can tighten the limits…
    assert_eq!(admin.put("/api/v1/admin/settings/profile", json!({"avatar_max_px": 128})).await.0, StatusCode::NO_CONTENT);
    assert!(detail(&ada.upload(Method::PUT, "/api/v1/auth/me/avatar", "image/png", png(256, 256)).await.1).contains("128×128"));
    // …and remove a picture that should not be there.
    assert_eq!(admin.patch(&format!("/api/v1/admin/users/{}", ada.user_id), json!({"remove_avatar": true})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(avatar(&ben, ada.user_id).await.1, "image/svg+xml");
    // A person can ask for a different generated pattern.
    let (s, _) = ada.send(Method::DELETE, "/api/v1/auth/me/avatar?shuffle=true", None).await;
    assert_eq!(s, StatusCode::OK);
    assert_ne!(avatar(&ben, ada.user_id).await.3, a1);
    // Nobody can set someone else's picture, and pictures need a sign-in.
    assert_eq!(app.anon().get(&format!("/api/v1/users/{}/avatar", ada.user_id)).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(ben.get(&format!("/api/v1/users/{}/avatar", uuid::Uuid::new_v4())).await.0, StatusCode::NOT_FOUND);
}

// ───────────────────────── AI agents ─────────────────────────

struct Bot {
    id: String,
    caller: Caller,
}

async fn bot(app: &TestApp, admin: &Caller, name: &str, scopes: &[&str]) -> Bot {
    let created = admin.created("/api/v1/admin/users", json!({"name": name, "kind": "agent", "description": "Test agent"})).await;
    let id = created["id"].as_str().unwrap().to_string();
    let tok = admin.created(&format!("/api/v1/admin/users/{id}/tokens"), json!({"name": "t", "scopes": scopes})).await;
    Bot { id, caller: Caller { token: Some(tok["token"].as_str().unwrap().to_string()), ..app.anon() } }
}

#[tokio::test]
async fn a_new_agent_can_do_nothing_until_an_administrator_writes_its_rules() {
    let app = spawn().await;
    let admin = app.admin().await;
    admin.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic", "access": "internal"})).await;
    admin.created("/api/v1/projects", json!({"key": "SEC", "name": "Secret", "template": "basic"})).await;
    admin.created("/api/v1/projects/OPS/items", json!({"title": "Open work"})).await;
    admin.created("/api/v1/projects/SEC/items", json!({"title": "Sealed"})).await;
    admin.created("/api/v1/spaces", json!({"key": "DOC", "name": "Docs", "access": "internal"})).await;
    let b = bot(&app, &admin, "Triage bot", &["read", "write"]).await;
    // It is a member of both projects with full project rights, and holds the member role…
    for key in ["OPS", "SEC"] {
        admin.created(&format!("/api/v1/projects/{key}/members"), json!({"user_id": b.id, "role": "project_admin"})).await;
    }
    // …and still sees and does nothing.
    let me = b.caller.ok("/api/v1/auth/me").await;
    assert_eq!(me["user"]["kind"], "agent");
    assert_eq!(me["permissions"], json!([]));
    assert_eq!(me["agent_policy"]["mode"], "restricted");
    assert_eq!(b.caller.ok("/api/v1/projects").await, json!([]));
    assert_eq!(b.caller.ok("/api/v1/items").await["total"], 0);
    assert_eq!(b.caller.ok("/api/v1/spaces").await.as_array().unwrap().len(), 0);
    for (method, path, body) in [
        (Method::GET, "/api/v1/items/OPS-1", None),
        (Method::GET, "/api/v1/projects/OPS", None),
        (Method::POST, "/api/v1/projects/OPS/items", Some(json!({"title": "x"}))),
        (Method::POST, "/api/v1/items/OPS-1/comments", Some(json!({"body": "x"}))),
        (Method::PATCH, "/api/v1/items/OPS-1", Some(json!({"title": "x"}))),
        (Method::GET, "/api/v1/spaces/DOC", None),
    ] {
        assert_eq!(b.caller.send(method.clone(), path, body).await.0, StatusCode::NOT_FOUND, "{method} {path}");
    }
    for path in ["/api/v1/users", "/api/v1/teams", "/api/v1/containers", "/api/v1/search?q=open"] {
        let (s, v) = b.caller.get(path).await;
        assert!(s == StatusCode::FORBIDDEN || v == json!([]) || v["results"] == json!([]), "{path}: {s} {v}");
    }
    assert_eq!(b.caller.post("/api/v1/projects", json!({"key": "BOT", "name": "Bot", "template": "basic"})).await.0, StatusCode::FORBIDDEN);

    // Rules: read everything in OPS, comment there, and nothing else.
    let ops: String = sqlx::query_scalar("SELECT id::text FROM projects WHERE key = 'OPS'").fetch_one(&app.st.db).await.unwrap();
    admin.agent_rules(&b.id, json!({"mode": "restricted", "permissions": ["project.view", "item.comment"], "projects": [ops]})).await;
    assert_eq!(b.caller.ok("/api/v1/projects").await.as_array().unwrap().len(), 1);
    assert_eq!(b.caller.ok("/api/v1/items/OPS-1").await["title"], "Open work");
    assert_eq!(b.caller.post("/api/v1/items/OPS-1/comments", json!({"body": "Looked at this."})).await.0, StatusCode::CREATED);
    // Its role would allow these; its rules do not.
    assert_eq!(b.caller.patch("/api/v1/items/OPS-1", json!({"title": "Renamed by the bot"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(b.caller.post("/api/v1/projects/OPS/items", json!({"title": "x"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(b.caller.send(Method::DELETE, "/api/v1/items/OPS-1", None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(b.caller.patch("/api/v1/projects/OPS", json!({"name": "Mine now"})).await.0, StatusCode::FORBIDDEN);
    // The other project stays invisible although the agent is its administrator by role.
    assert_eq!(b.caller.get("/api/v1/items/SEC-1").await.0, StatusCode::NOT_FOUND);
    assert_eq!(b.caller.ok("/api/v1/items").await["total"], 1);
    assert_eq!(b.caller.post("/api/v1/items/query", json!({"criteria": [{"field": "title", "op": "contains", "value": "Sealed"}]})).await.1["total"], 0);
    // Comments show plainly that an agent wrote them.
    let item = admin.ok("/api/v1/items/OPS-1").await;
    assert_eq!(item["comments"][0]["author"]["kind"], "agent");
    assert_eq!(item["comments"][0]["author"]["name"], "Triage bot");

    // The effective rights are visible to the administrator, and every change is audited with before and after.
    let view = admin.ok(&format!("/api/v1/admin/users/{}/agent-policy", b.id)).await;
    assert_eq!(view["policy"]["permissions"], json!(["item.comment", "project.view"]));
    assert_eq!(view["effective_global"], json!([]), "the member role's rights are not let through");
    assert!(view["role_global"].as_array().unwrap().contains(&json!("workspace.access")));
    let logged: Value = sqlx::query_scalar("SELECT detail FROM audit_log WHERE action = 'agent.policy_update' ORDER BY id DESC LIMIT 1").fetch_one(&app.st.db).await.unwrap();
    assert_eq!(logged["before"]["permissions"], json!([]));
    assert_eq!(logged["after"]["permissions"], json!(["item.comment", "project.view"]));
    // Denials leave a trace.
    assert!(app.security_kinds().await.contains(&"authz.denied".to_string()));
    // In the audit log the agent is found by its name, and each of its entries says what it is.
    let by_name = admin.ok("/api/v1/admin/audit?actor=triage%20bot").await;
    let entries = by_name["entries"].as_array().unwrap();
    assert!(entries.iter().any(|e| e["action"] == "comment.create"), "{by_name}");
    assert!(entries.iter().all(|e| e["actor_kind"] == "agent" && e["actor_name"] == "Triage bot"), "{by_name}");
    assert_eq!(admin.ok("/api/v1/admin/audit?actor=nobody-by-this-name").await["total"], 0);
    let people = admin.ok("/api/v1/admin/audit?actor=root@example.org").await;
    let people = people["entries"].as_array().unwrap();
    assert!(!people.is_empty() && people.iter().all(|e| e["actor_kind"] != "agent"));
    // Entries made while signed in carry the account's name; a sign-in attempt before that has only the address.
    assert!(people.iter().filter(|e| !e["actor_id"].is_null()).all(|e| e["actor_name"].is_string()));
}

#[tokio::test]
async fn agent_rules_have_their_own_guards() {
    let app = spawn().await;
    let admin = app.admin().await;
    let app_admin = app.user("appadmin@example.org", "app_admin").await;
    let member = app.user("ada@example.org", "member").await;
    let b = bot(&app, &admin, "Bot", &["read", "write"]).await;
    let path = format!("/api/v1/admin/users/{}/agent-policy", b.id);
    // Application administrators run the agents: they can create them, write rules and issue tokens.
    let created = app_admin.created("/api/v1/admin/users", json!({"name": "App admin's bot", "kind": "agent"})).await;
    let own = created["id"].as_str().unwrap();
    assert_eq!(app_admin.put(&format!("/api/v1/admin/users/{own}/agent-policy"), json!({"permissions": ["project.view", "workspace.access"], "all_projects": true})).await.0, StatusCode::OK);
    assert_eq!(app_admin.post(&format!("/api/v1/admin/users/{own}/tokens"), json!({"name": "t", "scopes": ["read"]})).await.0, StatusCode::CREATED);
    let listed = app_admin.ok("/api/v1/admin/users?kind=agent").await;
    let row = listed["users"].as_array().unwrap().iter().find(|u| u["id"] == own).unwrap();
    assert_eq!(row["agent_mode"], "restricted");
    assert_eq!(row["owner"]["name"], "appadmin", "whoever creates an agent answers for it");
    // …but cannot let an agent do what they themselves may not, lift all rules, or issue admin tokens.
    let (s, v) = app_admin.put(&path, json!({"permissions": ["admin.settings"]})).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(app_admin.put(&path, json!({"mode": "unrestricted"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app_admin.post(&format!("/api/v1/admin/users/{own}/tokens"), json!({"name": "t", "scopes": ["admin"]})).await.0, StatusCode::FORBIDDEN);
    // They do not get at people's tokens through this route.
    assert_eq!(app_admin.get(&format!("/api/v1/admin/users/{}/tokens", member.user_id)).await.0, StatusCode::FORBIDDEN);
    // Ordinary members have no say.
    assert_eq!(member.get(&path).await.0, StatusCode::FORBIDDEN);
    assert_eq!(member.put(&path, json!({"mode": "unrestricted"})).await.0, StatusCode::FORBIDDEN);
    // An agent cannot rewrite its own rules, even with an admin-scoped token and an admin role.
    let admin_bot = bot(&app, &admin, "Admin bot", &["admin"]).await;
    assert_eq!(admin.post(&format!("/api/v1/admin/users/{}/roles", admin_bot.id), json!({"role": "system_admin"})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(admin_bot.caller.put(&format!("/api/v1/admin/users/{}/agent-policy", admin_bot.id), json!({"mode": "unrestricted"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(admin_bot.caller.get("/api/v1/admin/users").await.0, StatusCode::FORBIDDEN);
    // Malformed rules are refused; rules are for agents only.
    for bad in [json!({"mode": "godmode"}), json!({"permissions": ["*"]}), json!({"permissions": ["made.up"]}), json!({"projects": ["not-an-id"]}), json!({"projects": [uuid::Uuid::new_v4()]}), json!({"surprise": 1}), json!({"rate_limit_per_minute": 0}), json!([])] {
        assert_eq!(admin.put(&path, bad.clone()).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(admin.put(&format!("/api/v1/admin/users/{}/agent-policy", member.user_id), json!({})).await.0, StatusCode::BAD_REQUEST);
    // A rate limit of its own keeps a runaway agent from crowding out people.
    admin.agent_rules(&b.id, json!({"permissions": ["workspace.access"], "rate_limit_per_minute": 3})).await;
    let mut codes = vec![];
    for _ in 0..6 {
        codes.push(b.caller.get("/api/v1/auth/me").await.0.as_u16());
    }
    assert!(codes.contains(&429), "{codes:?}");
    assert_eq!(member.get("/api/v1/auth/me").await.0, StatusCode::OK, "people are unaffected");
}

#[tokio::test]
async fn mcp_offers_an_agent_only_the_tools_its_rules_allow() {
    let app = spawn().await;
    let admin = app.admin().await;
    admin.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    admin.created("/api/v1/projects/OPS/items", json!({"title": "One"})).await;
    let b = bot(&app, &admin, "Reader", &["read", "write"]).await;
    admin.created("/api/v1/projects/OPS/members", json!({"user_id": b.id, "role": "project_member"})).await;
    let call = |method: &'static str, params: Value| {
        let c = b.caller.clone();
        async move { c.request(Method::POST, "/mcp").json(&json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params})).send().await.unwrap().json::<Value>().await.unwrap() }
    };
    let names = |v: &Value| v["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert!(names(&call("tools/list", json!({})).await).is_empty(), "no rules, no tools");
    let r = call("tools/call", json!({"name": "list_projects", "arguments": {}})).await;
    assert_eq!(r["result"]["isError"], true);
    assert!(r["result"]["content"][0]["text"].as_str().unwrap().contains("not allowed"));

    admin.agent_rules(&b.id, json!({"permissions": ["project.view", "item.comment"], "all_projects": true})).await;
    let offered = names(&call("tools/list", json!({})).await);
    assert_eq!(offered, vec!["list_projects", "get_project", "list_work_items", "get_work_item", "add_comment", "get_item_graph"]);
    let r = call("tools/call", json!({"name": "get_work_item", "arguments": {"key": "OPS-1"}})).await;
    assert_eq!(r["result"]["isError"], false, "{r}");
    let r = call("tools/call", json!({"name": "update_work_item", "arguments": {"key": "OPS-1", "changes": {"title": "x"}}})).await;
    assert_eq!(r["result"]["isError"], true);
    assert_eq!(admin.ok("/api/v1/items/OPS-1").await["title"], "One");
}

// ───────────────────────── teams ─────────────────────────

#[tokio::test]
async fn teams_are_positions_filled_over_time() {
    let app = spawn().await;
    let admin = app.admin().await;
    let ada = app.user("ada@example.org", "member").await;
    let ben = app.user("ben@example.org", "member").await;
    let chen = app.user("chen@example.org", "member").await;
    let robot = admin.created("/api/v1/admin/users", json!({"name": "Deploy bot", "kind": "agent"})).await;

    // Anyone may start a team and leads it.
    let team = ada.created("/api/v1/teams", json!({"name": "Avionics", "description": "Flight software"})).await;
    let tid = team["id"].as_str().unwrap().to_string();
    assert_eq!(team["can_edit"], true);
    assert_eq!(team["positions"][0]["title"], "Team lead");
    assert_eq!(team["positions"][0]["holders"][0]["user"]["name"], "ada");
    assert_eq!(ada.post("/api/v1/teams", json!({"name": "avionics"})).await.0, StatusCode::CONFLICT, "names are unique regardless of case");

    // Positions define the team; several accounts, including an AI agent, can fill one.
    let t = ada.created(&format!("/api/v1/teams/{tid}/positions"), json!({"title": "Flight software engineer"})).await;
    let eng = t["positions"].as_array().unwrap().iter().find(|p| p["title"] == "Flight software engineer").unwrap()["id"].as_str().unwrap().to_string();
    // Ben has been in the role since before the team was recorded here.
    let t = ada.created(&format!("/api/v1/teams/{tid}/positions/{eng}/assignments"), json!({"user_id": ben.user_id, "started_on": "2021-03-01"})).await;
    assert_eq!(t["positions"].as_array().unwrap().iter().find(|p| p["id"] == eng.as_str()).unwrap()["holders"][0]["started_on"], "2021-03-01");
    ada.created(&format!("/api/v1/teams/{tid}/positions/{eng}/assignments"), json!({"user_id": robot["id"]})).await;
    assert_eq!(ada.post(&format!("/api/v1/teams/{tid}/positions/{eng}/assignments"), json!({"user_id": ben.user_id})).await.0, StatusCode::CONFLICT, "already in the position");
    let t = ben.ok(&format!("/api/v1/teams/{tid}")).await;
    let holders = t["positions"].as_array().unwrap().iter().find(|p| p["id"] == eng.as_str()).unwrap()["holders"].as_array().unwrap().clone();
    assert_eq!(holders.len(), 2);
    assert!(holders.iter().any(|h| h["user"]["kind"] == "agent"), "an agent in a position is marked as one");
    assert_eq!(t["can_edit"], false, "members who are not leads read but do not edit");

    // Only leads (and administrators) edit.
    assert_eq!(ben.patch(&format!("/api/v1/teams/{tid}"), json!({"name": "Ben's team"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(ben.post(&format!("/api/v1/teams/{tid}/positions"), json!({"title": "Boss", "is_lead": true})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(chen.post(&format!("/api/v1/teams/{tid}/positions/{eng}/assignments"), json!({"user_id": chen.user_id})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(admin.patch(&format!("/api/v1/teams/{tid}"), json!({"description": "Flight software and its tools"})).await.0, StatusCode::OK);

    // People were told.
    let notes = ben.ok("/api/v1/notifications").await;
    assert!(notes.as_array().unwrap().iter().any(|n| n["category"] == "team" && n["title"].as_str().unwrap().contains("added you as Flight software engineer")), "{notes}");

    // Ben moves on: the assignment ends, the history stays on his profile with its length.
    let aid = holders.iter().find(|h| h["user"]["name"] == "ben").unwrap()["assignment_id"].as_str().unwrap().to_string();
    assert_eq!(ada.patch(&format!("/api/v1/teams/{tid}/assignments/{aid}"), json!({"ended_on": "2020-01-01"})).await.0, StatusCode::BAD_REQUEST, "cannot end before it started");
    assert_eq!(ada.patch(&format!("/api/v1/teams/{tid}/assignments/{aid}"), json!({"ended_on": "2024-03-01"})).await.0, StatusCode::OK);
    let profile = chen.ok(&format!("/api/v1/users/{}", ben.user_id)).await;
    let past = &profile["positions"][0];
    assert_eq!((past["team"].as_str(), past["title"].as_str(), past["started_on"].as_str(), past["ended_on"].as_str()), (Some("Avionics"), Some("Flight software engineer"), Some("2021-03-01"), Some("2024-03-01")));
    assert_eq!(past["days"], 1096, "three years, one of them a leap year");
    // A position that has history is closed, not erased.
    let t = ada.send(Method::DELETE, &format!("/api/v1/teams/{tid}/positions/{eng}"), None).await.1;
    let closed = t["positions"].as_array().unwrap().iter().find(|p| p["id"] == eng.as_str()).unwrap().clone();
    assert_eq!(closed["closed"], true);
    assert_eq!(closed["past"].as_array().unwrap().len(), 2, "the agent's time ended with the position");
    assert_eq!(chen.ok(&format!("/api/v1/users/{}", ben.user_id)).await["positions"].as_array().unwrap().len(), 1);

    // A team always has a lead.
    let lead_pos = team["positions"][0]["id"].as_str().unwrap();
    let lead_assignment = team["positions"][0]["holders"][0]["assignment_id"].as_str().unwrap();
    let (s, v) = ada.patch(&format!("/api/v1/teams/{tid}/assignments/{lead_assignment}"), json!({"ended_on": chrono::Utc::now().date_naive().to_string()})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(detail(&v).contains("at least one lead"));
    assert_eq!(ada.patch(&format!("/api/v1/teams/{tid}/positions/{lead_pos}"), json!({"is_lead": false})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ada.send(Method::DELETE, &format!("/api/v1/teams/{tid}/assignments/{lead_assignment}"), None).await.0, StatusCode::BAD_REQUEST);
    // Handing over works: name a second lead, then step down.
    ada.created(&format!("/api/v1/teams/{tid}/positions/{lead_pos}/assignments"), json!({"user_id": chen.user_id})).await;
    assert_eq!(ada.patch(&format!("/api/v1/teams/{tid}/assignments/{lead_assignment}"), json!({"ended_on": chrono::Utc::now().date_naive().to_string()})).await.0, StatusCode::OK);
    assert_eq!(ada.patch(&format!("/api/v1/teams/{tid}"), json!({"name": "Mine again"})).await.0, StatusCode::FORBIDDEN, "a former lead no longer edits");
    assert_eq!(chen.patch(&format!("/api/v1/teams/{tid}"), json!({"name": "Avionics and tools"})).await.0, StatusCode::OK);
    for action in ["team.create", "team.position_create", "team.assign", "team.assignment_update", "team.position_remove", "team.update"] {
        assert!(app.audit_actions().await.contains(&action.to_string()), "{action}");
    }
}

#[tokio::test]
async fn teams_of_teams_and_the_org_chart() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    let ben = app.user("ben@example.org", "member").await;
    let customer = app.user("cust@example.org", "customer").await;
    let id = |v: &Value| v["id"].as_str().unwrap().to_string();
    let division = id(&ada.created("/api/v1/teams", json!({"name": "Engineering"})).await);
    let group = id(&ada.created("/api/v1/teams", json!({"name": "Propulsion", "parent_id": division})).await);
    // Ben runs his own team and may not hang it under Ada's without her say.
    let bens = id(&ben.created("/api/v1/teams", json!({"name": "Test stand"})).await);
    assert_eq!(ben.patch(&format!("/api/v1/teams/{bens}"), json!({"parent_id": group})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(ben.post("/api/v1/teams", json!({"name": "Sneaky", "parent_id": division})).await.0, StatusCode::FORBIDDEN);
    // A lead of a team above edits the teams below.
    let unit = id(&ada.created("/api/v1/teams", json!({"name": "Turbopumps", "parent_id": group})).await);
    assert_eq!(ada.patch(&format!("/api/v1/teams/{unit}"), json!({"description": "Pumps"})).await.0, StatusCode::OK);
    // No loops.
    let (s, v) = ada.patch(&format!("/api/v1/teams/{division}"), json!({"parent_id": unit})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(detail(&v).contains("under itself"));
    assert_eq!(ada.patch(&format!("/api/v1/teams/{unit}"), json!({"parent_id": unit})).await.0, StatusCode::BAD_REQUEST);

    let list = ben.ok("/api/v1/teams").await;
    let teams = list["teams"].as_array().unwrap();
    assert_eq!(teams.len(), 4);
    let mine: Vec<&str> = teams.iter().filter(|t| t["mine"] == true).map(|t| t["name"].as_str().unwrap()).collect();
    assert_eq!(mine, vec!["Test stand"], "\"my teams\" are the ones I hold a position in");
    assert_eq!(teams.iter().find(|t| t["name"] == "Propulsion").unwrap()["leads"][0]["name"], "ada");
    let chart = ben.ok("/api/v1/org-chart").await;
    let node = chart.as_array().unwrap().iter().find(|t| t["name"] == "Turbopumps").unwrap();
    assert_eq!(node["parent_id"], group.as_str());
    assert_eq!(node["positions"][0]["holders"][0]["name"], "ada");
    // Archiving ends everyone's time there and lifts sub-teams up a level.
    assert_eq!(ada.patch(&format!("/api/v1/teams/{group}"), json!({"archived": true})).await.0, StatusCode::OK);
    let list = ada.ok("/api/v1/teams").await;
    assert!(!list["teams"].as_array().unwrap().iter().any(|t| t["name"] == "Propulsion"));
    assert_eq!(list["teams"].as_array().unwrap().iter().find(|t| t["name"] == "Turbopumps").unwrap()["parent_id"], division.as_str());
    let history = ada.ok(&format!("/api/v1/users/{}", ada.user_id)).await;
    assert!(history["positions"].as_array().unwrap().iter().any(|p| p["team"] == "Propulsion" && p["ended_on"].is_string()));
    // The directory of teams is for the workspace, not for portal customers.
    assert_eq!(customer.get("/api/v1/teams").await.0, StatusCode::FORBIDDEN);
    assert_eq!(customer.get("/api/v1/org-chart").await.0, StatusCode::FORBIDDEN);
    assert_eq!(customer.post("/api/v1/teams", json!({"name": "Customers united"})).await.0, StatusCode::FORBIDDEN);
}

// ───────────────────────── notifications ─────────────────────────

async fn kinds(c: &Caller) -> Vec<(String, String)> {
    c.ok("/api/v1/notifications").await.as_array().unwrap().iter().map(|n| (n["category"].as_str().unwrap().to_string(), n["title"].as_str().unwrap().to_string())).collect()
}

#[tokio::test]
async fn people_hear_about_what_concerns_them_and_can_see() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    let ben = app.user("ben@example.org", "member").await;
    let chen = app.user("chen@example.org", "member").await;
    let outsider = app.user("eve@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    for u in [&ben, &chen] {
        ada.created("/api/v1/projects/OPS/members", json!({"user_id": u.user_id, "role": "project_member"})).await;
    }
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Replace pump seal", "assignee_id": ben.user_id})).await;
    assert!(kinds(&ben).await.iter().any(|(c, t)| c == "assigned" && t.contains("created OPS-1")), "the assignee hears about new work");

    // A mention reaches the person once, as a mention, even though they also follow the item afterward.
    ada.created("/api/v1/items/OPS-1/comments", json!({"body": "@chen can you check the torque values? cc @eve @nobody-by-this-name"})).await;
    let chens = kinds(&chen).await;
    assert_eq!(chens.iter().filter(|(c, _)| c == "mention").count(), 1, "{chens:?}");
    assert!(!chens.iter().any(|(c, _)| c == "comment"), "not told twice about the same message: {chens:?}");
    assert!(kinds(&ben).await.iter().any(|(c, t)| c == "comment" && t.contains("commented on OPS-1")));
    // Eve was mentioned but cannot open the item: she learns nothing, not even that it exists.
    assert!(kinds(&outsider).await.is_empty());
    assert_eq!(outsider.ok("/api/v1/auth/me").await["unread_notifications"], 0);
    // Nobody is told about their own action.
    assert!(kinds(&ada).await.is_empty());

    // Chen, having been mentioned, now follows the item and hears about the next message and status change.
    ben.created("/api/v1/items/OPS-1/comments", json!({"body": "Values are in spec."})).await;
    assert!(kinds(&chen).await.iter().any(|(c, _)| c == "comment"));
    let to = ben.ok("/api/v1/items/OPS-1").await["transitions"][0]["to"].as_str().unwrap().to_string();
    ben.post("/api/v1/items/OPS-1/transition", json!({"to": to})).await;
    assert!(kinds(&chen).await.iter().any(|(c, _)| c == "status"));
    assert!(kinds(&ada).await.iter().any(|(c, _)| c == "status"), "the reporter follows too");
    // Unfollowing stops it.
    assert_eq!(chen.ok("/api/v1/items/OPS-1").await["watching"], true);
    assert_eq!(chen.send(Method::DELETE, "/api/v1/items/OPS-1/watch", None).await.1["watching"], false);
    let before = kinds(&chen).await.len();
    ben.created("/api/v1/items/OPS-1/comments", json!({"body": "Closing out."})).await;
    assert_eq!(kinds(&chen).await.len(), before);

    // Someone removed from the project stops hearing about it, though still listed as a follower.
    let binding = ada.ok("/api/v1/projects/OPS").await["members"].as_array().unwrap().iter().find(|m| m["user"]["name"] == "ben").unwrap()["binding_id"].as_str().unwrap().to_string();
    assert_eq!(ada.send(Method::DELETE, &format!("/api/v1/projects/OPS/members/{binding}"), None).await.0, StatusCode::NO_CONTENT);
    let before = kinds(&ben).await.len();
    ada.created("/api/v1/items/OPS-1/comments", json!({"body": "A note after Ben left: @ben should not see this."})).await;
    assert_eq!(kinds(&ben).await.len(), before);

    // Reading: one at a time or all at once.
    let first = chen.ok("/api/v1/notifications").await[0]["id"].as_i64().unwrap();
    let unread = chen.ok("/api/v1/auth/me").await["unread_notifications"].as_i64().unwrap();
    assert_eq!(chen.post(&format!("/api/v1/notifications/{first}/read"), json!({})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(chen.ok("/api/v1/auth/me").await["unread_notifications"], unread - 1);
    assert_eq!(ben.post(&format!("/api/v1/notifications/{first}/read"), json!({})).await.0, StatusCode::NOT_FOUND, "someone else's notification");
    assert_eq!(chen.post("/api/v1/notifications/read", json!({})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(chen.ok("/api/v1/auth/me").await["unread_notifications"], 0);
}

#[tokio::test]
async fn each_person_chooses_channels_within_what_the_workspace_allows() {
    let (port, mail) = mail_sink().await;
    let app = spawn_with(|c| use_mail(c, port)).await;
    let admin = app.admin().await;
    let ada = app.user("ada@example.org", "member").await;
    let ben = app.user("ben@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    ada.created("/api/v1/projects/OPS/members", json!({"user_id": ben.user_id, "role": "project_member"})).await;
    let sent = || async { mail.lock().await.clone() };

    let prefs = ben.ok("/api/v1/notifications/preferences").await;
    assert_eq!(prefs["email_available"], true);
    assert_eq!(prefs["email_address"], "ben@example.org");
    let cat = |p: &Value, key: &str| p["categories"].as_array().unwrap().iter().find(|c| c["key"] == key).cloned().unwrap_or(Value::Null);
    assert_eq!((cat(&prefs, "assigned")["in_app"].as_bool(), cat(&prefs, "assigned")["email"].as_bool()), (Some(true), Some(true)));
    assert_eq!(cat(&prefs, "status")["email"], false);
    assert!(cat(&prefs, "admin_system").is_null(), "administrative categories are offered only to administrators");
    assert!(!cat(&admin.ok("/api/v1/notifications/preferences").await, "admin_system").is_null());

    // Default: an assignment arrives in the application and by email.
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Calibrate <b>sensor</b>", "description": "Use the new jig.", "assignee_id": ben.user_id})).await;
    assert_eq!(app.drain_jobs().await, 1);
    let m = sent().await;
    assert_eq!(m.len(), 1);
    assert!(m[0].contains("RCPT TO:<ben@example.org>") && m[0].contains("MAIL FROM:<gantry@example.org>"));
    assert!(m[0].contains("Subject: [Gantry] OPS-1 Calibrate <b>sensor</b>"), "{}", m[0]);
    assert!(m[0].contains("Auto-Submitted: auto-generated"));
    assert!(m[0].contains("http://gantry.test/i/OPS-1"));
    assert!(m[0].contains("&lt;b&gt;sensor&lt;/b&gt;") && !m[0].contains("<b>sensor</b></p>"), "the HTML part escapes what people wrote");
    assert!(m[0].contains("http://gantry.test/profile#notifications"), "every message says how to stop it");

    // Ben turns email off for assignments and on for status changes.
    assert_eq!(ben.put("/api/v1/notifications/preferences", json!({"assigned": {"in_app": true, "email": false}, "status": {"in_app": false, "email": true}})).await.0, StatusCode::NO_CONTENT);
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Second", "assignee_id": ben.user_id})).await;
    app.drain_jobs().await;
    assert_eq!(sent().await.len(), 1, "no email for the second assignment: {:?}", sent().await.last());
    assert_eq!(kinds(&ben).await.iter().filter(|(c, _)| c == "assigned").count(), 2, "still shown in the application");
    let to = ada.ok("/api/v1/items/OPS-1").await["transitions"][0]["to"].as_str().unwrap().to_string();
    ada.post("/api/v1/items/OPS-1/transition", json!({"to": to})).await;
    app.drain_jobs().await;
    assert_eq!(sent().await.len(), 2);
    assert!(!kinds(&ben).await.iter().any(|(c, _)| c == "status"), "and not in the application, as he chose");

    // The administrator locks mentions on: nobody can opt out.
    assert_eq!(admin.put("/api/v1/admin/settings/notifications", json!({"categories": {"mention": {"in_app": true, "email": true, "locked": true}}})).await.0, StatusCode::NO_CONTENT);
    let (s, v) = ben.put("/api/v1/notifications/preferences", json!({"mention": {"in_app": false, "email": false}})).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    assert_eq!(cat(&ben.ok("/api/v1/notifications/preferences").await, "mention")["locked"], true);
    ada.created("/api/v1/items/OPS-1/comments", json!({"body": "@ben please confirm"})).await;
    app.drain_jobs().await;
    assert_eq!(sent().await.len(), 3);
    assert!(sent().await[2].contains("> @ben please confirm"), "the message is quoted in the plain-text part");
    // The administrator turns all email off for the workspace.
    assert_eq!(admin.put("/api/v1/admin/settings/notifications", json!({"email_enabled": false})).await.0, StatusCode::NO_CONTENT);
    ada.created("/api/v1/items/OPS-1/comments", json!({"body": "@ben one more"})).await;
    app.drain_jobs().await;
    assert_eq!(sent().await.len(), 3);
    assert_eq!(ben.ok("/api/v1/notifications/preferences").await["email_available"], false);

    for bad in [json!({"nonsense": {"in_app": true, "email": true}}), json!({"assigned": {"in_app": "yes", "email": true}}), json!({"assigned": {"email": true}}), json!({"admin_system": {"in_app": true, "email": true}})] {
        assert_eq!(ben.put("/api/v1/notifications/preferences", bad.clone()).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    // What was sent is on record: recipient, category, subject, outcome. Not the content.
    let log = admin.ok("/api/v1/admin/email").await;
    assert_eq!(log["configured"], true);
    assert_eq!(log["last_24h"]["sent"], 3);
    assert!(!log.to_string().contains("please confirm"));
    assert_eq!(ben.get("/api/v1/admin/email").await.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn email_is_never_sent_to_agents_or_inactive_accounts_and_failures_are_logged() {
    let (port, mail) = mail_sink().await;
    let app = spawn_with(|c| use_mail(c, port)).await;
    let admin = app.admin().await;
    admin.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    // An agent with a real-looking address: still no mail.
    let robot = admin.created("/api/v1/admin/users", json!({"name": "Robot", "kind": "agent", "email": "robot@example.org"})).await;
    admin.created("/api/v1/projects/OPS/members", json!({"user_id": robot["id"], "role": "project_member"})).await;
    admin.agent_rules(robot["id"].as_str().unwrap(), working_agent()).await;
    admin.created("/api/v1/projects/OPS/items", json!({"title": "For the robot", "assignee_id": robot["id"]})).await;
    let queued: Vec<String> = sqlx::query_scalar("SELECT kind || ' ' || payload::text FROM jobs").fetch_all(&app.st.db).await.unwrap();
    assert_eq!(app.drain_jobs().await, 0, "nothing was even queued: {queued:?}");
    let in_app: i64 = sqlx::query_scalar("SELECT count(*) FROM notifications WHERE user_id = $1::uuid").bind(robot["id"].as_str().unwrap()).fetch_one(&app.st.db).await.unwrap();
    assert_eq!(in_app, 1, "the agent still has the notification to read through the API");

    // A person deactivated between queueing and sending gets nothing.
    let ben = app.user("ben@example.org", "member").await;
    admin.created("/api/v1/projects/OPS/members", json!({"user_id": ben.user_id, "role": "project_member"})).await;
    admin.created("/api/v1/projects/OPS/items", json!({"title": "For Ben", "assignee_id": ben.user_id})).await;
    assert_eq!(admin.patch(&format!("/api/v1/admin/users/{}", ben.user_id), json!({"active": false})).await.0, StatusCode::NO_CONTENT);
    app.drain_jobs().await;
    assert!(mail.lock().await.is_empty());
    let skipped: i64 = sqlx::query_scalar("SELECT count(*) FROM email_log WHERE outcome = 'skipped'").fetch_one(&app.st.db).await.unwrap();
    assert_eq!(skipped, 1);

    // The test message tells the administrator what the mail server said.
    let r = admin.post("/api/v1/admin/email/test", json!({})).await.1;
    assert_eq!(r, json!({"ok": true, "to": "root@example.org"}));
    assert!(mail.lock().await[0].contains("Subject: [Gantry] Email from this server works."));
    for bad in ["not-an-address", "a@b.c\r\nBcc: x@y.zz", "<script>@x.yy"] {
        assert_eq!(admin.post("/api/v1/admin/email/test", json!({"to": bad})).await.0, StatusCode::BAD_REQUEST, "{bad:?}");
    }
    assert_eq!(mail.lock().await.len(), 1);

    // With the mail server gone, sending fails, is retried by the queue and shows in the log.
    let dead = spawn_with(|c| use_mail(c, 1)).await;
    let root = dead.admin().await;
    let r = root.post("/api/v1/admin/email/test", json!({})).await.1;
    assert_eq!(r["ok"], false);
    assert!(r["error"].as_str().unwrap().contains("mail server"));
    assert_eq!(root.ok("/api/v1/admin/email").await["last_24h"]["failed"], 1);
    // Without a mail server configured the application says so instead of pretending.
    let plain = spawn().await;
    let root = plain.admin().await;
    assert_eq!(root.ok("/api/v1/auth/me").await["features"]["email"], false);
    assert_eq!(root.post("/api/v1/admin/email/test", json!({})).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reminders_arrive_once_at_the_configured_hour() {
    let app = spawn().await;
    let admin = app.admin().await;
    let ben = app.user("ben@example.org", "member").await;
    admin.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    admin.created("/api/v1/projects/OPS/members", json!({"user_id": ben.user_id, "role": "project_member"})).await;
    let today = chrono::Utc::now().date_naive();
    let day = |offset: i64| (today + chrono::Duration::days(offset)).to_string();
    for (title, due) in [("Due today", day(0)), ("Due in two days", day(2)), ("Due tomorrow", day(1)), ("Overdue since yesterday", day(-1)), ("Far away", day(20))] {
        admin.created("/api/v1/projects/OPS/items", json!({"title": title, "assignee_id": ben.user_id, "due_date": due})).await;
    }
    let done = admin.created("/api/v1/projects/OPS/items", json!({"title": "Finished, though due today", "assignee_id": ben.user_id, "due_date": day(0)})).await;
    let detail = admin.ok(&format!("/api/v1/items/{}", done["key"].as_str().unwrap())).await;
    let to_done = detail["transitions"].as_array().unwrap().iter().find(|t| t["category"] == "done").unwrap()["to"].as_str().unwrap().to_string();
    admin.post(&format!("/api/v1/items/{}/transition", done["key"].as_str().unwrap()), json!({"to": to_done})).await;
    sqlx::query("DELETE FROM notifications").execute(&app.st.db).await.unwrap();

    // Not the reminder hour: nothing.
    let hour = chrono::Timelike::hour(&chrono::Utc::now());
    assert_eq!(admin.put("/api/v1/admin/settings/notifications", json!({"reminders": {"hour": (hour + 5) % 24}})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(gantry::notify::send_reminders(&app.st).await.unwrap(), 0);
    // The reminder hour (workspace time zone is UTC; Ben's is "auto", which follows it).
    assert_eq!(admin.put("/api/v1/admin/settings/notifications", json!({"reminders": {"hour": hour}})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(gantry::notify::send_reminders(&app.st).await.unwrap(), 1, "one digest for one person");
    let notes = ben.ok("/api/v1/notifications").await;
    assert_eq!(notes.as_array().unwrap().len(), 1);
    assert_eq!(notes[0]["category"], "reminder");
    assert_eq!(notes[0]["title"], "Your work: 1 overdue, 1 due today, 1 due soon");
    assert_eq!(notes[0]["body"], "3 work items need attention");
    // Running again in the same hour (a second worker, a restart) sends nothing more.
    assert_eq!(gantry::notify::send_reminders(&app.st).await.unwrap(), 0);
    // A person in another time zone is reminded at their own morning, not the server's.
    sqlx::query("DELETE FROM reminders_sent").execute(&app.st.db).await.unwrap();
    let ahead = if hour < 12 { "Etc/GMT-5" } else { "Etc/GMT+5" };
    assert_eq!(ben.patch("/api/v1/auth/me", json!({"timezone": ahead})).await.0, StatusCode::OK);
    assert_eq!(gantry::notify::send_reminders(&app.st).await.unwrap(), 0, "it is not {hour}:00 where Ben is");
}

// ───────────────────────── alerts ─────────────────────────

#[tokio::test]
async fn alert_rules_fire_once_resolve_and_reach_their_channels() {
    let (port, mail) = mail_sink().await;
    let app = spawn_with(|c| use_mail(c, port)).await;
    let admin = app.admin().await;
    let app_admin = app.user("appadmin@example.org", "app_admin").await;
    let member = app.user("ada@example.org", "member").await;
    let (hook, received) = recorder().await;
    let integ = admin.created("/api/v1/admin/integrations", json!({"kind": "webhook", "name": "Ops channel", "config": {"url": hook}, "events": []})).await;

    let listed = admin.ok("/api/v1/admin/alerts").await;
    assert!(listed["rules"].as_array().unwrap().len() >= 10, "sensible rules exist from the start");
    assert!(listed["kinds"].as_array().unwrap().iter().any(|k| k["kind"] == "http_error_rate" && k["scope"] == "system"));
    // Quiet the built-in rules so this test sees only its own.
    sqlx::query("UPDATE alert_rules SET enabled = false").execute(&app.st.db).await.unwrap();

    // An application administrator defines application alerts, not system ones.
    let (s, v) = app_admin.post("/api/v1/admin/alerts", json!({"name": "Queue", "kind": "job_backlog", "params": {"threshold": 1}})).await;
    assert_eq!(s, StatusCode::FORBIDDEN, "{v}");
    let rule = app_admin.created("/api/v1/admin/alerts", json!({"name": "Too much overdue work", "kind": "overdue_items", "params": {"threshold": 2, "project": "OPS"}, "severity": "warning",
        "channels": {"in_app": true, "email": true, "roles": ["app_admin"], "addresses": ["ops-lead@example.org"], "integrations": [integ["id"]]}, "cooldown_minutes": 60})).await;
    let id = rule["id"].as_str().unwrap().to_string();
    assert_eq!(member.get("/api/v1/admin/alerts").await.0, StatusCode::FORBIDDEN);
    assert_eq!(member.post("/api/v1/admin/alerts", json!({"name": "x", "kind": "overdue_items"})).await.0, StatusCode::FORBIDDEN);

    admin.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    let yesterday = (chrono::Utc::now().date_naive() - chrono::Duration::days(1)).to_string();
    admin.created("/api/v1/projects/OPS/items", json!({"title": "Late one", "due_date": yesterday})).await;
    assert_eq!(gantry::notify::check_alerts(&app.st).await.unwrap(), 0, "one overdue item is under the threshold");
    admin.created("/api/v1/projects/OPS/items", json!({"title": "Late two", "due_date": yesterday})).await;
    assert_eq!(gantry::notify::check_alerts(&app.st).await.unwrap(), 1);
    assert_eq!(gantry::notify::check_alerts(&app.st).await.unwrap(), 0, "still over the threshold: no second announcement");
    app.drain_jobs().await;

    // In the application, for people holding the role the rule names…
    let notes = kinds(&app_admin).await;
    assert_eq!(notes, vec![("admin_app".to_string(), "Alert: Too much overdue work".to_string())]);
    assert!(kinds(&member).await.is_empty());
    // …by email to them and to the extra address…
    let m = mail.lock().await.clone();
    let to: Vec<bool> = ["appadmin@example.org", "ops-lead@example.org"].iter().map(|a| m.iter().any(|x| x.contains(&format!("RCPT TO:<{a}>")))).collect();
    assert_eq!(to, vec![true, true], "{m:?}");
    assert!(m[0].contains("2 open work items are overdue in OPS"));
    // …and to the chat channel.
    let posted = received.lock().await.clone();
    assert_eq!(posted.len(), 1);
    let body: Value = serde_json::from_slice(&posted[0].1).unwrap();
    assert_eq!(body["type"], "alert.firing");
    assert_eq!(body["data"]["severity"], "warning");

    // Back to normal: recorded and announced as resolved.
    admin.patch("/api/v1/items/OPS-1", json!({"due_date": null})).await;
    assert_eq!(gantry::notify::check_alerts(&app.st).await.unwrap(), 1);
    assert!(kinds(&app_admin).await.iter().any(|(_, t)| t == "Resolved: Too much overdue work"));
    let events = admin.ok("/api/v1/admin/alerts").await["events"].as_array().unwrap().iter().map(|e| e["state"].as_str().unwrap().to_string()).collect::<Vec<_>>();
    assert_eq!(events, vec!["resolved", "firing"]);
    // Over the threshold again within the cooldown: recorded, not announced again.
    let before = kinds(&app_admin).await.len();
    admin.patch("/api/v1/items/OPS-1", json!({"due_date": yesterday})).await;
    assert_eq!(gantry::notify::check_alerts(&app.st).await.unwrap(), 1);
    assert_eq!(kinds(&app_admin).await.len(), before);
    assert_eq!(admin.ok("/api/v1/admin/alerts").await["events"].as_array().unwrap().len(), 3);

    // "Send a test" measures now and goes through every channel.
    let t = app_admin.post(&format!("/api/v1/admin/alerts/{id}/test"), json!({})).await.1;
    assert_eq!((t["over_threshold"].as_bool(), t["value"].as_f64()), (Some(true), Some(2.0)));
    // Validation.
    for bad in [
        json!({"name": "x", "kind": "made_up"}),
        json!({"name": "x", "kind": "overdue_items", "params": {"threshold": "many"}}),
        json!({"name": "x", "kind": "overdue_items", "params": {"project": "ops'; DROP TABLE items;--"}}),
        json!({"name": "x", "kind": "overdue_items", "channels": {"addresses": ["a@b.c\r\nBcc: evil@example.org"]}}),
        json!({"name": "x", "kind": "overdue_items", "channels": {"roles": ["no_such_role"]}}),
        json!({"name": "x", "kind": "overdue_items", "channels": {"integrations": [uuid::Uuid::new_v4()]}}),
        json!({"name": "x", "kind": "overdue_items", "severity": "apocalyptic"}),
        json!({"name": "x", "kind": "overdue_items", "cooldown_minutes": 0}),
        json!({"name": "", "kind": "overdue_items"}),
    ] {
        assert_eq!(app_admin.post("/api/v1/admin/alerts", bad.clone()).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert!(app_admin.post("/api/v1/admin/alerts", json!({"name": "x", "kind": "overdue_items", "surprise": true})).await.0.is_client_error(), "unknown fields are refused");
    // The alerts that come with the product can be changed and turned off, never deleted; the administrator's own can be.
    let builtin = admin.ok("/api/v1/admin/alerts").await["rules"].as_array().unwrap().iter().find(|r| r["builtin"] == true && r["scope"] == "application").unwrap()["id"].as_str().unwrap().to_string();
    assert_eq!(app_admin.send(Method::DELETE, &format!("/api/v1/admin/alerts/{builtin}"), None).await.0, StatusCode::CONFLICT);
    assert_eq!(app_admin.patch(&format!("/api/v1/admin/alerts/{builtin}"), json!({"enabled": false})).await.0, StatusCode::NO_CONTENT);
    let own = app_admin.created("/api/v1/admin/alerts", json!({"name": "Throwaway", "kind": "new_accounts"})).await["id"].as_str().unwrap().to_string();
    assert_eq!(app_admin.send(Method::DELETE, &format!("/api/v1/admin/alerts/{own}"), None).await.0, StatusCode::NO_CONTENT);
    assert_eq!(app_admin.patch(&format!("/api/v1/admin/alerts/{id}"), json!({"enabled": false})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(gantry::notify::check_alerts(&app.st).await.unwrap(), 0, "a rule that is off is not measured");
    assert_eq!(app_admin.send(Method::DELETE, &format!("/api/v1/admin/alerts/{id}"), None).await.0, StatusCode::NO_CONTENT);
    for a in ["alert.create", "alert.update", "alert.test", "alert.delete"] {
        assert!(app.audit_actions().await.contains(&a.to_string()), "{a}");
    }
}

#[tokio::test]
async fn system_alerts_are_measured_from_what_the_server_records() {
    let app = spawn().await;
    let admin = app.admin().await;
    sqlx::query("UPDATE alert_rules SET enabled = false").execute(&app.st.db).await.unwrap();
    let measure = |kind: &'static str, params: Value| {
        let st = app.st.clone();
        async move { gantry::notify::evaluate(&st, kind, &params).await.unwrap() }
    };
    // Error rate and latency come from per-minute samples that every node writes.
    for (name, value) in [("requests", 1000.0), ("server_errors", 80.0), ("duration_micros", 2_500_000_000.0), ("rate_limited", 5.0)] {
        sqlx::query("INSERT INTO metric_samples (at, node, name, value) VALUES (now() - interval '2 minutes', 'node-a', $1, $2)").bind(name).bind(value).execute(&app.st.db).await.unwrap();
    }
    // A second node's samples count toward the same totals.
    sqlx::query("INSERT INTO metric_samples (at, node, name, value) VALUES (now() - interval '1 minute', 'node-b', 'requests', 1000)").execute(&app.st.db).await.unwrap();
    let (rate, firing, text) = measure("http_error_rate", json!({"window_minutes": 10, "threshold_percent": 3, "min_requests": 50})).await;
    assert!((rate - 4.0).abs() < 0.01 && firing, "{text}");
    assert!(!measure("http_error_rate", json!({"window_minutes": 10, "threshold_percent": 5})).await.1);
    assert!(!measure("http_error_rate", json!({"window_minutes": 10, "threshold_percent": 1, "min_requests": 5000})).await.1, "too few requests to judge");
    let (ms, firing, _) = measure("http_latency", json!({"window_minutes": 10, "threshold_ms": 1000})).await;
    assert!((ms - 1250.0).abs() < 0.5 && firing);
    // Old samples fall out of the window.
    sqlx::query("UPDATE metric_samples SET at = now() - interval '3 hours'").execute(&app.st.db).await.unwrap();
    assert_eq!(measure("http_error_rate", json!({"window_minutes": 10, "threshold_percent": 1})).await.0, 0.0);

    // Security events: failed sign-ins pile up.
    for _ in 0..3 {
        let _ = app.login_with("root@example.org", "wrong password entirely").await;
    }
    let (n, firing, text) = measure("security_events", json!({"window_minutes": 15, "threshold": 3, "kinds": ["auth.login_failed"]})).await;
    assert!(n >= 3.0 && firing, "{text}");
    assert!(!measure("security_events", json!({"window_minutes": 15, "threshold": 1, "severity": "critical"})).await.1);
    // Jobs that failed for good, and jobs waiting.
    sqlx::query("INSERT INTO jobs (kind, state, finished_at, attempts) VALUES ('deliver', 'failed', now(), 6), ('deliver', 'queued', NULL, 0)").execute(&app.st.db).await.unwrap();
    assert!(measure("job_failures", json!({"window_minutes": 60, "threshold": 1})).await.1);
    assert_eq!(measure("job_backlog", json!({"threshold": 5})).await.0, 1.0);
    // Components answer their health checks in a healthy test server.
    assert_eq!(measure("component_down", json!({})).await, (0.0, false, "all components answer their health checks".to_string()));
    // A node that stopped reporting.
    sqlx::query("INSERT INTO cluster_nodes (node, role, version, seen_at) VALUES ('gone-node', 'server', '0.2.0', now() - interval '20 minutes')").execute(&app.st.db).await.unwrap();
    let (_, firing, text) = measure("node_missing", json!({"minutes": 5})).await;
    assert!(firing && text.contains("gone-node"));

    // The heartbeat writes the samples these rules read, and the system administrator's rule announces to system administrators.
    let mut last = gantry::realtime::Sample::default();
    gantry::realtime::heartbeat(&app.st, "server", &mut last).await.unwrap();
    let names: Vec<String> = sqlx::query_scalar("SELECT DISTINCT name FROM metric_samples WHERE node = $1 ORDER BY 1").bind(&app.st.node_id).fetch_all(&app.st.db).await.unwrap();
    assert!(names.contains(&"requests".to_string()) && names.contains(&"db_pool_percent".to_string()) && names.contains(&"ws_connections".to_string()), "{names:?}");
    admin.created("/api/v1/admin/alerts", json!({"name": "Dead jobs", "kind": "job_failures", "params": {"window_minutes": 60, "threshold": 1}, "severity": "critical"})).await;
    assert_eq!(gantry::notify::check_alerts(&app.st).await.unwrap(), 1);
    assert_eq!(kinds(&admin).await, vec![("admin_system".to_string(), "Alert: Dead jobs".to_string())]);
    // The periodic duties run all of this without failing.
    gantry::jobs::periodic(&app.st).await.unwrap();
}
