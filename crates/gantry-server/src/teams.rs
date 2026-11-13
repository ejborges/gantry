//! Teams: how people organize themselves.
//!
//! A team is defined by its *positions* ("Team lead", "Backend engineer", "On-call"). Each
//! position is filled by one or more people or AI agents, from a start date to an optional end
//! date. Teams nest (teams of teams). Assignments are never deleted when someone moves on:
//! they are ended, so every account carries the history of where it has been and for how long.
//!
//! Anyone with `team.create` can start a team and becomes its first lead. A team is edited by
//! its leads, by the leads of any team above it, and by holders of `team.manage`.

use crate::auth::Ctx;
use crate::error::{bad, AppError, AppResult};
use crate::http::Api;
use crate::notify::{self, Notice};
use crate::{audit, authz, realtime, AppState, St};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::{delete, get, patch, post};
use axum::Json;
use chrono::NaiveDate;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::Row;
use uuid::Uuid;

const MAX_DEPTH: i32 = 12;

/// Current and past positions of one account, newest first.
pub async fn positions_of(st: &AppState, user: Uuid) -> AppResult<Value> {
    Ok(sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.ended_on DESC NULLS FIRST, t.started_on DESC), '[]'::jsonb) FROM (
           SELECT a.id AS assignment_id, a.started_on, a.ended_on, p.id AS position_id, p.title, p.is_lead, tm.id AS team_id, tm.name AS team, tm.archived AS team_archived,
                  (coalesce(a.ended_on, current_date) - a.started_on) AS days
           FROM position_assignments a JOIN team_positions p ON p.id = a.position_id JOIN teams tm ON tm.id = p.team_id WHERE a.user_id = $1) t",
    )
    .bind(user)
    .fetch_one(&st.db)
    .await?)
}

/// Whether the caller may edit the team: `team.manage`, or a current lead of it or of a team above it.
async fn can_edit(st: &St, ctx: &Ctx, team: Uuid) -> AppResult<bool> {
    if ctx.has("team.manage") {
        return Ok(true);
    }
    if !ctx.can_write() || !ctx.has("workspace.access") {
        return Ok(false);
    }
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE up(id, parent_id, depth) AS (
            SELECT id, parent_id, 0 FROM teams WHERE id = $1
            UNION ALL SELECT t.id, t.parent_id, up.depth + 1 FROM teams t JOIN up ON t.id = up.parent_id WHERE up.depth < $3)
         SELECT EXISTS (SELECT 1 FROM up JOIN team_positions p ON p.team_id = up.id AND p.is_lead AND NOT p.closed
                        JOIN position_assignments a ON a.position_id = p.id AND a.user_id = $2 AND a.ended_on IS NULL)",
    )
    .bind(team)
    .bind(ctx.user_id)
    .bind(MAX_DEPTH)
    .fetch_one(&st.db)
    .await?)
}

async fn require_edit(st: &St, ctx: &Ctx, team: Uuid) -> AppResult<()> {
    let exists: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM teams WHERE id = $1)").bind(team).fetch_one(&st.db).await?;
    if !exists {
        return Err(AppError::NotFound);
    }
    if can_edit(st, ctx, team).await? {
        Ok(())
    } else {
        let _ = authz::deny(st, ctx, "team.manage", &format!("team:{team}"));
        Err(AppError::Forbidden("only a lead of this team, a lead of a team above it, or an administrator can change it".into()))
    }
}

const TEAM_LIST_SQL: &str = "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.name), '[]'::jsonb) FROM (
    SELECT tm.id, tm.name, tm.description, tm.parent_id, tm.archived,
      (SELECT coalesce(jsonb_agg(DISTINCT person_json(u)), '[]'::jsonb) FROM team_positions p JOIN position_assignments a ON a.position_id = p.id AND a.ended_on IS NULL JOIN users u ON u.id = a.user_id
        WHERE p.team_id = tm.id AND p.is_lead AND NOT p.closed) AS leads,
      (SELECT count(DISTINCT a.user_id) FROM team_positions p JOIN position_assignments a ON a.position_id = p.id AND a.ended_on IS NULL WHERE p.team_id = tm.id AND NOT p.closed) AS members,
      (SELECT count(*) FROM team_positions p WHERE p.team_id = tm.id AND NOT p.closed) AS positions,
      EXISTS (SELECT 1 FROM team_positions p JOIN position_assignments a ON a.position_id = p.id AND a.ended_on IS NULL WHERE p.team_id = tm.id AND a.user_id = $1) AS mine
    FROM teams tm WHERE NOT tm.archived OR $2) t";

#[derive(Deserialize)]
struct ListQuery {
    #[serde(default)]
    archived: bool,
}

async fn list(State(st): State<St>, ctx: Ctx, Query(q): Query<ListQuery>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    let teams: Value = sqlx::query_scalar(TEAM_LIST_SQL).bind(ctx.user_id).bind(q.archived).fetch_one(&st.db).await?;
    Ok(Json(json!({"teams": teams, "can_create": ctx.has("team.create"), "can_manage": ctx.has("team.manage")})))
}

/// Every team with its open positions and who fills them: the data behind the org chart.
async fn org_chart(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    Ok(Json(
        sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.name), '[]'::jsonb) FROM (
               SELECT tm.id, tm.name, tm.description, tm.parent_id,
                 (SELECT coalesce(jsonb_agg(jsonb_build_object('id', p.id, 'title', p.title, 'is_lead', p.is_lead,
                    'holders', (SELECT coalesce(jsonb_agg(person_json(u) ORDER BY a.started_on, u.name), '[]'::jsonb) FROM position_assignments a JOIN users u ON u.id = a.user_id WHERE a.position_id = p.id AND a.ended_on IS NULL))
                    ORDER BY p.is_lead DESC, p.sort, p.title), '[]'::jsonb)
                  FROM team_positions p WHERE p.team_id = tm.id AND NOT p.closed) AS positions
               FROM teams tm WHERE NOT tm.archived) t",
        )
        .fetch_one(&st.db)
        .await?,
    ))
}

async fn detail(st: &St, ctx: &Ctx, id: Uuid) -> AppResult<Value> {
    let mut team: Value = sqlx::query_scalar(
        "SELECT jsonb_build_object('id', tm.id, 'name', tm.name, 'description', tm.description, 'parent_id', tm.parent_id, 'archived', tm.archived, 'created_at', tm.created_at,
            'parent', (SELECT jsonb_build_object('id', p.id, 'name', p.name) FROM teams p WHERE p.id = tm.parent_id),
            'children', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', c.id, 'name', c.name) ORDER BY c.name), '[]'::jsonb) FROM teams c WHERE c.parent_id = tm.id AND NOT c.archived),
            'positions', (SELECT coalesce(jsonb_agg(jsonb_build_object('id', p.id, 'title', p.title, 'description', p.description, 'is_lead', p.is_lead, 'sort', p.sort, 'closed', p.closed,
                'holders', (SELECT coalesce(jsonb_agg(jsonb_build_object('assignment_id', a.id, 'user', person_json(u), 'started_on', a.started_on) ORDER BY a.started_on, u.name), '[]'::jsonb)
                            FROM position_assignments a JOIN users u ON u.id = a.user_id WHERE a.position_id = p.id AND a.ended_on IS NULL),
                'past', (SELECT coalesce(jsonb_agg(jsonb_build_object('assignment_id', a.id, 'user', person_json(u), 'started_on', a.started_on, 'ended_on', a.ended_on) ORDER BY a.ended_on DESC), '[]'::jsonb)
                         FROM position_assignments a JOIN users u ON u.id = a.user_id WHERE a.position_id = p.id AND a.ended_on IS NOT NULL))
                ORDER BY p.closed, p.is_lead DESC, p.sort, p.title), '[]'::jsonb) FROM team_positions p WHERE p.team_id = tm.id))
         FROM teams tm WHERE tm.id = $1",
    )
    .bind(id)
    .fetch_optional(&st.db)
    .await?
    .ok_or(AppError::NotFound)?;
    team["can_edit"] = json!(can_edit(st, ctx, id).await?);
    Ok(team)
}

async fn get_team(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    authz::require_global(&st, &ctx, "workspace.access")?;
    Ok(Json(detail(&st, &ctx, id).await?))
}

fn clean_name(v: Option<&str>, what: &str, max: usize) -> AppResult<String> {
    let n = v.map(str::trim).filter(|n| !n.is_empty() && n.chars().count() <= max && !n.chars().any(crate::accounts::deceptive)).ok_or_else(|| bad(format!("{what} must be 1 to {max} characters")))?;
    Ok(n.to_string())
}

/// Refuse a parent that would put a team under itself, or nest deeper than the limit.
async fn check_parent(st: &St, team: Option<Uuid>, parent: Uuid) -> AppResult<()> {
    let chain: Vec<Uuid> = sqlx::query_scalar(
        "WITH RECURSIVE up(id, parent_id, depth) AS (SELECT id, parent_id, 0 FROM teams WHERE id = $1 AND NOT archived
            UNION ALL SELECT t.id, t.parent_id, up.depth + 1 FROM teams t JOIN up ON t.id = up.parent_id WHERE up.depth < $2)
         SELECT id FROM up",
    )
    .bind(parent)
    .bind(MAX_DEPTH + 1)
    .fetch_all(&st.db)
    .await?;
    if chain.is_empty() {
        return Err(bad("the parent team does not exist"));
    }
    if team.is_some_and(|t| chain.contains(&t)) {
        return Err(bad("a team cannot sit under itself"));
    }
    if chain.len() as i32 >= MAX_DEPTH {
        return Err(bad(format!("teams can be nested at most {MAX_DEPTH} levels deep")));
    }
    Ok(())
}

#[derive(Deserialize)]
struct NewTeam {
    name: String,
    #[serde(default)]
    description: String,
    parent_id: Option<Uuid>,
}

async fn create(State(st): State<St>, ctx: Ctx, Json(b): Json<NewTeam>) -> AppResult<(StatusCode, Json<Value>)> {
    authz::require_global(&st, &ctx, "team.create")?;
    let name = clean_name(Some(&b.name), "name", 200)?;
    if b.description.chars().count() > 5000 {
        return Err(bad("description must be at most 5000 characters"));
    }
    if let Some(p) = b.parent_id {
        check_parent(&st, None, p).await?;
        require_edit(&st, &ctx, p).await?;
    }
    let mut tx = st.db.begin().await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO teams (name, description, parent_id, created_by) VALUES ($1,$2,$3,$4) RETURNING id").bind(&name).bind(&b.description).bind(b.parent_id).bind(ctx.user_id).fetch_one(&mut *tx).await.map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => AppError::Conflict(format!("a team named {name:?} already exists")),
        e => e.into(),
    })?;
    // Whoever starts a team leads it until they hand it over.
    let lead: Uuid = sqlx::query_scalar("INSERT INTO team_positions (team_id, title, is_lead) VALUES ($1, 'Team lead', true) RETURNING id").bind(id).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO position_assignments (position_id, user_id, created_by) VALUES ($1, $2, $2)").bind(lead).bind(ctx.user_id).execute(&mut *tx).await?;
    tx.commit().await?;
    audit::log(&st, &ctx, "team.create", "team", &id.to_string(), json!({"name": name, "parent_id": b.parent_id})).await;
    changed(&st, id).await;
    Ok((StatusCode::CREATED, Json(detail(&st, &ctx, id).await?)))
}

async fn changed(st: &AppState, team: Uuid) {
    realtime::publish(st, "teams", json!({"t": "event", "room": "teams", "kind": "team.updated", "data": {"id": team}})).await;
    let room = format!("team:{team}");
    realtime::publish(st, &room, json!({"t": "event", "room": room, "kind": "team.updated", "data": {"id": team}})).await;
}

async fn update(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Map<String, Value>>) -> AppResult<Json<Value>> {
    require_edit(&st, &ctx, id).await?;
    for (k, v) in &b {
        match k.as_str() {
            "name" => {
                let n = clean_name(v.as_str(), "name", 200)?;
                sqlx::query("UPDATE teams SET name = $2 WHERE id = $1").bind(id).bind(&n).execute(&st.db).await.map_err(|e| match e {
                    sqlx::Error::Database(d) if d.is_unique_violation() => AppError::Conflict(format!("a team named {n:?} already exists")),
                    e => e.into(),
                })?;
            }
            "description" => {
                let d = v.as_str().filter(|d| d.chars().count() <= 5000).ok_or_else(|| bad("description must be at most 5000 characters"))?;
                sqlx::query("UPDATE teams SET description = $2 WHERE id = $1").bind(id).bind(d).execute(&st.db).await?;
            }
            "parent_id" => {
                let parent: Option<Uuid> = match v {
                    Value::Null => None,
                    Value::String(s) => Some(s.parse().map_err(|_| bad("parent_id must be a team id"))?),
                    _ => return Err(bad("parent_id must be a team id or null")),
                };
                if let Some(p) = parent {
                    check_parent(&st, Some(id), p).await?;
                    // Moving a team under another needs a say in the destination as well.
                    require_edit(&st, &ctx, p).await?;
                }
                sqlx::query("UPDATE teams SET parent_id = $2 WHERE id = $1").bind(id).bind(parent).execute(&st.db).await?;
            }
            "archived" => {
                let a = v.as_bool().ok_or_else(|| bad("archived must be true or false"))?;
                if a {
                    // Archiving ends everyone's time in the team today; the history stays.
                    sqlx::query("UPDATE position_assignments a SET ended_on = greatest(current_date, a.started_on) FROM team_positions p WHERE p.id = a.position_id AND p.team_id = $1 AND a.ended_on IS NULL").bind(id).execute(&st.db).await?;
                    sqlx::query("UPDATE teams SET parent_id = $2 WHERE parent_id = $1").bind(id).bind(sqlx::query_scalar::<_, Option<Uuid>>("SELECT parent_id FROM teams WHERE id = $1").bind(id).fetch_one(&st.db).await?).execute(&st.db).await?;
                }
                sqlx::query("UPDATE teams SET archived = $2 WHERE id = $1").bind(id).bind(a).execute(&st.db).await.map_err(|e| match e {
                    sqlx::Error::Database(d) if d.is_unique_violation() => AppError::Conflict("another team now uses this name; rename one of them first".into()),
                    e => e.into(),
                })?;
            }
            other => return Err(bad(format!("unknown field {other:?}"))),
        }
    }
    sqlx::query("UPDATE teams SET updated_at = now() WHERE id = $1").bind(id).execute(&st.db).await?;
    audit::log(&st, &ctx, "team.update", "team", &id.to_string(), json!({"changed": b.keys().collect::<Vec<_>>(), "archived": b.get("archived")})).await;
    changed(&st, id).await;
    Ok(Json(detail(&st, &ctx, id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PositionBody {
    title: Option<String>,
    description: Option<String>,
    is_lead: Option<bool>,
    sort: Option<i32>,
    closed: Option<bool>,
}

async fn open_leads(st: &St, team: Uuid, without_position: Option<Uuid>, without_assignment: Option<Uuid>) -> AppResult<i64> {
    Ok(sqlx::query_scalar(
        "SELECT count(*) FROM team_positions p JOIN position_assignments a ON a.position_id = p.id AND a.ended_on IS NULL
         WHERE p.team_id = $1 AND p.is_lead AND NOT p.closed AND ($2::uuid IS NULL OR p.id <> $2) AND ($3::uuid IS NULL OR a.id <> $3)",
    )
    .bind(team)
    .bind(without_position)
    .bind(without_assignment)
    .fetch_one(&st.db)
    .await?)
}

const LAST_LEAD: &str = "a team needs at least one lead; name another lead first";

async fn add_position(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<PositionBody>) -> AppResult<(StatusCode, Json<Value>)> {
    require_edit(&st, &ctx, id).await?;
    let title = clean_name(b.title.as_deref(), "title", 200)?;
    let description = b.description.unwrap_or_default();
    if description.chars().count() > 2000 {
        return Err(bad("description must be at most 2000 characters"));
    }
    let pid: Uuid = sqlx::query_scalar("INSERT INTO team_positions (team_id, title, description, is_lead, sort) VALUES ($1,$2,$3,$4, coalesce($5, (SELECT coalesce(max(sort), 0) + 1 FROM team_positions WHERE team_id = $1))) RETURNING id")
        .bind(id)
        .bind(&title)
        .bind(&description)
        .bind(b.is_lead.unwrap_or(false))
        .bind(b.sort)
        .fetch_one(&st.db)
        .await?;
    audit::log(&st, &ctx, "team.position_create", "team", &id.to_string(), json!({"position_id": pid, "title": title, "is_lead": b.is_lead.unwrap_or(false)})).await;
    changed(&st, id).await;
    Ok((StatusCode::CREATED, Json(detail(&st, &ctx, id).await?)))
}

async fn position_row(st: &St, team: Uuid, pid: Uuid) -> AppResult<sqlx::postgres::PgRow> {
    sqlx::query("SELECT id, title, is_lead, closed FROM team_positions WHERE id = $1 AND team_id = $2").bind(pid).bind(team).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)
}

async fn update_position(State(st): State<St>, ctx: Ctx, Path((id, pid)): Path<(Uuid, Uuid)>, Json(b): Json<PositionBody>) -> AppResult<Json<Value>> {
    require_edit(&st, &ctx, id).await?;
    let pos = position_row(&st, id, pid).await?;
    let title = b.title.as_deref().map(|t| clean_name(Some(t), "title", 200)).transpose()?;
    if b.description.as_ref().is_some_and(|d| d.chars().count() > 2000) {
        return Err(bad("description must be at most 2000 characters"));
    }
    let loses_lead = pos.get::<bool, _>("is_lead") && (b.is_lead == Some(false) || b.closed == Some(true));
    if loses_lead && open_leads(&st, id, Some(pid), None).await? == 0 && !ctx.has("team.manage") {
        return Err(bad(LAST_LEAD));
    }
    if b.closed == Some(true) {
        sqlx::query("UPDATE position_assignments SET ended_on = greatest(current_date, started_on) WHERE position_id = $1 AND ended_on IS NULL").bind(pid).execute(&st.db).await?;
    }
    sqlx::query("UPDATE team_positions SET title = coalesce($2, title), description = coalesce($3, description), is_lead = coalesce($4, is_lead), sort = coalesce($5, sort), closed = coalesce($6, closed) WHERE id = $1")
        .bind(pid)
        .bind(&title)
        .bind(&b.description)
        .bind(b.is_lead)
        .bind(b.sort)
        .bind(b.closed)
        .execute(&st.db)
        .await?;
    audit::log(&st, &ctx, "team.position_update", "team", &id.to_string(), json!({"position_id": pid, "title": title, "is_lead": b.is_lead, "closed": b.closed})).await;
    changed(&st, id).await;
    Ok(Json(detail(&st, &ctx, id).await?))
}

/// Delete a position nobody ever held. One that has history is closed instead.
async fn delete_position(State(st): State<St>, ctx: Ctx, Path((id, pid)): Path<(Uuid, Uuid)>) -> AppResult<Json<Value>> {
    require_edit(&st, &ctx, id).await?;
    let pos = position_row(&st, id, pid).await?;
    if pos.get::<bool, _>("is_lead") && !pos.get::<bool, _>("closed") && open_leads(&st, id, Some(pid), None).await? == 0 && !ctx.has("team.manage") {
        return Err(bad(LAST_LEAD));
    }
    let used: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM position_assignments WHERE position_id = $1)").bind(pid).fetch_one(&st.db).await?;
    if used {
        sqlx::query("UPDATE position_assignments SET ended_on = greatest(current_date, started_on) WHERE position_id = $1 AND ended_on IS NULL").bind(pid).execute(&st.db).await?;
        sqlx::query("UPDATE team_positions SET closed = true WHERE id = $1").bind(pid).execute(&st.db).await?;
    } else {
        sqlx::query("DELETE FROM team_positions WHERE id = $1").bind(pid).execute(&st.db).await?;
    }
    audit::log(&st, &ctx, "team.position_remove", "team", &id.to_string(), json!({"position_id": pid, "title": pos.get::<String, _>("title"), "kept_as_history": used})).await;
    changed(&st, id).await;
    Ok(Json(detail(&st, &ctx, id).await?))
}

fn date(v: &Value, name: &str) -> AppResult<Option<NaiveDate>> {
    match v {
        Value::Null => Ok(None),
        Value::String(s) => NaiveDate::parse_from_str(s, "%Y-%m-%d").map(Some).map_err(|_| bad(format!("{name} must be a date like 2026-03-31"))),
        _ => Err(bad(format!("{name} must be a date like 2026-03-31"))),
    }
}

async fn tell(st: &AppState, ctx: &Ctx, user: Uuid, team: Uuid, title: String) {
    let name: String = sqlx::query_scalar("SELECT name FROM teams WHERE id = $1").bind(team).fetch_optional(&st.db).await.ok().flatten().unwrap_or_default();
    notify::deliver(st, &[user], &Notice { category: "team", kind: "team.membership".into(), title, body: name, link: format!("/teams/{team}"), actor_id: Some(ctx.user_id), ..Default::default() }).await;
}

#[derive(Deserialize)]
struct AssignBody {
    user_id: Uuid,
    /// When the person started. Defaults to today; may be any past or future date.
    started_on: Option<NaiveDate>,
}

async fn assign(State(st): State<St>, ctx: Ctx, Path((id, pid)): Path<(Uuid, Uuid)>, Json(b): Json<AssignBody>) -> AppResult<(StatusCode, Json<Value>)> {
    require_edit(&st, &ctx, id).await?;
    let pos = position_row(&st, id, pid).await?;
    if pos.get::<bool, _>("closed") {
        return Err(bad("this position is closed; reopen it first"));
    }
    let active: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM users WHERE id = $1 AND active)").bind(b.user_id).fetch_one(&st.db).await?;
    if !active {
        return Err(bad("that account does not exist or is inactive"));
    }
    let aid: Uuid = sqlx::query_scalar("INSERT INTO position_assignments (position_id, user_id, started_on, created_by) VALUES ($1, $2, coalesce($3, current_date), $4) RETURNING id").bind(pid).bind(b.user_id).bind(b.started_on).bind(ctx.user_id).fetch_one(&st.db).await.map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => AppError::Conflict("this account already holds the position".into()),
        e => e.into(),
    })?;
    let title: String = pos.get("title");
    audit::log(&st, &ctx, "team.assign", "team", &id.to_string(), json!({"position": title, "user_id": b.user_id, "started_on": b.started_on, "assignment_id": aid})).await;
    tell(&st, &ctx, b.user_id, id, format!("{} added you as {title}", ctx.name)).await;
    changed(&st, id).await;
    Ok((StatusCode::CREATED, Json(detail(&st, &ctx, id).await?)))
}

async fn assignment_row(st: &St, team: Uuid, aid: Uuid) -> AppResult<sqlx::postgres::PgRow> {
    sqlx::query("SELECT a.id, a.user_id, a.started_on, a.ended_on, p.id AS position_id, p.title, p.is_lead, p.closed FROM position_assignments a JOIN team_positions p ON p.id = a.position_id WHERE a.id = $1 AND p.team_id = $2").bind(aid).bind(team).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)
}

/// Correct the dates of an assignment, or end it (`ended_on`). The start may be set to any
/// date, which is how time served before the team was recorded here is credited.
async fn update_assignment(State(st): State<St>, ctx: Ctx, Path((id, aid)): Path<(Uuid, Uuid)>, Json(b): Json<Map<String, Value>>) -> AppResult<Json<Value>> {
    require_edit(&st, &ctx, id).await?;
    let a = assignment_row(&st, id, aid).await?;
    let (mut start, mut end): (NaiveDate, Option<NaiveDate>) = (a.get("started_on"), a.get("ended_on"));
    for (k, v) in &b {
        match k.as_str() {
            "started_on" => start = date(v, "started_on")?.ok_or_else(|| bad("started_on is required"))?,
            "ended_on" => end = date(v, "ended_on")?,
            other => return Err(bad(format!("unknown field {other:?}"))),
        }
    }
    if end.is_some_and(|e| e < start) {
        return Err(bad("the end date cannot be before the start date"));
    }
    let ending = end.is_some() && a.get::<Option<NaiveDate>, _>("ended_on").is_none();
    if ending && a.get::<bool, _>("is_lead") && !a.get::<bool, _>("closed") && open_leads(&st, id, None, Some(aid)).await? == 0 && !ctx.has("team.manage") {
        return Err(bad(LAST_LEAD));
    }
    sqlx::query("UPDATE position_assignments SET started_on = $2, ended_on = $3 WHERE id = $1").bind(aid).bind(start).bind(end).execute(&st.db).await.map_err(|e| match e {
        sqlx::Error::Database(d) if d.is_unique_violation() => AppError::Conflict("this account already holds the position".into()),
        e => e.into(),
    })?;
    let title: String = a.get("title");
    audit::log(&st, &ctx, "team.assignment_update", "team", &id.to_string(), json!({"position": title, "user_id": a.get::<Uuid, _>("user_id"), "started_on": start, "ended_on": end})).await;
    if ending {
        tell(&st, &ctx, a.get("user_id"), id, format!("{} ended your time as {title}", ctx.name)).await;
    }
    changed(&st, id).await;
    Ok(Json(detail(&st, &ctx, id).await?))
}

/// Remove an assignment that was recorded by mistake. To record that someone left, end it instead.
async fn delete_assignment(State(st): State<St>, ctx: Ctx, Path((id, aid)): Path<(Uuid, Uuid)>) -> AppResult<Json<Value>> {
    require_edit(&st, &ctx, id).await?;
    let a = assignment_row(&st, id, aid).await?;
    if a.get::<Option<NaiveDate>, _>("ended_on").is_none() && a.get::<bool, _>("is_lead") && !a.get::<bool, _>("closed") && open_leads(&st, id, None, Some(aid)).await? == 0 && !ctx.has("team.manage") {
        return Err(bad(LAST_LEAD));
    }
    sqlx::query("DELETE FROM position_assignments WHERE id = $1").bind(aid).execute(&st.db).await?;
    audit::log(&st, &ctx, "team.assignment_delete", "team", &id.to_string(), json!({"position": a.get::<String, _>("title"), "user_id": a.get::<Uuid, _>("user_id"), "started_on": a.get::<NaiveDate, _>("started_on"), "ended_on": a.get::<Option<NaiveDate>, _>("ended_on")})).await;
    changed(&st, id).await;
    Ok(Json(detail(&st, &ctx, id).await?))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/teams", "Teams", "Teams with their leads and sizes", get(list))
        .route("POST", "/api/v1/teams", "Teams", "Create a team; the creator becomes its first lead", post(create))
        .route("GET", "/api/v1/org-chart", "Teams", "Every team with its positions and who fills them", get(org_chart))
        .route("GET", "/api/v1/teams/{id}", "Teams", "A team with its positions, members and their history", get(get_team))
        .route("PATCH", "/api/v1/teams/{id}", "Teams", "Rename, describe, move or archive a team", patch(update))
        .route("POST", "/api/v1/teams/{id}/positions", "Teams", "Add a position to a team", post(add_position))
        .route("PATCH", "/api/v1/teams/{id}/positions/{position}", "Teams", "Change, close or reopen a position", patch(update_position))
        .route("DELETE", "/api/v1/teams/{id}/positions/{position}", "Teams", "Remove a position (closed instead when it has history)", delete(delete_position))
        .route("POST", "/api/v1/teams/{id}/positions/{position}/assignments", "Teams", "Put a person or an AI agent in a position", post(assign))
        .route("PATCH", "/api/v1/teams/{id}/assignments/{assignment}", "Teams", "Set the start date of an assignment, or end it", patch(update_assignment))
        .route("DELETE", "/api/v1/teams/{id}/assignments/{assignment}", "Teams", "Delete an assignment recorded by mistake", delete(delete_assignment))
}
