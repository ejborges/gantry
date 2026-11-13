//! Integration tests for everything that talks to another system: chat and webhook delivery,
//! Git hosting webhooks, Jira import, OpenID Connect sign-in, SCIM provisioning and MCP.
//! The other systems are stand-in HTTP servers on loopback ports.

mod common;
use axum::extract::{Query, State};
use axum::routing::{get, post};
use axum::Json;
use common::*;
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

async fn ops_project(app: &TestApp) -> Caller {
    let root = app.admin().await;
    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "software"})).await;
    root.created("/api/v1/projects/OPS/items", json!({"title": "Replace pump seal"})).await;
    root
}

#[tokio::test]
async fn events_are_delivered_to_slack_teams_and_signed_webhooks() {
    let app = spawn_with(|c| c.server.public_url = "https://gantry.example.org".into()).await;
    let (target, seen) = recorder().await;
    // public_url is https, so cookies are Secure-prefixed; sign in over the test's plain HTTP still works.
    let root = app.admin().await;
    root.created("/api/v1/admin/integrations", json!({"kind": "slack", "name": "#ops", "secrets": {"webhook_url": format!("{target}/slack")}, "events": ["item.created"]})).await;
    root.created("/api/v1/admin/integrations", json!({"kind": "teams", "name": "Ops team", "secrets": {"workflow_url": format!("{target}/teams")}, "events": ["*"]})).await;
    let hook = root.created("/api/v1/admin/integrations", json!({"kind": "webhook", "name": "ERP", "config": {"url": format!("{target}/erp")}, "events": ["item.transitioned"]})).await;
    let secret = hook["generated_signing_secret"].as_str().expect("a signing secret is generated and shown once").to_string();
    assert!(secret.starts_with("whsec_"));
    assert_eq!(root.post("/api/v1/admin/integrations", json!({"kind": "teams", "name": "No URL"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.post("/api/v1/admin/integrations", json!({"kind": "slack", "name": "x", "events": ["item.exploded"]})).await.0, StatusCode::BAD_REQUEST);

    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    root.created("/api/v1/projects/OPS/items", json!({"title": "Replace pump seal"})).await;
    root.post("/api/v1/items/OPS-1/transition", json!({"to": "done"})).await;
    // Nothing leaves until the worker runs; then: created → slack + teams, transitioned → teams + webhook.
    assert!(seen.lock().await.is_empty());
    app.drain_jobs().await;
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM jobs WHERE kind = 'deliver' AND state = 'done'").fetch_one(&app.st.db).await.unwrap(), 4);
    let got = seen.lock().await;
    let by_path = |frag: &str| got.iter().filter(|(_, b)| String::from_utf8_lossy(b).contains(frag)).collect::<Vec<_>>();

    let slack: Value = serde_json::from_slice(&by_path("mrkdwn")[0].1).unwrap();
    assert_eq!(slack["text"], "root created OPS-1: Replace pump seal");
    assert!(slack["blocks"][0]["text"]["text"].as_str().unwrap().contains("<https://gantry.example.org/i/OPS-1|OPS-1 Replace pump seal>"));

    let teams = by_path("AdaptiveCard");
    assert_eq!(teams.len(), 2);
    let card: Value = serde_json::from_slice(&teams[0].1).unwrap();
    assert_eq!(card["attachments"][0]["contentType"], "application/vnd.microsoft.card.adaptive");
    assert_eq!(card["attachments"][0]["content"]["actions"][0]["url"], "https://gantry.example.org/i/OPS-1");

    let (headers, body) = by_path("occurred_at")[0];
    let event: Value = serde_json::from_slice(body).unwrap();
    assert_eq!((event["type"].as_str(), event["data"]["to"].as_str()), (Some("item.transitioned"), Some("done")));
    let h = |n: &str| headers[n].to_str().unwrap().to_string();
    assert!(gantry::integrations::sw_verify(&secret, &h("webhook-id"), &h("webhook-timestamp"), body, &h("webhook-signature"), chrono::Utc::now().timestamp()), "the receiver can verify the signature");
    assert!(!gantry::integrations::sw_verify("whsec_b3RoZXI=", &h("webhook-id"), &h("webhook-timestamp"), body, &h("webhook-signature"), chrono::Utc::now().timestamp()));
    drop(got);

    // The test button reports the outcome and the delivery log records it.
    let id = hook["id"].as_str().unwrap();
    assert_eq!(root.post(&format!("/api/v1/admin/integrations/{id}/test"), json!({})).await.1["ok"], true);
    let log = root.ok(&format!("/api/v1/admin/integrations/{id}/deliveries")).await;
    assert_eq!(log.as_array().unwrap().len(), 2);
    assert!(log.as_array().unwrap().iter().all(|d| d["outcome"] == "ok" && d["status"] == 200));
}

#[tokio::test]
async fn internal_notes_and_scoped_integrations_do_not_leak() {
    let app = spawn().await;
    let (target, seen) = recorder().await;
    let root = app.admin().await;
    root.created("/api/v1/projects", json!({"key": "IT", "name": "IT", "template": "it-service-desk"})).await;
    root.created("/api/v1/projects", json!({"key": "HR", "name": "HR", "template": "basic"})).await;
    root.created("/api/v1/admin/integrations", json!({"kind": "slack", "name": "IT only", "secrets": {"webhook_url": target}, "events": ["*"], "project": "IT"})).await;
    root.created("/api/v1/projects/HR/items", json!({"title": "Confidential HR matter"})).await;
    root.created("/api/v1/projects/IT/items", json!({"title": "Printer"})).await;
    root.created("/api/v1/items/IT-1/comments", json!({"body": "internal: user is on a watch list", "internal": true})).await;
    root.created("/api/v1/items/IT-1/comments", json!({"body": "We are on it."})).await;
    app.drain_jobs().await;
    let bodies: Vec<String> = seen.lock().await.iter().map(|(_, b)| String::from_utf8_lossy(b).into_owned()).collect();
    assert_eq!(bodies.len(), 2, "IT item created + public comment only: {bodies:?}");
    assert!(bodies.iter().all(|b| !b.contains("HR") && !b.contains("watch list")));
}

#[tokio::test]
async fn failed_deliveries_retry_then_stop() {
    let app = spawn().await;
    let root = app.admin().await;
    // Nothing listens on this port.
    root.created("/api/v1/admin/integrations", json!({"kind": "webhook", "name": "Down", "config": {"url": "http://127.0.0.1:9/hook"}, "events": ["item.created"]})).await;
    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    root.created("/api/v1/projects/OPS/items", json!({"title": "x"})).await;
    app.drain_jobs().await;
    let (state, attempts, err, later): (String, i32, Option<String>, bool) = sqlx::query_as("SELECT state, attempts, last_error, run_at > now() + interval '20 seconds' FROM jobs WHERE kind = 'deliver'").fetch_one(&app.st.db).await.unwrap();
    assert_eq!((state.as_str(), attempts, later), ("queued", 1, true), "requeued with a delay");
    assert!(err.unwrap().contains("request failed"));
    // Exhaust the attempts.
    for _ in 0..5 {
        sqlx::query("UPDATE jobs SET run_at = now() WHERE kind = 'deliver'").execute(&app.st.db).await.unwrap();
        app.drain_jobs().await;
    }
    let (state, attempts): (String, i32) = sqlx::query_as("SELECT state, attempts FROM jobs WHERE kind = 'deliver'").fetch_one(&app.st.db).await.unwrap();
    assert_eq!((state.as_str(), attempts), ("failed", 6));
    let jobs = root.ok("/api/v1/admin/jobs").await;
    let failed = jobs.as_array().unwrap().iter().find(|j| j["state"] == "failed").unwrap();
    assert!(app.audit_actions().await.contains(&"job.failed".to_string()));
    assert_eq!(root.post(&format!("/api/v1/admin/jobs/{}/retry", failed["id"]), json!({})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(root.ok("/api/v1/admin/overview").await["counts"]["jobs_queued"], 1);
}

#[tokio::test]
async fn github_webhooks_link_commits_and_complete_items_on_merge() {
    let app = spawn().await;
    let root = ops_project(&app).await;
    root.created("/api/v1/projects/OPS/items", json!({"title": "Second"})).await;
    let gh = root.created("/api/v1/admin/integrations", json!({"kind": "github", "name": "GH", "secrets": {"webhook_secret": "s3cret"}, "config": {"auto_transition": "true"}})).await;
    assert!(gh["inbound_url"].as_str().unwrap().ends_with(&format!("/api/v1/hooks/github/{}", gh["id"].as_str().unwrap())));
    let url = format!("{}/api/v1/hooks/github/{}", app.base, gh["id"].as_str().unwrap());
    let send = |event: &'static str, delivery: &'static str, body: Value| {
        let raw = body.to_string();
        let sig = format!("sha256={}", hex(&gantry::crypto::hmac_sha256(b"s3cret", raw.as_bytes())));
        app.http.post(&url).header("x-github-event", event).header("x-github-delivery", delivery).header("x-hub-signature-256", sig).header("content-type", "application/json").body(raw).send()
    };
    let push = json!({"repository": {"full_name": "acme/pump"}, "commits": [
        {"id": "9fceb02d0ae598e95dc970b74767f19372d61af8", "message": "OPS-1: tighten seal housing\n\nmore detail", "url": "https://github.com/acme/pump/commit/9fceb02"},
        {"id": "aaaabbbbccccddddeeeeffff0000111122223333", "message": "unrelated chore", "url": "https://github.com/acme/pump/commit/aaaabbb"},
        {"id": "1111222233334444555566667777888899990000", "message": "ZZZ-9 not ours; javascript link", "url": "javascript:alert(1)"}]});
    let r: Value = send("push", "d-1", push.clone()).await.unwrap().json().await.unwrap();
    assert_eq!(r["linked"], 1);
    let r: Value = send("push", "d-1", push).await.unwrap().json().await.unwrap();
    assert_eq!(r["duplicate"], true, "a redelivered webhook is acknowledged and ignored");
    let item = root.ok("/api/v1/items/OPS-1").await;
    let link = &item["links"][0];
    assert_eq!((link["rel"].as_str(), link["other"]["type"].as_str(), link["other"]["external"].as_bool()), (Some("mentioned_in"), Some("commit"), Some(true)));
    assert_eq!(link["other"]["title"], "9fceb02d OPS-1: tighten seal housing");
    assert_eq!(link["other"]["url"], "https://github.com/acme/pump/commit/9fceb02");

    // An opened pull request links; a merged one that says "fixes" completes the item.
    let pr = |action: &str, merged: bool| json!({"action": action, "repository": {"full_name": "acme/pump"},
        "pull_request": {"number": 42, "title": "Seal housing rework", "body": "Fixes OPS-1. Relates to OPS-2.", "html_url": "https://github.com/acme/pump/pull/42", "state": if merged { "closed" } else { "open" }, "merged": merged, "head": {"ref": "feature/ops-1-seal"}}});
    assert_eq!(send("pull_request", "d-2", pr("opened", false)).await.unwrap().json::<Value>().await.unwrap()["linked"], 2);
    assert_eq!(root.ok("/api/v1/items/OPS-1").await["status_category"], "todo");
    let r: Value = send("pull_request", "d-3", pr("closed", true)).await.unwrap().json().await.unwrap();
    assert_eq!(r["completed"], 1);
    assert_eq!(root.ok("/api/v1/items/OPS-1").await["status"], "done");
    assert_eq!(root.ok("/api/v1/items/OPS-2").await["status_category"], "todo", "only items named after a closing keyword are completed");
    let row: (String, String) = sqlx::query_as("SELECT actor_label, detail->>'source' FROM audit_log WHERE action = 'item.transition'").fetch_one(&app.st.db).await.unwrap();
    assert_eq!(row, ("integration:GH".to_string(), "acme/pump#42".to_string()));
}

#[tokio::test]
async fn gitlab_webhooks_accept_signing_tokens_and_legacy_tokens() {
    let app = spawn().await;
    let root = ops_project(&app).await;
    let secret = "whsec_MfKQ9r8GKYqrTwjUPD8ILPZIo2LaLaSw";
    let signed = root.created("/api/v1/admin/integrations", json!({"kind": "gitlab", "name": "GL signed", "secrets": {"signing_token": secret, "secret_token": "legacy"}, "config": {"auto_transition": "true"}})).await;
    let url = format!("{}/api/v1/hooks/gitlab/{}", app.base, signed["id"].as_str().unwrap());
    let mr = json!({"object_kind": "merge_request", "project": {"path_with_namespace": "acme/pump"},
        "object_attributes": {"iid": 7, "title": "Seal rework", "description": "Closes OPS-1", "source_branch": "seal", "url": "https://gitlab.example.org/acme/pump/-/merge_requests/7", "state": "merged", "action": "merge"}}).to_string();
    let now = chrono::Utc::now().timestamp();
    let sig = gantry::integrations::sw_sign(secret, "wh_1", now, mr.as_bytes());
    let post = |id: &'static str, ts: i64, sig: String, token: Option<&'static str>| {
        let mut r = app.http.post(&url).header("x-gitlab-event", "Merge Request Hook").header("webhook-id", id).header("webhook-timestamp", ts.to_string()).header("webhook-signature", sig).body(mr.clone());
        if let Some(t) = token {
            r = r.header("x-gitlab-token", t);
        }
        r.send()
    };
    // With a signing token configured, the plain token alone is not enough, and stale or forged signatures fail.
    assert_eq!(app.http.post(&url).header("x-gitlab-token", "legacy").body(mr.clone()).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(post("wh_1", now - 3600, gantry::integrations::sw_sign(secret, "wh_1", now - 3600, mr.as_bytes()), None).await.unwrap().status(), StatusCode::UNAUTHORIZED);
    assert_eq!(post("wh_2", now, sig.clone(), None).await.unwrap().status(), StatusCode::UNAUTHORIZED, "signature is bound to the message id");
    let r: Value = post("wh_1", now, sig, None).await.unwrap().json().await.unwrap();
    assert_eq!((r["linked"].as_i64(), r["completed"].as_i64()), (Some(1), Some(1)));
    assert_eq!(root.ok("/api/v1/items/OPS-1").await["status"], "done");

    // Older GitLab: secret token only.
    let legacy = root.created("/api/v1/admin/integrations", json!({"kind": "gitlab", "name": "GL legacy", "secrets": {"secret_token": "legacy-token"}})).await;
    let url2 = format!("{}/api/v1/hooks/gitlab/{}", app.base, legacy["id"].as_str().unwrap());
    let push = json!({"object_kind": "push", "project": {"path_with_namespace": "acme/pump"}, "commits": [{"id": "abcdef0123456789", "message": "OPS-1 follow-up", "url": "https://gitlab.example.org/c/abcdef"}]}).to_string();
    assert_eq!(app.http.post(&url2).header("x-gitlab-token", "wrong").body(push.clone()).send().await.unwrap().status(), StatusCode::UNAUTHORIZED);
    let r: Value = app.http.post(&url2).header("x-gitlab-token", "legacy-token").header("x-gitlab-event", "Push Hook").body(push).send().await.unwrap().json().await.unwrap();
    assert_eq!(r["linked"], 1);
}

#[tokio::test]
async fn slack_slash_command_acts_as_the_configured_user() {
    let app = spawn().await;
    let root = ops_project(&app).await;
    let bot = root.created("/api/v1/admin/users", json!({"name": "Slack bridge", "kind": "agent"})).await;
    root.created("/api/v1/projects/OPS/members", json!({"user_id": bot["id"], "role": "project_member"})).await;
    root.agent_rules(bot["id"].as_str().unwrap(), working_agent()).await;
    root.created("/api/v1/projects", json!({"key": "HR", "name": "HR", "template": "basic"})).await;
    root.created("/api/v1/projects/HR/items", json!({"title": "Private HR matter"})).await;
    let slack = root.created("/api/v1/admin/integrations", json!({"kind": "slack", "name": "Slack", "secrets": {"signing_secret": "slack-secret"}, "config": {"acting_user_id": bot["id"]}})).await;
    let url = format!("{}/api/v1/hooks/slack/{}/command", app.base, slack["id"].as_str().unwrap());
    let cmd = |text: &str, ts_offset: i64| {
        let body: String = url::form_urlencoded::Serializer::new(String::new()).append_pair("command", "/gantry").append_pair("text", text).append_pair("user_name", "pat").finish();
        let ts = chrono::Utc::now().timestamp() + ts_offset;
        let sig = format!("v0={}", hex(&gantry::crypto::hmac_sha256(b"slack-secret", format!("v0:{ts}:{body}").as_bytes())));
        app.http.post(&url).header("x-slack-request-timestamp", ts.to_string()).header("x-slack-signature", sig).header("content-type", "application/x-www-form-urlencoded").body(body).send()
    };
    let text = |r: reqwest::Response| async move { r.json::<Value>().await.unwrap()["text"].as_str().unwrap().to_string() };
    assert_eq!(cmd("show OPS-1", -600).await.unwrap().status(), StatusCode::UNAUTHORIZED, "replayed request");
    assert!(text(cmd("show OPS-1", 0).await.unwrap()).await.contains("Replace pump seal"));
    assert!(text(cmd("show HR-1", 0).await.unwrap()).await.contains("No such work item"), "the bridge user cannot see HR");
    let created = text(cmd("create OPS Check spare seals in stores", 0).await.unwrap()).await;
    assert!(created.contains("OPS-2"), "{created}");
    let item = root.ok("/api/v1/items/OPS-2").await;
    assert_eq!((item["channel"].as_str(), item["reporter"]["name"].as_str()), (Some("slack"), Some("Slack bridge")));
    assert!(item["description"].as_str().unwrap().contains("@pat"));
    assert!(text(cmd("create HR Sneak one in", 0).await.unwrap()).await.contains("Could not create"));
    assert!(text(cmd("find seal", 0).await.unwrap()).await.contains("OPS-1"));
    assert!(text(cmd("", 0).await.unwrap()).await.contains("Usage"));
}

// ───────────────────────── Jira ─────────────────────────

fn jira_issue(key: &str, summary: &str, itype: &str, status: (&str, &str), parent: Option<&str>) -> Value {
    let mut f = json!({
        "summary": summary, "issuetype": {"name": itype}, "status": {"name": status.0, "statusCategory": {"key": status.1}},
        "priority": {"name": "High"}, "labels": ["Legacy", "pump"], "duedate": "2031-05-01",
        "assignee": {"displayName": "Root", "emailAddress": "root@example.org"}, "reporter": {"displayName": "Gone Person", "emailAddress": "gone@example.org"},
        "description": {"type": "doc", "version": 1, "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Seal fails at "}, {"type": "text", "text": "40 bar", "marks": [{"type": "strong"}]}]}]},
        "comment": {"comments": [{"author": {"displayName": "Gone Person"}, "created": "2024-03-05T10:20:30.000+0000", "body": {"type": "doc", "content": [{"type": "paragraph", "content": [{"type": "text", "text": "Seen twice this week."}]}]}}]}
    });
    if let Some(p) = parent {
        f["parent"] = json!({"key": p});
    }
    json!({"key": key, "fields": f})
}

#[derive(Clone, Default)]
struct JiraMock {
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

async fn jira_cloud_search(State(m): State<JiraMock>, headers: axum::http::HeaderMap, Json(body): Json<Value>) -> (axum::http::StatusCode, Json<Value>) {
    m.calls.lock().await.push((headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string(), body.clone()));
    if body["jql"] == "bad jql" {
        return (axum::http::StatusCode::BAD_REQUEST, Json(json!({"errorMessages": ["bad"]})));
    }
    // Two pages, linked by nextPageToken.
    if body["nextPageToken"].is_null() {
        (axum::http::StatusCode::OK, Json(json!({"issues": [jira_issue("PUMP-1", "Seal program", "Epic", ("To Do", "new"), None), jira_issue("PUMP-2", "Seal fails under pressure", "Bug", ("In Progress", "indeterminate"), Some("PUMP-1"))], "nextPageToken": "page2"})))
    } else {
        (axum::http::StatusCode::OK, Json(json!({"issues": [jira_issue("PUMP-3", "Order spare seals", "Shopping", ("Closed", "done"), None)], "isLast": true})))
    }
}

async fn jira_dc_search(State(m): State<JiraMock>, headers: axum::http::HeaderMap, Query(q): Query<HashMap<String, String>>) -> Json<Value> {
    m.calls.lock().await.push((headers.get("authorization").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string(), json!(q)));
    let start: usize = q["startAt"].parse().unwrap();
    let mut issue = jira_issue("DC-1", "From Data Center", "Task", ("Done", "done"), None);
    issue["fields"]["description"] = json!("h2. Wiki markup\n\nplain text body");
    issue["fields"]["comment"] = json!({"comments": []});
    Json(if start == 0 { json!({"total": 1, "issues": [issue]}) } else { json!({"total": 1, "issues": []}) })
}

async fn start_jira() -> (String, JiraMock) {
    let m = JiraMock::default();
    let app = axum::Router::new().route("/rest/api/3/search/jql", post(jira_cloud_search)).route("/rest/api/2/search", get(jira_dc_search)).with_state(m.clone());
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    (format!("http://{addr}"), m)
}

#[tokio::test]
async fn jira_cloud_import_maps_issues_and_is_repeatable() {
    let app = spawn().await;
    let (jira, mock) = start_jira().await;
    let root = app.admin().await;
    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "software"})).await;
    let integ = root.created("/api/v1/admin/integrations", json!({"kind": "jira", "name": "Old Jira", "config": {"base_url": jira, "deployment": "cloud", "email": "me@example.org"}, "secrets": {"api_token": "jira-api-token"}})).await;
    let path = format!("/api/v1/admin/integrations/{}/jira-import", integ["id"].as_str().unwrap());
    assert_eq!(root.post(&path, json!({"project": "NOPE", "jql": "project = PUMP"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.post(&path, json!({"project": "OPS", "jql": " "})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.post(&path, json!({"project": "OPS", "jql": "project = PUMP ORDER BY created"})).await.0, StatusCode::ACCEPTED);
    app.drain_jobs().await;

    let calls = mock.calls.lock().await.clone();
    assert_eq!(calls.len(), 2, "two pages were fetched");
    assert!(calls[0].0.starts_with("Basic "), "Jira Cloud uses email + API token");
    assert_eq!(calls[0].1["jql"], "project = PUMP ORDER BY created");
    assert!(calls[0].1["fields"].as_array().unwrap().contains(&json!("summary")), "fields are requested explicitly");
    assert_eq!(calls[1].1["nextPageToken"], "page2");

    let list = root.ok("/api/v1/items?project=OPS&sort=key").await;
    assert_eq!(list["total"], 3);
    let bug = root.ok("/api/v1/items/OPS-2").await;
    assert_eq!((bug["type"].as_str(), bug["status"].as_str(), bug["priority"].as_i64()), (Some("bug"), Some("in_progress"), Some(3)));
    assert_eq!(bug["title"], "Seal fails under pressure");
    assert_eq!(bug["description"], "Seal fails at **40 bar**", "Atlassian Document Format becomes Markdown");
    assert_eq!(bug["labels"], json!(["legacy", "pump"]));
    assert_eq!(bug["due_date"], "2031-05-01");
    assert_eq!(bug["assignee"]["name"], "root", "people are matched by email");
    assert_eq!(bug["parent"]["key"], "OPS-1", "the Jira parent link is kept");
    assert_eq!(bug["channel"], "jira");
    assert_eq!(bug["external_refs"][0]["external_id"], "PUMP-2");
    assert!(bug["external_refs"][0]["url"].as_str().unwrap().ends_with("/browse/PUMP-2"));
    assert!(bug["comments"][0]["body"].as_str().unwrap().starts_with("**Gone Person** (imported from Jira)"));
    assert!(bug["comments"][0]["created_at"].as_str().unwrap().starts_with("2024-03-05T10:20:30"));
    let epic = root.ok("/api/v1/items/OPS-1").await;
    assert_eq!((epic["type"].as_str(), epic["status_category"].as_str()), (Some("epic"), Some("todo")));
    let other = root.ok("/api/v1/items/OPS-3").await;
    assert_eq!((other["type"].as_str(), other["status_category"].as_str()), (Some("task"), Some("done")), "unknown Jira types fall back to the project default");

    // Edit locally, re-import: same three items, refreshed from Jira, comments not duplicated.
    root.patch("/api/v1/items/OPS-2", json!({"title": "Local edit"})).await;
    root.post(&path, json!({"project": "OPS", "jql": "project = PUMP"})).await;
    app.drain_jobs().await;
    assert_eq!(root.ok("/api/v1/items?project=OPS").await["total"], 3);
    let bug = root.ok("/api/v1/items/OPS-2").await;
    assert_eq!(bug["title"], "Seal fails under pressure");
    assert_eq!(bug["comments"].as_array().unwrap().len(), 1);
    assert!(app.audit_actions().await.iter().filter(|a| *a == "jira.import").count() == 2);

    // A rejected query fails the job with a useful message rather than importing nothing silently.
    root.post(&path, json!({"project": "OPS", "jql": "bad jql"})).await;
    app.drain_jobs().await;
    let err: String = sqlx::query_scalar("SELECT last_error FROM jobs WHERE kind = 'jira_import' AND last_error IS NOT NULL").fetch_one(&app.st.db).await.unwrap();
    assert!(err.contains("400") && err.contains("JQL"), "{err}");
}

#[tokio::test]
async fn jira_data_center_import_uses_bearer_token_and_offsets() {
    let app = spawn().await;
    let (jira, mock) = start_jira().await;
    let root = app.admin().await;
    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "basic"})).await;
    let integ = root.created("/api/v1/admin/integrations", json!({"kind": "jira", "name": "DC", "config": {"base_url": jira, "deployment": "datacenter"}, "secrets": {"api_token": "pat-123"}})).await;
    root.post(&format!("/api/v1/admin/integrations/{}/jira-import", integ["id"].as_str().unwrap()), json!({"project": "OPS", "jql": "project = DC"})).await;
    app.drain_jobs().await;
    let calls = mock.calls.lock().await.clone();
    assert_eq!(calls[0].0, "Bearer pat-123");
    assert_eq!(calls[0].1["startAt"], "0");
    let item = root.ok("/api/v1/items/OPS-1").await;
    assert_eq!(item["description"], "h2. Wiki markup\n\nplain text body", "Data Center text is kept as is");
    assert_eq!(item["status_category"], "done");
}

// ───────────────────────── OpenID Connect ─────────────────────────

#[derive(Clone)]
struct Idp {
    base: String,
    key_pem: Arc<String>,
    jwks: Arc<Value>,
    /// Claims to put in the next ID token; the nonce is filled in from the authorization request unless overridden.
    claims: Arc<Mutex<Value>>,
    last_token_form: Arc<Mutex<HashMap<String, String>>>,
    nonce: Arc<Mutex<String>>,
}

fn test_key() -> &'static (String, Value) {
    static KEY: std::sync::OnceLock<(String, Value)> = std::sync::OnceLock::new();
    KEY.get_or_init(|| {
        use base64::Engine;
        use rsa::pkcs1::EncodeRsaPrivateKey;
        use rsa::traits::PublicKeyParts;
        let key = rsa::RsaPrivateKey::new(&mut rand::thread_rng(), 2048).unwrap();
        let b64 = |b: Vec<u8>| base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(b);
        let jwks = json!({"keys": [{"kty": "RSA", "kid": "k1", "use": "sig", "alg": "RS256", "n": b64(key.n().to_bytes_be()), "e": b64(key.e().to_bytes_be())}]});
        (key.to_pkcs1_pem(rsa::pkcs1::LineEnding::LF).unwrap().to_string(), jwks)
    })
}

async fn start_idp() -> Idp {
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = format!("http://{}", l.local_addr().unwrap());
    let (pem, jwks) = test_key().clone();
    let idp = Idp { base: base.clone(), key_pem: Arc::new(pem), jwks: Arc::new(jwks), claims: Arc::new(Mutex::new(json!({}))), last_token_form: Default::default(), nonce: Default::default() };
    let app = axum::Router::new()
        .route("/.well-known/openid-configuration", get(|State(i): State<Idp>| async move { Json(json!({"issuer": i.base, "authorization_endpoint": format!("{}/authorize", i.base), "token_endpoint": format!("{}/token", i.base), "jwks_uri": format!("{}/jwks", i.base)})) }))
        .route("/jwks", get(|State(i): State<Idp>| async move { Json((*i.jwks).clone()) }))
        .route(
            "/token",
            post(|State(i): State<Idp>, axum::Form(form): axum::Form<HashMap<String, String>>| async move {
                *i.last_token_form.lock().await = form.clone();
                let mut claims = json!({"iss": i.base, "aud": form["client_id"], "sub": "idp-user-1", "exp": chrono::Utc::now().timestamp() + 300, "iat": chrono::Utc::now().timestamp(), "nonce": *i.nonce.lock().await});
                for (k, v) in i.claims.lock().await.as_object().unwrap() {
                    claims[k] = v.clone();
                }
                let mut header = jsonwebtoken::Header::new(jsonwebtoken::Algorithm::RS256);
                header.kid = Some("k1".into());
                let id_token = jsonwebtoken::encode(&header, &claims, &jsonwebtoken::EncodingKey::from_rsa_pem(i.key_pem.as_bytes()).unwrap()).unwrap();
                Json(json!({"access_token": "at", "token_type": "Bearer", "id_token": id_token}))
            }),
        )
        .with_state(idp.clone());
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    idp
}

/// Drive the browser side of the flow: start → (IdP) → callback. Returns the callback response.
async fn sso(app: &TestApp, idp: &Idp, provider: &str, tamper_state: bool) -> reqwest::Response {
    let start = app.http.get(format!("{}/api/v1/auth/oidc/{provider}/start", app.base)).send().await.unwrap();
    assert_eq!(start.status(), StatusCode::FOUND);
    let loc = url::Url::parse(start.headers()["location"].to_str().unwrap()).unwrap();
    let q: HashMap<String, String> = loc.query_pairs().into_owned().collect();
    assert!(loc.as_str().starts_with(&format!("{}/authorize", idp.base)));
    assert_eq!((q["response_type"].as_str(), q["code_challenge_method"].as_str()), ("code", "S256"));
    assert!(q["redirect_uri"].ends_with(&format!("/api/v1/auth/oidc/{provider}/callback")));
    *idp.nonce.lock().await = q["nonce"].clone();
    let state = if tamper_state { "forged-state".to_string() } else { q["state"].clone() };
    let cb = app.http.get(format!("{}/api/v1/auth/oidc/{provider}/callback?code=abc&state={state}", app.base)).send().await.unwrap();
    if !tamper_state {
        // The token request proves possession of the PKCE verifier that matches the challenge.
        let form = idp.last_token_form.lock().await.clone();
        if let Some(v) = form.get("code_verifier") {
            assert_eq!(gantry::oidc::pkce_challenge(v), q["code_challenge"]);
            assert_eq!(form["grant_type"], "authorization_code");
        }
    }
    cb
}

#[tokio::test]
async fn single_sign_on_provisions_links_and_rejects_forgeries() {
    let app = spawn().await;
    let idp = start_idp().await;
    let root = app.admin().await;
    let (s, v) = root.post("/api/v1/admin/auth-providers", json!({"name": "Bad", "issuer": "http://127.0.0.1:9", "client_id": "x"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "an unreachable issuer is refused when saving: {v}");
    let p = root.created("/api/v1/admin/auth-providers", json!({"name": "Corp SSO", "issuer": idp.base, "client_id": "gantry-client", "client_secret": "idp-secret", "allowed_domains": ["example.org"]})).await;
    let pid = p["id"].as_str().unwrap().to_string();
    let listed = root.ok("/api/v1/admin/auth-providers").await;
    assert_eq!(listed[0]["has_secret"], true);
    assert!(!listed.to_string().contains("idp-secret"));
    let public: Value = app.http.get(format!("{}/api/v1/auth/providers", app.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(public["providers"][0]["name"], "Corp SSO");

    // First sign-in provisions a member.
    *idp.claims.lock().await = json!({"email": "Grace@Example.org", "email_verified": true, "name": "Grace Hopper"});
    let cb = sso(&app, &idp, &pid, false).await;
    assert_eq!(cb.status(), StatusCode::FOUND);
    assert_eq!(cb.headers()["location"], "/");
    let cookie = cb.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_string();
    let me: Value = app.http.get(format!("{}/api/v1/auth/me", app.base)).header("cookie", &cookie).send().await.unwrap().json().await.unwrap();
    assert_eq!((me["user"]["email"].as_str(), me["user"]["name"].as_str()), (Some("grace@example.org"), Some("Grace Hopper")));
    assert!(me["permissions"].as_array().unwrap().contains(&json!("workspace.access")));
    assert_eq!(idp.last_token_form.lock().await["client_secret"], "idp-secret");
    // Second sign-in finds the same account through the stored identity, even if the email changes.
    *idp.claims.lock().await = json!({"email": "grace.hopper@example.org", "email_verified": true});
    assert_eq!(sso(&app, &idp, &pid, false).await.headers()["location"], "/");
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users WHERE kind = 'human'").fetch_one(&app.st.db).await.unwrap(), 2);
    let actions = app.audit_actions().await;
    assert!(actions.contains(&"user.provision".to_string()) && actions.iter().filter(|a| *a == "auth.login").count() >= 3);

    // Forgeries and policy violations all end at the login page with an error, never with a session.
    let failed = |r: reqwest::Response| {
        assert_eq!(r.headers()["location"], "/login?error=sso");
        assert!(!r.headers().contains_key("set-cookie"));
    };
    failed(sso(&app, &idp, &pid, true).await);
    *idp.claims.lock().await = json!({"sub": "u2", "email": "x@example.org", "nonce": "attacker-chosen"});
    failed(sso(&app, &idp, &pid, false).await);
    *idp.claims.lock().await = json!({"sub": "u3", "email": "x@example.org", "aud": "some-other-client"});
    failed(sso(&app, &idp, &pid, false).await);
    *idp.claims.lock().await = json!({"sub": "u4", "email": "x@example.org", "iss": "https://evil.example"});
    failed(sso(&app, &idp, &pid, false).await);
    *idp.claims.lock().await = json!({"sub": "u5", "email": "x@example.org", "exp": chrono::Utc::now().timestamp() - 3600});
    failed(sso(&app, &idp, &pid, false).await);
    *idp.claims.lock().await = json!({"sub": "u6", "email": "mallory@other.test", "email_verified": true});
    failed(sso(&app, &idp, &pid, false).await);
    // An unverified email must not take over the existing administrator account.
    *idp.claims.lock().await = json!({"sub": "u7", "email": "root@example.org", "email_verified": false});
    failed(sso(&app, &idp, &pid, false).await);
    // A state value cannot be replayed.
    let start = app.http.get(format!("{}/api/v1/auth/oidc/{pid}/start", app.base)).send().await.unwrap();
    let q: HashMap<String, String> = url::Url::parse(start.headers()["location"].to_str().unwrap()).unwrap().query_pairs().into_owned().collect();
    *idp.nonce.lock().await = q["nonce"].clone();
    *idp.claims.lock().await = json!({"email": "grace@example.org", "email_verified": true});
    let once = app.http.get(format!("{}/api/v1/auth/oidc/{pid}/callback?code=abc&state={}", app.base, q["state"])).send().await.unwrap();
    assert_eq!(once.headers()["location"], "/");
    failed(app.http.get(format!("{}/api/v1/auth/oidc/{pid}/callback?code=abc&state={}", app.base, q["state"])).send().await.unwrap());
    assert!(app.security_kinds().await.iter().filter(|k| *k == "auth.sso_failed").count() >= 8);
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM users WHERE kind = 'human'").fetch_one(&app.st.db).await.unwrap(), 2, "no account was created by any failed attempt");

    // Verified email links to an existing local account instead of duplicating it.
    *idp.claims.lock().await = json!({"sub": "root-at-idp", "email": "root@example.org", "email_verified": true});
    let cb = sso(&app, &idp, &pid, false).await;
    let cookie = cb.headers()["set-cookie"].to_str().unwrap().split(';').next().unwrap().to_string();
    let me: Value = app.http.get(format!("{}/api/v1/auth/me", app.base)).header("cookie", &cookie).send().await.unwrap().json().await.unwrap();
    assert_eq!(me["user"]["id"], json!(root.user_id));
    // Disabling the provider stops sign-in.
    root.patch(&format!("/api/v1/admin/auth-providers/{pid}"), json!({"enabled": false})).await;
    assert_eq!(app.http.get(format!("{}/api/v1/auth/oidc/{pid}/start", app.base)).send().await.unwrap().status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn local_sign_in_can_be_switched_off() {
    let app = spawn_with(|c| c.auth.allow_local_login = false).await;
    app.make_user("ada@example.org", "member").await;
    assert_eq!(app.login_with("ada@example.org", PASSWORD).await.err(), Some(StatusCode::FORBIDDEN));
    let public: Value = app.http.get(format!("{}/api/v1/auth/providers", app.base)).send().await.unwrap().json().await.unwrap();
    assert_eq!(public["local_login"], false);
}

// ───────────────────────── SCIM ─────────────────────────

#[tokio::test]
async fn scim_provisions_and_deactivates_accounts() {
    let app = spawn().await;
    let root = app.admin().await;
    // A dedicated provisioning identity: no access to content, only SCIM.
    let svc = root.created("/api/v1/admin/users", json!({"name": "Entra provisioning", "kind": "agent", "role": "provisioner"})).await;
    // Agents start with no rights at all: before its rules are written, even the role's one permission is withheld.
    let tok = root.created(&format!("/api/v1/admin/users/{}/tokens", svc["id"].as_str().unwrap()), json!({"name": "scim", "scopes": ["admin"]})).await;
    let early = Caller { token: Some(tok["token"].as_str().unwrap().to_string()), ..app.anon() };
    assert_eq!(early.get("/scim/v2/ServiceProviderConfig").await.0, StatusCode::FORBIDDEN);
    root.agent_rules(svc["id"].as_str().unwrap(), json!({"permissions": ["scim.provision"]})).await;
    let scim = Caller { token: Some(tok["token"].as_str().unwrap().to_string()), ..app.anon() };
    assert_eq!(scim.get("/api/v1/admin/users").await.0, StatusCode::FORBIDDEN, "the provisioning identity cannot use the admin API");
    assert_eq!(scim.ok("/api/v1/projects").await.as_array().unwrap().len(), 0);
    let member = app.user("member@example.org", "member").await;
    assert_eq!(member.get("/scim/v2/Users").await.0, StatusCode::FORBIDDEN);

    assert_eq!(scim.ok("/scim/v2/ServiceProviderConfig").await["patch"]["supported"], true);
    assert_eq!(scim.ok("/scim/v2/Users?filter=userName%20eq%20%22grace%40example.org%22").await["totalResults"], 0);
    let (s, u) = scim.post("/scim/v2/Users", json!({"schemas": ["urn:ietf:params:scim:schemas:core:2.0:User"], "userName": "Grace@Example.org", "externalId": "ext-1", "name": {"givenName": "Grace", "familyName": "Hopper"}, "active": true})).await;
    assert_eq!(s, StatusCode::CREATED, "{u}");
    let uid = u["id"].as_str().unwrap().to_string();
    assert_eq!((u["userName"].as_str(), u["displayName"].as_str(), u["active"].as_bool()), (Some("grace@example.org"), Some("Grace Hopper"), Some(true)));
    assert_eq!(scim.post("/scim/v2/Users", json!({"userName": "grace@example.org"})).await.0, StatusCode::CONFLICT);
    let found = scim.ok("/scim/v2/Users?filter=userName%20eq%20%22GRACE%40example.org%22").await;
    assert_eq!((found["totalResults"].as_i64(), found["Resources"][0]["id"].as_str()), (Some(1), Some(uid.as_str())));
    assert_eq!(scim.get("/scim/v2/Users?filter=userName%20co%20%22g%22").await.0, StatusCode::BAD_REQUEST);

    // Groups and membership.
    let (s, g) = scim.post("/scim/v2/Groups", json!({"displayName": "Propulsion", "members": [{"value": uid}]})).await;
    assert_eq!(s, StatusCode::CREATED);
    let gid = g["id"].as_str().unwrap().to_string();
    assert_eq!(g["members"][0]["value"], uid.as_str());
    let group_path = format!("/scim/v2/Groups/{gid}");
    let patch = |ops: Value| scim.patch(&group_path, json!({"schemas": ["urn:ietf:params:scim:api:messages:2.0:PatchOp"], "Operations": ops}));
    let (_, g) = patch(json!([{"op": "remove", "path": format!("members[value eq \"{uid}\"]")}])).await;
    assert_eq!(g["members"].as_array().unwrap().len(), 0);
    let (_, g) = patch(json!([{"op": "Add", "path": "members", "value": [{"value": uid}]}, {"op": "replace", "path": "displayName", "value": "Propulsion team"}])).await;
    assert_eq!((g["members"].as_array().unwrap().len(), g["displayName"].as_str()), (1, Some("Propulsion team")));
    assert_eq!(scim.ok("/scim/v2/Groups?filter=displayName%20eq%20%22Propulsion%20team%22").await["totalResults"], 1);

    // The provisioned person signs in through SSO in production; here, give them a session to prove deactivation cuts it.
    sqlx::query("UPDATE users SET password_hash = (SELECT password_hash FROM users WHERE email = 'member@example.org') WHERE id = $1::uuid").bind(&uid).execute(&app.st.db).await.unwrap();
    let grace = app.login("grace@example.org").await;
    let (s, u) = scim.patch(&format!("/scim/v2/Users/{uid}"), json!({"Operations": [{"op": "Replace", "path": "active", "value": "False"}]})).await;
    assert_eq!((s, u["active"].as_bool()), (StatusCode::OK, Some(false)), "Entra-style string booleans are understood");
    assert_eq!(grace.get("/api/v1/auth/me").await.0, StatusCode::UNAUTHORIZED, "deactivation ends sessions at once");
    assert!(app.login_with("grace@example.org", PASSWORD).await.is_err());
    let (_, u) = scim.patch(&format!("/scim/v2/Users/{uid}"), json!({"Operations": [{"op": "replace", "value": {"active": true, "displayName": "Rear Admiral Hopper"}}]})).await;
    assert_eq!((u["active"].as_bool(), u["displayName"].as_str()), (Some(true), Some("Rear Admiral Hopper")));
    assert_eq!(scim.delete(&format!("/scim/v2/Users/{uid}")).await.0, StatusCode::NO_CONTENT);
    assert_eq!(scim.ok(&format!("/scim/v2/Users/{uid}")).await["active"], false, "delete deactivates; history is kept");
    assert_eq!(scim.delete(&format!("/scim/v2/Groups/{gid}")).await.0, StatusCode::NO_CONTENT);
    let actions = app.audit_actions().await;
    for a in ["scim.user_create", "scim.user_patch", "scim.user_deactivate", "scim.group_create", "scim.group_patch", "scim.group_delete"] {
        assert!(actions.contains(&a.to_string()), "{a}");
    }
}

// ───────────────────────── MCP ─────────────────────────

struct Mcp {
    caller: Caller,
    modern: bool,
    next_id: std::sync::atomic::AtomicI64,
}

impl Mcp {
    async fn raw(&self, method: &str, mut params: Value) -> (StatusCode, Value) {
        let id = self.next_id.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let mut req = self.caller.request(Method::POST, "/mcp").header("accept", "application/json, text/event-stream");
        if self.modern {
            params["_meta"] = json!({"io.modelcontextprotocol/protocolVersion": "2026-07-28", "io.modelcontextprotocol/clientInfo": {"name": "test", "version": "1"}});
            req = req.header("MCP-Protocol-Version", "2026-07-28").header("Mcp-Method", method);
            if let Some(n) = params["name"].as_str().or(params["uri"].as_str()) {
                req = req.header("Mcp-Name", n);
            }
        }
        let r = req.json(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})).send().await.unwrap();
        let s = r.status();
        (s, r.json().await.unwrap_or(Value::Null))
    }
    async fn call(&self, tool: &str, args: Value) -> Value {
        let (s, v) = self.raw("tools/call", json!({"name": tool, "arguments": args})).await;
        assert_eq!(s, StatusCode::OK, "{v}");
        v["result"].clone()
    }
    async fn ok(&self, tool: &str, args: Value) -> Value {
        let r = self.call(tool, args).await;
        assert_eq!(r["isError"], false, "{tool}: {r}");
        r["structuredContent"].clone()
    }
}

async fn mcp_setup(app: &TestApp, scopes: &[&str]) -> (Caller, Caller) {
    let root = app.admin().await;
    root.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "software"})).await;
    root.created("/api/v1/projects", json!({"key": "HR", "name": "HR", "template": "basic"})).await;
    root.created("/api/v1/projects/HR/items", json!({"title": "Confidential salary review"})).await;
    root.created("/api/v1/spaces", json!({"key": "DOC", "name": "Docs", "access": "internal"})).await;
    let agent = root.created("/api/v1/admin/users", json!({"name": "Planner agent", "kind": "agent"})).await;
    root.created("/api/v1/projects/OPS/members", json!({"user_id": agent["id"], "role": "project_member"})).await;
    root.created("/api/v1/spaces/DOC/members", json!({"user_id": agent["id"], "role": "space_editor"})).await;
    root.agent_rules(agent["id"].as_str().unwrap(), working_agent()).await;
    let tok = root.created(&format!("/api/v1/admin/users/{}/tokens", agent["id"].as_str().unwrap()), json!({"name": "mcp", "scopes": scopes})).await;
    (root, Caller { token: Some(tok["token"].as_str().unwrap().to_string()), ..app.anon() })
}

#[tokio::test]
async fn mcp_modern_protocol_end_to_end() {
    let app = spawn().await;
    let (root, agent) = mcp_setup(&app, &["read", "write"]).await;
    let mcp = Mcp { caller: agent, modern: true, next_id: 1.into() };

    let (s, d) = mcp.raw("server/discover", json!({})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(d["result"]["supportedVersions"][0], "2026-07-28");
    assert_eq!(d["result"]["resultType"], "complete");
    assert_eq!(d["result"]["serverInfo"]["name"], "gantry");
    let (_, tools) = mcp.raw("tools/list", json!({})).await;
    assert!(tools["result"]["tools"].as_array().unwrap().len() >= 15);
    // The list depends on the caller's rules, so a client must not share it between users.
    assert_eq!((tools["result"]["ttlMs"].as_i64(), tools["result"]["cacheScope"].as_str()), (Some(60000), Some("private")));
    assert_eq!(tools["result"]["_meta"]["io.modelcontextprotocol/serverInfo"]["name"], "gantry");

    // Learn the project, create, update, transition, comment, link.
    let project = mcp.ok("get_project", json!({"project": "OPS"})).await;
    assert!(project["types"].as_array().unwrap().iter().any(|t| t["key"] == "bug" && t["workflow"]["initial"] == "backlog"));
    let created = mcp.ok("create_work_item", json!({"project": "OPS", "title": "Investigate vibration on P-101", "type": "bug", "priority": 3, "custom": {"severity": "Major"}})).await;
    assert_eq!(created["key"], "OPS-1");
    mcp.ok("update_work_item", json!({"key": "OPS-1", "changes": {"labels": ["vibration"], "description": "Seen at 1450 rpm."}})).await;
    let moved = mcp.ok("transition_work_item", json!({"key": "OPS-1", "to": "in_progress"})).await;
    assert_eq!(moved["status"], "in_progress");
    mcp.ok("add_comment", json!({"key": "OPS-1", "body": "Bearing temperature is normal."})).await;
    mcp.ok("create_work_item", json!({"project": "OPS", "title": "Balance the impeller"})).await;
    mcp.ok("link_work_items", json!({"key": "OPS-2", "rel": "blocks", "target_key": "OPS-1"})).await;
    let item = mcp.ok("get_work_item", json!({"key": "OPS-1"})).await;
    assert_eq!((item["comments"].as_array().unwrap().len(), item["links"][0]["other"]["key"].as_str(), item["reporter"]["name"].as_str()), (1, Some("OPS-2"), Some("Planner agent")));
    assert_eq!(item["channel"], "mcp");
    assert_eq!(mcp.ok("get_item_graph", json!({"key": "OPS-1"})).await["edges"].as_array().unwrap().len(), 1);
    assert_eq!(mcp.ok("list_work_items", json!({"project": "OPS", "category": "in_progress"})).await["total"], 1);

    // Pages, with optimistic concurrency.
    let page = mcp.ok("create_page", json!({"space": "DOC", "title": "Vibration findings", "content": "Initial notes."})).await;
    let pid = page["id"].as_str().unwrap();
    let stale = mcp.call("update_page", json!({"id": pid, "base_version": 9, "content": "x"})).await;
    assert_eq!(stale["isError"], true);
    assert!(stale["content"][0]["text"].as_str().unwrap().contains("conflict"));
    assert_eq!(mcp.ok("update_page", json!({"id": pid, "base_version": 1, "content": "Impeller imbalance confirmed.", "message": "Agent update"})).await["version"], 2);
    assert_eq!(mcp.ok("get_page", json!({"id": pid})).await["content"], "Impeller imbalance confirmed.");
    let hits = mcp.ok("search", json!({"query": "impeller"})).await;
    assert!(hits["results"].as_array().unwrap().len() >= 2);

    // Resources.
    let (s, r) = mcp.raw("resources/read", json!({"uri": "gantry://item/OPS-1"})).await;
    assert_eq!(s, StatusCode::OK);
    assert!(r["result"]["contents"][0]["text"].as_str().unwrap().starts_with("# OPS-1 Investigate vibration on P-101"));
    assert_eq!(mcp.raw("resources/read", json!({"uri": "gantry://item/HR-1"})).await.0, StatusCode::NOT_FOUND);

    // The agent's permissions are the boundary: no HR, no workflow shortcuts; errors are readable results.
    assert!(!mcp.ok("list_projects", json!({})).await.to_string().contains("HR"));
    assert!(!mcp.ok("search", json!({"query": "salary"})).await.to_string().contains("Confidential"));
    for (tool, args) in [("get_work_item", json!({"key": "HR-1"})), ("add_comment", json!({"key": "HR-1", "body": "x"})), ("create_work_item", json!({"project": "HR", "title": "x"})), ("link_work_items", json!({"key": "OPS-1", "rel": "relates_to", "target_key": "HR-1"}))] {
        let r = mcp.call(tool, args).await;
        assert_eq!(r["isError"], true, "{tool}");
        assert!(!r.to_string().contains("salary"), "{tool} leaked: {r}");
    }
    let r = mcp.call("transition_work_item", json!({"key": "OPS-1", "to": "nonexistent"})).await;
    assert!(r["isError"] == true && r["content"][0]["text"].as_str().unwrap().contains("allowed next statuses"), "the error tells the agent how to recover: {r}");
    assert_eq!(mcp.call("create_work_item", json!({"project": "OPS", "title": "x", "bogus": 1})).await["isError"], true);

    // Protocol errors.
    assert_eq!(mcp.raw("tools/call", json!({"name": "format_disk", "arguments": {}})).await.1["error"]["code"], -32602);
    let (s, v) = mcp.raw("no/such/method", json!({})).await;
    assert_eq!((s, v["error"]["code"].as_i64()), (StatusCode::NOT_FOUND, Some(-32601)));
    let mismatch = mcp.caller.request(Method::POST, "/mcp").header("MCP-Protocol-Version", "2026-07-28").header("Mcp-Method", "tools/call").header("Mcp-Name", "search")
        .json(&json!({"jsonrpc": "2.0", "id": 99, "method": "tools/call", "params": {"name": "create_work_item", "arguments": {"project": "OPS", "title": "smuggled"}, "_meta": {"io.modelcontextprotocol/protocolVersion": "2026-07-28"}}})).send().await.unwrap();
    assert_eq!(mismatch.status(), StatusCode::BAD_REQUEST);
    assert_eq!(mismatch.json::<Value>().await.unwrap()["error"]["code"], -32020, "a header that names a different tool than the body is refused");
    let future = mcp.caller.request(Method::POST, "/mcp").header("MCP-Protocol-Version", "2031-01-01").header("Mcp-Method", "tools/list")
        .json(&json!({"jsonrpc": "2.0", "id": 100, "method": "tools/list", "params": {"_meta": {"io.modelcontextprotocol/protocolVersion": "2031-01-01"}}})).send().await.unwrap();
    let v: Value = future.json().await.unwrap();
    assert_eq!((v["error"]["code"].as_i64(), v["error"]["data"]["supported"][0].as_str()), (Some(-32022), Some("2026-07-28")));
    assert_eq!(mcp.caller.request(Method::GET, "/mcp").send().await.unwrap().status(), StatusCode::METHOD_NOT_ALLOWED);
    assert_eq!(root.ok("/api/v1/items?project=OPS").await["total"], 2, "nothing was smuggled in");

    // Every tool call is in the audit log under the agent's name, without argument values.
    let rows: Vec<(String, serde_json::Value)> = sqlx::query_as("SELECT target_id, detail FROM audit_log WHERE action = 'mcp.tool_call' ORDER BY id").fetch_all(&app.st.db).await.unwrap();
    assert!(rows.len() >= 20);
    assert!(rows.iter().any(|(t, d)| t == "update_page" && d["ok"] == false));
    assert!(!serde_json::to_string(&rows.iter().map(|r| &r.1).collect::<Vec<_>>()).unwrap().contains("Impeller imbalance"), "page bodies are not copied into the audit log");
}

#[tokio::test]
async fn mcp_legacy_handshake_and_read_only_tokens() {
    let app = spawn().await;
    let (_root, agent) = mcp_setup(&app, &["read"]).await;
    let mcp = Mcp { caller: agent, modern: false, next_id: 1.into() };
    let (s, init) = mcp.raw("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "old-client", "version": "1"}})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(init["result"]["protocolVersion"], "2025-06-18");
    assert!(init["result"].get("resultType").is_none(), "legacy results are not decorated");
    assert_eq!(mcp.raw("initialize", json!({"protocolVersion": "1999-01-01"})).await.1["result"]["protocolVersion"], "2025-11-25", "unknown versions negotiate down to the newest legacy one");
    let note = mcp.caller.request(Method::POST, "/mcp").json(&json!({"jsonrpc": "2.0", "method": "notifications/initialized"})).send().await.unwrap();
    assert_eq!(note.status(), StatusCode::ACCEPTED);
    assert_eq!(mcp.raw("ping", json!({})).await.1["result"], json!({}));
    // This token can only read, so only the reading tools are offered.
    let offered: Vec<String> = mcp.raw("tools/list", json!({})).await.1["result"]["tools"].as_array().unwrap().iter().map(|t| t["name"].as_str().unwrap().to_string()).collect();
    assert!(offered.contains(&"search".to_string()) && offered.contains(&"get_work_item".to_string()) && offered.contains(&"get_page".to_string()), "{offered:?}");
    assert!(!offered.iter().any(|t| t.starts_with("create_") || t.starts_with("update_") || t == "add_comment" || t == "transition_work_item" || t == "link_work_items"), "{offered:?}");
    let (s, v) = mcp.raw("no/such/method", json!({})).await;
    assert_eq!((s, v["error"]["code"].as_i64()), (StatusCode::OK, Some(-32601)));

    // A read-only token can read through MCP but every writing tool is refused.
    assert_eq!(mcp.ok("list_projects", json!({})).await.as_array().unwrap().len(), 1);
    for (tool, args) in [("create_work_item", json!({"project": "OPS", "title": "x"})), ("create_page", json!({"space": "DOC", "title": "x"}))] {
        let r = mcp.call(tool, args).await;
        assert_eq!(r["isError"], true, "{tool}");
        assert!(r["content"][0]["text"].as_str().unwrap().contains("forbidden"), "{r}");
    }
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM items WHERE channel = 'mcp'").fetch_one(&app.st.db).await.unwrap(), 0);

    // Browsers on other sites cannot drive the endpoint, with or without credentials.
    let r = mcp.caller.request(Method::POST, "/mcp").header("origin", "https://evil.example").json(&json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"})).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert!(app.security_kinds().await.contains(&"mcp.origin_rejected".to_string()));
    let r = mcp.caller.request(Method::POST, "/mcp").header("content-type", "application/json").body("{nope").send().await.unwrap();
    assert_eq!(r.json::<Value>().await.unwrap()["error"]["code"], -32700);
}
