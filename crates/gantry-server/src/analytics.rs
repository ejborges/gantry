//! Analytics: aggregate figures over work items, saved visualizations and dashboards.
//!
//! A *visualization* is a small script (or an HTML fragment) that draws into a tile using
//! Chart.js, Apache ECharts, Plotly or D3. The server stores the script and never runs it.
//! It runs in the **viewer's** browser, inside a sandboxed frame that
//!
//! * has an opaque origin: no cookies, no storage, no access to the application's page;
//! * may not open any network connection (`connect-src 'none'`), load anything from other
//!   sites, submit forms, open windows or navigate the top page;
//! * reads data only by asking the application's page, which performs a `GET` under
//!   `/api/v1/` **as the viewer** and hands back the JSON. A shared script therefore shows each
//!   person only what that person may see, and cannot change anything.
//!
//! The frame's document is served by [`frame`] with its own Content-Security-Policy.

use crate::auth::Ctx;
use crate::error::{bad, AppError, AppResult};
use crate::http::{Api, Frameable};
use crate::realtime::Access;
use crate::{audit, authz, realtime, AppState, St};
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::Row;
use uuid::Uuid;

pub const LIBRARIES: &[(&str, &str, &[&str])] = &[
    ("chartjs", "Chart.js", &["/vendor/chartjs/chart.umd.min.js"]),
    ("echarts", "Apache ECharts", &["/vendor/echarts/echarts.min.js"]),
    ("plotly", "Plotly", &["/vendor/plotly/plotly.min.js"]),
    ("d3", "D3", &["/vendor/d3/d3.min.js"]),
    ("html", "HTML and script (no library)", &[]),
];

pub struct Template {
    pub key: &'static str,
    pub name: &'static str,
    pub description: &'static str,
    pub library: &'static str,
    pub source: &'static str,
    /// (parameter, label, default as JSON text)
    pub params: &'static [(&'static str, &'static str, &'static str)],
    /// Suggested tile size on a 12-column grid: (columns, rows).
    pub size: (i32, i32),
}

const SCOPE: (&str, &str, &str) = ("project", "Project key (empty for everything you can see)", "\"\"");

pub const TEMPLATES: &[Template] = &[
    Template { key: "kpi-tiles", name: "Key figures", description: "Open, overdue, unassigned and recently resolved work as four numbers.", library: "html", source: include_str!("../viz/kpi-tiles.html"), params: &[SCOPE], size: (6, 2) },
    Template { key: "status-donut", name: "Work by status", description: "To do, in progress and done as a doughnut.", library: "chartjs", source: include_str!("../viz/status-donut.js"), params: &[SCOPE], size: (4, 4) },
    Template { key: "throughput-bars", name: "Created and resolved", description: "Work created and resolved per week, side by side.", library: "chartjs", source: include_str!("../viz/throughput-bars.js"), params: &[SCOPE, ("weeks", "Weeks to show", "12")], size: (6, 4) },
    Template { key: "open-line", name: "Open work over time", description: "How much was unfinished at the end of each week.", library: "chartjs", source: include_str!("../viz/open-line.js"), params: &[SCOPE, ("weeks", "Weeks to show", "16")], size: (6, 4) },
    Template { key: "priority-bars", name: "Open work by priority", description: "Horizontal bars from none to critical.", library: "echarts", source: include_str!("../viz/priority-bars.js"), params: &[SCOPE], size: (4, 4) },
    Template { key: "type-treemap", name: "Work by type", description: "A treemap of work item types by count.", library: "echarts", source: include_str!("../viz/type-treemap.js"), params: &[SCOPE], size: (4, 4) },
    Template { key: "workload-stacked", name: "Workload by assignee", description: "Open work per person or AI agent, split by status, with overdue alongside.", library: "echarts", source: include_str!("../viz/workload-stacked.js"), params: &[SCOPE, ("top", "People to show", "15")], size: (6, 5) },
    Template { key: "activity-calendar", name: "Activity calendar", description: "Work created per day as a calendar heat map.", library: "echarts", source: include_str!("../viz/activity-calendar.js"), params: &[SCOPE, ("days", "Days to show", "180")], size: (12, 3) },
    Template { key: "cycle-scatter", name: "Time to resolve", description: "Days from creation to resolution for each resolved item, with the median and the 85th percentile.", library: "plotly", source: include_str!("../viz/cycle-scatter.js"), params: &[SCOPE, ("days", "Look back this many days", "90")], size: (6, 5) },
    Template { key: "label-bubbles", name: "Labels", description: "Labels as bubbles sized by the open work that carries them.", library: "d3", source: include_str!("../viz/label-bubbles.js"), params: &[SCOPE], size: (4, 4) },
    Template { key: "status-sunburst", name: "Projects, types and status", description: "Nested rings: project, then type, then status.", library: "d3", source: include_str!("../viz/status-sunburst.js"), params: &[SCOPE], size: (4, 5) },
    Template { key: "due-table", name: "Due next", description: "The open work with the nearest due dates, as a table.", library: "html", source: include_str!("../viz/due-table.html"), params: &[SCOPE, ("rows", "Rows to show", "15")], size: (6, 5) },
];

fn template_json(t: &Template, with_source: bool) -> Value {
    let params: Vec<Value> = t.params.iter().map(|(k, label, default)| json!({"key": k, "label": label, "default": serde_json::from_str::<Value>(default).unwrap_or(Value::Null)})).collect();
    let mut v = json!({"key": t.key, "name": t.name, "description": t.description, "library": t.library, "params": params, "size": {"w": t.size.0, "h": t.size.1}});
    if with_source {
        v["source"] = json!(t.source);
    }
    v
}

// ───────────────────────── the sandbox frame ─────────────────────────

/// The document that runs inside the sandboxed frame. It contains no data and no secrets:
/// everything it needs arrives by message from the application's page.
const FRAME_HTML: &str = include_str!("../viz/frame.html");

/// The frame's own policy. `connect-src 'none'` is what keeps a script from sending what it
/// was shown anywhere; `default-src 'none'` covers every other kind of load. Inline and
/// evaluated script are allowed because running a person's script is the frame's whole
/// purpose; what the script can reach is what is restricted.
pub fn frame_csp() -> &'static str {
    "default-src 'none'; script-src 'unsafe-inline' 'unsafe-eval' 'self'; style-src 'unsafe-inline'; img-src data: blob:; font-src data:; media-src data: blob:; connect-src 'none'; form-action 'none'; base-uri 'none'; frame-src 'none'; worker-src 'none'; object-src 'none'; webrtc 'block'; frame-ancestors 'self'"
}

async fn frame() -> Response {
    let mut r = ([(header::CONTENT_TYPE, "text/html; charset=utf-8")], FRAME_HTML).into_response();
    r.extensions_mut().insert(Frameable);
    let h = r.headers_mut();
    h.insert(header::CONTENT_SECURITY_POLICY, HeaderValue::from_static(frame_csp()));
    h.insert(header::CACHE_CONTROL, HeaderValue::from_static("public, max-age=300"));
    h.insert("cross-origin-resource-policy", HeaderValue::from_static("same-origin"));
    r
}

// ───────────────────────── figures ─────────────────────────

#[derive(Debug, Default, Deserialize)]
struct Scope {
    project: Option<String>,
    container: Option<Uuid>,
}

/// The projects a request covers: one project, everything under a container, or everything
/// visible. Always limited to what the caller may see.
async fn scope_ids(st: &St, ctx: &Ctx, s: &Scope) -> AppResult<Vec<Uuid>> {
    let visible = authz::visible_project_ids(st, ctx).await?;
    if let Some(key) = s.project.as_deref().map(str::trim).filter(|k| !k.is_empty()) {
        let id: Option<Uuid> = sqlx::query_scalar("SELECT id FROM projects WHERE key = $1").bind(key.to_uppercase()).fetch_optional(&st.db).await?;
        return Ok(id.into_iter().filter(|i| visible.contains(i)).collect());
    }
    if let Some(c) = s.container {
        let inside: Vec<Uuid> = sqlx::query_scalar(
            "WITH RECURSIVE down(id, depth) AS (SELECT id, 0 FROM containers WHERE id = $1
                UNION ALL SELECT c.id, down.depth + 1 FROM containers c JOIN down ON c.parent_id = down.id WHERE down.depth < 20 AND NOT c.archived)
             SELECT p.id FROM projects p WHERE p.container_id IN (SELECT id FROM down) AND NOT p.archived",
        )
        .bind(c)
        .fetch_all(&st.db)
        .await?;
        return Ok(inside.into_iter().filter(|i| visible.contains(i)).collect());
    }
    Ok(visible)
}

/// (name, key expression, label expression, extra FROM). Fixed text only.
const GROUPS: &[(&str, &str, &str, &str)] = &[
    ("status_category", "i.status_category", "CASE i.status_category WHEN 'todo' THEN 'To do' WHEN 'in_progress' THEN 'In progress' ELSE 'Done' END", ""),
    ("status", "i.status", "item_status_name(i.project_id, i.type_key, i.status)", ""),
    ("type", "i.type_key", "ty.name", ""),
    ("priority", "i.priority::text", "(ARRAY['None','Low','Medium','High','Critical'])[i.priority + 1]", ""),
    ("project", "p.key", "p.name", ""),
    ("assignee", "coalesce(i.assignee_id::text, '')", "coalesce(a.name || CASE WHEN a.kind = 'agent' THEN ' (AI agent)' ELSE '' END, 'Unassigned')", ""),
    ("reporter", "coalesce(i.reporter_id::text, '')", "coalesce(r.name || CASE WHEN r.kind = 'agent' THEN ' (AI agent)' ELSE '' END, 'Nobody')", ""),
    ("label", "lb.l", "lb.l", ", unnest(i.labels) AS lb(l)"),
    ("cycle", "coalesce(i.cycle_id::text, '')", "coalesce((SELECT c.name FROM cycles c WHERE c.id = i.cycle_id), 'No cycle')", ""),
    ("created_week", "to_char(date_trunc('week', i.created_at), 'YYYY-MM-DD')", "to_char(date_trunc('week', i.created_at), 'YYYY-MM-DD')", ""),
    ("created_month", "to_char(i.created_at, 'YYYY-MM')", "to_char(i.created_at, 'YYYY-MM')", ""),
    ("resolved_month", "coalesce(to_char(i.resolved_at, 'YYYY-MM'), '')", "coalesce(to_char(i.resolved_at, 'YYYY-MM'), 'Not resolved')", ""),
    ("due_month", "coalesce(to_char(i.due_date, 'YYYY-MM'), '')", "coalesce(to_char(i.due_date, 'YYYY-MM'), 'No due date')", ""),
];

#[derive(Debug, Deserialize)]
struct ItemsQuery {
    #[serde(flatten)]
    scope: Scope,
    /// One grouping, or up to three separated by commas for nested groups. Default: `status_category`.
    group_by: Option<String>,
    /// `count` (default) or `estimate`.
    metric: Option<String>,
    /// Only unfinished work.
    open: Option<String>,
}

fn truthy(v: &Option<String>) -> bool {
    matches!(v.as_deref(), Some("true" | "1" | "yes"))
}

/// Counts (or summed estimates) of work items by one to three groupings.
async fn items(State(st): State<St>, ctx: Ctx, Query(q): Query<ItemsQuery>) -> AppResult<Json<Value>> {
    let ids = scope_ids(&st, &ctx, &q.scope).await?;
    let names: Vec<&str> = q.group_by.as_deref().unwrap_or("status_category").split(',').map(str::trim).filter(|s| !s.is_empty()).collect();
    if names.is_empty() || names.len() > 3 {
        return Err(bad("group_by takes one to three groupings separated by commas"));
    }
    let mut defs = vec![];
    for n in &names {
        defs.push(GROUPS.iter().find(|g| g.0 == *n).ok_or_else(|| bad(format!("cannot group by {n:?}; choose from: {}", GROUPS.iter().map(|g| g.0).collect::<Vec<_>>().join(", "))))?);
    }
    let metric = match q.metric.as_deref().unwrap_or("count") {
        "count" => "count(*)::float8",
        "estimate" => "coalesce(sum(i.estimate), 0)::float8",
        _ => return Err(bad("metric must be count or estimate")),
    };
    let cols: Vec<String> = defs.iter().enumerate().map(|(n, d)| format!("{} AS k{n}, {} AS l{n}", d.1, d.2)).collect();
    let extra: String = defs.iter().map(|d| d.3).collect();
    let group: Vec<String> = (1..=defs.len() * 2).map(|n| n.to_string()).collect();
    let sql = format!(
        "SELECT {}, {metric} AS value, count(*) FILTER (WHERE i.status_category <> 'done') AS open, count(*) FILTER (WHERE i.status_category = 'done') AS done
         {}{extra} WHERE i.project_id = ANY($1) AND NOT i.archived AND ($2 = false OR i.status_category <> 'done') GROUP BY {} ORDER BY value DESC, 1 LIMIT 500",
        cols.join(", "),
        crate::items::ITEM_FROM,
        group.join(", ")
    );
    let rows = sqlx::query(&sql).bind(&ids).bind(truthy(&q.open)).fetch_all(&st.db).await?;
    let mut total = 0.0;
    let groups: Vec<Value> = rows
        .iter()
        .map(|r| {
            let value: f64 = r.get("value");
            total += value;
            let path: Vec<Value> = (0..defs.len()).map(|n| json!({"key": r.get::<String, _>(format!("k{n}").as_str()), "label": r.get::<String, _>(format!("l{n}").as_str())})).collect();
            json!({"key": path[0]["key"], "label": path[0]["label"], "path": path, "value": value, "open": r.get::<i64, _>("open"), "done": r.get::<i64, _>("done")})
        })
        .collect();
    Ok(Json(json!({"group_by": names, "groups": groups, "total": total})))
}

async fn summary(State(st): State<St>, ctx: Ctx, Query(scope): Query<Scope>) -> AppResult<Json<Value>> {
    let ids = scope_ids(&st, &ctx, &scope).await?;
    Ok(Json(
        sqlx::query_scalar(
            "SELECT jsonb_build_object('total', count(*), 'open', count(*) FILTER (WHERE status_category <> 'done'), 'todo', count(*) FILTER (WHERE status_category = 'todo'),
                'in_progress', count(*) FILTER (WHERE status_category = 'in_progress'), 'done', count(*) FILTER (WHERE status_category = 'done'),
                'overdue', count(*) FILTER (WHERE status_category <> 'done' AND due_date < current_date),
                'unassigned', count(*) FILTER (WHERE status_category <> 'done' AND assignee_id IS NULL),
                'created_30d', count(*) FILTER (WHERE created_at > now() - interval '30 days'),
                'resolved_30d', count(*) FILTER (WHERE resolved_at > now() - interval '30 days'),
                'projects', count(DISTINCT project_id))
             FROM items WHERE project_id = ANY($1) AND NOT archived",
        )
        .bind(&ids)
        .fetch_one(&st.db)
        .await?,
    ))
}

#[derive(Debug, Deserialize)]
struct TimelineQuery {
    #[serde(flatten)]
    scope: Scope,
    interval: Option<String>,
    periods: Option<String>,
}

/// Created and resolved counts per day, week or month, and how much was open at each period's end.
async fn timeline(State(st): State<St>, ctx: Ctx, Query(q): Query<TimelineQuery>) -> AppResult<Json<Value>> {
    let ids = scope_ids(&st, &ctx, &q.scope).await?;
    let (unit, step, max) = match q.interval.as_deref().unwrap_or("week") {
        "day" => ("day", "1 day", 366),
        "week" => ("week", "7 days", 104),
        "month" => ("month", "1 month", 60),
        _ => return Err(bad("interval must be day, week or month")),
    };
    let periods: i32 = q.periods.as_deref().map(|p| p.parse().map_err(|_| bad("periods must be a whole number"))).transpose()?.unwrap_or(12).clamp(1, max);
    let rows: Value = sqlx::query_scalar(&format!(
        "WITH s AS (SELECT generate_series(date_trunc('{unit}', now()) - ($2 - 1) * interval '{step}', date_trunc('{unit}', now()), interval '{step}') AS start),
              it AS (SELECT created_at, resolved_at FROM items WHERE project_id = ANY($1) AND NOT archived)
         SELECT coalesce(jsonb_agg(jsonb_build_object('start', s.start::date,
             'created', (SELECT count(*) FROM it WHERE created_at >= s.start AND created_at < s.start + interval '{step}'),
             'resolved', (SELECT count(*) FROM it WHERE resolved_at >= s.start AND resolved_at < s.start + interval '{step}'),
             'open_at_end', (SELECT count(*) FROM it WHERE created_at < s.start + interval '{step}' AND (resolved_at IS NULL OR resolved_at >= s.start + interval '{step}'))) ORDER BY s.start), '[]'::jsonb) FROM s"
    ))
    .bind(&ids)
    .bind(periods)
    .fetch_one(&st.db)
    .await?;
    Ok(Json(json!({"interval": unit, "periods": rows})))
}

async fn workload(State(st): State<St>, ctx: Ctx, Query(scope): Query<Scope>) -> AppResult<Json<Value>> {
    let ids = scope_ids(&st, &ctx, &scope).await?;
    Ok(Json(json!({"people": sqlx::query_scalar::<_, Value>(
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.open DESC), '[]'::jsonb) FROM (
           SELECT person_json(u) AS person, count(*) FILTER (WHERE i.status_category = 'todo') AS todo, count(*) FILTER (WHERE i.status_category = 'in_progress') AS in_progress,
                  count(*) FILTER (WHERE i.status_category <> 'done') AS open, count(*) FILTER (WHERE i.resolved_at > now() - interval '30 days') AS done_30d,
                  count(*) FILTER (WHERE i.status_category <> 'done' AND i.due_date < current_date) AS overdue,
                  coalesce(sum(i.estimate) FILTER (WHERE i.status_category <> 'done'), 0) AS estimate_open
           FROM items i LEFT JOIN users u ON u.id = i.assignee_id WHERE i.project_id = ANY($1) AND NOT i.archived
           GROUP BY u.id HAVING count(*) FILTER (WHERE i.status_category <> 'done') > 0 OR count(*) FILTER (WHERE i.resolved_at > now() - interval '30 days') > 0) t",
    )
    .bind(&ids)
    .fetch_one(&st.db)
    .await?})))
}

#[derive(Debug, Deserialize)]
struct CycleQuery {
    #[serde(flatten)]
    scope: Scope,
    days: Option<String>,
}

/// The value below which `p` (0..1) of the sorted numbers fall.
pub fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let rank = (p.clamp(0.0, 1.0) * (sorted.len() - 1) as f64).round() as usize;
    sorted[rank]
}

async fn cycle_time(State(st): State<St>, ctx: Ctx, Query(q): Query<CycleQuery>) -> AppResult<Json<Value>> {
    let ids = scope_ids(&st, &ctx, &q.scope).await?;
    let days: i32 = q.days.as_deref().map(|d| d.parse().map_err(|_| bad("days must be a whole number"))).transpose()?.unwrap_or(90).clamp(1, 730);
    let rows = sqlx::query(
        "SELECT p.key || '-' || i.number AS key, i.title, i.type_key, i.resolved_at, (extract(epoch FROM i.resolved_at - i.created_at) / 86400.0)::float8 AS days
         FROM items i JOIN projects p ON p.id = i.project_id
         WHERE i.project_id = ANY($1) AND NOT i.archived AND i.resolved_at > now() - make_interval(days => $2) ORDER BY i.resolved_at LIMIT 2000",
    )
    .bind(&ids)
    .bind(days)
    .fetch_all(&st.db)
    .await?;
    let round = |x: f64| (x * 10.0).round() / 10.0;
    let mut all: Vec<f64> = rows.iter().map(|r| r.get::<f64, _>("days")).collect();
    all.sort_by(|a, b| a.total_cmp(b));
    let avg = if all.is_empty() { 0.0 } else { all.iter().sum::<f64>() / all.len() as f64 };
    let items: Vec<Value> = rows.iter().map(|r| json!({"key": r.get::<String, _>("key"), "title": r.get::<String, _>("title"), "type": r.get::<String, _>("type_key"), "resolved_at": r.get::<chrono::DateTime<chrono::Utc>, _>("resolved_at"), "days": round(r.get("days"))})).collect();
    Ok(Json(json!({"items": items, "stats": {"count": all.len(), "average": round(avg), "median": round(percentile(&all, 0.5)), "p85": round(percentile(&all, 0.85))}})))
}

#[derive(Deserialize)]
struct QueryParam {
    q: Option<String>,
}

/// The criteria search as a `GET`, for scripts in the sandbox (which may only read).
async fn query_get(State(st): State<St>, ctx: Ctx, Query(p): Query<QueryParam>) -> AppResult<Json<Value>> {
    let text = p.q.ok_or_else(|| bad("q is required: the search as JSON, for example {\"criteria\":[{\"field\":\"assignee\",\"op\":\"in\",\"value\":[\"me\"]}]}"))?;
    let q: crate::planning::ItemQuery = serde_json::from_str(&text).map_err(|e| bad(format!("q is not a valid search: {e}")))?;
    Ok(Json(crate::planning::run_query(&st, &ctx, &q).await?))
}

// ───────────────────────── visualizations ─────────────────────────

fn share_guard(st: &St, ctx: &Ctx, visibility: Option<&str>) -> AppResult<()> {
    match visibility {
        None | Some("private") => Ok(()),
        Some("workspace") => authz::require_global(st, ctx, "analytics.share"),
        Some(_) => Err(bad("visibility must be private or workspace")),
    }
}

fn check_params(v: &Value) -> AppResult<()> {
    let o = v.as_object().filter(|o| o.len() <= 30).ok_or_else(|| bad("params must be an object with at most 30 values"))?;
    for (k, val) in o {
        let simple = val.is_string() || val.is_number() || val.is_boolean() || val.is_null();
        if k.is_empty() || k.len() > 40 || !k.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_') || !simple || val.as_str().is_some_and(|s| s.len() > 500) {
            return Err(bad("params are simple named values: text, numbers, true or false"));
        }
    }
    Ok(())
}

async fn check_editors(st: &St, ids: &[Uuid]) -> AppResult<()> {
    if ids.len() > 50 {
        return Err(bad("at most 50 editors"));
    }
    let known: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE id = ANY($1) AND active").bind(ids).fetch_one(&st.db).await?;
    if known != ids.iter().collect::<std::collections::HashSet<_>>().len() as i64 {
        return Err(bad("an editor is not an active account"));
    }
    Ok(())
}

const VIZ_JSON: &str = "jsonb_build_object('id', v.id, 'name', v.name, 'description', v.description, 'library', v.library, 'params', v.params, 'template', v.template,
    'visibility', v.visibility, 'version', v.version, 'updated_at', v.updated_at, 'owner', person_json(u), 'mine', v.owner_id = $1,
    'can_edit', v.owner_id = $1 OR $1 = ANY(v.editors),
    'editors', (SELECT coalesce(jsonb_agg(person_json(e) ORDER BY e.name), '[]'::jsonb) FROM users e WHERE e.id = ANY(v.editors)))";
/// Visible: one's own, ones one may edit, and shared ones for people with workspace access.
const VIZ_VISIBLE: &str = "NOT v.archived AND (v.owner_id = $1 OR $1 = ANY(v.editors) OR (v.visibility = 'workspace' AND $2))";

async fn list_viz(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    let mine: Value = sqlx::query_scalar(&format!("SELECT coalesce(jsonb_agg({VIZ_JSON} ORDER BY v.name), '[]'::jsonb) FROM visualizations v JOIN users u ON u.id = v.owner_id WHERE {VIZ_VISIBLE}")).bind(ctx.user_id).bind(ctx.has("workspace.access")).fetch_one(&st.db).await?;
    Ok(Json(json!({
        "visualizations": mine,
        "templates": TEMPLATES.iter().map(|t| template_json(t, false)).collect::<Vec<_>>(),
        "libraries": LIBRARIES.iter().map(|(key, name, scripts)| json!({"key": key, "name": name, "scripts": scripts})).collect::<Vec<_>>(),
        "can_share": ctx.has("analytics.share"),
    })))
}

async fn get_template(Path(key): Path<String>, _ctx: Ctx) -> AppResult<Json<Value>> {
    Ok(Json(template_json(TEMPLATES.iter().find(|t| t.key == key).ok_or(AppError::NotFound)?, true)))
}

async fn viz_row(st: &AppState, ctx: &Ctx, id: Uuid) -> AppResult<Value> {
    sqlx::query_scalar(&format!("SELECT {VIZ_JSON} || jsonb_build_object('source', v.source) FROM visualizations v JOIN users u ON u.id = v.owner_id WHERE v.id = $3 AND {VIZ_VISIBLE}")).bind(ctx.user_id).bind(ctx.has("workspace.access")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)
}

async fn get_viz(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    Ok(Json(viz_row(&st, &ctx, id).await?))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VizBody {
    name: Option<String>,
    description: Option<String>,
    library: Option<String>,
    source: Option<String>,
    params: Option<Value>,
    /// Start from a built-in template (create only).
    template: Option<String>,
    visibility: Option<String>,
    editors: Option<Vec<Uuid>>,
    /// Set when the source came out of a shared editing session.
    #[serde(default)]
    collab: bool,
}

async fn check_viz(st: &St, ctx: &Ctx, b: &VizBody) -> AppResult<()> {
    if let Some(n) = &b.name {
        if n.trim().is_empty() || n.chars().count() > 200 {
            return Err(bad("name must be 1 to 200 characters"));
        }
    }
    if b.description.as_ref().is_some_and(|d| d.chars().count() > 5000) {
        return Err(bad("description must be at most 5000 characters"));
    }
    if let Some(l) = &b.library {
        if !LIBRARIES.iter().any(|x| x.0 == l) {
            return Err(bad(format!("library must be one of {}", LIBRARIES.iter().map(|x| x.0).collect::<Vec<_>>().join(", "))));
        }
    }
    if b.source.as_ref().is_some_and(|s| s.len() > 200_000) {
        return Err(bad("the script is larger than 200 KB"));
    }
    if let Some(p) = &b.params {
        check_params(p)?;
    }
    if let Some(e) = &b.editors {
        check_editors(st, e).await?;
    }
    share_guard(st, ctx, b.visibility.as_deref())
}

async fn create_viz(State(st): State<St>, ctx: Ctx, Json(b): Json<VizBody>) -> AppResult<(StatusCode, Json<Value>)> {
    if !ctx.can_write() || !ctx.has("workspace.access") {
        return Err(authz::deny(&st, &ctx, "workspace.access", "visualization"));
    }
    check_viz(&st, &ctx, &b).await?;
    let template = b.template.as_deref().map(|k| TEMPLATES.iter().find(|t| t.key == k).ok_or_else(|| bad("unknown template"))).transpose()?;
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).map(str::to_string).or(template.map(|t| t.name.to_string())).ok_or_else(|| bad("name is required"))?;
    let library = b.library.as_deref().or(template.map(|t| t.library)).ok_or_else(|| bad("library is required"))?;
    let source = b.source.clone().or(template.map(|t| t.source.to_string())).unwrap_or_default();
    let params = b.params.clone().unwrap_or_else(|| template.map(|t| Value::Object(t.params.iter().map(|(k, _, d)| (k.to_string(), serde_json::from_str(d).unwrap_or(Value::Null))).collect())).unwrap_or(json!({})));
    let own: i64 = sqlx::query_scalar("SELECT count(*) FROM visualizations WHERE owner_id = $1 AND NOT archived").bind(ctx.user_id).fetch_one(&st.db).await?;
    if own >= 500 {
        return Err(bad("you have 500 visualizations; delete some first"));
    }
    let id: Uuid = sqlx::query_scalar("INSERT INTO visualizations (owner_id, name, description, library, source, params, template, visibility, editors) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9) RETURNING id")
        .bind(ctx.user_id)
        .bind(&name)
        .bind(b.description.as_deref().or(template.map(|t| t.description)).unwrap_or_default())
        .bind(library)
        .bind(&source)
        .bind(&params)
        .bind(template.map(|t| t.key))
        .bind(b.visibility.as_deref().unwrap_or("private"))
        .bind(b.editors.clone().unwrap_or_default())
        .fetch_one(&st.db)
        .await?;
    audit::log(&st, &ctx, "visualization.create", "visualization", &id.to_string(), json!({"name": name, "library": library, "template": b.template, "visibility": b.visibility})).await;
    Ok((StatusCode::CREATED, Json(viz_row(&st, &ctx, id).await?)))
}

async fn update_viz(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<VizBody>) -> AppResult<Json<Value>> {
    let cur = viz_row(&st, &ctx, id).await?;
    if cur["can_edit"] != true || !ctx.can_write() {
        return Err(AppError::Forbidden("only the owner and the editors they named can change this; save a copy of your own instead".into()));
    }
    // Who else may edit, and who may see it, is the owner's decision.
    if (b.editors.is_some() || b.visibility.is_some()) && cur["mine"] != true {
        return Err(AppError::Forbidden("only the owner decides who can see and edit this".into()));
    }
    if b.template.is_some() {
        return Err(bad("template applies only when creating"));
    }
    check_viz(&st, &ctx, &b).await?;
    sqlx::query("UPDATE visualizations SET name = coalesce($2, name), description = coalesce($3, description), library = coalesce($4, library), source = coalesce($5, source), params = coalesce($6, params),
                 visibility = coalesce($7, visibility), editors = coalesce($8, editors), version = version + 1, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(b.name.as_deref().map(str::trim))
        .bind(&b.description)
        .bind(&b.library)
        .bind(&b.source)
        .bind(&b.params)
        .bind(&b.visibility)
        .bind(&b.editors)
        .execute(&st.db)
        .await?;
    // The script is what runs in other people's browsers, so each change to it is on record.
    audit::log(&st, &ctx, "visualization.update", "visualization", &id.to_string(), json!({"name": cur["name"], "source_changed": b.source.is_some(), "source_bytes": b.source.as_ref().map(String::len), "visibility": b.visibility, "editors": b.editors})).await;
    let room = format!("viz:{id}");
    if b.source.is_some() {
        if b.collab {
            realtime::mark_saved(&st, &room, "source", cur["version"].as_i64().unwrap_or(0) + 1).await;
        } else {
            realtime::reset_doc(&st, &room, "source").await;
        }
    }
    realtime::publish(&st, &room, json!({"t": "event", "room": room, "kind": "visualization.updated", "data": {"id": id, "actor_id": ctx.user_id}})).await;
    Ok(Json(viz_row(&st, &ctx, id).await?))
}

async fn delete_viz(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let n = sqlx::query("UPDATE visualizations SET archived = true, updated_at = now() WHERE id = $1 AND owner_id = $2 AND NOT archived").bind(id).bind(ctx.user_id).execute(&st.db).await?.rows_affected();
    if n == 0 || !ctx.can_write() {
        return Err(AppError::NotFound);
    }
    audit::log(&st, &ctx, "visualization.delete", "visualization", &id.to_string(), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

// ───────────────────────── dashboards ─────────────────────────

const DASH_JSON: &str = "jsonb_build_object('id', d.id, 'name', d.name, 'description', d.description, 'params', d.params, 'visibility', d.visibility, 'version', d.version, 'updated_at', d.updated_at,
    'owner', person_json(u), 'mine', d.owner_id = $1, 'can_edit', d.owner_id = $1 OR $1 = ANY(d.editors), 'tiles', jsonb_array_length(d.layout),
    'editors', (SELECT coalesce(jsonb_agg(person_json(e) ORDER BY e.name), '[]'::jsonb) FROM users e WHERE e.id = ANY(d.editors)))";
const DASH_VISIBLE: &str = "NOT d.archived AND (d.owner_id = $1 OR $1 = ANY(d.editors) OR (d.visibility = 'workspace' AND $2))";

async fn list_dashboards(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    let list: Value = sqlx::query_scalar(&format!("SELECT coalesce(jsonb_agg({DASH_JSON} ORDER BY d.name), '[]'::jsonb) FROM dashboards d JOIN users u ON u.id = d.owner_id WHERE {DASH_VISIBLE}")).bind(ctx.user_id).bind(ctx.has("workspace.access")).fetch_one(&st.db).await?;
    Ok(Json(json!({"dashboards": list, "can_share": ctx.has("analytics.share")})))
}

/// A dashboard with everything needed to draw it: each tile's script, library and parameters.
/// A tile whose visualization the viewer may not see is returned without its script.
async fn dashboard_json(st: &AppState, ctx: &Ctx, id: Uuid) -> AppResult<Value> {
    let mut d: Value = sqlx::query_scalar(&format!("SELECT {DASH_JSON} || jsonb_build_object('layout', d.layout) FROM dashboards d JOIN users u ON u.id = d.owner_id WHERE d.id = $3 AND {DASH_VISIBLE}")).bind(ctx.user_id).bind(ctx.has("workspace.access")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let wanted: Vec<Uuid> = d["layout"].as_array().into_iter().flatten().filter(|t| t["kind"] == "visualization").filter_map(|t| t["ref"].as_str()?.parse().ok()).collect();
    let rows = sqlx::query(&format!("SELECT v.id, v.name, v.library, v.source, v.params, person_json(u) AS owner FROM visualizations v JOIN users u ON u.id = v.owner_id WHERE v.id = ANY($3) AND {VIZ_VISIBLE}")).bind(ctx.user_id).bind(ctx.has("workspace.access")).bind(&wanted).fetch_all(&st.db).await?;
    if let Some(tiles) = d["layout"].as_array_mut() {
        for t in tiles {
            let resolved = match (t["kind"].as_str(), t["ref"].as_str()) {
                (Some("template"), Some(key)) => TEMPLATES.iter().find(|x| x.key == key).map(|x| json!({"name": x.name, "library": x.library, "source": x.source, "params": template_json(x, false)["params"].as_array().map(|a| a.iter().map(|p| (p["key"].as_str().unwrap_or_default().to_string(), p["default"].clone())).collect::<Map<_, _>>()), "builtin": true})),
                (Some("visualization"), Some(vid)) => rows.iter().find(|r| r.get::<Uuid, _>("id").to_string() == vid).map(|r| json!({"name": r.get::<String, _>("name"), "library": r.get::<String, _>("library"), "source": r.get::<String, _>("source"), "params": r.get::<Value, _>("params"), "owner": r.get::<Value, _>("owner")})),
                _ => None,
            };
            t["content"] = resolved.unwrap_or(Value::Null);
        }
    }
    Ok(d)
}

async fn get_dashboard(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    Ok(Json(dashboard_json(&st, &ctx, id).await?))
}

/// The visualizations a dashboard already shows, as ids.
fn placed(layout: &Value) -> Vec<String> {
    layout.as_array().into_iter().flatten().filter(|t| t["kind"] == "visualization").filter_map(|t| t["ref"].as_str().map(str::to_string)).collect()
}

/// One tile: what it shows, where it sits on the 12-column grid, and its own parameter values.
/// `already` lists the visualizations the dashboard shows now: an editor may move or resize
/// such a tile without being able to open the visualization, but may not place a new one.
async fn check_tile(st: &St, ctx: &Ctx, t: &Value, already: &[String]) -> AppResult<Value> {
    let o = t.as_object().ok_or_else(|| bad("a tile must be an object"))?;
    if let Some(k) = o.keys().find(|k| !["id", "kind", "ref", "title", "x", "y", "w", "h", "params", "content"].contains(&k.as_str())) {
        return Err(bad(format!("unknown tile field {k:?}")));
    }
    let id = t["id"].as_str().filter(|i| !i.is_empty() && i.len() <= 40 && i.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')).ok_or_else(|| bad("a tile needs an id of letters, digits, dashes and underscores"))?;
    let reference = t["ref"].as_str().ok_or_else(|| bad("a tile needs a ref"))?;
    match t["kind"].as_str() {
        Some("template") => {
            if !TEMPLATES.iter().any(|x| x.key == reference) {
                return Err(bad(format!("unknown built-in visualization {reference:?}")));
            }
        }
        Some("visualization") => {
            let vid: Uuid = reference.parse().map_err(|_| bad("ref must be a visualization id"))?;
            // Only what the person placing the tile can see may be placed.
            if !already.iter().any(|r| r == reference) {
                viz_row(st, ctx, vid).await.map_err(|_| bad("that visualization does not exist or is not shared with you"))?;
            }
        }
        _ => return Err(bad("a tile's kind is template or visualization")),
    }
    let n = |k: &str, lo: i64, hi: i64, default: i64| -> AppResult<i64> {
        match t.get(k) {
            None | Some(Value::Null) => Ok(default),
            Some(v) => v.as_i64().filter(|x| (lo..=hi).contains(x)).ok_or_else(|| bad(format!("tile {k} must be a whole number between {lo} and {hi}"))),
        }
    };
    let (w, h) = (n("w", 1, 12, 4)?, n("h", 1, 12, 4)?);
    let (x, y) = (n("x", 0, 11, 0)?, n("y", 0, 500, 0)?);
    if x + w > 12 {
        return Err(bad("a tile cannot extend past the twelfth column"));
    }
    let title = t.get("title").and_then(Value::as_str).unwrap_or_default();
    if title.chars().count() > 120 {
        return Err(bad("a tile title is at most 120 characters"));
    }
    let params = t.get("params").cloned().unwrap_or(json!({}));
    check_params(&params)?;
    Ok(json!({"id": id, "kind": t["kind"], "ref": reference, "title": title, "x": x, "y": y, "w": w, "h": h, "params": params}))
}

async fn check_layout(st: &St, ctx: &Ctx, v: &Value, already: &[String]) -> AppResult<Value> {
    let list = v.as_array().filter(|a| a.len() <= 40).ok_or_else(|| bad("layout must be a list of at most 40 tiles"))?;
    let mut out = vec![];
    let mut seen = std::collections::HashSet::new();
    for t in list {
        let tile = check_tile(st, ctx, t, already).await?;
        if !seen.insert(tile["id"].as_str().unwrap_or_default().to_string()) {
            return Err(bad("two tiles have the same id"));
        }
        out.push(tile);
    }
    Ok(json!(out))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DashBody {
    name: Option<String>,
    description: Option<String>,
    layout: Option<Value>,
    params: Option<Value>,
    visibility: Option<String>,
    editors: Option<Vec<Uuid>>,
}

async fn check_dash(st: &St, ctx: &Ctx, b: &DashBody, already: &[String]) -> AppResult<Option<Value>> {
    if let Some(n) = &b.name {
        if n.trim().is_empty() || n.chars().count() > 200 {
            return Err(bad("name must be 1 to 200 characters"));
        }
    }
    if b.description.as_ref().is_some_and(|d| d.chars().count() > 5000) {
        return Err(bad("description must be at most 5000 characters"));
    }
    if let Some(p) = &b.params {
        check_params(p)?;
    }
    if let Some(e) = &b.editors {
        check_editors(st, e).await?;
    }
    share_guard(st, ctx, b.visibility.as_deref())?;
    match &b.layout {
        Some(l) => Ok(Some(check_layout(st, ctx, l, already).await?)),
        None => Ok(None),
    }
}

async fn create_dashboard(State(st): State<St>, ctx: Ctx, Json(b): Json<DashBody>) -> AppResult<(StatusCode, Json<Value>)> {
    if !ctx.can_write() || !ctx.has("workspace.access") {
        return Err(authz::deny(&st, &ctx, "workspace.access", "dashboard"));
    }
    let layout = check_dash(&st, &ctx, &b, &[]).await?;
    let name = b.name.as_deref().map(str::trim).filter(|n| !n.is_empty()).ok_or_else(|| bad("name is required"))?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO dashboards (owner_id, name, description, layout, params, visibility, editors) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id")
        .bind(ctx.user_id)
        .bind(name)
        .bind(b.description.as_deref().unwrap_or_default())
        .bind(layout.unwrap_or(json!([])))
        .bind(b.params.clone().unwrap_or(json!({})))
        .bind(b.visibility.as_deref().unwrap_or("private"))
        .bind(b.editors.clone().unwrap_or_default())
        .fetch_one(&st.db)
        .await?;
    audit::log(&st, &ctx, "dashboard.create", "dashboard", &id.to_string(), json!({"name": name, "visibility": b.visibility})).await;
    Ok((StatusCode::CREATED, Json(dashboard_json(&st, &ctx, id).await?)))
}

async fn editable_dashboard(st: &St, ctx: &Ctx, id: Uuid) -> AppResult<Value> {
    let cur = dashboard_json(st, ctx, id).await?;
    if cur["can_edit"] != true || !ctx.can_write() {
        return Err(AppError::Forbidden("only the owner and the editors they named can change this dashboard; save a copy of your own instead".into()));
    }
    Ok(cur)
}

async fn dashboard_changed(st: &AppState, ctx: &Ctx, id: Uuid) {
    let room = format!("dashboard:{id}");
    realtime::publish(st, &room, json!({"t": "event", "room": room, "kind": "dashboard.updated", "data": {"id": id, "actor_id": ctx.user_id}})).await;
}

async fn update_dashboard(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<DashBody>) -> AppResult<Json<Value>> {
    let cur = editable_dashboard(&st, &ctx, id).await?;
    if (b.editors.is_some() || b.visibility.is_some()) && cur["mine"] != true {
        return Err(AppError::Forbidden("only the owner decides who can see and edit this".into()));
    }
    let layout = check_dash(&st, &ctx, &b, &placed(&cur["layout"])).await?;
    sqlx::query("UPDATE dashboards SET name = coalesce($2, name), description = coalesce($3, description), layout = coalesce($4, layout), params = coalesce($5, params),
                 visibility = coalesce($6, visibility), editors = coalesce($7, editors), version = version + 1, updated_at = now() WHERE id = $1")
        .bind(id)
        .bind(b.name.as_deref().map(str::trim))
        .bind(&b.description)
        .bind(&layout)
        .bind(&b.params)
        .bind(&b.visibility)
        .bind(&b.editors)
        .execute(&st.db)
        .await?;
    audit::log(&st, &ctx, "dashboard.update", "dashboard", &id.to_string(), json!({"name": cur["name"], "visibility": b.visibility, "editors": b.editors, "layout_changed": b.layout.is_some()})).await;
    dashboard_changed(&st, &ctx, id).await;
    Ok(Json(dashboard_json(&st, &ctx, id).await?))
}

/// Add, change or remove one tile. Each call changes only that tile, under a row lock, so
/// people arranging the same dashboard at the same time do not undo each other's work.
async fn put_tile(State(st): State<St>, ctx: Ctx, Path((id, tile)): Path<(Uuid, String)>, Json(b): Json<Value>) -> AppResult<Json<Value>> {
    editable_dashboard(&st, &ctx, id).await?;
    let mut incoming = b;
    incoming["id"] = json!(tile);
    let mut tx = st.db.begin().await?;
    let mut layout: Vec<Value> = serde_json::from_value(sqlx::query_scalar("SELECT layout FROM dashboards WHERE id = $1 FOR UPDATE").bind(id).fetch_one(&mut *tx).await?).unwrap_or_default();
    let already = placed(&json!(layout));
    let new_tile = match layout.iter().position(|t| t["id"] == tile.as_str()) {
        Some(i) => {
            // Fields that were not sent keep their value.
            let mut merged = layout[i].clone();
            for (k, v) in incoming.as_object().into_iter().flatten() {
                merged[k] = v.clone();
            }
            let checked = check_tile(&st, &ctx, &merged, &already).await?;
            layout[i] = checked.clone();
            checked
        }
        None => {
            if layout.len() >= 40 {
                return Err(bad("a dashboard holds at most 40 tiles"));
            }
            let checked = check_tile(&st, &ctx, &incoming, &already).await?;
            layout.push(checked.clone());
            checked
        }
    };
    sqlx::query("UPDATE dashboards SET layout = $2, version = version + 1, updated_at = now() WHERE id = $1").bind(id).bind(json!(layout)).execute(&mut *tx).await?;
    tx.commit().await?;
    dashboard_changed(&st, &ctx, id).await;
    Ok(Json(new_tile))
}

async fn delete_tile(State(st): State<St>, ctx: Ctx, Path((id, tile)): Path<(Uuid, String)>) -> AppResult<StatusCode> {
    editable_dashboard(&st, &ctx, id).await?;
    let mut tx = st.db.begin().await?;
    let layout: Vec<Value> = serde_json::from_value(sqlx::query_scalar("SELECT layout FROM dashboards WHERE id = $1 FOR UPDATE").bind(id).fetch_one(&mut *tx).await?).unwrap_or_default();
    let kept: Vec<&Value> = layout.iter().filter(|t| t["id"] != tile.as_str()).collect();
    if kept.len() == layout.len() {
        return Err(AppError::NotFound);
    }
    sqlx::query("UPDATE dashboards SET layout = $2, version = version + 1, updated_at = now() WHERE id = $1").bind(id).bind(json!(kept)).execute(&mut *tx).await?;
    tx.commit().await?;
    dashboard_changed(&st, &ctx, id).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_dashboard(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let n = sqlx::query("UPDATE dashboards SET archived = true, updated_at = now() WHERE id = $1 AND owner_id = $2 AND NOT archived").bind(id).bind(ctx.user_id).execute(&st.db).await?.rows_affected();
    if n == 0 || !ctx.can_write() {
        return Err(AppError::NotFound);
    }
    audit::log(&st, &ctx, "dashboard.delete", "dashboard", &id.to_string(), json!({})).await;
    Ok(StatusCode::NO_CONTENT)
}

/// What the caller may do in the real-time room of a dashboard, visualization or Gantt chart.
pub async fn room_access(st: &St, ctx: &Ctx, kind: &str, id: Uuid) -> AppResult<Access> {
    let found = match kind {
        "dashboard" => dashboard_json(st, ctx, id).await.map(|d| d["can_edit"] == true),
        "viz" => viz_row(st, ctx, id).await.map(|v| v["can_edit"] == true),
        "gantt" => crate::planning::load_gantt(st, ctx, id).await.map(|g| g.get::<Uuid, _>("owner_id") == ctx.user_id),
        _ => Err(AppError::NotFound),
    };
    Ok(match found {
        Ok(edit) => Access { read: true, write: edit && ctx.can_write() },
        Err(AppError::NotFound) => Access { read: false, write: false },
        Err(e) => return Err(e),
    })
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/sandbox/frame", "Analytics", "The empty document that custom content and visualization scripts run in", get(frame))
        .public()
        .route("GET", "/api/v1/analytics/items", "Analytics", "Work item counts or estimates by one to three groupings", get(items))
        .route("GET", "/api/v1/analytics/summary", "Analytics", "Key figures: open, overdue, unassigned, recently created and resolved", get(summary))
        .route("GET", "/api/v1/analytics/timeline", "Analytics", "Created, resolved and open work per day, week or month", get(timeline))
        .route("GET", "/api/v1/analytics/workload", "Analytics", "Open and recently finished work per assignee", get(workload))
        .route("GET", "/api/v1/analytics/cycle-time", "Analytics", "Days from creation to resolution for recently resolved work", get(cycle_time))
        .route("GET", "/api/v1/items/query", "Search", "The criteria search as a GET (?q= the search as JSON), for read-only clients", get(query_get))
        .route("GET", "/api/v1/visualizations", "Analytics", "Saved visualizations, the built-in ones and the chart libraries", get(list_viz))
        .route("POST", "/api/v1/visualizations", "Analytics", "Create a visualization, from scratch or from a built-in one", post(create_viz))
        .route("GET", "/api/v1/visualization-templates/{key}", "Analytics", "A built-in visualization with its script", get(get_template))
        .route("GET", "/api/v1/visualizations/{id}", "Analytics", "A visualization with its script", get(get_viz))
        .route("PATCH", "/api/v1/visualizations/{id}", "Analytics", "Change a visualization's script, parameters, sharing or editors", patch(update_viz))
        .route("DELETE", "/api/v1/visualizations/{id}", "Analytics", "Delete a visualization", delete(delete_viz))
        .route("GET", "/api/v1/dashboards", "Analytics", "Dashboards: own, editable and shared", get(list_dashboards))
        .route("POST", "/api/v1/dashboards", "Analytics", "Create a dashboard", post(create_dashboard))
        .route("GET", "/api/v1/dashboards/{id}", "Analytics", "A dashboard with its tiles and their scripts", get(get_dashboard))
        .route("PATCH", "/api/v1/dashboards/{id}", "Analytics", "Change a dashboard's name, layout, parameters, sharing or editors", patch(update_dashboard))
        .route("DELETE", "/api/v1/dashboards/{id}", "Analytics", "Delete a dashboard", delete(delete_dashboard))
        .route("PUT", "/api/v1/dashboards/{id}/tiles/{tile}", "Analytics", "Add or change one tile", axum::routing::put(put_tile))
        .route("DELETE", "/api/v1/dashboards/{id}/tiles/{tile}", "Analytics", "Remove one tile", delete(delete_tile))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn templates_are_complete_and_use_known_libraries() {
        let mut seen = std::collections::HashSet::new();
        for t in TEMPLATES {
            assert!(seen.insert(t.key), "duplicate {}", t.key);
            assert!(LIBRARIES.iter().any(|l| l.0 == t.library), "{}", t.key);
            assert!(t.source.len() > 200 && t.source.contains("gantry.api.get"), "{} reads its data through the broker", t.key);
            assert!((1..=12).contains(&t.size.0) && (1..=12).contains(&t.size.1));
            for (key, _, default) in t.params {
                assert!(serde_json::from_str::<Value>(default).is_ok(), "{}: default of {key} is JSON", t.key);
            }
            // Built-in scripts never reach for the network or the page around them.
            for forbidden in ["fetch(", "XMLHttpRequest", "WebSocket", "window.parent", "window.top", "document.cookie", "localStorage", "eval(", "innerHTML"] {
                assert!(!t.source.contains(forbidden), "{} uses {forbidden}", t.key);
            }
        }
        for lib in ["chartjs", "echarts", "plotly", "d3", "html"] {
            assert!(TEMPLATES.iter().any(|t| t.library == lib), "no built-in example for {lib}");
        }
    }

    #[test]
    fn the_frame_policy_allows_no_way_out() {
        let csp = frame_csp();
        for must in ["default-src 'none'", "connect-src 'none'", "form-action 'none'", "base-uri 'none'", "frame-src 'none'", "worker-src 'none'", "object-src 'none'", "webrtc 'block'", "frame-ancestors 'self'"] {
            assert!(csp.contains(must), "{must}");
        }
        // Scripts come only from this server (the chart libraries) or the page itself;
        // pictures only from data the script already holds.
        assert!(csp.contains("script-src 'unsafe-inline' 'unsafe-eval' 'self';"));
        assert!(csp.contains("img-src data: blob:;"));
        assert!(!csp.contains("http:") && !csp.contains("https:") && !csp.contains('*'), "no remote origin is named anywhere");
        assert!(!FRAME_HTML.contains("http://") && !FRAME_HTML.contains("https://"), "the frame document refers to nothing off this server");
    }

    #[test]
    fn group_expressions_are_fixed_text() {
        for (name, key, label, extra) in GROUPS {
            for part in [key, label, extra] {
                assert!(!part.contains('$') && !part.contains(';') && !part.contains("--"), "{name}");
            }
        }
    }

    #[test]
    fn parameters_are_simple_named_values() {
        assert!(check_params(&json!({"project": "OPS", "weeks": 12, "show": true, "x": null})).is_ok());
        for bad in [json!({"a": {"nested": 1}}), json!({"a": [1]}), json!({"bad key": 1}), json!({"": 1}), json!({"a": "x".repeat(501)}), json!([1]), json!("x")] {
            assert!(check_params(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn percentiles() {
        let v = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0, 10.0];
        assert_eq!(percentile(&v, 0.5), 6.0);
        assert_eq!(percentile(&v, 0.85), 9.0);
        assert_eq!(percentile(&v, 0.0), 1.0);
        assert_eq!(percentile(&v, 1.0), 10.0);
        assert_eq!(percentile(&[], 0.5), 0.0);
        assert_eq!(percentile(&[4.2], 0.85), 4.2);
    }
}
