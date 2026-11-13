//! Planning and analytics: the levels of the hierarchy, programs and initiatives, reports,
//! search by criteria, Gantt charts, the hierarchy graph, figures, visualizations and
//! dashboards, the frame custom content runs in, and what the application says about itself.

mod common;
use common::*;
use reqwest::{Method, StatusCode};
use serde_json::{json, Value};

fn names(v: &Value) -> Vec<String> {
    v.as_array().unwrap().iter().map(|x| x["name"].as_str().unwrap().to_string()).collect()
}

/// Run a search and return the keys it found, in key order.
async fn search(c: &Caller, q: Value) -> Vec<String> {
    let (s, v) = c.post("/api/v1/items/query", q.clone()).await;
    assert_eq!(s, StatusCode::OK, "{q}: {v}");
    let mut keys: Vec<String> = v["items"].as_array().unwrap().iter().map(|i| i["key"].as_str().unwrap().to_string()).collect();
    assert_eq!(v["total"].as_u64().unwrap() as usize, keys.len(), "{q}");
    keys.sort();
    keys
}

fn one(field: &str, op: &str, value: Value) -> Value {
    json!({"criteria": [{"field": field, "op": op, "value": value}]})
}

/// Walk an item along its workflow until it is done.
async fn finish(c: &Caller, key: &str) {
    let mut visited = vec![];
    for _ in 0..8 {
        let item = c.ok(&format!("/api/v1/items/{key}")).await;
        if item["status_category"] == "done" {
            return;
        }
        visited.push(item["status"].as_str().unwrap().to_string());
        let ts = item["transitions"].as_array().unwrap().clone();
        let next = ts.iter().find(|t| !visited.contains(&t["to"].as_str().unwrap().to_string())).expect("a way forward");
        let (s, v) = c.post(&format!("/api/v1/items/{key}/transition"), json!({"to": next["to"]})).await;
        assert_eq!(s, StatusCode::OK, "{v}");
    }
    panic!("{key} did not reach a done status");
}

async fn team(app: &TestApp) -> (Caller, Caller, Caller) {
    let ada = app.user("ada@example.org", "member").await;
    let ben = app.user("ben@example.org", "member").await;
    let eve = app.user("eve@example.org", "member").await;
    ada.created("/api/v1/projects", json!({"key": "OPS", "name": "Ops", "template": "software"})).await;
    ada.created("/api/v1/projects/OPS/members", json!({"user_id": ben.user_id, "role": "project_member"})).await;
    eve.created("/api/v1/projects", json!({"key": "SEC", "name": "Secret", "template": "basic"})).await;
    eve.created("/api/v1/projects/SEC/items", json!({"title": "Sealed pump bid", "priority": 4})).await;
    (ada, ben, eve)
}

/// The same five pieces of work for the search, report and figure tests.
async fn work(ada: &Caller, ben: &Caller) -> Value {
    let cycle = ada.created("/api/v1/projects/OPS/cycles", json!({"name": "Sprint 9", "start_date": "2030-01-01", "end_date": "2030-01-14"})).await;
    let items = "/api/v1/projects/OPS/items";
    ada.created(items, json!({"title": "Pump overhaul", "type": "epic", "priority": 3, "labels": ["hardware"], "start_date": "2030-01-05", "due_date": "2030-03-01"})).await;
    ada.created(items, json!({"title": "Replace pump seal", "type": "task", "parent": "OPS-1", "assignee_id": ben.user_id, "priority": 4, "labels": ["hardware", "urgent"], "due_date": "2020-01-01", "estimate": 5, "cycle_id": cycle["id"]})).await;
    ada.created(items, json!({"title": "Order 100% cotton rags_", "type": "task", "assignee_id": ada.user_id, "priority": 1, "labels": ["supplies"], "estimate": 1})).await;
    ada.created(items, json!({"title": "Valve sticks", "type": "bug", "description": "It's the cold'; DROP TABLE items;--", "custom": {"severity": "Major"}})).await;
    ada.created(items, json!({"title": "Operator can see pump status", "type": "story", "assignee_id": ben.user_id, "custom": {"story_points": 8}})).await;
    finish(ben, "OPS-5").await;
    cycle
}

// ───────────────────────── hierarchy ─────────────────────────

#[tokio::test]
async fn the_levels_of_the_hierarchy_are_the_organizations_to_define() {
    let app = spawn().await;
    let root = app.admin().await;
    let mia = app.user("mia@example.org", "member").await;
    let h = mia.ok("/api/v1/hierarchy").await;
    assert_eq!(names(&h["container_levels"]), ["Program", "Initiative"]);
    assert_eq!((h["can_manage"].as_bool(), h["enforce_item_levels"].as_bool()), (Some(false), Some(false)));
    let (program, initiative) = (h["container_levels"][0]["id"].clone(), h["container_levels"][1]["id"].clone());

    // A level above programs, and another name for initiatives.
    let body = json!({"kind": "container", "levels": [{"name": "Portfolio", "color": "purple"}, {"id": program, "name": "Program"}, {"id": initiative, "name": "Workstream", "plural": "Workstreams"}]});
    assert_eq!(mia.put("/api/v1/hierarchy/levels", body.clone()).await.0, StatusCode::FORBIDDEN);
    let (s, levels) = root.put("/api/v1/hierarchy/levels", body).await;
    assert_eq!(s, StatusCode::OK, "{levels}");
    assert_eq!(names(&levels), ["Portfolio", "Program", "Workstream"]);
    assert_eq!(levels.as_array().unwrap().iter().map(|l| l["position"].as_i64().unwrap()).collect::<Vec<_>>(), [1, 2, 3]);
    assert_eq!((levels[0]["plural"].as_str(), levels[0]["color"].as_str()), (Some("Portfolios"), Some("purple")));
    assert_eq!(levels[2]["id"], initiative, "a renamed level is still the same level");
    let portfolio = levels[0]["id"].clone();
    for (bad, why) in [
        (json!({"kind": "team", "levels": []}), "kind"),
        (json!({"kind": "container", "levels": [{"name": "A"}, {"name": "a "}]}), "both named"),
        (json!({"kind": "container", "levels": [{"name": "A", "color": "chartreuse"}]}), "color"),
        (json!({"kind": "container", "levels": [{"name": " "}]}), "name"),
        (json!({"kind": "container", "levels": [{"id": "00000000-0000-0000-0000-000000000009", "name": "Ghost"}]}), "does not exist"),
        (json!({"kind": "container", "levels": (0..13).map(|n| json!({"name": format!("L{n}")})).collect::<Vec<_>>()}), "at most 12"),
    ] {
        let (s, v) = root.put("/api/v1/hierarchy/levels", bad.clone()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}");
        assert!(v["detail"].as_str().unwrap().contains(why), "{v}");
    }
    assert_eq!(names(&root.ok("/api/v1/hierarchy").await["container_levels"]), ["Portfolio", "Program", "Workstream"], "a refused change leaves the levels alone");

    // A parent is always on a higher level than what it holds.
    let lunar = root.created("/api/v1/containers", json!({"level_id": portfolio, "name": "Lunar"})).await;
    let lander = root.created("/api/v1/containers", json!({"level_id": program, "name": "Lander", "parent_id": lunar["id"]})).await;
    assert_eq!((lander["ancestors"][0]["name"].as_str(), lander["ancestors"][0]["level"].as_str()), (Some("Lunar"), Some("Portfolio")));
    let (s, v) = root.post("/api/v1/containers", json!({"level_id": portfolio, "name": "Upside down", "parent_id": lander["id"]})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains("not above"), "{v}");
    assert_eq!(root.patch(&format!("/api/v1/containers/{}", lunar["id"].as_str().unwrap()), json!({"parent_id": lander["id"]})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.post("/api/v1/containers", json!({"level_id": program, "name": "Same level", "parent_id": lander["id"]})).await.0, StatusCode::BAD_REQUEST);
    // A level that holds something cannot be removed, and levels cannot be reordered past what they hold.
    let (s, v) = root.put("/api/v1/hierarchy/levels", json!({"kind": "container", "levels": [{"id": program, "name": "Program"}, {"id": initiative, "name": "Workstream"}]})).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(v["detail"].as_str().unwrap().contains("in use"), "{v}");
    let (s, v) = root.put("/api/v1/hierarchy/levels", json!({"kind": "container", "levels": [{"id": program, "name": "Program"}, {"id": portfolio, "name": "Portfolio"}, {"id": initiative, "name": "Workstream"}]})).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(v["detail"].as_str().unwrap().contains("above its own parent"), "{v}");

    // Work item types can be ranked too. Until the rule is switched on it is only a description.
    let (s, item_levels) = root.put("/api/v1/hierarchy/levels", json!({"kind": "item", "levels": [{"name": "Epic"}, {"name": "Story"}, {"name": "Task"}]})).await;
    assert_eq!(s, StatusCode::OK);
    for (n, ty) in ["epic", "story", "task"].iter().enumerate() {
        assert_eq!(root.put(&format!("/api/v1/hierarchy/item-types/{ty}"), json!({"level_id": item_levels[n]["id"]})).await.0, StatusCode::NO_CONTENT);
    }
    assert_eq!(root.put("/api/v1/hierarchy/item-types/task", json!({"level_id": portfolio})).await.0, StatusCode::BAD_REQUEST, "a level above projects is not a work item level");
    assert_eq!(root.put("/api/v1/hierarchy/item-types/no_such_type", json!({"level_id": null})).await.0, StatusCode::NOT_FOUND);
    assert_eq!(mia.put("/api/v1/hierarchy/item-types/task", json!({"level_id": null})).await.0, StatusCode::FORBIDDEN);
    mia.created("/api/v1/projects", json!({"key": "MOON", "name": "Moon", "template": "software"})).await;
    let items = "/api/v1/projects/MOON/items";
    mia.created(items, json!({"title": "A task", "type": "task"})).await;
    mia.created(items, json!({"title": "An epic under a task", "type": "epic", "parent": "MOON-1"})).await;
    let types = mia.ok("/api/v1/projects/MOON").await["types"].clone();
    assert_eq!(types.as_array().unwrap().iter().find(|t| t["key"] == "story").unwrap()["level"]["name"], "Story");
    assert_eq!(mia.put("/api/v1/admin/settings/hierarchy", json!({"enforce_item_levels": true})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(root.put("/api/v1/admin/settings/hierarchy", json!({"enforce_item_levels": "yes"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(root.put("/api/v1/admin/settings/hierarchy", json!({"enforce_item_levels": true})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(mia.ok("/api/v1/hierarchy").await["enforce_item_levels"], true);
    let (s, v) = mia.post(items, json!({"title": "Another epic under a task", "type": "epic", "parent": "MOON-1"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains("cannot sit under"), "{v}");
    mia.created(items, json!({"title": "An epic", "type": "epic"})).await;
    mia.created(items, json!({"title": "A story", "type": "story", "parent": "MOON-3"})).await;
    mia.created(items, json!({"title": "A task in it", "type": "task", "parent": "MOON-4"})).await;
    assert_eq!(mia.patch("/api/v1/items/MOON-3", json!({"parent": "MOON-5"})).await.0, StatusCode::BAD_REQUEST, "moving follows the same rule");
    assert_eq!(mia.post(items, json!({"title": "A story beside a story", "type": "story", "parent": "MOON-4"})).await.0, StatusCode::BAD_REQUEST);
    // A type that is on no level goes anywhere.
    mia.created(items, json!({"title": "A bug", "type": "bug", "parent": "MOON-5", "custom": {"severity": "Minor"}})).await;
    let actions = app.audit_actions().await;
    for a in ["hierarchy.levels_update", "hierarchy.type_level", "settings.update", "container.create"] {
        assert!(actions.contains(&a.to_string()), "{a}");
    }
}

#[tokio::test]
async fn owning_a_program_gives_read_access_to_what_is_in_it_and_only_that() {
    let app = spawn().await;
    let root = app.admin().await;
    // Runs planning, but has no access to private projects.
    let planner = app.user("planner@example.org", "app_admin").await;
    let olive = app.user("olive@example.org", "member").await;
    let lead = app.user("lead@example.org", "member").await;
    let worker = app.user("worker@example.org", "member").await;
    let h = root.ok("/api/v1/hierarchy").await;
    let (program, initiative) = (h["container_levels"][0]["id"].clone(), h["container_levels"][1]["id"].clone());
    lead.created("/api/v1/projects", json!({"key": "ENG", "name": "Engine", "template": "software"})).await;
    lead.created("/api/v1/projects/ENG/items", json!({"title": "Turbopump", "type": "task", "estimate": 5})).await;
    lead.created("/api/v1/projects/ENG/members", json!({"user_id": worker.user_id, "role": "project_member"})).await;
    lead.created("/api/v1/projects", json!({"key": "PAY", "name": "Payroll", "template": "basic"})).await;
    lead.created("/api/v1/projects/PAY/items", json!({"title": "Salary bands"})).await;

    // Programs are created by whoever runs planning.
    assert_eq!(olive.post("/api/v1/containers", json!({"level_id": program, "name": "Mine"})).await.0, StatusCode::FORBIDDEN);
    let apollo = planner.created("/api/v1/containers", json!({"level_id": program, "name": "Apollo", "description": "To the Moon", "status": "active", "start_date": "2030-01-01", "target_date": "2031-12-31"})).await;
    let aid = apollo["id"].as_str().unwrap().to_string();
    assert_eq!((apollo["owner"]["name"].as_str(), apollo["status"].as_str(), apollo["level"]["name"].as_str(), apollo["target_date"].as_str()), (Some("planner"), Some("active"), Some("Program"), Some("2031-12-31")));
    for bad in [json!({"level_id": program, "name": "X", "start_date": "2031-01-01", "target_date": "2030-01-01"}), json!({"level_id": program, "name": "X", "color": "red"}), json!({"level_id": program, "name": "X", "status": "doomed"}), json!({"level_id": program, "name": ""}), json!({"name": "No level"}), json!({"level_id": initiative, "name": "X", "parent_id": "nope"})] {
        assert_eq!(planner.post("/api/v1/containers", bad.clone()).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(names(&planner.ok("/api/v1/containers").await["containers"]), ["Apollo"], "a refused program is not left behind half-made");

    // The planner hands the (still empty) program to Olive. She may describe it and add to it, not hand it on or remove it.
    let (s, v) = planner.patch(&format!("/api/v1/containers/{aid}"), json!({"owner_id": olive.user_id})).await;
    assert_eq!((s, v["owner"]["name"].as_str()), (StatusCode::OK, Some("olive")));
    assert_eq!(olive.patch(&format!("/api/v1/containers/{aid}"), json!({"description": "Updated by its owner", "status": "on_hold"})).await.0, StatusCode::OK);
    let stage = olive.created("/api/v1/containers", json!({"level_id": initiative, "name": "Descent stage", "parent_id": aid})).await;
    let sid = stage["id"].as_str().unwrap().to_string();
    assert_eq!((stage["owner"]["name"].as_str(), stage["can_edit"].as_bool(), stage["can_manage"].as_bool()), (Some("olive"), Some(true), Some(false)));
    assert_eq!(olive.patch(&format!("/api/v1/containers/{aid}"), json!({"owner_id": lead.user_id})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(olive.patch(&format!("/api/v1/containers/{aid}"), json!({"archived": true})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(lead.patch(&format!("/api/v1/containers/{aid}"), json!({"name": "Hijacked"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(lead.post("/api/v1/containers", json!({"level_id": initiative, "name": "Squatter", "parent_id": aid})).await.0, StatusCode::FORBIDDEN);

    // Placing a project lets the owners above read it. That takes the project's administrator
    // and a say over the program: neither alone is enough.
    let place = |c: &Caller, to: Value| {
        let c = c.clone();
        async move { c.put("/api/v1/projects/ENG/container", json!({"container_id": to})).await.0 }
    };
    assert_eq!(place(&olive, json!(sid)).await, StatusCode::NOT_FOUND, "Olive cannot see the project at all");
    assert_eq!(place(&planner, json!(sid)).await, StatusCode::NOT_FOUND);
    assert_eq!(place(&lead, json!(sid)).await, StatusCode::FORBIDDEN, "the project's administrator has no say over the program");
    assert_eq!(place(&root, json!("00000000-0000-0000-0000-000000000001")).await, StatusCode::BAD_REQUEST);
    assert_eq!(place(&root, json!(sid)).await, StatusCode::NO_CONTENT);
    assert_eq!(lead.ok("/api/v1/projects/ENG").await["container"].as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Apollo", "Descent stage"]);

    // Olive now reads the project and its report. She cannot change anything in it, and sees nothing else.
    assert_eq!(olive.ok("/api/v1/items/ENG-1").await["title"], "Turbopump");
    assert_eq!(olive.patch("/api/v1/items/ENG-1", json!({"title": "Edited by the owner above"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(olive.post("/api/v1/projects/ENG/items", json!({"title": "Slipped in"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(olive.post("/api/v1/items/ENG-1/comments", json!({"body": "hello"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(olive.get("/api/v1/projects/PAY").await.0, StatusCode::NOT_FOUND);
    assert_eq!(olive.ok("/api/v1/reports/project/ENG").await["totals"]["total"], 1);
    assert_eq!(worker.get("/api/v1/reports/project/ENG").await.0, StatusCode::FORBIDDEN, "reports are for owners and leads, not every member");
    assert_eq!(lead.ok("/api/v1/reports/project/ENG").await["scope"]["name"], "Engine");
    // The list shows each person the projects they may see, and nothing about the rest: not even how many there are.
    let find = |v: &Value, name: &str| v["containers"].as_array().unwrap().iter().find(|c| c["name"] == name).cloned().unwrap();
    let mine = find(&olive.ok("/api/v1/containers").await, "Descent stage");
    assert_eq!((mine["projects"][0]["key"].as_str(), mine["projects"][0]["open"].as_i64(), mine["mine"].as_bool()), (Some("ENG"), Some(1), Some(true)));
    let theirs = find(&planner.ok("/api/v1/containers").await, "Descent stage");
    assert_eq!(theirs["projects"].as_array().unwrap().len(), 0);
    assert!(theirs.get("hidden_projects").is_none() && planner.ok(&format!("/api/v1/containers/{sid}")).await.get("hidden_projects").is_none(), "no count of what the reader may not see");
    assert!(!theirs.to_string().contains("Engine"), "{theirs}");

    // Whoever runs planning cannot use it to read a project: not by taking the program over,
    // not by hanging it under a program of their own.
    let (s, v) = planner.patch(&format!("/api/v1/containers/{aid}"), json!({"owner_id": planner.user_id})).await;
    assert_eq!(s, StatusCode::FORBIDDEN);
    assert!(v["detail"].as_str().unwrap().contains("cannot see"), "{v}");
    let trojan = planner.created("/api/v1/containers", json!({"level_id": program, "name": "Trojan"})).await;
    assert_eq!(planner.patch(&format!("/api/v1/containers/{sid}"), json!({"parent_id": trojan["id"]})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(planner.get("/api/v1/projects/ENG").await.0, StatusCode::NOT_FOUND);
    assert_eq!(planner.get("/api/v1/items/ENG-1").await.0, StatusCode::NOT_FOUND);
    // Someone who can see everything inside may make the same change, and access follows the new owner.
    assert_eq!(root.patch(&format!("/api/v1/containers/{sid}"), json!({"parent_id": trojan["id"], "owner_id": null})).await.0, StatusCode::OK);
    assert_eq!(olive.get("/api/v1/projects/ENG").await.0, StatusCode::NOT_FOUND, "Olive's program no longer holds it");
    assert_eq!(planner.ok("/api/v1/items/ENG-1").await["title"], "Turbopump");
    assert_eq!(root.patch(&format!("/api/v1/containers/{sid}"), json!({"parent_id": aid})).await.0, StatusCode::OK);
    assert_eq!(planner.get("/api/v1/projects/ENG").await.0, StatusCode::NOT_FOUND);
    // An owner is a person: not an AI agent, not a deactivated account.
    let bot = root.created("/api/v1/admin/users", json!({"name": "Planning bot", "kind": "agent"})).await;
    let (s, v) = root.patch(&format!("/api/v1/containers/{aid}"), json!({"owner_id": bot["id"]})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains("active person"), "{v}");

    // Reports across the program count only the projects the reader may see reports for.
    assert_eq!(place(&root, json!(sid)).await, StatusCode::NO_CONTENT);
    assert_eq!(root.put("/api/v1/projects/PAY/container", json!({"container_id": aid})).await.0, StatusCode::NO_CONTENT);
    let r = olive.ok(&format!("/api/v1/reports/container/{aid}")).await;
    assert_eq!((r["scope"]["projects"].as_i64(), r["scope"]["not_included"].as_i64(), r["totals"]["total"].as_i64(), r["scope"]["level"].as_str()), (Some(2), Some(0), Some(2), Some("Program")));
    assert_eq!(r["by_project"].as_array().unwrap().len(), 2);
    let r = worker.ok(&format!("/api/v1/reports/container/{aid}")).await;
    // The worker can open Engine but not its report, so it is named as left out. Payroll, which the
    // worker cannot see at all, is not counted anywhere.
    assert_eq!((r["scope"]["projects"].as_i64(), r["scope"]["not_included"].as_i64(), r["totals"]["total"].as_i64()), (Some(0), Some(1), Some(0)));
    assert!(!r.to_string().contains("Salary") && !r.to_string().contains("Turbopump"));
    let r = lead.ok(&format!("/api/v1/reports/container/{sid}")).await;
    assert_eq!((r["scope"]["projects"].as_i64(), r["totals"]["estimate_open"].as_f64()), (Some(1), Some(5.0)));
    assert_eq!(olive.get("/api/v1/reports/container/00000000-0000-0000-0000-000000000001").await.0, StatusCode::NOT_FOUND);

    // Archiving an initiative moves what it held up one level instead of losing it.
    assert_eq!(root.patch(&format!("/api/v1/containers/{sid}"), json!({"archived": true})).await.0, StatusCode::OK);
    let apollo = root.ok(&format!("/api/v1/containers/{aid}")).await;
    assert_eq!(apollo["projects"].as_array().unwrap().iter().map(|p| p["key"].as_str().unwrap()).collect::<Vec<_>>(), ["ENG", "PAY"]);
    assert!(apollo["children"].as_array().unwrap().is_empty());
    assert_eq!(olive.ok("/api/v1/projects/ENG").await["container"].as_array().unwrap().len(), 1);
    // Taking a project out ends the access it gave.
    assert_eq!(lead.put("/api/v1/projects/PAY/container", json!({"container_id": null})).await.0, StatusCode::NO_CONTENT, "the project's administrator may always take it out");
    assert_eq!(olive.get("/api/v1/projects/PAY").await.0, StatusCode::NOT_FOUND);
    let actions = app.audit_actions().await;
    for a in ["container.create", "container.update", "container.owner_change", "project.place", "report.view"] {
        assert!(actions.contains(&a.to_string()), "{a}");
    }
}

#[tokio::test]
async fn reports_show_the_state_of_the_work_and_who_is_doing_it() {
    let app = spawn().await;
    let (ada, ben, eve) = team(&app).await;
    work(&ada, &ben).await;
    let auditor = app.user("audit@example.org", "auditor").await;
    let r = ada.ok("/api/v1/reports/project/OPS").await;
    let t = &r["totals"];
    assert_eq!((t["total"].as_i64(), t["done"].as_i64(), t["overdue"].as_i64(), t["unassigned"].as_i64(), t["created_30d"].as_i64(), t["resolved_30d"].as_i64()), (Some(5), Some(1), Some(1), Some(2), Some(5), Some(1)), "{t}");
    assert_eq!((t["estimate_open"].as_f64(), t["estimate_done"].as_f64()), (Some(6.0), Some(0.0)));
    let by = |group: &str, label: &str| r[group].as_array().unwrap().iter().find(|g| g["label"] == label).cloned().unwrap_or(Value::Null);
    assert_eq!((by("by_type", "Task")["total"].as_i64(), by("by_type", "Story")["done"].as_i64(), by("by_priority", "Critical")["open"].as_i64()), (Some(2), Some(1), Some(1)));
    assert_eq!(r["by_status"].as_array().unwrap().iter().map(|g| g["total"].as_i64().unwrap()).sum::<i64>(), 5);
    assert_eq!(by("by_project", "Ops")["total"], 5);
    // Who is doing the work: what each has open, finished lately and late.
    let person = |name: &str| r["people"].as_array().unwrap().iter().find(|p| p["person"]["name"] == name).cloned().unwrap();
    let b = person("ben");
    assert_eq!((b["open"].as_i64(), b["done_30d"].as_i64(), b["overdue"].as_i64(), b["estimate_open"].as_f64(), b["person"]["kind"].as_str()), (Some(1), Some(1), Some(1), Some(5.0), Some("human")));
    assert!(b["actions_30d"].as_i64().unwrap() >= 1, "{b}");
    assert_eq!(person("ada")["open"], 1);
    assert_eq!(r["contributors"][0]["person"]["name"], "ada", "Ada made the most changes");
    assert!(r["contributors"][0]["last_active"].is_string());
    let weeks = r["weekly"].as_array().unwrap();
    assert_eq!((weeks.len(), weeks[11]["created"].as_i64(), weeks[11]["resolved"].as_i64()), (12, Some(5), Some(1)));
    assert_eq!(r["due"].as_array().unwrap().iter().map(|i| i["key"].as_str().unwrap()).collect::<Vec<_>>(), ["OPS-2"], "late and soon, not what is due years from now");
    assert_eq!(r["scope"], json!({"kind": "project", "key": "OPS", "name": "Ops"}));
    // Members without the report permission, and outsiders, are refused; an auditor reads.
    assert_eq!(ben.get("/api/v1/reports/project/OPS").await.0, StatusCode::FORBIDDEN);
    assert_eq!(eve.get("/api/v1/reports/project/OPS").await.0, StatusCode::NOT_FOUND);
    assert_eq!(auditor.ok("/api/v1/reports/project/OPS").await["totals"]["total"], 5);
    // The project's lead reads its report even as an ordinary member.
    assert_eq!(ada.patch("/api/v1/projects/OPS", json!({"lead_id": ben.user_id})).await.0, StatusCode::OK);
    assert_eq!(ben.ok("/api/v1/reports/project/OPS").await["totals"]["total"], 5);
}

// ───────────────────────── search ─────────────────────────

#[tokio::test]
async fn search_matches_any_combination_of_fields_within_what_the_caller_may_see() {
    let app = spawn().await;
    let (ada, ben, eve) = team(&app).await;
    let cycle = work(&ada, &ben).await;
    let all = ["OPS-1", "OPS-2", "OPS-3", "OPS-4", "OPS-5"];
    assert_eq!(search(&ada, json!({})).await, all);
    assert_eq!(search(&eve, json!({})).await, ["SEC-1"], "the same search finds only what its caller may see");
    // Text.
    assert_eq!(search(&ada, one("text", "contains", json!("PUMP"))).await, ["OPS-1", "OPS-2", "OPS-5"]);
    assert_eq!(search(&eve, one("text", "contains", json!("pump"))).await, ["SEC-1"]);
    assert_eq!(search(&ben, one("title", "starts_with", json!("replace"))).await, ["OPS-2"]);
    assert_eq!(search(&ada, one("summary", "not_contains", json!("pump"))).await, ["OPS-3", "OPS-4"]);
    assert_eq!(search(&ada, one("key", "eq", json!("ops-4"))).await, ["OPS-4"]);
    assert_eq!(search(&ada, one("description", "contains", json!("DROP TABLE"))).await, ["OPS-4"]);
    // What would be wildcards in SQL are ordinary characters here.
    assert_eq!(search(&ada, one("title", "contains", json!("%"))).await, ["OPS-3"]);
    assert_eq!(search(&ada, one("title", "contains", json!("_"))).await, ["OPS-3"]);
    assert_eq!(search(&ada, one("title", "contains", json!("100% c"))).await, ["OPS-3"]);
    assert!(search(&ada, one("title", "eq", json!("%"))).await.is_empty());
    // Choices, people and numbers.
    assert_eq!(search(&ada, json!({"criteria": [{"field": "type", "op": "in", "value": ["task", "bug"]}, {"field": "assignee", "op": "in", "value": ["none"]}]})).await, ["OPS-4"]);
    assert_eq!(search(&ben, one("assignee", "in", json!(["me"]))).await, ["OPS-2", "OPS-5"]);
    assert_eq!(search(&ada, one("assignee", "in", json!([ben.user_id, "none"]))).await, ["OPS-1", "OPS-2", "OPS-4", "OPS-5"]);
    assert_eq!(search(&ada, one("assignee", "not_in", json!(["me"]))).await, ["OPS-1", "OPS-2", "OPS-4", "OPS-5"]);
    assert_eq!(search(&ada, one("assignee", "is_empty", Value::Null)).await, ["OPS-1", "OPS-4"]);
    assert_eq!(search(&ben, one("reporter", "in", json!([ada.user_id]))).await, all);
    assert_eq!(search(&ada, one("priority", "gte", json!(3))).await, ["OPS-1", "OPS-2"]);
    assert_eq!(search(&ada, one("priority", "in", json!([1, 4]))).await, ["OPS-2", "OPS-3"]);
    assert_eq!(search(&ada, one("estimate", "gt", json!("2.5"))).await, ["OPS-2"]);
    assert_eq!(search(&ada, one("estimate", "is_empty", Value::Null)).await, ["OPS-1", "OPS-4", "OPS-5"]);
    assert_eq!(search(&ada, one("status_category", "in", json!(["done"]))).await, ["OPS-5"]);
    assert_eq!(search(&ada, one("status_category", "not_in", json!(["done", "in_progress"]))).await, ["OPS-1", "OPS-2", "OPS-3", "OPS-4"]);
    assert_eq!(search(&ada, one("project", "in", json!(["ops"]))).await, all);
    assert!(search(&ada, one("project", "in", json!(["SEC"]))).await.is_empty(), "naming a project one cannot see finds nothing");
    assert_eq!(search(&ada, one("cycle", "in", json!([cycle["id"]]))).await, ["OPS-2"]);
    // Labels.
    assert_eq!(search(&ada, one("label", "all", json!(["hardware", "urgent"]))).await, ["OPS-2"]);
    assert_eq!(search(&ada, one("label", "in", json!(["Supplies", "urgent"]))).await, ["OPS-2", "OPS-3"]);
    assert_eq!(search(&ada, one("label", "not_in", json!(["hardware"]))).await, ["OPS-3", "OPS-4", "OPS-5"]);
    assert_eq!(search(&ada, one("label", "is_empty", Value::Null)).await, ["OPS-4", "OPS-5"]);
    // Dates, literal and relative to today.
    assert_eq!(search(&ada, one("due", "before", json!("2025-01-01"))).await, ["OPS-2"]);
    assert_eq!(search(&ada, one("due", "between", json!(["2030-01-01", "2030-12-31"]))).await, ["OPS-1"]);
    assert_eq!(search(&ada, one("due", "is_empty", Value::Null)).await, ["OPS-3", "OPS-4", "OPS-5"]);
    assert_eq!(search(&ada, one("due", "before", json!({"days": 0}))).await, ["OPS-2"]);
    assert_eq!(search(&ada, one("started", "on_or_after", json!("2030-01-05"))).await, ["OPS-1"]);
    assert_eq!(search(&ada, one("created", "on_or_after", json!({"days": -1}))).await, all);
    assert_eq!(search(&ada, one("resolved", "is_not_empty", Value::Null)).await, ["OPS-5"]);
    assert_eq!(search(&ada, one("overdue", "is", json!(true))).await, ["OPS-2"]);
    // Parents.
    assert_eq!(search(&ada, one("parent", "eq", json!("ops-1"))).await, ["OPS-2"]);
    assert_eq!(search(&ada, one("has_parent", "is", json!(false))).await, ["OPS-1", "OPS-3", "OPS-4", "OPS-5"]);
    assert!(search(&ada, one("parent", "eq", json!("SEC-1"))).await.is_empty());
    // Fields the organization defined itself.
    assert_eq!(search(&ada, one("custom.severity", "in", json!(["Major"]))).await, ["OPS-4"]);
    assert_eq!(search(&ada, one("custom.severity", "is_empty", Value::Null)).await, ["OPS-1", "OPS-2", "OPS-3", "OPS-5"]);
    assert_eq!(search(&ada, one("custom.story_points", "gte", json!(5))).await, ["OPS-5"]);
    assert!(search(&ada, one("custom.story_points", "lt", json!(5))).await.is_empty());
    // What one follows is one's own business.
    assert!(ben.send(Method::POST, "/api/v1/items/OPS-4/watch", None).await.0.is_success());
    assert!(search(&ben, one("watcher", "in", json!(["me"]))).await.contains(&"OPS-4".to_string()));
    assert_eq!(ben.post("/api/v1/items/query", one("watcher", "in", json!([ada.user_id]))).await.0, StatusCode::BAD_REQUEST);
    // Any of several criteria.
    assert_eq!(search(&ada, json!({"match": "any", "criteria": [{"field": "type", "op": "in", "value": ["bug"]}, {"field": "priority", "op": "eq", "value": 4}]})).await, ["OPS-2", "OPS-4"]);

    // Sorting and paging; each result carries the high-level data for a list.
    let (_, v) = ada.post("/api/v1/items/query", json!({"sort": [{"field": "priority", "dir": "desc"}, {"field": "key"}]})).await;
    assert_eq!(v["items"].as_array().unwrap().iter().map(|i| i["key"].as_str().unwrap()).collect::<Vec<_>>(), ["OPS-2", "OPS-1", "OPS-4", "OPS-5", "OPS-3"]);
    let first = &v["items"][0];
    assert_eq!((first["title"].as_str(), first["priority"].as_i64(), first["assignee"]["name"].as_str(), first["status_category"].as_str(), first["due_date"].as_str(), first["type"].as_str()), (Some("Replace pump seal"), Some(4), Some("ben"), Some("todo"), Some("2020-01-01"), Some("task")), "{first}");
    assert!(first["status_name"].is_string() && first["labels"].is_array());
    let (_, v) = ada.post("/api/v1/items/query", json!({"sort": [{"field": "key"}], "limit": 2, "offset": 2})).await;
    assert_eq!((v["items"].as_array().unwrap().iter().map(|i| i["key"].as_str().unwrap()).collect::<Vec<_>>(), v["total"].as_i64(), v["limit"].as_i64(), v["offset"].as_i64()), (vec!["OPS-3", "OPS-4"], Some(5), Some(2), Some(2)));
    let (_, v) = ada.post("/api/v1/items/query", json!({"sort": [{"field": "due", "dir": "asc"}, {"field": "assignee"}, {"field": "title", "dir": "desc"}]})).await;
    assert_eq!(v["items"][0]["key"], "OPS-2", "dated work first, undated last");

    // What cannot be understood is refused with a reason rather than guessed at.
    for (bad, why) in [
        (one("nope", "eq", json!(1)), "not a field"),
        (one("custom.nope", "eq", json!(1)), "not a custom field"),
        (one("priority", "contains", json!("x")), "not an operator"),
        (one("priority", "gte", json!("high")), "number"),
        (one("due", "before", json!("next week")), "dates look like"),
        (one("due", "between", json!(["2030-01-01"])), "two dates"),
        (one("assignee", "in", json!(["not-an-id"])), "account ids"),
        (one("cycle", "in", json!(["1 OR 1=1"])), "ids"),
        (one("overdue", "is", json!("true")), "true or false"),
        (one("label", "in", json!([])), "between 1 and 200"),
        (json!({"match": "some"}), "all or any"),
        (json!({"sort": [{"field": "title; DROP TABLE items"}]}), "cannot sort"),
        (json!({"sort": [{"field": "title", "dir": "sideways"}]}), "asc or desc"),
        (json!({"sort": (0..5).map(|_| json!({"field": "key"})).collect::<Vec<_>>()}), "four"),
        (json!({"criteria": (0..41).map(|_| json!({"field": "priority", "op": "eq", "value": 1})).collect::<Vec<_>>()}), "at most 40"),
    ] {
        let (s, v) = ada.post("/api/v1/items/query", bad.clone()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{bad}: {v}");
        assert!(v["detail"].as_str().unwrap().contains(why), "{bad}: {v}");
    }
    assert!(ada.post("/api/v1/items/query", json!({"criteria": [], "where": "1=1"})).await.0.is_client_error(), "unknown parts of a search are refused");
    assert!(ada.post("/api/v1/items/query", json!({"criteria": [{"field": "title", "op": "eq", "value": "x", "sql": "1=1"}]})).await.0.is_client_error());
    assert_eq!(app.anon().post("/api/v1/items/query", json!({})).await.0, StatusCode::UNAUTHORIZED);
    assert_eq!(search(&ada, json!({})).await, all, "nothing was harmed by any of it");

    // The same search as a GET, for clients that may only read.
    let q = serde_json::to_string(&one("priority", "gte", json!(3))).unwrap();
    let got = ada.request(Method::GET, "/api/v1/items/query").query(&[("q", q.as_str())]).send().await.unwrap().json::<Value>().await.unwrap();
    assert_eq!(got["total"], 2);
    assert_eq!(ada.get("/api/v1/items/query").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ada.request(Method::GET, "/api/v1/items/query").query(&[("q", "{not json")]).send().await.unwrap().status(), StatusCode::BAD_REQUEST);

    // The form is built from what exists where the caller can see.
    let f = ben.ok("/api/v1/items/query/fields").await;
    let field = |key: &str| f["fields"].as_array().unwrap().iter().find(|x| x["key"] == key).cloned().unwrap_or(Value::Null);
    assert_eq!((field("due")["kind"].as_str(), field("due")["label"].as_str()), (Some("date"), Some("Due date")));
    assert!(field("due")["ops"].as_array().unwrap().contains(&json!("between")));
    assert_eq!((field("custom.severity")["custom"].as_bool(), field("custom.severity")["label"].as_str()), (Some(true), Some("Severity")));
    assert!(field("custom.severity")["options"].as_array().unwrap().contains(&json!("Major")));
    assert_eq!(f["values"]["project"].as_array().unwrap().iter().map(|p| p["value"].as_str().unwrap()).collect::<Vec<_>>(), ["OPS"], "Ben is not offered projects he cannot see");
    assert_eq!(f["values"]["label"], json!(["hardware", "supplies", "urgent"]));
    assert!(f["values"]["cycle"].as_array().unwrap().iter().any(|c| c["label"] == "OPS · Sprint 9"));
    assert!(f["sorts"].as_array().unwrap().contains(&json!("due")));
    assert!(!eve.ok("/api/v1/items/query/fields").await["values"].to_string().contains("hardware"));
}

#[tokio::test]
async fn searches_can_be_saved_shared_and_asked_to_alert() {
    let app = spawn().await;
    let (ada, ben, eve) = team(&app).await;
    let customer = app.user("cust@example.org", "customer").await;
    let q = json!({"criteria": [{"field": "priority", "op": "gte", "value": 4}], "sort": [{"field": "due"}]});
    let saved = ada.created("/api/v1/searches", json!({"name": "Critical work", "query": q, "alert": true})).await;
    let id = saved["id"].as_str().unwrap().to_string();
    assert_eq!(ada.post("/api/v1/searches", json!({"name": "Broken", "query": one("nope", "eq", json!(1))})).await.0, StatusCode::BAD_REQUEST, "a search that cannot run is not saved");
    assert_eq!(ada.post("/api/v1/searches", json!({"name": " ", "query": q})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ada.post("/api/v1/searches", json!({"name": "No query"})).await.0, StatusCode::BAD_REQUEST);
    let list = ada.ok("/api/v1/searches").await;
    assert_eq!(list.as_array().unwrap().len(), 1);
    assert_eq!((list[0]["mine"].as_bool(), list[0]["alert"].as_bool(), list[0]["shared"].as_bool(), list[0]["query"]["match"].as_str(), list[0]["query"]["sort"][0]["dir"].as_str()), (Some(true), Some(true), Some(false), Some("all"), Some("asc")));
    // Private until shared; shared ones can be used by others but changed only by their owner.
    assert!(ben.ok("/api/v1/searches").await.as_array().unwrap().is_empty());
    assert_eq!(ben.patch(&format!("/api/v1/searches/{id}"), json!({"name": "Mine now"})).await.0, StatusCode::NOT_FOUND);
    assert_eq!(ada.patch(&format!("/api/v1/searches/{id}"), json!({"shared": true})).await.0, StatusCode::NO_CONTENT);
    let theirs = ben.ok("/api/v1/searches").await;
    assert_eq!((theirs[0]["mine"].as_bool(), theirs[0]["owner"]["name"].as_str()), (Some(false), Some("ada")));
    assert_eq!(ben.patch(&format!("/api/v1/searches/{id}"), json!({"name": "Mine now"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(ben.delete(&format!("/api/v1/searches/{id}")).await.0, StatusCode::NOT_FOUND);
    assert!(customer.ok("/api/v1/searches").await.as_array().unwrap().is_empty(), "portal customers are not shown the workspace's searches");

    // Alerts. The first check only takes stock of what already matches.
    let later = || async {
        sqlx::query("UPDATE saved_searches SET alert_checked = now() - interval '10 minutes' WHERE alert_checked IS NOT NULL").execute(&app.st.db).await.unwrap();
        gantry::planning::saved_search_alerts(&app.st).await.unwrap()
    };
    let alerts = |c: &Caller| {
        let c = c.clone();
        async move { c.ok("/api/v1/notifications").await.as_array().unwrap().iter().filter(|n| n["category"] == "search_alert").cloned().collect::<Vec<_>>() }
    };
    ada.created("/api/v1/projects/OPS/items", json!({"title": "A fire that was already burning", "priority": 4})).await;
    assert_eq!(gantry::planning::saved_search_alerts(&app.st).await.unwrap(), 0);
    assert_eq!(gantry::planning::saved_search_alerts(&app.st).await.unwrap(), 0, "not checked again within minutes");
    assert_eq!(later().await, 0, "nothing new");
    // New matches: one Ada can see, one she cannot. The search runs with her permissions.
    ben.created("/api/v1/projects/OPS/items", json!({"title": "A new fire", "priority": 4})).await;
    eve.created("/api/v1/projects/SEC/items", json!({"title": "A fire Ada must not hear about", "priority": 4})).await;
    assert_eq!(later().await, 1);
    let got = alerts(&ada).await;
    assert_eq!(got.len(), 1, "{got:?}");
    assert_eq!((got[0]["title"].as_str(), got[0]["link"].as_str()), (Some("1 new in \"Critical work\""), Some(format!("/search?saved={id}").as_str())));
    assert!(got[0]["body"].as_str().unwrap().contains("A new fire") && !got[0].to_string().contains("must not hear"));
    assert!(alerts(&ben).await.is_empty(), "the alert is its owner's, even on a shared search");
    assert_eq!(later().await, 0, "told once");
    // Changing what is searched for starts over, so old matches are not announced as new.
    assert_eq!(ada.patch(&format!("/api/v1/searches/{id}"), json!({"query": one("priority", "gte", json!(2))})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(gantry::planning::saved_search_alerts(&app.st).await.unwrap(), 0);
    assert_eq!(alerts(&ada).await.len(), 1);
    // Losing access to the project ends the alerts about it.
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Seen by its owner"})).await;
    assert_eq!(later().await, 1);
    assert_eq!(ada.patch(&format!("/api/v1/searches/{id}"), json!({"alert": false})).await.0, StatusCode::NO_CONTENT);
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Nobody asked"})).await;
    assert_eq!(later().await, 0);
    assert_eq!(ada.delete(&format!("/api/v1/searches/{id}")).await.0, StatusCode::NO_CONTENT);
    assert!(ben.ok("/api/v1/searches").await.as_array().unwrap().is_empty());
    for a in ["search.save", "search.update", "search.delete"] {
        assert!(app.audit_actions().await.contains(&a.to_string()), "{a}");
    }
}

// ───────────────────────── Gantt and graph ─────────────────────────

#[tokio::test]
async fn gantt_charts_show_dated_work_and_carry_their_owners_annotations() {
    let app = spawn().await;
    let root = app.admin().await;
    let (ada, ben, eve) = team(&app).await;
    let viewer = app.user("viewer@example.org", "member").await;
    ada.created("/api/v1/projects/OPS/members", json!({"user_id": viewer.user_id, "role": "project_viewer"})).await;
    work(&ada, &ben).await;
    let data = ben.ok("/api/v1/gantt/data?project=ops").await;
    let item = |d: &Value, key: &str| d["items"].as_array().unwrap().iter().find(|i| i["key"] == key).cloned().unwrap();
    assert_eq!(data["items"].as_array().unwrap().len(), 5);
    let epic = item(&data, "OPS-1");
    assert_eq!((epic["start_date"].as_str(), epic["due_date"].as_str(), epic["type_name"].as_str(), epic["status_category"].as_str()), (Some("2030-01-05"), Some("2030-03-01"), Some("Epic"), Some("todo")));
    assert_eq!((item(&data, "OPS-2")["parent_id"].clone(), item(&data, "OPS-2")["assignee"]["name"].as_str()), (epic["id"].clone(), Some("ben")));
    assert_eq!(data["cycles"].as_array().unwrap().iter().find(|c| c["name"] == "Sprint 9").unwrap()["end_date"], "2030-01-14");
    assert_eq!((data["editable"]["OPS"].as_bool(), viewer.ok("/api/v1/gantt/data?project=OPS").await["editable"]["OPS"].as_bool()), (Some(true), Some(false)));
    let outside = eve.ok("/api/v1/gantt/data?project=OPS").await;
    assert_eq!((outside["items"].as_array().unwrap().len(), outside["editable"].as_object().unwrap().len()), (0, 0));
    assert_eq!(ada.get("/api/v1/gantt/data").await.0, StatusCode::BAD_REQUEST);

    // Dragging a bar is an ordinary change to the work item, with the same rules and the same history.
    assert_eq!(ben.patch("/api/v1/items/OPS-2", json!({"start_date": "2030-01-10", "due_date": "2030-01-20"})).await.0, StatusCode::OK);
    assert_eq!(item(&ada.ok("/api/v1/gantt/data?project=OPS").await, "OPS-2")["due_date"], "2030-01-20");
    assert_eq!(viewer.patch("/api/v1/items/OPS-2", json!({"due_date": "2030-01-25"})).await.0, StatusCode::FORBIDDEN);
    let (s, v) = ben.patch("/api/v1/items/OPS-2", json!({"due_date": "2030-01-09"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains("before the start date"), "{v}");
    assert_eq!(ben.patch("/api/v1/items/OPS-2", json!({"start_date": "2030-02-01"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ben.patch("/api/v1/items/OPS-2", json!({"start_date": "2030-02-01", "due_date": "2030-02-03"})).await.0, StatusCode::OK, "both ends may move together");
    assert_eq!(ben.post("/api/v1/projects/OPS/items", json!({"title": "Backward", "start_date": "2030-02-02", "due_date": "2030-02-01"})).await.0, StatusCode::BAD_REQUEST);
    assert!(ada.ok("/api/v1/items/OPS-2/history").await["entries"].as_array().unwrap().iter().any(|e| e["field"] == "due_date" && e["new"] == "2030-02-03" && e["actor"]["name"] == "ben"));

    // A saved chart: its owner's lines, shaded spans and groups.
    let ann = json!([
        {"id": "a1", "type": "line", "date": "2030-03-01", "color": "red", "label": "Launch", "style": "dashed"},
        {"id": "a2", "type": "range", "start": "2030-02-01", "end": "2030-02-14", "color": "#ffcc00", "label": "Design freeze"},
        {"id": "a3", "type": "group", "items": ["OPS-1", "OPS-2"], "color": "blue", "label": "Pump work"}]);
    let settings = json!({"zoom": "week", "group_by": "assignee", "show_done": false, "color_by": "priority"});
    let id = ada.created("/api/v1/gantt", json!({"name": "Q1 plan", "scope": {"project": "OPS"}, "annotations": ann, "settings": settings})).await["id"].as_str().unwrap().to_string();
    let chart = ada.ok(&format!("/api/v1/gantt/{id}")).await;
    assert_eq!((chart["chart"]["annotations"].clone(), chart["chart"]["settings"].clone(), chart["chart"]["mine"].as_bool(), chart["items"].as_array().unwrap().len()), (ann.clone(), settings, Some(true), 5));
    assert_eq!(ben.get(&format!("/api/v1/gantt/{id}")).await.0, StatusCode::NOT_FOUND, "private until shared");
    for bad in [
        json!([{"id": "x", "type": "line", "color": "red"}]),
        json!([{"id": "x", "type": "line", "date": "2030-02-30", "color": "red"}]),
        json!([{"id": "x", "type": "line", "date": "2030-03-01", "color": "red", "style": "wavy"}]),
        json!([{"id": "x", "type": "range", "start": "2030-03-01", "end": "2030-02-01", "color": "red"}]),
        json!([{"id": "x", "type": "group", "items": ["not a key"], "color": "red"}]),
        json!([{"id": "x", "type": "group", "items": [], "color": "red"}]),
        json!([{"id": "x", "type": "line", "date": "2030-03-01", "color": "javascript:alert(1)"}]),
        json!([{"id": "x", "type": "line", "date": "2030-03-01", "color": "red;background:url(https://evil.example)"}]),
        json!([{"id": "x", "type": "line", "date": "2030-03-01", "color": "red", "onclick": "alert(1)"}]),
        json!([{"id": "x", "type": "line", "date": "2030-03-01", "color": "red", "label": "x".repeat(121)}]),
        json!([{"type": "line", "date": "2030-03-01", "color": "red"}]),
        json!([{"id": "x", "type": "circle", "color": "red"}]),
        json!({"id": "x"}),
    ] {
        assert_eq!(ada.patch(&format!("/api/v1/gantt/{id}"), json!({"annotations": bad})).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    for bad in [json!({"zoom": "century"}), json!({"nope": true}), json!({"show_done": "yes"}), json!([])] {
        assert_eq!(ada.patch(&format!("/api/v1/gantt/{id}"), json!({"settings": bad})).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    for bad in [json!({"project": "NOPE"}), json!({}), json!({"project": "SEC"}), json!({"container": "00000000-0000-0000-0000-000000000001"})] {
        assert_eq!(ada.post("/api/v1/gantt", json!({"name": "X", "scope": bad})).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }
    assert_eq!(ada.ok(&format!("/api/v1/gantt/{id}")).await["chart"]["annotations"], ann, "refused changes leave the chart as it was");

    // Shared: others open it, only its owner changes it, and each sees only the work they may see.
    assert_eq!(ada.patch(&format!("/api/v1/gantt/{id}"), json!({"shared": true})).await.0, StatusCode::NO_CONTENT);
    let theirs = ben.ok(&format!("/api/v1/gantt/{id}")).await;
    assert_eq!((theirs["chart"]["mine"].as_bool(), theirs["chart"]["owner"]["name"].as_str(), theirs["items"].as_array().unwrap().len()), (Some(false), Some("ada"), 5));
    assert_eq!(ben.patch(&format!("/api/v1/gantt/{id}"), json!({"annotations": []})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(ben.delete(&format!("/api/v1/gantt/{id}")).await.0, StatusCode::NOT_FOUND);
    let outside = eve.ok(&format!("/api/v1/gantt/{id}")).await;
    assert_eq!((outside["chart"]["name"].as_str(), outside["items"].as_array().unwrap().len()), (Some("Q1 plan"), 0));
    assert_eq!(ben.ok("/api/v1/gantt").await.as_array().unwrap().iter().map(|g| g["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Q1 plan"]);
    // People looking at the same chart see changes to it as they are made.
    let mut b = Ws::connect(&app, &ben).await;
    let mut a = Ws::connect(&app, &ada).await;
    assert_eq!(b.subscribe(&format!("gantt:{id}")).await["write"], false);
    assert_eq!(a.subscribe(&format!("gantt:{id}")).await["write"], true);
    assert_eq!(ada.patch(&format!("/api/v1/gantt/{id}"), json!({"name": "Q1 plan, revised"})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(b.until(|m| m["t"] == "event").await["kind"], "gantt.updated");

    // A chart can cover a whole program.
    let program = root.ok("/api/v1/hierarchy").await["container_levels"][0]["id"].clone();
    let cid = root.created("/api/v1/containers", json!({"level_id": program, "name": "Apollo"})).await["id"].as_str().unwrap().to_string();
    assert_eq!(root.put("/api/v1/projects/OPS/container", json!({"container_id": cid})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(root.put("/api/v1/projects/SEC/container", json!({"container_id": cid})).await.0, StatusCode::NO_CONTENT);
    let wide = ada.created("/api/v1/gantt", json!({"name": "Apollo", "scope": {"container": cid}})).await["id"].as_str().unwrap().to_string();
    let seen = ada.ok(&format!("/api/v1/gantt/{wide}")).await;
    assert!(seen["items"].as_array().unwrap().iter().all(|i| i["project"] == "OPS"), "only the projects Ada may see");
    assert_eq!(root.ok(&format!("/api/v1/gantt/data?container={cid}")).await["items"].as_array().unwrap().len(), 6);
    assert_eq!(ada.delete(&format!("/api/v1/gantt/{id}")).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ben.get(&format!("/api/v1/gantt/{id}")).await.0, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn the_graph_ties_every_piece_of_work_to_its_project_and_program() {
    let app = spawn().await;
    let root = app.admin().await;
    let (ada, ben, eve) = team(&app).await;
    let customer = app.user("cust@example.org", "customer").await;
    work(&ada, &ben).await;
    // An AI agent holds one of the tasks.
    let bot = root.created("/api/v1/admin/users", json!({"name": "Triage bot", "kind": "agent"})).await;
    root.agent_rules(bot["id"].as_str().unwrap(), working_agent()).await;
    ada.created("/api/v1/projects/OPS/members", json!({"user_id": bot["id"], "role": "project_member"})).await;
    assert_eq!(ada.patch("/api/v1/items/OPS-3", json!({"assignee_id": bot["id"]})).await.0, StatusCode::OK);
    let h = root.ok("/api/v1/hierarchy").await;
    let apollo = root.created("/api/v1/containers", json!({"level_id": h["container_levels"][0]["id"], "name": "Apollo", "status": "active"})).await["id"].as_str().unwrap().to_string();
    let stage = root.created("/api/v1/containers", json!({"level_id": h["container_levels"][1]["id"], "name": "Descent stage", "parent_id": apollo})).await["id"].as_str().unwrap().to_string();
    assert_eq!(root.put("/api/v1/projects/OPS/container", json!({"container_id": stage})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(root.put("/api/v1/projects/SEC/container", json!({"container_id": stage})).await.0, StatusCode::NO_CONTENT);

    let node = |g: &Value, label: &str| g["nodes"].as_array().unwrap().iter().find(|n| n["label"] == label).cloned();
    let edge = |g: &Value, from: &str, to: &str| {
        let (f, t) = (node(g, from).unwrap()["id"].clone(), node(g, to).unwrap()["id"].clone());
        g["edges"].as_array().unwrap().iter().any(|e| e["from"] == f && e["to"] == t)
    };
    let g = ada.ok("/api/v1/graph/hierarchy").await;
    // Program → initiative → project → epic → task, and loose work straight from its project.
    for (from, to) in [("Apollo", "Descent stage"), ("Descent stage", "OPS"), ("OPS", "OPS-1"), ("OPS-1", "OPS-2"), ("OPS", "OPS-3"), ("OPS", "OPS-4")] {
        assert!(edge(&g, from, to), "{from} → {to}: {}", g["edges"]);
    }
    assert!(!edge(&g, "OPS", "OPS-2"), "a child hangs from its parent, not from the project as well");
    let (program, project, task, held) = (node(&g, "Apollo").unwrap(), node(&g, "OPS").unwrap(), node(&g, "OPS-2").unwrap(), node(&g, "OPS-3").unwrap());
    assert_eq!((program["kind"].as_str(), program["level"].as_str(), program["depth"].as_i64(), program["status"].as_str(), program["person"]["name"].as_str(), program["url"].as_str()), (Some("container"), Some("Program"), Some(1), Some("active"), Some("root"), Some(format!("/planning/{apollo}").as_str())));
    assert_eq!((project["kind"].as_str(), project["title"].as_str(), project["status"].as_str(), project["open"].as_i64(), project["total"].as_i64(), project["person"]["name"].as_str()), (Some("project"), Some("Ops"), Some("in_progress"), Some(4), Some(5), Some("ada")));
    assert_eq!((task["kind"].as_str(), task["title"].as_str(), task["level"].as_str(), task["status"].as_str(), task["person"]["name"].as_str(), task["person"]["kind"].as_str(), task["url"].as_str()), (Some("item"), Some("Replace pump seal"), Some("Task"), Some("todo"), Some("ben"), Some("human"), Some("/i/OPS-2")));
    assert!(task["status_name"].is_string() && task["person"]["avatar"].is_string(), "{task}");
    assert_eq!((held["person"]["kind"].as_str(), held["person"]["name"].as_str()), (Some("agent"), Some("Triage bot")), "an AI agent is marked as one");
    assert_eq!(node(&g, "OPS-4").unwrap()["person"], Value::Null, "unassigned work has nobody on it");
    // Finished work is left out unless asked for.
    assert!(node(&g, "OPS-5").is_none());
    assert_eq!(node(&ada.ok("/api/v1/graph/hierarchy?done=true").await, "OPS-5").unwrap()["status"], "done");
    // What Ada cannot see is not in her graph: Eve's project is placed in the same initiative.
    assert!(node(&g, "SEC").is_none() && node(&g, "SEC-1").is_none());
    assert!(!g.to_string().contains("Sealed"));
    let theirs = eve.ok("/api/v1/graph/hierarchy").await;
    assert!(node(&theirs, "SEC-1").is_some() && node(&theirs, "OPS").is_none() && !theirs.to_string().contains("pump seal"));
    assert_eq!(g["truncated"], false);
    // Narrowed to one project or one branch.
    let narrow = ada.ok("/api/v1/graph/hierarchy?project=OPS").await;
    assert!(node(&narrow, "Apollo").is_none() && node(&narrow, "OPS-1").is_some());
    let branch = ada.ok(&format!("/api/v1/graph/hierarchy?container={stage}")).await;
    assert!(node(&branch, "Apollo").is_none() && node(&branch, "Descent stage").is_some() && edge(&branch, "Descent stage", "OPS"));
    assert_eq!(customer.get("/api/v1/graph/hierarchy").await.0, StatusCode::FORBIDDEN);

    // The graph stays current: changes to work and to the structure are announced to open pages.
    let mut ws = Ws::connect(&app, &ada).await;
    ws.subscribe("work").await;
    ws.subscribe("planning").await;
    assert_eq!(ben.patch("/api/v1/items/OPS-2", json!({"assignee_id": null})).await.0, StatusCode::OK);
    let ev = ws.until(|m| m["t"] == "event" && m["room"] == "work").await;
    assert_eq!((ev["kind"].as_str(), ev["data"]["key"].as_str()), (Some("item.updated"), Some("OPS-2")));
    assert_eq!(root.patch(&format!("/api/v1/containers/{apollo}"), json!({"status": "done"})).await.0, StatusCode::OK);
    assert_eq!(ws.until(|m| m["t"] == "event" && m["room"] == "planning").await["kind"], "planning.updated");
    assert_eq!(node(&ada.ok("/api/v1/graph/hierarchy").await, "OPS-2").unwrap()["person"], Value::Null);
}

// ───────────────────────── figures, visualizations, dashboards ─────────────────────────

#[tokio::test]
async fn figures_cover_only_the_work_the_caller_can_see() {
    let app = spawn().await;
    let (ada, ben, eve) = team(&app).await;
    work(&ada, &ben).await;
    let s = ada.ok("/api/v1/analytics/summary").await;
    assert_eq!((s["total"].as_i64(), s["open"].as_i64(), s["done"].as_i64(), s["overdue"].as_i64(), s["unassigned"].as_i64(), s["projects"].as_i64(), s["resolved_30d"].as_i64()), (Some(5), Some(4), Some(1), Some(1), Some(2), Some(1), Some(1)));
    assert_eq!(eve.ok("/api/v1/analytics/summary").await["total"], 1);
    assert_eq!(ada.ok("/api/v1/analytics/summary?project=SEC").await["total"], 0, "asking for a project one cannot see yields nothing");
    assert_eq!(ada.ok("/api/v1/analytics/summary?project=ops").await["total"], 5);
    let odd = ada.request(Method::GET, "/api/v1/analytics/summary").query(&[("project", "OPS' OR '1'='1")]).send().await.unwrap().json::<Value>().await.unwrap();
    assert_eq!(odd["total"], 0);

    let g = ada.ok("/api/v1/analytics/items?group_by=type").await;
    let group = |g: &Value, label: &str| g["groups"].as_array().unwrap().iter().find(|x| x["label"] == label).cloned().unwrap_or(Value::Null);
    assert_eq!((g["total"].as_f64(), group(&g, "Task")["value"].as_f64(), group(&g, "Story")["done"].as_i64(), group(&g, "Task")["key"].as_str()), (Some(5.0), Some(2.0), Some(1), Some("task")));
    assert_eq!(ada.ok("/api/v1/analytics/items").await["group_by"], json!(["status_category"]));
    let nested = ada.ok("/api/v1/analytics/items?group_by=assignee,status_category&open=true").await;
    assert_eq!(nested["total"], 4.0);
    assert!(nested["groups"].as_array().unwrap().iter().all(|x| x["path"].as_array().unwrap().len() == 2));
    assert_eq!(group(&nested, "Unassigned")["path"][1]["label"], "To do");
    assert_eq!(group(&ada.ok("/api/v1/analytics/items?group_by=label").await, "hardware")["value"], 2.0);
    assert_eq!(group(&ada.ok("/api/v1/analytics/items?group_by=project&metric=estimate").await, "Ops")["value"], 6.0);
    assert_eq!(group(&ada.ok("/api/v1/analytics/items?group_by=priority").await, "Critical")["value"], 1.0);
    for bad in ["group_by=nope", "group_by=type,status,priority,label", "metric=secrets", "group_by=type;DROP%20TABLE%20items"] {
        assert_eq!(ada.get(&format!("/api/v1/analytics/items?{bad}")).await.0, StatusCode::BAD_REQUEST, "{bad}");
    }

    let t = ada.ok("/api/v1/analytics/timeline?interval=week&periods=4").await;
    let periods = t["periods"].as_array().unwrap();
    assert_eq!((periods.len(), periods[3]["created"].as_i64(), periods[3]["resolved"].as_i64(), periods[3]["open_at_end"].as_i64(), periods[0]["open_at_end"].as_i64()), (4, Some(5), Some(1), Some(4), Some(0)));
    assert_eq!(ada.ok("/api/v1/analytics/timeline?interval=day&periods=9999").await["periods"].as_array().unwrap().len(), 366, "long ranges are capped");
    assert_eq!(ada.get("/api/v1/analytics/timeline?interval=year").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ada.get("/api/v1/analytics/timeline?periods=many").await.0, StatusCode::BAD_REQUEST);

    let w = ada.ok("/api/v1/analytics/workload").await;
    let who = |name: &str| w["people"].as_array().unwrap().iter().find(|p| p["person"]["name"] == name).cloned().unwrap();
    assert_eq!((who("ben")["open"].as_i64(), who("ben")["done_30d"].as_i64(), who("ben")["overdue"].as_i64(), who("ada")["estimate_open"].as_f64()), (Some(1), Some(1), Some(1), Some(1.0)));
    assert!(w["people"].as_array().unwrap().iter().any(|p| p["person"].is_null() && p["open"] == 2), "unassigned work is counted too: {w}");
    let c = ada.ok("/api/v1/analytics/cycle-time?days=30").await;
    assert_eq!((c["stats"]["count"].as_i64(), c["items"][0]["key"].as_str(), c["items"][0]["type"].as_str()), (Some(1), Some("OPS-5"), Some("story")));
    assert!(c["stats"]["median"].as_f64().unwrap() >= 0.0);
    assert_eq!(ada.get("/api/v1/analytics/cycle-time?days=soon").await.0, StatusCode::BAD_REQUEST);
    assert_eq!(eve.ok("/api/v1/analytics/cycle-time").await["stats"]["count"], 0);
    for path in ["summary", "items", "timeline", "workload", "cycle-time"] {
        assert_eq!(app.anon().get(&format!("/api/v1/analytics/{path}")).await.0, StatusCode::UNAUTHORIZED, "{path}");
    }
}

#[tokio::test]
async fn visualizations_are_saved_shared_and_edited_by_those_their_owner_names() {
    let app = spawn().await;
    let (ada, ben, eve) = team(&app).await;
    let customer = app.user("cust@example.org", "customer").await;
    let auditor = app.user("audit@example.org", "auditor").await;
    let lib = ada.ok("/api/v1/visualizations").await;
    assert_eq!((lib["templates"].as_array().unwrap().len(), lib["can_share"].as_bool(), lib["visualizations"].as_array().unwrap().len()), (12, Some(true), 0));
    assert_eq!(lib["libraries"].as_array().unwrap().iter().map(|l| l["key"].as_str().unwrap()).collect::<Vec<_>>(), ["chartjs", "echarts", "plotly", "d3", "html"]);
    assert_eq!(lib["libraries"][1]["scripts"], json!(["/vendor/echarts/echarts.min.js"]));
    assert!(lib["templates"][0].get("source").is_none(), "the list does not carry every script");
    // Every chart library has ready-made examples, each reading through the script API.
    for l in ["chartjs", "echarts", "plotly", "d3", "html"] {
        let t = lib["templates"].as_array().unwrap().iter().find(|t| t["library"] == l).unwrap_or_else(|| panic!("a built-in visualization for {l}"));
        let full = ada.ok(&format!("/api/v1/visualization-templates/{}", t["key"].as_str().unwrap())).await;
        assert!(full["source"].as_str().unwrap().contains("gantry.api.get("), "{}", t["key"]);
    }
    assert_eq!(ada.get("/api/v1/visualization-templates/nope").await.0, StatusCode::NOT_FOUND);

    // From a built-in one, or from scratch.
    let copy = ada.created("/api/v1/visualizations", json!({"template": "status-donut"})).await;
    assert_eq!((copy["name"].as_str(), copy["library"].as_str(), copy["template"].as_str(), copy["visibility"].as_str(), copy["version"].as_i64(), copy["params"]["project"].as_str()), (Some("Work by status"), Some("chartjs"), Some("status-donut"), Some("private"), Some(1), Some("")));
    assert!(copy["source"].as_str().unwrap().contains("new Chart("));
    let source = "const s = await gantry.api.get('/api/v1/analytics/summary', {project: gantry.params.project});\nd3.select(root).append('p').text(s.open);";
    let own = ada.created("/api/v1/visualizations", json!({"name": "Open work, big", "description": "One number.", "library": "d3", "source": source, "params": {"project": "OPS", "top": 5, "stacked": true}})).await;
    let id = own["id"].as_str().unwrap().to_string();
    assert_eq!((own["mine"].as_bool(), own["can_edit"].as_bool(), own["source"].as_str(), own["owner"]["name"].as_str()), (Some(true), Some(true), Some(source), Some("ada")));
    for (bad, why) in [
        (json!({"name": "X", "library": "vega"}), "library"),
        (json!({"library": "d3"}), "name"),
        (json!({"name": "X"}), "library"),
        (json!({"name": "X", "library": "d3", "params": {"nested": {"a": 1}}}), "simple named values"),
        (json!({"name": "X", "library": "d3", "params": {"bad key!": 1}}), "simple named values"),
        (json!({"name": "X", "library": "d3", "source": "x".repeat(200_001)}), "200 KB"),
        (json!({"name": "X", "library": "d3", "visibility": "public"}), "visibility"),
        (json!({"name": "X", "template": "nope"}), "template"),
        (json!({"name": "X", "library": "d3", "editors": ["00000000-0000-0000-0000-000000000001"]}), "active account"),
    ] {
        let (s, v) = ada.post("/api/v1/visualizations", bad.clone()).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{}", v);
        assert!(v["detail"].as_str().unwrap().contains(why), "{v}");
    }
    assert!(ada.post("/api/v1/visualizations", json!({"name": "X", "library": "d3", "owner_id": ben.user_id})).await.0.is_client_error());
    assert_eq!(customer.post("/api/v1/visualizations", json!({"template": "status-donut"})).await.0, StatusCode::FORBIDDEN);

    // Private until shared with the workspace; sharing is a permission of its own.
    assert_eq!(ben.get(&format!("/api/v1/visualizations/{id}")).await.0, StatusCode::NOT_FOUND);
    assert!(ben.ok("/api/v1/visualizations").await["visualizations"].as_array().unwrap().is_empty());
    assert_eq!(ben.patch(&format!("/api/v1/visualizations/{id}"), json!({"source": "x"})).await.0, StatusCode::NOT_FOUND);
    assert_eq!(auditor.created("/api/v1/visualizations", json!({"template": "kpi-tiles"})).await["visibility"], "private");
    assert_eq!(auditor.post("/api/v1/visualizations", json!({"template": "kpi-tiles", "visibility": "workspace"})).await.0, StatusCode::FORBIDDEN);
    let shared = ada.patch(&format!("/api/v1/visualizations/{id}"), json!({"visibility": "workspace"})).await.1;
    assert_eq!((shared["visibility"].as_str(), shared["version"].as_i64()), (Some("workspace"), Some(2)));
    let seen = ben.ok(&format!("/api/v1/visualizations/{id}")).await;
    assert_eq!((seen["mine"].as_bool(), seen["can_edit"].as_bool(), seen["source"].as_str()), (Some(false), Some(false), Some(source)));
    assert_eq!(ben.patch(&format!("/api/v1/visualizations/{id}"), json!({"source": "document.title = 'hijacked'"})).await.0, StatusCode::FORBIDDEN);
    assert!(customer.ok("/api/v1/visualizations").await["visualizations"].as_array().unwrap().is_empty(), "portal customers are outside the workspace");

    // Editors named by the owner change the script and its parameters; who sees and edits it stays with the owner.
    let with = ada.patch(&format!("/api/v1/visualizations/{id}"), json!({"editors": [ben.user_id]})).await.1;
    assert_eq!(with["editors"][0]["name"], "ben");
    let (s, edited) = ben.patch(&format!("/api/v1/visualizations/{id}"), json!({"source": "root.textContent = 'Ben was here';", "params": {"project": ""}})).await;
    assert_eq!((s, edited["version"].as_i64(), edited["can_edit"].as_bool(), edited["owner"]["name"].as_str()), (StatusCode::OK, Some(4), Some(true), Some("ada")));
    for body in [json!({"visibility": "private"}), json!({"editors": []}), json!({"editors": [ben.user_id, eve.user_id]})] {
        assert_eq!(ben.patch(&format!("/api/v1/visualizations/{id}"), body).await.0, StatusCode::FORBIDDEN);
    }
    assert_eq!(ben.patch(&format!("/api/v1/visualizations/{id}"), json!({"template": "kpi-tiles"})).await.0, StatusCode::BAD_REQUEST);
    assert_eq!(ben.delete(&format!("/api/v1/visualizations/{id}")).await.0, StatusCode::NOT_FOUND, "only its owner deletes it");
    // An editor still sees it after it is made private again; everyone else loses it.
    assert_eq!(ada.patch(&format!("/api/v1/visualizations/{id}"), json!({"visibility": "private"})).await.0, StatusCode::OK);
    assert_eq!(ben.ok(&format!("/api/v1/visualizations/{id}")).await["can_edit"], true);
    assert_eq!(eve.get(&format!("/api/v1/visualizations/{id}")).await.0, StatusCode::NOT_FOUND);
    // The script runs in other people's browsers, so every change to it is on record.
    let logged: Value = sqlx::query_scalar("SELECT detail FROM audit_log WHERE action = 'visualization.update' AND detail->>'source_changed' = 'true' ORDER BY at DESC LIMIT 1").fetch_one(&app.st.db).await.unwrap();
    assert_eq!(logged["source_bytes"], 34);
    // Editing the script together: the owner and editors write, others who can see it follow.
    assert_eq!(ada.patch(&format!("/api/v1/visualizations/{id}"), json!({"visibility": "workspace"})).await.0, StatusCode::OK);
    let (mut b, mut e) = (Ws::connect(&app, &ben).await, Ws::connect(&app, &eve).await);
    assert_eq!(b.subscribe(&format!("viz:{id}")).await["write"], true);
    assert_eq!(e.subscribe(&format!("viz:{id}")).await["write"], false);
    assert_eq!(e.subscribe(&format!("viz:{}", copy["id"].as_str().unwrap())).await["t"], "denied", "a private one has no room for others");
    assert_eq!(ada.patch(&format!("/api/v1/visualizations/{id}"), json!({"name": "Renamed"})).await.0, StatusCode::OK);
    assert_eq!(e.until(|m| m["t"] == "event").await["kind"], "visualization.updated");
    assert_eq!(ada.delete(&format!("/api/v1/visualizations/{id}")).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ben.get(&format!("/api/v1/visualizations/{id}")).await.0, StatusCode::NOT_FOUND);
    for a in ["visualization.create", "visualization.update", "visualization.delete"] {
        assert!(app.audit_actions().await.contains(&a.to_string()), "{a}");
    }
}

#[tokio::test]
async fn dashboards_are_arranged_tile_by_tile_and_show_each_viewer_their_own_data() {
    let app = spawn().await;
    let (ada, ben, eve) = team(&app).await;
    let private_viz = ada.created("/api/v1/visualizations", json!({"name": "Ada's own", "library": "html", "source": "<p>Mine</p>"})).await["id"].as_str().unwrap().to_string();
    let shared_viz = ada.created("/api/v1/visualizations", json!({"template": "workload-stacked", "visibility": "workspace"})).await["id"].as_str().unwrap().to_string();
    let d = ada.created("/api/v1/dashboards", json!({"name": "Ops room", "description": "The state of the line.", "params": {"project": "OPS"}, "layout": [
        {"id": "figures", "kind": "template", "ref": "kpi-tiles", "x": 0, "y": 0, "w": 6, "h": 2, "params": {"project": "OPS"}},
        {"id": "own", "kind": "visualization", "ref": private_viz, "title": "Notes", "x": 6, "y": 0, "w": 6, "h": 2},
        {"id": "load", "kind": "visualization", "ref": shared_viz, "x": 0, "y": 2, "w": 12, "h": 5}]})).await;
    let id = d["id"].as_str().unwrap().to_string();
    assert_eq!((d["tiles"].as_i64(), d["mine"].as_bool(), d["visibility"].as_str(), d["version"].as_i64()), (Some(3), Some(true), Some("private"), Some(1)));
    let tile = |d: &Value, id: &str| d["layout"].as_array().unwrap().iter().find(|t| t["id"] == id).cloned().unwrap_or(Value::Null);
    // Each tile arrives with what is needed to draw it.
    assert_eq!((tile(&d, "figures")["content"]["builtin"].as_bool(), tile(&d, "figures")["content"]["library"].as_str(), tile(&d, "figures")["content"]["params"]["project"].as_str()), (Some(true), Some("html"), Some("")));
    assert!(tile(&d, "figures")["content"]["source"].as_str().unwrap().contains("gantry.api.get("));
    assert_eq!((tile(&d, "own")["content"]["source"].as_str(), tile(&d, "own")["title"].as_str(), tile(&d, "load")["content"]["library"].as_str()), (Some("<p>Mine</p>"), Some("Notes"), Some("echarts")));
    for (bad, why) in [
        (json!([{"id": "a", "kind": "template", "ref": "kpi-tiles", "x": 8, "w": 6}]), "twelfth column"),
        (json!([{"id": "a", "kind": "template", "ref": "kpi-tiles"}, {"id": "a", "kind": "template", "ref": "due-table"}]), "same id"),
        (json!([{"id": "a", "kind": "template", "ref": "nope"}]), "unknown built-in"),
        (json!([{"id": "a", "kind": "iframe", "ref": "https://evil.example"}]), "kind"),
        (json!([{"id": "a b", "kind": "template", "ref": "kpi-tiles"}]), "id"),
        (json!([{"id": "a", "kind": "template", "ref": "kpi-tiles", "h": 0}]), "between 1 and 12"),
        (json!([{"id": "a", "kind": "template", "ref": "kpi-tiles", "html": "<script>"}]), "unknown tile field"),
        (json!([{"id": "a", "kind": "template", "ref": "kpi-tiles", "params": {"q": {"$where": 1}}}]), "simple named values"),
        (json!([{"id": "a", "kind": "visualization", "ref": "not-an-id"}]), "visualization id"),
        (json!((0..41).map(|n| json!({"id": format!("t{n}"), "kind": "template", "ref": "kpi-tiles"})).collect::<Vec<_>>()), "at most 40"),
    ] {
        let (s, v) = ada.patch(&format!("/api/v1/dashboards/{id}"), json!({"layout": bad})).await;
        assert_eq!(s, StatusCode::BAD_REQUEST, "{v}");
        assert!(v["detail"].as_str().unwrap().contains(why), "{v}");
    }
    assert_eq!(ada.ok(&format!("/api/v1/dashboards/{id}")).await["version"], 1, "refused changes change nothing");

    // Private until shared. A viewer gets every tile they may see; the owner's private one arrives empty.
    assert_eq!(ben.get(&format!("/api/v1/dashboards/{id}")).await.0, StatusCode::NOT_FOUND);
    assert!(ben.ok("/api/v1/dashboards").await["dashboards"].as_array().unwrap().is_empty());
    assert_eq!(ada.patch(&format!("/api/v1/dashboards/{id}"), json!({"visibility": "workspace"})).await.0, StatusCode::OK);
    let seen = ben.ok(&format!("/api/v1/dashboards/{id}")).await;
    assert_eq!((seen["can_edit"].as_bool(), seen["mine"].as_bool(), tile(&seen, "own")["content"].clone(), tile(&seen, "own")["title"].as_str()), (Some(false), Some(false), Value::Null, Some("Notes")));
    assert!(!seen.to_string().contains("<p>Mine</p>"));
    assert!(tile(&seen, "load")["content"]["source"].is_string());
    assert_eq!(ben.put(&format!("/api/v1/dashboards/{id}/tiles/figures"), json!({"x": 6})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(ben.patch(&format!("/api/v1/dashboards/{id}"), json!({"name": "Ben's"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(ben.delete(&format!("/api/v1/dashboards/{id}/tiles/figures")).await.0, StatusCode::FORBIDDEN);

    // An editor arranges tiles one at a time. Only what the editor can open may be placed anew.
    assert_eq!(ada.patch(&format!("/api/v1/dashboards/{id}"), json!({"editors": [ben.user_id]})).await.1["editors"][0]["name"], "ben");
    let (s, added) = ben.put(&format!("/api/v1/dashboards/{id}/tiles/due"), json!({"kind": "template", "ref": "due-table", "y": 7, "params": {"rows": 5}})).await;
    assert_eq!((s, added["w"].as_i64(), added["h"].as_i64(), added["x"].as_i64(), added["params"]["rows"].as_i64()), (StatusCode::OK, Some(4), Some(4), Some(0), Some(5)));
    let moved = ben.put(&format!("/api/v1/dashboards/{id}/tiles/figures"), json!({"h": 3})).await.1;
    assert_eq!((moved["h"].as_i64(), moved["w"].as_i64(), moved["ref"].as_str(), moved["params"]["project"].as_str()), (Some(3), Some(6), Some("kpi-tiles"), Some("OPS")), "what was not sent keeps its value");
    assert_eq!(ben.put(&format!("/api/v1/dashboards/{id}/tiles/own"), json!({"w": 4})).await.0, StatusCode::OK, "a tile already there can be resized without being able to open it");
    let (s, v) = ben.put(&format!("/api/v1/dashboards/{id}/tiles/sneak"), json!({"kind": "visualization", "ref": private_viz})).await;
    assert_eq!(s, StatusCode::OK, "it is already on this dashboard: {v}");
    let eves = eve.created("/api/v1/visualizations", json!({"name": "Eve's own", "library": "html", "source": "<p>Eve's</p>"})).await["id"].as_str().unwrap().to_string();
    let (s, v) = ben.put(&format!("/api/v1/dashboards/{id}/tiles/stolen"), json!({"kind": "visualization", "ref": eves})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["detail"].as_str().unwrap().contains("not shared with you"), "{v}");
    assert_eq!(ben.put(&format!("/api/v1/dashboards/{id}/tiles/figures"), json!({"x": 9})).await.0, StatusCode::BAD_REQUEST, "six wide from column nine does not fit");
    for body in [json!({"visibility": "private"}), json!({"editors": []})] {
        assert_eq!(ben.patch(&format!("/api/v1/dashboards/{id}"), body).await.0, StatusCode::FORBIDDEN, "who sees and edits it is the owner's decision");
    }
    // Two people moving different tiles at the same moment both keep their change.
    let (load, due) = (format!("/api/v1/dashboards/{id}/tiles/load"), format!("/api/v1/dashboards/{id}/tiles/due"));
    let (a, b) = tokio::join!(ada.put(&load, json!({"y": 9})), ben.put(&due, json!({"x": 8})));
    assert_eq!((a.0, b.0), (StatusCode::OK, StatusCode::OK));
    let now = ada.ok(&format!("/api/v1/dashboards/{id}")).await;
    assert_eq!((tile(&now, "load")["y"].as_i64(), tile(&now, "due")["x"].as_i64(), now["tiles"].as_i64()), (Some(9), Some(8), Some(5)));
    assert_eq!(ben.delete(&format!("/api/v1/dashboards/{id}/tiles/sneak")).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ben.delete(&format!("/api/v1/dashboards/{id}/tiles/sneak")).await.0, StatusCode::NOT_FOUND);

    // Those with the dashboard open learn of each change, and see who else is arranging it.
    let (mut b, mut e) = (Ws::connect(&app, &ben).await, Ws::connect(&app, &eve).await);
    assert_eq!(b.subscribe(&format!("dashboard:{id}")).await["write"], true);
    let joined = e.subscribe(&format!("dashboard:{id}")).await;
    assert_eq!((joined["write"].as_bool(), joined["peers"][0]["user"]["name"].as_str()), (Some(false), Some("ben")));
    assert_eq!(ada.put(&format!("/api/v1/dashboards/{id}/tiles/due"), json!({"title": "Due next"})).await.0, StatusCode::OK);
    let ev = e.until(|m| m["t"] == "event").await;
    assert_eq!((ev["kind"].as_str(), ev["data"]["id"].as_str()), (Some("dashboard.updated"), Some(id.as_str())));
    assert!(!ev.to_string().contains("Due next"), "the hint carries no content");

    // The data behind every tile is fetched by the viewer, as the viewer: the same dashboard
    // shows Eve nothing about a project she cannot see.
    assert_eq!(eve.ok("/api/v1/analytics/summary?project=OPS").await["total"], 0);
    assert_eq!(ben.delete(&format!("/api/v1/dashboards/{id}")).await.0, StatusCode::NOT_FOUND, "only its owner deletes it");
    assert_eq!(ada.delete(&format!("/api/v1/dashboards/{id}")).await.0, StatusCode::NO_CONTENT);
    assert_eq!(ada.get(&format!("/api/v1/dashboards/{id}")).await.0, StatusCode::NOT_FOUND);
    for a in ["dashboard.create", "dashboard.update", "dashboard.delete"] {
        assert!(app.audit_actions().await.contains(&a.to_string()), "{a}");
    }
}

#[tokio::test]
async fn custom_content_runs_in_a_frame_that_can_reach_nothing_and_may_only_read() {
    let app = spawn().await;
    let (ada, _, _) = team(&app).await;
    ada.created("/api/v1/projects/OPS/items", json!({"title": "Untouched"})).await;
    // The frame's document is public and holds no data. Its policy is what confines a script.
    let r = app.http.get(format!("{}/api/v1/sandbox/frame", app.base)).send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    let h = r.headers().clone();
    let csp = h["content-security-policy"].to_str().unwrap().to_string();
    for part in ["default-src 'none'", "connect-src 'none'", "form-action 'none'", "base-uri 'none'", "frame-src 'none'", "worker-src 'none'", "object-src 'none'", "frame-ancestors 'self'", "img-src data: blob:"] {
        assert!(csp.contains(part), "{part} in {csp}");
    }
    assert!(!csp.contains("http") && !csp.contains('*'), "no other site is allowed for anything: {csp}");
    assert!(h.get("x-frame-options").is_none(), "this one document may be framed, by the application alone");
    assert_eq!(h["cross-origin-resource-policy"], "same-origin");
    assert_eq!(h["x-content-type-options"], "nosniff");
    let body = r.text().await.unwrap();
    assert!(body.contains("gantry-sandbox") && body.contains("type: 'api'") && !body.contains("fetch(") && !body.contains("XMLHttpRequest") && !body.contains("cookie"));
    // Every other page refuses to be framed, runs no inline or evaluated script, and may frame only itself.
    let r = app.http.get(format!("{}/", app.base)).send().await.unwrap();
    let csp = r.headers()["content-security-policy"].to_str().unwrap().to_string();
    assert_eq!(r.headers()["x-frame-options"], "DENY");
    assert!(csp.contains("frame-ancestors 'none'") && csp.contains("frame-src 'self'") && csp.contains("script-src 'self';") && !csp.contains("unsafe-eval"), "{csp}");

    // The page asks for data on a script's behalf and marks those requests. Marked requests read; they never change anything.
    let marked = |m: Method, path: &str| ada.request(m, path).header("x-gantry-sandbox", "1");
    assert_eq!(marked(Method::GET, "/api/v1/analytics/summary").send().await.unwrap().status(), StatusCode::OK);
    assert_eq!(marked(Method::GET, "/api/v1/items/OPS-1").send().await.unwrap().status(), StatusCode::OK);
    for (m, path, body) in [
        (Method::PATCH, "/api/v1/items/OPS-1", json!({"title": "Changed by a chart"})),
        (Method::POST, "/api/v1/projects/OPS/items", json!({"title": "Created by a chart"})),
        (Method::POST, "/api/v1/items/OPS-1/comments", json!({"body": "Said by a chart"})),
        (Method::POST, "/api/v1/items/query", json!({})),
        (Method::PATCH, "/api/v1/auth/me", json!({"name": "Renamed by a chart"})),
        (Method::DELETE, "/api/v1/auth/me/avatar", json!({})),
        (Method::DELETE, "/api/v1/items/OPS-1/watch", json!({})),
        (Method::POST, "/api/v1/auth/tokens", json!({"name": "exfil", "scopes": ["read", "write"]})),
        (Method::POST, "/api/v1/auth/logout", json!({})),
    ] {
        let r = marked(m.clone(), path).json(&body).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::FORBIDDEN, "{m} {path}");
        assert!(r.json::<Value>().await.unwrap()["detail"].as_str().unwrap().contains("read data but not change it"));
    }
    let item = ada.ok("/api/v1/items/OPS-1").await;
    assert_eq!((item["title"].as_str(), item["comments"].as_array().unwrap().len()), (Some("Untouched"), 0));
    assert_eq!(ada.ok("/api/v1/auth/me").await["user"]["id"], json!(ada.user_id), "still signed in");
    assert!(app.security_kinds().await.contains(&"sandbox.write_refused".to_string()));
    // The mark restricts; it never grants. Without a session it changes nothing about who is asking.
    let r = app.http.get(format!("{}/api/v1/analytics/summary", app.base)).header("x-gantry-sandbox", "1").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
}

// ───────────────────────── about ─────────────────────────

#[tokio::test]
async fn the_application_says_who_runs_it_what_the_rules_are_and_what_it_is_made_of() {
    let app = spawn().await;
    let root = app.admin().await;
    let mia = app.user("mia@example.org", "member").await;
    let planner = app.user("planner@example.org", "app_admin").await;
    // Before sign-in: the name, the statements and how to get help. Not the exact version.
    let about = app.anon().ok("/api/v1/about").await;
    assert_eq!((about["site"]["name"].as_str(), about["product"]["name"].as_str(), about["product"]["license"].as_str()), (Some("Gantry"), Some("Gantry"), Some("Apache-2.0")));
    assert!(about["product"].get("version").is_none(), "{about}");
    assert_eq!(about["documents"].as_array().unwrap().iter().map(|d| d["key"].as_str().unwrap()).collect::<Vec<_>>(), ["privacy", "terms", "cookies", "retention", "security"]);
    assert!(about["documents"].as_array().unwrap().iter().all(|d| d["text"] == "" && d["title"].is_string()));
    assert_eq!(mia.ok("/api/v1/about").await["product"]["version"], gantry::VERSION);
    assert_eq!(app.anon().ok("/api/v1/auth/providers").await["legal"], json!([]));

    // An administrator writes the statements and the support contact.
    assert_eq!(mia.put("/api/v1/admin/settings/legal", json!({"privacy": "x"})).await.0, StatusCode::FORBIDDEN);
    assert_eq!(planner.put("/api/v1/admin/settings/site", json!({"name": "Mine"})).await.0, StatusCode::FORBIDDEN, "running the application is not the same as owning the installation");
    assert_eq!(root.put("/api/v1/admin/settings/legal", json!({"privacy": "We keep **as little** as the work needs.", "retention": "Audit records are kept for 400 days."})).await.0, StatusCode::NO_CONTENT);
    assert_eq!(root.put("/api/v1/admin/settings/site", json!({"name": "Mission Control", "login_notice": "Authorized use only.", "support": {"name": "Help desk", "email": "help@example.org", "url": "https://help.example.org/start"}})).await.0, StatusCode::NO_CONTENT);
    // A later change to one field keeps the others.
    assert_eq!(root.put("/api/v1/admin/settings/site", json!({"support": {"text": "Weekdays, 08:00 to 17:00 Central."}})).await.0, StatusCode::NO_CONTENT);
    let about = app.anon().ok("/api/v1/about").await;
    assert_eq!(about["site"], json!({"name": "Mission Control", "support": {"name": "Help desk", "email": "help@example.org", "url": "https://help.example.org/start", "text": "Weekdays, 08:00 to 17:00 Central."}}));
    let doc = |key: &str| about["documents"].as_array().unwrap().iter().find(|d| d["key"] == key).unwrap()["text"].as_str().unwrap().to_string();
    assert_eq!((doc("privacy").as_str(), doc("terms").as_str()), ("We keep **as little** as the work needs.", ""));
    let login = app.anon().ok("/api/v1/auth/providers").await;
    assert_eq!((login["legal"].clone(), login["site"]["login_notice"].as_str(), login["site"]["support"]["email"].as_str()), (json!(["privacy", "retention"]), Some("Authorized use only."), Some("help@example.org")));
    for (key, bad) in [
        ("site", json!({"support": {"email": "not an address"}})),
        ("site", json!({"support": {"url": "javascript:alert(1)"}})),
        ("site", json!({"timezone": "Mars/Olympus_Mons"})),
        ("site", json!({"theme": "hotdog"})),
        ("site", json!("Mission Control")),
        ("legal", json!({"imprint": "x"})),
        ("legal", json!({"privacy": 7})),
        ("nope", json!({})),
    ] {
        assert_eq!(root.put(&format!("/api/v1/admin/settings/{key}"), bad.clone()).await.0, StatusCode::BAD_REQUEST, "{key}: {bad}");
    }
    // Each administrator sees the settings that are theirs to change.
    let theirs = planner.ok("/api/v1/admin/settings").await;
    let mut keys: Vec<&str> = theirs["stored"].as_object().unwrap().keys().map(String::as_str).collect();
    keys.sort();
    assert_eq!(keys, ["hierarchy", "notifications", "profile"]);
    assert!(root.ok("/api/v1/admin/settings").await["stored"]["legal"]["retention"].as_str().unwrap().contains("400 days"));
    assert_eq!(mia.get("/api/v1/admin/settings").await.0, StatusCode::FORBIDDEN);
    // The log records that a statement changed and how long it is, not its text.
    let logged: Value = sqlx::query_scalar("SELECT detail FROM audit_log WHERE action = 'settings.update' AND target_id = 'legal' LIMIT 1").fetch_one(&app.st.db).await.unwrap();
    assert_eq!(logged["changed"]["retention"], 36);
    assert!(!logged.to_string().contains("400 days"));

    // The parts of the installation, for signed-in people.
    assert_eq!(app.anon().get("/api/v1/about/components").await.0, StatusCode::UNAUTHORIZED);
    std::fs::write(app.st.cfg.server.web_root.join("build.json"), r#"{"version": "0.2.0", "date": "2026-11-11"}"#).unwrap();
    let c = mia.ok("/api/v1/about/components").await;
    let part = |key: &str| c["components"].as_array().unwrap().iter().find(|x| x["key"] == key).cloned().unwrap();
    assert_eq!((part("server")["version"].as_str(), part("web")["version"].as_str(), part("web")["date"].as_str(), part("manual")["version"].clone()), (Some(gantry::VERSION), Some("0.2.0"), Some("2026-11-11"), Value::Null));
    assert!(c["components"].as_array().unwrap().iter().all(|x| x["summary"].is_string() && x["description"].is_string() && x["name"].is_string()));
    assert_eq!((c["addons"][0]["id"].as_str(), c["license"]["third_party"].as_str()), (Some("core"), Some("/licenses.json")));
    assert!(c["addons"][0]["version"].is_string() && c["addons"][0]["installed_at"].is_string() && c["storage"]["files"].is_string());
    // Installing an add-on adds it to the list with its own version and description.
    assert!(root.post("/api/v1/admin/packs/nasa/install", json!({})).await.0.is_success());
    let c = mia.ok("/api/v1/about/components").await;
    let nasa = c["addons"].as_array().unwrap().iter().find(|a| a["id"] == "nasa").cloned().expect("the add-on is listed");
    assert!(nasa["version"].is_string() && !nasa["description"].as_str().unwrap().is_empty(), "{nasa}");
}

/// A person who opens a failing address in a browser gets a page they can read, with the
/// status code kept; programs keep getting problem documents. The web application reads the
/// same texts for its own error pages.
#[tokio::test]
async fn a_browser_gets_a_readable_page_for_every_failure_and_programs_get_json() {
    let app = spawn_with(|c| c.server.public_url = "https://gantry.example.org".into()).await;
    let root = app.admin().await;
    assert_eq!(root.put("/api/v1/admin/settings/site", json!({"name": "Mission <Control>", "support": {"name": "Help desk", "email": "help@example.org", "url": "https://help.example.org/start", "text": "Weekdays, 08:00 to 17:00."}})).await.0, StatusCode::NO_CONTENT);
    let mia = app.user("mia@example.org", "member").await;
    let browser = |c: &Caller, path: &str| c.request(Method::GET, path).header("accept", "text/html,application/xhtml+xml,*/*;q=0.8").header("sec-fetch-mode", "navigate").header("sec-fetch-dest", "document");

    // Not found, as a signed-in person: the page names the site, says what to do, and how to reach support.
    let r = browser(&mia, "/api/v1/items/NOPE-1").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert!(r.headers()["content-type"].to_str().unwrap().starts_with("text/html"));
    assert_eq!(r.headers()["cache-control"], "no-store");
    assert!(r.headers().contains_key("content-security-policy") && r.headers().contains_key("x-request-id"));
    let id = r.headers()["x-request-id"].to_str().unwrap().to_string();
    let html = r.text().await.unwrap();
    for must in ["Error 404", "This does not exist, or you cannot see it", "Mission &lt;Control&gt;", "Help desk", "mailto:help@example.org", "https://help.example.org/start", "Weekdays, 08:00 to 17:00.", "Go to my work", id.as_str()] {
        assert!(html.contains(must), "the page lacks {must:?}:\n{html}");
    }
    assert!(!html.contains("<Control>"), "the site name is escaped");

    // Not signed in: the page offers to sign in, and reveals nothing about what is behind the address.
    let r = browser(&app.anon(), "/api/v1/items/NOPE-1").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::UNAUTHORIZED);
    let html = r.text().await.unwrap();
    assert!(html.contains("You need to sign in") && html.contains("href=\"/login\"") && !html.contains("Go to my work"));

    // Forbidden, and a manual page that does not exist.
    let r = browser(&mia, "/api/v1/admin/settings").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::FORBIDDEN);
    assert!(r.text().await.unwrap().contains("You do not have access to this"));
    let r = browser(&mia, "/manual/no-such-page.html").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::NOT_FOUND);
    assert!(r.text().await.unwrap().contains("Error 404"));

    // The same addresses asked for by a program, or by the web application, answer in JSON.
    for c in [&mia, &app.anon()] {
        let r = c.request(Method::GET, "/api/v1/items/NOPE-1").header("accept", "application/json").send().await.unwrap();
        assert!(r.headers()["content-type"].to_str().unwrap().contains("json"), "{:?}", r.headers());
    }
    let r = mia.request(Method::GET, "/api/v1/items/NOPE-1").header("accept", "text/html").header("sec-fetch-mode", "cors").send().await.unwrap();
    assert!(r.headers()["content-type"].to_str().unwrap().contains("json"), "a fetch from a page is not a navigation");
    // Changing something is never answered with a page.
    let r = mia.request(Method::POST, "/api/v1/items/NOPE-1/comments").header("accept", "text/html").header("sec-fetch-mode", "navigate").json(&json!({"body": "x"})).send().await.unwrap();
    assert!(r.headers()["content-type"].to_str().unwrap().contains("json"));
    // A page that works is left alone.
    let r = browser(&mia, "/api/v1/auth/me").send().await.unwrap();
    assert_eq!(r.status(), StatusCode::OK);
    assert!(r.headers()["content-type"].to_str().unwrap().contains("json"));

    // The texts, for the web application: public, complete, with the support contact.
    let codes = app.anon().ok("/api/v1/about/status-codes").await;
    let list = codes["codes"].as_array().unwrap();
    for must in [400, 401, 403, 404, 409, 413, 422, 429, 500, 502, 503, 504] {
        let c = list.iter().find(|c| c["code"] == must).unwrap_or_else(|| panic!("{must} is listed"));
        assert!(c["title"].is_string() && c["meaning"].is_string() && c["action"].is_string());
    }
    assert_eq!(codes["support"]["email"], "help@example.org");

    // The application's own page: link previews get absolute addresses when the public address is known.
    std::fs::write(app.st.cfg.server.web_root.join("index.html"), "<html><head><meta property=\"og:type\" content=\"website\" /><meta property=\"og:image\" content=\"/og.png\" /></head><body><div id=\"root\"></div></body></html>").unwrap();
    for path in ["/", "/planning/graph", "/i/OPS-1"] {
        let r = browser(&app.anon(), path).send().await.unwrap();
        assert_eq!(r.status(), StatusCode::OK, "{path}");
        let html = r.text().await.unwrap();
        assert!(html.contains("content=\"https://gantry.example.org/og.png\"") && html.contains("<meta property=\"og:url\" content=\"https://gantry.example.org/\" />"), "{path}: {html}");
    }

    // The overview lists this node, and says how email and live updates stand.
    let o = root.ok("/api/v1/admin/overview").await;
    let part = |name: &str| o["components"].as_array().unwrap().iter().find(|c| c["name"] == name).cloned().unwrap_or_else(|| panic!("{name} is listed"));
    assert!(part("Email")["backend"].as_str().unwrap().starts_with("off"), "{}", part("Email"));
    assert!(part("Live updates")["backend"].as_str().unwrap().contains("connection"));
    assert!(o["nodes"].is_array());

    // A project that designs its own workflow still shows the shared one it came from.
    let (s, _) = root.post("/api/v1/projects", json!({"key": "OPS", "name": "Operations", "template": "software"})).await;
    assert_eq!(s, StatusCode::CREATED);
    let p = root.ok("/api/v1/projects/OPS").await;
    let bug = p["types"].as_array().unwrap().iter().find(|t| t["key"] == "bug").cloned().unwrap();
    assert_eq!((bug["workflow"]["own"].as_bool(), bug["shared_workflow"].clone()), (Some(false), Value::Null));
    let mut def = json!({"initial": bug["workflow"]["initial"], "statuses": bug["workflow"]["statuses"], "transitions": bug["workflow"]["transitions"], "layout": {}});
    def["layout"][bug["workflow"]["initial"].as_str().unwrap()] = json!({"x": 40, "y": 60});
    assert_eq!(root.put("/api/v1/projects/OPS/workflows/bug", json!({"definition": def})).await.0, StatusCode::OK);
    let p = root.ok("/api/v1/projects/OPS").await;
    let bug = p["types"].as_array().unwrap().iter().find(|t| t["key"] == "bug").cloned().unwrap();
    assert_eq!(bug["workflow"]["own"], true);
    assert_eq!(bug["workflow"]["layout"][bug["workflow"]["initial"].as_str().unwrap()], json!({"x": 40, "y": 60}));
    assert!(bug["shared_workflow"]["statuses"].is_array(), "{bug}");
}
