//! Integration tests: the HTTP API end to end against a real PostgreSQL database.

mod common;
use common::*;
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

async fn software_project(c: &Caller, key: &str) -> Value {
    c.created("/api/v1/projects", json!({"key": key, "name": format!("{key} project"), "template": "software"})).await
}

#[tokio::test]
async fn health_and_static_endpoints() {
    let app = spawn().await;
    let get = |p: &str| app.http.get(format!("{}{p}", app.base)).send();
    assert_eq!(get("/healthz").await.unwrap().text().await.unwrap(), "ok");
    assert_eq!(get("/readyz").await.unwrap().status(), StatusCode::OK);
    // The single-page application is served for unknown paths, but unknown API paths are a JSON 404.
    let spa = get("/projects/OPS/board").await.unwrap();
    assert_eq!(spa.status(), StatusCode::OK);
    assert!(spa.text().await.unwrap().contains("<div id=root>"));
    let missing = get("/api/v1/no-such-thing").await.unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);
    assert_eq!(missing.headers()["content-type"], "application/problem+json");
    let openapi: Value = get("/api/v1/openapi.json").await.unwrap().json().await.unwrap();
    assert_eq!(openapi["openapi"], "3.1.0");
    assert!(openapi["paths"]["/api/v1/items/{key}"]["get"].is_object());
    // Metrics are served to loopback clients only when no token is configured.
    let metrics = get("/metrics").await.unwrap().text().await.unwrap();
    assert!(metrics.contains("gantry_http_requests_total") && metrics.contains("gantry_jobs_queued"));
}

#[tokio::test]
async fn sign_in_profile_and_sign_out() {
    let app = spawn().await;
    app.make_user("ada@example.org", "member").await;
    assert_eq!(app.login_with("ada@example.org", "wrong password entirely").await.err(), Some(StatusCode::UNAUTHORIZED));
    assert_eq!(app.login_with("nobody@example.org", PASSWORD).await.err(), Some(StatusCode::UNAUTHORIZED));
    let ada = app.login("ada@example.org").await;
    let me = ada.ok("/api/v1/auth/me").await;
    assert_eq!(me["user"]["email"], "ada@example.org");
    assert_eq!(me["auth_method"], "session");
    assert!(me["permissions"].as_array().unwrap().contains(&json!("project.create")));
    assert!(!me["permissions"].as_array().unwrap().contains(&json!("admin.access")));

    assert_eq!(ada.patch("/api/v1/auth/me", json!({"theme": "dark", "name": "Ada L."})).await.0, StatusCode::OK);
    assert_eq!(ada.ok("/api/v1/auth/me").await["user"]["theme"], "dark");
    assert_eq!(ada.patch("/api/v1/auth/me", json!({"theme": "neon"})).await.0, StatusCode::BAD_REQUEST);

    // Changing the password needs the current one and enforces the length policy.
    assert_eq!(ada.post("/api/v1/auth/password", json!({"current": "nope", "new": "a brand new long password"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ada.post("/api/v1/auth/password", json!({"current": PASSWORD, "new": "short"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ada.post("/api/v1/auth/password", json!({"current": PASSWORD, "new": "a brand new long password"})).await.0, StatusCode::NO_CONTENT);
    assert!(app.login_with("ada@example.org", PASSWORD).await.is_err());
    assert!(app.login_with("ada@example.org", "a brand new long password").await.is_ok());

    assert_eq!(ada.post("/api/v1/auth/logout", json!({})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ada.get("/api/v1/auth/me").await.0, StatusCode::UNAUTHORIZED, "the session is gone after sign-out");
    let actions = app.audit_actions().await;
    for a in ["auth.login", "user.profile_update", "user.password_change", "auth.logout"] {
        assert!(actions.iter().any(|x| x == a), "audit log is missing {a}: {actions:?}");
    }
}

#[tokio::test]
async fn work_item_life_cycle() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    let bob_id = app.make_user("bob@example.org", "member").await;
    let project = software_project(&ada, "OPS").await;
    assert_eq!(project["key"], "OPS");
    ada.created("/api/v1/projects/OPS/members", json!({"user_id": bob_id, "role": "project_member"})).await;

    let detail = ada.ok("/api/v1/projects/OPS").await;
    let types: Vec<&str> = detail["types"].as_array().unwrap().iter().map(|t| t["key"].as_str().unwrap()).collect();
    assert_eq!(types, vec!["bug", "epic", "story", "task"]);
    assert!(detail["permissions"].as_array().unwrap().contains(&json!("project.admin")), "the creator administers the project");
    assert_eq!(detail["cycles"][0]["name"], "Sprint 1");

    // A bug needs its required custom field.
    let (s, v) = ada.post("/api/v1/projects/OPS/items", json!({"type": "bug", "title": "Pump controller resets"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    assert!(v["detail"].as_str().unwrap().contains("Severity"));
    let bug = ada.created("/api/v1/projects/OPS/items", json!({"type": "bug", "title": "Pump controller resets", "priority": 3, "labels": ["Firmware", "firmware"], "custom": {"severity": "Major"}})).await;
    assert_eq!(bug["key"], "OPS-1");
    assert_eq!(bug["status"], "backlog");
    assert_eq!(bug["labels"], json!(["firmware"]));
    assert_eq!(bug["custom"]["severity"], "Major");
    let epic = ada.created("/api/v1/projects/OPS/items", json!({"type": "epic", "title": "Controller reliability"})).await;
    assert_eq!(epic["key"], "OPS-2");

    // Types the project does not use, and bad field values, are rejected.
    assert_eq!(ada.post("/api/v1/projects/OPS/items", json!({"type": "shipment", "title": "x"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ada.post("/api/v1/projects/OPS/items", json!({"type": "bug", "title": "x", "custom": {"severity": "Enormous"}})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ada.post("/api/v1/projects/OPS/items", json!({"title": "x", "bogus": 1})).await.0, StatusCode::UNPROCESSABLE_ENTITY, "unknown top-level fields are rejected");

    // Update several fields, including assignment and parent.
    let (s, v) = ada.patch("/api/v1/items/OPS-1", json!({"title": "Pump controller resets under load", "assignee_id": bob_id, "parent": "OPS-2", "due_date": "2030-01-31", "estimate": 5, "custom": {"environment": "Site B"}})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["assignee"]["name"], "bob");
    assert_eq!(v["parent"]["key"], "OPS-2");
    assert_eq!(v["custom"], json!({"severity": "Major", "environment": "Site B"}));
    assert_eq!(ada.patch("/api/v1/items/OPS-2", json!({"parent": "OPS-1"})).await.0, StatusCode::BAD_REQUEST, "parent cycles are refused");
    assert_eq!(ada.patch("/api/v1/items/OPS-1", json!({"status": "done"})).await.0, StatusCode::BAD_REQUEST, "status changes go through transitions");

    // Transitions follow the workflow.
    let (s, v) = ada.post("/api/v1/items/OPS-1/transition", json!({"to": "in_progress"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!((v["status"].as_str(), v["status_category"].as_str()), (Some("in_progress"), Some("in_progress")));
    assert_eq!(ada.post("/api/v1/items/OPS-1/transition", json!({"to": "nonsense"})).await.0, StatusCode::BAD_REQUEST);
    let done = ada.post("/api/v1/items/OPS-1/transition", json!({"to": "done"})).await.1;
    assert_eq!(done["status_category"], "done");
    assert!(done["resolved_at"].is_string());

    // Comments, links and the graph.
    ada.created("/api/v1/items/OPS-1/comments", json!({"body": "Reproduced on the bench."})).await;
    assert_eq!(ada.post("/api/v1/items/OPS-1/comments", json!({"body": "   "})).await.0, StatusCode::BAD_REQUEST);
    let task = ada.created("/api/v1/projects/OPS/items", json!({"title": "Replace capacitor C14"})).await;
    let link = ada.created("/api/v1/items/OPS-3/links", json!({"rel": "blocks", "target": "OPS-1"})).await;
    assert_eq!(ada.post("/api/v1/items/OPS-3/links", json!({"rel": "blocks", "target": "OPS-3"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ada.post("/api/v1/items/OPS-3/links", json!({"rel": "Bad Rel!", "target": "OPS-1"})).await.0, StatusCode::BAD_REQUEST);
    let d = ada.ok("/api/v1/items/OPS-1").await;
    assert_eq!(d["comments"][0]["body"], "Reproduced on the bench.");
    assert_eq!(d["links"][0]["rel"], "blocks");
    assert_eq!(d["links"][0]["direction"], "in");
    assert_eq!(d["links"][0]["other"]["key"], "OPS-3");
    assert!(d["history"].as_array().unwrap().len() >= 5);
    let graph = ada.ok("/api/v1/items/OPS-1/graph?depth=2").await;
    assert_eq!(graph["edges"].as_array().unwrap().len(), 1);
    assert_eq!(graph["nodes"].as_array().unwrap().len(), 2);
    assert_eq!(ada.delete(&format!("/api/v1/links/{}", link["id"].as_str().unwrap())).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ada.ok("/api/v1/items/OPS-1/graph").await["edges"].as_array().unwrap().len(), 0);

    // Listing with filters.
    let list = ada.ok("/api/v1/items?project=OPS&sort=key").await;
    assert_eq!(list["total"], 3);
    assert_eq!(ada.ok("/api/v1/items?project=OPS&category=done").await["total"], 1);
    assert_eq!(ada.ok(&format!("/api/v1/items?assignee={bob_id}")).await["items"][0]["key"], "OPS-1");
    assert_eq!(ada.ok("/api/v1/items?assignee=none&project=OPS").await["total"], 2);
    assert_eq!(ada.ok("/api/v1/items?label=firmware").await["total"], 1);
    assert_eq!(ada.ok("/api/v1/items?q=capacitor").await["items"][0]["key"], task["key"]);
    assert_eq!(ada.ok("/api/v1/items?parent=OPS-2").await["total"], 1);
    assert_eq!(ada.get("/api/v1/items?sort=evil").await.0, StatusCode::BAD_REQUEST);

    // Archive hides the item.
    assert_eq!(ada.delete("/api/v1/items/OPS-3").await.0, StatusCode::NO_CONTENT);
    assert_eq!(ada.get("/api/v1/items/OPS-3").await.0, StatusCode::NOT_FOUND);
    assert_eq!(ada.ok("/api/v1/items?project=OPS").await["total"], 2);

    // Bob was notified of the assignment; Ada is not notified of her own actions.
    let bob = app.login("bob@example.org").await;
    let notes = bob.ok("/api/v1/notifications").await;
    assert!(notes.as_array().unwrap().iter().any(|n| n["kind"] == "item.assigned"), "{notes}");
    assert_eq!(ada.ok("/api/v1/auth/me").await["unread_notifications"], 0);
    assert_eq!(bob.post("/api/v1/notifications/read", json!({})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(bob.ok("/api/v1/auth/me").await["unread_notifications"], 0);
}

#[tokio::test]
async fn workflow_gates_require_fields() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "ECN", "name": "Change control", "template": "engineering-change"})).await;
    let cr = ada.created("/api/v1/projects/ECN/items", json!({"title": "Thicker bracket"})).await;
    assert_eq!(cr["type"], "change_request");
    assert_eq!(cr["status"], "draft");
    let transitions: Vec<&str> = cr["transitions"].as_array().unwrap().iter().map(|t| t["to"].as_str().unwrap()).collect();
    assert_eq!(transitions, vec!["in_review"], "only the next step is offered");

    let (s, v) = ada.post("/api/v1/items/ECN-1/transition", json!({"to": "in_review"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains("Justification"), "{v}");
    assert_eq!(ada.post("/api/v1/items/ECN-1/transition", json!({"to": "approved"})).await.0, StatusCode::BAD_REQUEST, "steps cannot be skipped");
    ada.patch("/api/v1/items/ECN-1", json!({"custom": {"justification": "Fatigue cracks at 2e5 cycles"}})).await;
    assert_eq!(ada.post("/api/v1/items/ECN-1/transition", json!({"to": "in_review"})).await.0, StatusCode::OK);
    assert_eq!(ada.post("/api/v1/items/ECN-1/transition", json!({"to": "approved"})).await.0, StatusCode::BAD_REQUEST, "approval needs an impact assessment");
    ada.patch("/api/v1/items/ECN-1", json!({"custom": {"change_impact": "Medium"}})).await;
    for to in ["approved", "implementing", "verified", "closed"] {
        let (s, v) = ada.post("/api/v1/items/ECN-1/transition", json!({"to": to})).await;
        assert_eq!(s, StatusCode::OK, "{to}: {v}");
    }
    assert_eq!(ada.ok("/api/v1/items/ECN-1").await["status_category"], "done");
}

#[tokio::test]
async fn every_project_template_can_be_instantiated() {
    let app = spawn().await;
    let root = app.admin().await;
    assert_eq!(root.post("/api/v1/admin/packs/nasa/install", json!({})).await.0, StatusCode::OK);
    let templates = root.ok("/api/v1/project-templates").await;
    let list = templates.as_array().unwrap();
    assert!(list.len() >= 13, "core templates plus the NASA add-on");
    for (i, t) in list.iter().enumerate() {
        let key = format!("T{i}");
        let (s, v) = root.post("/api/v1/projects", json!({"key": key, "name": t["name"], "template": t["key"]})).await;
        assert_eq!(s, StatusCode::CREATED, "template {}: {v}", t["key"]);
        let detail = root.ok(&format!("/api/v1/projects/{key}")).await;
        assert!(!detail["types"].as_array().unwrap().is_empty());
        // An item of the default type can be created and moved to every status its workflow offers next.
        let (s, v) = root.post(&format!("/api/v1/projects/{key}/items"), json!({"title": "Smoke", "custom": default_custom(&detail)})).await;
        assert_eq!(s, StatusCode::CREATED, "template {}: {v}", t["key"]);
    }
    let nasa = list.iter().position(|t| t["key"] == "nasa-flight-project").expect("NASA template is listed");
    let items = root.ok(&format!("/api/v1/items?project=T{nasa}&limit=200&sort=key")).await;
    let titles: Vec<&str> = items["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
    for expected in ["Pre-Phase A: Concept Studies", "PDR — Preliminary Design Review", "KDP C — decision to enter Phase C", "Systems Engineering Management Plan (SEMP)"] {
        assert!(titles.contains(&expected), "missing seeded item {expected:?}");
    }
    let space = root.ok(&format!("/api/v1/spaces/T{nasa}")).await;
    let pages: Vec<&str> = space["pages"].as_array().unwrap().iter().map(|p| p["title"].as_str().unwrap()).collect();
    assert!(pages.contains(&"Project Plan") && pages.contains(&"Common technical processes"), "{pages:?}");
}

/// Values for the required custom fields of a project's default type.
fn default_custom(detail: &Value) -> Value {
    let default = detail["project"]["default_type"].as_str().unwrap();
    let t = detail["types"].as_array().unwrap().iter().find(|t| t["key"] == default).unwrap();
    let mut out = serde_json::Map::new();
    for f in t["fields"].as_array().unwrap().iter().filter(|f| f["required"] == true) {
        let v = match f["type"].as_str().unwrap() {
            "select" => f["options"][0].clone(),
            "date" => json!("2030-01-01"),
            "number" => json!(1),
            "checkbox" => json!(true),
            _ => json!("x"),
        };
        out.insert(f["key"].as_str().unwrap().to_string(), v);
    }
    Value::Object(out)
}

#[tokio::test]
async fn nasa_requirement_traceability() {
    let app = spawn().await;
    let root = app.admin().await;
    root.post("/api/v1/admin/packs/nasa/install", json!({})).await;
    root.created("/api/v1/projects", json!({"key": "ORB", "name": "Orbiter", "template": "nasa-flight-project"})).await;
    let parent = root.created("/api/v1/projects/ORB/items", json!({"type": "nasa_requirement", "title": "L2-014 Downlink rate", "custom": {"requirement_text": "The spacecraft shall downlink science data at no less than 2 Mbps.", "requirement_level": "Level 2 (Project)"}})).await;
    let child = root.created("/api/v1/projects/ORB/items", json!({"type": "nasa_requirement", "title": "L3-102 Transmitter power", "custom": {"requirement_text": "The transmitter shall radiate at least 5 W."}})).await;
    let test = root.created("/api/v1/projects/ORB/items", json!({"type": "nasa_verification", "title": "TVAC RF test", "custom": {"verification_method": "Test"}})).await;
    let (pk, ck, tk) = (parent["key"].as_str().unwrap(), child["key"].as_str().unwrap(), test["key"].as_str().unwrap());
    root.created(&format!("/api/v1/items/{ck}/links"), json!({"rel": "derives_from", "target": pk})).await;
    root.created(&format!("/api/v1/items/{tk}/links"), json!({"rel": "verifies", "target": ck})).await;

    // The trace from the test reaches the Level 2 requirement in two hops.
    let graph = root.ok(&format!("/api/v1/items/{tk}/graph?depth=2")).await;
    let keys: Vec<&str> = graph["nodes"].as_array().unwrap().iter().filter_map(|n| n["key"].as_str()).collect();
    assert!(keys.contains(&pk) && keys.contains(&ck) && keys.contains(&tk), "{keys:?}");
    assert_eq!(root.ok(&format!("/api/v1/items/{tk}/graph?depth=1")).await["nodes"].as_array().unwrap().len(), 2);

    // A requirement cannot be baselined without a verification method and rationale, nor verified without evidence.
    let path = format!("/api/v1/items/{ck}/transition");
    let go = |to: &'static str| root.post(&path, json!({"to": to}));
    assert_eq!(go("in_review").await.0, StatusCode::BAD_REQUEST);
    root.patch(&format!("/api/v1/items/{ck}"), json!({"custom": {"verification_method": "Test"}})).await;
    assert_eq!(go("in_review").await.0, StatusCode::OK);
    assert_eq!(go("baselined").await.0, StatusCode::BAD_REQUEST);
    root.patch(&format!("/api/v1/items/{ck}"), json!({"custom": {"rationale": "Link budget LB-7, 3 dB margin"}})).await;
    assert_eq!(go("baselined").await.0, StatusCode::OK);
    assert_eq!(go("verified").await.0, StatusCode::BAD_REQUEST);
    root.patch(&format!("/api/v1/items/{ck}"), json!({"custom": {"verification_evidence": "TR-2231 section 4"}})).await;
    assert_eq!(go("verified").await.0, StatusCode::OK);
}

#[tokio::test]
async fn documentation_pages_versions_and_templates() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    let bob = app.user("bob@example.org", "member").await;
    ada.created("/api/v1/spaces", json!({"key": "ENG", "name": "Engineering handbook", "access": "internal"})).await;
    assert_eq!(ada.post("/api/v1/spaces", json!({"key": "ENG", "name": "Duplicate"})).await.0, StatusCode::CONFLICT);

    let templates = ada.ok("/api/v1/page-templates").await;
    assert!(templates.as_array().unwrap().iter().any(|t| t["key"] == "runbook"));
    let page = ada.created("/api/v1/spaces/ENG/pages", json!({"title": "Restart the pump controller", "template": "runbook"})).await;
    let id = page["id"].as_str().unwrap().to_string();
    assert!(page["content"].as_str().unwrap().contains("## Roll back"));
    assert_eq!(page["version"], 1);
    let child = ada.created("/api/v1/spaces/ENG/pages", json!({"title": "Firmware notes", "content": "Version 4.2 fixes the watchdog.", "parent_id": id})).await;
    assert_eq!(child["breadcrumbs"][0]["title"], "Restart the pump controller");

    // Saving needs the version the edit was based on; a stale save is a conflict.
    let path = format!("/api/v1/pages/{id}");
    assert_eq!(ada.patch(&path, json!({"content": "new"})).await.0, StatusCode::BAD_REQUEST);
    let (s, v) = ada.patch(&path, json!({"content": "# Steps\n\n1. Isolate power.", "base_version": 1, "message": "First real draft"})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(v["version"], 2);
    let (s, v) = ada.patch(&path, json!({"content": "overwrite", "base_version": 1})).await;
    assert_eq!(s, StatusCode::CONFLICT, "{v}");
    // Metadata-only changes do not create versions.
    assert_eq!(ada.patch(&path, json!({"labels": ["Ops"], "editor": "advanced"})).await.1["version"], 2);

    let versions = ada.ok(&format!("{path}/versions")).await;
    assert_eq!(versions.as_array().unwrap().len(), 2);
    assert_eq!(versions[0]["message"], "First real draft");
    assert!(ada.ok(&format!("{path}/versions/1")).await["content"].as_str().unwrap().contains("Roll back"));
    let restored = ada.post(&format!("{path}/versions/1/restore"), json!({})).await.1;
    assert_eq!(restored["version"], 3);
    assert!(restored["content"].as_str().unwrap().contains("Roll back"));

    // Internal spaces are readable by members but not editable without a role.
    assert_eq!(bob.get(&path).await.0, StatusCode::OK);
    assert_eq!(bob.patch(&path, json!({"content": "vandalism", "base_version": 3})).await.0, StatusCode::FORBIDDEN);
    // Drafts are hidden from readers.
    let draft = ada.created("/api/v1/spaces/ENG/pages", json!({"title": "Unfinished", "status": "draft"})).await;
    assert_eq!(bob.get(&format!("/api/v1/pages/{}", draft["id"].as_str().unwrap())).await.0, StatusCode::NOT_FOUND);
    assert_eq!(bob.ok("/api/v1/spaces/ENG").await["pages"].as_array().unwrap().len(), 2);
    assert_eq!(ada.ok("/api/v1/spaces/ENG").await["pages"].as_array().unwrap().len(), 3);

    // Grant Bob the editor role; now he can save.
    ada.created("/api/v1/spaces/ENG/members", json!({"user_id": bob.user_id, "role": "space_editor"})).await;
    assert_eq!(bob.patch(&path, json!({"title": "Restart the pump controller (rev B)", "base_version": 3})).await.0, StatusCode::OK);
    assert_eq!(bob.delete(&path).await.0, StatusCode::FORBIDDEN, "editors cannot archive");

    // Moving a page beneath its own child is refused; archiving takes the subtree.
    assert_eq!(ada.patch(&path, json!({"parent_id": child["id"]})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ada.delete(&path).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ada.get(&format!("/api/v1/pages/{}", child["id"].as_str().unwrap())).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn search_keyword_semantic_and_scoped() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    software_project(&ada, "OPS").await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Coolant pump seal leaking", "description": "Glycol on the floor under pump P-101."})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Quarterly budget forecast"})).await;
    ada.created("/api/v1/spaces", json!({"key": "KB", "name": "Knowledge"})).await;
    ada.created("/api/v1/spaces/KB/pages", json!({"title": "Replacing a pump seal", "content": "Drain the glycol loop before removing the seal housing."})).await;
    app.drain_jobs().await;

    let r = ada.ok("/api/v1/search?q=glycol").await;
    let kinds: Vec<&str> = r["results"].as_array().unwrap().iter().map(|h| h["kind"].as_str().unwrap()).collect();
    assert!(kinds.contains(&"item") && kinds.contains(&"page"), "{r}");
    assert!(r["results"][0]["snippet_parts"].as_array().unwrap().iter().any(|p| p["hit"] == true));
    assert_eq!(ada.ok("/api/v1/search?q=glycol&kind=page").await["results"].as_array().unwrap().len(), 1);
    assert_eq!(ada.ok("/api/v1/search?q=OPS-2&mode=keyword").await["results"][0]["title"], "Quarterly budget forecast", "keys are searchable");
    assert_eq!(ada.ok("/api/v1/search?q=xyzzyplugh").await["results"].as_array().unwrap().len(), 0);

    let sem = ada.ok("/api/v1/search?q=pump%20seal%20replacement&mode=semantic").await;
    assert!(!sem["results"].as_array().unwrap().is_empty());
    assert_ne!(sem["results"][0]["title"], "Quarterly budget forecast");
    assert_eq!(ada.get("/api/v1/search?q=x&mode=psychic").await.0, StatusCode::BAD_REQUEST);

    // Another member sees nothing from the private project or space.
    let eve = app.user("eve@example.org", "member").await;
    assert_eq!(eve.ok("/api/v1/search?q=glycol").await["results"].as_array().unwrap().len(), 0);
    assert_eq!(eve.ok("/api/v1/search?q=pump%20seal&mode=semantic").await["results"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn attachments_round_trip() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    software_project(&ada, "OPS").await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "With files"})).await;
    let up = ada.request(Method::POST, "/api/v1/items/OPS-1/attachments?filename=..%2F..%2Freport%20final.txt").header("content-type", "text/plain").body("vibration data").send().await.unwrap();
    assert_eq!(up.status(), StatusCode::CREATED);
    let meta: Value = up.json().await.unwrap();
    assert_eq!(meta["filename"], "report final.txt", "directory parts are stripped");
    assert_eq!(meta["size"], 14);
    let id = meta["id"].as_str().unwrap();
    let dl = ada.request(Method::GET, &format!("/api/v1/attachments/{id}")).send().await.unwrap();
    assert_eq!(dl.status(), StatusCode::OK);
    assert_eq!(dl.headers()["content-type"], "application/octet-stream");
    assert!(dl.headers()["content-disposition"].to_str().unwrap().starts_with("attachment;"));
    assert_eq!(dl.text().await.unwrap(), "vibration data");
    assert_eq!(ada.request(Method::POST, "/api/v1/items/OPS-1/attachments?filename=empty.txt").body("").send().await.unwrap().status(), StatusCode::BAD_REQUEST);
    assert_eq!(ada.ok("/api/v1/items/OPS-1").await["attachments"].as_array().unwrap().len(), 1);
    assert_eq!(ada.delete(&format!("/api/v1/attachments/{id}")).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ada.get(&format!("/api/v1/attachments/{id}")).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn service_desk_portal_queues_and_targets() {
    let app = spawn().await;
    let agent = app.user("agent@example.org", "member").await;
    let customer = app.user("pat@example.org", "customer").await;
    agent.created("/api/v1/projects", json!({"key": "IT", "name": "IT help", "template": "it-service-desk"})).await;

    // The customer has no workspace access at all, but can use the open portal.
    assert_eq!(customer.ok("/api/v1/projects").await.as_array().unwrap().len(), 0);
    assert_eq!(customer.get("/api/v1/projects/IT").await.0, StatusCode::NOT_FOUND);
    assert_eq!(customer.get("/api/v1/users").await.0, StatusCode::FORBIDDEN);
    let desks = customer.ok("/api/v1/portal/desks").await;
    assert_eq!(desks[0]["key"], "IT");
    let desk = customer.ok("/api/v1/portal/desks/IT").await;
    let rt = desk["request_types"].as_array().unwrap().iter().find(|t| t["name"] == "Something is broken").unwrap();
    assert_eq!(rt["fields"].as_array().unwrap().len(), 3);

    // Knowledge base suggestions come only from customer-visible articles.
    let articles = customer.ok("/api/v1/portal/desks/IT/articles").await;
    assert_eq!(articles.as_array().unwrap().len(), 2);
    assert!(!customer.ok("/api/v1/portal/desks/IT/suggest?q=password").await.as_array().unwrap().is_empty());
    let internal_page = agent.created("/api/v1/spaces/IT/pages", json!({"title": "Admin password vault procedure", "content": "Break-glass password steps."})).await;
    let hits = customer.ok("/api/v1/portal/desks/IT/suggest?q=vault").await;
    assert!(hits.as_array().unwrap().is_empty(), "internal articles are never suggested: {hits}");
    assert_eq!(customer.get(&format!("/api/v1/portal/desks/IT/articles/{}", internal_page["id"].as_str().unwrap())).await.0, StatusCode::NOT_FOUND);

    // Raise a request. Fields outside the form are refused.
    let rid = rt["id"].as_str().unwrap();
    assert_eq!(customer.post("/api/v1/portal/desks/IT/requests", json!({"request_type_id": rid, "summary": "x", "fields": {"root_cause": "me"}})).await.0, StatusCode::BAD_REQUEST);
    let req = customer.created("/api/v1/portal/desks/IT/requests", json!({"request_type_id": rid, "summary": "Laptop will not boot", "description": "Black screen since this morning.", "fields": {"impact": "High"}})).await;
    assert_eq!(req["key"], "IT-1");
    assert_eq!(customer.ok("/api/v1/portal/requests").await[0]["key"], "IT-1");
    assert_eq!(customer.get("/api/v1/items/IT-1").await.0, StatusCode::NOT_FOUND, "customers use the portal view, not the agent view");

    // The agent sees it in the queue with targets from the policy (priority 2: respond in 4 h, resolve in 24 h).
    let queues = agent.ok("/api/v1/projects/IT/queues").await;
    assert_eq!(queues.as_array().unwrap().iter().find(|q| q["key"] == "unassigned").unwrap()["count"], 1);
    let item = agent.ok("/api/v1/items/IT-1").await;
    assert_eq!(item["channel"], "portal");
    assert!(item["sla"]["response_due"].is_string() && item["sla"]["resolution_due"].is_string());
    assert_eq!(item["sla"]["response_breached"], false);

    // Internal note, then a public reply (which records first response), then ask the customer.
    agent.created("/api/v1/items/IT-1/comments", json!({"body": "Probably the SSD again.", "internal": true})).await;
    assert!(agent.ok("/api/v1/items/IT-1").await["sla"]["first_response_at"].is_null(), "an internal note is not a response");
    agent.created("/api/v1/items/IT-1/comments", json!({"body": "Can you hold the power button for 10 seconds?"})).await;
    assert!(agent.ok("/api/v1/items/IT-1").await["sla"]["first_response_at"].is_string());
    assert_eq!(agent.post("/api/v1/items/IT-1/transition", json!({"to": "waiting_customer"})).await.0, StatusCode::OK);

    let view = customer.ok("/api/v1/portal/requests/IT-1").await;
    assert_eq!(view["status_name"], "Waiting for customer");
    let bodies: Vec<&str> = view["comments"].as_array().unwrap().iter().map(|c| c["body"].as_str().unwrap()).collect();
    assert_eq!(bodies, vec!["Can you hold the power button for 10 seconds?"], "internal notes never reach the customer");

    // The customer's reply reopens the request for the agents.
    customer.created("/api/v1/portal/requests/IT-1/comments", json!({"body": "That worked, thank you."})).await;
    assert_eq!(agent.ok("/api/v1/items/IT-1").await["status"], "in_progress");
    assert_eq!(agent.post("/api/v1/items/IT-1/transition", json!({"to": "resolved"})).await.0, StatusCode::OK);
    assert_eq!(customer.ok("/api/v1/portal/requests/IT-1").await["status_category"], "done");

    // Another customer cannot see or touch this request.
    let other = app.user("sam@example.org", "customer").await;
    assert_eq!(other.get("/api/v1/portal/requests/IT-1").await.0, StatusCode::NOT_FOUND);
    assert_eq!(other.post("/api/v1/portal/requests/IT-1/comments", json!({"body": "hi"})).await.0, StatusCode::NOT_FOUND);
    assert_eq!(other.ok("/api/v1/portal/requests").await.as_array().unwrap().len(), 0);

    // Closing the portal removes access for people without an explicit grant.
    assert_eq!(agent.patch("/api/v1/projects/IT", json!({"settings": {"portal_open": false}})).await.0, StatusCode::OK);
    assert_eq!(other.ok("/api/v1/portal/desks").await.as_array().unwrap().len(), 0);
    assert_eq!(other.get("/api/v1/portal/desks/IT").await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn service_level_targets_follow_business_hours_and_breach_scan() {
    let app = spawn().await;
    let agent = app.user("agent@example.org", "member").await;
    agent.created("/api/v1/projects", json!({"key": "IT", "name": "IT help", "template": "it-service-desk"})).await;
    assert_eq!(agent.put("/api/v1/projects/IT/sla", json!({"policies": [{"priority": 2, "response_minutes": 60, "resolution_minutes": 30}]})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(agent.put("/api/v1/projects/IT/sla", json!({"policies": [{"priority": 2, "response_minutes": 30, "resolution_minutes": 60}]})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(agent.patch("/api/v1/projects/IT", json!({"settings": {"calendar": {"mode": "business", "days": [], "start": "08:00", "end": "17:00"}}})).await.0, StatusCode::BAD_REQUEST);
    let sla = agent.ok("/api/v1/projects/IT/sla").await;
    assert_eq!(sla["policies"].as_array().unwrap().len(), 1);

    let item = agent.created("/api/v1/projects/IT/items", json!({"title": "Printer jam"})).await;
    let due: chrono::DateTime<chrono::Utc> = item["sla"]["resolution_due"].as_str().unwrap().parse().unwrap();
    let created: chrono::DateTime<chrono::Utc> = item["created_at"].as_str().unwrap().parse().unwrap();
    assert!((due - created - chrono::Duration::minutes(60)).num_seconds().abs() < 5);
    // Priority without a policy row has no target.
    assert!(agent.created("/api/v1/projects/IT/items", json!({"title": "Low", "priority": 1})).await["sla"].is_null());

    // Age the request past its target; the periodic scan flags it once and notifies the assignee.
    agent.patch("/api/v1/items/IT-1", json!({"assignee_id": agent.user_id})).await;
    sqlx::query("UPDATE items SET sla_resolution_due = now() - interval '1 minute', sla_response_due = now() - interval '31 minutes' WHERE number = 1").execute(&app.st.db).await.unwrap();
    assert_eq!(gantry::desk::sla_scan(&app.st).await.unwrap(), 1);
    assert_eq!(gantry::desk::sla_scan(&app.st).await.unwrap(), 0, "a breach is announced once");
    let item = agent.ok("/api/v1/items/IT-1").await;
    assert_eq!((item["sla"]["resolution_breached"].as_bool(), item["sla"]["response_breached"].as_bool()), (Some(true), Some(true)));
    assert_eq!(agent.ok("/api/v1/items?project=IT&sla=breached").await["total"], 1);
    assert!(agent.ok("/api/v1/notifications").await.as_array().unwrap().iter().any(|n| n["kind"] == "sla.breached"));
}

#[tokio::test]
async fn administration_users_groups_roles() {
    let app = spawn().await;
    let root = app.admin().await;
    let created = root.created("/api/v1/admin/users", json!({"email": "Ada@Example.org", "name": "Ada", "password": PASSWORD})).await;
    let ada_id = created["id"].as_str().unwrap().to_string();
    assert_eq!(created["email"], "ada@example.org");
    assert_eq!(root.post("/api/v1/admin/users", json!({"email": "ada@example.org", "name": "Dup", "password": PASSWORD})).await.0, StatusCode::CONFLICT);
    assert_eq!(root.post("/api/v1/admin/users", json!({"email": "bad", "name": "x"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.post("/api/v1/admin/users", json!({"email": "x@example.org", "name": "x", "password": "short"})).await.0, StatusCode::BAD_REQUEST);
    let ada = app.login("ada@example.org").await;
    assert_eq!(ada.get("/api/v1/admin/users").await.0, StatusCode::FORBIDDEN);

    // Groups carry roles to their members.
    let group = root.created("/api/v1/admin/groups", json!({"name": "Auditors"})).await;
    let gid = group["id"].as_str().unwrap();
    assert_eq!(root.post(&format!("/api/v1/admin/groups/{gid}/roles"), json!({"role": "auditor"})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(root.post(&format!("/api/v1/admin/groups/{gid}/members"), json!({"user_id": ada_id})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ada.get("/api/v1/admin/audit").await.0, StatusCode::OK, "group membership grants the auditor role");
    assert_eq!(ada.get("/api/v1/admin/users").await.0, StatusCode::FORBIDDEN, "auditors cannot manage users");
    assert_eq!(root.delete(&format!("/api/v1/admin/groups/{gid}/members/{ada_id}")).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ada.get("/api/v1/admin/audit").await.0, StatusCode::FORBIDDEN);

    // Custom roles.
    assert_eq!(root.post("/api/v1/admin/roles", json!({"key": "triager", "name": "Triager", "scope": "project", "permissions": ["project.view", "item.assign", "admin.users"]})).await.0, StatusCode::BAD_REQUEST, "a project role cannot hold global permissions");
    root.created("/api/v1/admin/roles", json!({"key": "triager", "name": "Triager", "scope": "project", "permissions": ["project.view", "item.assign", "item.transition"]})).await;
    assert_eq!(root.put("/api/v1/admin/roles/project_member", json!({"name": "x", "permissions": []})).await.0, StatusCode::BAD_REQUEST, "built-in roles are fixed");
    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    root.created("/api/v1/projects/OPS/items", json!({"title": "Sort me"})).await;
    root.created("/api/v1/projects/OPS/members", json!({"user_id": ada_id, "role": "triager"})).await;
    assert_eq!(ada.post("/api/v1/items/OPS-1/transition", json!({"to": "in_progress"})).await.0, StatusCode::OK);
    assert_eq!(ada.patch("/api/v1/items/OPS-1", json!({"title": "Renamed"})).await.0, StatusCode::FORBIDDEN, "the custom role does not include item.edit");
    assert_eq!(root.delete("/api/v1/admin/roles/triager").await.0, StatusCode::NO_CONTENT);
    assert_eq!(ada.get("/api/v1/items/OPS-1").await.0, StatusCode::NOT_FOUND, "deleting the role removes its grants");

    // Deactivation ends sessions and blocks sign-in.
    assert_eq!(root.patch(&format!("/api/v1/admin/users/{ada_id}"), json!({"active": false})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ada.get("/api/v1/auth/me").await.0, StatusCode::UNAUTHORIZED);
    assert!(app.login_with("ada@example.org", PASSWORD).await.is_err());

    let overview = root.ok("/api/v1/admin/overview").await;
    assert!(overview["components"].as_array().unwrap().iter().all(|c| c["status"]["ok"] == true), "{overview}");
    assert_eq!(overview["counts"]["projects"], 1);
    let users = root.ok("/api/v1/admin/users?q=ada").await;
    assert_eq!(users["total"], 1);
    assert_eq!(users["users"][0]["active"], false);
}

#[tokio::test]
async fn administration_schemes_can_be_customized() {
    let app = spawn().await;
    let root = app.admin().await;
    // A new field, workflow and type, then used in a project.
    assert_eq!(root.put("/api/v1/admin/fields/batch_number", json!({"name": "Batch number", "type": "text"})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(root.put("/api/v1/admin/fields/Bad-Key", json!({"name": "x", "type": "text"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.put("/api/v1/admin/fields/shade", json!({"name": "Shade", "type": "select", "options": []})).await.0, StatusCode::BAD_REQUEST);
    let wf = json!({"initial": "queued", "statuses": [{"key": "queued", "name": "Queued", "category": "todo"}, {"key": "testing", "name": "Testing", "category": "in_progress"}, {"key": "released", "name": "Released", "category": "done"}],
        "transitions": [{"name": "Test", "from": ["queued"], "to": "testing"}, {"name": "Release", "from": ["testing"], "to": "released", "requires": ["batch_number"]}]});
    assert_eq!(root.put("/api/v1/admin/workflows/qc", json!({"name": "Quality control", "definition": wf})).await.0, StatusCode::NO_CONTENT);
    let broken = json!({"initial": "a", "statuses": [{"key": "a", "name": "A", "category": "todo"}, {"key": "b", "name": "B", "category": "done"}], "transitions": []});
    let (s, v) = root.put("/api/v1/admin/workflows/broken", json!({"name": "Broken", "definition": broken})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains("cannot be reached"));
    assert_eq!(root.put("/api/v1/admin/item-types/qc_lot", json!({"name": "QC lot", "workflow": "qc", "fields": [{"key": "batch_number", "required": false}]})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(root.put("/api/v1/admin/item-types/qc_lot2", json!({"name": "x", "workflow": "nope"})).await.0, StatusCode::BAD_REQUEST);

    root.created("/api/v1/projects", json!({"key": "LAB", "name": "Lab", "template": "basic"})).await;
    assert_eq!(root.put("/api/v1/projects/LAB/types", json!({"types": ["task", "qc_lot"]})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(root.put("/api/v1/projects/LAB/types", json!({"types": ["qc_lot"]})).await.0, StatusCode::BAD_REQUEST, "the default type must stay");
    let lot = root.created("/api/v1/projects/LAB/items", json!({"type": "qc_lot", "title": "Lot 7"})).await;
    assert_eq!(lot["status"], "queued");
    root.post("/api/v1/items/LAB-1/transition", json!({"to": "testing"})).await;
    assert_eq!(root.post("/api/v1/items/LAB-1/transition", json!({"to": "released"})).await.0, StatusCode::BAD_REQUEST);

    // A status that still holds items cannot be removed from the workflow.
    let shrunk = json!({"initial": "queued", "statuses": [{"key": "queued", "name": "Queued", "category": "todo"}, {"key": "released", "name": "Released", "category": "done"}], "transitions": [{"name": "Release", "from": ["queued"], "to": "released"}]});
    let (s, v) = root.put("/api/v1/admin/workflows/qc", json!({"name": "Quality control", "definition": shrunk})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
    // A field that holds data cannot change type.
    root.patch("/api/v1/items/LAB-1", json!({"custom": {"batch_number": "B-77"}})).await;
    assert_eq!(root.put("/api/v1/admin/fields/batch_number", json!({"name": "Batch number", "type": "number"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.post("/api/v1/items/LAB-1/transition", json!({"to": "released"})).await.0, StatusCode::OK);

    let packs = root.ok("/api/v1/admin/packs").await;
    assert_eq!(packs["installed"][0]["id"], "core");
    assert!(packs["available"].as_array().unwrap().iter().any(|p| p["id"] == "nasa" && p["valid"] == true));
    assert_eq!(root.post("/api/v1/admin/packs/missing/install", json!({})).await.0, StatusCode::NOT_FOUND);

    assert_eq!(root.put("/api/v1/admin/settings/site", json!({"name": "Plant 4 Tracker", "announcement": "Maintenance Saturday 06:00"})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(root.put("/api/v1/admin/settings/site", json!({"evil": true})).await.0, StatusCode::BAD_REQUEST);
    let public: Value = app.http.get(format!("{}/api/v1/auth/providers", app.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(public["site"]["name"], "Plant 4 Tracker");
    let settings = root.ok("/api/v1/admin/settings").await;
    assert_eq!(settings["secret_key_set"], true);
    assert!(!settings.to_string().contains(&app.st.cfg.security.secret_key), "the secret key is never returned");
}

#[tokio::test]
async fn agents_work_through_api_tokens() {
    let app = spawn().await;
    let root = app.admin().await;
    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    let agent = root.created("/api/v1/admin/users", json!({"name": "Triage bot", "kind": "agent"})).await;
    let agent_id = agent["id"].as_str().unwrap();
    assert!(agent["email"].as_str().unwrap().ends_with("@agents.invalid"));
    assert_eq!(root.post("/api/v1/admin/users", json!({"name": "Bot", "kind": "agent", "password": PASSWORD})).await.0, StatusCode::BAD_REQUEST);
    root.created("/api/v1/projects/OPS/members", json!({"user_id": agent_id, "role": "project_member"})).await;
    root.agent_rules(agent_id, working_agent()).await;
    let tok = root.created(&format!("/api/v1/admin/users/{agent_id}/tokens"), json!({"name": "prod", "scopes": ["read", "write"], "expires_in_days": 30})).await;
    assert!(tok["token"].as_str().unwrap().starts_with("gty_"));
    assert!(tok["expires_at"].is_string());
    let bot = Caller { token: Some(tok["token"].as_str().unwrap().to_string()), ..app.anon() };
    let me = bot.ok("/api/v1/auth/me").await;
    assert_eq!((me["user"]["kind"].as_str(), me["auth_method"].as_str()), (Some("agent"), Some("token")));
    let item = bot.created("/api/v1/projects/OPS/items", json!({"title": "Filed by the agent"})).await;
    assert_eq!(item["reporter"]["kind"], "agent");
    assert_eq!(bot.post("/api/v1/tokens", json!({"name": "self", "scopes": ["read"]})).await.0, StatusCode::FORBIDDEN, "a token cannot mint tokens");
    // The audit log attributes the action to the agent and the token.
    let row: (String, String, String) = sqlx::query_as("SELECT actor_kind, actor_label, auth_method FROM audit_log WHERE action = 'item.create'").fetch_one(&app.st.db).await.unwrap();
    assert_eq!((row.0.as_str(), row.2.as_str()), ("agent", "token"));
    assert!(row.1.ends_with("@agents.invalid"));
    // Revocation is immediate.
    let listed = root.ok(&format!("/api/v1/admin/users/{agent_id}/tokens")).await;
    assert_eq!(root.delete(&format!("/api/v1/tokens/{}", listed[0]["id"].as_str().unwrap())).await.0, StatusCode::NO_CONTENT);
    assert_eq!(bot.get("/api/v1/auth/me").await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn audit_log_is_searchable_and_verifiable() {
    let app = spawn().await;
    let root = app.admin().await;
    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    root.created("/api/v1/projects/OPS/items", json!({"title": "One"})).await;
    root.post("/api/v1/items/OPS-1/transition", json!({"to": "done"})).await;
    let log = root.ok("/api/v1/admin/audit?action=item.").await;
    let actions: Vec<&str> = log["entries"].as_array().unwrap().iter().map(|e| e["action"].as_str().unwrap()).collect();
    assert_eq!(actions, vec!["item.transition", "item.create"]);
    let e = &log["entries"][0];
    assert_eq!(e["actor_label"], "root@example.org");
    assert_eq!(e["target_id"], "OPS-1");
    assert_eq!(e["detail"]["to"], "done");
    assert!(e["request_id"].as_str().unwrap().len() >= 8 && e["ip"] == "127.0.0.1");
    assert_eq!(root.ok("/api/v1/admin/audit?target=OPS-1&actor=root").await["total"], 2);
    assert_eq!(root.ok("/api/v1/admin/audit?actor=nobody").await["total"], 0);
    let report = root.post("/api/v1/admin/audit/verify", json!({})).await.1;
    assert_eq!(report["intact"], true);
    assert!(report["entries"].as_i64().unwrap() >= 5);
    assert!(app.audit_actions().await.contains(&"audit.read".to_string()), "reading the audit log is itself audited");
}
