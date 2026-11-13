//! Projects: creation from templates, settings, membership, cycles, and the metadata bundle
//! (types, fields, workflows) that drives forms and boards in the client.

use crate::auth::Ctx;
use crate::authz::{self, Perms};
use crate::error::{bad, AppError, AppResult};
use crate::http::Api;
use crate::packs::{SeedItem, SeedPage, TemplateDef};
use crate::{audit, items, wiki, AppState, St};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Project {
    pub id: Uuid,
    pub key: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    pub access: String,
    pub default_type: String,
    pub lead_id: Option<Uuid>,
    pub settings: Value,
    pub archived: bool,
}

const PROJECT_COLS: &str = "id, key, name, description, kind, access, default_type, lead_id, settings, archived";

impl Project {
    pub fn portal_open(&self) -> bool {
        self.kind == "service_desk" && self.settings.get("portal_open").and_then(Value::as_bool).unwrap_or(false)
    }
    pub fn json(&self) -> Value {
        json!({"id": self.id, "key": self.key, "name": self.name, "description": self.description, "kind": self.kind, "access": self.access,
               "default_type": self.default_type, "lead_id": self.lead_id, "settings": self.settings, "archived": self.archived})
    }
}

pub fn valid_project_key(k: &str) -> bool {
    let b = k.as_bytes();
    (2..=10).contains(&b.len()) && b[0].is_ascii_uppercase() && b.iter().all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

pub async fn find_project(st: &AppState, key: &str) -> AppResult<Option<Project>> {
    Ok(sqlx::query_as(&format!("SELECT {PROJECT_COLS} FROM projects WHERE key = $1")).bind(key.to_uppercase()).fetch_optional(&st.db).await?)
}

pub async fn project_by_id(st: &AppState, id: Uuid) -> AppResult<Project> {
    Ok(sqlx::query_as(&format!("SELECT {PROJECT_COLS} FROM projects WHERE id = $1")).bind(id).fetch_one(&st.db).await?)
}

/// Load a project and the caller's permissions on it. Callers without any access get 404.
pub async fn load_project_any(st: &St, ctx: &Ctx, key: &str) -> AppResult<(Project, Perms)> {
    let project = find_project(st, key).await?.ok_or(AppError::NotFound)?;
    let perms = authz::project_perms(st, ctx, project.id, &project.access, project.portal_open()).await?;
    if !perms.has("project.view") && !perms.has("desk.request") {
        return Err(authz::hide(st, ctx, "project.view", &format!("project:{}", project.key)));
    }
    Ok((project, perms))
}

/// Load a project the caller may view as a team member (not merely as a portal customer).
pub async fn load_project(st: &St, ctx: &Ctx, key: &str) -> AppResult<(Project, Perms)> {
    let (project, perms) = load_project_any(st, ctx, key).await?;
    if !perms.has("project.view") {
        return Err(authz::hide(st, ctx, "project.view", &format!("project:{}", project.key)));
    }
    Ok((project, perms))
}

async fn list_projects(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    let visible = authz::visible_project_ids(&st, &ctx).await?;
    Ok(Json(
        sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.name), '[]'::jsonb) FROM (
               SELECT p.id, p.key, p.name, p.description, p.kind, p.access, p.template_key,
                      (SELECT person_json(u) FROM users u WHERE u.id = p.lead_id) AS lead,
                      (SELECT count(*) FROM items i WHERE i.project_id = p.id AND NOT i.archived AND i.status_category <> 'done') AS open_items,
                      (SELECT count(*) FROM items i WHERE i.project_id = p.id AND NOT i.archived) AS total_items
               FROM projects p WHERE p.id = ANY($1)) t",
        )
        .bind(&visible)
        .fetch_one(&st.db)
        .await?,
    ))
}

async fn list_templates(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    Ok(Json(
        sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(jsonb_build_object('key', key, 'name', name, 'domain', domain, 'kind', kind, 'description', description, 'source', source,
                      'item_types', definition->'item_types') ORDER BY domain, name), '[]'::jsonb) FROM project_templates",
        )
        .fetch_one(&st.db)
        .await?,
    ))
}

#[derive(Deserialize)]
struct NewProject {
    key: String,
    name: String,
    #[serde(default)]
    description: String,
    template: String,
    #[serde(default)]
    access: Option<String>,
}

async fn create_project(State(st): State<St>, ctx: Ctx, Json(b): Json<NewProject>) -> AppResult<(StatusCode, Json<Value>)> {
    authz::require_global(&st, &ctx, "project.create")?;
    let key = b.key.trim().to_uppercase();
    if !valid_project_key(&key) {
        return Err(bad("key must be 2-10 characters: an uppercase letter followed by uppercase letters or digits"));
    }
    if b.name.trim().is_empty() || b.name.len() > 200 {
        return Err(bad("name must be 1 to 200 characters"));
    }
    let access = b.access.unwrap_or_else(|| "private".into());
    if !["private", "internal"].contains(&access.as_str()) {
        return Err(bad("access must be private or internal"));
    }
    let def: Value = sqlx::query_scalar("SELECT definition FROM project_templates WHERE key = $1").bind(&b.template).fetch_optional(&st.db).await?.ok_or_else(|| bad("unknown project template"))?;
    let tpl: TemplateDef = serde_json::from_value(def).map_err(|e| AppError::Other(e.into()))?;

    let mut tx = st.db.begin().await?;
    let settings = if tpl.kind == "service_desk" { json!({"portal_open": tpl.portal_open, "calendar": {"mode": "24x7"}}) } else { json!({}) };
    let id: Uuid = sqlx::query_scalar("INSERT INTO projects (key, name, description, kind, access, template_key, default_type, lead_id, settings, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$8) RETURNING id")
        .bind(&key)
        .bind(b.name.trim())
        .bind(&b.description)
        .bind(&tpl.kind)
        .bind(&access)
        .bind(&tpl.key)
        .bind(&tpl.default_type)
        .bind(ctx.user_id)
        .bind(settings)
        .fetch_one(&mut *tx)
        .await?;
    for t in &tpl.item_types {
        sqlx::query("INSERT INTO project_types (project_id, type_key) VALUES ($1, $2)").bind(id).bind(t).execute(&mut *tx).await?;
    }
    for c in &tpl.cycles {
        sqlx::query("INSERT INTO cycles (project_id, name) VALUES ($1, $2)").bind(id).bind(c).execute(&mut *tx).await?;
    }
    for (pos, r) in tpl.request_types.iter().enumerate() {
        let form: Vec<Value> = r.fields.iter().map(|f| json!({"key": f, "required": false})).collect();
        sqlx::query("INSERT INTO request_types (project_id, name, description, icon, type_key, form, position) VALUES ($1,$2,$3,$4,$5,$6,$7)")
            .bind(id)
            .bind(&r.name)
            .bind(&r.description)
            .bind(&r.icon)
            .bind(&r.type_key)
            .bind(json!(form))
            .bind(pos as i32)
            .execute(&mut *tx)
            .await?;
    }
    for s in &tpl.sla {
        sqlx::query("INSERT INTO sla_policies (project_id, priority, response_minutes, resolution_minutes) VALUES ($1,$2,$3,$4)").bind(id).bind(s.priority).bind(s.response_minutes).bind(s.resolution_minutes).execute(&mut *tx).await?;
    }
    sqlx::query("INSERT INTO role_bindings (role_id, user_id, scope_type, scope_id) SELECT id, $1, 'project', $2 FROM roles WHERE key = 'project_admin'").bind(ctx.user_id).bind(id).execute(&mut *tx).await?;
    tx.commit().await?;

    let project = project_by_id(&st, id).await?;
    seed_items(&st, &ctx, &project, &tpl.items, None).await?;
    if let Some(space) = &tpl.space {
        // The linked space reuses the project key so the pair is easy to recognize; fall back if taken.
        let space_key = if wiki::space_key_free(&st, &key).await? { key.clone() } else { format!("{}D", &key[..key.len().min(9)]) };
        let space_id = wiki::create_space_raw(&st, &ctx, &space_key, &space.name, "", &space.kind, &access, Some(id)).await?;
        seed_pages(&st, &ctx, space_id, &space.pages, None).await?;
    }
    audit::log(&st, &ctx, "project.create", "project", &key, json!({"name": b.name.trim(), "template": tpl.key, "access": access})).await;
    Ok((StatusCode::CREATED, Json(project.json())))
}

fn seed_items<'a>(st: &'a St, ctx: &'a Ctx, project: &'a Project, seeds: &'a [SeedItem], parent: Option<Uuid>) -> std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<()>> + Send + 'a>> {
    Box::pin(async move {
        for s in seeds {
            let new = items::NewItem { type_key: Some(s.type_key.clone()), title: s.title.clone(), description: Some(s.description.clone()), labels: Some(s.labels.clone()), custom: Some(s.custom.clone()), parent_id: parent, channel: Some("template".into()), lenient: true, ..Default::default() };
            let (id, _) = items::create_item(st, ctx.user_id, project, new).await?;
            seed_items(st, ctx, project, &s.children, Some(id)).await?;
        }
        Ok(())
    })
}

fn seed_pages<'a>(st: &'a St, ctx: &'a Ctx, space_id: Uuid, seeds: &'a [SeedPage], parent: Option<Uuid>) -> std::pin::Pin<Box<dyn std::future::Future<Output = AppResult<()>> + Send + 'a>> {
    Box::pin(async move {
        for (pos, s) in seeds.iter().enumerate() {
            let body = match (&s.body, &s.template) {
                (Some(b), _) => b.clone(),
                (None, Some(t)) => sqlx::query_scalar("SELECT body FROM page_templates WHERE key = $1").bind(t).fetch_optional(&st.db).await?.unwrap_or_default(),
                (None, None) => String::new(),
            };
            let id = wiki::create_page_raw(st, ctx.user_id, space_id, parent, &s.title, &body, "published", s.customer_visible, pos as i32).await?;
            seed_pages(st, ctx, space_id, &s.children, Some(id)).await?;
        }
        Ok(())
    })
}

/// Types enabled in a project, each with its field definitions and workflow.
pub async fn project_types_json(st: &AppState, project_id: Uuid) -> AppResult<Value> {
    Ok(sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(jsonb_build_object(
            'key', t.key, 'name', t.name, 'icon', t.icon, 'color', t.color, 'description', t.description,
            'workflow', coalesce(pw.definition, w.definition) || jsonb_build_object('key', w.key, 'name', w.name, 'own', pw.project_id IS NOT NULL),
            'shared_workflow', CASE WHEN pw.project_id IS NULL THEN NULL ELSE w.definition END,
            'level', (SELECT jsonb_build_object('id', l.id, 'name', l.name, 'position', l.position) FROM hierarchy_levels l WHERE l.id = t.level_id),
            'fields', (SELECT coalesce(jsonb_agg(jsonb_build_object('key', f.key, 'name', f.name, 'type', f.type, 'options', f.options, 'description', f.description,
                         'required', coalesce((tf.value->>'required')::boolean, false)) ORDER BY tf.ord), '[]'::jsonb)
                       FROM jsonb_array_elements(t.fields) WITH ORDINALITY AS tf(value, ord) JOIN fields f ON f.key = tf.value->>'key')
          ) ORDER BY t.name), '[]'::jsonb)
         FROM project_types pt JOIN item_types t ON t.key = pt.type_key JOIN workflows w ON w.key = t.workflow_key
              LEFT JOIN project_workflows pw ON pw.project_id = pt.project_id AND pw.type_key = t.key
         WHERE pt.project_id = $1",
    )
    .bind(project_id)
    .fetch_one(&st.db)
    .await?)
}

async fn members_json(st: &AppState, project_id: Uuid, scope: &str) -> AppResult<Value> {
    Ok(sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(jsonb_build_object('binding_id', b.id, 'role', r.key, 'role_name', r.name,
            'user', CASE WHEN u.id IS NULL THEN NULL ELSE person_json(u) || jsonb_build_object('email', u.email) END,
            'group', CASE WHEN g.id IS NULL THEN NULL ELSE jsonb_build_object('id', g.id, 'name', g.name) END) ORDER BY coalesce(u.name, g.name)), '[]'::jsonb)
         FROM role_bindings b JOIN roles r ON r.id = b.role_id LEFT JOIN users u ON u.id = b.user_id LEFT JOIN groups g ON g.id = b.group_id
         WHERE b.scope_type = $2 AND b.scope_id = $1",
    )
    .bind(project_id)
    .bind(scope)
    .fetch_one(&st.db)
    .await?)
}

async fn get_project(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    let (project, perms) = load_project(&st, &ctx, &key).await?;
    let cycles: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(to_jsonb(c) ORDER BY c.created_at), '[]'::jsonb) FROM (SELECT id, name, goal, start_date, end_date, state, created_at FROM cycles WHERE project_id = $1) c")
        .bind(project.id)
        .fetch_one(&st.db)
        .await?;
    let space: Option<Value> = sqlx::query_scalar("SELECT jsonb_build_object('key', key, 'name', name, 'kind', kind) FROM spaces WHERE project_id = $1 AND NOT archived LIMIT 1").bind(project.id).fetch_optional(&st.db).await?;
    let counts: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object(
            'todo', count(*) FILTER (WHERE status_category = 'todo'),
            'in_progress', count(*) FILTER (WHERE status_category = 'in_progress'),
            'done', count(*) FILTER (WHERE status_category = 'done'),
            'overdue', count(*) FILTER (WHERE status_category <> 'done' AND due_date < current_date),
            'unassigned', count(*) FILTER (WHERE status_category <> 'done' AND assignee_id IS NULL),
            'sla_breached', count(*) FILTER (WHERE status_category <> 'done' AND (sla_resolution_breached OR sla_resolution_due < now())))
         FROM items WHERE project_id = $1 AND NOT archived",
    )
    .bind(project.id)
    .fetch_one(&st.db)
    .await?;
    let perm_list: Vec<String> = perms.list().into_iter().filter(|p| ctx.token_allows(p) && authz::PERMISSIONS.iter().any(|(k, scope, _)| k == p && *scope == "project")).collect();
    let lead: Option<Value> = sqlx::query_scalar("SELECT person_json(u) FROM users u WHERE u.id = $1").bind(project.lead_id).fetch_optional(&st.db).await?;
    Ok(Json(json!({
        "project": project.json(),
        "lead": lead,
        "container": crate::planning::chain_of_project(&st, project.id).await?,
        "permissions": perm_list,
        "types": project_types_json(&st, project.id).await?,
        "cycles": cycles,
        "members": members_json(&st, project.id, "project").await?,
        "space": space,
        "counts": counts,
    })))
}

async fn update_project(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<serde_json::Map<String, Value>>) -> AppResult<Json<Value>> {
    let (project, perms) = load_project(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "project.admin", &format!("project:{}", project.key))?;
    let mut p = project.clone();
    for (k, v) in &b {
        match k.as_str() {
            "name" => p.name = v.as_str().filter(|s| !s.trim().is_empty() && s.len() <= 200 && !s.chars().any(crate::accounts::deceptive)).ok_or_else(|| bad("name must be 1 to 200 characters"))?.trim().to_string(),
            "description" => p.description = v.as_str().ok_or_else(|| bad("description must be text"))?.to_string(),
            "access" => p.access = v.as_str().filter(|s| ["private", "internal"].contains(s)).ok_or_else(|| bad("access must be private or internal"))?.to_string(),
            "archived" => p.archived = v.as_bool().ok_or_else(|| bad("archived must be true or false"))?,
            "lead_id" => p.lead_id = if v.is_null() { None } else { Some(v.as_str().and_then(|s| s.parse().ok()).ok_or_else(|| bad("lead_id must be a user id"))?) },
            "default_type" => {
                let t = v.as_str().ok_or_else(|| bad("default_type must be text"))?;
                let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM project_types WHERE project_id = $1 AND type_key = $2)").bind(project.id).bind(t).fetch_one(&st.db).await?;
                if !ok {
                    return Err(bad("default_type must be one of the project's work item types"));
                }
                p.default_type = t.to_string();
            }
            "settings" => {
                let s = v.as_object().ok_or_else(|| bad("settings must be an object"))?;
                let mut merged = p.settings.as_object().cloned().unwrap_or_default();
                for (sk, sv) in s {
                    match sk.as_str() {
                        "portal_open" if sv.is_boolean() => {}
                        "calendar" => crate::desk::Calendar::from_json(sv).map(|_| ()).map_err(bad)?,
                        _ => return Err(bad(format!("unknown or invalid setting {sk:?}"))),
                    }
                    merged.insert(sk.clone(), sv.clone());
                }
                p.settings = Value::Object(merged);
            }
            other => return Err(bad(format!("unknown field {other:?}"))),
        }
    }
    sqlx::query("UPDATE projects SET name=$2, description=$3, access=$4, archived=$5, lead_id=$6, default_type=$7, settings=$8, updated_at=now() WHERE id=$1")
        .bind(p.id)
        .bind(&p.name)
        .bind(&p.description)
        .bind(&p.access)
        .bind(p.archived)
        .bind(p.lead_id)
        .bind(&p.default_type)
        .bind(&p.settings)
        .execute(&st.db)
        .await?;
    audit::log(&st, &ctx, "project.update", "project", &p.key, json!({"changed": b.keys().collect::<Vec<_>>()})).await;
    changed(&st, &p).await;
    Ok(Json(p.json()))
}

/// Tell open tabs that the project's settings (not its items) changed.
pub async fn changed(st: &AppState, p: &Project) {
    crate::realtime::publish_work(st, p.id, &p.key, None, "project.updated", json!({"project": p.key})).await;
}

/// Accounts that can open the project: the choices for an assignee, and for mentions.
async fn people(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    let (project, _) = load_project(&st, &ctx, &key).await?;
    let all: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM users WHERE active").fetch_all(&st.db).await?;
    let able = authz::project_audience(&st, project.id, "project.view", &all).await?;
    Ok(Json(sqlx::query_scalar("SELECT coalesce(jsonb_agg(person_json(u) ORDER BY u.name), '[]'::jsonb) FROM users u WHERE u.id = ANY($1)").bind(&able).fetch_one(&st.db).await?))
}

#[derive(Deserialize)]
struct WorkflowBody {
    /// `initial`, `statuses`, `transitions` and optionally `layout`. `null` goes back to the shared workflow.
    definition: Option<crate::packs::WorkflowDef>,
    /// For items whose status is not in the new workflow: `{"old_status": "new_status"}`.
    #[serde(default)]
    mapping: std::collections::HashMap<String, String>,
}

/// Give one work item type its own workflow in this project, or return it to the shared one.
/// Items already in a status the new workflow lacks must be mapped to one it has.
async fn put_workflow(State(st): State<St>, ctx: Ctx, Path((key, type_key)): Path<(String, String)>, Json(b): Json<WorkflowBody>) -> AppResult<Json<Value>> {
    let (project, perms) = load_project(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "project.admin", &format!("project:{}", project.key))?;
    let enabled: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM project_types WHERE project_id = $1 AND type_key = $2)").bind(project.id).bind(&type_key).fetch_one(&st.db).await?;
    if !enabled {
        return Err(AppError::NotFound);
    }
    let own = b.definition.is_some();
    let target = match b.definition {
        Some(mut def) => {
            def.key = String::new();
            def.name = String::new();
            def.validate().map_err(bad)?;
            def
        }
        None => items::load_type(&st, &type_key).await?.workflow,
    };
    // Custom fields a transition requires must exist on the type.
    let ti = items::load_type(&st, &type_key).await?;
    if let Some(missing) = target.transitions.iter().flat_map(|t| &t.requires).find(|r| !ti.fields.iter().any(|f| &f.key == *r)) {
        return Err(bad(format!("a transition requires {missing:?}, which is not a field of {}", ti.name)));
    }
    let in_use: Vec<String> = sqlx::query_scalar("SELECT DISTINCT status FROM items WHERE project_id = $1 AND type_key = $2").bind(project.id).bind(&type_key).fetch_all(&st.db).await?;
    let mut moves: Vec<(String, String)> = vec![];
    for status in &in_use {
        if target.status(status).is_some() {
            continue;
        }
        let to = b.mapping.get(status).ok_or_else(|| bad(format!("work items are in the status {status:?}, which the new workflow does not have; say which status they should move to")))?;
        if target.status(to).is_none() {
            return Err(bad(format!("{status:?} is mapped to {to:?}, which is not a status of the new workflow")));
        }
        moves.push((status.clone(), to.clone()));
    }
    let mut tx = st.db.begin().await?;
    for (from, to) in &moves {
        let moved: Vec<Uuid> = sqlx::query_scalar("UPDATE items SET status = $4, updated_at = now() WHERE project_id = $1 AND type_key = $2 AND status = $3 RETURNING id").bind(project.id).bind(&type_key).bind(from).bind(to).fetch_all(&mut *tx).await?;
        sqlx::query("INSERT INTO item_history (item_id, actor_id, field, old, new) SELECT i, $2, 'status', $3, $4 FROM unnest($1::uuid[]) AS i")
            .bind(&moved)
            .bind(ctx.user_id)
            .bind(json!({"key": from, "name": from, "reason": "workflow changed"}))
            .bind(json!({"key": to, "name": target.status(to).map(|s| s.name.as_str()).unwrap_or(to)}))
            .execute(&mut *tx)
            .await?;
    }
    if own {
        sqlx::query("INSERT INTO project_workflows (project_id, type_key, definition, updated_by) VALUES ($1,$2,$3,$4) ON CONFLICT (project_id, type_key) DO UPDATE SET definition = EXCLUDED.definition, updated_by = EXCLUDED.updated_by, updated_at = now()").bind(project.id).bind(&type_key).bind(target.definition()).bind(ctx.user_id).execute(&mut *tx).await?;
    } else {
        sqlx::query("DELETE FROM project_workflows WHERE project_id = $1 AND type_key = $2").bind(project.id).bind(&type_key).execute(&mut *tx).await?;
    }
    // A status may have moved to another category (for example from in progress to done).
    for s in &target.statuses {
        sqlx::query("UPDATE items SET status_category = $4, resolved_at = CASE WHEN $4 = 'done' THEN coalesce(resolved_at, now()) ELSE NULL END WHERE project_id = $1 AND type_key = $2 AND status = $3 AND status_category <> $4").bind(project.id).bind(&type_key).bind(&s.key).bind(&s.category).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    audit::log(&st, &ctx, "project.workflow_update", "project", &project.key, json!({"type": type_key, "own": own, "statuses": target.statuses.iter().map(|s| &s.key).collect::<Vec<_>>(), "moved": moves})).await;
    changed(&st, &project).await;
    Ok(Json(json!({"types": project_types_json(&st, project.id).await?})))
}

#[derive(Deserialize)]
pub struct MemberBody {
    pub user_id: Option<Uuid>,
    pub group_id: Option<Uuid>,
    pub role: String,
}

/// Bind a role on a project or space. Shared by projects and wiki spaces.
pub async fn add_binding(st: &AppState, ctx: &Ctx, scope: &str, scope_id: Uuid, label: &str, b: &MemberBody) -> AppResult<Uuid> {
    if b.user_id.is_some() == b.group_id.is_some() {
        return Err(bad("give exactly one of user_id or group_id"));
    }
    let role_id: Uuid = sqlx::query_scalar("SELECT id FROM roles WHERE key = $1 AND scope = $2").bind(&b.role).bind(scope).fetch_optional(&st.db).await?.ok_or_else(|| bad(format!("role must be a {scope} role")))?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO role_bindings (role_id, user_id, group_id, scope_type, scope_id) VALUES ($1,$2,$3,$4,$5) RETURNING id").bind(role_id).bind(b.user_id).bind(b.group_id).bind(scope).bind(scope_id).fetch_one(&st.db).await?;
    audit::log(st, ctx, "member.add", scope, label, json!({"role": b.role, "user_id": b.user_id, "group_id": b.group_id})).await;
    Ok(id)
}

pub async fn remove_binding(st: &AppState, ctx: &Ctx, scope: &str, scope_id: Uuid, label: &str, binding: Uuid) -> AppResult<()> {
    let row: Option<(String, Option<Uuid>, Option<Uuid>)> = sqlx::query_as("DELETE FROM role_bindings b USING roles r WHERE r.id = b.role_id AND b.id = $1 AND b.scope_type = $2 AND b.scope_id = $3 RETURNING r.key, b.user_id, b.group_id")
        .bind(binding)
        .bind(scope)
        .bind(scope_id)
        .fetch_optional(&st.db)
        .await?;
    let (role, user_id, group_id) = row.ok_or(AppError::NotFound)?;
    audit::log(st, ctx, "member.remove", scope, label, json!({"role": role, "user_id": user_id, "group_id": group_id})).await;
    Ok(())
}

async fn add_member(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<MemberBody>) -> AppResult<(StatusCode, Json<Value>)> {
    let (project, perms) = load_project(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "project.admin", &format!("project:{}", project.key))?;
    let id = add_binding(&st, &ctx, "project", project.id, &project.key, &b).await?;
    Ok((StatusCode::CREATED, Json(json!({"binding_id": id}))))
}

async fn remove_member(State(st): State<St>, ctx: Ctx, Path((key, binding)): Path<(String, Uuid)>) -> AppResult<StatusCode> {
    let (project, perms) = load_project(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "project.admin", &format!("project:{}", project.key))?;
    remove_binding(&st, &ctx, "project", project.id, &project.key, binding).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Deserialize)]
struct CycleBody {
    name: Option<String>,
    goal: Option<String>,
    start_date: Option<chrono::NaiveDate>,
    end_date: Option<chrono::NaiveDate>,
    state: Option<String>,
}

async fn create_cycle(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<CycleBody>) -> AppResult<(StatusCode, Json<Value>)> {
    let (project, perms) = load_project(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "project.admin", &format!("project:{}", project.key))?;
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("name is required"))?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO cycles (project_id, name, goal, start_date, end_date) VALUES ($1,$2,$3,$4,$5) RETURNING id").bind(project.id).bind(name).bind(b.goal.unwrap_or_default()).bind(b.start_date).bind(b.end_date).fetch_one(&st.db).await?;
    audit::log(&st, &ctx, "cycle.create", "cycle", &id.to_string(), json!({"project": project.key, "name": name})).await;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

async fn update_cycle(State(st): State<St>, ctx: Ctx, Path((key, id)): Path<(String, Uuid)>, Json(b): Json<CycleBody>) -> AppResult<StatusCode> {
    let (project, perms) = load_project(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "project.admin", &format!("project:{}", project.key))?;
    if let Some(s) = &b.state {
        if !["planned", "active", "closed"].contains(&s.as_str()) {
            return Err(bad("state must be planned, active or closed"));
        }
    }
    let n = sqlx::query("UPDATE cycles SET name = coalesce($3, name), goal = coalesce($4, goal), start_date = coalesce($5, start_date), end_date = coalesce($6, end_date), state = coalesce($7, state) WHERE id = $1 AND project_id = $2")
        .bind(id)
        .bind(project.id)
        .bind(b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()))
        .bind(&b.goal)
        .bind(b.start_date)
        .bind(b.end_date)
        .bind(&b.state)
        .execute(&st.db)
        .await?
        .rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    audit::log(&st, &ctx, "cycle.update", "cycle", &id.to_string(), json!({"project": project.key, "state": b.state})).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Catalog data that project and space administrators need for pickers. No sensitive content.
async fn catalog(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    let roles: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('key', key, 'name', name, 'scope', scope, 'description', description) ORDER BY builtin DESC, name), '[]'::jsonb) FROM roles WHERE scope IN ('project', 'space')").fetch_one(&st.db).await?;
    let groups: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'name', name) ORDER BY name), '[]'::jsonb) FROM groups").fetch_one(&st.db).await?;
    let types: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('key', key, 'name', name, 'icon', icon, 'color', color, 'description', description, 'source', source) ORDER BY name), '[]'::jsonb) FROM item_types WHERE active").fetch_one(&st.db).await?;
    Ok(Json(json!({"roles": roles, "groups": groups, "item_types": types})))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/catalog", "Projects", "Roles, groups and work item types for pickers", get(catalog))
        .route("GET", "/api/v1/projects", "Projects", "List projects visible to the caller", get(list_projects))
        .route("POST", "/api/v1/projects", "Projects", "Create a project from a template", post(create_project))
        .route("GET", "/api/v1/project-templates", "Projects", "List project templates", get(list_templates))
        .route("GET", "/api/v1/projects/{key}", "Projects", "Project with types, workflows, members and counts", get(get_project))
        .route("PATCH", "/api/v1/projects/{key}", "Projects", "Update project settings", patch(update_project))
        .route("GET", "/api/v1/projects/{key}/people", "Projects", "Accounts that can open the project (assignee and mention choices)", get(people))
        .route("PUT", "/api/v1/projects/{key}/workflows/{type}", "Projects", "Give a work item type its own workflow in this project, or return to the shared one", axum::routing::put(put_workflow))
        .route("POST", "/api/v1/projects/{key}/members", "Projects", "Grant a project role to a user or group", post(add_member))
        .route("DELETE", "/api/v1/projects/{key}/members/{binding}", "Projects", "Remove a project role grant", delete(remove_member))
        .route("POST", "/api/v1/projects/{key}/cycles", "Projects", "Create a cycle (sprint, iteration)", post(create_cycle))
        .route("PATCH", "/api/v1/projects/{key}/cycles/{id}", "Projects", "Update a cycle", patch(update_cycle))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_keys() {
        for ok in ["OPS", "A1", "ENG2024", "ABCDEFGHIJ"] {
            assert!(valid_project_key(ok), "{ok}");
        }
        for no in ["", "A", "ops", "1AB", "AB-C", "ABCDEFGHIJK", "AB C", "ÀB"] {
            assert!(!valid_project_key(no), "{no}");
        }
    }

    #[test]
    fn portal_is_open_only_for_service_desks_that_opt_in() {
        let mut p = Project { id: Uuid::nil(), key: "IT".into(), name: "IT".into(), description: String::new(), kind: "service_desk".into(), access: "private".into(), default_type: "service_request".into(), lead_id: None, settings: json!({"portal_open": true}), archived: false };
        assert!(p.portal_open());
        p.settings = json!({});
        assert!(!p.portal_open());
        p.settings = json!({"portal_open": true});
        p.kind = "standard".into();
        assert!(!p.portal_open());
    }
}
