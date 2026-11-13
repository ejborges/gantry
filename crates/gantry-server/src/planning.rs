//! Planning above and across projects.
//!
//! * **Hierarchy** — administrators name the levels above a project (Program, Initiative, …)
//!   and may rank work item types below it (Epic, Story, Task, …). *Containers* are the
//!   programs and initiatives themselves; projects are placed in them.
//! * **Reports** — statistics and workload for a project or for everything under a container.
//! * **Search** — work items matched by any combination of fields, including custom ones;
//!   searches can be saved, shared, and asked to alert their owner about new matches.
//! * **Gantt charts** and the **hierarchy graph** read their data here.
//!
//! Owning a container gives read access to the projects under it (see `authz::project_perms`),
//! so who may name an owner, move a container or place a project is deliberately narrow:
//! nobody can hand out access they do not hold themselves.

use crate::auth::Ctx;
use crate::authz;
use crate::error::{bad, AppError, AppResult};
use crate::http::{Api, ReqInfo};
use crate::items::{ITEM_FROM, ITEM_JSON};
use crate::notify::{self, Notice};
use crate::{audit, projects, realtime, settings, AppState, St};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post, put};
use axum::Json;
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::{Postgres, QueryBuilder, Row};
use uuid::Uuid;

const MAX_DEPTH: i32 = 20;

// ───────────────────────── hierarchy levels ─────────────────────────

async fn levels_json(st: &AppState, kind: &str) -> AppResult<Value> {
    Ok(sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'position', position, 'name', name, 'plural', plural, 'color', color) ORDER BY position), '[]'::jsonb) FROM hierarchy_levels WHERE kind = $1").bind(kind).fetch_one(&st.db).await?)
}

async fn get_hierarchy(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    let types: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('key', key, 'name', name, 'icon', icon, 'color', color, 'level_id', level_id) ORDER BY name), '[]'::jsonb) FROM item_types WHERE active").fetch_one(&st.db).await?;
    Ok(Json(json!({
        "container_levels": levels_json(&st, "container").await?,
        "item_levels": levels_json(&st, "item").await?,
        "item_types": types,
        "enforce_item_levels": settings::get(&st, "hierarchy").await["enforce_item_levels"],
        "can_manage": ctx.has("planning.manage"),
    })))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LevelIn {
    id: Option<Uuid>,
    name: String,
    plural: Option<String>,
    color: Option<String>,
}

#[derive(Deserialize)]
struct LevelsBody {
    kind: String,
    /// The levels of this kind, from the top. A level left out is removed.
    levels: Vec<LevelIn>,
}

const COLORS: &[&str] = &["neutral", "blue", "green", "amber", "red", "purple", "teal", "pink"];

async fn put_levels(State(st): State<St>, ctx: Ctx, Json(b): Json<LevelsBody>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "planning.manage")?;
    if !["container", "item"].contains(&b.kind.as_str()) {
        return Err(bad("kind must be container or item"));
    }
    if b.levels.len() > 12 {
        return Err(bad("at most 12 levels"));
    }
    let mut names = std::collections::HashSet::new();
    for l in &b.levels {
        let n = l.name.trim();
        if n.is_empty() || n.chars().count() > 60 || l.plural.as_ref().is_some_and(|p| p.trim().is_empty() || p.chars().count() > 60) {
            return Err(bad("each level needs a name of 1 to 60 characters"));
        }
        if !names.insert(n.to_lowercase()) {
            return Err(bad(format!("two levels are both named {n:?}")));
        }
        if l.color.as_ref().is_some_and(|c| !COLORS.contains(&c.as_str())) {
            return Err(bad(format!("color must be one of {}", COLORS.join(", "))));
        }
    }
    let mut tx = st.db.begin().await?;
    let keep: Vec<Uuid> = b.levels.iter().filter_map(|l| l.id).collect();
    let known: i64 = sqlx::query_scalar("SELECT count(*) FROM hierarchy_levels WHERE kind = $1 AND id = ANY($2)").bind(&b.kind).bind(&keep).fetch_one(&mut *tx).await?;
    if known != keep.len() as i64 {
        return Err(bad("a level id does not exist (or belongs to the other kind)"));
    }
    // A level that still holds programs or initiatives cannot be removed from under them.
    let busy: Option<String> = sqlx::query_scalar("SELECT l.name FROM hierarchy_levels l WHERE l.kind = $1 AND NOT (l.id = ANY($2)) AND EXISTS (SELECT 1 FROM containers c WHERE c.level_id = l.id) LIMIT 1").bind(&b.kind).bind(&keep).fetch_optional(&mut *tx).await?;
    if let Some(name) = busy {
        return Err(AppError::Conflict(format!("the level {name:?} is in use; move or archive what is on it first")));
    }
    sqlx::query("DELETE FROM hierarchy_levels WHERE kind = $1 AND NOT (id = ANY($2))").bind(&b.kind).bind(&keep).execute(&mut *tx).await?;
    // The uniqueness of (kind, position) is checked at commit, so levels can swap places.
    for (i, l) in b.levels.iter().enumerate() {
        let (name, plural) = (l.name.trim(), l.plural.as_deref().map(str::trim).map(str::to_string).unwrap_or_else(|| format!("{}s", l.name.trim())));
        match l.id {
            Some(id) => {
                sqlx::query("UPDATE hierarchy_levels SET position = $2, name = $3, plural = $4, color = coalesce($5, color) WHERE id = $1").bind(id).bind(i as i32 + 1).bind(name).bind(&plural).bind(&l.color).execute(&mut *tx).await?;
            }
            None => {
                sqlx::query("INSERT INTO hierarchy_levels (kind, position, name, plural, color) VALUES ($1,$2,$3,$4,$5)").bind(&b.kind).bind(i as i32 + 1).bind(name).bind(&plural).bind(l.color.as_deref().unwrap_or("neutral")).execute(&mut *tx).await?;
            }
        }
    }
    if b.kind == "container" {
        // After a reorder every container must still sit below its parent's level.
        let broken: Option<String> = sqlx::query_scalar("SELECT c.name FROM containers c JOIN hierarchy_levels cl ON cl.id = c.level_id JOIN containers p ON p.id = c.parent_id JOIN hierarchy_levels pl ON pl.id = p.level_id WHERE cl.position <= pl.position AND NOT c.archived LIMIT 1").fetch_optional(&mut *tx).await?;
        if let Some(name) = broken {
            return Err(AppError::Conflict(format!("with this order {name:?} would sit above its own parent; move it first")));
        }
    }
    tx.commit().await?;
    audit::log(&st, &ctx, "hierarchy.levels_update", "hierarchy", &b.kind, json!({"levels": b.levels.iter().map(|l| l.name.trim()).collect::<Vec<_>>()})).await;
    changed(&st).await;
    Ok(Json(levels_json(&st, &b.kind).await?))
}

#[derive(Deserialize)]
struct TypeLevel {
    level_id: Option<Uuid>,
}

async fn put_type_level(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<TypeLevel>) -> AppResult<StatusCode> {
    authz::require_global(&st, &ctx, "planning.manage")?;
    if let Some(l) = b.level_id {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM hierarchy_levels WHERE id = $1 AND kind = 'item')").bind(l).fetch_one(&st.db).await?;
        if !ok {
            return Err(bad("level_id must be a work item level"));
        }
    }
    let n = sqlx::query("UPDATE item_types SET level_id = $2 WHERE key = $1").bind(&key).bind(b.level_id).execute(&st.db).await?.rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    audit::log(&st, &ctx, "hierarchy.type_level", "item_type", &key, json!({"level_id": b.level_id})).await;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── containers ─────────────────────────

async fn changed(st: &AppState) {
    realtime::publish(st, "planning", json!({"t": "event", "room": "planning", "kind": "planning.updated", "data": {}})).await;
}

/// The containers a project sits in, outermost first (for breadcrumbs).
pub async fn chain_of_project(st: &AppState, project: Uuid) -> AppResult<Value> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE up(id, parent_id, name, level_id, depth) AS (
            SELECT c.id, c.parent_id, c.name, c.level_id, 0 FROM containers c JOIN projects p ON p.container_id = c.id WHERE p.id = $1
            UNION ALL SELECT c.id, c.parent_id, c.name, c.level_id, up.depth + 1 FROM containers c JOIN up ON c.id = up.parent_id WHERE up.depth < $2)
         SELECT coalesce(jsonb_agg(jsonb_build_object('id', up.id, 'name', up.name, 'level', l.name) ORDER BY up.depth DESC), '[]'::jsonb) FROM up JOIN hierarchy_levels l ON l.id = up.level_id",
    )
    .bind(project)
    .bind(MAX_DEPTH)
    .fetch_one(&st.db)
    .await?)
}

/// Ids of `root` and every container beneath it.
async fn subtree(st: &AppState, root: Uuid) -> AppResult<Vec<Uuid>> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE down(id, depth) AS (SELECT id, 0 FROM containers WHERE id = $1
            UNION ALL SELECT c.id, down.depth + 1 FROM containers c JOIN down ON c.parent_id = down.id WHERE down.depth < $2 AND NOT c.archived)
         SELECT id FROM down",
    )
    .bind(root)
    .bind(MAX_DEPTH)
    .fetch_all(&st.db)
    .await?)
}

/// Whether the caller owns the container or one above it.
async fn owns(st: &AppState, ctx: &Ctx, container: Uuid) -> AppResult<bool> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE up(id, parent_id, owner_id, depth) AS (SELECT id, parent_id, owner_id, 0 FROM containers WHERE id = $1
            UNION ALL SELECT c.id, c.parent_id, c.owner_id, up.depth + 1 FROM containers c JOIN up ON c.id = up.parent_id WHERE up.depth < $3)
         SELECT EXISTS (SELECT 1 FROM up WHERE owner_id = $2)",
    )
    .bind(container)
    .bind(ctx.user_id)
    .bind(MAX_DEPTH)
    .fetch_one(&st.db)
    .await?)
}

/// May change the container's own details: an owner (of it or above), or a planning manager.
async fn can_edit(st: &AppState, ctx: &Ctx, container: Uuid) -> AppResult<bool> {
    Ok(ctx.has("planning.manage") || (ctx.can_write() && ctx.has("workspace.access") && owns(st, ctx, container).await?))
}

/// Changing who owns a container, or where it hangs, changes who can read the projects in
/// it. That needs `planning.manage` *and* the caller must already be able to see every
/// project underneath, so the change cannot be used to obtain access.
async fn require_structural(st: &St, ctx: &Ctx, container: Uuid) -> AppResult<()> {
    authz::require_global(st, ctx, "planning.manage")?;
    let ids = subtree(st, container).await?;
    let inside: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM projects WHERE container_id = ANY($1) AND NOT archived").bind(&ids).fetch_all(&st.db).await?;
    let visible = authz::visible_project_ids(st, ctx).await?;
    if inside.iter().any(|p| !visible.contains(p)) {
        let _ = authz::deny(st, ctx, "planning.manage", &format!("container:{container}"));
        return Err(AppError::Forbidden("this contains projects you cannot see; someone with access to all of them must make this change".into()));
    }
    Ok(())
}

const CONTAINER_JSON: &str = "jsonb_build_object('id', c.id, 'name', c.name, 'description', c.description, 'parent_id', c.parent_id, 'status', c.status,
    'start_date', c.start_date, 'target_date', c.target_date, 'archived', c.archived, 'owner', person_json(o),
    'level', jsonb_build_object('id', l.id, 'name', l.name, 'plural', l.plural, 'position', l.position, 'color', l.color))";
const CONTAINER_FROM: &str = "FROM containers c JOIN hierarchy_levels l ON l.id = c.level_id LEFT JOIN users o ON o.id = c.owner_id";

/// Every program and initiative, with the projects in them that the caller may see.
async fn list_containers(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    let visible = authz::visible_project_ids(&st, &ctx).await?;
    let containers: Value = sqlx::query_scalar(&format!(
        "SELECT coalesce(jsonb_agg({CONTAINER_JSON} || jsonb_build_object(
            'projects', (SELECT coalesce(jsonb_agg(jsonb_build_object('key', p.key, 'name', p.name, 'kind', p.kind,
                            'open', (SELECT count(*) FROM items i WHERE i.project_id = p.id AND NOT i.archived AND i.status_category <> 'done'),
                            'done', (SELECT count(*) FROM items i WHERE i.project_id = p.id AND NOT i.archived AND i.status_category = 'done')) ORDER BY p.name), '[]'::jsonb)
                         FROM projects p WHERE p.container_id = c.id AND p.id = ANY($1)),
            'mine', c.owner_id = $2) ORDER BY l.position, c.name), '[]'::jsonb) {CONTAINER_FROM} WHERE NOT c.archived"
    ))
    .bind(&visible)
    .bind(ctx.user_id)
    .fetch_one(&st.db)
    .await?;
    let loose: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('key', key, 'name', name, 'kind', kind) ORDER BY name), '[]'::jsonb) FROM projects WHERE container_id IS NULL AND id = ANY($1)").bind(&visible).fetch_one(&st.db).await?;
    Ok(Json(json!({"containers": containers, "unplaced_projects": loose, "levels": levels_json(&st, "container").await?, "can_manage": ctx.has("planning.manage")})))
}

async fn container_detail(st: &St, ctx: &Ctx, id: Uuid) -> AppResult<Value> {
    let mut c: Value = sqlx::query_scalar(&format!("SELECT {CONTAINER_JSON} {CONTAINER_FROM} WHERE c.id = $1")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let visible = authz::visible_project_ids(st, ctx).await?;
    c["ancestors"] = sqlx::query_scalar(
        "WITH RECURSIVE up(id, parent_id, name, level_id, depth) AS (SELECT p.id, p.parent_id, p.name, p.level_id, 1 FROM containers c JOIN containers p ON p.id = c.parent_id WHERE c.id = $1
            UNION ALL SELECT p.id, p.parent_id, p.name, p.level_id, up.depth + 1 FROM containers p JOIN up ON p.id = up.parent_id WHERE up.depth < $2)
         SELECT coalesce(jsonb_agg(jsonb_build_object('id', up.id, 'name', up.name, 'level', l.name) ORDER BY up.depth DESC), '[]'::jsonb) FROM up JOIN hierarchy_levels l ON l.id = up.level_id",
    )
    .bind(id)
    .bind(MAX_DEPTH)
    .fetch_one(&st.db)
    .await?;
    c["children"] = sqlx::query_scalar(&format!("SELECT coalesce(jsonb_agg({CONTAINER_JSON} ORDER BY l.position, c.name), '[]'::jsonb) {CONTAINER_FROM} WHERE c.parent_id = $1 AND NOT c.archived")).bind(id).fetch_one(&st.db).await?;
    c["projects"] = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('key', key, 'name', name, 'kind', kind, 'description', description) ORDER BY name), '[]'::jsonb) FROM projects WHERE container_id = $1 AND id = ANY($2)").bind(id).bind(&visible).fetch_one(&st.db).await?;
    // Projects the caller may not see are not counted or hinted at: a program looks exactly as
    // large as the part of it the caller has access to.
    c["can_edit"] = json!(can_edit(st, ctx, id).await?);
    c["can_manage"] = json!(ctx.has("planning.manage"));
    Ok(c)
}

async fn get_container(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    Ok(Json(container_detail(&st, &ctx, id).await?))
}

fn opt_date(v: &Value, name: &str) -> AppResult<Option<NaiveDate>> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map(Some).map_err(|_| bad(format!("{name} must be a date like 2026-03-31"))),
        _ => Err(bad(format!("{name} must be a date like 2026-03-31"))),
    }
}

fn opt_id(v: &Value, name: &str) -> AppResult<Option<Uuid>> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => s.parse().map(Some).map_err(|_| bad(format!("{name} must be an id"))),
        _ => Err(bad(format!("{name} must be an id or null"))),
    }
}

/// A parent must be on a higher level than its child, and never beneath it.
async fn check_parent(st: &AppState, container: Option<Uuid>, level: Uuid, parent: Uuid) -> AppResult<()> {
    let row = sqlx::query("SELECT pl.position AS parent_pos, pl.name AS parent_level, (SELECT position FROM hierarchy_levels WHERE id = $2) AS pos FROM containers p JOIN hierarchy_levels pl ON pl.id = p.level_id WHERE p.id = $1 AND NOT p.archived").bind(parent).bind(level).fetch_optional(&st.db).await?.ok_or_else(|| bad("the parent does not exist"))?;
    if row.get::<i32, _>("pos") <= row.get::<i32, _>("parent_pos") {
        return Err(bad(format!("the parent is a {}, which is not above this level", row.get::<String, _>("parent_level"))));
    }
    if let Some(c) = container {
        if subtree(st, c).await?.contains(&parent) {
            return Err(bad("that would put this beneath itself"));
        }
    }
    Ok(())
}

async fn create_container(State(st): State<St>, ctx: Ctx, Json(b): Json<Map<String, Value>>) -> AppResult<(StatusCode, Json<Value>)> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    let level: Uuid = b.get("level_id").and_then(Value::as_str).and_then(|s| s.parse().ok()).ok_or_else(|| bad("level_id is required"))?;
    let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM hierarchy_levels WHERE id = $1 AND kind = 'container')").bind(level).fetch_one(&st.db).await?;
    if !ok {
        return Err(bad("level_id must be one of the levels above projects"));
    }
    let parent = b.get("parent_id").map(|v| opt_id(v, "parent_id")).transpose()?.flatten();
    // Planning managers create anywhere; an owner may add to what they own.
    match parent {
        Some(p) if !ctx.has("planning.manage") => {
            if !can_edit(&st, &ctx, p).await? {
                return Err(authz::deny(&st, &ctx, "planning.manage", &format!("container:{p}")));
            }
        }
        _ => authz::require_global(&st, &ctx, "planning.manage")?,
    }
    if let Some(p) = parent {
        check_parent(&st, None, level, p).await?;
    }
    let name = b.get("name").and_then(Value::as_str).map(str::trim).filter(|n| !n.is_empty() && n.chars().count() <= 200).ok_or_else(|| bad("name must be 1 to 200 characters"))?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO containers (level_id, parent_id, name, owner_id, created_by) VALUES ($1,$2,$3,$4,$4) RETURNING id").bind(level).bind(parent).bind(name).bind(ctx.user_id).fetch_one(&st.db).await?;
    audit::log(&st, &ctx, "container.create", "container", &id.to_string(), json!({"name": name, "level_id": level, "parent_id": parent})).await;
    // The remaining fields go through the same checks as an update.
    let rest: Map<String, Value> = b.into_iter().filter(|(k, _)| !["level_id", "parent_id", "name"].contains(&k.as_str())).collect();
    if !rest.is_empty() {
        if let Err(e) = apply_container(&st, &ctx, id, &rest).await {
            sqlx::query("DELETE FROM containers WHERE id = $1").bind(id).execute(&st.db).await?;
            return Err(e);
        }
    }
    changed(&st).await;
    Ok((StatusCode::CREATED, Json(container_detail(&st, &ctx, id).await?)))
}

async fn apply_container(st: &St, ctx: &Ctx, id: Uuid, b: &Map<String, Value>) -> AppResult<()> {
    let cur = sqlx::query("SELECT level_id, owner_id, parent_id, start_date, target_date FROM containers WHERE id = $1").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    if !can_edit(st, ctx, id).await? {
        return Err(authz::deny(st, ctx, "planning.manage", &format!("container:{id}")));
    }
    let (mut start, mut target): (Option<NaiveDate>, Option<NaiveDate>) = (cur.get("start_date"), cur.get("target_date"));
    for (k, v) in b {
        match k.as_str() {
            "name" => {
                let n = v.as_str().map(str::trim).filter(|n| !n.is_empty() && n.chars().count() <= 200).ok_or_else(|| bad("name must be 1 to 200 characters"))?;
                sqlx::query("UPDATE containers SET name = $2 WHERE id = $1").bind(id).bind(n).execute(&st.db).await?;
            }
            "description" => {
                let d = v.as_str().filter(|d| d.chars().count() <= 20_000).ok_or_else(|| bad("description must be at most 20000 characters"))?;
                sqlx::query("UPDATE containers SET description = $2 WHERE id = $1").bind(id).bind(d).execute(&st.db).await?;
            }
            "status" => {
                let s = v.as_str().filter(|s| ["planned", "active", "on_hold", "done"].contains(s)).ok_or_else(|| bad("status must be planned, active, on_hold or done"))?;
                sqlx::query("UPDATE containers SET status = $2 WHERE id = $1").bind(id).bind(s).execute(&st.db).await?;
            }
            "start_date" => start = opt_date(v, "start_date")?,
            "target_date" => target = opt_date(v, "target_date")?,
            "owner_id" => {
                let owner = opt_id(v, "owner_id")?;
                if owner != cur.get::<Option<Uuid>, _>("owner_id") {
                    require_structural(st, ctx, id).await?;
                    if let Some(o) = owner {
                        let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND active AND kind = 'human')").bind(o).fetch_one(&st.db).await?;
                        if !ok {
                            return Err(bad("the owner must be an active person"));
                        }
                    }
                    sqlx::query("UPDATE containers SET owner_id = $2 WHERE id = $1").bind(id).bind(owner).execute(&st.db).await?;
                    audit::log(st, ctx, "container.owner_change", "container", &id.to_string(), json!({"from": cur.get::<Option<Uuid>, _>("owner_id"), "to": owner})).await;
                }
            }
            "parent_id" => {
                let parent = opt_id(v, "parent_id")?;
                if parent != cur.get::<Option<Uuid>, _>("parent_id") {
                    require_structural(st, ctx, id).await?;
                    if let Some(p) = parent {
                        check_parent(st, Some(id), cur.get("level_id"), p).await?;
                    }
                    sqlx::query("UPDATE containers SET parent_id = $2 WHERE id = $1").bind(id).bind(parent).execute(&st.db).await?;
                }
            }
            "archived" => {
                authz::require_global(st, ctx, "planning.manage")?;
                let a = v.as_bool().ok_or_else(|| bad("archived must be true or false"))?;
                if a {
                    // What was inside moves up one level rather than disappearing with it.
                    let parent: Option<Uuid> = cur.get("parent_id");
                    sqlx::query("UPDATE containers SET parent_id = $2 WHERE parent_id = $1").bind(id).bind(parent).execute(&st.db).await?;
                    sqlx::query("UPDATE projects SET container_id = $2 WHERE container_id = $1").bind(id).bind(parent).execute(&st.db).await?;
                }
                sqlx::query("UPDATE containers SET archived = $2 WHERE id = $1").bind(id).bind(a).execute(&st.db).await?;
            }
            other => return Err(bad(format!("unknown field {other:?}"))),
        }
    }
    if let (Some(s), Some(t)) = (start, target) {
        if t < s {
            return Err(bad("the target date cannot be before the start date"));
        }
    }
    sqlx::query("UPDATE containers SET start_date = $2, target_date = $3, updated_at = now() WHERE id = $1").bind(id).bind(start).bind(target).execute(&st.db).await?;
    Ok(())
}

async fn update_container(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Map<String, Value>>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    apply_container(&st, &ctx, id, &b).await?;
    audit::log(&st, &ctx, "container.update", "container", &id.to_string(), json!({"changed": b.keys().collect::<Vec<_>>()})).await;
    changed(&st).await;
    Ok(Json(container_detail(&st, &ctx, id).await?))
}

#[derive(Deserialize)]
struct PlaceBody {
    container_id: Option<Uuid>,
}

/// Put a project in a program or initiative (or take it out). The owners of that container
/// and of those above it will be able to read the project, so this is the project
/// administrator's decision, and also needs a say over the container.
async fn place_project(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<PlaceBody>) -> AppResult<StatusCode> {
    let (project, perms) = projects::load_project(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "project.admin", &format!("project:{}", project.key))?;
    if let Some(c) = b.container_id {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM containers WHERE id = $1 AND NOT archived)").bind(c).fetch_one(&st.db).await?;
        if !ok {
            return Err(bad("that program or initiative does not exist"));
        }
        if !can_edit(&st, &ctx, c).await? {
            return Err(authz::deny(&st, &ctx, "planning.manage", &format!("container:{c}")));
        }
    }
    sqlx::query("UPDATE projects SET container_id = $2, updated_at = now() WHERE id = $1").bind(project.id).bind(b.container_id).execute(&st.db).await?;
    audit::log(&st, &ctx, "project.place", "project", &project.key, json!({"container_id": b.container_id})).await;
    changed(&st).await;
    projects::changed(&st, &project).await;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── reports ─────────────────────────

/// Statistics over a set of projects. Callers pass only projects the reader holds
/// `project.reports` on.
async fn report(st: &AppState, project_ids: &[Uuid]) -> AppResult<Value> {
    let totals: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object(
            'total', count(*), 'todo', count(*) FILTER (WHERE status_category = 'todo'), 'in_progress', count(*) FILTER (WHERE status_category = 'in_progress'),
            'done', count(*) FILTER (WHERE status_category = 'done'),
            'overdue', count(*) FILTER (WHERE status_category <> 'done' AND due_date < current_date),
            'unassigned', count(*) FILTER (WHERE status_category <> 'done' AND assignee_id IS NULL),
            'created_30d', count(*) FILTER (WHERE created_at > now() - interval '30 days'),
            'resolved_30d', count(*) FILTER (WHERE resolved_at > now() - interval '30 days'),
            'estimate_open', coalesce(sum(estimate) FILTER (WHERE status_category <> 'done'), 0),
            'estimate_done', coalesce(sum(estimate) FILTER (WHERE status_category = 'done'), 0),
            'cycle_days', round((avg(extract(epoch FROM resolved_at - created_at) / 86400.0) FILTER (WHERE resolved_at > now() - interval '90 days'))::numeric, 1))
         FROM items WHERE project_id = ANY($1) AND NOT archived",
    )
    .bind(project_ids)
    .fetch_one(&st.db)
    .await?;
    let group = |expr: &str, label: &str, join: &str| {
        format!(
            "SELECT coalesce(jsonb_agg(to_jsonb(g) ORDER BY g.total DESC, g.label), '[]'::jsonb) FROM (
               SELECT {expr} AS key, {label} AS label, count(*) AS total, count(*) FILTER (WHERE i.status_category <> 'done') AS open,
                      count(*) FILTER (WHERE i.status_category = 'done') AS done
               FROM items i {join} WHERE i.project_id = ANY($1) AND NOT i.archived GROUP BY 1, 2) g"
        )
    };
    let by_type: Value = sqlx::query_scalar(&group("i.type_key", "ty.name", "JOIN item_types ty ON ty.key = i.type_key")).bind(project_ids).fetch_one(&st.db).await?;
    let by_priority: Value = sqlx::query_scalar(&group("i.priority::text", "(ARRAY['None','Low','Medium','High','Critical'])[i.priority + 1]", "")).bind(project_ids).fetch_one(&st.db).await?;
    let by_status: Value = sqlx::query_scalar(&group("i.status_category || ':' || i.status", "item_status_name(i.project_id, i.type_key, i.status)", "")).bind(project_ids).fetch_one(&st.db).await?;
    let by_project: Value = sqlx::query_scalar(&group("p.key", "p.name", "JOIN projects p ON p.id = i.project_id")).bind(project_ids).fetch_one(&st.db).await?;
    // The accounts doing the work: what each has open, what they finished lately, what is late.
    let people: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.open DESC, t.done_30d DESC), '[]'::jsonb) FROM (
           SELECT person_json(u) AS person, count(*) FILTER (WHERE i.status_category = 'todo') AS todo, count(*) FILTER (WHERE i.status_category = 'in_progress') AS in_progress,
                  count(*) FILTER (WHERE i.status_category <> 'done') AS open, count(*) FILTER (WHERE i.resolved_at > now() - interval '30 days') AS done_30d,
                  count(*) FILTER (WHERE i.status_category <> 'done' AND i.due_date < current_date) AS overdue,
                  coalesce(sum(i.estimate) FILTER (WHERE i.status_category <> 'done'), 0) AS estimate_open,
                  (SELECT count(*) FROM item_history h JOIN items hi ON hi.id = h.item_id WHERE h.actor_id = u.id AND hi.project_id = ANY($1) AND h.at > now() - interval '30 days') AS actions_30d
           FROM items i JOIN users u ON u.id = i.assignee_id WHERE i.project_id = ANY($1) AND NOT i.archived GROUP BY u.id) t",
    )
    .bind(project_ids)
    .fetch_one(&st.db)
    .await?;
    let contributors: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.actions DESC), '[]'::jsonb) FROM (
           SELECT person_json(u) AS person, count(*) AS actions, max(h.at) AS last_active
           FROM item_history h JOIN items i ON i.id = h.item_id JOIN users u ON u.id = h.actor_id
           WHERE i.project_id = ANY($1) AND h.at > now() - interval '30 days' GROUP BY u.id ORDER BY count(*) DESC LIMIT 100) t",
    )
    .bind(project_ids)
    .fetch_one(&st.db)
    .await?;
    let weekly: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(jsonb_build_object('week', w::date,
            'created', (SELECT count(*) FROM items i WHERE i.project_id = ANY($1) AND NOT i.archived AND i.created_at >= w AND i.created_at < w + interval '7 days'),
            'resolved', (SELECT count(*) FROM items i WHERE i.project_id = ANY($1) AND NOT i.archived AND i.resolved_at >= w AND i.resolved_at < w + interval '7 days')) ORDER BY w), '[]'::jsonb)
         FROM generate_series(date_trunc('week', now()) - interval '11 weeks', date_trunc('week', now()), interval '7 days') AS w",
    )
    .bind(project_ids)
    .fetch_one(&st.db)
    .await?;
    let upcoming: Value = sqlx::query_scalar(&format!(
        "SELECT coalesce(jsonb_agg(t.j ORDER BY t.due), '[]'::jsonb) FROM (SELECT {ITEM_JSON} AS j, i.due_date AS due {ITEM_FROM}
           WHERE i.project_id = ANY($1) AND NOT i.archived AND i.status_category <> 'done' AND i.due_date <= current_date + 14 ORDER BY i.due_date LIMIT 50) t"
    ))
    .bind(project_ids)
    .fetch_one(&st.db)
    .await?;
    Ok(json!({"totals": totals, "by_type": by_type, "by_priority": by_priority, "by_status": by_status, "by_project": by_project, "people": people, "contributors": contributors, "weekly": weekly, "due": upcoming}))
}

async fn project_report(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    let (project, perms) = projects::load_project(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "project.reports", &format!("project:{}", project.key))?;
    let mut r = report(&st, &[project.id]).await?;
    r["scope"] = json!({"kind": "project", "key": project.key, "name": project.name});
    audit::log(&st, &ctx, "report.view", "project", &project.key, json!({})).await;
    Ok(Json(r))
}

/// Projects under a container that the caller may see reports for, and how many of the
/// projects the caller can open are left out. Projects the caller cannot see at all are not
/// counted: their existence is not the caller's to know.
async fn reportable(st: &St, ctx: &Ctx, container: Uuid) -> AppResult<(Vec<Uuid>, usize)> {
    let ids = subtree(st, container).await?;
    let rows = sqlx::query("SELECT id, access, kind, settings FROM projects WHERE container_id = ANY($1) AND NOT archived").bind(&ids).fetch_all(&st.db).await?;
    let mut allowed = vec![];
    let mut left_out = 0;
    for r in &rows {
        let portal_open = r.get::<String, _>("kind") == "service_desk" && r.get::<Value, _>("settings")["portal_open"] == true;
        let perms = authz::project_perms(st, ctx, r.get("id"), &r.get::<String, _>("access"), portal_open).await?;
        if perms.has("project.reports") {
            allowed.push(r.get("id"));
        } else if perms.has("project.view") {
            left_out += 1;
        }
    }
    Ok((allowed, left_out))
}

async fn container_report(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    let c = sqlx::query("SELECT c.name, l.name AS level FROM containers c JOIN hierarchy_levels l ON l.id = c.level_id WHERE c.id = $1").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let (allowed, hidden) = reportable(&st, &ctx, id).await?;
    let mut r = report(&st, &allowed).await?;
    r["scope"] = json!({"kind": "container", "id": id, "name": c.get::<String, _>("name"), "level": c.get::<String, _>("level"), "projects": allowed.len(), "not_included": hidden});
    audit::log(&st, &ctx, "report.view", "container", &id.to_string(), json!({"projects": allowed.len()})).await;
    Ok(Json(r))
}

// ───────────────────────── search by criteria ─────────────────────────

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Criterion {
    pub field: String,
    pub op: String,
    #[serde(default)]
    pub value: Value,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SortKey {
    pub field: String,
    #[serde(default)]
    pub dir: Option<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ItemQuery {
    /// `all` (every criterion must hold, the default) or `any`.
    #[serde(default)]
    pub r#match: Option<String>,
    #[serde(default)]
    pub criteria: Vec<Criterion>,
    #[serde(default)]
    pub sort: Vec<SortKey>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// (field, kind, SQL expression). The expression is fixed text; values are always bound.
const FIELDS: &[(&str, &str, &str)] = &[
    ("key", "text", "(p.key || '-' || i.number)"),
    ("title", "text", "i.title"),
    ("summary", "text", "i.title"),
    ("description", "text", "i.description"),
    ("text", "text", "(i.title || ' ' || i.description)"),
    ("project", "choice", "p.key"),
    ("type", "choice", "i.type_key"),
    ("status", "choice", "i.status"),
    ("status_category", "choice", "i.status_category"),
    ("channel", "choice", "i.channel"),
    ("priority", "number", "i.priority"),
    ("estimate", "number", "i.estimate"),
    ("assignee", "user", "i.assignee_id"),
    ("reporter", "user", "i.reporter_id"),
    ("cycle", "id", "i.cycle_id"),
    ("request_type", "id", "i.request_type_id"),
    ("container", "id", "p.container_id"),
    ("created", "date", "i.created_at::date"),
    ("updated", "date", "i.updated_at::date"),
    ("resolved", "date", "i.resolved_at::date"),
    ("started", "date", "i.start_date"),
    ("due", "date", "i.due_date"),
    ("label", "labels", "i.labels"),
    ("parent", "parent", "i.parent_id"),
    ("watcher", "watcher", ""),
    ("overdue", "flag", "(i.status_category <> 'done' AND i.due_date < current_date)"),
    ("sla_breached", "flag", "(i.sla_resolution_breached OR i.sla_response_breached OR (i.resolved_at IS NULL AND i.sla_resolution_due < now()) OR (i.first_response_at IS NULL AND i.sla_response_due < now()))"),
    ("has_parent", "flag", "(i.parent_id IS NOT NULL)"),
    ("has_attachments", "flag", "EXISTS (SELECT 1 FROM attachments a WHERE a.owner_type = 'item' AND a.owner_id = i.id)"),
];

const SORTS: &[(&str, &str)] = &[
    ("key", "p.key {d}, i.number {d}"),
    ("title", "lower(i.title) {d}"),
    ("summary", "lower(i.title) {d}"),
    ("status", "i.status_category {d}, i.status {d}"),
    ("priority", "i.priority {d}"),
    ("type", "i.type_key {d}"),
    ("project", "p.key {d}"),
    ("assignee", "a.name {d} NULLS LAST"),
    ("reporter", "r.name {d} NULLS LAST"),
    ("created", "i.created_at {d}"),
    ("updated", "i.updated_at {d}"),
    ("resolved", "i.resolved_at {d} NULLS LAST"),
    ("started", "i.start_date {d} NULLS LAST"),
    ("due", "i.due_date {d} NULLS LAST"),
    ("estimate", "i.estimate {d} NULLS LAST"),
    ("rank", "i.rank {d}"),
];

fn strings(v: &Value, field: &str) -> AppResult<Vec<String>> {
    let list: Vec<String> = match v {
        Value::String(s) => vec![s.clone()],
        Value::Number(n) => vec![n.to_string()],
        Value::Array(a) => a.iter().map(|x| x.as_str().map(str::to_string).or_else(|| x.as_i64().map(|n| n.to_string())).ok_or_else(|| bad(format!("{field}: values must be text")))).collect::<AppResult<_>>()?,
        _ => return Err(bad(format!("{field}: a value is required"))),
    };
    if list.is_empty() || list.len() > 200 || list.iter().any(|s| s.len() > 500) {
        return Err(bad(format!("{field}: give between 1 and 200 values")));
    }
    Ok(list)
}

/// A date given literally (`2026-03-31`) or relative to today (`{"days": -7}`).
fn date_value(v: &Value, field: &str) -> AppResult<NaiveDate> {
    match v {
        Value::String(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map_err(|_| bad(format!("{field}: dates look like 2026-03-31"))),
        Value::Object(o) => {
            let days = o.get("days").and_then(Value::as_i64).filter(|d| d.abs() <= 36_500).ok_or_else(|| bad(format!("{field}: a relative date is {{\"days\": -7}}")))?;
            Ok(chrono::Utc::now().date_naive() + chrono::Duration::days(days))
        }
        _ => Err(bad(format!("{field}: a date is required"))),
    }
}

fn number(v: &Value, field: &str) -> AppResult<f64> {
    v.as_f64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).filter(|n| n.is_finite()).ok_or_else(|| bad(format!("{field}: a number is required")))
}

fn like(s: &str) -> String {
    format!("%{}%", crate::storage::like_escape(s))
}

/// Append one criterion as a parenthesized condition. Everything the caller supplied is
/// bound as a parameter; the only text added to the statement comes from the tables above.
async fn push_criterion(qb: &mut QueryBuilder<'_, Postgres>, st: &AppState, ctx: &Ctx, c: &Criterion) -> AppResult<()> {
    let f = c.field.as_str();
    let op = c.op.as_str();
    let wrong_op = || bad(format!("{f}: {op:?} is not an operator for this field"));
    qb.push("(");
    if let Some(custom) = f.strip_prefix("custom.") {
        let kind: String = sqlx::query_scalar("SELECT type FROM fields WHERE key = $1").bind(custom).fetch_optional(&st.db).await?.ok_or_else(|| bad(format!("{custom:?} is not a custom field")))?;
        let key = custom.to_string();
        match (kind.as_str(), op) {
            (_, "is_empty") => {
                qb.push("coalesce(i.custom->>").push_bind(key).push(", '') = ''");
            }
            (_, "is_not_empty") => {
                qb.push("coalesce(i.custom->>").push_bind(key).push(", '') <> ''");
            }
            ("number", "eq" | "ne" | "gt" | "gte" | "lt" | "lte") => {
                let sql_op = match op { "eq" => "=", "ne" => "<>", "gt" => ">", "gte" => ">=", "lt" => "<", _ => "<=" };
                qb.push("jsonb_typeof(i.custom->").push_bind(key.clone()).push(") = 'number' AND (i.custom->>").push_bind(key).push(format!(")::float8 {sql_op} ")).push_bind(number(&c.value, f)?);
            }
            ("date", "on" | "before" | "after" | "on_or_before" | "on_or_after") => {
                let sql_op = match op { "on" => "=", "before" => "<", "after" => ">", "on_or_before" => "<=", _ => ">=" };
                // Stored dates are validated ISO text, which sorts like the dates themselves.
                qb.push("coalesce(i.custom->>").push_bind(key).push(format!(", '') <> '' AND i.custom->>")).push_bind(custom.to_string()).push(format!(" {sql_op} ")).push_bind(date_value(&c.value, f)?.to_string());
            }
            ("checkbox", "is") => {
                let want = c.value.as_bool().ok_or_else(|| bad(format!("{f}: true or false is required")))?;
                qb.push("coalesce((i.custom->>").push_bind(key).push(")::boolean, false) = ").push_bind(want);
            }
            ("multiselect", "in" | "not_in") => {
                if op == "not_in" {
                    qb.push("NOT ");
                }
                qb.push("coalesce(i.custom->").push_bind(key).push(", '[]'::jsonb) ?| ").push_bind(strings(&c.value, f)?);
            }
            ("select" | "user" | "text" | "textarea" | "url", "in" | "eq") => {
                qb.push("i.custom->>").push_bind(key).push(" = ANY(").push_bind(strings(&c.value, f)?).push(")");
            }
            ("select" | "user" | "text" | "textarea" | "url", "not_in" | "ne") => {
                qb.push("NOT (coalesce(i.custom->>").push_bind(key).push(", '') = ANY(").push_bind(strings(&c.value, f)?).push("))");
            }
            ("text" | "textarea" | "url", "contains" | "not_contains") => {
                let s = c.value.as_str().filter(|s| !s.is_empty() && s.len() <= 500).ok_or_else(|| bad(format!("{f}: text is required")))?;
                if op == "not_contains" {
                    qb.push("NOT ");
                }
                qb.push("(coalesce(i.custom->>").push_bind(key).push(", '') ILIKE ").push_bind(like(s)).push(")");
            }
            _ => return Err(wrong_op()),
        }
        qb.push(")");
        return Ok(());
    }
    let (_, kind, expr) = FIELDS.iter().find(|x| x.0 == f).ok_or_else(|| bad(format!("{f:?} is not a field that can be searched; see GET /api/v1/items/query/fields")))?;
    match (*kind, op) {
        ("text", "contains" | "not_contains" | "starts_with" | "eq" | "ne") => {
            let s = c.value.as_str().filter(|s| !s.is_empty() && s.len() <= 500).ok_or_else(|| bad(format!("{f}: text is required")))?;
            let esc = crate::storage::like_escape(s);
            let pattern = match op { "contains" | "not_contains" => format!("%{esc}%"), "starts_with" => format!("{esc}%"), _ => esc };
            if matches!(op, "not_contains" | "ne") {
                qb.push("NOT ");
            }
            qb.push(format!("({expr} ILIKE ")).push_bind(pattern).push(")");
        }
        ("choice", "in" | "eq") => {
            let mut vals = strings(&c.value, f)?;
            if f == "project" {
                vals.iter_mut().for_each(|v| *v = v.to_uppercase());
            }
            qb.push(format!("{expr} = ANY(")).push_bind(vals).push(")");
        }
        ("choice", "not_in" | "ne") => {
            let mut vals = strings(&c.value, f)?;
            if f == "project" {
                vals.iter_mut().for_each(|v| *v = v.to_uppercase());
            }
            qb.push(format!("NOT ({expr} = ANY(")).push_bind(vals).push("))");
        }
        ("number", "eq" | "ne" | "gt" | "gte" | "lt" | "lte") => {
            let sql_op = match op { "eq" => "=", "ne" => "<>", "gt" => ">", "gte" => ">=", "lt" => "<", _ => "<=" };
            qb.push(format!("{expr} {sql_op} ")).push_bind(number(&c.value, f)?);
        }
        ("number", "in") => {
            let vals: Vec<f64> = match &c.value {
                Value::Array(a) => a.iter().map(|x| number(x, f)).collect::<AppResult<_>>()?,
                v => vec![number(v, f)?],
            };
            qb.push(format!("{expr}::float8 = ANY(")).push_bind(vals).push(")");
        }
        ("number" | "user" | "id" | "date" | "parent", "is_empty") => {
            qb.push(format!("{expr} IS NULL"));
        }
        ("number" | "user" | "id" | "date" | "parent", "is_not_empty") => {
            qb.push(format!("{expr} IS NOT NULL"));
        }
        ("user", "in" | "eq" | "not_in" | "ne") => {
            // `me` is the caller; `none` matches work that has nobody in the field.
            let raw = strings(&c.value, f)?;
            let none = raw.iter().any(|v| v == "none");
            let ids: Vec<Uuid> = raw.iter().filter(|v| *v != "none").map(|v| if v == "me" { Ok(ctx.user_id) } else { v.parse().map_err(|_| bad(format!("{f}: values are me, none or account ids"))) }).collect::<AppResult<_>>()?;
            if matches!(op, "not_in" | "ne") {
                qb.push("NOT ");
            }
            qb.push(format!("(coalesce({expr} = ANY(")).push_bind(ids).push("), false)");
            if none {
                qb.push(format!(" OR {expr} IS NULL"));
            }
            qb.push(")");
        }
        ("id", "in" | "eq" | "not_in" | "ne") => {
            let ids: Vec<Uuid> = strings(&c.value, f)?.iter().map(|v| v.parse().map_err(|_| bad(format!("{f}: values are ids")))).collect::<AppResult<_>>()?;
            if matches!(op, "not_in" | "ne") {
                qb.push("NOT ");
            }
            qb.push(format!("coalesce({expr} = ANY(")).push_bind(ids).push("), false)");
        }
        ("date", "on" | "before" | "after" | "on_or_before" | "on_or_after") => {
            let sql_op = match op { "on" => "=", "before" => "<", "after" => ">", "on_or_before" => "<=", _ => ">=" };
            qb.push(format!("{expr} {sql_op} ")).push_bind(date_value(&c.value, f)?);
        }
        ("date", "between") => {
            let pair = c.value.as_array().filter(|a| a.len() == 2).ok_or_else(|| bad(format!("{f}: between takes two dates")))?;
            qb.push(format!("{expr} BETWEEN ")).push_bind(date_value(&pair[0], f)?).push(" AND ").push_bind(date_value(&pair[1], f)?);
        }
        ("labels", "in" | "eq") => {
            qb.push("i.labels && ").push_bind(strings(&c.value, f)?.into_iter().map(|l| l.trim().to_lowercase()).collect::<Vec<_>>());
        }
        ("labels", "all") => {
            qb.push("i.labels @> ").push_bind(strings(&c.value, f)?.into_iter().map(|l| l.trim().to_lowercase()).collect::<Vec<_>>());
        }
        ("labels", "not_in" | "ne") => {
            qb.push("NOT (i.labels && ").push_bind(strings(&c.value, f)?.into_iter().map(|l| l.trim().to_lowercase()).collect::<Vec<_>>()).push(")");
        }
        ("labels", "is_empty") => {
            qb.push("cardinality(i.labels) = 0");
        }
        ("labels", "is_not_empty") => {
            qb.push("cardinality(i.labels) > 0");
        }
        ("parent", "eq" | "in") => {
            // Parents are named by key. A key the caller cannot see simply matches nothing.
            let keys: Vec<String> = strings(&c.value, f)?.iter().map(|k| k.to_uppercase()).collect();
            qb.push("i.parent_id IN (SELECT x.id FROM items x JOIN projects xp ON xp.id = x.project_id WHERE xp.key || '-' || x.number = ANY(").push_bind(keys).push("))");
        }
        ("watcher", "in" | "eq") => {
            let ids: Vec<Uuid> = strings(&c.value, f)?.iter().map(|v| if v == "me" { Ok(ctx.user_id) } else { v.parse().map_err(|_| bad(format!("{f}: values are me or account ids"))) }).collect::<AppResult<_>>()?;
            // Who follows what is personal: one may only search by one's own watching.
            if ids.iter().any(|u| *u != ctx.user_id) {
                return Err(bad("watcher: you can search only for work that you follow yourself (use \"me\")"));
            }
            qb.push("EXISTS (SELECT 1 FROM item_watchers w WHERE w.item_id = i.id AND w.user_id = ").push_bind(ctx.user_id).push(")");
        }
        ("flag", "is") => {
            let want = c.value.as_bool().ok_or_else(|| bad(format!("{f}: true or false is required")))?;
            qb.push(format!("coalesce({expr}, false) = ")).push_bind(want);
        }
        _ => return Err(wrong_op()),
    }
    qb.push(")");
    Ok(())
}

fn order_by(sort: &[SortKey]) -> AppResult<String> {
    if sort.len() > 4 {
        return Err(bad("at most four sort keys"));
    }
    let mut parts = vec![];
    for s in sort {
        let template = SORTS.iter().find(|x| x.0 == s.field).ok_or_else(|| bad(format!("cannot sort by {:?}; choose from: {}", s.field, SORTS.iter().map(|x| x.0).collect::<Vec<_>>().join(", "))))?.1;
        let dir = match s.dir.as_deref().unwrap_or("asc") {
            "asc" => "ASC",
            "desc" => "DESC",
            _ => return Err(bad("sort direction is asc or desc")),
        };
        parts.push(template.replace("{d}", dir));
    }
    // A final, unique key keeps pages stable when the chosen keys tie.
    parts.push("i.id".into());
    Ok(parts.join(", "))
}

/// Run a search as `ctx`: only work in projects the caller can see is ever matched.
pub async fn run_query(st: &AppState, ctx: &Ctx, q: &ItemQuery) -> AppResult<Value> {
    if q.criteria.len() > 40 {
        return Err(bad("at most 40 criteria"));
    }
    let any = match q.r#match.as_deref().unwrap_or("all") {
        "all" => false,
        "any" => true,
        _ => return Err(bad("match must be all or any")),
    };
    let order = if q.sort.is_empty() { "i.updated_at DESC, i.id".to_string() } else { order_by(&q.sort)? };
    let visible = authz::visible_project_ids(st, ctx).await?;
    let (limit, offset) = (q.limit.unwrap_or(50).clamp(1, 500), q.offset.unwrap_or(0).max(0));
    let mut out = vec![];
    for count in [false, true] {
        let mut qb = QueryBuilder::<Postgres>::new(if count { format!("SELECT count(*) {ITEM_FROM}") } else { format!("SELECT {ITEM_JSON} {ITEM_FROM}") });
        qb.push(" WHERE NOT i.archived AND i.project_id = ANY(").push_bind(visible.clone()).push(")");
        if !q.criteria.is_empty() {
            qb.push(" AND (");
            for (n, c) in q.criteria.iter().enumerate() {
                if n > 0 {
                    qb.push(if any { " OR " } else { " AND " });
                }
                push_criterion(&mut qb, st, ctx, c).await?;
            }
            qb.push(")");
        }
        if count {
            out.push(json!(qb.build_query_scalar::<i64>().fetch_one(&st.db).await?));
        } else {
            qb.push(format!(" ORDER BY {order} LIMIT ")).push_bind(limit).push(" OFFSET ").push_bind(offset);
            out.push(json!(qb.build_query_scalar::<Value>().fetch_all(&st.db).await?));
        }
    }
    Ok(json!({"items": out[0], "total": out[1], "limit": limit, "offset": offset}))
}

async fn h_query(State(st): State<St>, ctx: Ctx, Json(q): Json<ItemQuery>) -> AppResult<Json<Value>> {
    Ok(Json(run_query(&st, &ctx, &q).await?))
}

/// What can be searched on, for building a search form: fields, their operators, and the
/// values that exist in the projects the caller can see.
async fn query_fields(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    let visible = authz::visible_project_ids(&st, &ctx).await?;
    let ops = |kind: &str| -> Vec<&'static str> {
        match kind {
            "text" => vec!["contains", "not_contains", "starts_with", "eq", "ne"],
            "choice" => vec!["in", "not_in"],
            "number" => vec!["eq", "ne", "gt", "gte", "lt", "lte", "is_empty", "is_not_empty"],
            "user" | "id" => vec!["in", "not_in", "is_empty", "is_not_empty"],
            "date" => vec!["on", "before", "after", "on_or_before", "on_or_after", "between", "is_empty", "is_not_empty"],
            "labels" => vec!["in", "all", "not_in", "is_empty", "is_not_empty"],
            "parent" => vec!["eq", "is_empty", "is_not_empty"],
            "watcher" => vec!["in"],
            _ => vec!["is"],
        }
    };
    let labels: [(&str, &str); 29] = [
        ("key", "Key"), ("title", "Name"), ("summary", "Summary"), ("description", "Description"), ("text", "Name or description"), ("project", "Project"), ("type", "Type"),
        ("status", "Status"), ("status_category", "Status category"), ("channel", "Created through"), ("priority", "Priority"), ("estimate", "Estimate"), ("assignee", "Assignee"),
        ("reporter", "Reporter"), ("cycle", "Cycle"), ("request_type", "Request type"), ("container", "Program or initiative"), ("created", "Created"), ("updated", "Updated"),
        ("resolved", "Resolved"), ("started", "Start date"), ("due", "Due date"), ("label", "Label"), ("parent", "Parent"), ("watcher", "Followed by"), ("overdue", "Overdue"),
        ("sla_breached", "Missed a service-level target"), ("has_parent", "Has a parent"), ("has_attachments", "Has files"),
    ];
    let mut fields: Vec<Value> = FIELDS.iter().filter(|f| f.0 != "summary").map(|(key, kind, _)| json!({"key": key, "kind": kind, "label": labels.iter().find(|l| l.0 == *key).map(|l| l.1).unwrap_or(key), "ops": ops(kind)})).collect();
    let custom = sqlx::query("SELECT key, name, type, options FROM fields ORDER BY name").fetch_all(&st.db).await?;
    for c in &custom {
        let kind: String = c.get("type");
        let c_ops: Vec<&str> = match kind.as_str() {
            "number" => vec!["eq", "ne", "gt", "gte", "lt", "lte", "is_empty", "is_not_empty"],
            "date" => vec!["on", "before", "after", "on_or_before", "on_or_after", "is_empty", "is_not_empty"],
            "checkbox" => vec!["is"],
            "select" | "multiselect" | "user" => vec!["in", "not_in", "is_empty", "is_not_empty"],
            _ => vec!["contains", "not_contains", "eq", "ne", "is_empty", "is_not_empty"],
        };
        fields.push(json!({"key": format!("custom.{}", c.get::<String, _>("key")), "kind": kind, "label": c.get::<String, _>("name"), "ops": c_ops, "options": c.get::<Value, _>("options"), "custom": true}));
    }
    let values: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object(
            'project', (SELECT coalesce(jsonb_agg(jsonb_build_object('value', key, 'label', key || ' · ' || name) ORDER BY key), '[]'::jsonb) FROM projects WHERE id = ANY($1)),
            'type', (SELECT coalesce(jsonb_agg(DISTINCT jsonb_build_object('value', t.key, 'label', t.name)), '[]'::jsonb) FROM project_types pt JOIN item_types t ON t.key = pt.type_key WHERE pt.project_id = ANY($1)),
            'status', (SELECT coalesce(jsonb_agg(DISTINCT jsonb_build_object('value', s->>'key', 'label', s->>'name', 'category', s->>'category')), '[]'::jsonb)
                       FROM project_types pt JOIN item_types t ON t.key = pt.type_key JOIN workflows w ON w.key = t.workflow_key
                            LEFT JOIN project_workflows pw ON pw.project_id = pt.project_id AND pw.type_key = t.key,
                            jsonb_array_elements(coalesce(pw.definition, w.definition)->'statuses') s WHERE pt.project_id = ANY($1)),
            'label', (SELECT coalesce(jsonb_agg(DISTINCT l ORDER BY l), '[]'::jsonb) FROM items i, unnest(i.labels) l WHERE i.project_id = ANY($1) AND NOT i.archived),
            'cycle', (SELECT coalesce(jsonb_agg(jsonb_build_object('value', c.id, 'label', p.key || ' · ' || c.name) ORDER BY p.key, c.created_at), '[]'::jsonb) FROM cycles c JOIN projects p ON p.id = c.project_id WHERE c.project_id = ANY($1)),
            'container', (SELECT coalesce(jsonb_agg(jsonb_build_object('value', c.id, 'label', l.name || ': ' || c.name) ORDER BY l.position, c.name), '[]'::jsonb) FROM containers c JOIN hierarchy_levels l ON l.id = c.level_id WHERE NOT c.archived),
            'request_type', (SELECT coalesce(jsonb_agg(jsonb_build_object('value', r.id, 'label', p.key || ' · ' || r.name) ORDER BY p.key, r.position), '[]'::jsonb) FROM request_types r JOIN projects p ON p.id = r.project_id WHERE r.project_id = ANY($1)))",
    )
    .bind(&visible)
    .fetch_one(&st.db)
    .await?;
    Ok(Json(json!({
        "fields": fields, "values": values, "sorts": SORTS.iter().filter(|s| s.0 != "summary").map(|s| s.0).collect::<Vec<_>>(),
        "fixed": {"status_category": [{"value": "todo", "label": "To do"}, {"value": "in_progress", "label": "In progress"}, {"value": "done", "label": "Done"}],
                  "priority": [{"value": 0, "label": "None"}, {"value": 1, "label": "Low"}, {"value": 2, "label": "Medium"}, {"value": 3, "label": "High"}, {"value": 4, "label": "Critical"}],
                  "channel": [{"value": "web", "label": "The application"}, {"value": "portal", "label": "The service portal"}, {"value": "api", "label": "The API"}, {"value": "mcp", "label": "An AI agent (MCP)"}, {"value": "slack", "label": "Slack"}, {"value": "import", "label": "An import"}]},
    })))
}

// ───────────────────────── saved searches ─────────────────────────

const SEARCH_JSON: &str = "jsonb_build_object('id', s.id, 'name', s.name, 'query', s.query, 'shared', s.shared, 'alert', s.alert, 'updated_at', s.updated_at, 'owner', person_json(u), 'mine', s.owner_id = $1)";

async fn list_searches(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    Ok(Json(sqlx::query_scalar(&format!("SELECT coalesce(jsonb_agg({SEARCH_JSON} ORDER BY s.name), '[]'::jsonb) FROM saved_searches s JOIN users u ON u.id = s.owner_id WHERE s.owner_id = $1 OR (s.shared AND $2)")).bind(ctx.user_id).bind(ctx.has("workspace.access")).fetch_one(&st.db).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct SearchBody {
    name: Option<String>,
    query: Option<ItemQuery>,
    shared: Option<bool>,
    alert: Option<bool>,
}

fn query_value(q: &ItemQuery) -> Value {
    json!({"match": q.r#match.as_deref().unwrap_or("all"), "criteria": q.criteria.iter().map(|c| json!({"field": c.field, "op": c.op, "value": c.value})).collect::<Vec<_>>(),
           "sort": q.sort.iter().map(|s| json!({"field": s.field, "dir": s.dir.as_deref().unwrap_or("asc")})).collect::<Vec<_>>()})
}

async fn create_search(State(st): State<St>, ctx: Ctx, Json(b): Json<SearchBody>) -> AppResult<(StatusCode, Json<Value>)> {
    if !ctx.can_write() {
        return Err(authz::deny(&st, &ctx, "search.save", "saved_search"));
    }
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty() && n.chars().count() <= 200).ok_or_else(|| bad("name must be 1 to 200 characters"))?;
    let q = b.query.ok_or_else(|| bad("query is required"))?;
    // A search that cannot run is not worth saving.
    run_query(&st, &ctx, &ItemQuery { limit: Some(1), ..q.clone() }).await?;
    let own: i64 = sqlx::query_scalar("SELECT count(*) FROM saved_searches WHERE owner_id = $1").bind(ctx.user_id).fetch_one(&st.db).await?;
    if own >= 200 {
        return Err(bad("you have 200 saved searches; delete some first"));
    }
    if b.shared == Some(true) && !ctx.has("workspace.access") {
        return Err(authz::deny(&st, &ctx, "workspace.access", "saved_search"));
    }
    let id: Uuid = sqlx::query_scalar("INSERT INTO saved_searches (owner_id, name, query, shared, alert) VALUES ($1,$2,$3,$4,$5) RETURNING id").bind(ctx.user_id).bind(name).bind(query_value(&q)).bind(b.shared.unwrap_or(false)).bind(b.alert.unwrap_or(false)).fetch_one(&st.db).await?;
    audit::log(&st, &ctx, "search.save", "saved_search", &id.to_string(), json!({"name": name, "shared": b.shared.unwrap_or(false), "alert": b.alert.unwrap_or(false)})).await;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

async fn update_search(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<SearchBody>) -> AppResult<StatusCode> {
    let owner: Uuid = sqlx::query_scalar("SELECT owner_id FROM saved_searches WHERE id = $1 AND (owner_id = $2 OR (shared AND $3))").bind(id).bind(ctx.user_id).bind(ctx.has("workspace.access")).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    if owner != ctx.user_id || !ctx.can_write() {
        return Err(AppError::Forbidden("only its owner can change a saved search; save a copy of your own instead".into()));
    }
    if let Some(n) = &b.name {
        if n.trim().is_empty() || n.chars().count() > 200 {
            return Err(bad("name must be 1 to 200 characters"));
        }
    }
    let query = match &b.query {
        Some(q) => {
            run_query(&st, &ctx, &ItemQuery { limit: Some(1), ..q.clone() }).await?;
            Some(query_value(q))
        }
        None => None,
    };
    // Changing what is searched for starts the alert afresh, so old matches are not announced as new.
    sqlx::query("UPDATE saved_searches SET name = coalesce($2, name), query = coalesce($3, query), shared = coalesce($4, shared), alert = coalesce($5, alert),
                 alert_checked = CASE WHEN $3 IS NOT NULL OR $5 IS NOT NULL THEN NULL ELSE alert_checked END, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(b.name.as_deref().map(str::trim))
        .bind(&query)
        .bind(b.shared)
        .bind(b.alert)
        .execute(&st.db)
        .await?;
    audit::log(&st, &ctx, "search.update", "saved_search", &id.to_string(), json!({"shared": b.shared, "alert": b.alert})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_search(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let n = sqlx::query("DELETE FROM saved_searches WHERE id = $1 AND owner_id = $2").bind(id).bind(ctx.user_id).execute(&st.db).await?.rows_affected();
    if n == 0 || !ctx.can_write() {
        return Err(AppError::NotFound);
    }
    audit::log(&st, &ctx, "search.delete", "saved_search", &id.to_string(), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

/// Every few minutes on one node: for each saved search with alerts on, run it as its owner
/// and tell them about work that matches now and did not at the last check.
pub async fn saved_search_alerts(st: &AppState) -> anyhow::Result<u32> {
    let due = sqlx::query("SELECT id, owner_id, name, query, alert_seen, alert_checked IS NULL AS first FROM saved_searches WHERE alert AND (alert_checked IS NULL OR alert_checked < now() - interval '5 minutes') ORDER BY alert_checked NULLS FIRST LIMIT 200").fetch_all(&st.db).await?;
    let mut told = 0;
    for row in &due {
        let (id, owner): (Uuid, Uuid) = (row.get("id"), row.get("owner_id"));
        // The search runs with the owner's present permissions, not those of the day it was saved.
        let Some(ctx) = crate::auth::service_ctx(st, owner, &ReqInfo { id: "search-alert".into(), ..Default::default() }).await.map_err(|e| anyhow::anyhow!("{e:?}"))? else { continue };
        let Ok(q) = serde_json::from_value::<ItemQuery>(row.get("query")) else { continue };
        let result = match run_query(st, &ctx, &ItemQuery { limit: Some(500), offset: None, sort: vec![], ..q }).await {
            Ok(r) => r,
            Err(e) => {
                tracing::warn!(target: "alerts", search = %id, error = ?e, "a saved search could not be run for its alert");
                sqlx::query("UPDATE saved_searches SET alert_checked = now() WHERE id = $1").bind(id).execute(&st.db).await?;
                continue;
            }
        };
        let items = result["items"].as_array().cloned().unwrap_or_default();
        let now: Vec<Uuid> = items.iter().filter_map(|i| i["id"].as_str()?.parse().ok()).collect();
        let seen: Vec<Uuid> = row.get("alert_seen");
        let fresh: Vec<&Value> = items.iter().filter(|i| i["id"].as_str().and_then(|s| s.parse::<Uuid>().ok()).is_some_and(|u| !seen.contains(&u))).collect();
        sqlx::query("UPDATE saved_searches SET alert_seen = $2, alert_checked = now() WHERE id = $1").bind(id).bind(&now).execute(&st.db).await?;
        // The first check only records what already matches.
        if row.get::<bool, _>("first") || fresh.is_empty() {
            continue;
        }
        let name: String = row.get("name");
        let lines: Vec<String> = fresh.iter().take(20).map(|i| format!("{} {}", i["key"].as_str().unwrap_or_default(), i["title"].as_str().unwrap_or_default())).collect();
        let n = Notice { category: "search_alert", kind: "search.alert".into(), title: format!("{} new in \"{name}\"", fresh.len()), body: lines.first().cloned().unwrap_or_default(), link: format!("/search?saved={id}"), actor_id: None, excerpt: lines.join("\n"), thread: Some(id.to_string()) };
        notify::deliver(st, &[owner], &n).await;
        told += 1;
    }
    Ok(told)
}

// ───────────────────────── Gantt charts ─────────────────────────

const GANTT_ITEM: &str = "jsonb_build_object('id', i.id, 'key', p.key || '-' || i.number, 'project', p.key, 'title', i.title, 'type', i.type_key, 'type_name', ty.name, 'type_color', ty.color,
    'status_category', i.status_category, 'status_name', item_status_name(i.project_id, i.type_key, i.status), 'priority', i.priority,
    'start_date', i.start_date, 'due_date', i.due_date, 'parent_id', i.parent_id, 'rank', i.rank, 'assignee', person_json(a), 'estimate', i.estimate,
    'created_at', i.created_at, 'resolved_at', i.resolved_at)";

/// The projects a chart's scope covers, limited to those the caller may see.
async fn scope_projects(st: &St, ctx: &Ctx, scope: &Value) -> AppResult<Vec<Uuid>> {
    let visible = authz::visible_project_ids(st, ctx).await?;
    if let Some(key) = scope["project"].as_str() {
        let id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM projects WHERE key = $1").bind(key.to_uppercase()).fetch_optional(&st.db).await?;
        return Ok(id.into_iter().filter(|i| visible.contains(i)).collect());
    }
    if let Some(c) = scope["container"].as_str().and_then(|s| s.parse::<Uuid>().ok()) {
        let ids = subtree(st, c).await?;
        let inside: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM projects WHERE container_id = ANY($1) AND NOT archived").bind(&ids).fetch_all(&st.db).await?;
        return Ok(inside.into_iter().filter(|i| visible.contains(i)).collect());
    }
    Err(bad("a chart shows a project ({\"project\": \"KEY\"}) or a program or initiative ({\"container\": \"<id>\"})"))
}

async fn gantt_items(st: &St, ctx: &Ctx, scope: &Value) -> AppResult<Value> {
    let projects = scope_projects(st, ctx, scope).await?;
    let items: Value = sqlx::query_scalar(&format!(
        "SELECT coalesce(jsonb_agg(t.j ORDER BY t.pk, t.rank, t.number), '[]'::jsonb) FROM (
           SELECT {GANTT_ITEM} AS j, p.key AS pk, i.rank, i.number {ITEM_FROM}
           WHERE i.project_id = ANY($1) AND NOT i.archived ORDER BY p.key, i.rank, i.number LIMIT 2000) t"
    ))
    .bind(&projects)
    .fetch_one(&st.db)
    .await?;
    let cycles: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', c.id, 'name', c.name, 'project', p.key, 'start_date', c.start_date, 'end_date', c.end_date, 'state', c.state) ORDER BY c.start_date), '[]'::jsonb) FROM cycles c JOIN projects p ON p.id = c.project_id WHERE c.project_id = ANY($1) AND c.start_date IS NOT NULL").bind(&projects).fetch_one(&st.db).await?;
    // Whether the caller may drag bars: decided per project, enforced again when the item is saved.
    let mut editable = Map::new();
    for r in sqlx::query("SELECT id, key, access, kind, settings FROM projects WHERE id = ANY($1)").bind(&projects).fetch_all(&st.db).await? {
        let portal_open = r.get::<String, _>("kind") == "service_desk" && r.get::<Value, _>("settings")["portal_open"] == true;
        let perms = authz::project_perms(st, ctx, r.get("id"), &r.get::<String, _>("access"), portal_open).await?;
        editable.insert(r.get("key"), json!(perms.has("item.edit") && ctx.token_allows("item.edit")));
    }
    Ok(json!({"items": items, "cycles": cycles, "editable": editable}))
}

#[derive(Deserialize)]
struct GanttQuery {
    project: Option<String>,
    container: Option<String>,
}

/// Dated work for a project or a container, without a saved chart.
async fn gantt_data(State(st): State<St>, ctx: Ctx, Query(q): Query<GanttQuery>) -> AppResult<Json<Value>> {
    let scope = match (q.project, q.container) {
        (Some(p), _) => json!({"project": p}),
        (None, Some(c)) => json!({"container": c}),
        _ => return Err(bad("give a project or a container")),
    };
    Ok(Json(gantt_items(&st, &ctx, &scope).await?))
}

/// The owner's visual cues on a chart. Each is a marker line at a date, a shaded span of
/// dates, or a labeled band around chosen rows.
fn check_annotations(v: &Value) -> AppResult<()> {
    let list = v.as_array().filter(|a| a.len() <= 200).ok_or_else(|| bad("annotations must be a list of at most 200 entries"))?;
    let color = |a: &Value| a["color"].as_str().is_some_and(|c| COLORS.contains(&c) || (c.len() == 7 && c.starts_with('#') && c[1..].bytes().all(|b| b.is_ascii_hexdigit())));
    let date = |a: &Value, k: &str| a[k].as_str().is_some_and(|d| NaiveDate::parse_from_str(d, "%Y-%m-%d").is_ok());
    let label = |a: &Value| a.get("label").is_none_or(|l| l.as_str().is_some_and(|s| s.chars().count() <= 120));
    for a in list {
        let o = a.as_object().ok_or_else(|| bad("each annotation must be an object"))?;
        let ok = match a["type"].as_str() {
            Some("line") => o.keys().all(|k| ["id", "type", "date", "color", "label", "style"].contains(&k.as_str())) && date(a, "date") && a.get("style").is_none_or(|s| ["solid", "dashed", "dotted"].contains(&s.as_str().unwrap_or_default())),
            Some("range") => o.keys().all(|k| ["id", "type", "start", "end", "color", "label"].contains(&k.as_str())) && date(a, "start") && date(a, "end") && a["start"].as_str() <= a["end"].as_str(),
            Some("group") => o.keys().all(|k| ["id", "type", "items", "color", "label"].contains(&k.as_str())) && a["items"].as_array().is_some_and(|i| !i.is_empty() && i.len() <= 500 && i.iter().all(|k| k.as_str().is_some_and(|s| crate::items::parse_key(s).is_some()))),
            _ => false,
        };
        if !ok || !color(a) || !label(a) || a["id"].as_str().is_none_or(|i| i.is_empty() || i.len() > 40) {
            return Err(bad("an annotation is a line (date), a range (start, end) or a group (items), each with an id, a color and an optional label"));
        }
    }
    Ok(())
}

fn check_settings(v: &Value) -> AppResult<()> {
    let o = v.as_object().ok_or_else(|| bad("settings must be an object"))?;
    for (k, val) in o {
        let ok = match k.as_str() {
            "zoom" => val.as_str().is_some_and(|z| ["day", "week", "month", "quarter"].contains(&z)),
            "group_by" => val.as_str().is_some_and(|g| ["none", "project", "assignee", "type", "parent", "status"].contains(&g)),
            "show_done" | "show_undated" | "show_cycles" | "show_today" => val.is_boolean(),
            "color_by" => val.as_str().is_some_and(|c| ["status", "type", "priority", "assignee"].contains(&c)),
            _ => false,
        };
        if !ok {
            return Err(bad(format!("{k:?} is not a valid chart setting")));
        }
    }
    Ok(())
}

async fn list_gantt(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    Ok(Json(
        sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(jsonb_build_object('id', g.id, 'name', g.name, 'scope', g.scope, 'shared', g.shared, 'updated_at', g.updated_at, 'owner', person_json(u), 'mine', g.owner_id = $1) ORDER BY g.name), '[]'::jsonb)
             FROM gantt_charts g JOIN users u ON u.id = g.owner_id WHERE g.owner_id = $1 OR (g.shared AND $2)",
        )
        .bind(ctx.user_id)
        .bind(ctx.has("workspace.access"))
        .fetch_one(&st.db)
        .await?,
    ))
}

/// A chart the caller may open: their own, or a shared one (for workspace members).
pub async fn load_gantt(st: &AppState, ctx: &Ctx, id: Uuid) -> AppResult<sqlx::postgres::PgRow> {
    sqlx::query("SELECT g.id, g.owner_id, g.name, g.scope, g.annotations, g.settings, g.shared, g.updated_at, person_json(u) AS owner FROM gantt_charts g JOIN users u ON u.id = g.owner_id WHERE g.id = $1 AND (g.owner_id = $2 OR (g.shared AND $3))")
        .bind(id)
        .bind(ctx.user_id)
        .bind(ctx.has("workspace.access"))
        .fetch_optional(&st.db)
        .await?
        .ok_or(AppError::NotFound)
}

async fn get_gantt(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    let g = load_gantt(&st, &ctx, id).await?;
    let scope: Value = g.get("scope");
    let mut out = gantt_items(&st, &ctx, &scope).await?;
    let mine = g.get::<Uuid, _>("owner_id") == ctx.user_id;
    out["chart"] = json!({"id": id, "name": g.get::<String, _>("name"), "scope": scope, "annotations": g.get::<Value, _>("annotations"), "settings": g.get::<Value, _>("settings"),
        "shared": g.get::<bool, _>("shared"), "owner": g.get::<Value, _>("owner"), "mine": mine, "updated_at": g.get::<chrono::DateTime<chrono::Utc>, _>("updated_at")});
    Ok(Json(out))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GanttBody {
    name: Option<String>,
    scope: Option<Value>,
    annotations: Option<Value>,
    settings: Option<Value>,
    shared: Option<bool>,
}

async fn check_gantt(st: &St, ctx: &Ctx, b: &GanttBody) -> AppResult<()> {
    if let Some(n) = &b.name {
        if n.trim().is_empty() || n.chars().count() > 200 {
            return Err(bad("name must be 1 to 200 characters"));
        }
    }
    if let Some(s) = &b.scope {
        if scope_projects(st, ctx, s).await?.is_empty() {
            return Err(bad("the chart's project or container does not exist, or holds nothing you can see"));
        }
    }
    if let Some(a) = &b.annotations {
        check_annotations(a)?;
    }
    if let Some(s) = &b.settings {
        check_settings(s)?;
    }
    if b.shared == Some(true) && !ctx.has("workspace.access") {
        return Err(authz::deny(st, ctx, "workspace.access", "gantt"));
    }
    Ok(())
}

async fn create_gantt(State(st): State<St>, ctx: Ctx, Json(b): Json<GanttBody>) -> AppResult<(StatusCode, Json<Value>)> {
    if !ctx.can_write() || !ctx.has("workspace.access") {
        return Err(authz::deny(&st, &ctx, "workspace.access", "gantt"));
    }
    check_gantt(&st, &ctx, &b).await?;
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("name is required"))?;
    let scope = b.scope.clone().ok_or_else(|| bad("scope is required"))?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO gantt_charts (owner_id, name, scope, annotations, settings, shared) VALUES ($1,$2,$3,$4,$5,$6) RETURNING id").bind(ctx.user_id).bind(name).bind(&scope).bind(b.annotations.clone().unwrap_or(json!([]))).bind(b.settings.clone().unwrap_or(json!({}))).bind(b.shared.unwrap_or(false)).fetch_one(&st.db).await?;
    audit::log(&st, &ctx, "gantt.create", "gantt", &id.to_string(), json!({"name": name, "scope": scope})).await;
    Ok((StatusCode::CREATED, Json(json!({"id": id}))))
}

/// Only the chart's owner changes its name, scope, annotations and settings.
async fn update_gantt(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<GanttBody>) -> AppResult<StatusCode> {
    let g = load_gantt(&st, &ctx, id).await?;
    if g.get::<Uuid, _>("owner_id") != ctx.user_id || !ctx.can_write() {
        return Err(AppError::Forbidden("only the chart's owner can change it; save a copy of your own instead".into()));
    }
    check_gantt(&st, &ctx, &b).await?;
    sqlx::query("UPDATE gantt_charts SET name = coalesce($2, name), scope = coalesce($3, scope), annotations = coalesce($4, annotations), settings = coalesce($5, settings), shared = coalesce($6, shared), updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(b.name.as_deref().map(str::trim))
        .bind(&b.scope)
        .bind(&b.annotations)
        .bind(&b.settings)
        .bind(b.shared)
        .execute(&st.db)
        .await?;
    audit::log(&st, &ctx, "gantt.update", "gantt", &id.to_string(), json!({"shared": b.shared, "annotations": b.annotations.as_ref().and_then(|a| a.as_array().map(Vec::len))})).await;
    let room = format!("gantt:{id}");
    realtime::publish(&st, &room, json!({"t": "event", "room": room, "kind": "gantt.updated", "data": {"id": id, "actor_id": ctx.user_id}})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_gantt(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let n = sqlx::query("DELETE FROM gantt_charts WHERE id = $1 AND owner_id = $2").bind(id).bind(ctx.user_id).execute(&st.db).await?.rows_affected();
    if n == 0 || !ctx.can_write() {
        return Err(AppError::NotFound);
    }
    audit::log(&st, &ctx, "gantt.delete", "gantt", &id.to_string(), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── the hierarchy graph ─────────────────────────

#[derive(Deserialize)]
struct GraphQuery {
    container: Option<Uuid>,
    project: Option<String>,
    /// Leave out finished work (the default).
    #[serde(default)]
    done: bool,
}

/// Nodes and edges from the top level down to single tasks: containers, the projects in them
/// and the work items in those, each with a name, a status and who it is assigned to.
/// Limited to what the caller can see.
async fn hierarchy_graph(State(st): State<St>, ctx: Ctx, Query(q): Query<GraphQuery>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    let visible = authz::visible_project_ids(&st, &ctx).await?;
    let container_ids: Option<Vec<Uuid>> = match q.container {
        Some(c) => Some(subtree(&st, c).await?),
        None => None,
    };
    let project_ids: Vec<Uuid> = match (&q.project, &container_ids) {
        (Some(key), _) => sqlx::query_scalar("SELECT id FROM projects WHERE key = $1 AND id = ANY($2)").bind(key.to_uppercase()).bind(&visible).fetch_all(&st.db).await?,
        (None, Some(ids)) => sqlx::query_scalar("SELECT id FROM projects WHERE container_id = ANY($1) AND id = ANY($2)").bind(ids).bind(&visible).fetch_all(&st.db).await?,
        (None, None) => visible.clone(),
    };
    let mut nodes: Vec<Value> = vec![];
    let mut edges: Vec<Value> = vec![];
    if q.project.is_none() {
        let rows = sqlx::query(&format!("SELECT c.id, c.parent_id, {CONTAINER_JSON} AS j {CONTAINER_FROM} WHERE NOT c.archived AND ($1::uuid[] IS NULL OR c.id = ANY($1))")).bind(&container_ids).fetch_all(&st.db).await?;
        let present: Vec<Uuid> = rows.iter().map(|r| r.get("id")).collect();
        for r in &rows {
            let (id, j): (Uuid, Value) = (r.get("id"), r.get("j"));
            nodes.push(json!({"id": format!("c:{id}"), "kind": "container", "level": j["level"]["name"], "depth": j["level"]["position"], "color": j["level"]["color"], "label": j["name"], "title": j["name"],
                "status": j["status"], "person": j["owner"], "url": format!("/planning/{id}"), "start_date": j["start_date"], "target_date": j["target_date"]}));
            if let Some(p) = r.get::<Option<Uuid>, _>("parent_id").filter(|p| present.contains(p)) {
                edges.push(json!({"from": format!("c:{p}"), "to": format!("c:{id}")}));
            }
        }
    }
    for r in sqlx::query(
        "SELECT p.id, p.key, p.name, p.kind, p.container_id, person_json(u) AS lead,
                (SELECT count(*) FROM items i WHERE i.project_id = p.id AND NOT i.archived AND i.status_category <> 'done') AS open,
                (SELECT count(*) FROM items i WHERE i.project_id = p.id AND NOT i.archived) AS total
         FROM projects p LEFT JOIN users u ON u.id = p.lead_id WHERE p.id = ANY($1)",
    )
    .bind(&project_ids)
    .fetch_all(&st.db)
    .await?
    {
        let (id, key): (Uuid, String) = (r.get("id"), r.get("key"));
        let (open, total): (i64, i64) = (r.get("open"), r.get("total"));
        nodes.push(json!({"id": format!("p:{id}"), "kind": "project", "level": "Project", "label": key, "title": r.get::<String, _>("name"), "person": r.get::<Option<Value>, _>("lead"),
            "status": if total > 0 && open == 0 { "done" } else if total > open { "in_progress" } else { "todo" }, "open": open, "total": total, "url": format!("/projects/{key}"), "project_kind": r.get::<String, _>("kind")}));
        if let Some(c) = r.get::<Option<Uuid>, _>("container_id").filter(|_| q.project.is_none()) {
            if nodes.iter().any(|n| n["id"] == format!("c:{c}")) {
                edges.push(json!({"from": format!("c:{c}"), "to": format!("p:{id}")}));
            }
        }
    }
    let limit = 3000i64;
    let rows = sqlx::query(
        "SELECT i.id, i.project_id, i.parent_id, p.key || '-' || i.number AS key, i.title, i.status_category, item_status_name(i.project_id, i.type_key, i.status) AS status_name,
                ty.name AS type_name, ty.color AS type_color, ty.icon AS type_icon, i.priority, i.due_date, person_json(a) AS assignee
         FROM items i JOIN projects p ON p.id = i.project_id JOIN item_types ty ON ty.key = i.type_key LEFT JOIN users a ON a.id = i.assignee_id
         WHERE i.project_id = ANY($1) AND NOT i.archived AND ($2 OR i.status_category <> 'done') ORDER BY i.updated_at DESC LIMIT $3",
    )
    .bind(&project_ids)
    .bind(q.done)
    .bind(limit + 1)
    .fetch_all(&st.db)
    .await?;
    let truncated = rows.len() as i64 > limit;
    let shown: std::collections::HashSet<Uuid> = rows.iter().take(limit as usize).map(|r| r.get("id")).collect();
    for r in rows.iter().take(limit as usize) {
        let (id, project): (Uuid, Uuid) = (r.get("id"), r.get("project_id"));
        let key: String = r.get("key");
        nodes.push(json!({"id": format!("i:{id}"), "kind": "item", "level": r.get::<String, _>("type_name"), "color": r.get::<String, _>("type_color"), "icon": r.get::<String, _>("type_icon"), "label": key, "title": r.get::<String, _>("title"),
            "status": r.get::<String, _>("status_category"), "status_name": r.get::<String, _>("status_name"), "person": r.get::<Option<Value>, _>("assignee"), "priority": r.get::<i16, _>("priority"),
            "due_date": r.get::<Option<NaiveDate>, _>("due_date"), "url": format!("/i/{key}")}));
        // A child hangs from its parent when the parent is shown, otherwise from the project.
        match r.get::<Option<Uuid>, _>("parent_id").filter(|p| shown.contains(p)) {
            Some(parent) => edges.push(json!({"from": format!("i:{parent}"), "to": format!("i:{id}")})),
            None => edges.push(json!({"from": format!("p:{project}"), "to": format!("i:{id}")})),
        }
    }
    Ok(Json(json!({"nodes": nodes, "edges": edges, "truncated": truncated, "levels": levels_json(&st, "container").await?})))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/hierarchy", "Planning", "The levels above projects and the levels of work item types", get(get_hierarchy))
        .route("PUT", "/api/v1/hierarchy/levels", "Planning", "Define the levels of one kind, from the top", put(put_levels))
        .route("PUT", "/api/v1/hierarchy/item-types/{key}", "Planning", "Put a work item type on a level", put(put_type_level))
        .route("GET", "/api/v1/containers", "Planning", "Programs and initiatives with the projects in them", get(list_containers))
        .route("POST", "/api/v1/containers", "Planning", "Create a program, an initiative or whatever the level is called", post(create_container))
        .route("GET", "/api/v1/containers/{id}", "Planning", "One container with what it holds", get(get_container))
        .route("PATCH", "/api/v1/containers/{id}", "Planning", "Update, move, hand over or archive a container", patch(update_container))
        .route("PUT", "/api/v1/projects/{key}/container", "Planning", "Place a project in a container, or take it out", put(place_project))
        .route("GET", "/api/v1/reports/project/{key}", "Reports", "Statistics and workload for a project", get(project_report))
        .route("GET", "/api/v1/reports/container/{id}", "Reports", "Statistics and workload across everything under a container", get(container_report))
        .route("POST", "/api/v1/items/query", "Search", "Find work items by any combination of fields, including custom ones", post(h_query))
        .route("GET", "/api/v1/items/query/fields", "Search", "The fields, operators and values a search can use", get(query_fields))
        .route("GET", "/api/v1/searches", "Search", "Saved searches: own and shared", get(list_searches))
        .route("POST", "/api/v1/searches", "Search", "Save a search", post(create_search))
        .route("PATCH", "/api/v1/searches/{id}", "Search", "Rename, change, share or set alerts on a saved search", patch(update_search))
        .route("DELETE", "/api/v1/searches/{id}", "Search", "Delete a saved search", delete(delete_search))
        .route("GET", "/api/v1/gantt", "Gantt", "Saved Gantt charts: own and shared", get(list_gantt))
        .route("POST", "/api/v1/gantt", "Gantt", "Save a Gantt chart", post(create_gantt))
        .route("GET", "/api/v1/gantt/data", "Gantt", "Dated work for a project or container (?project= or ?container=)", get(gantt_data))
        .route("GET", "/api/v1/gantt/{id}", "Gantt", "A saved chart with its work items and annotations", get(get_gantt))
        .route("PATCH", "/api/v1/gantt/{id}", "Gantt", "Change a chart's name, scope, annotations, settings or sharing", patch(update_gantt))
        .route("DELETE", "/api/v1/gantt/{id}", "Gantt", "Delete a chart", delete(delete_gantt))
        .route("GET", "/api/v1/graph/hierarchy", "Planning", "The hierarchy as a graph: containers, projects and work items", get(hierarchy_graph))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_keys_come_from_a_fixed_list() {
        let s = |f: &str, d: Option<&str>| SortKey { field: f.into(), dir: d.map(str::to_string) };
        assert_eq!(order_by(&[s("due", None), s("priority", Some("desc"))]).unwrap(), "i.due_date ASC NULLS LAST, i.priority DESC, i.id");
        assert_eq!(order_by(&[s("key", Some("desc"))]).unwrap(), "p.key DESC, i.number DESC, i.id");
        for hostile in ["i.title; DROP TABLE items", "title desc", "1", "", "(SELECT password_hash FROM users LIMIT 1)"] {
            assert!(order_by(&[s(hostile, None)]).is_err(), "{hostile}");
        }
        assert!(order_by(&[s("title", Some("asc; DROP TABLE items"))]).is_err());
        assert!(order_by(&[s("title", None), s("due", None), s("key", None), s("rank", None), s("type", None)]).is_err(), "too many keys");
    }

    #[test]
    fn field_expressions_are_fixed_text_without_placeholders() {
        for (name, kind, expr) in FIELDS {
            assert!(!expr.contains('$') && !expr.contains(';') && !expr.contains("--"), "{name}");
            assert!(["text", "choice", "number", "user", "id", "date", "labels", "parent", "watcher", "flag"].contains(kind), "{name}");
        }
        for (name, template) in SORTS {
            assert!(template.contains("{d}") && !template.contains('$') && !template.contains(';'), "{name}");
        }
    }

    #[test]
    fn dates_are_literal_or_relative() {
        assert_eq!(date_value(&json!("2026-03-31"), "due").unwrap().to_string(), "2026-03-31");
        assert_eq!(date_value(&json!({"days": 0}), "due").unwrap(), chrono::Utc::now().date_naive());
        assert_eq!(date_value(&json!({"days": -7}), "due").unwrap(), chrono::Utc::now().date_naive() - chrono::Duration::days(7));
        for bad in [json!("31/03/2026"), json!("2026-03-31'; --"), json!({"days": "7"}), json!({"days": 9_999_999}), json!(5), json!(null), json!(["2026-03-31"])] {
            assert!(date_value(&bad, "due").is_err(), "{bad}");
        }
    }

    #[test]
    fn values_are_bounded() {
        assert_eq!(strings(&json!("a"), "f").unwrap(), vec!["a"]);
        assert_eq!(strings(&json!(["a", 2]), "f").unwrap(), vec!["a", "2"]);
        assert!(strings(&json!([]), "f").is_err());
        assert!(strings(&json!([{"x": 1}]), "f").is_err());
        assert!(strings(&json!(vec!["x"; 201]), "f").is_err());
        assert!(strings(&json!("x".repeat(501)), "f").is_err());
        assert!(number(&json!("12.5"), "f").is_ok() && number(&json!(3), "f").is_ok());
        assert!(number(&json!("1; DROP"), "f").is_err() && number(&json!("NaN"), "f").is_err() && number(&json!("inf"), "f").is_err());
    }

    #[test]
    fn annotations_are_validated() {
        let ok = json!([
            {"id": "a1", "type": "line", "date": "2026-06-30", "color": "red", "label": "Launch", "style": "dashed"},
            {"id": "a2", "type": "range", "start": "2026-07-01", "end": "2026-07-14", "color": "#aabbcc", "label": "Freeze"},
            {"id": "a3", "type": "group", "items": ["OPS-1", "OPS-2"], "color": "blue", "label": "Phase 1"},
        ]);
        assert!(check_annotations(&ok).is_ok());
        for bad in [
            json!([{"id": "x", "type": "line", "date": "soon", "color": "red"}]),
            json!([{"id": "x", "type": "line", "date": "2026-06-30", "color": "red; background:url(https://evil.test)"}]),
            json!([{"id": "x", "type": "line", "date": "2026-06-30", "color": "#12345"}]),
            json!([{"id": "x", "type": "range", "start": "2026-07-14", "end": "2026-07-01", "color": "red"}]),
            json!([{"id": "x", "type": "group", "items": [], "color": "red"}]),
            json!([{"id": "x", "type": "group", "items": ["not a key"], "color": "red"}]),
            json!([{"id": "x", "type": "line", "date": "2026-06-30", "color": "red", "onclick": "alert(1)"}]),
            json!([{"id": "x", "type": "image", "src": "https://evil.test/x.png", "color": "red"}]),
            json!([{"type": "line", "date": "2026-06-30", "color": "red"}]),
            json!({"not": "a list"}),
        ] {
            assert!(check_annotations(&bad).is_err(), "{bad}");
        }
        assert!(check_settings(&json!({"zoom": "week", "group_by": "assignee", "show_done": false})).is_ok());
        assert!(check_settings(&json!({"zoom": "century"})).is_err());
        assert!(check_settings(&json!({"script": "x"})).is_err());
    }
}
