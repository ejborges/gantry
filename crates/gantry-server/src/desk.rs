//! Service desk: request types, the customer portal, agent queues, and service-level targets.
//!
//! A service desk is a project of kind `service_desk`. Customers hold only `desk.request`:
//! they reach the desk through the `/portal` endpoints, see their own requests, and never see
//! internal notes. Agents hold `desk.agent` and work the same items through the normal item API.

use crate::auth::Ctx;
use crate::authz::{self, Perms};
use crate::error::{bad, AppError, AppResult};
use crate::events::{self, Event};
use crate::http::Api;
use crate::items::{self, Item};
use crate::projects::{self, Project};
use crate::storage::SearchScope;
use crate::{audit, AppState, St};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::{delete, get, patch, post, put};
use axum::Json;
use chrono::{DateTime, Datelike, Duration, NaiveTime, Timelike, Utc};
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::Row;
use uuid::Uuid;

// ───────────────────────── working calendar ─────────────────────────

/// Working hours for SLA clocks. A fixed UTC offset is used (no daylight-saving rules).
#[derive(Debug, Clone, PartialEq)]
pub struct Calendar {
    pub utc_offset_minutes: i32,
    /// Monday = index 0.
    pub days: [bool; 7],
    pub start_minute: u32,
    pub end_minute: u32,
}

fn parse_hhmm(s: &str) -> Option<u32> {
    let (h, m) = s.split_once(':')?;
    let (h, m): (u32, u32) = (h.parse().ok()?, m.parse().ok()?);
    (h <= 24 && m < 60 && h * 60 + m <= 1440).then_some(h * 60 + m)
}

impl Calendar {
    /// `{"mode":"24x7"}` → `None`; `{"mode":"business","utc_offset_minutes":-300,"days":[1,2,3,4,5],"start":"08:00","end":"17:00"}`
    /// (days: 1 = Monday … 7 = Sunday) → `Some`.
    pub fn from_json(v: &Value) -> Result<Option<Calendar>, String> {
        match v.get("mode").and_then(Value::as_str) {
            Some("24x7") | None => Ok(None),
            Some("business") => {
                let offset = v.get("utc_offset_minutes").and_then(Value::as_i64).unwrap_or(0);
                if !(-840..=840).contains(&offset) {
                    return Err("utc_offset_minutes must be between -840 and 840".into());
                }
                let mut days = [false; 7];
                for d in v.get("days").and_then(Value::as_array).ok_or("days must be a list of weekday numbers, 1 = Monday")? {
                    let n = d.as_i64().filter(|n| (1..=7).contains(n)).ok_or("days must contain numbers from 1 (Monday) to 7 (Sunday)")?;
                    days[(n - 1) as usize] = true;
                }
                if !days.iter().any(|d| *d) {
                    return Err("at least one working day is required".into());
                }
                let start = v.get("start").and_then(Value::as_str).and_then(parse_hhmm).ok_or("start must look like 08:00")?;
                let end = v.get("end").and_then(Value::as_str).and_then(parse_hhmm).ok_or("end must look like 17:00")?;
                if start >= end {
                    return Err("start must be earlier than end".into());
                }
                Ok(Some(Calendar { utc_offset_minutes: offset as i32, days, start_minute: start, end_minute: end }))
            }
            Some(other) => Err(format!("unknown calendar mode {other:?}; use 24x7 or business")),
        }
    }
}

/// The instant that lies `minutes` of working time after `start`.
pub fn add_working_minutes(start: DateTime<Utc>, minutes: i64, cal: Option<&Calendar>) -> DateTime<Utc> {
    let Some(cal) = cal else { return start + Duration::minutes(minutes) };
    let offset = Duration::minutes(cal.utc_offset_minutes as i64);
    let mut cursor = (start + offset).naive_utc();
    let mut remaining = minutes.max(0) * 60;
    let day_start = |d: chrono::NaiveDate| d.and_time(NaiveTime::MIN) + Duration::minutes(cal.start_minute as i64);
    for _ in 0..20_000 {
        let working_day = cal.days[cursor.weekday().num_days_from_monday() as usize];
        let sec_of_day = cursor.time().num_seconds_from_midnight() as i64;
        let (open, close) = (cal.start_minute as i64 * 60, cal.end_minute as i64 * 60);
        if !working_day || sec_of_day >= close {
            cursor = day_start(cursor.date() + Duration::days(1));
            continue;
        }
        if sec_of_day < open {
            cursor = day_start(cursor.date());
            continue;
        }
        let take = (close - sec_of_day).min(remaining);
        cursor += Duration::seconds(take);
        remaining -= take;
        if remaining == 0 {
            break;
        }
    }
    DateTime::<Utc>::from_naive_utc_and_offset(cursor, Utc) - offset
}

/// Response and resolution deadlines for a new or re-prioritized item, if the project has a policy.
pub async fn sla_targets(st: &AppState, project: &Project, priority: i16, from: DateTime<Utc>) -> AppResult<(Option<DateTime<Utc>>, Option<DateTime<Utc>>)> {
    if project.kind != "service_desk" {
        return Ok((None, None));
    }
    let row: Option<(i32, i32)> = sqlx::query_as("SELECT response_minutes, resolution_minutes FROM sla_policies WHERE project_id = $1 AND priority = $2").bind(project.id).bind(priority).fetch_optional(&st.db).await?;
    let Some((resp, res)) = row else { return Ok((None, None)) };
    let cal = project.settings.get("calendar").map(Calendar::from_json).transpose().ok().flatten().flatten();
    Ok((Some(add_working_minutes(from, resp as i64, cal.as_ref())), Some(add_working_minutes(from, res as i64, cal.as_ref()))))
}

/// Mark newly breached targets and announce them once. Run periodically by the worker.
pub async fn sla_scan(st: &AppState) -> anyhow::Result<u64> {
    let rows = sqlx::query(
        "WITH hit AS (
           UPDATE items SET
             sla_resolution_breached = sla_resolution_breached OR (sla_resolution_due < now()),
             sla_response_breached = sla_response_breached OR (first_response_at IS NULL AND sla_response_due < now())
           WHERE NOT archived AND status_category <> 'done'
             AND ((NOT sla_resolution_breached AND sla_resolution_due < now()) OR (NOT sla_response_breached AND first_response_at IS NULL AND sla_response_due < now()))
           RETURNING id, project_id, number, title, assignee_id, sla_resolution_breached AS resolution)
         SELECT hit.id, hit.project_id, p.key || '-' || hit.number AS key, hit.title, hit.assignee_id, hit.resolution FROM hit JOIN projects p ON p.id = hit.project_id",
    )
    .fetch_all(&st.db)
    .await?;
    for r in &rows {
        let key: String = r.get("key");
        let title: String = r.get("title");
        let (id, project_id): (Uuid, Uuid) = (r.get("id"), r.get("project_id"));
        let target = if r.get::<bool, _>("resolution") { "resolution" } else { "response" };
        items::history(st, id, None, "sla", Value::Null, json!({"missed": target})).await.map_err(|e| anyhow::anyhow!("{e:?}"))?;
        // The owner hears about it; a request nobody owns is everyone's problem in that desk.
        let notify = match r.get::<Option<Uuid>, _>("assignee_id") {
            Some(a) => vec![a],
            None => authz::project_members_with(st, project_id, "desk.agent").await?,
        };
        events::emit(
            st,
            Event {
                kind: "sla.breached",
                project_id: Some(project_id),
                space_id: None,
                actor_id: None,
                actor_name: "Gantry",
                subject: format!("{key} {title}"),
                url: format!("/i/{key}"),
                summary: format!("{key} has missed its {target} target"),
                data: json!({"key": key, "target": target}),
                notify,
                item: Some((id, key.clone())),
                page: None,
                excerpt: String::new(),
                skip: vec![],
            },
        )
        .await;
    }
    Ok(rows.len() as u64)
}

// ───────────────────────── agent side ─────────────────────────

async fn desk_project(st: &St, ctx: &Ctx, key: &str, permission: &str) -> AppResult<(Project, Perms)> {
    let (project, perms) = projects::load_project(st, ctx, key).await?;
    if project.kind != "service_desk" {
        return Err(bad("this project is not a service desk"));
    }
    authz::require(st, ctx, &perms, permission, &format!("project:{}", project.key))?;
    Ok((project, perms))
}

async fn queues(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    let (project, _) = desk_project(&st, &ctx, &key, "desk.agent").await?;
    let c = sqlx::query(
        "SELECT count(*) FILTER (WHERE status_category <> 'done') AS open,
                count(*) FILTER (WHERE status_category <> 'done' AND assignee_id IS NULL) AS unassigned,
                count(*) FILTER (WHERE status_category <> 'done' AND assignee_id = $2) AS mine,
                count(*) FILTER (WHERE status_category <> 'done' AND (sla_resolution_breached OR sla_resolution_due < now() OR (first_response_at IS NULL AND sla_response_due < now()))) AS breached,
                count(*) FILTER (WHERE status_category <> 'done' AND sla_resolution_due BETWEEN now() AND now() + interval '2 hours') AS at_risk
         FROM items WHERE project_id = $1 AND NOT archived",
    )
    .bind(project.id)
    .bind(ctx.user_id)
    .fetch_one(&st.db)
    .await?;
    let q = |k: &str, name: &str, col: &str, filter: Value| json!({"key": k, "name": name, "count": c.get::<i64, _>(col), "filter": filter});
    let open = "todo,in_progress";
    Ok(Json(json!([
        q("unassigned", "Unassigned", "unassigned", json!({"category": open, "assignee": "none", "sort": "sla"})),
        q("mine", "Assigned to me", "mine", json!({"category": open, "assignee": "me", "sort": "sla"})),
        q("breached", "Target missed", "breached", json!({"sla": "breached", "sort": "sla"})),
        q("at_risk", "Due within 2 hours", "at_risk", json!({"sla": "at_risk", "sort": "sla"})),
        q("open", "All open", "open", json!({"category": open, "sort": "sla"})),
    ])))
}

const REQUEST_TYPES_SQL: &str = "SELECT coalesce(jsonb_agg(jsonb_build_object(
    'id', rt.id, 'name', rt.name, 'description', rt.description, 'icon', rt.icon, 'type', rt.type_key, 'position', rt.position, 'active', rt.active,
    'fields', (SELECT coalesce(jsonb_agg(jsonb_build_object('key', f.key, 'name', f.name, 'type', f.type, 'options', f.options, 'description', f.description,
                 'required', coalesce((x.value->>'required')::boolean, false)) ORDER BY x.ord), '[]'::jsonb)
               FROM jsonb_array_elements(rt.form) WITH ORDINALITY AS x(value, ord) JOIN fields f ON f.key = x.value->>'key')
  ) ORDER BY rt.position, rt.name), '[]'::jsonb)
  FROM request_types rt WHERE rt.project_id = $1 AND (rt.active OR $2)";

async fn list_request_types(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    let (project, _) = desk_project(&st, &ctx, &key, "project.view").await?;
    Ok(Json(sqlx::query_scalar(REQUEST_TYPES_SQL).bind(project.id).bind(true).fetch_one(&st.db).await?))
}

#[derive(Deserialize)]
struct RequestTypeBody {
    name: Option<String>,
    description: Option<String>,
    icon: Option<String>,
    #[serde(rename = "type")]
    type_key: Option<String>,
    /// `[{"key": "affected_system", "required": true}]`
    fields: Option<Vec<Value>>,
    position: Option<i32>,
    active: Option<bool>,
}

async fn check_form(st: &AppState, project: &Project, type_key: &str, fields: &[Value]) -> AppResult<()> {
    let enabled: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM project_types WHERE project_id = $1 AND type_key = $2)").bind(project.id).bind(type_key).fetch_one(&st.db).await?;
    if !enabled {
        return Err(bad("type must be one of the desk's work item types"));
    }
    let ti = items::load_type_in(st, Some(project.id), type_key).await?;
    for f in fields {
        let k = f.get("key").and_then(Value::as_str).ok_or_else(|| bad("each form field needs a key"))?;
        if !ti.fields.iter().any(|x| x.key == k) {
            return Err(bad(format!("{k:?} is not a field of {}", ti.name)));
        }
    }
    Ok(())
}

async fn create_request_type(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<RequestTypeBody>) -> AppResult<(StatusCode, Json<Value>)> {
    let (project, _) = desk_project(&st, &ctx, &key, "project.admin").await?;
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty() && n.len() <= 200).ok_or_else(|| bad("name is required"))?;
    let type_key = b.type_key.clone().unwrap_or_else(|| project.default_type.clone());
    let fields = b.fields.unwrap_or_default();
    check_form(&st, &project, &type_key, &fields).await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO request_types (project_id, name, description, icon, type_key, form, position) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id")
        .bind(project.id)
        .bind(name)
        .bind(b.description.unwrap_or_default())
        .bind(b.icon.unwrap_or_else(|| "help-circle".into()))
        .bind(&type_key)
        .bind(json!(fields))
        .bind(b.position.unwrap_or(100))
        .fetch_one(&st.db)
        .await?;
    audit::log(&st, &ctx, "request_type.create", "request_type", &id.to_string(), json!({"project": project.key, "name": name})).await;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

async fn update_request_type(State(st): State<St>, ctx: Ctx, Path((key, id)): Path<(String, Uuid)>, Json(b): Json<RequestTypeBody>) -> AppResult<StatusCode> {
    let (project, _) = desk_project(&st, &ctx, &key, "project.admin").await?;
    let current: String = sqlx::query_scalar("SELECT type_key FROM request_types WHERE id = $1 AND project_id = $2").bind(id).bind(project.id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let type_key = b.type_key.clone().unwrap_or(current);
    if let Some(f) = &b.fields {
        check_form(&st, &project, &type_key, f).await?;
    } else if b.type_key.is_some() {
        check_form(&st, &project, &type_key, &[]).await?;
    }
    sqlx::query("UPDATE request_types SET name = coalesce($3, name), description = coalesce($4, description), icon = coalesce($5, icon), type_key = $6, form = coalesce($7, form), position = coalesce($8, position), active = coalesce($9, active) WHERE id = $1 AND project_id = $2")
        .bind(id)
        .bind(project.id)
        .bind(b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()))
        .bind(&b.description)
        .bind(&b.icon)
        .bind(&type_key)
        .bind(b.fields.as_ref().map(|f| json!(f)))
        .bind(b.position)
        .bind(b.active)
        .execute(&st.db)
        .await?;
    audit::log(&st, &ctx, "request_type.update", "request_type", &id.to_string(), json!({"project": project.key})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_request_type(State(st): State<St>, ctx: Ctx, Path((key, id)): Path<(String, Uuid)>) -> AppResult<StatusCode> {
    let (project, _) = desk_project(&st, &ctx, &key, "project.admin").await?;
    let n = sqlx::query("DELETE FROM request_types WHERE id = $1 AND project_id = $2").bind(id).bind(project.id).execute(&st.db).await?.rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    audit::log(&st, &ctx, "request_type.delete", "request_type", &id.to_string(), json!({"project": project.key})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn get_sla(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    let (project, _) = desk_project(&st, &ctx, &key, "project.view").await?;
    let policies: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('priority', priority, 'response_minutes', response_minutes, 'resolution_minutes', resolution_minutes) ORDER BY priority DESC), '[]'::jsonb) FROM sla_policies WHERE project_id = $1").bind(project.id).fetch_one(&st.db).await?;
    Ok(Json(json!({"policies": policies, "calendar": project.settings.get("calendar").cloned().unwrap_or(json!({"mode": "24x7"}))})))
}

#[derive(Deserialize)]
struct SlaRow {
    priority: i16,
    response_minutes: i32,
    resolution_minutes: i32,
}
#[derive(Deserialize)]
struct SlaBody {
    policies: Vec<SlaRow>,
}

async fn put_sla(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<SlaBody>) -> AppResult<StatusCode> {
    let (project, _) = desk_project(&st, &ctx, &key, "project.admin").await?;
    if b.policies.iter().any(|p| !(0..=4).contains(&p.priority) || p.response_minutes <= 0 || p.resolution_minutes <= 0 || p.response_minutes > p.resolution_minutes) {
        return Err(bad("each row needs a priority from 0 to 4 and positive minutes, with response no longer than resolution"));
    }
    let mut tx = st.db.begin().await?;
    sqlx::query("DELETE FROM sla_policies WHERE project_id = $1").bind(project.id).execute(&mut *tx).await?;
    for p in &b.policies {
        sqlx::query("INSERT INTO sla_policies (project_id, priority, response_minutes, resolution_minutes) VALUES ($1,$2,$3,$4)").bind(project.id).bind(p.priority).bind(p.response_minutes).bind(p.resolution_minutes).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    audit::log(&st, &ctx, "sla.update", "project", &project.key, json!({"rows": b.policies.len()})).await;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── customer portal ─────────────────────────

async fn portal_desks(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    let rows: Vec<Project> = sqlx::query_as("SELECT id, key, name, description, kind, access, default_type, lead_id, settings, archived FROM projects WHERE kind = 'service_desk' AND NOT archived ORDER BY name").fetch_all(&st.db).await?;
    let mut out = vec![];
    for p in rows {
        let perms = authz::project_perms(&st, &ctx, p.id, &p.access, p.portal_open()).await?;
        if perms.has("desk.request") && ctx.token_allows("desk.request") {
            out.push(json!({"key": p.key, "name": p.name, "description": p.description}));
        }
    }
    Ok(Json(json!(out)))
}

async fn portal_project(st: &St, ctx: &Ctx, key: &str) -> AppResult<Project> {
    let (project, perms) = projects::load_project_any(st, ctx, key).await?;
    if project.kind != "service_desk" || project.archived {
        return Err(AppError::NotFound);
    }
    authz::require(st, ctx, &perms, "desk.request", &format!("desk:{}", project.key))?;
    Ok(project)
}

async fn portal_desk(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    let project = portal_project(&st, &ctx, &key).await?;
    let types: Value = sqlx::query_scalar(REQUEST_TYPES_SQL).bind(project.id).bind(false).fetch_one(&st.db).await?;
    Ok(Json(json!({"desk": {"key": project.key, "name": project.name, "description": project.description}, "request_types": types})))
}

#[derive(Deserialize)]
struct PortalRequest {
    request_type_id: Uuid,
    summary: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    fields: Map<String, Value>,
}

async fn portal_create(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<PortalRequest>) -> AppResult<(StatusCode, Json<Value>)> {
    let project = portal_project(&st, &ctx, &key).await?;
    let rt = sqlx::query("SELECT name, type_key, form FROM request_types WHERE id = $1 AND project_id = $2 AND active").bind(b.request_type_id).bind(project.id).fetch_optional(&st.db).await?.ok_or_else(|| bad("unknown request type"))?;
    let form: Vec<Value> = serde_json::from_value(rt.get("form")).unwrap_or_default();
    // Customers may fill in only the fields the request type exposes.
    for k in b.fields.keys() {
        if !form.iter().any(|f| f["key"].as_str() == Some(k.as_str())) {
            return Err(bad(format!("{k:?} is not part of this request form")));
        }
    }
    if let Some(missing) = form.iter().find(|f| f["required"].as_bool().unwrap_or(false) && !items::is_filled(b.fields.get(f["key"].as_str().unwrap_or_default()))) {
        return Err(bad(format!("{} is required", missing["key"].as_str().unwrap_or("a field"))));
    }
    let new = items::NewItem { type_key: Some(rt.get("type_key")), title: b.summary.clone(), description: Some(b.description), custom: Some(b.fields), channel: Some("portal".into()), request_type_id: Some(b.request_type_id), lenient: true, ..Default::default() };
    let excerpt = new.description.clone().unwrap_or_default();
    let (item_id, item_key) = items::create_item(&st, ctx.user_id, &project, new).await?;
    audit::log(&st, &ctx, "request.create", "item", &item_key, json!({"desk": project.key, "request_type": rt.get::<String, _>("name")})).await;
    let agents = authz::project_members_with(&st, project.id, "desk.agent").await?;
    events::emit(
        &st,
        Event {
            kind: "request.created",
            project_id: Some(project.id),
            space_id: None,
            actor_id: Some(ctx.user_id),
            actor_name: &ctx.name,
            subject: format!("{item_key} {}", b.summary.trim()),
            url: format!("/i/{item_key}"),
            summary: format!("{} raised {item_key}: {}", ctx.name, b.summary.trim()),
            data: json!({"key": item_key, "request_type": rt.get::<String, _>("name")}),
            notify: agents,
            item: Some((item_id, item_key.clone())),
            page: None,
            excerpt,
            skip: vec![],
        },
    )
    .await;
    Ok((StatusCode::CREATED, Json(json!({"key": item_key}))))
}

async fn portal_my_requests(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    Ok(Json(
        sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.updated_at DESC), '[]'::jsonb) FROM (
               SELECT p.key || '-' || i.number AS key, i.title, i.status, i.status_category, i.created_at, i.updated_at, p.name AS desk,
                      item_status_name(i.project_id, i.type_key, i.status) AS status_name,
                      (SELECT name FROM request_types rt WHERE rt.id = i.request_type_id) AS request_type
               FROM items i JOIN projects p ON p.id = i.project_id
               WHERE i.reporter_id = $1 AND p.kind = 'service_desk' AND NOT i.archived AND NOT p.archived
               ORDER BY i.updated_at DESC LIMIT 200) t",
        )
        .bind(ctx.user_id)
        .fetch_one(&st.db)
        .await?,
    ))
}

/// A request as its reporter may see it: their own, in a desk they can still use.
async fn load_request(st: &St, ctx: &Ctx, key: &str) -> AppResult<(Item, Project)> {
    let item = items::find_item(st, key).await?.ok_or(AppError::NotFound)?;
    let project = projects::project_by_id(st, item.project_id).await?;
    let perms = authz::project_perms(st, ctx, project.id, &project.access, project.portal_open()).await?;
    if project.kind == "service_desk" && item.reporter_id == Some(ctx.user_id) && perms.has("desk.request") && ctx.token_allows("desk.request") {
        Ok((item, project))
    } else {
        Err(authz::hide(st, ctx, "desk.request", &format!("request:{key}")))
    }
}

async fn portal_request(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    let (item, project) = load_request(&st, &ctx, &key).await?;
    let status_name: String = sqlx::query_scalar("SELECT item_status_name($1, $2, $3)").bind(project.id).bind(&item.type_key).bind(&item.status).fetch_one(&st.db).await?;
    // A customer sees the people taking part in their request, and nobody else in the directory.
    let participants: Vec<Uuid> = sqlx::query_scalar("SELECT DISTINCT u FROM (SELECT author_id AS u FROM comments WHERE item_id = $1 AND NOT internal UNION SELECT assignee_id FROM items WHERE id = $1 UNION SELECT reporter_id FROM items WHERE id = $1) t WHERE u IS NOT NULL").bind(item.id).fetch_all(&st.db).await?;
    let comments = items::comments_json(&st, &ctx, item.id, false, crate::refs::People::Only(&participants), None).await?;
    let assignee: Option<Value> = sqlx::query_scalar("SELECT person_json(u) FROM users u WHERE u.id = $1").bind(item.assignee_id).fetch_optional(&st.db).await?;
    let attachments: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'filename', filename, 'size', size, 'created_at', created_at) ORDER BY created_at), '[]'::jsonb) FROM attachments WHERE owner_type = 'item' AND owner_id = $1").bind(item.id).fetch_one(&st.db).await?;
    Ok(Json(json!({
        "key": format!("{}-{}", project.key, item.number), "desk": {"key": project.key, "name": project.name},
        "title": item.title, "description": item.description, "status": item.status, "status_name": status_name,
        "status_category": item.status_category, "created_at": item.created_at, "comments": comments, "attachments": attachments, "assignee": assignee,
    })))
}

#[derive(Deserialize)]
struct PortalComment {
    body: String,
}

async fn portal_comment(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<PortalComment>) -> AppResult<(StatusCode, Json<Value>)> {
    let (item, project) = load_request(&st, &ctx, &key).await?;
    // References are checked under the customer's own access, which is usually none.
    let refs = crate::refs::check(&st, &ctx, &b.body, None).await?;
    let id = items::insert_comment(&st, &item, &project, ctx.user_id, &b.body, false, false, &refs).await?;
    // A reply from the customer puts a waiting request back in front of the agents.
    if item.status.starts_with("waiting") {
        let ti = items::load_type_in(&st, Some(project.id), &item.type_key).await?;
        if let Some(next) = ti.workflow.available(&item.status).into_iter().find(|t| ti.workflow.status(&t.to).map(|s| s.category == "in_progress").unwrap_or(false)) {
            items::apply_status(&st, Some(ctx.user_id), &item, &ti.workflow, &next.to.clone()).await?;
        }
    }
    let ukey = format!("{}-{}", project.key, item.number);
    audit::log(&st, &ctx, "request.comment", "item", &ukey, json!({"comment_id": id})).await;
    events::emit(
        &st,
        Event {
            kind: "comment.created",
            project_id: Some(project.id),
            space_id: None,
            actor_id: Some(ctx.user_id),
            actor_name: &ctx.name,
            subject: format!("{ukey} {}", item.title),
            url: format!("/i/{ukey}"),
            summary: format!("{} replied on {ukey}", ctx.name),
            data: json!({"key": ukey, "comment_id": id, "from_customer": true}),
            // An unowned request is answered by whoever in the desk sees the reply first.
            notify: match item.assignee_id {
                Some(a) => vec![a],
                None => authz::project_members_with(&st, project.id, "desk.agent").await?,
            },
            item: Some((item.id, ukey.clone())),
            page: None,
            excerpt: b.body.clone(),
            skip: vec![],
        },
    )
    .await;
    let mut out = items::comments_json(&st, &ctx, item.id, false, crate::refs::People::Only(&[ctx.user_id]), Some(id)).await?;
    Ok((StatusCode::CREATED, Json(out.pop().unwrap_or(json!({"id": id})))))
}

#[derive(Deserialize)]
struct UploadQuery {
    filename: String,
}

async fn portal_attach(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Query(q): Query<UploadQuery>, headers: HeaderMap, body: Bytes) -> AppResult<(StatusCode, Json<Value>)> {
    let (item, project) = load_request(&st, &ctx, &key).await?;
    let ct = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream");
    let out = items::store_attachment(&st, "item", item.id, &q.filename, ct, body, ctx.user_id).await?;
    items::history(&st, item.id, Some(ctx.user_id), "attachment", Value::Null, json!(out["filename"])).await?;
    audit::log(&st, &ctx, "attachment.upload", "item", &format!("{}-{}", project.key, item.number), out.clone()).await;
    Ok((StatusCode::CREATED, Json(out)))
}

#[derive(Deserialize)]
struct SuggestQuery {
    q: String,
}

async fn kb_spaces(st: &AppState, project_id: Uuid) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar("SELECT id FROM spaces WHERE project_id = $1 AND kind = 'knowledge_base' AND NOT archived").bind(project_id).fetch_all(&st.db).await?)
}

/// Knowledge base articles that may answer the question before a request is filed.
async fn portal_suggest(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Query(q): Query<SuggestQuery>) -> AppResult<Json<Value>> {
    let project = portal_project(&st, &ctx, &key).await?;
    let scope = SearchScope { projects: vec![], spaces: kb_spaces(&st, project.id).await?, kinds: vec!["page".into()], customer_visible_only: true };
    let hits = st.stores.search.search(&q.q, &scope, 5).await.map_err(AppError::Other)?;
    Ok(Json(json!(hits.iter().map(|h| json!({"id": h.ref_id, "title": h.title, "snippet": h.snippet})).collect::<Vec<_>>())))
}

async fn portal_articles(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    let project = portal_project(&st, &ctx, &key).await?;
    Ok(Json(
        sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'title', title, 'updated_at', updated_at) ORDER BY title), '[]'::jsonb) FROM pages WHERE space_id = ANY($1) AND customer_visible AND status = 'published' AND NOT archived")
            .bind(kb_spaces(&st, project.id).await?)
            .fetch_one(&st.db)
            .await?,
    ))
}

async fn portal_article(State(st): State<St>, ctx: Ctx, Path((key, id)): Path<(String, Uuid)>) -> AppResult<Json<Value>> {
    let project = portal_project(&st, &ctx, &key).await?;
    let page: Option<Value> = sqlx::query_scalar("SELECT jsonb_build_object('id', id, 'title', title, 'content', content, 'updated_at', updated_at) FROM pages WHERE id = $1 AND space_id = ANY($2) AND customer_visible AND status = 'published' AND NOT archived")
        .bind(id)
        .bind(kb_spaces(&st, project.id).await?)
        .fetch_optional(&st.db)
        .await?;
    Ok(Json(page.ok_or(AppError::NotFound)?))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/projects/{key}/queues", "Service desk", "Agent queues with counts and the item filter for each", get(queues))
        .route("GET", "/api/v1/projects/{key}/request-types", "Service desk", "List request types", get(list_request_types))
        .route("POST", "/api/v1/projects/{key}/request-types", "Service desk", "Create a request type", post(create_request_type))
        .route("PATCH", "/api/v1/projects/{key}/request-types/{id}", "Service desk", "Update a request type", patch(update_request_type))
        .route("DELETE", "/api/v1/projects/{key}/request-types/{id}", "Service desk", "Delete a request type", delete(delete_request_type))
        .route("GET", "/api/v1/projects/{key}/sla", "Service desk", "Service-level targets and working calendar", get(get_sla))
        .route("PUT", "/api/v1/projects/{key}/sla", "Service desk", "Replace service-level targets", put(put_sla))
        .route("GET", "/api/v1/portal/desks", "Portal", "Service desks the caller may raise requests in", get(portal_desks))
        .route("GET", "/api/v1/portal/desks/{key}", "Portal", "A desk's request types and forms", get(portal_desk))
        .route("POST", "/api/v1/portal/desks/{key}/requests", "Portal", "Raise a request", post(portal_create))
        .route("GET", "/api/v1/portal/desks/{key}/suggest", "Portal", "Suggest knowledge base articles for a question", get(portal_suggest))
        .route("GET", "/api/v1/portal/desks/{key}/articles", "Portal", "List customer-visible knowledge base articles", get(portal_articles))
        .route("GET", "/api/v1/portal/desks/{key}/articles/{id}", "Portal", "Read a customer-visible article", get(portal_article))
        .route("GET", "/api/v1/portal/requests", "Portal", "The caller's own requests", get(portal_my_requests))
        .route("GET", "/api/v1/portal/requests/{key}", "Portal", "One of the caller's requests, without internal notes", get(portal_request))
        .route("POST", "/api/v1/portal/requests/{key}/comments", "Portal", "Reply on one's own request", post(portal_comment))
        .route("POST", "/api/v1/portal/requests/{key}/attachments", "Portal", "Attach a file to one's own request", post(portal_attach))
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn business() -> Calendar {
        Calendar::from_json(&json!({"mode": "business", "utc_offset_minutes": 0, "days": [1, 2, 3, 4, 5], "start": "08:00", "end": "17:00"})).unwrap().unwrap()
    }
    fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap()
    }

    #[test]
    fn round_the_clock_is_plain_addition() {
        assert_eq!(add_working_minutes(at(2026, 11, 10, 9, 0), 90, None), at(2026, 11, 10, 10, 30));
    }

    #[test]
    fn within_one_working_day() {
        // Tuesday, November 10, 2026.
        assert_eq!(add_working_minutes(at(2026, 11, 10, 9, 0), 240, Some(&business())), at(2026, 11, 10, 13, 0));
    }

    #[test]
    fn rolls_over_to_the_next_morning() {
        assert_eq!(add_working_minutes(at(2026, 11, 10, 16, 0), 120, Some(&business())), at(2026, 11, 11, 9, 0));
    }

    #[test]
    fn skips_the_weekend() {
        // Friday, November 13, 16:30 + 60 working minutes → Monday, November 16, 08:30.
        assert_eq!(add_working_minutes(at(2026, 11, 13, 16, 30), 60, Some(&business())), at(2026, 11, 16, 8, 30));
    }

    #[test]
    fn starts_counting_at_opening_time() {
        assert_eq!(add_working_minutes(at(2026, 11, 10, 3, 0), 30, Some(&business())), at(2026, 11, 10, 8, 30));
        assert_eq!(add_working_minutes(at(2026, 11, 10, 22, 0), 30, Some(&business())), at(2026, 11, 11, 8, 30));
        assert_eq!(add_working_minutes(at(2026, 11, 14, 12, 0), 30, Some(&business())), at(2026, 11, 16, 8, 30), "Saturday start");
    }

    #[test]
    fn multi_day_targets() {
        // 2 880 minutes = 48 working hours = 5 nine-hour days + 3 hours.
        assert_eq!(add_working_minutes(at(2026, 11, 9, 8, 0), 2880, Some(&business())), at(2026, 11, 16, 11, 0));
    }

    #[test]
    fn honors_the_utc_offset() {
        let cal = Calendar::from_json(&json!({"mode": "business", "utc_offset_minutes": -300, "days": [1, 2, 3, 4, 5], "start": "08:00", "end": "17:00"})).unwrap().unwrap();
        // 12:00 UTC is 07:00 local, before opening; 08:00 local is 13:00 UTC.
        assert_eq!(add_working_minutes(at(2026, 11, 10, 12, 0), 60, Some(&cal)), at(2026, 11, 10, 14, 0));
    }

    #[test]
    fn calendar_validation() {
        assert_eq!(Calendar::from_json(&json!({"mode": "24x7"})).unwrap(), None);
        assert_eq!(Calendar::from_json(&json!({})).unwrap(), None);
        assert!(Calendar::from_json(&json!({"mode": "business", "days": [], "start": "08:00", "end": "17:00"})).is_err());
        assert!(Calendar::from_json(&json!({"mode": "business", "days": [8], "start": "08:00", "end": "17:00"})).is_err());
        assert!(Calendar::from_json(&json!({"mode": "business", "days": [1], "start": "17:00", "end": "08:00"})).is_err());
        assert!(Calendar::from_json(&json!({"mode": "business", "days": [1], "start": "8am", "end": "17:00"})).is_err());
        assert!(Calendar::from_json(&json!({"mode": "lunar"})).is_err());
    }
}
