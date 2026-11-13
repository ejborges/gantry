//! Working together: the discussion on work items and requests, references and mentions,
//! the complete history of a work item, page workflows, and the real-time connection with
//! presence and concurrent editing.

mod common;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use common::*;
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

async fn team(app: &TestApp) -> (Caller, Caller, Caller) {
    let ada = app.user("ada@example.org", "member").await;
    let ben = app.user("ben@example.org", "member").await;
    let eve = app.user("eve@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "software"})).await;
    ada.created("/api/v1/projects/OPS/members", json!({"user_id": ben.user_id, "role": "project_member"})).await;
    (ada, ben, eve)
}

// ───────────────────────── discussion ─────────────────────────

#[tokio::test]
async fn references_link_only_what_both_author_and_viewer_may_see() {
    let app = spawn().await;
    let (ada, ben, eve) = team(&app).await;
    // Eve has her own private project and a private space; Ada and Ben cannot see them.
    eve.created("/api/v1/projects", json!({"key": "SEC", "name": "Secret", "template": "basic"})).await;
    eve.created("/api/v1/projects/SEC/items", json!({"title": "Sealed bid figures"})).await;
    // Ada has a private space that Ben cannot see, and a second project only she is in.
    ada.created("/api/v1/spaces", json!({"key": "ENG", "name": "Engineering", "access": "internal"})).await;
    ada.created("/api/v1/spaces/ENG/pages", json!({"title": "Pump runbook", "content": "Steps."})).await;
    ada.created("/api/v1/projects", json!({"key": "HR", "name": "People", "template": "basic"})).await;
    ada.created("/api/v1/projects/HR/items", json!({"title": "Salary review"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Replace pump seal", "type": "task"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Order spare seals", "type": "task"})).await;

    let body = "See OPS-2 and HR-1, also SEC-1 and NOPE-9. Ask @ben or @eve. Docs: [[Pump runbook]] and [[HR]] and [[No such page]].\n\n`OPS-2 in code is not a link` and neither is ada@example.org.";
    let c = ada.created("/api/v1/items/OPS-1/comments", json!({"body": body})).await;
    // Ada sees links to everything she could see when she wrote it: not Eve's project.
    assert_eq!(c["refs"]["items"].as_object().unwrap().keys().collect::<Vec<_>>(), vec!["HR-1", "OPS-2"]);
    assert_eq!(c["refs"]["items"]["OPS-2"]["title"], "Order spare seals");
    assert_eq!(c["refs"]["items"]["OPS-2"]["url"], "/i/OPS-2");
    assert_eq!(c["refs"]["users"].as_object().unwrap().keys().collect::<Vec<_>>(), vec!["ben", "eve"]);
    assert_eq!(c["refs"]["users"]["ben"]["kind"], "human");
    assert_eq!(c["refs"]["pages"]["Pump runbook"]["title"], "Pump runbook");
    assert_eq!(c["refs"]["projects"]["HR"]["name"], "People");
    assert_eq!(c["body"], body, "the text itself is stored as written");

    // Ben reads the same message: HR-1 and the HR project are not links for him, and he learns nothing about them.
    let seen = ben.ok("/api/v1/items/OPS-1").await["comments"][0].clone();
    assert_eq!(seen["refs"]["items"].as_object().unwrap().keys().collect::<Vec<_>>(), vec!["OPS-2"]);
    assert!(seen["refs"].get("projects").is_none(), "{}", seen["refs"]);
    assert!(!seen["refs"].to_string().contains("Salary"));
    assert!(seen["refs"]["pages"]["Pump runbook"].is_object(), "the space is internal, so Ben can follow the page link");

    // Eve can see SEC-1, but Ada could not when she wrote the message, so it never became a link.
    ada.created("/api/v1/projects/OPS/members", json!({"user_id": eve.user_id, "role": "project_viewer"})).await;
    let seen = eve.ok("/api/v1/items/OPS-1").await["comments"][0].clone();
    assert!(seen["refs"]["items"].get("SEC-1").is_none(), "{}", seen["refs"]);
    // Eve writes about her own item: she sees the link, Ada and Ben do not.
    assert_eq!(eve.post("/api/v1/items/OPS-1/comments", json!({"body": "x"})).await.0, StatusCode::FORBIDDEN, "viewers do not post");
    ada.created("/api/v1/projects/OPS/members", json!({"user_id": eve.user_id, "role": "project_member"})).await;
    let c = eve.created("/api/v1/items/OPS-1/comments", json!({"body": "Related to SEC-1."})).await;
    assert_eq!(c["refs"]["items"]["SEC-1"]["title"], "Sealed bid figures");
    let seen = ada.ok("/api/v1/items/OPS-1").await["comments"][1].clone();
    assert_eq!(seen["refs"], json!({}), "{seen}");
    assert_eq!(seen["body"], "Related to SEC-1.", "the key is visible as text, exactly as the author typed it");

    // The description resolves the same way, and the preview endpoint answers for a draft.
    ada.patch("/api/v1/items/OPS-1", json!({"description": "Blocked by HR-1; see [[OPS-2]]."})).await;
    assert_eq!(ada.ok("/api/v1/items/OPS-1").await["description_refs"]["items"].as_object().unwrap().len(), 2);
    assert_eq!(ben.ok("/api/v1/items/OPS-1").await["description_refs"]["items"].as_object().unwrap().len(), 1);
    let p = ben.post("/api/v1/refs/preview", json!({"text": "HR-1 OPS-2 @ada [[Pump runbook]]"})).await.1;
    assert_eq!(p["items"].as_object().unwrap().keys().collect::<Vec<_>>(), vec!["OPS-2"]);
    assert_eq!(p["users"]["ada"]["name"], "ada");
    assert_eq!(app.anon().post("/api/v1/refs/preview", json!({"text": "OPS-2"})).await.0, StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn messages_can_be_edited_and_removed_and_internal_notes_stay_internal() {
    let app = spawn().await;
    let admin = app.admin().await;
    admin.created("/api/v1/projects", json!({"key": "IT", "name": "IT help", "template": "it-service-desk"})).await;
    let agent = app.user("agent@example.org", "member").await;
    let other = app.user("other@example.org", "member").await;
    let customer = app.user("cust@example.org", "customer").await;
    admin.created("/api/v1/projects/IT/members", json!({"user_id": agent.user_id, "role": "service_agent"})).await;
    admin.created("/api/v1/projects/IT/members", json!({"user_id": other.user_id, "role": "project_member"})).await;
    let rt = customer.ok("/api/v1/portal/desks/IT").await["request_types"][0]["id"].as_str().unwrap().to_string();
    let req = customer.created("/api/v1/portal/desks/IT/requests", json!({"request_type_id": rt, "summary": "Laptop will not start", "description": "Black screen."})).await;
    let key = req["key"].as_str().unwrap().to_string();

    let public = agent.created(&format!("/api/v1/items/{key}/comments"), json!({"body": "Have you tried holding the power button? @other is our hardware person."})).await;
    let note = agent.created(&format!("/api/v1/items/{key}/comments"), json!({"body": "Third one this week from this batch. See IT-1.", "internal": true})).await;
    let (pid, nid) = (public["id"].as_str().unwrap().to_string(), note["id"].as_str().unwrap().to_string());
    assert_eq!(note["internal"], true);

    // The customer sees the reply, not the note, and only the people taking part in the request.
    let seen = customer.ok(&format!("/api/v1/portal/requests/{key}")).await;
    assert_eq!(seen["comments"].as_array().unwrap().len(), 1);
    assert_eq!(seen["comments"][0]["author"]["name"], "agent");
    assert!(seen["comments"][0]["refs"].get("users").is_none(), "someone who has not taken part is not looked up for a customer: {}", seen["comments"][0]["refs"]);
    assert!(!seen.to_string().contains("this batch"));
    // A project member who is not a desk agent does not see internal notes either.
    assert_eq!(other.ok(&format!("/api/v1/items/{key}")).await["comments"].as_array().unwrap().len(), 1);
    assert_eq!(other.ok(&format!("/api/v1/items/{key}/comments")).await.as_array().unwrap().len(), 1);
    assert_eq!(agent.ok(&format!("/api/v1/items/{key}/comments")).await.as_array().unwrap().len(), 2);
    // Nor can they or the customer touch the note by its id.
    for c in [&other, &customer] {
        assert_eq!(c.patch(&format!("/api/v1/comments/{nid}"), json!({"body": "x"})).await.0, StatusCode::NOT_FOUND);
        assert_eq!(c.send(Method::DELETE, &format!("/api/v1/comments/{nid}"), None).await.0, StatusCode::NOT_FOUND);
    }

    // The customer replies; the reply puts the request back in front of the agent, with the customer marked as author.
    let reply = customer.created(&format!("/api/v1/portal/requests/{key}/comments"), json!({"body": "Tried it, still dead."})).await;
    assert_eq!(reply["mine"], true);
    let rid = reply["id"].as_str().unwrap().to_string();
    let notes = agent.ok("/api/v1/notifications").await;
    assert!(notes.as_array().unwrap().iter().any(|n| n["category"] == "desk_new"), "new request: {notes}");
    assert!(notes.as_array().unwrap().iter().any(|n| n["category"] == "comment" && n["title"].as_str().unwrap().contains("replied")), "{notes}");
    // The customer was told about the agent's public reply, with a link into the portal, and never about the note.
    let cnotes = customer.ok("/api/v1/notifications").await;
    assert_eq!(cnotes.as_array().unwrap().len(), 1, "{cnotes}");
    assert_eq!(cnotes[0]["link"], format!("/portal/requests/{key}"));

    // Editing: one's own message only; it is marked as edited.
    assert_eq!(other.patch(&format!("/api/v1/comments/{pid}"), json!({"body": "Tampered"})).await.0, StatusCode::FORBIDDEN);
    let edited = agent.patch(&format!("/api/v1/comments/{pid}"), json!({"body": "Please hold the power button for ten seconds."})).await.1;
    assert!(edited["edited_at"].is_string());
    assert_eq!(edited["body"], "Please hold the power button for ten seconds.");
    assert_eq!(customer.patch(&format!("/api/v1/comments/{rid}"), json!({"body": "Tried it twice, still dead."})).await.0, StatusCode::OK);
    assert_eq!(agent.patch(&format!("/api/v1/comments/{pid}"), json!({"body": "   "})).await.0, StatusCode::BAD_REQUEST);

    // Removing: authors remove their own; a project administrator may remove anyone's; the place stays visible.
    assert_eq!(other.send(Method::DELETE, &format!("/api/v1/comments/{pid}"), None).await.0, StatusCode::FORBIDDEN);
    assert_eq!(admin.send(Method::DELETE, &format!("/api/v1/comments/{rid}"), None).await.0, StatusCode::NO_CONTENT);
    let after = agent.ok(&format!("/api/v1/items/{key}/comments")).await;
    let removed = after.as_array().unwrap().iter().find(|c| c["id"] == rid.as_str()).unwrap();
    assert_eq!((removed["deleted"].as_bool(), removed["body"].as_str()), (Some(true), Some("")));
    let kept: String = sqlx::query_scalar("SELECT body FROM comments WHERE id = $1::uuid").bind(&rid).fetch_one(&app.st.db).await.unwrap();
    assert_eq!(kept, "(removed)", "the text is erased from the database, not just hidden");
    assert_eq!(customer.patch(&format!("/api/v1/comments/{rid}"), json!({"body": "back"})).await.0, StatusCode::BAD_REQUEST, "a removed message cannot be edited back");
    // Search no longer finds the removed text.
    assert!(agent.ok("/api/v1/search?q=dead").await["results"].as_array().unwrap().is_empty());
    for a in ["comment.create", "comment.edit", "comment.delete", "request.comment"] {
        assert!(app.audit_actions().await.contains(&a.to_string()), "{a}");
    }
}

#[tokio::test]
async fn the_history_records_every_change_with_who_and_when() {
    let app = spawn().await;
    let (ada, ben, _) = team(&app).await;
    let cycle = ada.created("/api/v1/projects/OPS/cycles", json!({"name": "Sprint 7"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Parent", "type": "epic"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Flaky valve", "type": "bug", "custom": {"severity": "Minor"}})).await;
    ada.patch("/api/v1/items/OPS-2", json!({"title": "Flaky valve on line 3", "description": "It sticks when cold.", "priority": 4, "assignee_id": ben.user_id, "labels": ["hardware", "line-3"],
        "start_date": "2030-01-10", "due_date": "2030-01-31", "estimate": 3, "cycle_id": cycle["id"], "parent": "OPS-1", "custom": {"severity": "Major"}})).await;
    ben.patch("/api/v1/items/OPS-2", json!({"due_date": "2030-02-15", "description": "It sticks when cold. Heater tape helps.", "assignee_id": null})).await;
    let to = ben.ok("/api/v1/items/OPS-2").await["transitions"][0]["to"].as_str().unwrap().to_string();
    ben.post("/api/v1/items/OPS-2/transition", json!({"to": to})).await;
    let c = ben.created("/api/v1/items/OPS-2/comments", json!({"body": "Tape fitted."})).await;
    ben.patch(&format!("/api/v1/comments/{}", c["id"].as_str().unwrap()), json!({"body": "Tape fitted on both sides."})).await;
    ben.upload(Method::POST, "/api/v1/items/OPS-2/attachments?filename=photo.png", "image/png", vec![0x89, 0x50, 0x4e, 0x47]).await;
    ada.created("/api/v1/items/OPS-2/links", json!({"rel": "relates_to", "target": "OPS-1"})).await;
    // Sending the same value again is not a change.
    ada.patch("/api/v1/items/OPS-2", json!({"priority": 4, "title": "Flaky valve on line 3"})).await;

    let h = ada.ok("/api/v1/items/OPS-2/history?limit=200").await;
    let entries = h["entries"].as_array().unwrap();
    assert_eq!(h["total"].as_i64().unwrap() as usize, entries.len());
    let find = |field: &str| entries.iter().filter(|e| e["field"] == field).cloned().collect::<Vec<_>>();
    let one = |field: &str| {
        let v = find(field);
        assert_eq!(v.len(), 1, "{field}: {v:?}");
        v[0].clone()
    };
    assert_eq!(one("created")["actor"]["name"], "ada");
    assert_eq!((one("title")["old"].as_str(), one("title")["new"].as_str()), (Some("Flaky valve"), Some("Flaky valve on line 3")));
    assert_eq!((one("priority")["old"].as_i64(), one("priority")["new"].as_i64()), (Some(2), Some(4)));
    assert_eq!(one("labels")["new"], json!(["hardware", "line-3"]));
    assert_eq!((one("start_date")["old"].clone(), one("start_date")["new"].as_str()), (Value::Null, Some("2030-01-10")));
    assert_eq!(one("estimate")["new"], 3.0);
    assert_eq!(one("cycle")["new"]["name"], "Sprint 7");
    assert_eq!(one("parent")["new"]["key"], "OPS-1");
    // A custom field is named the way the form names it.
    let sev = one("custom.severity");
    assert_eq!((sev["old"].as_str(), sev["new"].as_str(), sev["field_name"].as_str()), (Some("Minor"), Some("Major"), Some("Severity")));
    // Two due-date changes by two people, newest first, each with the value before and after.
    let due = find("due_date");
    assert_eq!(due.len(), 2);
    assert_eq!((due[0]["actor"]["name"].as_str(), due[0]["old"].as_str(), due[0]["new"].as_str()), (Some("ben"), Some("2030-01-31"), Some("2030-02-15")));
    assert_eq!(due[1]["actor"]["name"], "ada");
    // The description keeps both texts so the difference can be shown.
    let desc = find("description");
    assert_eq!((desc[0]["old"].as_str(), desc[0]["new"].as_str()), (Some("It sticks when cold."), Some("It sticks when cold. Heater tape helps.")));
    // Assigned, then unassigned: with the name as it was at the time.
    let assign = find("assignee");
    assert_eq!((assign[1]["old"].clone(), assign[1]["new"]["name"].as_str()), (Value::Null, Some("ben")));
    assert_eq!((assign[0]["old"]["name"].as_str(), assign[0]["new"].clone()), (Some("ben"), Value::Null));
    assert_eq!(one("status")["new"]["key"], to.as_str());
    assert!(one("status")["new"]["name"].is_string());
    let comments: Vec<String> = find("comment").iter().map(|e| e["new"]["action"].as_str().unwrap().to_string()).collect();
    assert_eq!(comments, vec!["edited", "added"]);
    assert_eq!(one("attachment")["new"], "photo.png");
    assert_eq!(one("link")["new"]["rel"], "relates_to");
    for e in entries {
        assert!(e["at"].is_string() && e["actor"]["id"].is_string(), "every entry says who and when: {e}");
    }
    // Paging.
    let page = ada.ok("/api/v1/items/OPS-2/history?limit=5&offset=5").await;
    assert_eq!(page["entries"].as_array().unwrap().len(), 5);
    assert_eq!(page["entries"][0], entries[5]);
    // The item itself carries the latest entries and the total.
    let item = ada.ok("/api/v1/items/OPS-2").await;
    assert_eq!(item["history_total"], h["total"]);
    assert_eq!(item["cycle"]["name"], "Sprint 7");
}

#[tokio::test]
async fn a_project_can_have_its_own_workflow_for_a_type() {
    let app = spawn().await;
    let (ada, ben, _) = team(&app).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "A task", "type": "task"})).await;
    let types = ada.ok("/api/v1/projects/OPS").await["types"].clone();
    let task = types.as_array().unwrap().iter().find(|t| t["key"] == "task").unwrap().clone();
    assert_eq!(task["workflow"]["own"], false);
    let start = task["workflow"]["initial"].as_str().unwrap().to_string();

    // A workflow with a review stage and a branch back, drawn by the project administrator.
    let own = json!({"initial": "queued", "statuses": [
            {"key": "queued", "name": "Queued", "category": "todo"}, {"key": "building", "name": "Building", "category": "in_progress"},
            {"key": "review", "name": "In review", "category": "in_progress"}, {"key": "shipped", "name": "Shipped", "category": "done"}],
        "transitions": [
            {"name": "Start", "from": ["queued"], "to": "building"}, {"name": "Ask for review", "from": ["building"], "to": "review"},
            {"name": "Send back", "from": ["review"], "to": "building"}, {"name": "Ship", "from": ["review"], "to": "shipped"}],
        "layout": {"queued": {"x": 0, "y": 0}, "building": {"x": 220, "y": 0}, "review": {"x": 440, "y": 0}, "shipped": {"x": 660, "y": 0}}});
    // The existing item is in a status the new workflow lacks: the change is refused until told where it goes.
    let (s, v) = ada.put("/api/v1/projects/OPS/workflows/task", json!({"definition": own})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains(&start), "{v}");
    assert_eq!(ada.put("/api/v1/projects/OPS/workflows/task", json!({"definition": own, "mapping": {&start: "nowhere"}})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ben.put("/api/v1/projects/OPS/workflows/task", json!({"definition": own, "mapping": {&start: "queued"}})).await.0, StatusCode::FORBIDDEN, "members do not redesign the workflow");
    let (s, v) = ada.put("/api/v1/projects/OPS/workflows/task", json!({"definition": own, "mapping": {&start: "queued"}})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    let task = v["types"].as_array().unwrap().iter().find(|t| t["key"] == "task").unwrap().clone();
    assert_eq!((task["workflow"]["own"].as_bool(), task["workflow"]["layout"]["review"]["x"].as_i64()), (Some(true), Some(440)));

    // The item moved with the workflow, its history says why, and it now follows the new arrows.
    let item = ben.ok("/api/v1/items/OPS-1").await;
    assert_eq!((item["status"].as_str(), item["status_name"].as_str()), (Some("queued"), Some("Queued")));
    assert!(item["history"].as_array().unwrap().iter().any(|e| e["field"] == "status" && e["old"]["reason"] == "workflow changed"));
    assert_eq!(item["transitions"].as_array().unwrap().iter().map(|t| t["to"].as_str().unwrap()).collect::<Vec<_>>(), vec!["building"]);
    assert_eq!(ben.post("/api/v1/items/OPS-1/transition", json!({"to": "shipped"})).await.0, StatusCode::BAD_REQUEST, "no shortcut past review");
    for to in ["building", "review", "building", "review", "shipped"] {
        assert_eq!(ben.post("/api/v1/items/OPS-1/transition", json!({"to": to})).await.0, StatusCode::OK, "{to}");
    }
    assert_eq!(ben.ok("/api/v1/items/OPS-1").await["status_category"], "done");
    // Other types in the project, and the same type in other projects, are untouched.
    ada.created("/api/v1/projects", json!({"key": "ENG", "name": "Eng", "template": "software"})).await;
    let other = ada.created("/api/v1/projects/ENG/items", json!({"title": "Elsewhere", "type": "task"})).await;
    assert_eq!(other["status"], start.as_str());
    assert_eq!(ada.created("/api/v1/projects/OPS/items", json!({"title": "New", "type": "task"})).await["status"], "queued");
    assert_eq!(ada.created("/api/v1/projects/OPS/items", json!({"title": "A bug", "type": "bug", "custom": {"severity": "Minor"}})).await["status_name"].as_str().is_some(), true);
    // The search form offers the project's own statuses.
    let fields = ben.ok("/api/v1/items/query/fields").await;
    assert!(fields["values"]["status"].as_array().unwrap().iter().any(|s| s["value"] == "review" && s["label"] == "In review"));

    // Invalid designs are refused with a reason.
    for (bad, why) in [
        (json!({"initial": "a", "statuses": [{"key": "a", "name": "A", "category": "todo"}], "transitions": []}), "done"),
        (json!({"initial": "a", "statuses": [{"key": "a", "name": "A", "category": "todo"}, {"key": "b", "name": "B", "category": "done"}], "transitions": []}), "cannot be reached"),
        (json!({"initial": "zz", "statuses": [{"key": "a", "name": "A", "category": "done"}], "transitions": []}), "initial"),
        (json!({"initial": "a", "statuses": [{"key": "a", "name": "A", "category": "done"}], "transitions": [{"name": "x", "from": ["a"], "to": "nope"}]}), "unknown status"),
        (json!({"initial": "a", "statuses": [{"key": "a", "name": "A", "category": "done"}], "transitions": [{"name": "x", "from": ["*"], "to": "a", "requires": ["no_such_field"]}]}), "not a field"),
    ] {
        let (s, v) = ada.put("/api/v1/projects/OPS/workflows/bug", json!({"definition": bad})).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
        assert!(v["detail"].as_str().unwrap().contains(why), "{v}");
    }
    assert_eq!(ada.put("/api/v1/projects/OPS/workflows/no_such_type", json!({"definition": own})).await.0, StatusCode::NOT_FOUND);
    // Going back to the shared workflow needs the same care for items in the project's own statuses.
    let (s, _) = ada.put("/api/v1/projects/OPS/workflows/task", json!({"definition": null})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let done = task["workflow"]["statuses"].as_array().unwrap().len();
    assert_eq!(done, 4);
    let shared_done = types.as_array().unwrap().iter().find(|t| t["key"] == "task").unwrap()["workflow"]["statuses"].as_array().unwrap().iter().find(|s| s["category"] == "done").unwrap()["key"].as_str().unwrap().to_string();
    let (s, v) = ada.put("/api/v1/projects/OPS/workflows/task", json!({"definition": null, "mapping": {"queued": start, "shipped": shared_done}})).await;
    assert_eq!(s, StatusCode::OK, "{v}");
    assert_eq!(ada.ok("/api/v1/items/OPS-1").await["status"], shared_done.as_str());
    assert!(app.audit_actions().await.contains(&"project.workflow_update".to_string()));
}

// ───────────────────────── pages ─────────────────────────

#[tokio::test]
async fn a_space_can_define_the_stages_a_page_goes_through() {
    let app = spawn().await;
    let ada = app.user("ada@example.org", "member").await;
    let ben = app.user("ben@example.org", "member").await;
    let reader = app.user("reader@example.org", "member").await;
    ada.created("/api/v1/spaces", json!({"key": "QA", "name": "Quality", "access": "internal"})).await;
    ada.created("/api/v1/spaces/QA/members", json!({"user_id": ben.user_id, "role": "space_editor"})).await;
    let old = ada.created("/api/v1/spaces/QA/pages", json!({"title": "Existing procedure", "content": "v1"})).await;
    let workflow = json!({"initial": "draft", "statuses": [
            {"key": "draft", "name": "Draft", "category": "todo"}, {"key": "review", "name": "In review", "category": "in_progress"}, {"key": "approved", "name": "Approved", "category": "done"}],
        "transitions": [{"name": "Submit for review", "from": ["draft"], "to": "review"}, {"name": "Approve", "from": ["review"], "to": "approved"},
                        {"name": "Request changes", "from": ["review"], "to": "draft"}, {"name": "Revise", "from": ["approved"], "to": "draft"}]});
    assert_eq!(ben.patch("/api/v1/spaces/QA", json!({"workflow": workflow})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(ada.patch("/api/v1/spaces/QA", json!({"workflow": workflow})).await.0, StatusCode::NO_CONTENT);
    // A page that was already published sits at the publishing stage.
    let p = ada.ok(&format!("/api/v1/pages/{}", old["id"].as_str().unwrap())).await;
    assert_eq!((p["stage"]["key"].as_str(), p["status"].as_str()), (Some("approved"), Some("published")));

    // New pages start as drafts at the first stage, whatever the request says.
    assert_eq!(ben.post("/api/v1/spaces/QA/pages", json!({"title": "New procedure", "status": "published"})).await.0, StatusCode::BAD_REQUEST);
    let page = ben.created("/api/v1/spaces/QA/pages", json!({"title": "New procedure", "content": "Step one."})).await;
    let id = page["id"].as_str().unwrap().to_string();
    assert_eq!((page["status"].as_str(), page["stage"]["name"].as_str()), (Some("draft"), Some("Draft")));
    assert_eq!(page["stage"]["next"][0]["name"], "Submit for review");
    assert_eq!(reader.get(&format!("/api/v1/pages/{id}")).await.0, StatusCode::NOT_FOUND, "readers do not see drafts");
    // It follows the arrows: no publishing by setting the status, no skipping review.
    assert_eq!(ben.patch(&format!("/api/v1/pages/{id}"), json!({"status": "published"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ben.patch(&format!("/api/v1/pages/{id}"), json!({"stage": "approved"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ben.patch(&format!("/api/v1/pages/{id}"), json!({"stage": "review"})).await.0, StatusCode::OK);
    assert_eq!(reader.get(&format!("/api/v1/pages/{id}")).await.0, StatusCode::NOT_FOUND, "in review is still not published");
    let p = ben.patch(&format!("/api/v1/pages/{id}"), json!({"stage": "approved"})).await.1;
    assert_eq!((p["status"].as_str(), p["stage"]["key"].as_str()), (Some("published"), Some("approved")));
    assert_eq!(reader.ok(&format!("/api/v1/pages/{id}")).await["title"], "New procedure");
    // Revising takes it out of view again until it is approved once more.
    assert_eq!(ben.patch(&format!("/api/v1/pages/{id}"), json!({"stage": "draft"})).await.0, StatusCode::OK);
    assert_eq!(reader.get(&format!("/api/v1/pages/{id}")).await.0, StatusCode::NOT_FOUND);
    // The space administrator may place a page at any stage directly.
    assert_eq!(ada.patch(&format!("/api/v1/pages/{id}"), json!({"stage": "approved"})).await.0, StatusCode::OK);
    // Removing the workflow leaves pages as they are: published stay published.
    assert_eq!(ada.patch("/api/v1/spaces/QA", json!({"workflow": null})).await.0, StatusCode::NO_CONTENT);
    let p = reader.ok(&format!("/api/v1/pages/{id}")).await;
    assert_eq!((p["status"].as_str(), p["stage"].clone()), (Some("published"), Value::Null));
    assert_eq!(ben.patch(&format!("/api/v1/pages/{id}"), json!({"stage": "draft"})).await.0, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn mentions_and_links_on_pages() {
    let app = spawn().await;
    let (ada, ben, eve) = team(&app).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Replace pump seal"})).await;
    ada.created("/api/v1/spaces", json!({"key": "ENG", "name": "Engineering"})).await;
    ada.created("/api/v1/spaces/ENG/members", json!({"user_id": ben.user_id, "role": "space_viewer"})).await;
    ada.created("/api/v1/spaces/ENG/pages", json!({"title": "Torque table", "content": "Values."})).await;
    // A draft mentions people: nobody is told yet.
    let page = ada.created("/api/v1/spaces/ENG/pages", json!({"title": "Seal procedure", "status": "draft", "content": "Owner: @ben. Reviewer: @eve. Work: OPS-1. See [[Torque table]]."})).await;
    let id = page["id"].as_str().unwrap().to_string();
    assert!(ben.ok("/api/v1/notifications").await.as_array().unwrap().is_empty());
    assert_eq!(page["refs"]["items"]["OPS-1"]["title"], "Replace pump seal");
    assert_eq!(page["refs"]["pages"]["Torque table"]["url"].as_str().unwrap().starts_with("/docs/ENG/"), true);
    // Published: Ben, who can read the space, hears about it. Eve cannot read the space and hears nothing.
    ada.patch(&format!("/api/v1/pages/{id}"), json!({"status": "published"})).await;
    let notes = ben.ok("/api/v1/notifications").await;
    assert_eq!(notes.as_array().unwrap().len(), 1, "{notes}");
    assert_eq!((notes[0]["category"].as_str(), notes[0]["link"].as_str()), (Some("mention"), Some(format!("/docs/ENG/{id}").as_str())));
    assert!(eve.ok("/api/v1/notifications").await.as_array().unwrap().is_empty());
    // Saving again without new mentions tells nobody again.
    ada.patch(&format!("/api/v1/pages/{id}"), json!({"content": "Owner: @ben. Reviewer: @eve. Work: OPS-1. See [[Torque table]]. Updated.", "base_version": 1})).await;
    assert_eq!(ben.ok("/api/v1/notifications").await.as_array().unwrap().len(), 1);
    // Ben reads the page: he is in the project, so the item link resolves for him too.
    assert!(ben.ok(&format!("/api/v1/pages/{id}")).await["refs"]["items"]["OPS-1"].is_object());
    // The version list names its authors as accounts, so an agent's edits would be marked.
    assert_eq!(ada.ok(&format!("/api/v1/pages/{id}/versions")).await[0]["author_user"]["kind"], "human");
}

// ───────────────────────── real time ─────────────────────────

#[tokio::test]
async fn the_socket_needs_a_sign_in_and_the_applications_own_origin() {
    let app = spawn().await;
    let (ada, _, _) = team(&app).await;
    assert_eq!(Ws::try_connect(&app, &app.anon(), Some(&app.base)).await.err(), Some(401));
    // A page on another site holding the visitor's cookie is refused, as is a session with no Origin at all.
    assert_eq!(Ws::try_connect(&app, &ada, Some("https://evil.example")).await.err(), Some(403));
    assert_eq!(Ws::try_connect(&app, &ada, None).await.err(), Some(403));
    assert!(app.security_kinds().await.contains(&"ws.origin_rejected".to_string()));
    // API clients authenticate with a token and need no Origin.
    let token = ada.with_token(&["read"]).await;
    assert!(Ws::try_connect(&app, &token, None).await.is_ok());
    let bad = Caller { token: Some("gty_nope_nope".into()), ..app.anon() };
    assert_eq!(Ws::try_connect(&app, &bad, None).await.err(), Some(401));
    // Turned off in the configuration, the endpoint does not exist.
    let off = spawn_with(|c| c.realtime.enabled = false).await;
    let root = off.admin().await;
    assert_eq!(Ws::try_connect(&off, &root, Some(&off.base)).await.err(), Some(404));
    // One account cannot hold an unbounded number of connections.
    let capped = spawn_with(|c| c.realtime.max_connections_per_user = 2).await;
    let root = capped.admin().await;
    let _a = Ws::connect(&capped, &root).await;
    let _b = Ws::connect(&capped, &root).await;
    assert_eq!(Ws::try_connect(&capped, &root, Some(&capped.base)).await.err(), Some(429));
}

#[tokio::test]
async fn rooms_follow_permissions_and_carry_hints_not_content() {
    let app = spawn().await;
    let (ada, ben, eve) = team(&app).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Confidential pump design"})).await;
    let mut a = Ws::connect(&app, &ada).await;
    let mut b = Ws::connect(&app, &ben).await;
    let mut e = Ws::connect(&app, &eve).await;
    // Eve is not in the project: every room about it is closed to her, and a room that does not exist looks the same.
    for room in ["item:OPS-1", "project:OPS", "item:OPS-999", "item:NOPE-1", "page:00000000-0000-0000-0000-000000000001", "space:NOPE", "user:00000000-0000-0000-0000-000000000001", "admin", "", "item:OPS-1'; DROP TABLE items;--"] {
        if room.is_empty() {
            e.send(json!({"t": "sub", "room": room})).await;
            assert_eq!(e.until(|m| m["t"] == "error").await["of"], "sub");
            continue;
        }
        assert_eq!(e.subscribe(room).await["t"], "denied", "{room}");
    }
    assert_eq!(e.subscribe(&format!("user:{}", ada.user_id)).await["t"], "denied", "nobody listens to someone else's notifications");
    assert_eq!(e.subscribe("work").await["t"], "joined", "the catch-all room is open, but filtered per project");

    let joined = a.subscribe("item:OPS-1").await;
    assert_eq!((joined["t"].as_str(), joined["write"].as_bool(), joined["peers"].as_array().unwrap().len()), (Some("joined"), Some(true), 0));
    let joined = b.subscribe("item:OPS-1").await;
    assert_eq!(joined["peers"][0]["user"]["name"], "ada", "Ben sees that Ada has the item open");
    assert_eq!(a.until(|m| m["t"] == "peer").await["user"]["name"], "ben");
    b.subscribe("project:OPS").await;
    b.subscribe("work").await;

    // Ada changes the item. Ben gets a hint on every room that concerns it; the hint has no content.
    ada.patch("/api/v1/items/OPS-1", json!({"title": "Confidential pump design, rev B", "description": "Secret tolerances: 0.01 mm"})).await;
    let mut rooms = vec![];
    for _ in 0..3 {
        let ev = b.until(|m| m["t"] == "event" && m["kind"] == "item.updated").await;
        assert_eq!(ev["data"]["key"], "OPS-1");
        assert!(!ev.to_string().contains("rev B") && !ev.to_string().contains("tolerances"), "{ev}");
        rooms.push(ev["room"].as_str().unwrap().to_string());
    }
    rooms.sort();
    assert_eq!(rooms, vec!["item:OPS-1", "project:OPS", "work"]);
    // Eve is in the catch-all room and hears nothing about a project she cannot see.
    assert!(e.drain(250).await.iter().all(|m| m["t"] != "event"), "nothing leaks through the shared room");
    // A message in the discussion, and Ben's own notification, arrive live too.
    ada.created("/api/v1/items/OPS-1/comments", json!({"body": "@ben have a look"})).await;
    let got = b.drain(400).await;
    assert!(got.iter().any(|m| m["t"] == "event" && m["kind"] == "comment.created" && m["room"] == "item:OPS-1"), "{got:?}");
    // The notification says what the bell will say (so the page can announce it) and no more:
    // the text of the message stays on the server until Ben opens the item.
    let note = got.iter().find(|m| m["t"] == "notify" && m["category"] == "mention").unwrap_or_else(|| panic!("{got:?}"));
    assert_eq!(note["room"], format!("user:{}", ben.user_id));
    assert!(note["title"].as_str().unwrap().contains("mentioned ben") && note["title"].as_str().unwrap().contains("OPS-1"), "{note}");
    assert_eq!(note["link"], "/i/OPS-1");
    assert!(!got.iter().any(|m| m.to_string().contains("have a look")));
    // It goes to Ben alone.
    assert!(a.drain(200).await.iter().chain(e.drain(200).await.iter()).all(|m| m["t"] != "notify"), "a notification reaches only the person it is for");

    // Presence: where someone is working is shared with the room and nobody else.
    a.drain(200).await;
    b.send(json!({"t": "presence", "room": "item:OPS-1", "state": {"focus": "field:due_date"}})).await;
    let p = a.until(|m| m["t"] == "peer" && m["state"]["focus"].is_string()).await;
    assert_eq!((p["user"]["name"].as_str(), p["state"]["focus"].as_str()), (Some("ben"), Some("field:due_date")));
    b.send(json!({"t": "presence", "room": "item:OPS-1", "state": {"blob": "x".repeat(5000)}})).await;
    assert_eq!(b.until(|m| m["t"] == "error").await["of"], "presence");
    e.send(json!({"t": "presence", "room": "item:OPS-1", "state": {"focus": "spoof"}})).await;
    assert_eq!(e.until(|m| m["t"] == "error").await["of"], "presence", "one cannot speak in a room one is not in");
    // Leaving is announced.
    b.send(json!({"t": "unsub", "room": "item:OPS-1"})).await;
    assert_eq!(a.until(|m| m["t"] == "left").await["conn"], b.conn.as_str());
    drop(b);

    // Losing access ends the stream: Ben is removed from the project and his rooms close at the next check.
    let mut b = Ws::connect(&app, &ben).await;
    assert_eq!(b.subscribe("project:OPS").await["t"], "joined");
    let binding = ada.ok("/api/v1/projects/OPS").await["members"].as_array().unwrap().iter().find(|m| m["user"]["name"] == "ben").unwrap()["binding_id"].as_str().unwrap().to_string();
    ada.send(Method::DELETE, &format!("/api/v1/projects/OPS/members/{binding}"), None).await;
    assert_eq!(b.subscribe("project:OPS").await["t"], "denied", "a new subscription is refused at once");
    // Garbage does not break the connection.
    a.send(json!("just a string")).await;
    a.send(json!({"t": "launch_missiles", "room": "item:OPS-1"})).await;
    assert_eq!(a.until(|m| m["t"] == "error").await["of"], Value::Null, "something that is not a message is answered, not fatal");
    assert_eq!(a.until(|m| m["t"] == "error").await["of"], "launch_missiles");
    assert_eq!(a.subscribe("project:OPS").await["t"], "joined");
}

#[tokio::test]
async fn several_people_edit_the_same_text() {
    let app = spawn().await;
    let (ada, ben, _) = team(&app).await;
    let viewer = app.user("viewer@example.org", "member").await;
    ada.created("/api/v1/projects/OPS/members", json!({"user_id": viewer.user_id, "role": "project_viewer"})).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Shared", "description": "Saved text"})).await;
    let room = "item:OPS-1";
    let mut a = Ws::connect(&app, &ada).await;
    let mut b = Ws::connect(&app, &ben).await;
    let mut v = Ws::connect(&app, &viewer).await;
    for ws in [&mut a, &mut b, &mut v] {
        ws.subscribe(room).await;
    }
    let u = |bytes: &[u8]| B64.encode(bytes);

    // The first editor is asked to load the saved text; the second waits for it.
    a.send(json!({"t": "ysync", "room": room, "doc": "description", "mode": "text"})).await;
    let st = a.until(|m| m["t"] == "ystate").await;
    assert_eq!((st["seed"].as_bool(), st["updates"].as_array().unwrap().len()), (Some(true), 0));
    b.send(json!({"t": "ysync", "room": room, "doc": "description"})).await;
    assert_eq!(b.until(|m| m["t"] == "ystate").await["seed"], false, "only one connection loads the saved content");
    // A viewer may follow along but is never asked to load, and cannot write.
    v.send(json!({"t": "ysync", "room": room, "doc": "description"})).await;
    assert_eq!(v.until(|m| m["t"] == "ystate").await["seed"], false);
    v.send(json!({"t": "y", "room": room, "doc": "description", "u": u(b"vandal")})).await;
    assert!(v.until(|m| m["t"] == "error").await["detail"].as_str().unwrap().contains("view this but not edit"));

    a.send(json!({"t": "y", "room": room, "doc": "description", "u": u(b"seed-update"), "seed": true})).await;
    let ack = a.until(|m| m["t"] == "yack").await;
    let first = ack["id"].as_i64().unwrap();
    // Everyone else in the room receives it, including the viewer; the sender does not get an echo.
    for ws in [&mut b, &mut v] {
        let got = ws.until(|m| m["t"] == "y").await;
        assert_eq!((got["u"].as_str(), got["from"].as_str()), (Some(u(b"seed-update").as_str()), Some(a.conn.as_str())));
    }
    assert!(a.drain(150).await.iter().all(|m| m["t"] != "y"));
    // Seeding twice is refused, so the saved text cannot be inserted a second time.
    b.send(json!({"t": "y", "room": room, "doc": "description", "u": u(b"second-seed"), "seed": true})).await;
    assert!(b.until(|m| m["t"] == "error").await["detail"].as_str().unwrap().contains("already loaded"));
    // Ordinary edits flow both ways and are stored in order.
    b.send(json!({"t": "y", "room": room, "doc": "description", "u": u(b"ben-1")})).await;
    assert_eq!(a.until(|m| m["t"] == "y").await["u"], u(b"ben-1").as_str());
    a.send(json!({"t": "y", "room": room, "doc": "description", "u": u(b"ada-2")})).await;
    let last = a.until(|m| m["t"] == "yack").await["id"].as_i64().unwrap();
    // Cursor positions are forwarded and never stored.
    b.send(json!({"t": "ya", "room": room, "doc": "description", "u": u(b"cursor")})).await;
    assert_eq!(a.until(|m| m["t"] == "ya").await["from"], b.conn.as_str());

    // Someone who opens the item later replays everything.
    let mut late = Ws::connect(&app, &ben).await;
    late.subscribe(room).await;
    late.send(json!({"t": "ysync", "room": room, "doc": "description"})).await;
    let st = late.until(|m| m["t"] == "ystate").await;
    let stored: Vec<String> = st["updates"].as_array().unwrap().iter().map(|x| String::from_utf8(B64.decode(x.as_str().unwrap()).unwrap()).unwrap()).collect();
    assert_eq!(stored, vec!["seed-update", "ben-1", "ada-2"]);
    assert_eq!((st["seed"].as_bool(), st["last"].as_i64()), (Some(false), Some(last)));
    // An editor folds the history into one update; what came after the fold point is kept.
    a.send(json!({"t": "ycompact", "room": room, "doc": "description", "upto": first + 1, "u": u(b"merged")})).await;
    tokio::time::sleep(std::time::Duration::from_millis(150)).await;
    let stored: Vec<Vec<u8>> = sqlx::query_scalar("SELECT data FROM collab_updates WHERE room = $1 ORDER BY id").bind(room).fetch_all(&app.st.db).await.unwrap();
    assert_eq!(stored, vec![b"ada-2".to_vec(), b"merged".to_vec()]);

    // Limits: oversized and malformed updates, and unknown documents.
    a.send(json!({"t": "y", "room": room, "doc": "description", "u": u(&vec![0u8; 1024 * 1024 + 1])})).await;
    assert_eq!(a.until(|m| m["t"] == "error").await["of"], "y");
    a.send(json!({"t": "y", "room": room, "doc": "../secrets", "u": u(b"x")})).await;
    assert_eq!(a.until(|m| m["t"] == "error").await["of"], "y");
    a.send(json!({"t": "y", "room": room, "doc": "description", "u": "%%% not base64 %%%"})).await;
    assert_eq!(a.until(|m| m["t"] == "error").await["of"], "y");

    // Saving from the shared session keeps the working copy. Saving through the API by someone
    // outside it replaces the text, so the working copy is discarded and everyone is told to reload.
    ada.patch("/api/v1/items/OPS-1", json!({"description": "Saved from the editor", "collab": true})).await;
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM collab_updates WHERE room = $1").bind(room).fetch_one(&app.st.db).await.unwrap(), 2);
    b.drain(200).await;
    ben.patch("/api/v1/items/OPS-1", json!({"description": "Overwritten through the API"})).await;
    assert_eq!(b.until(|m| m["t"] == "yreset").await["doc"], "description");
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM collab_updates WHERE room = $1").bind(room).fetch_one(&app.st.db).await.unwrap(), 0);
    // The next editor to open it is asked to load the saved text again.
    b.send(json!({"t": "ysync", "room": room, "doc": "description"})).await;
    assert_eq!(b.until(|m| m["t"] == "ystate").await["seed"], true);

    // Text typed and never saved stays for the next editor, until an editor discards it.
    b.send(json!({"t": "y", "room": room, "doc": "description", "u": u(b"fresh-seed"), "seed": true})).await;
    b.until(|m| m["t"] == "yack").await;
    b.send(json!({"t": "y", "room": room, "doc": "description", "u": u(b"half a thought")})).await;
    b.until(|m| m["t"] == "yack").await;
    a.drain(150).await;
    // A viewer cannot discard other people's unsaved text.
    v.send(json!({"t": "ydiscard", "room": room, "doc": "description"})).await;
    assert_eq!(v.until(|m| m["t"] == "error").await["of"], "ydiscard");
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM collab_updates WHERE room = $1").bind(room).fetch_one(&app.st.db).await.unwrap(), 2);
    // An editor can. Everyone with it open is told, and told who did it; the saved text is untouched.
    b.send(json!({"t": "ydiscard", "room": room, "doc": "description"})).await;
    assert_eq!(a.until(|m| m["t"] == "yreset").await["from"], b.conn.as_str());
    b.until(|m| m["t"] == "ydiscarded").await;
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM collab_updates WHERE room = $1").bind(room).fetch_one(&app.st.db).await.unwrap(), 0);
    assert_eq!(ada.ok("/api/v1/items/OPS-1").await["description"], "Overwritten through the API");
    let logged: (String, String) = sqlx::query_as("SELECT actor_label, target_id FROM audit_log WHERE action = 'draft.discard' ORDER BY id DESC LIMIT 1").fetch_one(&app.st.db).await.unwrap();
    assert_eq!(logged, ("ben@example.org".to_string(), "item:OPS-1/description".to_string()));
    // Discarding when there is nothing to discard is not an error and is not logged again.
    b.send(json!({"t": "ydiscard", "room": room, "doc": "description"})).await;
    b.until(|m| m["t"] == "ydiscarded").await;
    assert_eq!(sqlx::query_scalar::<_, i64>("SELECT count(*) FROM audit_log WHERE action = 'draft.discard'").fetch_one(&app.st.db).await.unwrap(), 1);
}

#[tokio::test]
async fn a_closed_connection_gives_back_its_duties_and_ends_with_the_session() {
    let app = spawn().await;
    let (ada, ben, _) = team(&app).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Shared"})).await;
    let room = "item:OPS-1";
    // Ada is asked to load the text and disconnects before doing so.
    let mut a = Ws::connect(&app, &ada).await;
    a.subscribe(room).await;
    a.send(json!({"t": "ysync", "room": room, "doc": "description"})).await;
    assert_eq!(a.until(|m| m["t"] == "ystate").await["seed"], true);
    drop(a);
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    // Ben is asked instead, rather than waiting forever for a load that will never come.
    let mut b = Ws::connect(&app, &ben).await;
    b.subscribe(room).await;
    b.send(json!({"t": "ysync", "room": room, "doc": "description"})).await;
    assert_eq!(b.until(|m| m["t"] == "ystate").await["seed"], true);
    assert_eq!(app.st.hub.connections(), 1, "the closed connection is gone from the server");

    // Signing out ends the socket at the next check; here the check is run directly.
    assert_eq!(ben.post("/api/v1/auth/logout", json!({})).await.0, StatusCode::NO_CONTENT);
    let alive: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM sessions s JOIN users u ON u.id = s.user_id WHERE u.email = 'ben@example.org')").fetch_one(&app.st.db).await.unwrap();
    assert!(!alive);
    // Shutting the server down closes every socket promptly.
    app.st.hub.close_all();
    assert!(b.drain(1500).await.iter().all(|m| m["t"] != "event"));
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    assert_eq!(app.st.hub.connections(), 0);
}

#[tokio::test]
async fn messages_reach_browsers_connected_to_another_node() {
    // Two server processes on one database, as behind a load balancer.
    let one = spawn().await;
    let db_url = one.st.cfg.database.url.clone();
    let mut cfg = one.st.cfg.clone();
    cfg.database.url = db_url;
    let db = gantry::connect_db(&cfg).await.unwrap();
    let st2 = gantry::AppState::init(cfg, db).await.unwrap();
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base2 = format!("http://{}", listener.local_addr().unwrap());
    let router = gantry::http::build_router(st2.clone());
    tokio::spawn(async move { axum::serve(listener, router.into_make_service_with_connect_info::<std::net::SocketAddr>()).await.unwrap() });
    assert_ne!(one.st.node_id, st2.node_id);

    let (ada, ben, _) = team(&one).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Across nodes"})).await;
    // Each node learns of the other from the heartbeat.
    for st in [&one.st, &st2] {
        gantry::realtime::heartbeat(st, "server", &mut Default::default()).await.unwrap();
    }
    for st in [&one.st, &st2] {
        gantry::realtime::heartbeat(st, "server", &mut Default::default()).await.unwrap();
        assert!(st.hub.peers.load(std::sync::atomic::Ordering::Relaxed));
    }
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    // Ben's browser is connected to node two; Ada works through node one.
    let mut b = Ws::try_connect_at(&base2, &ben, Some(&base2)).await.expect("the second node accepts the same session");
    assert_eq!(b.subscribe("item:OPS-1").await["t"], "joined");
    ada.patch("/api/v1/items/OPS-1", json!({"title": "Across nodes, changed"})).await;
    let ev = b.until(|m| m["t"] == "event" && m["kind"] == "item.updated").await;
    assert_eq!(ev["data"]["key"], "OPS-1");
    // Shared editing crosses nodes too.
    let mut a = Ws::connect(&one, &ada).await;
    a.subscribe("item:OPS-1").await;
    a.send(json!({"t": "ysync", "room": "item:OPS-1", "doc": "description"})).await;
    a.until(|m| m["t"] == "ystate").await;
    a.send(json!({"t": "y", "room": "item:OPS-1", "doc": "description", "u": B64.encode(b"from-node-one"), "seed": true})).await;
    assert_eq!(b.until(|m| m["t"] == "y").await["u"], B64.encode(b"from-node-one").as_str());
    st2.hub.close_all();
}
