//! Regression tests: behaviors that are easy to break without noticing — concurrency,
//! idempotent upgrades, index consistency, queue recovery, and the optional storage engines.

mod common;
use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode as AxStatus};
use axum::routing::{get, post, put};
use bytes::Bytes;
use common::*;
use gantry::storage::{BlobStore, Kv, SearchDoc, SearchIndex, SearchScope};
use reqwest::StatusCode;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

#[tokio::test]
async fn concurrent_creates_get_unique_consecutive_numbers() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    let mut handles = vec![];
    for i in 0..24 {
        let c = ada.clone();
        handles.push(tokio::spawn(async move { c.created("/api/v1/projects/OPS/items", json!({"title": format!("Parallel {i}")})).await["number"].as_i64().unwrap() }));
    }
    let mut numbers = vec![];
    for h in handles {
        numbers.push(h.await.unwrap());
    }
    numbers.sort();
    assert_eq!(numbers, (1..=24).collect::<Vec<i64>>(), "no gaps, no duplicates");
    // Numbers are never reused after an archive.
    ada.delete("/api/v1/items/OPS-24").await;
    assert_eq!(ada.created("/api/v1/projects/OPS/items", json!({"title": "Next"})).await["key"], "OPS-25");
    // The audit hash chain stays intact when entries are written concurrently.
    let report = gantry::audit::verify_chain(&app.st.db).await.unwrap();
    assert!(report.intact && report.entries >= 27, "{report:?}");
}

#[tokio::test]
async fn concurrent_page_saves_have_exactly_one_winner() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/spaces", json!({"key": "DOC", "name": "Docs"})).await;
    let page = ada.created("/api/v1/spaces/DOC/pages", json!({"title": "Contested"})).await;
    let path = format!("/api/v1/pages/{}", page["id"].as_str().unwrap());
    let mut handles = vec![];
    for i in 0..8 {
        let (c, p) = (ada.clone(), path.clone());
        handles.push(tokio::spawn(async move { c.patch(&p, json!({"content": format!("writer {i}"), "base_version": 1})).await.0 }));
    }
    let mut statuses = vec![];
    for h in handles {
        statuses.push(h.await.unwrap());
    }
    assert_eq!(statuses.iter().filter(|s| **s == StatusCode::OK).count(), 1, "{statuses:?}");
    assert_eq!(statuses.iter().filter(|s| **s == StatusCode::CONFLICT).count(), 7);
    assert_eq!(ada.ok(&path).await["version"], 2);
    assert_eq!(ada.ok(&format!("{path}/versions")).await.as_array().unwrap().len(), 2, "losers left no version behind");
}

#[tokio::test]
async fn upgrades_are_idempotent_and_keep_customizations() {
    let app = spawn().await;
    let root = app.admin().await;
    let counts = || async {
        sqlx::query_as::<_, (i64, i64, i64, i64, i64)>("SELECT (SELECT count(*) FROM roles), (SELECT count(*) FROM fields), (SELECT count(*) FROM workflows), (SELECT count(*) FROM item_types), (SELECT count(*) FROM project_templates)").fetch_one(&app.st.db).await.unwrap()
    };
    let before = counts().await;
    // An administrator renames a status in a built-in workflow and adds a custom role and field.
    let mut simple = root.ok("/api/v1/admin/schemes").await["workflows"].as_array().unwrap().iter().find(|w| w["key"] == "simple").unwrap()["definition"].clone();
    simple["statuses"][0]["name"] = json!("Not started");
    assert_eq!(root.put("/api/v1/admin/workflows/simple", json!({"name": "Simple (ours)", "definition": simple})).await.0, StatusCode::NO_CONTENT);
    root.created("/api/v1/admin/roles", json!({"key": "reviewer", "name": "Reviewer", "scope": "project", "permissions": ["project.view", "item.comment"]})).await;
    root.put("/api/v1/admin/fields/site_code", json!({"name": "Site code", "type": "text"})).await;
    // Simulate an untouched built-in drifting (as after a package upgrade ships a new definition).
    sqlx::query("UPDATE workflows SET name = 'stale name' WHERE key = 'software'").execute(&app.st.db).await.unwrap();

    for _ in 0..2 {
        gantry::migrate(&app.st.db).await.unwrap();
    }
    let after = counts().await;
    assert_eq!((after.0, after.1, after.2, after.3, after.4), (before.0 + 1, before.1 + 1, before.2, before.3, before.4));
    let wf: Vec<(String, String, String)> = sqlx::query_as("SELECT key, name, source FROM workflows WHERE key IN ('simple', 'software') ORDER BY key").fetch_all(&app.st.db).await.unwrap();
    assert_eq!(wf[0], ("simple".into(), "Simple (ours)".into(), "custom".into()), "the customized workflow survives the upgrade");
    assert_eq!(wf[1], ("software".into(), "Software delivery".into(), "core".into()), "untouched built-ins are refreshed");
    assert!(gantry::schema_is_current(&app.st.db).await);
    // Add-on packs reinstall cleanly too.
    for _ in 0..2 {
        assert_eq!(root.post("/api/v1/admin/packs/nasa/install", json!({})).await.0, StatusCode::OK);
    }
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM packs").fetch_one(&app.st.db).await.unwrap(), 2);
}

#[tokio::test]
async fn workflow_edits_keep_item_categories_in_step() {
    let app = spawn().await;
    let root = app.admin().await;
    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "software"})).await;
    root.created("/api/v1/projects/OPS/items", json!({"type": "story", "title": "In review"})).await;
    assert_eq!(root.post("/api/v1/items/OPS-1/transition", json!({"to": "in_review"})).await.0, StatusCode::OK);
    assert_eq!(root.ok("/api/v1/items/OPS-1").await["status_category"], "in_progress");
    // The team decides "In review" counts as done.
    let mut def = root.ok("/api/v1/admin/schemes").await["workflows"].as_array().unwrap().iter().find(|w| w["key"] == "software").unwrap()["definition"].clone();
    def["statuses"][3]["category"] = json!("done");
    assert_eq!(def["statuses"][3]["key"], "in_review");
    assert_eq!(root.put("/api/v1/admin/workflows/software", json!({"name": "Software delivery", "definition": def})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(root.ok("/api/v1/items/OPS-1").await["status_category"], "done");
    assert_eq!(root.ok("/api/v1/items?project=OPS&category=done").await["total"], 1, "boards and filters follow the new category");
}

#[tokio::test]
async fn search_index_follows_every_change() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    ada.created("/api/v1/spaces", json!({"key": "DOC", "name": "Docs"})).await;
    let found = |q: &'static str| {
        let c = ada.clone();
        async move { c.ok(&format!("/api/v1/search?q={q}&mode=keyword")).await["results"].as_array().unwrap().len() }
    };
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Zirconium flange"})).await;
    assert_eq!(found("zirconium").await, 1);
    ada.patch("/api/v1/items/OPS-1", json!({"title": "Titanium flange"})).await;
    assert_eq!((found("zirconium").await, found("titanium").await), (0, 1), "renames reindex");
    ada.created("/api/v1/items/OPS-1/comments", json!({"body": "Supplier quotes molybdenum instead"})).await;
    assert_eq!(found("molybdenum").await, 1, "comments are searchable");
    ada.delete("/api/v1/items/OPS-1").await;
    assert_eq!(found("titanium").await, 0, "archived items leave the index");

    let page = ada.created("/api/v1/spaces/DOC/pages", json!({"title": "Beryllium handling", "status": "draft"})).await;
    let path = format!("/api/v1/pages/{}", page["id"].as_str().unwrap());
    assert_eq!(found("beryllium").await, 0, "drafts are not searchable");
    ada.patch(&path, json!({"status": "published"})).await;
    assert_eq!(found("beryllium").await, 1);
    ada.patch(&path, json!({"status": "draft"})).await;
    assert_eq!(found("beryllium").await, 0, "unpublishing removes the page from search");
    ada.patch(&path, json!({"status": "published"})).await;
    // A wiped index is rebuilt by `gantry reindex`.
    sqlx::query("DELETE FROM search_docs").execute(&app.st.db).await.unwrap();
    assert_eq!(found("beryllium").await, 0);
    let (items, pages) = gantry::search::reindex_all(&app.st).await.unwrap();
    assert_eq!((items, pages), (1, 1));
    assert_eq!((found("beryllium").await, found("titanium").await), (1, 0), "reindexing does not resurrect archived items");
}

#[tokio::test]
async fn internal_notes_are_not_indexed_for_search() {
    let app = spawn().await;
    let agent = app.user("agent@example.org", "member").await;
    agent.created("/api/v1/projects", json!({"key": "IT", "name": "IT", "template": "it-service-desk", "access": "internal"})).await;
    agent.created("/api/v1/projects/IT/items", json!({"title": "Laptop"})).await;
    agent.created("/api/v1/items/IT-1/comments", json!({"body": "suspect xylophonic malware", "internal": true})).await;
    // A colleague with only internal visibility can find the item but not through the note's text.
    let peer = app.user("peer@example.org", "member").await;
    assert_eq!(peer.ok("/api/v1/search?q=laptop&mode=keyword").await["results"].as_array().unwrap().len(), 1);
    assert_eq!(peer.ok("/api/v1/search?q=xylophonic&mode=keyword").await["results"].as_array().unwrap().len(), 0);
    assert_eq!(peer.ok("/api/v1/items/IT-1").await["comments"].as_array().unwrap().len(), 0);
}

#[tokio::test]
async fn job_queue_recovers_from_dead_workers_and_cleans_up() {
    let app = spawn().await;
    // A job claimed long ago by a worker that died.
    sqlx::query("INSERT INTO jobs (kind, payload, state, locked_by, locked_at, attempts) VALUES ('embed', '{\"kind\":\"item\",\"id\":\"00000000-0000-0000-0000-000000000001\"}', 'running', 'dead-node', now() - interval '1 hour', 1)").execute(&app.st.db).await.unwrap();
    // A job claimed just now by a live worker must be left alone.
    sqlx::query("INSERT INTO jobs (kind, payload, state, locked_by, locked_at) VALUES ('embed', '{}', 'running', 'live-node', now())").execute(&app.st.db).await.unwrap();
    sqlx::query("INSERT INTO jobs (kind, state, finished_at) VALUES ('embed', 'done', now() - interval '8 days'), ('embed', 'done', now())").execute(&app.st.db).await.unwrap();
    sqlx::query("INSERT INTO kv (key, value, expires_at) VALUES ('old', '1', now() - interval '1 minute')").execute(&app.st.db).await.unwrap();
    gantry::jobs::periodic(&app.st).await.unwrap();
    let states: Vec<(String, Option<String>)> = sqlx::query_as("SELECT state, locked_by FROM jobs ORDER BY id").fetch_all(&app.st.db).await.unwrap();
    assert_eq!(states, vec![("queued".into(), Some("dead-node".into())), ("running".into(), Some("live-node".into())), ("done".into(), None)], "rescued, untouched, and the old finished job purged");
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM kv WHERE key = 'old'").fetch_one(&app.st.db).await.unwrap(), 0);
    // Two workers never run the same job: claim concurrently and count.
    sqlx::query("DELETE FROM jobs").execute(&app.st.db).await.unwrap();
    for _ in 0..30 {
        gantry::jobs::enqueue(&app.st, "embed", json!({"kind": "item", "id": "00000000-0000-0000-0000-000000000002"})).await.unwrap();
    }
    let mut handles = vec![];
    for _ in 0..6 {
        let st = app.st.clone();
        handles.push(tokio::spawn(async move { gantry::jobs::drain(&st).await.unwrap() }));
    }
    let mut total = 0;
    for h in handles {
        total += h.await.unwrap();
    }
    assert_eq!(total, 30, "each job ran exactly once across six concurrent workers");
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM jobs WHERE state = 'done' AND attempts = 1").fetch_one(&app.st.db).await.unwrap(), 30);
    // An unknown job kind fails cleanly instead of poisoning the queue.
    sqlx::query("INSERT INTO jobs (kind, max_attempts) VALUES ('mystery', 1)").execute(&app.st.db).await.unwrap();
    app.drain_jobs().await;
    assert_eq!(sqlx::query_scalar::<_, String>("SELECT state FROM jobs WHERE kind = 'mystery'").fetch_one(&app.st.db).await.unwrap(), "failed");
}

#[tokio::test]
async fn counters_expire_and_expired_tokens_stop_working() {
    let app = spawn().await;
    let kv = &app.st.stores.kv;
    assert_eq!(kv.incr("c", 1).await.unwrap(), 1);
    assert_eq!(kv.incr("c", 1).await.unwrap(), 2);
    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    assert_eq!(kv.incr("c", 1).await.unwrap(), 1, "the window restarts after it expires");
    kv.set("k", "v", 60).await.unwrap();
    assert_eq!(kv.get("k").await.unwrap().as_deref(), Some("v"));
    kv.del("k").await.unwrap();
    assert_eq!(kv.get("k").await.unwrap(), None);

    let ada = app.user("ada@example.org", "member").await;
    let tok = ada.with_token(&["read"]).await;
    assert_eq!(tok.get("/api/v1/auth/me").await.0, StatusCode::OK);
    assert!(ada.ok("/api/v1/tokens").await[0]["last_used_at"].is_string(), "use is recorded");
    sqlx::query("UPDATE api_tokens SET expires_at = now() - interval '1 second'").execute(&app.st.db).await.unwrap();
    assert_eq!(tok.get("/api/v1/auth/me").await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(ada.post("/api/v1/tokens", json!({"name": "x", "scopes": ["read"], "expires_in_days": 0})).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn reopening_and_responses_keep_service_clocks_honest() {
    let app = spawn().await;
    let agent = app.user("agent@example.org", "member").await;
    let customer = app.user("pat@example.org", "customer").await;
    agent.created("/api/v1/projects", json!({"key": "IT", "name": "IT", "template": "it-service-desk"})).await;
    let rt = customer.ok("/api/v1/portal/desks/IT").await["request_types"][0]["id"].as_str().unwrap().to_string();
    customer.created("/api/v1/portal/desks/IT/requests", json!({"request_type_id": rt, "summary": "Help"})).await;
    // The customer's own follow-up is not a first response.
    customer.created("/api/v1/portal/requests/IT-1/comments", json!({"body": "Any news?"})).await;
    assert!(agent.ok("/api/v1/items/IT-1").await["sla"]["first_response_at"].is_null());
    agent.created("/api/v1/items/IT-1/comments", json!({"body": "Looking now."})).await;
    let first = agent.ok("/api/v1/items/IT-1").await["sla"]["first_response_at"].as_str().unwrap().to_string();
    agent.created("/api/v1/items/IT-1/comments", json!({"body": "Still looking."})).await;
    assert_eq!(agent.ok("/api/v1/items/IT-1").await["sla"]["first_response_at"], first.as_str(), "only the first response counts");
    agent.post("/api/v1/items/IT-1/transition", json!({"to": "resolved"})).await;
    assert!(agent.ok("/api/v1/items/IT-1").await["resolved_at"].is_string());
    agent.post("/api/v1/items/IT-1/transition", json!({"to": "open"})).await;
    assert!(agent.ok("/api/v1/items/IT-1").await["resolved_at"].is_null(), "reopening clears the resolution time");
    // Raising priority recomputes the targets from the original creation time.
    let before = agent.ok("/api/v1/items/IT-1").await["sla"]["resolution_due"].as_str().unwrap().to_string();
    agent.patch("/api/v1/items/IT-1", json!({"priority": 4})).await;
    let after = agent.ok("/api/v1/items/IT-1").await["sla"]["resolution_due"].as_str().unwrap().to_string();
    assert!(after < before, "critical requests are due sooner: {after} vs {before}");
}

#[tokio::test]
async fn unicode_and_boundary_values_round_trip() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "I18N", "name": "Международный проект", "template": "basic"})).await;
    let title = "Überprüfung der Dichtung — 密封检查 — فحص الختم — 🔧";
    let item = ada.created("/api/v1/projects/I18N/items", json!({"title": title, "description": "ligne 1\nligne 2\ttab", "labels": ["ÉTIQUETTE"]})).await;
    assert_eq!(item["title"], title);
    assert_eq!(item["labels"], json!(["étiquette"]));
    assert_eq!(ada.ok("/api/v1/items?q=%E5%AF%86%E5%B0%81").await["total"], 1, "substring search works for non-Latin scripts");
    assert_eq!(ada.created("/api/v1/projects/I18N/items", json!({"title": "é".repeat(500)})).await["title"].as_str().unwrap().chars().count(), 500, "limits count characters, not bytes");
    assert_eq!(ada.post("/api/v1/projects/I18N/items", json!({"title": "é".repeat(501)})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ada.ok("/api/v1/items?limit=100000&offset=-5").await["limit"], 500, "paging is clamped");
    for key in ["A", "toolongkeyname", "1AB", "A B"] {
        assert_eq!(ada.post("/api/v1/projects", json!({"key": key, "name": "x", "template": "basic"})).await.0, StatusCode::BAD_REQUEST, "{key}");
    }
    assert_eq!(ada.post("/api/v1/projects", json!({"key": "i18n", "name": "dup", "template": "basic"})).await.0, StatusCode::CONFLICT, "keys are case-insensitive");
}

/// The audit log page once failed with "invalid type: string "50", expected i64": list
/// endpoints that embed paging rejected the very parameters the web client sends.
#[tokio::test]
async fn every_list_endpoint_accepts_paging_parameters() {
    let app = spawn().await;
    let admin = app.admin().await;
    admin.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    admin.created("/api/v1/projects/OPS/items", json!({"title": "One"})).await;
    let mut checked = 0;
    for route in gantry::http::all_routes().docs {
        // Sign-in redirects and the SCIM dialect (startIndex/count) have their own tests.
        if route.method != "GET" || route.path.contains("/auth/oidc/") || route.path.starts_with("/scim/") || route.path == "/mcp" {
            continue;
        }
        let path = route.path.replace("{key}", "OPS").replace("{id}", "00000000-0000-0000-0000-000000000001").replace("{version}", "1").replace("{user}", "00000000-0000-0000-0000-000000000001");
        if path.contains('{') {
            continue;
        }
        let (status, body) = admin.get(&format!("{path}?limit=5&offset=0&q=one")).await;
        // A 400 for a reason of the endpoint's own ("not a service desk") is fine; one that
        // says the query string could not be read is the defect.
        let unreadable = status == StatusCode::BAD_REQUEST && body["detail"].as_str().is_some_and(|d| d.contains("deserialize") || d.contains("invalid type"));
        assert!(!unreadable && !status.is_server_error(), "GET {path}?limit=5&offset=0 answered {status}: {body}");
        checked += 1;
    }
    assert!(checked > 30, "only {checked} routes were checked");
    for path in ["/api/v1/admin/audit", "/api/v1/admin/users", "/api/v1/admin/security-events"] {
        let (status, _) = admin.get(&format!("{path}?limit=2&offset=1")).await;
        assert_eq!(status, StatusCode::OK, "{path}");
    }
    assert_eq!(admin.ok("/api/v1/admin/audit?limit=1").await["entries"].as_array().unwrap().len(), 1);
    // A parameter that cannot be read is a 400 in the same problem format as every other error.
    let resp = admin.request(reqwest::Method::GET, "/api/v1/admin/audit?limit=ten").send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(resp.headers()["content-type"], "application/problem+json");
    let problem: Value = resp.json().await.unwrap();
    assert_eq!(problem["status"], 400);
    assert!(problem["detail"].as_str().unwrap().contains("whole number"), "{problem}");
    let resp = admin.request(reqwest::Method::POST, "/api/v1/projects").header("content-type", "application/json").body("{not json").send().await.unwrap();
    assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    assert_eq!(resp.headers()["content-type"], "application/problem+json");
}

// ───────────────────────── optional storage engines ─────────────────────────

type Objects = Arc<Mutex<HashMap<String, Vec<u8>>>>;

/// A stand-in S3 endpoint that recomputes the SigV4 signature of every request and rejects mismatches.
async fn s3_handler(State(objects): State<Objects>, method: axum::http::Method, uri: axum::http::Uri, headers: HeaderMap, body: Bytes) -> (AxStatus, Vec<u8>) {
    let h = |n: &str| headers.get(n).and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let signed = vec![("host".to_string(), h("host")), ("x-amz-content-sha256".to_string(), h("x-amz-content-sha256")), ("x-amz-date".to_string(), h("x-amz-date"))];
    let expect = gantry::storage::sigv4_authorization(method.as_str(), uri.path(), "", &signed, &h("x-amz-content-sha256"), "us-east-1", "test-access", "test-secret");
    if h("authorization") != expect || h("x-amz-content-sha256") != gantry::crypto::sha256_hex(&body) {
        return (AxStatus::FORBIDDEN, b"SignatureDoesNotMatch".to_vec());
    }
    let mut o = objects.lock().await;
    match method.as_str() {
        "PUT" => {
            o.insert(uri.path().to_string(), body.to_vec());
            (AxStatus::OK, vec![])
        }
        "GET" => o.get(uri.path()).map(|b| (AxStatus::OK, b.clone())).unwrap_or((AxStatus::NOT_FOUND, vec![])),
        "DELETE" => {
            o.remove(uri.path());
            (AxStatus::NO_CONTENT, vec![])
        }
        "HEAD" => (AxStatus::OK, vec![]),
        _ => (AxStatus::METHOD_NOT_ALLOWED, vec![]),
    }
}

#[tokio::test]
async fn s3_compatible_object_storage_backend() {
    let objects: Objects = Default::default();
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", l.local_addr().unwrap());
    let mock = axum::Router::new().fallback(s3_handler).with_state(objects.clone());
    tokio::spawn(async move { axum::serve(l, mock).await.unwrap() });

    let app = spawn_with(|c| {
        c.storage.blob.backend = "s3".into();
        c.storage.blob.endpoint = endpoint.clone();
        c.storage.blob.bucket = "gantry-files".into();
        c.storage.blob.access_key = "test-access".into();
        c.storage.blob.secret_key = "test-secret".into();
    })
    .await;
    assert_eq!(app.st.stores.blob.name(), "s3");
    app.st.stores.blob.health().await.unwrap();
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Files"})).await;
    let up: Value = ada.request(reqwest::Method::POST, "/api/v1/items/OPS-1/attachments?filename=a.txt").body("object storage works").send().await.unwrap().json().await.unwrap();
    let key = format!("/gantry-files/{}", up["sha256"].as_str().unwrap());
    assert_eq!(objects.lock().await.get(&key).map(|b| b.as_slice()), Some(&b"object storage works"[..]), "stored path-style under its content hash");
    let dl = ada.request(reqwest::Method::GET, &format!("/api/v1/attachments/{}", up["id"].as_str().unwrap())).send().await.unwrap();
    assert_eq!(dl.text().await.unwrap(), "object storage works");
    assert_eq!(app.st.stores.blob.get(&"0".repeat(64)).await.unwrap(), None);
    app.st.stores.blob.delete(up["sha256"].as_str().unwrap()).await.unwrap();
    assert!(objects.lock().await.is_empty());
    // Wrong credentials are reported, not swallowed.
    let mut cfg = app.st.cfg.storage.blob.clone();
    cfg.secret_key = "wrong".into();
    let bad = gantry::storage::S3Blob::new(&cfg, app.st.http.clone()).unwrap();
    assert!(bad.put(&"a".repeat(64), Bytes::from_static(b"x"), "text/plain").await.unwrap_err().to_string().contains("403"));
}

#[tokio::test]
async fn opensearch_backend_sends_scoped_queries_and_parses_hits() {
    let seen: Arc<Mutex<Vec<(String, Value)>>> = Default::default();
    let s2 = seen.clone();
    let record = move |path: String, body: Value| {
        let s = s2.clone();
        async move { s.lock().await.push((path, body)) }
    };
    let (r1, r2) = (record.clone(), record.clone());
    let mock = axum::Router::new()
        .route("/_cluster/health", get(|| async { axum::Json(json!({"status": "green"})) }))
        .route("/gantry/_doc/{id}", put(move |Path(id): Path<String>, axum::Json(b): axum::Json<Value>| { let r = r1.clone(); async move { r(format!("doc/{id}"), b).await; axum::Json(json!({"result": "created"})) } }).delete(|| async { axum::Json(json!({"result": "deleted"})) }))
        .route("/gantry/_search", post(move |headers: HeaderMap, axum::Json(b): axum::Json<Value>| { let r = r2.clone(); async move {
            r(format!("search auth={}", headers.get("authorization").map(|v| v.to_str().unwrap().starts_with("Basic ")).unwrap_or(false)), b).await;
            axum::Json(json!({"hits": {"hits": [{"_score": 3.5, "_source": {"kind": "item", "ref_id": "00000000-0000-0000-0000-0000000000aa", "label": "OPS-1", "url": "/i/OPS-1", "title": "Pump seal"}, "highlight": {"body": ["the \u{e000}seal\u{e001} leaks"]}}]}}))
        } }));
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", l.local_addr().unwrap());
    tokio::spawn(async move { axum::serve(l, mock).await.unwrap() });

    let cfg = gantry::config::Search { backend: "opensearch".into(), url, index: "gantry".into(), username: "gantry".into(), password: "pw".into() };
    let os = gantry::storage::OpenSearch::new(&cfg, reqwest::Client::new()).unwrap();
    os.health().await.unwrap();
    let project = uuid::Uuid::new_v4();
    let doc = SearchDoc { kind: "item".into(), ref_id: uuid::Uuid::from_u128(0xaa), project_id: Some(project), label: "OPS-1".into(), title: "Pump seal".into(), body: "the seal leaks".into(), ..Default::default() };
    os.index(&doc).await.unwrap();
    let hits = os.search("seal", &SearchScope { projects: vec![project], ..Default::default() }, 10).await.unwrap();
    assert_eq!((hits.len(), hits[0].label.as_str(), hits[0].score), (1, "OPS-1", 3.5));
    assert_eq!(gantry::search::hit_json(&hits[0])["snippet_parts"][1], json!({"text": "seal", "hit": true}));
    os.remove("item", doc.ref_id).await.unwrap();
    let calls = seen.lock().await;
    assert_eq!(calls[0].0, format!("doc/item:{}", doc.ref_id));
    assert_eq!(calls[0].1["project_id"], json!(project));
    assert_eq!(calls[1].0, "search auth=true");
    assert_eq!(calls[1].1["query"]["bool"]["filter"][0]["bool"]["should"][0]["terms"]["project_id"][0], json!(project), "the permission scope is part of the query");
    assert!(os.search("   ", &SearchScope::default(), 10).await.unwrap().is_empty());
}

#[tokio::test]
async fn valkey_backend_against_a_live_server() {
    // Runs when a Valkey or Redis server binary is installed; otherwise reports that it was skipped.
    let Some(bin) = ["valkey-server", "redis-server"].into_iter().find(|b| std::process::Command::new(b).arg("--version").output().is_ok()) else {
        eprintln!("SKIPPED: no valkey-server or redis-server binary on this machine");
        return;
    };
    let port = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap().local_addr().unwrap().port();
    let mut child = std::process::Command::new(bin).args(["--port", &port.to_string(), "--bind", "127.0.0.1", "--save", "", "--appendonly", "no", "--requirepass", "kv-pass"]).stdout(std::process::Stdio::null()).spawn().unwrap();
    let kv = gantry::storage::ValkeyKv::new(&format!("valkey://:kv-pass@127.0.0.1:{port}/1")).unwrap();
    let mut ready = false;
    for _ in 0..50 {
        if kv.set("probe", "1", 5).await.is_ok() {
            ready = true;
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    let result: anyhow::Result<()> = async {
        anyhow::ensure!(ready, "server did not start");
        assert_eq!(kv.incr("n", 1).await?, 1);
        assert_eq!(kv.incr("n", 1).await?, 2);
        tokio::time::sleep(std::time::Duration::from_millis(1300)).await;
        assert_eq!(kv.incr("n", 1).await?, 1, "the counter expired");
        kv.set("k", "value with spaces\r\nand a line break", 60).await?;
        assert_eq!(kv.get("k").await?.as_deref(), Some("value with spaces\r\nand a line break"));
        kv.del("k").await?;
        assert_eq!(kv.get("k").await?, None);
        let wrong = gantry::storage::ValkeyKv::new(&format!("valkey://:nope@127.0.0.1:{port}"))?;
        assert!(wrong.get("k").await.is_err(), "a wrong password is an error");
        Ok(())
    }
    .await;
    let _ = child.kill();
    let _ = child.wait();
    result.unwrap();
    // The whole application works with it as the shared counter store.
    let _ = port;
}

#[tokio::test]
async fn pgvector_backend_when_the_extension_is_installed() {
    let app = spawn().await;
    let available: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'vector')").fetch_one(&app.st.db).await.unwrap();
    if !available {
        eprintln!("SKIPPED: the pgvector extension is not installed on the test PostgreSQL server");
        // Asking for it without the extension must fail clearly at start-up, not at query time.
        let err = gantry::storage::PgVector::connect(app.st.db.clone(), 256).await.err().expect("must fail").to_string();
        assert!(err.contains("pgvector extension is not available"), "{err}");
        return;
    }
    use gantry::storage::{VecMeta, VectorStore};
    let v = gantry::storage::PgVector::connect(app.st.db.clone(), 4).await.unwrap();
    let project = uuid::Uuid::new_v4();
    let meta = |n: u128, label: &str| VecMeta { kind: "item".into(), ref_id: uuid::Uuid::from_u128(n), project_id: Some(project), label: label.into(), model: "t".into(), ..Default::default() };
    v.upsert(&meta(1, "A"), &[("east".into(), vec![1.0, 0.0, 0.0, 0.0])]).await.unwrap();
    v.upsert(&meta(2, "B"), &[("north".into(), vec![0.0, 1.0, 0.0, 0.0])]).await.unwrap();
    let hits = v.search(&[0.9, 0.1, 0.0, 0.0], &SearchScope { projects: vec![project], ..Default::default() }, 5).await.unwrap();
    assert_eq!(hits[0].label, "A");
    assert!(v.search(&[1.0, 0.0, 0.0, 0.0], &SearchScope::default(), 5).await.unwrap().is_empty(), "an empty scope sees nothing");
}

/// Found while rehearsing an upgrade: an object changed twice in a row had two indexing jobs
/// running at once, and the second failed on a duplicate key (it was retried, with a warning
/// in the log). Writers of the same object now take turns.
#[tokio::test]
async fn indexing_the_same_object_from_several_jobs_at_once_does_not_collide() {
    use gantry::storage::{VecMeta, VectorStore};
    let app = spawn().await;
    let mut stores: Vec<(&str, std::sync::Arc<dyn VectorStore>)> = vec![("embeddings", app.st.stores.vector.clone())];
    let available: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'vector')").fetch_one(&app.st.db).await.unwrap();
    if available {
        stores.push(("embeddings_pgv", std::sync::Arc::new(gantry::storage::PgVector::connect(app.st.db.clone(), 4).await.unwrap())));
    } else {
        eprintln!("SKIPPED for pgvector: the extension is not installed on the test PostgreSQL server");
    }
    for (table, store) in stores {
        let id = uuid::Uuid::new_v4();
        let dims = if table == "embeddings_pgv" { 4 } else { 8 };
        let mut jobs = tokio::task::JoinSet::new();
        for writer in 0..24usize {
            let store = store.clone();
            jobs.spawn(async move {
                let meta = VecMeta { kind: "item".into(), ref_id: id, project_id: Some(uuid::Uuid::new_v4()), label: format!("writer {writer}"), model: "t".into(), ..Default::default() };
                let chunks: Vec<(String, Vec<f32>)> = (0..1 + writer % 3).map(|_| (format!("writer {writer}"), vec![0.5; dims])).collect();
                store.upsert(&meta, &chunks).await
            });
        }
        while let Some(done) = jobs.join_next().await {
            done.unwrap().unwrap_or_else(|e| panic!("{table}: a concurrent writer failed: {e}"));
        }
        // What is left is the work of exactly one writer, whole.
        let (chunks, writers): (Vec<i32>, i64) = sqlx::query_as(&format!("SELECT array_agg(chunk ORDER BY chunk), count(DISTINCT content) FROM {table} WHERE ref_id = $1")).bind(id).fetch_one(&app.st.db).await.unwrap();
        assert_eq!(writers, 1, "{table}: rows of several writers are mixed");
        assert_eq!(chunks, (0..chunks.len() as i32).collect::<Vec<_>>(), "{table}");
    }
}

#[tokio::test]
async fn semantic_search_through_pgvector_end_to_end() {
    let probe = spawn().await;
    let available: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pg_available_extensions WHERE name = 'vector')").fetch_one(&probe.st.db).await.unwrap();
    drop(probe);
    if !available {
        eprintln!("SKIPPED: the pgvector extension is not installed on the test PostgreSQL server");
        return;
    }
    let app = spawn_with(|c| c.storage.vector.backend = "pgvector".into()).await;
    assert_eq!(app.st.stores.vector.name(), "pgvector");
    let ada = app.user("ada@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Coolant pump seal leaking", "description": "Glycol under pump P-101."})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Quarterly budget forecast"})).await;
    app.drain_jobs().await;
    let r = ada.ok("/api/v1/search?q=pump%20seal%20leak&mode=semantic").await;
    assert_eq!(r["results"][0]["label"], "OPS-1", "{r}");
    let eve = app.user("eve@example.org", "member").await;
    assert!(eve.ok("/api/v1/search?q=pump%20seal%20leak&mode=semantic").await["results"].as_array().unwrap().is_empty(), "scope applies to the vector index too");
    ada.delete("/api/v1/items/OPS-1").await;
    app.drain_jobs().await;
    assert!(ada.ok("/api/v1/search?q=pump%20seal%20leak&mode=semantic").await["results"].as_array().unwrap().iter().all(|h| h["label"] != "OPS-1"), "archived items leave the vector index");
}

/// Add-on packages only place files on disk; `gantry migrate` loads them. It must report a
/// pack as changed the first time and when its version changes, and not on every start.
#[tokio::test]
async fn addon_sync_reports_only_real_changes() {
    let app = spawn().await;
    let dir = std::env::temp_dir().join(format!("gantry-addons-sync-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(dir.join("quality")).unwrap();
    let write = |version: &str| {
        let pack = json!({"id": "quality", "name": "Quality", "version": version,
            "fields": [{"key": "qm_root_cause", "name": "Root cause", "type": "textarea"}],
            "item_types": [{"key": "qm_ncr", "name": "Nonconformance report", "workflow": "simple", "fields": [{"key": "qm_root_cause"}]}]});
        std::fs::write(dir.join("quality/pack.json"), pack.to_string()).unwrap();
    };
    std::fs::create_dir_all(dir.join("broken")).unwrap();
    std::fs::write(dir.join("broken/pack.json"), "{not json").unwrap();

    write("1.0.0");
    let (done, failed) = gantry::packs::sync_addons(&app.st.db, &dir).await;
    assert_eq!(done.len(), 1);
    assert!(done[0].changed() && done[0].previous.is_none(), "a new pack is a change");
    assert_eq!(failed.len(), 1, "an unreadable pack is reported, not hidden: {failed:?}");
    assert_eq!(failed[0].0, "broken");

    let (done, _) = gantry::packs::sync_addons(&app.st.db, &dir).await;
    assert!(!done[0].changed(), "the same version again changes nothing");

    write("1.1.0");
    let (done, _) = gantry::packs::sync_addons(&app.st.db, &dir).await;
    assert!(done[0].changed());
    assert_eq!(done[0].previous.as_deref(), Some("1.0.0"));
    let admin = app.admin().await;
    let schemes = admin.ok("/api/v1/admin/schemes").await;
    assert!(schemes["item_types"].as_array().unwrap().iter().any(|t| t["key"] == "qm_ncr" && t["source"] == "quality"), "the pack's type is installed: {schemes}");
    std::fs::remove_dir_all(dir).unwrap();
}
