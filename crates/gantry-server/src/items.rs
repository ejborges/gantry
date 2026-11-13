//! Work items: the unit of tracking for every domain. Types, fields and workflows are data
//! (see `packs.rs`); this module enforces them. The functions taking `(&St, &Ctx, ...)` carry
//! their own permission checks and are shared by the REST handlers, the MCP tools and integrations.

use crate::auth::Ctx;
use crate::authz::{self, Perms};
use crate::error::{bad, AppError, AppResult};
use crate::events::{self, Event};
use crate::http::Api;
use crate::packs::WorkflowDef;
use crate::projects::{self, Project};
use crate::storage::SearchDoc;
use crate::{audit, crypto, desk, jobs, AppState, St};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, post};
use axum::Json;
use chrono::{DateTime, NaiveDate, Utc};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::{Postgres, QueryBuilder, Row};
use std::collections::HashMap;
use uuid::Uuid;

pub const ITEM_JSON: &str = "jsonb_build_object(
  'id', i.id, 'key', p.key || '-' || i.number, 'project', p.key, 'number', i.number, 'type', i.type_key,
  'type_name', ty.name, 'type_icon', ty.icon, 'type_color', ty.color,
  'title', i.title, 'status', i.status, 'status_category', i.status_category, 'priority', i.priority,
  'status_name', item_status_name(i.project_id, i.type_key, i.status),
  'assignee', person_json(a), 'reporter', person_json(r),
  'parent_id', i.parent_id, 'labels', i.labels, 'start_date', i.start_date, 'due_date', i.due_date, 'estimate', i.estimate,
  'cycle_id', i.cycle_id, 'rank', i.rank, 'channel', i.channel, 'created_at', i.created_at, 'updated_at', i.updated_at, 'resolved_at', i.resolved_at,
  'sla', CASE WHEN i.sla_resolution_due IS NULL AND i.sla_response_due IS NULL THEN NULL ELSE jsonb_build_object(
     'response_due', i.sla_response_due, 'first_response_at', i.first_response_at, 'resolution_due', i.sla_resolution_due,
     'response_breached', i.sla_response_breached OR (i.first_response_at IS NULL AND i.sla_response_due < now()),
     'resolution_breached', i.sla_resolution_breached OR (i.resolved_at IS NULL AND i.sla_resolution_due < now())) END)";
pub const ITEM_FROM: &str = "FROM items i JOIN projects p ON p.id = i.project_id JOIN item_types ty ON ty.key = i.type_key LEFT JOIN users a ON a.id = i.assignee_id LEFT JOIN users r ON r.id = i.reporter_id";

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Item {
    pub id: Uuid,
    pub project_id: Uuid,
    pub number: i32,
    pub type_key: String,
    pub title: String,
    pub description: String,
    pub status: String,
    pub status_category: String,
    pub priority: i16,
    pub assignee_id: Option<Uuid>,
    pub reporter_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub labels: Vec<String>,
    pub start_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub estimate: Option<f64>,
    pub cycle_id: Option<Uuid>,
    pub custom: Value,
    pub refs: Value,
    pub first_response_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub archived: bool,
}
const ITEM_COLS: &str = "i.id, i.project_id, i.number, i.type_key, i.title, i.description, i.status, i.status_category, i.priority, i.assignee_id, i.reporter_id, i.parent_id, i.labels, i.start_date, i.due_date, i.estimate, i.cycle_id, i.custom, i.refs, i.first_response_at, i.created_at, i.archived";

/// Split `OPS-12` into (`OPS`, 12).
pub fn parse_key(key: &str) -> Option<(String, i32)> {
    let (p, n) = key.trim().rsplit_once('-')?;
    let p = p.to_uppercase();
    (projects::valid_project_key(&p) && !n.is_empty() && n.len() <= 9 && n.bytes().all(|b| b.is_ascii_digit())).then(|| (p, n.parse().ok())).and_then(|(p, n)| Some((p, n?)))
}

pub async fn find_item(st: &AppState, key: &str) -> AppResult<Option<Item>> {
    let Some((pkey, number)) = parse_key(key) else { return Ok(None) };
    Ok(sqlx::query_as(&format!("SELECT {ITEM_COLS} FROM items i JOIN projects p ON p.id = i.project_id WHERE p.key = $1 AND i.number = $2 AND NOT i.archived")).bind(pkey).bind(number).fetch_optional(&st.db).await?)
}

pub async fn item_by_id(st: &AppState, id: Uuid) -> AppResult<Option<Item>> {
    Ok(sqlx::query_as(&format!("SELECT {ITEM_COLS} FROM items i WHERE i.id = $1")).bind(id).fetch_optional(&st.db).await?)
}

/// Load an item the caller may view as a team member. Anything else is a 404.
pub async fn load_item(st: &St, ctx: &Ctx, key: &str) -> AppResult<(Item, Project, Perms)> {
    let item = find_item(st, key).await?.ok_or(AppError::NotFound)?;
    let project = projects::project_by_id(st, item.project_id).await?;
    let perms = authz::project_perms(st, ctx, project.id, &project.access, project.portal_open()).await?;
    if !perms.has("project.view") {
        return Err(authz::hide(st, ctx, "project.view", &format!("item:{key}")));
    }
    Ok((item, project, perms))
}

pub struct FieldInfo {
    pub key: String,
    pub name: String,
    pub kind: String,
    pub options: Vec<String>,
    pub required: bool,
}

pub struct TypeInfo {
    pub key: String,
    pub name: String,
    pub workflow: WorkflowDef,
    pub fields: Vec<FieldInfo>,
}

/// A work item type with the workflow it follows in the shared scheme.
pub async fn load_type(st: &AppState, type_key: &str) -> AppResult<TypeInfo> {
    load_type_in(st, None, type_key).await
}

/// A work item type as it applies in one project: the project's own workflow for the type
/// if it defined one, otherwise the shared workflow.
pub async fn load_type_in(st: &AppState, project: Option<Uuid>, type_key: &str) -> AppResult<TypeInfo> {
    let row = sqlx::query(
        "SELECT t.key, t.name, t.fields, w.key AS wkey, w.name AS wname, coalesce(pw.definition, w.definition) AS definition, pw.project_id IS NOT NULL AS own
         FROM item_types t JOIN workflows w ON w.key = t.workflow_key LEFT JOIN project_workflows pw ON pw.type_key = t.key AND pw.project_id = $2 WHERE t.key = $1",
    )
    .bind(type_key)
    .bind(project)
    .fetch_optional(&st.db)
    .await?
    .ok_or_else(|| bad(format!("unknown work item type {type_key:?}")))?;
    let mut workflow: WorkflowDef = serde_json::from_value(row.get("definition")).map_err(|e| AppError::Other(anyhow::anyhow!("workflow for {type_key} is corrupt: {e}")))?;
    workflow.key = row.get("wkey");
    workflow.name = if row.get::<bool, _>("own") { format!("{} (this project)", row.get::<String, _>("wname")) } else { row.get("wname") };
    let refs: Vec<Value> = serde_json::from_value(row.get("fields")).unwrap_or_default();
    let wanted: Vec<String> = refs.iter().filter_map(|r| r["key"].as_str().map(str::to_string)).collect();
    let defs = sqlx::query("SELECT key, name, type, options FROM fields WHERE key = ANY($1)").bind(&wanted).fetch_all(&st.db).await?;
    let by_key: HashMap<String, &sqlx::postgres::PgRow> = defs.iter().map(|d| (d.get::<String, _>("key"), d)).collect();
    let fields = refs
        .iter()
        .filter_map(|r| {
            let d = by_key.get(r["key"].as_str()?)?;
            Some(FieldInfo { key: d.get("key"), name: d.get("name"), kind: d.get("type"), options: serde_json::from_value(d.get("options")).unwrap_or_default(), required: r["required"].as_bool().unwrap_or(false) })
        })
        .collect();
    Ok(TypeInfo { key: row.get("key"), name: row.get("name"), workflow, fields })
}

pub fn is_filled(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::String(s)) => !s.trim().is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(_) => true,
    }
}

/// Check custom field values against the type's field definitions.
pub fn validate_custom(ti: &TypeInfo, custom: &Map<String, Value>, check_required: bool) -> AppResult<()> {
    for (k, v) in custom {
        let f = ti.fields.iter().find(|f| &f.key == k).ok_or_else(|| bad(format!("{:?} is not a field of {}", k, ti.name)))?;
        if v.is_null() {
            continue;
        }
        let ok = match f.kind.as_str() {
            "text" | "textarea" => v.as_str().map(|s| s.len() <= 100_000).unwrap_or(false),
            "url" => v.as_str().and_then(|s| url::Url::parse(s).ok()).map(|u| matches!(u.scheme(), "http" | "https")).unwrap_or(false),
            "number" => v.is_number(),
            "date" => v.as_str().map(|s| NaiveDate::parse_from_str(s, "%Y-%m-%d").is_ok()).unwrap_or(false),
            "select" => v.as_str().map(|s| f.options.iter().any(|o| o == s)).unwrap_or(false),
            "multiselect" => v.as_array().map(|a| a.iter().all(|x| x.as_str().map(|s| f.options.iter().any(|o| o == s)).unwrap_or(false))).unwrap_or(false),
            "user" => v.as_str().map(|s| s.parse::<Uuid>().is_ok()).unwrap_or(false),
            "checkbox" => v.is_boolean(),
            _ => false,
        };
        if !ok {
            return Err(bad(format!("{} has an invalid value for a {} field", f.name, f.kind)));
        }
    }
    if check_required {
        if let Some(f) = ti.fields.iter().find(|f| f.required && !is_filled(custom.get(&f.key))) {
            return Err(bad(format!("{} is required for {}", f.name, ti.name)));
        }
    }
    Ok(())
}

/// When the workspace enforces level order, a child's type must sit on a lower level than
/// its parent's. Types that were never given a level are not restricted.
pub async fn check_levels(st: &AppState, child_type: &str, parent: Uuid) -> AppResult<()> {
    if crate::settings::get(st, "hierarchy").await["enforce_item_levels"] != true {
        return Ok(());
    }
    let row = sqlx::query(
        "SELECT cl.position AS child_pos, cl.name AS child_level, pl.position AS parent_pos, pl.name AS parent_level
         FROM item_types ct LEFT JOIN hierarchy_levels cl ON cl.id = ct.level_id,
              items p JOIN item_types pt ON pt.key = p.type_key LEFT JOIN hierarchy_levels pl ON pl.id = pt.level_id
         WHERE ct.key = $1 AND p.id = $2",
    )
    .bind(child_type)
    .bind(parent)
    .fetch_optional(&st.db)
    .await?;
    if let Some(r) = row {
        if let (Some(c), Some(p)) = (r.get::<Option<i32>, _>("child_pos"), r.get::<Option<i32>, _>("parent_pos")) {
            if c <= p {
                return Err(bad(format!("a {} cannot sit under a {}: the hierarchy puts {} at or above {}", r.get::<String, _>("child_level"), r.get::<String, _>("parent_level"), r.get::<String, _>("child_level"), r.get::<String, _>("parent_level"))));
            }
        }
    }
    Ok(())
}

pub fn clean_labels(labels: &[String]) -> AppResult<Vec<String>> {
    let mut out: Vec<String> = labels.iter().map(|l| l.trim().to_lowercase()).filter(|l| !l.is_empty()).collect();
    out.sort();
    out.dedup();
    if out.len() > 30 || out.iter().any(|l| l.chars().count() > 50) {
        return Err(bad("at most 30 labels of up to 50 characters each"));
    }
    Ok(out)
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewItem {
    #[serde(rename = "type")]
    pub type_key: Option<String>,
    pub title: String,
    pub description: Option<String>,
    pub priority: Option<i16>,
    pub assignee_id: Option<Uuid>,
    /// Key of the parent item, e.g. `OPS-3`.
    pub parent: Option<String>,
    pub labels: Option<Vec<String>>,
    pub start_date: Option<NaiveDate>,
    pub due_date: Option<NaiveDate>,
    pub estimate: Option<f64>,
    pub cycle_id: Option<Uuid>,
    pub custom: Option<Map<String, Value>>,
    #[serde(skip)]
    pub parent_id: Option<Uuid>,
    #[serde(skip)]
    pub channel: Option<String>,
    #[serde(skip)]
    pub request_type_id: Option<Uuid>,
    /// Imports and templates: start in this status and skip required-field checks.
    #[serde(skip)]
    pub status: Option<String>,
    #[serde(skip)]
    pub lenient: bool,
}

/// Insert a work item. No permission check: callers decide who may create.
pub async fn create_item(st: &AppState, reporter: Uuid, project: &Project, n: NewItem) -> AppResult<(Uuid, String)> {
    let type_key = n.type_key.clone().unwrap_or_else(|| project.default_type.clone());
    let enabled: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM project_types WHERE project_id = $1 AND type_key = $2)").bind(project.id).bind(&type_key).fetch_one(&st.db).await?;
    if !enabled {
        return Err(bad(format!("work item type {type_key:?} is not used in project {}", project.key)));
    }
    let ti = load_type_in(st, Some(project.id), &type_key).await?;
    // A title is one line. Imported work is tidied to fit; anything else is told.
    let tidied: String;
    let title = if n.lenient {
        tidied = n.title.chars().map(|c| if crate::accounts::deceptive(c) { ' ' } else { c }).collect::<String>().trim().to_string();
        tidied.as_str()
    } else {
        n.title.trim()
    };
    if title.is_empty() || title.chars().count() > 500 || title.chars().any(crate::accounts::deceptive) {
        return Err(bad("title must be 1 to 500 characters, on one line"));
    }
    let priority = n.priority.unwrap_or(2);
    if !(0..=4).contains(&priority) {
        return Err(bad("priority must be between 0 (none) and 4 (critical)"));
    }
    // Imported work keeps the dates it came with, whatever their order.
    if !n.lenient && matches!((n.start_date, n.due_date), (Some(s), Some(d)) if d < s) {
        return Err(bad("the due date cannot be before the start date"));
    }
    let custom = n.custom.unwrap_or_default();
    validate_custom(&ti, &custom, !n.lenient)?;
    let labels = clean_labels(&n.labels.unwrap_or_default())?;
    let status = n.status.as_deref().filter(|s| ti.workflow.status(s).is_some()).unwrap_or(&ti.workflow.initial).to_string();
    let category = ti.workflow.status(&status).map(|s| s.category.clone()).unwrap_or_else(|| "todo".into());
    let parent_id = match (&n.parent, n.parent_id) {
        (_, Some(id)) => Some(id),
        (Some(k), None) => Some(find_item(st, k).await?.filter(|p| p.project_id == project.id).ok_or_else(|| bad("parent must be an item in the same project"))?.id),
        (None, None) => None,
    };
    if let Some(pid) = parent_id {
        check_levels(st, &type_key, pid).await?;
    }
    if let Some(c) = n.cycle_id {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cycles WHERE id = $1 AND project_id = $2)").bind(c).bind(project.id).fetch_one(&st.db).await?;
        if !ok {
            return Err(bad("cycle does not belong to this project"));
        }
    }
    let (resp_due, res_due) = desk::sla_targets(st, project, priority, Utc::now()).await?;

    let mut tx = st.db.begin().await?;
    let number: i32 = sqlx::query_scalar("UPDATE projects SET next_number = next_number + 1 WHERE id = $1 RETURNING next_number - 1").bind(project.id).fetch_one(&mut *tx).await?;
    let id: Uuid = sqlx::query_scalar(
        "INSERT INTO items (project_id, number, type_key, title, description, status, status_category, priority, assignee_id, reporter_id, parent_id, labels,
                            start_date, due_date, estimate, cycle_id, rank, custom, channel, request_type_id, sla_response_due, sla_resolution_due, resolved_at)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19,$20,$21,$22, CASE WHEN $7 = 'done' THEN now() END) RETURNING id",
    )
    .bind(project.id)
    .bind(number)
    .bind(&type_key)
    .bind(title)
    .bind(n.description.unwrap_or_default())
    .bind(&status)
    .bind(&category)
    .bind(priority)
    .bind(n.assignee_id)
    .bind(reporter)
    .bind(parent_id)
    .bind(&labels)
    .bind(n.start_date)
    .bind(n.due_date)
    .bind(n.estimate)
    .bind(n.cycle_id)
    .bind(number as f64)
    .bind(Value::Object(custom))
    .bind(n.channel.unwrap_or_else(|| "web".into()))
    .bind(n.request_type_id)
    .bind(resp_due)
    .bind(res_due)
    .fetch_one(&mut *tx)
    .await?;
    sqlx::query("INSERT INTO item_history (item_id, actor_id, field, new) VALUES ($1,$2,'created',$3)").bind(id).bind(reporter).bind(json!(status)).execute(&mut *tx).await?;
    // The people involved from the start follow the item.
    sqlx::query("INSERT INTO item_watchers (item_id, user_id) SELECT $1, u FROM unnest($2::uuid[]) AS u ON CONFLICT DO NOTHING").bind(id).bind([Some(reporter), n.assignee_id].into_iter().flatten().collect::<Vec<_>>()).execute(&mut *tx).await?;
    tx.commit().await?;
    index_item(st, id).await;
    Ok((id, format!("{}-{number}", project.key)))
}

/// Refresh the search document (and queue an embedding refresh) for one item.
pub async fn index_item(st: &AppState, id: Uuid) {
    let r: Result<(), anyhow::Error> = async {
        let row = sqlx::query(
            "SELECT i.project_id, p.key || '-' || i.number AS key, i.title, i.description, i.custom, i.archived,
                    coalesce((SELECT string_agg(c.body, E'\n\n' ORDER BY c.created_at) FROM comments c WHERE c.item_id = i.id AND NOT c.internal), '') AS comments
             FROM items i JOIN projects p ON p.id = i.project_id WHERE i.id = $1",
        )
        .bind(id)
        .fetch_one(&st.db)
        .await?;
        if row.get::<bool, _>("archived") {
            st.stores.search.remove("item", id).await?;
            st.stores.vector.remove("item", id).await?;
            return Ok(());
        }
        let key: String = row.get("key");
        let custom_text: String = row.get::<Value, _>("custom").as_object().map(|o| o.values().filter_map(|v| v.as_str()).collect::<Vec<_>>().join("\n")).unwrap_or_default();
        st.stores
            .search
            .index(&SearchDoc {
                kind: "item".into(),
                ref_id: id,
                project_id: Some(row.get("project_id")),
                space_id: None,
                url: format!("/i/{key}"),
                label: key,
                title: row.get("title"),
                body: format!("{}\n\n{}\n\n{}", row.get::<String, _>("description"), custom_text, row.get::<String, _>("comments")),
                customer_visible: false,
            })
            .await?;
        if st.stores.embedder.is_some() {
            jobs::enqueue(st, "embed", json!({"kind": "item", "id": id})).await?;
        }
        Ok(())
    }
    .await;
    if let Err(e) = r {
        tracing::error!(error = %e, item = %id, "search indexing failed");
    }
}

/// Record one change in the item's history: which field, from what, to what, by whom.
pub async fn history(st: &AppState, item: Uuid, actor: Option<Uuid>, field: &str, old: Value, new: Value) -> AppResult<()> {
    sqlx::query("INSERT INTO item_history (item_id, actor_id, field, old, new) VALUES ($1,$2,$3,$4,$5)").bind(item).bind(actor).bind(field).bind(old).bind(new).execute(&st.db).await?;
    Ok(())
}

/// A person as recorded in history: the id, and the name they had at the time.
async fn person_ref(st: &AppState, id: Option<Uuid>) -> AppResult<Value> {
    Ok(match id {
        Some(u) => sqlx::query_scalar("SELECT jsonb_build_object('id', id, 'name', name, 'kind', kind) FROM users WHERE id = $1").bind(u).fetch_optional(&st.db).await?.unwrap_or(json!({"id": u})),
        None => Value::Null,
    })
}

async fn cycle_ref(st: &AppState, id: Option<Uuid>) -> AppResult<Value> {
    Ok(match id {
        Some(c) => sqlx::query_scalar("SELECT jsonb_build_object('id', id, 'name', name) FROM cycles WHERE id = $1").bind(c).fetch_optional(&st.db).await?.unwrap_or(json!({"id": c})),
        None => Value::Null,
    })
}

async fn item_ref(st: &AppState, id: Option<Uuid>) -> AppResult<Value> {
    Ok(match id {
        Some(i) => sqlx::query_scalar("SELECT jsonb_build_object('id', i.id, 'key', p.key || '-' || i.number) FROM items i JOIN projects p ON p.id = i.project_id WHERE i.id = $1").bind(i).fetch_optional(&st.db).await?.unwrap_or(json!({"id": i})),
        None => Value::Null,
    })
}

/// Long text is kept in history up to this size so a change can be shown as a difference.
fn history_text(s: &str) -> Value {
    const MAX: usize = 60_000;
    if s.len() <= MAX {
        json!(s)
    } else {
        let cut = (0..=MAX).rev().find(|i| s.is_char_boundary(*i)).unwrap_or(0);
        json!({"truncated": true, "text": &s[..cut]})
    }
}

#[allow(clippy::too_many_arguments)]
fn item_event<'a>(kind: &'a str, ctx: &'a Ctx, project: &Project, item: Uuid, key: &str, title: &str, summary: String, data: Value, notify: Vec<Uuid>) -> Event<'a> {
    Event { kind, project_id: Some(project.id), space_id: None, actor_id: Some(ctx.user_id), actor_name: &ctx.name, subject: format!("{key} {title}"), url: format!("/i/{key}"), summary, data, notify, item: Some((item, key.to_string())), page: None, excerpt: String::new(), skip: vec![] }
}

/// Tell the people newly mentioned in a piece of text, if they may see the item.
async fn notify_mentions(st: &St, ctx: &Ctx, project: &Project, item: Uuid, key: &str, title: &str, before: &Value, after: &Value, text: &str, internal: bool, what: &str) -> Vec<Uuid> {
    let old = crate::refs::mentioned(before);
    let new: Vec<Uuid> = crate::refs::mentioned(after).into_iter().filter(|u| !old.contains(u) && *u != ctx.user_id).collect();
    if !new.is_empty() {
        let _ = sqlx::query("INSERT INTO item_watchers (item_id, user_id) SELECT $1, u FROM unnest($2::uuid[]) AS u ON CONFLICT DO NOTHING").bind(item).bind(&new).execute(&st.db).await;
        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM users WHERE id = ANY($1) ORDER BY name").bind(&new).fetch_all(&st.db).await.unwrap_or_default();
        let whom = if names.len() == 1 { names[0].clone() } else { format!("{} people", names.len()) };
        let mut e = item_event("mention.created", ctx, project, item, key, title, format!("{} mentioned {whom} in {what} {key}", ctx.name), json!({"key": key, "internal": internal}), new.clone());
        e.excerpt = text.to_string();
        events::emit(st, e).await;
    }
    new
}

/// Work can only be given to an active account that is able to open it.
async fn check_assignee(st: &AppState, project: &Project, user: Uuid) -> AppResult<()> {
    if authz::project_audience(st, project.id, "project.view", &[user]).await?.is_empty() {
        return Err(bad(format!("the assignee must be an active account with access to {}; add them to the project first", project.key)));
    }
    Ok(())
}

pub async fn create_item_checked(st: &St, ctx: &Ctx, project_key: &str, mut n: NewItem, channel: &str) -> AppResult<Value> {
    let (project, perms) = projects::load_project(st, ctx, project_key).await?;
    authz::require(st, ctx, &perms, "item.create", &format!("project:{}", project.key))?;
    if let Some(a) = n.assignee_id {
        authz::require(st, ctx, &perms, "item.assign", &format!("project:{}", project.key))?;
        check_assignee(st, &project, a).await?;
    }
    n.channel = Some(channel.to_string());
    let title = n.title.trim().to_string();
    let assignee = n.assignee_id;
    let description = n.description.clone().unwrap_or_default();
    let type_key = n.type_key.clone().unwrap_or_else(|| project.default_type.clone());
    let refs = crate::refs::check(st, ctx, &description, None).await?;
    let (id, key) = create_item(st, ctx.user_id, &project, n).await?;
    sqlx::query("UPDATE items SET refs = $2 WHERE id = $1").bind(id).bind(&refs).execute(&st.db).await?;
    audit::log(st, ctx, "item.create", "item", &key, json!({"project": project.key, "type": type_key, "title": title, "channel": channel})).await;
    let mentioned = notify_mentions(st, ctx, &project, id, &key, &title, &json!({}), &refs, &description, false, "the description of").await;
    let mut e = item_event("item.created", ctx, &project, id, &key, &title, format!("{} created {key}: {title}", ctx.name), json!({"key": key, "type": type_key}), assignee.into_iter().collect());
    e.skip = mentioned;
    events::emit(st, e).await;
    detail_json(st, ctx, id, &project, &perms).await
}

/// Set a new status directly. Used by transitions, imports and integrations after their own checks.
pub async fn apply_status(st: &AppState, actor: Option<Uuid>, item: &Item, wf: &WorkflowDef, to: &str) -> AppResult<()> {
    let category = wf.status(to).map(|s| s.category.clone()).ok_or_else(|| bad(format!("unknown status {to:?}")))?;
    sqlx::query(
        "UPDATE items SET status = $2, status_category = $3, updated_at = now(),
                resolved_at = CASE WHEN $3 = 'done' THEN coalesce(resolved_at, now()) ELSE NULL END,
                sla_resolution_breached = CASE WHEN $3 = 'done' THEN sla_resolution_breached OR coalesce(sla_resolution_due < now(), false) ELSE sla_resolution_breached END
         WHERE id = $1",
    )
    .bind(item.id)
    .bind(to)
    .bind(&category)
    .execute(&st.db)
    .await?;
    let name = |key: &str| json!({"key": key, "name": wf.status(key).map(|s| s.name.as_str()).unwrap_or(key)});
    history(st, item.id, actor, "status", name(&item.status), name(to)).await
}

pub async fn transition_item(st: &St, ctx: &Ctx, key: &str, to: &str) -> AppResult<Value> {
    let (item, project, perms) = load_item(st, ctx, key).await?;
    authz::require(st, ctx, &perms, "item.transition", &format!("item:{key}"))?;
    let ti = load_type_in(st, Some(project.id), &item.type_key).await?;
    let t = ti.workflow.transition(&item.status, to).ok_or_else(|| {
        let allowed: Vec<&str> = ti.workflow.available(&item.status).iter().map(|t| t.to.as_str()).collect();
        bad(format!("the {} workflow has no transition from {:?} to {to:?}; allowed next statuses: {}", ti.workflow.name, item.status, if allowed.is_empty() { "none".into() } else { allowed.join(", ") }))
    })?;
    if let Some(missing) = t.requires.iter().find(|r| !is_filled(item.custom.get(r.as_str()))) {
        let name = ti.fields.iter().find(|f| &f.key == missing).map(|f| f.name.clone()).unwrap_or_else(|| missing.clone());
        return Err(bad(format!("{name} must be filled in before \"{}\"", t.name)));
    }
    apply_status(st, Some(ctx.user_id), &item, &ti.workflow, to).await?;
    let ukey = key.to_uppercase();
    audit::log(st, ctx, "item.transition", "item", &ukey, json!({"from": item.status, "to": to, "transition": t.name})).await;
    let to_name = ti.workflow.status(to).map(|s| s.name.clone()).unwrap_or_default();
    let notify = [item.assignee_id, item.reporter_id].into_iter().flatten().collect();
    events::emit(st, item_event("item.transitioned", ctx, &project, item.id, &ukey, &item.title, format!("{} moved {ukey} to {to_name}", ctx.name), json!({"key": ukey, "from": item.status, "to": to}), notify)).await;
    detail_json(st, ctx, item.id, &project, &perms).await
}

pub async fn update_item(st: &St, ctx: &Ctx, key: &str, changes: Map<String, Value>) -> AppResult<Value> {
    let (item, project, perms) = load_item(st, ctx, key).await?;
    let target = format!("item:{key}");
    authz::require(st, ctx, &perms, "item.edit", &target)?;
    let ti = load_type_in(st, Some(project.id), &item.type_key).await?;
    let ukey = key.to_uppercase();
    let me = Some(ctx.user_id);
    let mut changed: Vec<&str> = vec![];
    let mut assigned_to: Option<Uuid> = None;
    let mut mentioned: Vec<Uuid> = vec![];
    let opt_date = |v: &Value, name: &str| -> AppResult<Option<NaiveDate>> {
        match v {
            Value::Null => Ok(None),
            Value::String(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map(Some).map_err(|_| bad(format!("{name} must be a date like 2026-03-31"))),
            _ => Err(bad(format!("{name} must be a date like 2026-03-31"))),
        }
    };
    let opt_uuid = |v: &Value, name: &str| -> AppResult<Option<Uuid>> {
        match v {
            Value::Null => Ok(None),
            Value::String(s) => s.parse().map(Some).map_err(|_| bad(format!("{name} must be an id"))),
            _ => Err(bad(format!("{name} must be an id"))),
        }
    };
    // Dates are checked together before anything is saved, so dragging a bar on a chart
    // cannot leave work that ends before it starts.
    if changes.contains_key("start_date") || changes.contains_key("due_date") {
        let start = changes.get("start_date").map(|v| opt_date(v, "start_date")).transpose()?.unwrap_or(item.start_date);
        let due = changes.get("due_date").map(|v| opt_date(v, "due_date")).transpose()?.unwrap_or(item.due_date);
        if matches!((start, due), (Some(s), Some(d)) if d < s) {
            return Err(bad("the due date cannot be before the start date"));
        }
    }
    for (k, v) in &changes {
        match k.as_str() {
            "title" => {
                let t = v.as_str().map(str::trim).filter(|t| !t.is_empty() && t.chars().count() <= 500 && !t.chars().any(crate::accounts::deceptive)).ok_or_else(|| bad("title must be 1 to 500 characters, on one line"))?;
                if t == item.title {
                    continue;
                }
                sqlx::query("UPDATE items SET title = $2 WHERE id = $1").bind(item.id).bind(t).execute(&st.db).await?;
                history(st, item.id, me, "title", json!(item.title), json!(t)).await?;
            }
            "description" => {
                let d = v.as_str().filter(|d| d.len() <= 1_000_000).ok_or_else(|| bad("description must be text"))?;
                if d == item.description {
                    continue;
                }
                let refs = crate::refs::check(st, ctx, d, None).await?;
                sqlx::query("UPDATE items SET description = $2, refs = $3 WHERE id = $1").bind(item.id).bind(d).bind(&refs).execute(&st.db).await?;
                history(st, item.id, me, "description", history_text(&item.description), history_text(d)).await?;
                // A description saved through the API replaces whatever people were co-editing.
                if changes.get("collab") != Some(&json!(true)) {
                    crate::realtime::reset_doc(st, &format!("item:{ukey}"), "description").await;
                }
                mentioned = notify_mentions(st, ctx, &project, item.id, &ukey, &item.title, &item.refs, &refs, d, false, "the description of").await;
            }
            // Sent with a description that came out of a shared editing session.
            "collab" => continue,
            "priority" => {
                let p = v.as_i64().filter(|p| (0..=4).contains(p)).ok_or_else(|| bad("priority must be between 0 and 4"))? as i16;
                if p == item.priority {
                    continue;
                }
                let (resp_due, res_due) = desk::sla_targets(st, &project, p, item.created_at).await?;
                sqlx::query("UPDATE items SET priority = $2, sla_response_due = $3, sla_resolution_due = $4 WHERE id = $1").bind(item.id).bind(p).bind(resp_due).bind(res_due).execute(&st.db).await?;
                history(st, item.id, me, "priority", json!(item.priority), json!(p)).await?;
            }
            "assignee_id" => {
                authz::require(st, ctx, &perms, "item.assign", &target)?;
                let a = opt_uuid(v, "assignee_id")?;
                if a == item.assignee_id {
                    continue;
                }
                if let Some(u) = a {
                    check_assignee(st, &project, u).await?;
                    sqlx::query("INSERT INTO item_watchers (item_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(item.id).bind(u).execute(&st.db).await?;
                }
                sqlx::query("UPDATE items SET assignee_id = $2 WHERE id = $1").bind(item.id).bind(a).execute(&st.db).await?;
                history(st, item.id, me, "assignee", person_ref(st, item.assignee_id).await?, person_ref(st, a).await?).await?;
                assigned_to = a;
            }
            "labels" => {
                let l: Vec<String> = serde_json::from_value(v.clone()).map_err(|_| bad("labels must be a list of text"))?;
                let l = clean_labels(&l)?;
                if l == item.labels {
                    continue;
                }
                sqlx::query("UPDATE items SET labels = $2 WHERE id = $1").bind(item.id).bind(&l).execute(&st.db).await?;
                history(st, item.id, me, "labels", json!(item.labels), json!(l)).await?;
            }
            "start_date" | "due_date" => {
                let d = opt_date(v, k)?;
                let old = if k == "due_date" { item.due_date } else { item.start_date };
                if d == old {
                    continue;
                }
                sqlx::query(if k == "due_date" { "UPDATE items SET due_date = $2 WHERE id = $1" } else { "UPDATE items SET start_date = $2 WHERE id = $1" }).bind(item.id).bind(d).execute(&st.db).await?;
                history(st, item.id, me, k, json!(old), json!(d)).await?;
            }
            "estimate" => {
                let e = if v.is_null() { None } else { Some(v.as_f64().filter(|e| *e >= 0.0 && e.is_finite()).ok_or_else(|| bad("estimate must be a non-negative number"))?) };
                if e == item.estimate {
                    continue;
                }
                sqlx::query("UPDATE items SET estimate = $2 WHERE id = $1").bind(item.id).bind(e).execute(&st.db).await?;
                history(st, item.id, me, "estimate", json!(item.estimate), json!(e)).await?;
            }
            "rank" => {
                // Position on a board. Not part of the history: it changes constantly and says little.
                let r = v.as_f64().filter(|r| r.is_finite()).ok_or_else(|| bad("rank must be a number"))?;
                sqlx::query("UPDATE items SET rank = $2 WHERE id = $1").bind(item.id).bind(r).execute(&st.db).await?;
            }
            "cycle_id" => {
                let c = opt_uuid(v, "cycle_id")?;
                if c == item.cycle_id {
                    continue;
                }
                if let Some(cid) = c {
                    let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM cycles WHERE id = $1 AND project_id = $2)").bind(cid).bind(project.id).fetch_one(&st.db).await?;
                    if !ok {
                        return Err(bad("cycle does not belong to this project"));
                    }
                }
                sqlx::query("UPDATE items SET cycle_id = $2 WHERE id = $1").bind(item.id).bind(c).execute(&st.db).await?;
                history(st, item.id, me, "cycle", cycle_ref(st, item.cycle_id).await?, cycle_ref(st, c).await?).await?;
            }
            "parent" => {
                let pid = match v {
                    Value::Null => None,
                    Value::String(pk) => {
                        let parent = find_item(st, pk).await?.filter(|p| p.project_id == project.id && p.id != item.id).ok_or_else(|| bad("parent must be another item in the same project"))?;
                        // Refuse cycles: the new parent must not be a descendant of this item.
                        let cyclic: bool = sqlx::query_scalar(
                            "WITH RECURSIVE up(id, parent_id, depth) AS (SELECT id, parent_id, 0 FROM items WHERE id = $1
                               UNION ALL SELECT i.id, i.parent_id, up.depth + 1 FROM items i JOIN up ON i.id = up.parent_id WHERE up.depth < 50)
                             SELECT EXISTS (SELECT 1 FROM up WHERE id = $2)",
                        )
                        .bind(parent.id)
                        .bind(item.id)
                        .fetch_one(&st.db)
                        .await?;
                        if cyclic {
                            return Err(bad("that would make an item its own ancestor"));
                        }
                        check_levels(st, &item.type_key, parent.id).await?;
                        Some(parent.id)
                    }
                    _ => return Err(bad("parent must be an item key or null")),
                };
                if pid == item.parent_id {
                    continue;
                }
                sqlx::query("UPDATE items SET parent_id = $2 WHERE id = $1").bind(item.id).bind(pid).execute(&st.db).await?;
                history(st, item.id, me, "parent", item_ref(st, item.parent_id).await?, item_ref(st, pid).await?).await?;
            }
            "custom" => {
                let c = v.as_object().ok_or_else(|| bad("custom must be an object"))?;
                validate_custom(&ti, c, false)?;
                let mut merged = item.custom.as_object().cloned().unwrap_or_default();
                let mut any = false;
                for (ck, cv) in c {
                    let old = merged.get(ck).cloned().unwrap_or(Value::Null);
                    if &old == cv {
                        continue;
                    }
                    any = true;
                    if cv.is_null() {
                        merged.remove(ck);
                    } else {
                        merged.insert(ck.clone(), cv.clone());
                    }
                    // The field's own key, so the history names it the way the form does.
                    history(st, item.id, me, &format!("custom.{ck}"), old, cv.clone()).await?;
                }
                if !any {
                    continue;
                }
                sqlx::query("UPDATE items SET custom = $2 WHERE id = $1").bind(item.id).bind(Value::Object(merged)).execute(&st.db).await?;
            }
            other => return Err(bad(format!("unknown field {other:?}; to change status use the transition endpoint"))),
        }
        changed.push(k.as_str());
    }
    if changed.is_empty() {
        return detail_json(st, ctx, item.id, &project, &perms).await;
    }
    sqlx::query("UPDATE items SET updated_at = now() WHERE id = $1").bind(item.id).execute(&st.db).await?;
    index_item(st, item.id).await;
    audit::log(st, ctx, "item.update", "item", &ukey, json!({"changed": changed})).await;
    if changed.iter().any(|c| *c != "rank") {
        let (kind, summary, notify) = match assigned_to {
            Some(u) => ("item.assigned", format!("{} assigned {ukey}", ctx.name), vec![u]),
            None => ("item.updated", format!("{} updated {ukey} ({})", ctx.name, changed.join(", ")), item.assignee_id.into_iter().collect()),
        };
        let mut e = item_event(kind, ctx, &project, item.id, &ukey, &item.title, summary, json!({"key": ukey, "changed": changed}), notify);
        e.skip = mentioned;
        events::emit(st, e).await;
    } else {
        // A reordering still moves cards on other people's boards.
        crate::realtime::publish_work(st, project.id, &project.key, Some(&ukey), "item.ranked", json!({"key": ukey, "actor_id": ctx.user_id})).await;
    }
    detail_json(st, ctx, item.id, &project, &perms).await
}

// ───────────────────────── discussion ─────────────────────────

/// Insert a message and apply service desk bookkeeping. No permission check.
pub async fn insert_comment(st: &AppState, item: &Item, project: &Project, author: Uuid, body: &str, internal: bool, by_agent: bool, refs: &Value) -> AppResult<Uuid> {
    let body = body.trim();
    if body.is_empty() || body.len() > 100_000 {
        return Err(bad("a message must be 1 to 100000 characters"));
    }
    let id: Uuid = sqlx::query_scalar("INSERT INTO comments (item_id, author_id, body, internal, refs) VALUES ($1,$2,$3,$4,$5) RETURNING id").bind(item.id).bind(author).bind(body).bind(internal).bind(refs).fetch_one(&st.db).await?;
    let first_response = project.kind == "service_desk" && by_agent && !internal && item.first_response_at.is_none() && item.reporter_id != Some(author);
    sqlx::query(
        "UPDATE items SET updated_at = now(),
                first_response_at = CASE WHEN $2 THEN now() ELSE first_response_at END,
                sla_response_breached = CASE WHEN $2 THEN coalesce(sla_response_due < now(), false) ELSE sla_response_breached END
         WHERE id = $1",
    )
    .bind(item.id)
    .bind(first_response)
    .execute(&st.db)
    .await?;
    // Taking part in a discussion means following it.
    sqlx::query("INSERT INTO item_watchers (item_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(item.id).bind(author).execute(&st.db).await?;
    history(st, item.id, Some(author), if internal { "note" } else { "comment" }, Value::Null, json!({"id": id, "action": "added"})).await?;
    if !internal {
        index_item(st, item.id).await;
    }
    Ok(id)
}

const COMMENT_SQL: &str = "SELECT c.id, c.body, c.internal, c.created_at, c.edited_at, c.deleted_at IS NOT NULL AS deleted, c.refs, c.author_id, person_json(u) AS author
    FROM comments c LEFT JOIN users u ON u.id = c.author_id";

/// Messages as the viewer may see them: internal notes only for agents, removed messages as
/// a marker without their text, and references resolved under the viewer's own access.
pub async fn comments_json(st: &AppState, ctx: &Ctx, item: Uuid, show_internal: bool, people: crate::refs::People<'_>, only: Option<Uuid>) -> AppResult<Vec<Value>> {
    let rows = sqlx::query(&format!("{COMMENT_SQL} WHERE c.item_id = $1 AND ($2 OR NOT c.internal) AND ($3::uuid IS NULL OR c.id = $3) ORDER BY c.created_at, c.id")).bind(item).bind(show_internal).bind(only).fetch_all(&st.db).await?;
    let stored: Vec<Value> = rows.iter().map(|r| if r.get::<bool, _>("deleted") { json!({}) } else { r.get("refs") }).collect();
    let resolved = crate::refs::resolve(st, ctx, &stored.iter().collect::<Vec<_>>(), people).await?;
    Ok(rows
        .iter()
        .zip(resolved)
        .map(|(r, refs)| {
            let deleted: bool = r.get("deleted");
            json!({
                "id": r.get::<Uuid, _>("id"), "internal": r.get::<bool, _>("internal"), "created_at": r.get::<DateTime<Utc>, _>("created_at"),
                "edited_at": r.get::<Option<DateTime<Utc>>, _>("edited_at"), "deleted": deleted, "author": r.get::<Option<Value>, _>("author"),
                "body": if deleted { String::new() } else { r.get::<String, _>("body") }, "refs": refs,
                "mine": r.get::<Option<Uuid>, _>("author_id") == Some(ctx.user_id),
            })
        })
        .collect())
}

pub async fn add_comment(st: &St, ctx: &Ctx, key: &str, body: &str, internal: bool) -> AppResult<Value> {
    let (item, project, perms) = load_item(st, ctx, key).await?;
    authz::require(st, ctx, &perms, "item.comment", &format!("item:{key}"))?;
    if internal {
        authz::require(st, ctx, &perms, "desk.agent", &format!("item:{key}"))?;
    }
    let refs = crate::refs::check(st, ctx, body, None).await?;
    let id = insert_comment(st, &item, &project, ctx.user_id, body, internal, perms.has("desk.agent"), &refs).await?;
    let ukey = key.to_uppercase();
    audit::log(st, ctx, "comment.create", "item", &ukey, json!({"comment_id": id, "internal": internal})).await;
    let mentioned = notify_mentions(st, ctx, &project, item.id, &ukey, &item.title, &json!({}), &refs, body, internal, "a message on").await;
    let notify: Vec<Uuid> = if internal { item.assignee_id.into_iter().collect() } else { [item.assignee_id, item.reporter_id].into_iter().flatten().collect() };
    let mut e = item_event("comment.created", ctx, &project, item.id, &ukey, &item.title, format!("{} commented on {ukey}", ctx.name), json!({"key": ukey, "comment_id": id, "internal": internal}), notify);
    e.excerpt = body.to_string();
    e.skip = mentioned;
    events::emit(st, e).await;
    let mut out = comments_json(st, ctx, item.id, perms.has("desk.agent"), crate::refs::People::Directory, Some(id)).await?;
    Ok(out.pop().unwrap_or(json!({"id": id})))
}

/// The message, its item and project, and the caller's permissions there. 404 unless the
/// caller can see the item (and, for an internal note, is an agent).
async fn load_comment(st: &St, ctx: &Ctx, id: Uuid) -> AppResult<(sqlx::postgres::PgRow, Item, Project, Perms)> {
    let row = sqlx::query("SELECT id, item_id, author_id, body, internal, refs, deleted_at IS NOT NULL AS deleted FROM comments WHERE id = $1").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let item = item_by_id(st, row.get("item_id")).await?.filter(|i| !i.archived).ok_or(AppError::NotFound)?;
    let project = projects::project_by_id(st, item.project_id).await?;
    let perms = authz::project_perms(st, ctx, project.id, &project.access, project.portal_open()).await?;
    let visible = perms.has("project.view") || (perms.has("desk.request") && item.reporter_id == Some(ctx.user_id));
    if !visible || (row.get::<bool, _>("internal") && !perms.has("desk.agent")) {
        return Err(authz::hide(st, ctx, "project.view", &format!("comment:{id}")));
    }
    Ok((row, item, project, perms))
}

#[derive(Deserialize)]
struct CommentEdit {
    body: String,
}

/// Edit one's own message. The message is marked as edited.
async fn edit_comment(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<CommentEdit>) -> AppResult<Json<Value>> {
    let (row, item, project, perms) = load_comment(&st, &ctx, id).await?;
    if row.get::<Option<Uuid>, _>("author_id") != Some(ctx.user_id) || !ctx.can_write() {
        return Err(authz::deny(&st, &ctx, "comment.edit", &format!("comment:{id}")));
    }
    if row.get::<bool, _>("deleted") {
        return Err(bad("this message was removed"));
    }
    let body = b.body.trim();
    if body.is_empty() || body.len() > 100_000 {
        return Err(bad("a message must be 1 to 100000 characters"));
    }
    let internal: bool = row.get("internal");
    let refs = crate::refs::check(&st, &ctx, body, None).await?;
    sqlx::query("UPDATE comments SET body = $2, refs = $3, edited_at = now() WHERE id = $1").bind(id).bind(body).bind(&refs).execute(&st.db).await?;
    let key = format!("{}-{}", project.key, item.number);
    history(&st, item.id, Some(ctx.user_id), if internal { "note" } else { "comment" }, Value::Null, json!({"id": id, "action": "edited"})).await?;
    audit::log(&st, &ctx, "comment.edit", "item", &key, json!({"comment_id": id})).await;
    if !internal {
        index_item(&st, item.id).await;
    }
    notify_mentions(&st, &ctx, &project, item.id, &key, &item.title, &row.get::<Value, _>("refs"), &refs, body, internal, "a message on").await;
    crate::realtime::publish_work(&st, project.id, &project.key, Some(&key), "comment.updated", json!({"key": key, "comment_id": id, "actor_id": ctx.user_id})).await;
    let team = perms.has("project.view");
    let mut out = comments_json(&st, &ctx, item.id, perms.has("desk.agent"), if team { crate::refs::People::Directory } else { crate::refs::People::Only(&[]) }, Some(id)).await?;
    Ok(Json(out.pop().unwrap_or(json!({"id": id}))))
}

/// Remove a message: one's own, or anyone's with the moderation permission. The text is
/// erased; a marker stays so the discussion still reads in order.
async fn delete_comment(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let (row, item, project, perms) = load_comment(&st, &ctx, id).await?;
    let own = row.get::<Option<Uuid>, _>("author_id") == Some(ctx.user_id) && ctx.can_write();
    if !own {
        authz::require(&st, &ctx, &perms, "comment.moderate", &format!("comment:{id}"))?;
    }
    if row.get::<bool, _>("deleted") {
        return Ok(StatusCode::NO_CONTENT);
    }
    let internal: bool = row.get("internal");
    sqlx::query("UPDATE comments SET body = '(removed)', refs = '{}', deleted_at = now(), deleted_by = $2 WHERE id = $1").bind(id).bind(ctx.user_id).execute(&st.db).await?;
    let key = format!("{}-{}", project.key, item.number);
    history(&st, item.id, Some(ctx.user_id), if internal { "note" } else { "comment" }, Value::Null, json!({"id": id, "action": "removed", "own": own})).await?;
    audit::log(&st, &ctx, "comment.delete", "item", &key, json!({"comment_id": id, "author_id": row.get::<Option<Uuid>, _>("author_id"), "moderated": !own})).await;
    index_item(&st, item.id).await;
    crate::realtime::publish_work(&st, project.id, &project.key, Some(&key), "comment.updated", json!({"key": key, "comment_id": id, "actor_id": ctx.user_id})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn h_comments(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    let (item, _, perms) = load_item(&st, &ctx, &key).await?;
    Ok(Json(json!(comments_json(&st, &ctx, item.id, perms.has("desk.agent"), crate::refs::People::Directory, None).await?)))
}

async fn set_watch(st: &St, ctx: &Ctx, key: &str, on: bool) -> AppResult<Json<Value>> {
    let (item, _, _) = load_item(st, ctx, key).await?;
    if !ctx.can_write() {
        return Err(authz::deny(st, ctx, "item.watch", &format!("item:{key}")));
    }
    if on {
        sqlx::query("INSERT INTO item_watchers (item_id, user_id) VALUES ($1, $2) ON CONFLICT DO NOTHING").bind(item.id).bind(ctx.user_id).execute(&st.db).await?;
    } else {
        sqlx::query("DELETE FROM item_watchers WHERE item_id = $1 AND user_id = $2").bind(item.id).bind(ctx.user_id).execute(&st.db).await?;
    }
    let count: i64 = sqlx::query_scalar("SELECT count(*) FROM item_watchers WHERE item_id = $1").bind(item.id).fetch_one(&st.db).await?;
    Ok(Json(json!({"watching": on, "watchers": count})))
}

async fn h_watch(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    set_watch(&st, &ctx, &key, true).await
}

async fn h_unwatch(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    set_watch(&st, &ctx, &key, false).await
}

const HISTORY_SQL: &str = "SELECT coalesce(jsonb_agg(to_jsonb(h) ORDER BY h.at DESC, h.id DESC), '[]'::jsonb) FROM (
    SELECT h.id, h.field, h.old, h.new, h.at, person_json(u) AS actor,
           (SELECT f.name FROM fields f WHERE 'custom.' || f.key = h.field OR f.key = h.field) AS field_name
    FROM item_history h LEFT JOIN users u ON u.id = h.actor_id
    WHERE h.item_id = $1 AND ($2 OR h.field <> 'note') ORDER BY h.at DESC, h.id DESC LIMIT $3 OFFSET $4) h";

/// Everything that happened to the item, newest first: who changed which field from what to
/// what, and when; messages, files, links and status moves.
async fn h_history(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Query(p): Query<crate::http::Paging>) -> AppResult<Json<Value>> {
    let (item, _, perms) = load_item(&st, &ctx, &key).await?;
    let (limit, offset) = p.get();
    let entries: Value = sqlx::query_scalar(HISTORY_SQL).bind(item.id).bind(perms.has("desk.agent")).bind(limit).bind(offset).fetch_one(&st.db).await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM item_history WHERE item_id = $1 AND ($2 OR field <> 'note')").bind(item.id).bind(perms.has("desk.agent")).fetch_one(&st.db).await?;
    Ok(Json(json!({"entries": entries, "total": total, "limit": limit, "offset": offset})))
}

/// Resolve the far ends of graph edges into display data, hiding anything the caller cannot see.
async fn resolve_nodes(st: &St, ctx: &Ctx, nodes: &[(String, String)]) -> AppResult<HashMap<(String, String), Value>> {
    let ids = |t: &str| nodes.iter().filter(|n| n.0 == t).filter_map(|n| n.1.parse::<Uuid>().ok()).collect::<Vec<_>>();
    let mut out = HashMap::new();
    let item_ids = ids("item");
    if !item_ids.is_empty() {
        let visible = authz::visible_project_ids(st, ctx).await?;
        let rows = sqlx::query("SELECT i.id, p.key || '-' || i.number AS key, i.title, i.status, i.status_category, i.type_key FROM items i JOIN projects p ON p.id = i.project_id WHERE i.id = ANY($1) AND i.project_id = ANY($2) AND NOT i.archived").bind(&item_ids).bind(&visible).fetch_all(&st.db).await?;
        for r in rows {
            let id: Uuid = r.get("id");
            let key: String = r.get("key");
            out.insert(("item".to_string(), id.to_string()), json!({"type": "item", "id": id, "key": key, "title": r.get::<String, _>("title"), "status": r.get::<String, _>("status"), "status_category": r.get::<String, _>("status_category"), "item_type": r.get::<String, _>("type_key"), "url": format!("/i/{key}")}));
        }
    }
    let page_ids = ids("page");
    if !page_ids.is_empty() {
        let visible = authz::visible_space_ids(st, ctx).await?;
        let rows = sqlx::query("SELECT pg.id, pg.title, s.key FROM pages pg JOIN spaces s ON s.id = pg.space_id WHERE pg.id = ANY($1) AND pg.space_id = ANY($2) AND NOT pg.archived").bind(&page_ids).bind(&visible).fetch_all(&st.db).await?;
        for r in rows {
            let id: Uuid = r.get("id");
            out.insert(("page".to_string(), id.to_string()), json!({"type": "page", "id": id, "title": r.get::<String, _>("title"), "url": format!("/docs/{}/{id}", r.get::<String, _>("key"))}));
        }
    }
    Ok(out)
}

fn external_node(t: &str, id: &str, meta: &Value) -> Value {
    let url = meta.get("url").and_then(Value::as_str).filter(|u| u.starts_with("https://") || u.starts_with("http://")).unwrap_or_default();
    json!({"type": t, "id": id, "title": meta.get("title").and_then(Value::as_str).unwrap_or(id), "url": url, "external": true})
}

pub async fn detail_json(st: &St, ctx: &Ctx, id: Uuid, project: &Project, perms: &Perms) -> AppResult<Value> {
    let show_internal = perms.has("desk.agent");
    let mut base: Value = sqlx::query_scalar(&format!(
        "SELECT {ITEM_JSON} || jsonb_build_object('description', i.description, 'custom', i.custom, 'request_type_id', i.request_type_id,
            'cycle', (SELECT jsonb_build_object('id', c.id, 'name', c.name) FROM cycles c WHERE c.id = i.cycle_id),
            'watchers', (SELECT count(*) FROM item_watchers w WHERE w.item_id = i.id),
            'watching', EXISTS (SELECT 1 FROM item_watchers w WHERE w.item_id = i.id AND w.user_id = $2)) {ITEM_FROM} WHERE i.id = $1"
    ))
    .bind(id)
    .bind(ctx.user_id)
    .fetch_one(&st.db)
    .await?;
    let comments = comments_json(st, ctx, id, show_internal, crate::refs::People::Directory, None).await?;
    let stored_refs: Value = sqlx::query_scalar("SELECT refs FROM items WHERE id = $1").bind(id).fetch_one(&st.db).await?;
    let description_refs = crate::refs::resolve(st, ctx, &[&stored_refs], crate::refs::People::Directory).await?.pop().unwrap_or(json!({}));
    let attachments: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(jsonb_build_object('id', a.id, 'filename', a.filename, 'content_type', a.content_type, 'size', a.size, 'created_at', a.created_at, 'uploaded_by', person_json(u)) ORDER BY a.created_at), '[]'::jsonb)
         FROM attachments a LEFT JOIN users u ON u.id = a.uploaded_by WHERE a.owner_type = 'item' AND a.owner_id = $1",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await?;
    let hist: Value = sqlx::query_scalar(HISTORY_SQL).bind(id).bind(show_internal).bind(200i64).bind(0i64).fetch_one(&st.db).await?;
    let hist_total: i64 = sqlx::query_scalar("SELECT count(*) FROM item_history WHERE item_id = $1 AND ($2 OR field <> 'note')").bind(id).bind(show_internal).fetch_one(&st.db).await?;
    let children: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(jsonb_build_object('key', p.key || '-' || c.number, 'title', c.title, 'status', c.status, 'status_category', c.status_category, 'type', c.type_key, 'assignee', person_json(a)) ORDER BY c.rank), '[]'::jsonb)
         FROM items c JOIN projects p ON p.id = c.project_id LEFT JOIN users a ON a.id = c.assignee_id WHERE c.parent_id = $1 AND NOT c.archived",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await?;
    let parent: Option<Value> = sqlx::query_scalar("SELECT jsonb_build_object('key', p.key || '-' || x.number, 'title', x.title) FROM items i JOIN items x ON x.id = i.parent_id JOIN projects p ON p.id = x.project_id WHERE i.id = $1").bind(id).fetch_optional(&st.db).await?;
    let refs: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('system', system, 'external_id', external_id, 'url', url)), '[]'::jsonb) FROM external_refs WHERE item_id = $1").bind(id).fetch_one(&st.db).await?;

    let me = ("item".to_string(), id.to_string());
    let edges = st.stores.graph.edges_of(("item", &me.1)).await.map_err(AppError::Other)?;
    let others: Vec<(String, String)> = edges.iter().map(|e| if (e.src_type.as_str(), e.src_id.as_str()) == (me.0.as_str(), me.1.as_str()) { (e.dst_type.clone(), e.dst_id.clone()) } else { (e.src_type.clone(), e.src_id.clone()) }).collect();
    let resolved = resolve_nodes(st, ctx, &others).await?;
    let links: Vec<Value> = edges
        .iter()
        .zip(&others)
        .filter_map(|(e, o)| {
            let other = match o.0.as_str() {
                "item" | "page" => resolved.get(o)?.clone(),
                t => external_node(t, &o.1, &e.meta),
            };
            Some(json!({"id": e.id, "rel": e.rel, "direction": if e.src_id == me.1 && e.src_type == "item" { "out" } else { "in" }, "other": other}))
        })
        .collect();

    let item_status = base["status"].as_str().unwrap_or_default().to_string();
    let ti = load_type_in(st, Some(project.id), base["type"].as_str().unwrap_or_default()).await?;
    let transitions: Vec<Value> = ti
        .workflow
        .available(&item_status)
        .iter()
        .map(|t| {
            let s = ti.workflow.status(&t.to);
            json!({"to": t.to, "name": t.name, "to_name": s.map(|s| s.name.as_str()), "category": s.map(|s| s.category.as_str()), "requires": t.requires})
        })
        .collect();
    let perm_list: Vec<String> = perms.list().into_iter().filter(|p| ctx.token_allows(p)).collect();
    let obj = base.as_object_mut().expect("item JSON is an object");
    obj.insert("comments".into(), json!(comments));
    obj.insert("description_refs".into(), description_refs);
    obj.insert("attachments".into(), attachments);
    obj.insert("history".into(), hist);
    obj.insert("history_total".into(), json!(hist_total));
    obj.insert("children".into(), children);
    obj.insert("parent".into(), parent.unwrap_or(Value::Null));
    obj.insert("external_refs".into(), refs);
    obj.insert("links".into(), json!(links));
    obj.insert("transitions".into(), json!(transitions));
    obj.insert("permissions".into(), json!(perm_list));
    obj.insert("project_kind".into(), json!(project.kind));
    obj.insert("project_name".into(), json!(project.name));
    Ok(base)
}

#[derive(Debug, Default, Deserialize)]
pub struct ItemFilter {
    pub project: Option<String>,
    pub status: Option<String>,
    pub category: Option<String>,
    #[serde(rename = "type")]
    pub type_key: Option<String>,
    pub assignee: Option<String>,
    pub reporter: Option<String>,
    pub label: Option<String>,
    pub cycle: Option<String>,
    pub parent: Option<String>,
    pub priority: Option<i16>,
    pub q: Option<String>,
    pub sla: Option<String>,
    pub overdue: Option<bool>,
    pub sort: Option<String>,
    pub limit: Option<i64>,
    pub offset: Option<i64>,
}

enum Who {
    User(Uuid),
    Nobody,
}

fn parse_who(v: &str, me: Uuid, name: &str) -> AppResult<Who> {
    match v {
        "me" => Ok(Who::User(me)),
        "none" => Ok(Who::Nobody),
        other => other.parse().map(Who::User).map_err(|_| bad(format!("{name} must be me, none or a user id"))),
    }
}

fn csv(v: &str) -> Vec<String> {
    v.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()
}

pub async fn list_items(st: &St, ctx: &Ctx, f: &ItemFilter) -> AppResult<Value> {
    let visible = authz::visible_project_ids(st, ctx).await?;
    let assignee = f.assignee.as_deref().map(|v| parse_who(v, ctx.user_id, "assignee")).transpose()?;
    let reporter = f.reporter.as_deref().map(|v| parse_who(v, ctx.user_id, "reporter")).transpose()?;
    let cycle = f.cycle.as_deref().map(|v| parse_who(v, Uuid::nil(), "cycle")).transpose()?;
    let parent = match &f.parent {
        Some(k) => Some(find_item(st, k).await?.ok_or_else(|| bad("unknown parent item"))?.id),
        None => None,
    };
    if let Some(s) = &f.sla {
        if !["breached", "at_risk", "tracked"].contains(&s.as_str()) {
            return Err(bad("sla must be breached, at_risk or tracked"));
        }
    }
    let filters = |qb: &mut QueryBuilder<'_, Postgres>| {
        qb.push(" WHERE NOT i.archived AND i.project_id = ANY(").push_bind(visible.clone()).push(")");
        if let Some(p) = &f.project {
            qb.push(" AND p.key = ").push_bind(p.to_uppercase());
        }
        if let Some(s) = &f.status {
            qb.push(" AND i.status = ANY(").push_bind(csv(s)).push(")");
        }
        if let Some(c) = &f.category {
            qb.push(" AND i.status_category = ANY(").push_bind(csv(c)).push(")");
        }
        if let Some(t) = &f.type_key {
            qb.push(" AND i.type_key = ANY(").push_bind(csv(t)).push(")");
        }
        match &assignee {
            Some(Who::User(u)) => {
                qb.push(" AND i.assignee_id = ").push_bind(*u);
            }
            Some(Who::Nobody) => {
                qb.push(" AND i.assignee_id IS NULL");
            }
            None => {}
        }
        match &reporter {
            Some(Who::User(u)) => {
                qb.push(" AND i.reporter_id = ").push_bind(*u);
            }
            Some(Who::Nobody) => {
                qb.push(" AND i.reporter_id IS NULL");
            }
            None => {}
        }
        match &cycle {
            Some(Who::User(c)) => {
                qb.push(" AND i.cycle_id = ").push_bind(*c);
            }
            Some(Who::Nobody) => {
                qb.push(" AND i.cycle_id IS NULL");
            }
            None => {}
        }
        if let Some(l) = &f.label {
            qb.push(" AND ").push_bind(l.trim().to_lowercase()).push(" = ANY(i.labels)");
        }
        if let Some(pid) = parent {
            qb.push(" AND i.parent_id = ").push_bind(pid);
        }
        if let Some(p) = f.priority {
            qb.push(" AND i.priority >= ").push_bind(p);
        }
        if let Some(q) = f.q.as_deref().map(str::trim).filter(|q| !q.is_empty()) {
            let esc = crate::storage::like_escape(q);
            qb.push(" AND (i.title ILIKE ").push_bind(format!("%{esc}%")).push(" OR (p.key || '-' || i.number) ILIKE ").push_bind(format!("{esc}%")).push(")");
        }
        match f.sla.as_deref() {
            Some("breached") => {
                qb.push(" AND i.status_category <> 'done' AND (i.sla_resolution_breached OR i.sla_resolution_due < now() OR (i.first_response_at IS NULL AND i.sla_response_due < now()))");
            }
            Some("at_risk") => {
                qb.push(" AND i.status_category <> 'done' AND i.sla_resolution_due BETWEEN now() AND now() + interval '2 hours'");
            }
            Some(_) => {
                qb.push(" AND i.sla_resolution_due IS NOT NULL");
            }
            None => {}
        }
        if f.overdue == Some(true) {
            qb.push(" AND i.status_category <> 'done' AND i.due_date < current_date");
        }
    };
    let order = match f.sort.as_deref().unwrap_or("updated") {
        "rank" => "i.rank, i.number",
        "created" => "i.created_at DESC",
        "priority" => "i.priority DESC, i.updated_at DESC",
        "due" => "i.due_date NULLS LAST, i.priority DESC",
        "sla" => "i.sla_resolution_due NULLS LAST",
        "key" => "p.key, i.number",
        "updated" => "i.updated_at DESC",
        _ => return Err(bad("sort must be one of rank, created, updated, priority, due, sla, key")),
    };
    let limit = f.limit.unwrap_or(100).clamp(1, 500);
    let offset = f.offset.unwrap_or(0).max(0);
    let mut qb = QueryBuilder::<Postgres>::new(format!("SELECT {ITEM_JSON} {ITEM_FROM}"));
    filters(&mut qb);
    qb.push(format!(" ORDER BY {order} LIMIT ")).push_bind(limit).push(" OFFSET ").push_bind(offset);
    let rows: Vec<Value> = qb.build_query_scalar().fetch_all(&st.db).await?;
    let mut cq = QueryBuilder::<Postgres>::new(format!("SELECT count(*) {ITEM_FROM}"));
    filters(&mut cq);
    let total: i64 = cq.build_query_scalar().fetch_one(&st.db).await?;
    Ok(json!({"items": rows, "total": total, "limit": limit, "offset": offset}))
}

#[derive(Deserialize)]
pub struct LinkBody {
    pub rel: String,
    /// An item key such as `OPS-7`, or `{"type": "page", "id": "<uuid>"}`.
    pub target: Value,
}

pub async fn add_link(st: &St, ctx: &Ctx, key: &str, b: &LinkBody) -> AppResult<Value> {
    let (item, _project, perms) = load_item(st, ctx, key).await?;
    authz::require(st, ctx, &perms, "item.link", &format!("item:{key}"))?;
    if b.rel.len() < 2 || b.rel.len() > 41 || !b.rel.starts_with(|c: char| c.is_ascii_lowercase()) || !b.rel.chars().all(|c| c.is_ascii_lowercase() || c == '_') {
        return Err(bad("rel must be lowercase letters and underscores, e.g. blocks, relates_to, verifies"));
    }
    let (ttype, tid, label) = match &b.target {
        Value::String(k) => {
            let (target, _, _) = load_item(st, ctx, k).await.map_err(|_| bad("target item not found"))?;
            if target.id == item.id {
                return Err(bad("an item cannot link to itself"));
            }
            ("item", target.id.to_string(), k.to_uppercase())
        }
        Value::Object(o) if o.get("type").and_then(Value::as_str) == Some("page") => {
            let pid: Uuid = o.get("id").and_then(Value::as_str).and_then(|s| s.parse().ok()).ok_or_else(|| bad("page id is required"))?;
            let visible = authz::visible_space_ids(st, ctx).await?;
            let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pages WHERE id = $1 AND space_id = ANY($2) AND NOT archived)").bind(pid).bind(&visible).fetch_one(&st.db).await?;
            if !ok {
                return Err(bad("target page not found"));
            }
            ("page", pid.to_string(), format!("page:{pid}"))
        }
        _ => return Err(bad("target must be an item key or {\"type\":\"page\",\"id\":...}")),
    };
    let id = st.stores.graph.add_edge(("item", &item.id.to_string()), &b.rel, (ttype, &tid), json!({}), Some(ctx.user_id)).await.map_err(AppError::Other)?;
    // History names only the relation: the target may be invisible to some readers of this item.
    history(st, item.id, Some(ctx.user_id), "link", Value::Null, json!({"rel": b.rel})).await?;
    audit::log(st, ctx, "link.create", "item", &key.to_uppercase(), json!({"rel": b.rel, "target": label})).await;
    Ok(json!({"id": id}))
}

async fn remove_link(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let edge = st.stores.graph.get_edge(id).await.map_err(AppError::Other)?.ok_or(AppError::NotFound)?;
    let item_id: Uuid = (edge.src_type == "item").then(|| edge.src_id.parse().ok()).flatten().ok_or(AppError::NotFound)?;
    let item = item_by_id(&st, item_id).await?.ok_or(AppError::NotFound)?;
    let project = projects::project_by_id(&st, item.project_id).await?;
    let perms = authz::project_perms(&st, &ctx, project.id, &project.access, project.portal_open()).await?;
    if !perms.has("project.view") {
        return Err(authz::hide(&st, &ctx, "project.view", &format!("link:{id}")));
    }
    authz::require(&st, &ctx, &perms, "item.link", &format!("link:{id}"))?;
    st.stores.graph.remove_edge(id).await.map_err(AppError::Other)?;
    let key = format!("{}-{}", project.key, item.number);
    history(&st, item.id, Some(ctx.user_id), "link", json!({"rel": edge.rel}), Value::Null).await?;
    audit::log(&st, &ctx, "link.delete", "item", &key, json!({"rel": edge.rel, "dst_type": edge.dst_type, "dst_id": edge.dst_id})).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct GraphQuery {
    depth: Option<i32>,
}

/// Dependency and traceability graph around one item, limited to what the caller may see.
pub async fn item_graph(st: &St, ctx: &Ctx, key: &str, depth: i32) -> AppResult<Value> {
    let (item, _, _) = load_item(st, ctx, key).await?;
    let root = ("item".to_string(), item.id.to_string());
    let edges = st.stores.graph.neighborhood(("item", &root.1), depth.clamp(1, 4)).await.map_err(AppError::Other)?;
    let mut nodes: Vec<(String, String)> = vec![root.clone()];
    for e in &edges {
        nodes.push((e.src_type.clone(), e.src_id.clone()));
        nodes.push((e.dst_type.clone(), e.dst_id.clone()));
    }
    nodes.sort();
    nodes.dedup();
    let resolved = resolve_nodes(st, ctx, &nodes).await?;
    let visible = |t: &str, id: &str| !matches!(t, "item" | "page") || resolved.contains_key(&(t.to_string(), id.to_string()));
    let kept: Vec<&crate::storage::Edge> = edges.iter().filter(|e| visible(&e.src_type, &e.src_id) && visible(&e.dst_type, &e.dst_id)).collect();
    let mut out_nodes: Vec<Value> = vec![];
    let mut seen = std::collections::HashSet::new();
    for e in &kept {
        for (t, id) in [(&e.src_type, &e.src_id), (&e.dst_type, &e.dst_id)] {
            if seen.insert((t.clone(), id.clone())) {
                out_nodes.push(resolved.get(&(t.clone(), id.clone())).cloned().unwrap_or_else(|| external_node(t, id, &e.meta)));
            }
        }
    }
    if seen.insert(root.clone()) {
        if let Some(n) = resolved.get(&root) {
            out_nodes.push(n.clone());
        }
    }
    let out_edges: Vec<Value> = kept.iter().map(|e| json!({"id": e.id, "rel": e.rel, "src": {"type": e.src_type, "id": e.src_id}, "dst": {"type": e.dst_type, "id": e.dst_id}})).collect();
    Ok(json!({"root": {"type": "item", "id": item.id}, "nodes": out_nodes, "edges": out_edges}))
}

pub fn clean_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or_default();
    let cleaned: String = base.chars().filter(|c| !crate::accounts::deceptive(*c) && !matches!(c, '"' | '<' | '>' | '|' | ':' | '*' | '?')).collect();
    let cleaned = cleaned.trim().trim_matches('.').to_string();
    if cleaned.is_empty() {
        "file".into()
    } else {
        cleaned.chars().take(200).collect()
    }
}

/// Store an attachment for an item or page. No permission check.
pub async fn store_attachment(st: &AppState, owner_type: &str, owner_id: Uuid, filename: &str, content_type: &str, data: Bytes, by: Uuid) -> AppResult<Value> {
    if data.is_empty() {
        return Err(bad("the file is empty"));
    }
    let name = clean_filename(filename);
    let sha = crypto::sha256_hex(&data);
    let size = data.len() as i64;
    let ct: String = content_type.chars().filter(|c| c.is_ascii_graphic() || *c == ' ').take(120).collect();
    st.stores.blob.put(&sha, data, &ct).await.map_err(AppError::Other)?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO attachments (owner_type, owner_id, filename, content_type, size, sha256, blob_key, uploaded_by) VALUES ($1,$2,$3,$4,$5,$6,$6,$7) RETURNING id")
        .bind(owner_type)
        .bind(owner_id)
        .bind(&name)
        .bind(if ct.is_empty() { "application/octet-stream" } else { ct.as_str() })
        .bind(size)
        .bind(&sha)
        .bind(by)
        .fetch_one(&st.db)
        .await?;
    Ok(json!({"id": id, "filename": name, "size": size, "sha256": sha}))
}

#[derive(Deserialize)]
struct UploadQuery {
    filename: String,
}

async fn upload_attachment(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Query(q): Query<UploadQuery>, headers: HeaderMap, body: Bytes) -> AppResult<(StatusCode, Json<Value>)> {
    let (item, _, perms) = load_item(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "item.comment", &format!("item:{key}"))?;
    let ct = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream");
    let out = store_attachment(&st, "item", item.id, &q.filename, ct, body, ctx.user_id).await?;
    history(&st, item.id, Some(ctx.user_id), "attachment", Value::Null, json!(out["filename"])).await?;
    audit::log(&st, &ctx, "attachment.upload", "item", &key.to_uppercase(), out.clone()).await;
    Ok((StatusCode::CREATED, Json(out)))
}

/// Types a browser may show in place: pictures, audio, video and PDF. Everything else is
/// offered as a download. SVG is shown only through `<img>`, where scripts in it never run,
/// and every response is sandboxed in case it is opened directly.
const INLINE_TYPES: &[&str] = &[
    "image/png", "image/jpeg", "image/gif", "image/webp", "image/svg+xml", "image/avif",
    "audio/mpeg", "audio/mp4", "audio/ogg", "audio/wav", "audio/x-wav", "audio/webm", "audio/flac", "audio/aac",
    "video/mp4", "video/webm", "video/ogg", "video/quicktime",
    "application/pdf",
];

/// Parse a single `Range: bytes=a-b` header against a body of `len` bytes.
/// `None` means "send everything"; `Some(Err(()))` means the range cannot be satisfied.
pub fn byte_range(header: Option<&str>, len: u64) -> Option<Result<(u64, u64), ()>> {
    let spec = header?.trim().strip_prefix("bytes=")?;
    if spec.contains(',') {
        return None;
    }
    let (a, b) = spec.split_once('-')?;
    let range = match (a.trim(), b.trim()) {
        ("", "") => return None,
        ("", suffix) => {
            let n: u64 = suffix.parse().ok()?;
            if n == 0 {
                return Some(Err(()));
            }
            (len.saturating_sub(n), len.saturating_sub(1))
        }
        (start, "") => (start.parse().ok()?, len.saturating_sub(1)),
        (start, end) => (start.parse().ok()?, end.parse::<u64>().ok()?.min(len.saturating_sub(1))),
    };
    Some(if len == 0 || range.0 > range.1 || range.0 >= len { Err(()) } else { Ok(range) })
}

/// Serve an attachment. The response is sandboxed so an uploaded HTML or SVG file cannot script the application.
async fn download_attachment(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, headers: HeaderMap) -> AppResult<Response> {
    let row = sqlx::query("SELECT owner_type, owner_id, filename, content_type, blob_key FROM attachments WHERE id = $1").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let owner_id: Uuid = row.get("owner_id");
    let allowed = match row.get::<String, _>("owner_type").as_str() {
        "item" => match item_by_id(&st, owner_id).await? {
            Some(item) => {
                let project = projects::project_by_id(&st, item.project_id).await?;
                let perms = authz::project_perms(&st, &ctx, project.id, &project.access, project.portal_open()).await?;
                perms.has("project.view") || (perms.has("desk.request") && item.reporter_id == Some(ctx.user_id))
            }
            None => false,
        },
        _ => crate::wiki::can_view_page(&st, &ctx, owner_id).await?,
    };
    if !allowed {
        return Err(authz::hide(&st, &ctx, "project.view", &format!("attachment:{id}")));
    }
    let data = st.stores.blob.get(&row.get::<String, _>("blob_key")).await.map_err(AppError::Other)?.ok_or(AppError::NotFound)?;
    let filename: String = row.get("filename");
    let stored_ct: String = row.get("content_type");
    let base_ct = stored_ct.split(';').next().unwrap_or_default().trim().to_ascii_lowercase();
    let inline = INLINE_TYPES.contains(&base_ct.as_str());
    let encoded: String = url::form_urlencoded::byte_serialize(filename.as_bytes()).collect::<String>().replace('+', "%20");
    let total = data.len() as u64;
    // Media players ask for pieces of a file; a partial request is not logged as a download each time.
    let range = byte_range(headers.get(header::RANGE).and_then(|v| v.to_str().ok()), total);
    if !matches!(range, Some(Ok((start, _))) if start > 0) {
        audit::log(&st, &ctx, "attachment.download", "attachment", &id.to_string(), json!({"filename": filename})).await;
    }
    let mut resp = match range {
        Some(Err(())) => {
            let mut r = StatusCode::RANGE_NOT_SATISFIABLE.into_response();
            r.headers_mut().insert(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes */{total}")).unwrap_or(HeaderValue::from_static("bytes */0")));
            return Ok(r);
        }
        Some(Ok((start, end))) => {
            let mut r = (StatusCode::PARTIAL_CONTENT, data.slice(start as usize..=end as usize)).into_response();
            r.headers_mut().insert(header::CONTENT_RANGE, HeaderValue::from_str(&format!("bytes {start}-{end}/{total}")).unwrap_or(HeaderValue::from_static("bytes */0")));
            r
        }
        None => data.into_response(),
    };
    let h = resp.headers_mut();
    h.insert(header::ACCEPT_RANGES, HeaderValue::from_static("bytes"));
    h.insert(header::CONTENT_TYPE, HeaderValue::from_str(if inline { &base_ct } else { "application/octet-stream" }).unwrap_or(HeaderValue::from_static("application/octet-stream")));
    h.insert(header::CONTENT_DISPOSITION, HeaderValue::from_str(&format!("{}; filename*=UTF-8''{encoded}", if inline { "inline" } else { "attachment" })).unwrap_or(HeaderValue::from_static("attachment")));
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static("sandbox; default-src 'none'; style-src 'unsafe-inline'"));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("private, max-age=300"));
    Ok(resp)
}

async fn delete_attachment(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let row = sqlx::query("SELECT owner_id, filename FROM attachments WHERE id = $1 AND owner_type = 'item'").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let item = item_by_id(&st, row.get("owner_id")).await?.ok_or(AppError::NotFound)?;
    let project = projects::project_by_id(&st, item.project_id).await?;
    let perms = authz::project_perms(&st, &ctx, project.id, &project.access, project.portal_open()).await?;
    if !perms.has("project.view") {
        return Err(authz::hide(&st, &ctx, "project.view", &format!("attachment:{id}")));
    }
    authz::require(&st, &ctx, &perms, "item.edit", &format!("attachment:{id}"))?;
    sqlx::query("DELETE FROM attachments WHERE id = $1").bind(id).execute(&st.db).await?;
    let filename: String = row.get("filename");
    history(&st, item.id, Some(ctx.user_id), "attachment", json!(filename), Value::Null).await?;
    audit::log(&st, &ctx, "attachment.delete", "attachment", &id.to_string(), json!({"filename": filename, "item": format!("{}-{}", project.key, item.number)})).await;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── REST handlers ─────────────────────────

async fn h_list(State(st): State<St>, ctx: Ctx, Query(f): Query<ItemFilter>) -> AppResult<Json<Value>> {
    Ok(Json(list_items(&st, &ctx, &f).await?))
}

async fn h_create(State(st): State<St>, ctx: Ctx, Path(project): Path<String>, Json(n): Json<NewItem>) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::CREATED, Json(create_item_checked(&st, &ctx, &project, n, "web").await?)))
}

async fn h_get(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    let (item, project, perms) = load_item(&st, &ctx, &key).await?;
    Ok(Json(detail_json(&st, &ctx, item.id, &project, &perms).await?))
}

async fn h_update(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(changes): Json<Map<String, Value>>) -> AppResult<Json<Value>> {
    Ok(Json(update_item(&st, &ctx, &key, changes).await?))
}

async fn h_archive(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<StatusCode> {
    let (item, _, perms) = load_item(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "item.delete", &format!("item:{key}"))?;
    sqlx::query("UPDATE items SET archived = true, updated_at = now() WHERE id = $1").bind(item.id).execute(&st.db).await?;
    history(&st, item.id, Some(ctx.user_id), "archived", json!(false), json!(true)).await?;
    index_item(&st, item.id).await;
    audit::log(&st, &ctx, "item.archive", "item", &key.to_uppercase(), json!({"title": item.title})).await;
    let ukey = key.to_uppercase();
    let project_key = ukey.rsplit_once('-').map(|k| k.0).unwrap_or(&ukey);
    crate::realtime::publish_work(&st, item.project_id, project_key, Some(&ukey), "item.archived", json!({"key": ukey, "actor_id": ctx.user_id})).await;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct TransitionBody {
    to: String,
}

async fn h_transition(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<TransitionBody>) -> AppResult<Json<Value>> {
    Ok(Json(transition_item(&st, &ctx, &key, &b.to).await?))
}

#[derive(Deserialize)]
struct CommentBody {
    body: String,
    #[serde(default)]
    internal: bool,
}

async fn h_comment(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<CommentBody>) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::CREATED, Json(add_comment(&st, &ctx, &key, &b.body, b.internal).await?)))
}

async fn h_link(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<LinkBody>) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::CREATED, Json(add_link(&st, &ctx, &key, &b).await?)))
}

async fn h_graph(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Query(q): Query<GraphQuery>) -> AppResult<Json<Value>> {
    Ok(Json(item_graph(&st, &ctx, &key, q.depth.unwrap_or(2)).await?))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/items", "Work items", "List and filter work items across visible projects", get(h_list))
        .route("POST", "/api/v1/projects/{key}/items", "Work items", "Create a work item", post(h_create))
        .route("GET", "/api/v1/items/{key}", "Work items", "Work item with comments, links, history and allowed transitions", get(h_get))
        .route("PATCH", "/api/v1/items/{key}", "Work items", "Update fields of a work item", axum::routing::patch(h_update))
        .route("DELETE", "/api/v1/items/{key}", "Work items", "Archive a work item", delete(h_archive))
        .route("POST", "/api/v1/items/{key}/transition", "Work items", "Move a work item to another status", post(h_transition))
        .route("GET", "/api/v1/items/{key}/comments", "Discussion", "The discussion on a work item", get(h_comments))
        .route("POST", "/api/v1/items/{key}/comments", "Discussion", "Post a message (optionally an internal note)", post(h_comment))
        .route("PATCH", "/api/v1/comments/{id}", "Discussion", "Edit one's own message", axum::routing::patch(edit_comment))
        .route("DELETE", "/api/v1/comments/{id}", "Discussion", "Remove a message (one's own, or any with the moderation permission)", delete(delete_comment))
        .route("GET", "/api/v1/items/{key}/history", "Work items", "Every change to a work item: field, old and new value, who and when", get(h_history))
        .route("POST", "/api/v1/items/{key}/watch", "Work items", "Follow a work item", post(h_watch))
        .route("DELETE", "/api/v1/items/{key}/watch", "Work items", "Stop following a work item", delete(h_unwatch))
        .route("POST", "/api/v1/items/{key}/links", "Work items", "Link a work item to an item or page", post(h_link))
        .route("DELETE", "/api/v1/links/{id}", "Work items", "Remove a link", delete(remove_link))
        .route("GET", "/api/v1/items/{key}/graph", "Work items", "Dependency and traceability graph around an item", get(h_graph))
        .route("POST", "/api/v1/items/{key}/attachments", "Attachments", "Upload a file (raw body, ?filename=)", post(upload_attachment))
        .route("GET", "/api/v1/attachments/{id}", "Attachments", "Download a file", get(download_attachment))
        .route("DELETE", "/api/v1/attachments/{id}", "Attachments", "Delete a file", delete(delete_attachment))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::packs::{StatusDef, TransitionDef};

    fn type_info() -> TypeInfo {
        TypeInfo {
            key: "bug".into(),
            name: "Bug".into(),
            workflow: WorkflowDef {
                key: "w".into(),
                name: "W".into(),
                initial: "open".into(),
                statuses: vec![StatusDef { key: "open".into(), name: "Open".into(), category: "todo".into() }, StatusDef { key: "done".into(), name: "Done".into(), category: "done".into() }],
                transitions: vec![TransitionDef { name: "Close".into(), from: vec!["open".into()], to: "done".into(), requires: vec![] }],
                layout: None,
            },
            fields: vec![
                FieldInfo { key: "severity".into(), name: "Severity".into(), kind: "select".into(), options: vec!["Major".into(), "Minor".into()], required: true },
                FieldInfo { key: "points".into(), name: "Points".into(), kind: "number".into(), options: vec![], required: false },
                FieldInfo { key: "seen".into(), name: "Seen on".into(), kind: "date".into(), options: vec![], required: false },
                FieldInfo { key: "link".into(), name: "Link".into(), kind: "url".into(), options: vec![], required: false },
                FieldInfo { key: "tags".into(), name: "Tags".into(), kind: "multiselect".into(), options: vec!["a".into(), "b".into()], required: false },
                FieldInfo { key: "flag".into(), name: "Flag".into(), kind: "checkbox".into(), options: vec![], required: false },
            ],
        }
    }

    fn obj(v: Value) -> Map<String, Value> {
        v.as_object().unwrap().clone()
    }

    #[test]
    fn item_keys_parse() {
        assert_eq!(parse_key("OPS-12"), Some(("OPS".into(), 12)));
        assert_eq!(parse_key("ops-7"), Some(("OPS".into(), 7)));
        assert_eq!(parse_key(" ENG2-001 "), Some(("ENG2".into(), 1)));
        for bad in ["OPS", "OPS-", "-12", "OPS-1a", "OPS-12-3x", "O-1", "OPS--1", "OPS-99999999999", "'; DROP TABLE items;--"] {
            assert_eq!(parse_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn custom_fields_are_type_checked() {
        let ti = type_info();
        assert!(validate_custom(&ti, &obj(json!({"severity": "Major", "points": 3, "seen": "2026-01-31", "link": "https://x.test/a", "tags": ["a"], "flag": true})), true).is_ok());
        assert!(validate_custom(&ti, &obj(json!({"severity": "Catastrophic"})), false).is_err(), "option not in list");
        assert!(validate_custom(&ti, &obj(json!({"points": "three"})), false).is_err());
        assert!(validate_custom(&ti, &obj(json!({"seen": "31/01/2026"})), false).is_err());
        assert!(validate_custom(&ti, &obj(json!({"link": "javascript:alert(1)"})), false).is_err(), "only http(s) links");
        assert!(validate_custom(&ti, &obj(json!({"tags": ["a", "z"]})), false).is_err());
        assert!(validate_custom(&ti, &obj(json!({"flag": "yes"})), false).is_err());
        assert!(validate_custom(&ti, &obj(json!({"unknown": 1})), false).is_err(), "unknown fields are rejected");
        assert!(validate_custom(&ti, &obj(json!({"points": null})), false).is_ok(), "null clears a value");
    }

    #[test]
    fn required_fields_are_enforced_only_when_asked() {
        let ti = type_info();
        assert!(validate_custom(&ti, &Map::new(), true).is_err());
        assert!(validate_custom(&ti, &obj(json!({"severity": ""})), true).is_err());
        assert!(validate_custom(&ti, &Map::new(), false).is_ok());
    }

    #[test]
    fn byte_ranges() {
        assert_eq!(byte_range(None, 100), None);
        assert_eq!(byte_range(Some("bytes=0-9"), 100), Some(Ok((0, 9))));
        assert_eq!(byte_range(Some("bytes=90-"), 100), Some(Ok((90, 99))));
        assert_eq!(byte_range(Some("bytes=-10"), 100), Some(Ok((90, 99))));
        assert_eq!(byte_range(Some("bytes=50-1000"), 100), Some(Ok((50, 99))), "the end is clamped");
        assert_eq!(byte_range(Some("bytes=100-"), 100), Some(Err(())));
        assert_eq!(byte_range(Some("bytes=9-2"), 100), Some(Err(())));
        assert_eq!(byte_range(Some("bytes=-0"), 100), Some(Err(())));
        assert_eq!(byte_range(Some("bytes=0-"), 0), Some(Err(())));
        // Several ranges, other units and nonsense fall back to the whole file.
        assert_eq!(byte_range(Some("bytes=0-1,5-6"), 100), None);
        assert_eq!(byte_range(Some("items=0-1"), 100), None);
        assert_eq!(byte_range(Some("bytes=a-b"), 100), None);
        assert_eq!(byte_range(Some("bytes=-"), 100), None);
        assert_eq!(byte_range(Some("bytes=18446744073709551615-"), 100), Some(Err(())));
    }

    #[test]
    fn labels_are_normalized() {
        assert_eq!(clean_labels(&[" Backend ".into(), "backend".into(), "".into(), "API".into()]).unwrap(), vec!["api", "backend"]);
        assert!(clean_labels(&["x".repeat(51)]).is_err());
        assert!(clean_labels(&(0..31).map(|i| format!("l{i}")).collect::<Vec<_>>()).is_err());
    }

    #[test]
    fn filenames_are_sanitized() {
        assert_eq!(clean_filename("../../etc/passwd"), "passwd");
        assert_eq!(clean_filename("C:\\Users\\x\\report.pdf"), "report.pdf");
        assert_eq!(clean_filename("a\"b<c>.txt"), "abc.txt");
        assert_eq!(clean_filename("line\nbreak.txt"), "linebreak.txt");
        assert_eq!(clean_filename("..."), "file");
        assert_eq!(clean_filename(""), "file");
    }

    #[test]
    fn filled_values() {
        assert!(!is_filled(None) && !is_filled(Some(&Value::Null)) && !is_filled(Some(&json!("  "))) && !is_filled(Some(&json!([]))));
        assert!(is_filled(Some(&json!("x"))) && is_filled(Some(&json!(0))) && is_filled(Some(&json!(false))) && is_filled(Some(&json!(["a"]))));
    }
}
