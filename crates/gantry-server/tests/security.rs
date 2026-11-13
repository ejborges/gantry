//! Security tests: authentication, authorization boundaries, injection, tampering, abuse limits.

mod common;
use common::*;
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

async fn private_project(app: &TestApp) -> Caller {
    let owner = app.user("owner@example.org", "member").await;
    owner.created("/api/v1/projects", json!({"key": "SEC", "name": "Secret program", "template": "basic"})).await;
    owner.created("/api/v1/projects/SEC/items", json!({"title": "Prototype reactor shielding", "description": "confidential-marker-7781"})).await;
    owner.created("/api/v1/spaces", json!({"key": "SECD", "name": "Secret docs"})).await;
    owner.created("/api/v1/spaces/SECD/pages", json!({"title": "Shielding design", "content": "confidential-marker-7781"})).await;
    owner
}

#[tokio::test]
async fn every_private_route_rejects_anonymous_callers() {
    let app = spawn().await;
    let anon = app.anon();
    let id = "00000000-0000-0000-0000-000000000001";
    let mut checked = 0;
    for route in gantry::http::all_routes().docs {
        if route.public || route.path == "/mcp" {
            continue;
        }
        let path = route.path.replace("{key}", "OPS-1").replace("{id}", id).replace("{binding}", id).replace("{user}", id).replace("{version}", "1").replace("{role}", "member").replace("{position}", id).replace("{assignment}", id).replace("{type}", "task").replace("{tile}", "t1");
        assert!(!path.contains('{'), "unsubstituted parameter in {path}");
        let method = Method::from_bytes(route.method.as_bytes()).unwrap();
        let (status, _) = anon.send(method, &path, (route.method != "GET" && route.method != "DELETE").then(|| json!({}))).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{} {} answered {status} to an anonymous caller", route.method, route.path);
        checked += 1;
    }
    assert!(checked > 100, "only {checked} routes were checked");
    // The MCP endpoint answers in JSON-RPC form, still with 401.
    let r = app.http.post(format!("{}/mcp", app.base)).json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert!(r.headers().contains_key("www-authenticate"));
}

#[tokio::test]
async fn invalid_credentials_are_rejected_and_recorded() {
    let app = spawn().await;
    for bad in ["Bearer gty_0123456789_AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA", "Bearer nonsense", "Basic dXNlcjpwYXNz"] {
        let r = app.http.get(format!("{}/api/v1/projects", app.base)).header("authorization", bad).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::UNAUTHORIZED, "{bad}");
    }
    // A forged session cookie is just an anonymous request.
    let r = app.http.get(format!("{}/api/v1/projects", app.base)).header("cookie", "gantry_session=forged-value").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    assert!(app.security_kinds().await.contains(&"auth.token_rejected".to_string()));
}

#[tokio::test]
async fn csrf_protection_on_cookie_sessions() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    let cookie = ada.cookie.clone().unwrap();
    let post = |csrf: Option<&str>, origin: Option<&str>| {
        let mut r = app.http.post(format!("{}/api/v1/projects", app.base)).header("cookie", &cookie).json(&json!({"key": "CSRF", "name": "x", "template": "basic"}));
        if let Some(c) = csrf {
            r = r.header("x-gantry-csrf", c);
        }
        if let Some(o) = origin {
            r = r.header("origin", o);
        }
        r.send()
    };
    assert_eq!(post(None, None).await.unwrap().status(), StatusCode::FORBIDDEN, "no token");
    assert_eq!(post(Some("guess"), None).await.unwrap().status(), StatusCode::FORBIDDEN, "wrong token");
    let csrf = ada.csrf.clone().unwrap();
    assert_eq!(post(Some(&csrf), Some("https://evil.example")).await.unwrap().status(), StatusCode::FORBIDDEN, "right token from a foreign origin");
    assert_eq!(ada.ok("/api/v1/projects").await.as_array().unwrap().len(), 0, "none of the forged requests had any effect");
    assert_eq!(post(Some(&csrf), Some(&app.base)).await.unwrap().status(), StatusCode::CREATED, "same-origin request with the token succeeds");
    assert!(app.security_kinds().await.iter().filter(|k| *k == "csrf.rejected").count() >= 3);
    // API tokens are not ambient credentials, so they need no CSRF token.
    let tok = ada.with_token(&["read", "write"]).await;
    assert_eq!(tok.post("/api/v1/projects", json!({"key": "TOK", "name": "x", "template": "basic"})).await.0, StatusCode::CREATED);
}

#[tokio::test]
async fn token_scopes_limit_what_a_token_can_do() {
    let app = spawn().await;
    let root = app.admin().await;
    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    root.created("/api/v1/projects/OPS/items", json!({"title": "Existing"})).await;

    let read = root.with_token(&["read"]).await;
    assert_eq!(read.get("/api/v1/items/OPS-1").await.0, StatusCode::OK);
    assert_eq!(read.post("/api/v1/projects/OPS/items", json!({"title": "x"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(read.patch("/api/v1/items/OPS-1", json!({"title": "x"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(read.delete("/api/v1/items/OPS-1").await.0, StatusCode::FORBIDDEN);
    assert_eq!(read.get("/api/v1/admin/users").await.0, StatusCode::FORBIDDEN, "even a system administrator's read token cannot read admin data");

    let write = root.with_token(&["read", "write"]).await;
    assert_eq!(write.post("/api/v1/projects/OPS/items", json!({"title": "Created by token"})).await.0, StatusCode::CREATED);
    assert_eq!(write.get("/api/v1/admin/audit").await.0, StatusCode::FORBIDDEN);
    assert_eq!(write.post("/api/v1/admin/users", json!({"email": "x@example.org", "name": "x", "password": PASSWORD})).await.0, StatusCode::FORBIDDEN);

    let admin = root.with_token(&["read", "write", "admin"]).await;
    assert_eq!(admin.get("/api/v1/admin/users").await.0, StatusCode::OK);

    // People without administrative rights cannot mint admin-scoped tokens at all.
    let ada = app.user("ada@example.org", "member").await;
    assert_eq!(ada.post("/api/v1/tokens", json!({"name": "x", "scopes": ["admin"]})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(ada.post("/api/v1/tokens", json!({"name": "x", "scopes": ["root"]})).await.0, StatusCode::BAD_REQUEST);
    // Nobody can revoke someone else's token unless they administer security.
    let listed = root.ok("/api/v1/tokens").await;
    assert_eq!(ada.delete(&format!("/api/v1/tokens/{}", listed[0]["id"].as_str().unwrap())).await.0, StatusCode::NOT_FOUND);
    assert!(app.security_kinds().await.contains(&"authz.read_only_token".to_string()));
}

#[tokio::test]
async fn private_content_is_invisible_to_other_users() {
    let app = spawn().await;
    let owner = private_project(&app).await;
    let page_id = owner.ok("/api/v1/spaces/SECD").await["pages"][0]["id"].as_str().unwrap().to_string();
    let up = owner.request(Method::POST, "/api/v1/items/SEC-1/attachments?filename=design.txt").body("confidential-marker-7781").send().await.unwrap();
    let att_id = up.json::<Value>().await.unwrap()["id"].as_str().unwrap().to_string();
    owner.created("/api/v1/projects", json!({"key": "PUB", "name": "Open", "template": "basic", "access": "internal"})).await;
    owner.created("/api/v1/projects/PUB/items", json!({"title": "Visible to all"})).await;
    let link = owner.created("/api/v1/items/PUB-1/links", json!({"rel": "relates_to", "target": "SEC-1"})).await;
    owner.created("/api/v1/items/PUB-1/links", json!({"rel": "documented_in", "target": {"type": "page", "id": page_id}})).await;
    app.drain_jobs().await;

    let eve = app.user("eve@example.org", "member").await;
    // Direct access answers 404, exactly as for something that does not exist.
    for path in ["/api/v1/projects/SEC", "/api/v1/items/SEC-1", "/api/v1/items/SEC-1/graph", "/api/v1/spaces/SECD", &format!("/api/v1/pages/{page_id}"), &format!("/api/v1/pages/{page_id}/versions"), &format!("/api/v1/attachments/{att_id}"), "/api/v1/projects/SEC/queues"] {
        assert_eq!(eve.get(path).await.0, StatusCode::NOT_FOUND, "{path}");
    }
    assert_eq!(eve.get("/api/v1/items/SEC-999").await.0, StatusCode::NOT_FOUND);
    for (method, path, body) in [
        (Method::POST, "/api/v1/projects/SEC/items", json!({"title": "x"})),
        (Method::PATCH, "/api/v1/items/SEC-1", json!({"title": "x"})),
        (Method::POST, "/api/v1/items/SEC-1/comments", json!({"body": "x"})),
        (Method::POST, "/api/v1/items/SEC-1/transition", json!({"to": "done"})),
        (Method::DELETE, "/api/v1/items/SEC-1", json!({})),
        (Method::PATCH, "/api/v1/projects/SEC", json!({"access": "internal"})),
        (Method::POST, "/api/v1/projects/SEC/members", json!({"user_id": eve.user_id, "role": "project_admin"})),
        (Method::PATCH, &format!("/api/v1/pages/{page_id}"), json!({"content": "x", "base_version": 1})),
        (Method::POST, "/api/v1/spaces/SECD/pages", json!({"title": "x"})),
        (Method::DELETE, &format!("/api/v1/links/{}", link["id"].as_str().unwrap()), json!({})),
    ] {
        let (s, _) = eve.send(method.clone(), path, Some(body)).await;
        assert!(s == StatusCode::NOT_FOUND || s == StatusCode::FORBIDDEN, "{method} {path} answered {s}");
    }
    // Lists, search and graph neighbors never mention it.
    let everything = [
        eve.ok("/api/v1/projects").await, eve.ok("/api/v1/spaces").await, eve.ok("/api/v1/items?limit=500").await, eve.ok("/api/v1/items?project=SEC").await,
        eve.ok("/api/v1/search?q=confidential-marker-7781").await, eve.ok("/api/v1/search?q=shielding&mode=semantic").await, eve.ok("/api/v1/search?q=reactor&mode=hybrid").await,
        eve.ok("/api/v1/items/PUB-1").await, eve.ok("/api/v1/items/PUB-1/graph?depth=4").await,
    ];
    for v in &everything {
        let text = v.to_string();
        assert!(!text.contains("7781") && !text.contains("SEC-1") && !text.to_lowercase().contains("shielding") && !text.contains("Secret"), "leak: {text}");
    }
    assert_eq!(eve.ok("/api/v1/items/PUB-1").await["links"].as_array().unwrap().len(), 0, "links to invisible things are omitted");
    assert_eq!(eve.post("/api/v1/items/PUB-1/links", json!({"rel": "relates_to", "target": "SEC-1"})).await.0, StatusCode::FORBIDDEN, "internal visibility does not include linking");
    owner.created("/api/v1/projects/PUB/members", json!({"user_id": eve.user_id, "role": "project_member"})).await;
    let (s, v) = eve.post("/api/v1/items/PUB-1/links", json!({"rel": "relates_to", "target": "SEC-1"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "even with link rights, you cannot link to what you cannot see");
    assert_eq!(v["detail"], "target item not found");
    assert_eq!(eve.post("/api/v1/items/PUB-1/links", json!({"rel": "relates_to", "target": "SEC-404"})).await.1["detail"], "target item not found", "same answer as for an item that does not exist");
    assert_eq!(eve.post("/api/v1/items/PUB-1/links", json!({"rel": "documented_in", "target": {"type": "page", "id": page_id}})).await.0, StatusCode::BAD_REQUEST);
    // The item is untouched and the denials were recorded.
    assert_eq!(owner.ok("/api/v1/items/SEC-1").await["title"], "Prototype reactor shielding");
    assert!(app.security_kinds().await.iter().filter(|k| *k == "authz.denied").count() >= 10);
    // An auditor can read everything but change nothing.
    let auditor = app.user("audit@example.org", "auditor").await;
    assert_eq!(auditor.get("/api/v1/items/SEC-1").await.0, StatusCode::OK);
    assert_eq!(auditor.patch("/api/v1/items/SEC-1", json!({"title": "x"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(auditor.patch(&format!("/api/v1/pages/{page_id}"), json!({"content": "x", "base_version": 1})).await.0, StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn project_roles_bound_what_members_can_do() {
    let app = spawn().await;
    let owner = private_project(&app).await;
    let viewer = app.user("viewer@example.org", "member").await;
    owner.created("/api/v1/projects/SEC/members", json!({"user_id": viewer.user_id, "role": "project_viewer"})).await;
    assert_eq!(viewer.get("/api/v1/items/SEC-1").await.0, StatusCode::OK);
    for (method, path, body) in [
        (Method::POST, "/api/v1/projects/SEC/items", json!({"title": "x"})),
        (Method::PATCH, "/api/v1/items/SEC-1", json!({"title": "x"})),
        (Method::POST, "/api/v1/items/SEC-1/comments", json!({"body": "x"})),
        (Method::POST, "/api/v1/items/SEC-1/transition", json!({"to": "done"})),
        (Method::DELETE, "/api/v1/items/SEC-1", json!({})),
        (Method::PATCH, "/api/v1/projects/SEC", json!({"name": "Mine"})),
        (Method::POST, "/api/v1/projects/SEC/members", json!({"user_id": viewer.user_id, "role": "project_admin"})),
        (Method::POST, "/api/v1/projects/SEC/cycles", json!({"name": "x"})),
        (Method::PUT, "/api/v1/projects/SEC/types", json!({"types": ["task"]})),
    ] {
        assert_eq!(viewer.send(method.clone(), path, Some(body)).await.0, StatusCode::FORBIDDEN, "{method} {path}");
    }
    // A member can work items but not administer the project or archive items.
    let member = app.user("member@example.org", "member").await;
    owner.created("/api/v1/projects/SEC/members", json!({"user_id": member.user_id, "role": "project_member"})).await;
    assert_eq!(member.patch("/api/v1/items/SEC-1", json!({"priority": 4})).await.0, StatusCode::OK);
    assert_eq!(member.delete("/api/v1/items/SEC-1").await.0, StatusCode::FORBIDDEN);
    assert_eq!(member.patch("/api/v1/projects/SEC", json!({"access": "internal"})).await.0, StatusCode::FORBIDDEN);
    // Space roles cannot be used on projects and vice versa.
    assert_eq!(owner.post("/api/v1/projects/SEC/members", json!({"user_id": member.user_id, "role": "space_admin"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(owner.post("/api/v1/projects/SEC/members", json!({"user_id": member.user_id, "role": "system_admin"})).await.0, StatusCode::BAD_REQUEST);
    // Removing the grant removes access at once.
    let binding = owner.ok("/api/v1/projects/SEC").await["members"].as_array().unwrap().iter().find(|m| m["user"]["id"] == json!(member.user_id)).unwrap()["binding_id"].as_str().unwrap().to_string();
    assert_eq!(owner.delete(&format!("/api/v1/projects/SEC/members/{binding}")).await.0, StatusCode::NO_CONTENT);
    assert_eq!(member.get("/api/v1/items/SEC-1").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn privilege_escalation_is_blocked() {
    let app = spawn().await;
    let root = app.admin().await;
    let app_admin = app.user("appadmin@example.org", "app_admin").await;
    let ada = app.make_user("ada@example.org", "member").await;
    // An application administrator manages users but cannot create or promote system administrators.
    assert_eq!(app_admin.post("/api/v1/admin/users", json!({"email": "new@example.org", "name": "New", "password": PASSWORD})).await.0, StatusCode::CREATED);
    assert_eq!(app_admin.post("/api/v1/admin/users", json!({"email": "boss@example.org", "name": "Boss", "password": PASSWORD, "role": "system_admin"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app_admin.post(&format!("/api/v1/admin/users/{ada}/roles"), json!({"role": "system_admin"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app_admin.post(&format!("/api/v1/admin/users/{}/roles", app_admin.user_id), json!({"role": "system_admin"})).await.0, StatusCode::FORBIDDEN, "self-promotion");
    assert_eq!(app_admin.post(&format!("/api/v1/admin/users/{ada}/roles"), json!({"role": "auditor"})).await.0, StatusCode::FORBIDDEN, "auditor includes admin.security, which an application administrator lacks");
    assert_eq!(app_admin.delete(&format!("/api/v1/admin/users/{}/roles/system_admin", root.user_id)).await.0, StatusCode::FORBIDDEN, "cannot demote a system administrator");
    // Nor through a custom role, a group, a password reset, sign-in settings or system settings.
    assert_eq!(app_admin.post("/api/v1/admin/roles", json!({"key": "sneaky", "name": "Sneaky", "scope": "global", "permissions": ["admin.settings"]})).await.0, StatusCode::FORBIDDEN);
    let g = root.created("/api/v1/admin/groups", json!({"name": "Admins"})).await;
    let gid = g["id"].as_str().unwrap();
    root.post(&format!("/api/v1/admin/groups/{gid}/roles"), json!({"role": "system_admin"})).await;
    assert_eq!(app_admin.post(&format!("/api/v1/admin/groups/{gid}/members"), json!({"user_id": app_admin.user_id})).await.0, StatusCode::FORBIDDEN, "joining a group is a grant of its roles");
    assert_eq!(app_admin.patch(&format!("/api/v1/admin/users/{}", root.user_id), json!({"password": "a password I now control"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app_admin.get("/api/v1/admin/auth-providers").await.0, StatusCode::FORBIDDEN);
    assert_eq!(app_admin.put("/api/v1/admin/settings/site", json!({"name": "x"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(app_admin.get("/api/v1/admin/security-events").await.0, StatusCode::FORBIDDEN);
    assert_eq!(app_admin.post(&format!("/api/v1/admin/users/{ada}/tokens"), json!({"name": "x", "scopes": ["read"]})).await.0, StatusCode::FORBIDDEN);
    assert!(app.login_with("root@example.org", PASSWORD).await.is_ok(), "the administrator's password is unchanged");
    // The last system administrator cannot be removed.
    assert_eq!(root.patch(&format!("/api/v1/admin/users/{}", root.user_id), json!({"active": false})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.delete(&format!("/api/v1/admin/users/{}/roles/system_admin", root.user_id)).await.0, StatusCode::BAD_REQUEST);
    // Single sign-on can never provision people straight into an administrative role.
    let (s, v) = root.post("/api/v1/admin/auth-providers", json!({"name": "IdP", "issuer": "https://idp.invalid", "client_id": "c", "default_role": "system_admin"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains("administrative"));
}

#[tokio::test]
async fn injection_attempts_are_treated_as_data() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    let sqli = "'; DROP TABLE items; --";
    let xss = "<script>alert(document.cookie)</script><img src=x onerror=alert(1)>";
    let item = ada.created("/api/v1/projects/OPS/items", json!({"title": sqli, "description": xss, "labels": ["a' OR '1'='1"]})).await;
    assert_eq!(item["title"], sqli);
    assert_eq!(item["description"], xss, "content is stored and returned verbatim as JSON data; the client renders it safely");
    for q in [sqli, "%", "_", "\\", "a' OR '1'='1", "\"; SELECT pg_sleep(5); --", "$1", "\u{0}"] {
        let enc: String = url::form_urlencoded::byte_serialize(q.as_bytes()).collect();
        for path in [format!("/api/v1/items?q={enc}"), format!("/api/v1/items?label={enc}"), format!("/api/v1/search?q={enc}"), format!("/api/v1/search?q={enc}&mode=semantic"), format!("/api/v1/items?status={enc}&type={enc}")] {
            let (s, _) = ada.get(&path).await;
            assert!(s == StatusCode::OK || s == StatusCode::BAD_REQUEST, "{path} answered {s}");
        }
    }
    assert_eq!(ada.ok("/api/v1/items?q=%25").await["total"], 0, "a literal % matches nothing; it is not a wildcard");
    assert_eq!(ada.get(&format!("/api/v1/items/{}", "OPS-1%27%3B--")).await.0, StatusCode::NOT_FOUND);
    assert_eq!(ada.get("/api/v1/projects/OPS%27%20OR%201=1").await.0, StatusCode::NOT_FOUND);
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM items").fetch_one(&app.st.db).await.unwrap();
    assert_eq!(count, 1, "the table is intact");
    // Responses carry the headers that stop a browser from interpreting data as code.
    let r = ada.request(Method::GET, "/api/v1/items/OPS-1").send().await.unwrap();
    let h = r.headers();
    assert_eq!(h["content-type"], "application/json");
    assert_eq!(h["x-content-type-options"], "nosniff");
    assert_eq!(h["x-frame-options"], "DENY");
    assert_eq!(h["cache-control"], "no-store");
    assert!(h["content-security-policy"].to_str().unwrap().contains("script-src 'self'") && h["content-security-policy"].to_str().unwrap().contains("frame-ancestors 'none'"));
    assert!(h.contains_key("x-request-id"));
    // Malformed bodies are client errors, not crashes.
    let r = ada.request(Method::POST, "/api/v1/projects/OPS/items").header("content-type", "application/json").body("{not json").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::BAD_REQUEST);
    assert_eq!(ada.post("/api/v1/projects/OPS/items", json!({"title": "x".repeat(501)})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ada.post("/api/v1/projects/OPS/items", json!({"title": "x", "priority": 99})).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn uploaded_files_cannot_run_as_pages() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Files"})).await;
    // Every attachment answer is sandboxed with no script permission and no network access,
    // whatever it is served as.
    let locked_down = |r: &reqwest::Response, name: &str| {
        let csp = r.headers()["content-security-policy"].to_str().unwrap().to_string();
        assert!(csp.starts_with("sandbox;") && csp.contains("default-src 'none'"), "{name}: {csp}");
        assert!(!csp.contains("allow-") && !csp.contains("script-src") && !csp.contains("connect-src"), "{name}: {csp}");
        assert_eq!(r.headers()["x-content-type-options"], "nosniff");
    };
    for (name, ctype) in [("evil.html", "text/html"), ("evil.xhtml", "application/xhtml+xml"), ("evil.js", "application/javascript"), ("evil.xml", "text/xml")] {
        let up: Value = ada.request(Method::POST, &format!("/api/v1/items/OPS-1/attachments?filename={name}")).header("content-type", ctype).body("<script>alert(1)</script>").send().await.unwrap().json().await.unwrap();
        let r = ada.request(Method::GET, &format!("/api/v1/attachments/{}", up["id"].as_str().unwrap())).send().await.unwrap();
        assert_eq!(r.headers()["content-type"], "application/octet-stream", "{name} must not be served with an executable type");
        assert!(r.headers()["content-disposition"].to_str().unwrap().starts_with("attachment"), "{name} must download, not render");
        locked_down(&r, name);
    }
    // SVG is a picture format that can carry script. It is served as a picture so `<img>`
    // can show it (scripts never run there), and the sandbox keeps it inert if someone opens
    // the address directly.
    let up: Value = ada.request(Method::POST, "/api/v1/items/OPS-1/attachments?filename=evil.svg").header("content-type", "image/svg+xml").body("<svg xmlns='http://www.w3.org/2000/svg'><script>alert(1)</script></svg>").send().await.unwrap().json().await.unwrap();
    let r = ada.request(Method::GET, &format!("/api/v1/attachments/{}", up["id"].as_str().unwrap())).send().await.unwrap();
    assert_eq!(r.headers()["content-type"], "image/svg+xml");
    locked_down(&r, "evil.svg");
    // Images may display inline, still sandboxed.
    let up: Value = ada.request(Method::POST, "/api/v1/items/OPS-1/attachments?filename=photo.png").header("content-type", "image/png").body(vec![0x89u8, 0x50, 0x4e, 0x47]).send().await.unwrap().json().await.unwrap();
    let r = ada.request(Method::GET, &format!("/api/v1/attachments/{}", up["id"].as_str().unwrap())).send().await.unwrap();
    assert_eq!(r.headers()["content-type"], "image/png");
    locked_down(&r, "photo.png");
    // Files land under content hashes, never under a caller-supplied name.
    let up: Value = ada.request(Method::POST, "/api/v1/items/OPS-1/attachments?filename=..%2F..%2F..%2Fetc%2Fcron.d%2Fowned").body("x").send().await.unwrap().json().await.unwrap();
    assert_eq!(up["filename"], "owned");
    let stored: Vec<String> = sqlx::query_scalar("SELECT blob_key FROM attachments").fetch_all(&app.st.db).await.unwrap();
    assert!(stored.iter().all(|k| k.len() == 64 && k.bytes().all(|b| b.is_ascii_hexdigit())));
}

#[tokio::test]
async fn oversized_requests_are_refused() {
    let app = spawn_with(|c| c.server.max_body_mb = 1).await;
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Files"})).await;
    let r = ada.request(Method::POST, "/api/v1/items/OPS-1/attachments?filename=big.bin").body(vec![0u8; 2 * 1024 * 1024]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::PAYLOAD_TOO_LARGE);
    let r = ada.request(Method::POST, "/api/v1/items/OPS-1/attachments?filename=ok.bin").body(vec![0u8; 512 * 1024]).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::CREATED);
}

#[tokio::test]
async fn repeated_failed_sign_ins_lock_the_account() {
    let app = spawn_with(|c| {
        c.auth.lockout_threshold = 3;
        c.auth.lockout_minutes = 15;
    })
    .await;
    app.make_user("ada@example.org", "member").await;
    for _ in 0..3 {
        assert_eq!(app.login_with("ada@example.org", "wrong wrong wrong").await.err(), Some(StatusCode::UNAUTHORIZED));
    }
    assert_eq!(app.login_with("ada@example.org", PASSWORD).await.err(), Some(StatusCode::UNAUTHORIZED), "locked: even the right password is refused");
    let kinds = app.security_kinds().await;
    assert!(kinds.contains(&"auth.lockout".to_string()) && kinds.iter().filter(|k| *k == "auth.login_failed").count() >= 2, "{kinds:?}");
    // An administrator can unlock.
    let root = app.admin().await;
    let ada_id: uuid::Uuid = sqlx::query_scalar("SELECT id FROM users WHERE email = 'ada@example.org'").fetch_one(&app.st.db).await.unwrap();
    assert_eq!(root.patch(&format!("/api/v1/admin/users/{ada_id}"), json!({"unlock": true})).await.0, StatusCode::NO_CONTENT);
    assert!(app.login_with("ada@example.org", PASSWORD).await.is_ok());
    // The response for an unknown account is indistinguishable from a wrong password.
    let a = app.http.post(format!("{}/api/v1/auth/login", app.base)).json(&json!({"email": "ghost@example.org", "password": "whatever it is"})).send().await.unwrap();
    let b = app.http.post(format!("{}/api/v1/auth/login", app.base)).json(&json!({"email": "root@example.org", "password": "whatever it is"})).send().await.unwrap();
    assert_eq!(a.status(), b.status());
    assert_eq!(a.text().await.unwrap(), b.text().await.unwrap());
}

#[tokio::test]
async fn sign_in_attempts_are_throttled_per_address() {
    let app = spawn_with(|c| c.auth.login_attempts_per_ip = 5).await;
    for i in 0..5 {
        assert_eq!(app.login_with(&format!("user{i}@example.org"), "guess guess guess").await.err(), Some(StatusCode::UNAUTHORIZED));
    }
    assert_eq!(app.login_with("user9@example.org", "guess guess guess").await.err(), Some(StatusCode::TOO_MANY_REQUESTS));
    assert!(app.security_kinds().await.contains(&"auth.login_throttled".to_string()));
}

#[tokio::test]
async fn successful_sign_ins_do_not_count_toward_the_address_limit() {
    // Everyone in an office arrives from one address; only failures may throttle it.
    let app = spawn_with(|c| c.auth.login_attempts_per_ip = 3).await;
    app.make_user("ada@example.org", "member").await;
    for _ in 0..8 {
        assert!(app.login_with("ada@example.org", PASSWORD).await.is_ok());
    }
    for _ in 0..3 {
        assert_eq!(app.login_with("ghost@example.org", "guess guess guess").await.err(), Some(StatusCode::UNAUTHORIZED));
    }
    // Once the address is throttled, even a correct password waits: the limit cannot be probed.
    assert_eq!(app.login_with("ada@example.org", PASSWORD).await.err(), Some(StatusCode::TOO_MANY_REQUESTS));
}

#[tokio::test]
async fn request_rate_is_limited() {
    let app = spawn_with(|c| c.server.rate_limit_per_minute = 20).await;
    let ada = app.user("ada@example.org", "member").await;
    let mut limited = 0;
    for _ in 0..40 {
        let r = ada.request(Method::GET, "/api/v1/projects").send().await.unwrap();
        if r.status() == StatusCode::TOO_MANY_REQUESTS {
            assert_eq!(r.headers()["retry-after"], "60");
            limited += 1;
        }
    }
    assert!(limited >= 15, "only {limited} of 40 requests were limited");
}

#[tokio::test]
async fn sessions_expire_and_cookies_are_hardened() {
    let app = spawn_with(|c| c.server.public_url = "https://gantry.example.org".into()).await;
    app.make_user("ada@example.org", "member").await;
    let r = app.http.post(format!("{}/api/v1/auth/login", app.base)).json(&json!({"email": "ada@example.org", "password": PASSWORD})).send().await.unwrap();
    let set = r.headers()["set-cookie"].to_str().unwrap().to_string();
    assert!(set.starts_with("__Host-gantry_session="), "{set}");
    for attr in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/"] {
        assert!(set.contains(attr), "cookie lacks {attr}: {set}");
    }
    assert!(r.headers()["strict-transport-security"].to_str().unwrap().contains("max-age=31536000"));
    let cookie = set.split(';').next().unwrap().to_string();
    let me = || app.http.get(format!("{}/api/v1/auth/me", app.base)).header("cookie", &cookie).send();
    assert_eq!(me().await.unwrap().status(), StatusCode::OK);
    // Only a hash of the session token is stored.
    let token = cookie.split_once('=').unwrap().1;
    let stored: Vec<u8> = sqlx::query_scalar("SELECT token_hash FROM sessions").fetch_one(&app.st.db).await.unwrap();
    assert_eq!(stored, gantry::crypto::sha256(token.as_bytes()));
    // Idle timeout.
    sqlx::query("UPDATE sessions SET last_seen_at = now() - interval '9 hours'").execute(&app.st.db).await.unwrap();
    assert_eq!(me().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    // Absolute timeout.
    sqlx::query("UPDATE sessions SET last_seen_at = now(), expires_at = now() - interval '1 second'").execute(&app.st.db).await.unwrap();
    assert_eq!(me().await.unwrap().status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn secrets_never_appear_in_logs_or_responses() {
    let app = spawn().await;
    let root = app.admin().await;
    let secret_pw = "a very secret password 4471";
    let created = root.created("/api/v1/admin/users", json!({"email": "ada@example.org", "name": "Ada", "password": secret_pw})).await;
    root.patch(&format!("/api/v1/admin/users/{}", created["id"].as_str().unwrap()), json!({"password": "another secret password 9902"})).await;
    let tok = root.created("/api/v1/tokens", json!({"name": "t", "scopes": ["read"]})).await;
    let token = tok["token"].as_str().unwrap();
    let integ = root.created("/api/v1/admin/integrations", json!({"kind": "teams", "name": "Ops channel", "secrets": {"workflow_url": "https://example.org/hook/super-secret-path-5512"}, "events": ["*"]})).await;
    root.patch(&format!("/api/v1/admin/integrations/{}", integ["id"].as_str().unwrap()), json!({"name": "Ops channel 2"})).await;

    let audit: String = sqlx::query_scalar("SELECT string_agg(detail::text || target_id || actor_label, ' ') FROM audit_log").fetch_one(&app.st.db).await.unwrap();
    // (Whole secrets, or parts long enough that a random identifier cannot contain them by chance.)
    for secret in [secret_pw, "another secret password 9902", "secret password", token, token.rsplit('_').next().unwrap(), "super-secret-path-5512", PASSWORD] {
        assert!(!audit.contains(secret), "the audit log contains a secret ({secret})");
    }
    // Stored forms: password hashed with Argon2id, token hashed, integration secret encrypted.
    let hash: String = sqlx::query_scalar("SELECT password_hash FROM users WHERE email = 'ada@example.org'").fetch_one(&app.st.db).await.unwrap();
    assert!(hash.starts_with("$argon2id$"));
    let sealed: String = sqlx::query_scalar("SELECT secrets_enc FROM integrations").fetch_one(&app.st.db).await.unwrap();
    assert!(sealed.starts_with("v1:") && !sealed.contains("super-secret"));
    let token_rows: String = sqlx::query_scalar("SELECT string_agg(prefix || encode(secret_hash, 'hex'), ' ') FROM api_tokens").fetch_one(&app.st.db).await.unwrap();
    assert!(!token_rows.contains(token.rsplit('_').next().unwrap()));
    // API responses list which secrets are set, never their values.
    let listed = root.ok("/api/v1/admin/integrations").await;
    assert_eq!(listed["integrations"][0]["secrets_set"], json!(["workflow_url"]));
    assert!(!listed.to_string().contains("super-secret"));
    assert!(root.ok("/api/v1/tokens").await.to_string().find(token).is_none());
    // Server errors do not leak internals.
    let (s, v) = root.get("/api/v1/pages/not-a-uuid").await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(!v.to_string().to_lowercase().contains("sqlx") && !v.to_string().contains("postgres"));
}

#[tokio::test]
async fn audit_log_cannot_be_altered_and_tampering_is_detected() {
    let app = spawn().await;
    let root = app.admin().await;
    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    for i in 0..5 {
        root.created("/api/v1/projects/OPS/items", json!({"title": format!("Item {i}")})).await;
    }
    // The application's database role cannot rewrite history through SQL.
    for sql in ["UPDATE audit_log SET actor_label = 'someone else'", "DELETE FROM audit_log", "TRUNCATE audit_log"] {
        let err = sqlx::query(sql).execute(&app.st.db).await.unwrap_err().to_string();
        assert!(err.contains("append-only"), "{sql}: {err}");
    }
    assert_eq!(root.post("/api/v1/admin/audit/verify", json!({})).await.1["intact"], true);
    // Someone with superuser access who disables the trigger still breaks the hash chain.
    sqlx::query("ALTER TABLE audit_log DISABLE TRIGGER audit_log_no_change").execute(&app.st.db).await.unwrap();
    let victim: i64 = sqlx::query_scalar("SELECT id FROM audit_log WHERE action = 'item.create' ORDER BY id LIMIT 1").fetch_one(&app.st.db).await.unwrap();
    sqlx::query("UPDATE audit_log SET actor_label = 'innocent@example.org' WHERE id = $1").bind(victim).execute(&app.st.db).await.unwrap();
    let report = root.post("/api/v1/admin/audit/verify", json!({})).await.1;
    assert_eq!(report["intact"], false);
    assert_eq!(report["first_broken_id"], victim);
    assert!(app.security_kinds().await.contains(&"audit.chain_broken".to_string()));
    // Deleting a row is detected too.
    sqlx::query("UPDATE audit_log SET actor_label = 'root@example.org' WHERE id = $1").bind(victim).execute(&app.st.db).await.unwrap();
    assert!(gantry::audit::verify_chain(&app.st.db).await.unwrap().intact, "restoring the original value repairs the chain");
    sqlx::query("DELETE FROM audit_log WHERE id = $1").bind(victim).execute(&app.st.db).await.unwrap();
    let report = gantry::audit::verify_chain(&app.st.db).await.unwrap();
    assert!(!report.intact && report.first_broken_id == Some(victim + 1));
}

#[tokio::test]
async fn every_change_is_audited_with_the_actor() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "A"})).await;
    ada.patch("/api/v1/items/OPS-1", json!({"priority": 4})).await;
    ada.post("/api/v1/items/OPS-1/transition", json!({"to": "done"})).await;
    ada.created("/api/v1/items/OPS-1/comments", json!({"body": "done"})).await;
    ada.created("/api/v1/spaces", json!({"key": "DOC", "name": "Docs"})).await;
    let page = ada.created("/api/v1/spaces/DOC/pages", json!({"title": "P"})).await;
    ada.patch(&format!("/api/v1/pages/{}", page["id"].as_str().unwrap()), json!({"content": "x", "base_version": 1})).await;
    ada.delete("/api/v1/items/OPS-1").await;
    let rows: Vec<(String, String, String, String)> = sqlx::query_as("SELECT action, actor_label, ip, request_id FROM audit_log WHERE actor_label = 'ada@example.org' ORDER BY id").fetch_all(&app.st.db).await.unwrap();
    let actions: Vec<&str> = rows.iter().map(|r| r.0.as_str()).collect();
    assert_eq!(actions, vec!["auth.login", "project.create", "item.create", "item.update", "item.transition", "comment.create", "space.create", "page.create", "page.update", "item.archive"]);
    assert!(rows.iter().all(|r| r.2 == "127.0.0.1" && !r.3.is_empty()), "every entry carries the address and request id");
}

#[tokio::test]
async fn outbound_requests_cannot_reach_internal_addresses() {
    let app = spawn_with(|c| {
        c.security.allow_insecure_outbound = false;
        c.security.block_private_networks = true;
    })
    .await;
    let root = app.admin().await;
    for (url, why) in [("http://example.org/hook", "https"), ("https://169.254.169.254/latest/meta-data/", "link-local"), ("https://127.0.0.1:9/x", "loopback"), ("https://[::1]/x", "loopback"), ("https://10.0.0.5/x", "private"), ("https://user:pw@example.org/x", "credentials")] {
        let created = root.created("/api/v1/admin/integrations", json!({"kind": "webhook", "name": url, "config": {"url": url}, "events": ["*"]})).await;
        let (s, v) = root.post(&format!("/api/v1/admin/integrations/{}/test", created["id"].as_str().unwrap()), json!({})).await;
        assert_eq!(s, StatusCode::OK);
        assert_eq!(v["ok"], false, "{url} must be refused");
        assert!(v["error"].as_str().unwrap().to_lowercase().contains(why), "{url}: {v}");
    }
    assert_eq!(root.post("/api/v1/admin/integrations", json!({"kind": "webhook", "name": "x", "config": {"url": "file:///etc/passwd"}})).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn inbound_webhooks_require_valid_signatures() {
    let app = spawn().await;
    let root = app.admin().await;
    let gh = root.created("/api/v1/admin/integrations", json!({"kind": "github", "name": "GH", "secrets": {"webhook_secret": "s3cret"}})).await;
    let url = format!("{}/api/v1/hooks/github/{}", app.base, gh["id"].as_str().unwrap());
    let body = r#"{"commits":[]}"#;
    let post = |sig: Option<String>| {
        let mut r = app.http.post(&url).header("x-github-event", "push").header("content-type", "application/json").body(body);
        if let Some(s) = sig {
            r = r.header("x-hub-signature-256", s);
        }
        r.send()
    };
    assert_eq!(post(None).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(post(Some("sha256=00".into())).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    let wrong = format!("sha256={}", hex(&gantry::crypto::hmac_sha256(b"other", body.as_bytes())));
    assert_eq!(post(Some(wrong)).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    let good = format!("sha256={}", hex(&gantry::crypto::hmac_sha256(b"s3cret", body.as_bytes())));
    assert_eq!(post(Some(good)).await.unwrap().status(), StatusCode::OK);
    // Unknown or disabled integrations do not reveal themselves.
    let r = app.http.post(format!("{}/api/v1/hooks/github/00000000-0000-0000-0000-000000000009", app.base)).body(body).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    root.patch(&format!("/api/v1/admin/integrations/{}", gh["id"].as_str().unwrap()), json!({"enabled": false})).await;
    let good = format!("sha256={}", hex(&gantry::crypto::hmac_sha256(b"s3cret", body.as_bytes())));
    assert_eq!(post(Some(good)).await.unwrap().status(), StatusCode::NOT_FOUND);
    assert!(app.security_kinds().await.iter().filter(|k| *k == "webhook.signature_invalid").count() >= 3);
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

#[tokio::test]
async fn metrics_endpoint_is_not_public_when_proxied_or_tokened() {
    let app = spawn().await;
    // Through the reverse proxy (which adds X-Real-IP) the endpoint does not exist.
    let r = app.http.get(format!("{}/metrics", app.base)).header("x-real-ip", "203.0.113.9").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    let app = spawn_with(|c| c.server.metrics_token = "scrape-token-123".into()).await;
    assert_eq!(app.http.get(format!("{}/metrics", app.base)).send().await.unwrap().status(), StatusCode::NOT_FOUND);
    assert_eq!(app.http.get(format!("{}/metrics", app.base)).bearer_auth("wrong").send().await.unwrap().status(), StatusCode::NOT_FOUND);
    assert_eq!(app.http.get(format!("{}/metrics", app.base)).bearer_auth("scrape-token-123").send().await.unwrap().status(), StatusCode::OK);
}

#[tokio::test]
async fn forwarded_address_is_trusted_only_from_the_proxy() {
    // Loopback is the trusted proxy by default, so the forwarded address is recorded.
    let app = spawn().await;
    app.make_user("ada@example.org", "member").await;
    app.http.post(format!("{}/api/v1/auth/login", app.base)).header("x-real-ip", "203.0.113.50").json(&json!({"email": "ada@example.org", "password": PASSWORD})).send().await.unwrap();
    let ip: String = sqlx::query_scalar("SELECT ip FROM audit_log WHERE action = 'auth.login'").fetch_one(&app.st.db).await.unwrap();
    assert_eq!(ip, "203.0.113.50");
    // With no trusted proxy, a client cannot spoof its address.
    let app = spawn_with(|c| c.server.trusted_proxies = vec![]).await;
    app.make_user("ada@example.org", "member").await;
    app.http.post(format!("{}/api/v1/auth/login", app.base)).header("x-real-ip", "203.0.113.50").header("x-forwarded-for", "198.51.100.1").json(&json!({"email": "ada@example.org", "password": PASSWORD})).send().await.unwrap();
    let ip: String = sqlx::query_scalar("SELECT ip FROM audit_log WHERE action = 'auth.login'").fetch_one(&app.st.db).await.unwrap();
    assert_eq!(ip, "127.0.0.1");
}
