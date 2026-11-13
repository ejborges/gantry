//! Documentation: spaces, pages, version history, templates. Knowledge bases are spaces of
//! kind `knowledge_base` whose published, customer-visible pages surface in the service desk portal.
//!
//! Pages are stored as Markdown. Both editors in the client (the simple rich-text editor and the
//! advanced source editor), the REST API and the MCP tools read and write the same text, so nothing
//! is lost switching between them. Every save that changes title or content creates a version.

use crate::auth::Ctx;
use crate::authz::{self, Perms};
use crate::error::{bad, AppError, AppResult};
use crate::events::{self, Event};
use crate::http::Api;
use crate::items;
use crate::projects::{self, MemberBody};
use crate::storage::SearchDoc;
use crate::{audit, jobs, AppState, St};
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::routing::{delete, get, patch, post};
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::Row;
use uuid::Uuid;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Space {
    pub id: Uuid,
    pub key: String,
    pub name: String,
    pub description: String,
    pub kind: String,
    pub access: String,
    pub project_id: Option<Uuid>,
    pub archived: bool,
    /// The stages a page moves through before it is published, if the space defines them.
    pub workflow: Option<Value>,
}
const SPACE_COLS: &str = "id, key, name, description, kind, access, project_id, archived, workflow";

impl Space {
    fn json(&self) -> Value {
        json!({"id": self.id, "key": self.key, "name": self.name, "description": self.description, "kind": self.kind, "access": self.access, "archived": self.archived, "workflow": self.workflow})
    }

    /// The space's page workflow. Stages in the `done` category are published; the others are drafts.
    pub fn stages(&self) -> Option<crate::packs::WorkflowDef> {
        self.workflow.as_ref().and_then(|w| serde_json::from_value(w.clone()).ok())
    }
}

/// `published` for a stage in the done category, otherwise `draft`.
fn status_of(wf: &crate::packs::WorkflowDef, stage: &str) -> &'static str {
    if wf.status(stage).is_some_and(|s| s.category == "done") {
        "published"
    } else {
        "draft"
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Page {
    pub id: Uuid,
    pub space_id: Uuid,
    pub parent_id: Option<Uuid>,
    pub title: String,
    pub content: String,
    pub version: i32,
    pub status: String,
    pub customer_visible: bool,
    pub labels: Vec<String>,
    pub editor: String,
    pub archived: bool,
    pub stage: String,
    pub refs: Value,
}
const PAGE_COLS: &str = "id, space_id, parent_id, title, content, version, status, customer_visible, labels, editor, archived, stage, refs";

pub async fn space_key_free(st: &AppState, key: &str) -> AppResult<bool> {
    Ok(!sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM spaces WHERE key = $1)").bind(key).fetch_one(&st.db).await?)
}

pub async fn load_space(st: &St, ctx: &Ctx, key: &str) -> AppResult<(Space, Perms)> {
    let space: Space = sqlx::query_as(&format!("SELECT {SPACE_COLS} FROM spaces WHERE key = $1")).bind(key.to_uppercase()).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let perms = authz::space_perms(st, ctx, space.id, &space.access).await?;
    if !perms.has("space.view") {
        return Err(authz::hide(st, ctx, "space.view", &format!("space:{}", space.key)));
    }
    Ok((space, perms))
}

pub async fn load_page(st: &St, ctx: &Ctx, id: Uuid) -> AppResult<(Page, Space, Perms)> {
    let page: Page = sqlx::query_as(&format!("SELECT {PAGE_COLS} FROM pages WHERE id = $1 AND NOT archived")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let space: Space = sqlx::query_as(&format!("SELECT {SPACE_COLS} FROM spaces WHERE id = $1")).bind(page.space_id).fetch_one(&st.db).await?;
    let perms = authz::space_perms(st, ctx, space.id, &space.access).await?;
    // Drafts are visible only to people who can edit.
    if !perms.has("space.view") || (page.status == "draft" && !perms.has("page.edit")) {
        return Err(authz::hide(st, ctx, "space.view", &format!("page:{id}")));
    }
    Ok((page, space, perms))
}

/// (may view, may edit) for a page, or `None` when there is no such page. Makes no record
/// of a refusal; for callers that only need to know.
pub async fn page_access(st: &St, ctx: &Ctx, id: Uuid) -> AppResult<Option<(bool, bool)>> {
    let Some(row) = sqlx::query("SELECT pg.status, s.id, s.access FROM pages pg JOIN spaces s ON s.id = pg.space_id WHERE pg.id = $1 AND NOT pg.archived").bind(id).fetch_optional(&st.db).await? else { return Ok(None) };
    let perms = authz::space_perms(st, ctx, row.get("id"), &row.get::<String, _>("access")).await?;
    let edit = perms.has("page.edit");
    Ok(Some((perms.has("space.view") && (edit || row.get::<String, _>("status") != "draft"), edit)))
}

pub async fn can_view_page(st: &St, ctx: &Ctx, id: Uuid) -> AppResult<bool> {
    match load_page(st, ctx, id).await {
        Ok(_) => Ok(true),
        Err(AppError::NotFound) => Ok(false),
        Err(e) => Err(e),
    }
}

pub async fn index_page(st: &AppState, id: Uuid) {
    let r: Result<(), anyhow::Error> = async {
        let row = sqlx::query("SELECT pg.space_id, pg.title, pg.content, pg.status, pg.customer_visible, pg.archived, s.key FROM pages pg JOIN spaces s ON s.id = pg.space_id WHERE pg.id = $1").bind(id).fetch_one(&st.db).await?;
        if row.get::<bool, _>("archived") || row.get::<String, _>("status") != "published" {
            st.stores.search.remove("page", id).await?;
            st.stores.vector.remove("page", id).await?;
            return Ok(());
        }
        let key: String = row.get("key");
        st.stores
            .search
            .index(&SearchDoc { kind: "page".into(), ref_id: id, project_id: None, space_id: Some(row.get("space_id")), url: format!("/docs/{key}/{id}"), label: key, title: row.get("title"), body: row.get("content"), customer_visible: row.get("customer_visible") })
            .await?;
        if st.stores.embedder.is_some() {
            jobs::enqueue(st, "embed", json!({"kind": "page", "id": id})).await?;
        }
        Ok(())
    }
    .await;
    if let Err(e) = r {
        tracing::error!(error = %e, page = %id, "search indexing failed");
    }
}

/// Insert a space and make the creator its administrator. No permission check.
#[allow(clippy::too_many_arguments)]
pub async fn create_space_raw(st: &AppState, ctx: &Ctx, key: &str, name: &str, description: &str, kind: &str, access: &str, project_id: Option<Uuid>) -> AppResult<Uuid> {
    let mut tx = st.db.begin().await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO spaces (key, name, description, kind, access, project_id, created_by) VALUES ($1,$2,$3,$4,$5,$6,$7) RETURNING id").bind(key).bind(name).bind(description).bind(kind).bind(access).bind(project_id).bind(ctx.user_id).fetch_one(&mut *tx).await?;
    sqlx::query("INSERT INTO role_bindings (role_id, user_id, scope_type, scope_id) SELECT id, $1, 'space', $2 FROM roles WHERE key = 'space_admin'").bind(ctx.user_id).bind(id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(id)
}

/// Insert a page with its first version. No permission check.
#[allow(clippy::too_many_arguments)]
pub async fn create_page_raw(st: &AppState, author: Uuid, space_id: Uuid, parent: Option<Uuid>, title: &str, content: &str, status: &str, customer_visible: bool, position: i32) -> AppResult<Uuid> {
    let mut tx = st.db.begin().await?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO pages (space_id, parent_id, title, content, status, customer_visible, position, created_by, updated_by) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$8) RETURNING id")
        .bind(space_id)
        .bind(parent)
        .bind(title)
        .bind(content)
        .bind(status)
        .bind(customer_visible)
        .bind(position)
        .bind(author)
        .fetch_one(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO page_versions (page_id, version, title, content, author_id, message) VALUES ($1, 1, $2, $3, $4, 'Created')").bind(id).bind(title).bind(content).bind(author).execute(&mut *tx).await?;
    tx.commit().await?;
    index_page(st, id).await;
    Ok(id)
}

fn check_title(t: &str) -> AppResult<&str> {
    let t = t.trim();
    if t.is_empty() || t.chars().count() > 300 || t.chars().any(crate::accounts::deceptive) {
        return Err(bad("title must be 1 to 300 characters, on one line"));
    }
    Ok(t)
}

fn check_content(c: &str) -> AppResult<()> {
    if c.len() > 2_000_000 {
        return Err(bad("page content is limited to 2 MB of text; move large material into attachments"));
    }
    Ok(())
}

pub async fn list_spaces(st: &St, ctx: &Ctx) -> AppResult<Value> {
    let visible = authz::visible_space_ids(st, ctx).await?;
    Ok(sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(to_jsonb(t) ORDER BY t.name), '[]'::jsonb) FROM (
           SELECT s.id, s.key, s.name, s.description, s.kind, s.access,
                  (SELECT p.key FROM projects p WHERE p.id = s.project_id) AS project,
                  (SELECT count(*) FROM pages pg WHERE pg.space_id = s.id AND NOT pg.archived AND pg.status = 'published') AS pages,
                  (SELECT max(pg.updated_at) FROM pages pg WHERE pg.space_id = s.id AND NOT pg.archived) AS updated_at
           FROM spaces s WHERE s.id = ANY($1)) t",
    )
    .bind(&visible)
    .fetch_one(&st.db)
    .await?)
}

#[derive(Deserialize)]
struct NewSpace {
    key: String,
    name: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    kind: Option<String>,
    #[serde(default)]
    access: Option<String>,
}

async fn h_create_space(State(st): State<St>, ctx: Ctx, Json(b): Json<NewSpace>) -> AppResult<(StatusCode, Json<Value>)> {
    authz::require_global(&st, &ctx, "space.create")?;
    let key = b.key.trim().to_uppercase();
    if !projects::valid_project_key(&key) {
        return Err(bad("key must be 2-10 characters: an uppercase letter followed by uppercase letters or digits"));
    }
    if b.name.trim().is_empty() || b.name.len() > 200 {
        return Err(bad("name must be 1 to 200 characters"));
    }
    let kind = b.kind.unwrap_or_else(|| "wiki".into());
    let access = b.access.unwrap_or_else(|| "private".into());
    if !["wiki", "knowledge_base"].contains(&kind.as_str()) || !["private", "internal"].contains(&access.as_str()) {
        return Err(bad("kind must be wiki or knowledge_base; access must be private or internal"));
    }
    let id = create_space_raw(&st, &ctx, &key, b.name.trim(), &b.description, &kind, &access, None).await?;
    audit::log(&st, &ctx, "space.create", "space", &key, json!({"name": b.name.trim(), "kind": kind, "access": access})).await;
    Ok((StatusCode::CREATED, Json(json!({"id": id, "key": key}))))
}

pub async fn space_json(st: &St, ctx: &Ctx, key: &str) -> AppResult<Value> {
    let (space, perms) = load_space(st, ctx, key).await?;
    let tree: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'parent_id', parent_id, 'title', title, 'position', position, 'status', status,
            'customer_visible', customer_visible, 'updated_at', updated_at) ORDER BY position, title), '[]'::jsonb)
         FROM pages WHERE space_id = $1 AND NOT archived AND (status = 'published' OR $2)",
    )
    .bind(space.id)
    .bind(perms.has("page.edit"))
    .fetch_one(&st.db)
    .await?;
    let members: Value = if perms.has("space.admin") {
        sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(jsonb_build_object('binding_id', b.id, 'role', r.key, 'role_name', r.name,
                'user', CASE WHEN u.id IS NULL THEN NULL ELSE person_json(u) || jsonb_build_object('email', u.email) END,
                'group', CASE WHEN g.id IS NULL THEN NULL ELSE jsonb_build_object('id', g.id, 'name', g.name) END) ORDER BY coalesce(u.name, g.name)), '[]'::jsonb)
             FROM role_bindings b JOIN roles r ON r.id = b.role_id LEFT JOIN users u ON u.id = b.user_id LEFT JOIN groups g ON g.id = b.group_id
             WHERE b.scope_type = 'space' AND b.scope_id = $1",
        )
        .bind(space.id)
        .fetch_one(&st.db)
        .await?
    } else {
        json!([])
    };
    let project: Option<String> = match space.project_id {
        Some(p) => sqlx::query_scalar("SELECT key FROM projects WHERE id = $1").bind(p).fetch_optional(&st.db).await?,
        None => None,
    };
    let perm_list: Vec<String> = perms.list().into_iter().filter(|p| ctx.token_allows(p) && authz::PERMISSIONS.iter().any(|(k, s, _)| k == p && *s == "space")).collect();
    Ok(json!({"space": space.json(), "permissions": perm_list, "pages": tree, "members": members, "project": project}))
}

async fn h_get_space(State(st): State<St>, ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    Ok(Json(space_json(&st, &ctx, &key).await?))
}

async fn h_update_space(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<Map<String, Value>>) -> AppResult<StatusCode> {
    let (mut space, perms) = load_space(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "space.admin", &format!("space:{}", space.key))?;
    for (k, v) in &b {
        match k.as_str() {
            "name" => space.name = v.as_str().filter(|s| !s.trim().is_empty() && s.len() <= 200 && !s.chars().any(crate::accounts::deceptive)).ok_or_else(|| bad("name must be 1 to 200 characters"))?.trim().to_string(),
            "description" => space.description = v.as_str().ok_or_else(|| bad("description must be text"))?.to_string(),
            "access" => space.access = v.as_str().filter(|s| ["private", "internal"].contains(s)).ok_or_else(|| bad("access must be private or internal"))?.to_string(),
            "kind" => space.kind = v.as_str().filter(|s| ["wiki", "knowledge_base"].contains(s)).ok_or_else(|| bad("kind must be wiki or knowledge_base"))?.to_string(),
            "archived" => space.archived = v.as_bool().ok_or_else(|| bad("archived must be true or false"))?,
            "workflow" => {
                if v.is_null() {
                    space.workflow = None;
                    sqlx::query("UPDATE pages SET stage = '' WHERE space_id = $1").bind(space.id).execute(&st.db).await?;
                } else {
                    let mut wf: crate::packs::WorkflowDef = serde_json::from_value(v.clone()).map_err(|e| bad(format!("workflow is not valid: {e}")))?;
                    wf.key = String::new();
                    wf.name = String::new();
                    wf.validate().map_err(bad)?;
                    if wf.transitions.iter().any(|t| !t.requires.is_empty()) {
                        return Err(bad("page workflows cannot require fields"));
                    }
                    // Pages keep their stage when it still exists; otherwise published pages go to
                    // the first publishing stage and drafts to the start.
                    let keys: Vec<String> = wf.statuses.iter().map(|s| s.key.clone()).collect();
                    let published = wf.first_in_category("done").map(|s| s.key.clone()).unwrap_or_default();
                    sqlx::query("UPDATE pages SET stage = CASE WHEN status = 'published' THEN $3 ELSE $4 END WHERE space_id = $1 AND NOT (stage = ANY($2))").bind(space.id).bind(&keys).bind(&published).bind(&wf.initial).execute(&st.db).await?;
                    // A stage whose category changed takes its pages along.
                    for s in &wf.statuses {
                        sqlx::query("UPDATE pages SET status = $3 WHERE space_id = $1 AND stage = $2 AND status <> $3").bind(space.id).bind(&s.key).bind(if s.category == "done" { "published" } else { "draft" }).execute(&st.db).await?;
                    }
                    space.workflow = Some(wf.definition());
                }
            }
            other => return Err(bad(format!("unknown field {other:?}"))),
        }
    }
    sqlx::query("UPDATE spaces SET workflow = $2 WHERE id = $1").bind(space.id).bind(&space.workflow).execute(&st.db).await?;
    sqlx::query("UPDATE spaces SET name=$2, description=$3, access=$4, kind=$5, archived=$6 WHERE id=$1").bind(space.id).bind(&space.name).bind(&space.description).bind(&space.access).bind(&space.kind).bind(space.archived).execute(&st.db).await?;
    audit::log(&st, &ctx, "space.update", "space", &space.key, json!({"changed": b.keys().collect::<Vec<_>>()})).await;
    let room = format!("space:{}", space.key);
    crate::realtime::publish(&st, &room, json!({"t": "event", "room": room, "kind": "space.updated", "data": {"space": space.key}})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn h_add_member(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(b): Json<MemberBody>) -> AppResult<(StatusCode, Json<Value>)> {
    let (space, perms) = load_space(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "space.admin", &format!("space:{}", space.key))?;
    let id = projects::add_binding(&st, &ctx, "space", space.id, &space.key, &b).await?;
    Ok((StatusCode::CREATED, Json(json!({"binding_id": id}))))
}

async fn h_remove_member(State(st): State<St>, ctx: Ctx, Path((key, binding)): Path<(String, Uuid)>) -> AppResult<StatusCode> {
    let (space, perms) = load_space(&st, &ctx, &key).await?;
    authz::require(&st, &ctx, &perms, "space.admin", &format!("space:{}", space.key))?;
    projects::remove_binding(&st, &ctx, "space", space.id, &space.key, binding).await?;
    Ok(StatusCode::NO_CONTENT)
}

#[derive(Debug, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NewPage {
    pub title: String,
    pub content: Option<String>,
    pub parent_id: Option<Uuid>,
    /// Key of a page template to start from (ignored when `content` is given).
    pub template: Option<String>,
    pub status: Option<String>,
    pub editor: Option<String>,
}

fn page_event<'a>(kind: &'a str, ctx: &'a Ctx, space: &Space, id: Uuid, title: &str, verb: &str) -> Event<'a> {
    Event { kind, project_id: None, space_id: Some(space.id), actor_id: Some(ctx.user_id), actor_name: &ctx.name, subject: title.to_string(), url: format!("/docs/{}/{id}", space.key), summary: format!("{} {verb} the page \"{title}\" in {}", ctx.name, space.name), data: json!({"page_id": id, "space": space.key}), notify: vec![], item: None, page: Some((id, space.key.clone())), excerpt: String::new(), skip: vec![] }
}

/// Tell people newly mentioned on a published page, if they can read the space.
async fn notify_mentions(st: &St, ctx: &Ctx, space: &Space, id: Uuid, title: &str, before: &Value, after: &Value, text: &str) -> Vec<Uuid> {
    let old = crate::refs::mentioned(before);
    let new: Vec<Uuid> = crate::refs::mentioned(after).into_iter().filter(|u| !old.contains(u) && *u != ctx.user_id).collect();
    if !new.is_empty() {
        let names: Vec<String> = sqlx::query_scalar("SELECT name FROM users WHERE id = ANY($1) ORDER BY name").bind(&new).fetch_all(&st.db).await.unwrap_or_default();
        let whom = if names.len() == 1 { names[0].clone() } else { format!("{} people", names.len()) };
        let mut e = page_event("mention.created", ctx, space, id, title, "updated");
        e.summary = format!("{} mentioned {whom} on the page \"{title}\"", ctx.name);
        e.notify = new.clone();
        // A few lines around the mention are more useful than the top of a long page.
        e.excerpt = text.lines().filter(|l| l.contains('@')).take(6).collect::<Vec<_>>().join("\n");
        events::emit(st, e).await;
    }
    new
}

/// A "this changed" hint for people who have the page or its space open.
async fn hint(st: &AppState, space: &Space, id: Uuid, kind: &str) {
    for room in [format!("page:{id}"), format!("space:{}", space.key)] {
        crate::realtime::publish(st, &room, json!({"t": "event", "room": room, "kind": kind, "data": {"page_id": id}})).await;
    }
}

pub async fn create_page(st: &St, ctx: &Ctx, space_key: &str, n: NewPage) -> AppResult<Value> {
    let (space, perms) = load_space(st, ctx, space_key).await?;
    authz::require(st, ctx, &perms, "page.edit", &format!("space:{}", space.key))?;
    let title = check_title(&n.title)?;
    let stages = space.stages();
    // In a space with a page workflow, a new page starts at the first stage.
    let status = match (&stages, n.status.as_deref()) {
        (Some(wf), requested) => {
            let s = status_of(wf, &wf.initial);
            if requested.is_some_and(|r| r != s) {
                return Err(bad(format!("pages in this space start as {s}: the space's workflow begins at \"{}\"", wf.status(&wf.initial).map(|x| x.name.as_str()).unwrap_or(&wf.initial))));
            }
            s
        }
        (None, requested) => requested.unwrap_or("published"),
    };
    if !["draft", "published"].contains(&status) {
        return Err(bad("status must be draft or published"));
    }
    let content = match (&n.content, &n.template) {
        (Some(c), _) => c.clone(),
        (None, Some(t)) => sqlx::query_scalar("SELECT body FROM page_templates WHERE key = $1").bind(t).fetch_optional(&st.db).await?.ok_or_else(|| bad("unknown page template"))?,
        (None, None) => String::new(),
    };
    check_content(&content)?;
    if let Some(parent) = n.parent_id {
        let ok: bool = sqlx::query_scalar("SELECT EXISTS (SELECT 1 FROM pages WHERE id = $1 AND space_id = $2 AND NOT archived)").bind(parent).bind(space.id).fetch_one(&st.db).await?;
        if !ok {
            return Err(bad("parent page must be in the same space"));
        }
    }
    let position: i32 = sqlx::query_scalar("SELECT coalesce(max(position), -1) + 1 FROM pages WHERE space_id = $1 AND parent_id IS NOT DISTINCT FROM $2").bind(space.id).bind(n.parent_id).fetch_one(&st.db).await?;
    let id = create_page_raw(st, ctx.user_id, space.id, n.parent_id, title, &content, status, false, position).await?;
    if let Some(e) = n.editor.as_deref().filter(|e| ["simple", "advanced"].contains(e)) {
        sqlx::query("UPDATE pages SET editor = $2 WHERE id = $1").bind(id).bind(e).execute(&st.db).await?;
    }
    let refs = crate::refs::check(st, ctx, &content, Some(space.id)).await?;
    sqlx::query("UPDATE pages SET refs = $2, stage = $3 WHERE id = $1").bind(id).bind(&refs).bind(stages.as_ref().map(|w| w.initial.as_str()).unwrap_or("")).execute(&st.db).await?;
    audit::log(st, ctx, "page.create", "page", &id.to_string(), json!({"space": space.key, "title": title, "status": status})).await;
    if status == "published" {
        let mentioned = notify_mentions(st, ctx, &space, id, title, &json!({}), &refs, &content).await;
        let mut e = page_event("page.created", ctx, &space, id, title, "created");
        e.skip = mentioned;
        events::emit(st, e).await;
    } else {
        hint(st, &space, id, "page.created").await;
    }
    page_json(st, ctx, id).await
}

pub async fn page_json(st: &St, ctx: &Ctx, id: Uuid) -> AppResult<Value> {
    let (page, space, perms) = load_page(st, ctx, id).await?;
    let meta = sqlx::query(
        "SELECT pg.created_at, pg.updated_at, (SELECT name FROM users WHERE id = pg.created_by) AS created_by, (SELECT name FROM users WHERE id = pg.updated_by) AS updated_by,
                (SELECT person_json(u) FROM users u WHERE u.id = pg.created_by) AS created_by_user, (SELECT person_json(u) FROM users u WHERE u.id = pg.updated_by) AS updated_by_user,
                (WITH RECURSIVE up(id, parent_id, title, depth) AS (
                    SELECT id, parent_id, title, 0 FROM pages WHERE id = pg.parent_id
                  UNION ALL SELECT p.id, p.parent_id, p.title, up.depth + 1 FROM pages p JOIN up ON p.id = up.parent_id WHERE up.depth < 30)
                 SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'title', title) ORDER BY depth DESC), '[]'::jsonb) FROM up) AS breadcrumbs,
                (SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'filename', filename, 'size', size, 'content_type', content_type) ORDER BY created_at), '[]'::jsonb)
                 FROM attachments WHERE owner_type = 'page' AND owner_id = pg.id) AS attachments
         FROM pages pg WHERE pg.id = $1",
    )
    .bind(id)
    .fetch_one(&st.db)
    .await?;
    // Work items that link to this page, limited to projects the caller can see.
    let visible = authz::visible_project_ids(st, ctx).await?;
    let backlinks: Value = sqlx::query_scalar(
        "SELECT coalesce(jsonb_agg(jsonb_build_object('key', p.key || '-' || i.number, 'title', i.title, 'rel', l.rel, 'status_category', i.status_category)), '[]'::jsonb)
         FROM links l JOIN items i ON l.src_type = 'item' AND i.id::text = l.src_id JOIN projects p ON p.id = i.project_id
         WHERE l.dst_type = 'page' AND l.dst_id = $1 AND i.project_id = ANY($2) AND NOT i.archived",
    )
    .bind(id.to_string())
    .bind(&visible)
    .fetch_one(&st.db)
    .await?;
    let perm_list: Vec<String> = perms.list().into_iter().filter(|p| ctx.token_allows(p) && authz::PERMISSIONS.iter().any(|(k, s, _)| k == p && *s == "space")).collect();
    let refs = crate::refs::resolve(st, ctx, &[&page.refs], crate::refs::People::Directory).await?.pop().unwrap_or(json!({}));
    // Where the page is in the space's workflow, and where it may go next.
    let stage = space.stages().map(|wf| {
        let next: Vec<Value> = wf.available(&page.stage).iter().map(|t| json!({"to": t.to, "name": t.name, "to_name": wf.status(&t.to).map(|s| s.name.as_str()), "publishes": wf.status(&t.to).is_some_and(|s| s.category == "done")})).collect();
        json!({"key": page.stage, "name": wf.status(&page.stage).map(|s| s.name.as_str()).unwrap_or(&page.stage), "next": next,
               "stages": wf.statuses.iter().map(|s| json!({"key": s.key, "name": s.name, "published": s.category == "done"})).collect::<Vec<_>>()})
    });
    Ok(json!({
        "id": page.id, "space": {"key": space.key, "name": space.name, "kind": space.kind}, "parent_id": page.parent_id,
        "refs": refs, "stage": stage,
        "created_by_user": meta.get::<Option<Value>, _>("created_by_user"), "updated_by_user": meta.get::<Option<Value>, _>("updated_by_user"),
        "title": page.title, "content": page.content, "version": page.version, "status": page.status,
        "customer_visible": page.customer_visible, "labels": page.labels, "editor": page.editor,
        "created_at": meta.get::<chrono::DateTime<chrono::Utc>, _>("created_at"), "updated_at": meta.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"),
        "created_by": meta.get::<Option<String>, _>("created_by"), "updated_by": meta.get::<Option<String>, _>("updated_by"),
        "breadcrumbs": meta.get::<Value, _>("breadcrumbs"), "attachments": meta.get::<Value, _>("attachments"),
        "backlinks": backlinks, "permissions": perm_list,
    }))
}

pub async fn update_page(st: &St, ctx: &Ctx, id: Uuid, changes: Map<String, Value>) -> AppResult<Value> {
    let (page, space, perms) = load_page(st, ctx, id).await?;
    authz::require(st, ctx, &perms, "page.edit", &format!("page:{id}"))?;
    let mut p = page.clone();
    let mut message = String::new();
    let mut base_version: Option<i64> = None;
    let mut from_collab = false;
    let stages = space.stages();
    for (k, v) in &changes {
        match k.as_str() {
            "title" => p.title = check_title(v.as_str().ok_or_else(|| bad("title must be text"))?)?.to_string(),
            "content" => {
                p.content = v.as_str().ok_or_else(|| bad("content must be text"))?.to_string();
                check_content(&p.content)?;
            }
            "base_version" => base_version = Some(v.as_i64().ok_or_else(|| bad("base_version must be a number"))?),
            "message" => message = v.as_str().unwrap_or_default().chars().take(300).collect(),
            "status" => {
                let s = v.as_str().filter(|s| ["draft", "published"].contains(s)).ok_or_else(|| bad("status must be draft or published"))?;
                if stages.is_some() && s != page.status {
                    return Err(bad("this space has a page workflow; move the page to another stage instead of setting its status"));
                }
                p.status = s.to_string();
            }
            "stage" => {
                let wf = stages.as_ref().ok_or_else(|| bad("this space has no page workflow"))?;
                let to = v.as_str().ok_or_else(|| bad("stage must be text"))?;
                if to != page.stage {
                    // Space administrators may put a page anywhere; everyone else follows the arrows.
                    if wf.status(to).is_none() || (wf.transition(&page.stage, to).is_none() && !perms.has("space.admin")) {
                        let allowed: Vec<&str> = wf.available(&page.stage).iter().map(|t| t.to.as_str()).collect();
                        return Err(bad(format!("a page cannot move from {:?} to {to:?}; allowed next stages: {}", page.stage, if allowed.is_empty() { "none".into() } else { allowed.join(", ") })));
                    }
                    p.stage = to.to_string();
                    p.status = status_of(wf, to).to_string();
                }
            }
            // Sent with content that came out of a shared editing session.
            "collab" => from_collab = v == &json!(true),
            "customer_visible" => p.customer_visible = v.as_bool().ok_or_else(|| bad("customer_visible must be true or false"))?,
            "labels" => p.labels = items::clean_labels(&serde_json::from_value::<Vec<String>>(v.clone()).map_err(|_| bad("labels must be a list of text"))?)?,
            "editor" => p.editor = v.as_str().filter(|s| ["simple", "advanced"].contains(s)).ok_or_else(|| bad("editor must be simple or advanced"))?.to_string(),
            "parent_id" => {
                p.parent_id = if v.is_null() { None } else { Some(v.as_str().and_then(|s| s.parse().ok()).ok_or_else(|| bad("parent_id must be a page id"))?) };
                if let Some(parent) = p.parent_id {
                    let bad_parent: bool = sqlx::query_scalar(
                        "WITH RECURSIVE up(id, parent_id, depth) AS (SELECT id, parent_id, 0 FROM pages WHERE id = $1 AND space_id = $3 AND NOT archived
                           UNION ALL SELECT pg.id, pg.parent_id, up.depth + 1 FROM pages pg JOIN up ON pg.id = up.parent_id WHERE up.depth < 50)
                         SELECT NOT EXISTS (SELECT 1 FROM up) OR EXISTS (SELECT 1 FROM up WHERE id = $2)",
                    )
                    .bind(parent)
                    .bind(id)
                    .bind(space.id)
                    .fetch_one(&st.db)
                    .await?;
                    if bad_parent {
                        return Err(bad("parent must be another page in the same space that is not beneath this page"));
                    }
                }
            }
            "position" => {
                let pos = v.as_i64().filter(|n| (0..100_000).contains(n)).ok_or_else(|| bad("position must be a non-negative number"))? as i32;
                sqlx::query("UPDATE pages SET position = $2 WHERE id = $1").bind(id).bind(pos).execute(&st.db).await?;
            }
            other => return Err(bad(format!("unknown field {other:?}"))),
        }
    }
    let body_changed = p.title != page.title || p.content != page.content;
    if body_changed {
        // Optimistic concurrency: the caller must say which version their edit was based on.
        let base = base_version.ok_or_else(|| bad("base_version is required when changing title or content"))?;
        if base != page.version as i64 {
            return Err(AppError::Conflict(format!("this page was changed by someone else and is now at version {}; reload, merge your changes and save again", page.version)));
        }
    }
    if p.editor != page.editor {
        // The two editors keep their working copies in different forms; switching starts a new one.
        crate::realtime::reset_doc(st, &format!("page:{id}"), "content").await;
    }
    let refs = if p.content != page.content { crate::refs::check(st, ctx, &p.content, Some(space.id)).await? } else { page.refs.clone() };
    let mut tx = st.db.begin().await?;
    let new_version = if body_changed { page.version + 1 } else { page.version };
    let updated = sqlx::query("UPDATE pages SET title=$2, content=$3, version=$4, status=$5, customer_visible=$6, labels=$7, editor=$8, parent_id=$9, updated_by=$10, updated_at=now(), stage=$12, refs=$13 WHERE id=$1 AND version=$11")
        .bind(id)
        .bind(&p.title)
        .bind(&p.content)
        .bind(new_version)
        .bind(&p.status)
        .bind(p.customer_visible)
        .bind(&p.labels)
        .bind(&p.editor)
        .bind(p.parent_id)
        .bind(ctx.user_id)
        .bind(page.version)
        .bind(&p.stage)
        .bind(&refs)
        .execute(&mut *tx)
        .await?
        .rows_affected();
    if updated == 0 {
        return Err(AppError::Conflict("this page was changed by someone else while you were saving; reload and try again".into()));
    }
    if body_changed {
        sqlx::query("INSERT INTO page_versions (page_id, version, title, content, author_id, message) VALUES ($1,$2,$3,$4,$5,$6)").bind(id).bind(new_version).bind(&p.title).bind(&p.content).bind(ctx.user_id).bind(&message).execute(&mut *tx).await?;
    }
    tx.commit().await?;
    index_page(st, id).await;
    let changed: Vec<&String> = changes.keys().filter(|k| !["base_version", "message", "collab"].contains(&k.as_str())).collect();
    audit::log(st, ctx, "page.update", "page", &id.to_string(), json!({"space": space.key, "title": p.title, "changed": changed, "version": new_version, "stage": (p.stage != page.stage).then_some(&p.stage)})).await;
    if p.content != page.content {
        let room = format!("page:{id}");
        if from_collab {
            // The people editing together keep their working copy; it now matches this version.
            crate::realtime::mark_saved(st, &room, "content", new_version as i64).await;
        } else {
            // Saved by another route (the API, a restore): open editors must start from it.
            crate::realtime::reset_doc(st, &room, "content").await;
        }
    }
    if p.status == "published" && (body_changed || page.status != "published") {
        // Someone mentioned on a draft hears about it when the page is published.
        let before = if page.status == "published" { page.refs.clone() } else { json!({}) };
        let mentioned = notify_mentions(st, ctx, &space, id, &p.title, &before, &refs, &p.content).await;
        let mut e = page_event("page.updated", ctx, &space, id, &p.title, if page.status == "published" { "updated" } else { "published" });
        e.skip = mentioned;
        events::emit(st, e).await;
    } else {
        hint(st, &space, id, "page.updated").await;
    }
    page_json(st, ctx, id).await
}

async fn h_create_page(State(st): State<St>, ctx: Ctx, Path(key): Path<String>, Json(n): Json<NewPage>) -> AppResult<(StatusCode, Json<Value>)> {
    Ok((StatusCode::CREATED, Json(create_page(&st, &ctx, &key, n).await?)))
}

async fn h_get_page(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    Ok(Json(page_json(&st, &ctx, id).await?))
}

async fn h_update_page(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(changes): Json<Map<String, Value>>) -> AppResult<Json<Value>> {
    Ok(Json(update_page(&st, &ctx, id, changes).await?))
}

async fn h_archive_page(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    let (page, space, perms) = load_page(&st, &ctx, id).await?;
    authz::require(&st, &ctx, &perms, "page.delete", &format!("page:{id}"))?;
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "WITH RECURSIVE down(id, depth) AS (SELECT id, 0 FROM pages WHERE id = $1
           UNION ALL SELECT pg.id, down.depth + 1 FROM pages pg JOIN down ON pg.parent_id = down.id WHERE down.depth < 50)
         UPDATE pages SET archived = true, updated_at = now() WHERE id IN (SELECT id FROM down) RETURNING id",
    )
    .bind(id)
    .fetch_all(&st.db)
    .await?;
    for pid in &ids {
        index_page(&st, *pid).await;
    }
    audit::log(&st, &ctx, "page.archive", "page", &id.to_string(), json!({"space": space.key, "title": page.title, "pages_archived": ids.len()})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn h_versions(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    load_page(&st, &ctx, id).await?;
    Ok(Json(
        sqlx::query_scalar(
            "SELECT coalesce(jsonb_agg(jsonb_build_object('version', v.version, 'title', v.title, 'message', v.message, 'created_at', v.created_at,
                'author', (SELECT name FROM users WHERE id = v.author_id), 'author_user', (SELECT person_json(u) FROM users u WHERE u.id = v.author_id), 'size', length(v.content)) ORDER BY v.version DESC), '[]'::jsonb)
             FROM page_versions v WHERE v.page_id = $1",
        )
        .bind(id)
        .fetch_one(&st.db)
        .await?,
    ))
}

async fn h_version(State(st): State<St>, ctx: Ctx, Path((id, version)): Path<(Uuid, i32)>) -> AppResult<Json<Value>> {
    load_page(&st, &ctx, id).await?;
    let v: Option<Value> = sqlx::query_scalar("SELECT jsonb_build_object('version', version, 'title', title, 'content', content, 'message', message, 'created_at', created_at, 'author', (SELECT name FROM users WHERE id = author_id)) FROM page_versions WHERE page_id = $1 AND version = $2").bind(id).bind(version).fetch_optional(&st.db).await?;
    Ok(Json(v.ok_or(AppError::NotFound)?))
}

async fn h_restore(State(st): State<St>, ctx: Ctx, Path((id, version)): Path<(Uuid, i32)>) -> AppResult<Json<Value>> {
    let (page, _, _) = load_page(&st, &ctx, id).await?;
    let old = sqlx::query("SELECT title, content FROM page_versions WHERE page_id = $1 AND version = $2").bind(id).bind(version).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let mut changes = Map::new();
    changes.insert("title".into(), json!(old.get::<String, _>("title")));
    changes.insert("content".into(), json!(old.get::<String, _>("content")));
    changes.insert("base_version".into(), json!(page.version));
    changes.insert("message".into(), json!(format!("Restored version {version}")));
    Ok(Json(update_page(&st, &ctx, id, changes).await?))
}

async fn h_templates(State(st): State<St>, _ctx: Ctx) -> AppResult<Json<Value>> {
    Ok(Json(sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('key', key, 'name', name, 'category', category, 'description', description, 'source', source) ORDER BY category, name), '[]'::jsonb) FROM page_templates").fetch_one(&st.db).await?))
}

async fn h_template(State(st): State<St>, _ctx: Ctx, Path(key): Path<String>) -> AppResult<Json<Value>> {
    let t: Option<Value> = sqlx::query_scalar("SELECT jsonb_build_object('key', key, 'name', name, 'category', category, 'description', description, 'body', body) FROM page_templates WHERE key = $1").bind(key).fetch_optional(&st.db).await?;
    Ok(Json(t.ok_or(AppError::NotFound)?))
}

#[derive(Deserialize)]
struct UploadQuery {
    filename: String,
}

async fn h_page_attach(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Query(q): Query<UploadQuery>, headers: HeaderMap, body: Bytes) -> AppResult<(StatusCode, Json<Value>)> {
    let (_, _, perms) = load_page(&st, &ctx, id).await?;
    authz::require(&st, &ctx, &perms, "page.edit", &format!("page:{id}"))?;
    let ct = headers.get(header::CONTENT_TYPE).and_then(|v| v.to_str().ok()).unwrap_or("application/octet-stream");
    let out = items::store_attachment(&st, "page", id, &q.filename, ct, body, ctx.user_id).await?;
    audit::log(&st, &ctx, "attachment.upload", "page", &id.to_string(), out.clone()).await;
    Ok((StatusCode::CREATED, Json(out)))
}

async fn h_list_spaces(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    Ok(Json(list_spaces(&st, &ctx).await?))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/spaces", "Docs", "List documentation spaces visible to the caller", get(h_list_spaces))
        .route("POST", "/api/v1/spaces", "Docs", "Create a space", post(h_create_space))
        .route("GET", "/api/v1/spaces/{key}", "Docs", "Space with its page tree", get(h_get_space))
        .route("PATCH", "/api/v1/spaces/{key}", "Docs", "Update space settings", patch(h_update_space))
        .route("POST", "/api/v1/spaces/{key}/members", "Docs", "Grant a space role to a user or group", post(h_add_member))
        .route("DELETE", "/api/v1/spaces/{key}/members/{binding}", "Docs", "Remove a space role grant", delete(h_remove_member))
        .route("POST", "/api/v1/spaces/{key}/pages", "Docs", "Create a page, optionally from a template", post(h_create_page))
        .route("GET", "/api/v1/pages/{id}", "Docs", "Page content and metadata", get(h_get_page))
        .route("PATCH", "/api/v1/pages/{id}", "Docs", "Save a page (creates a version when title or content change)", patch(h_update_page))
        .route("DELETE", "/api/v1/pages/{id}", "Docs", "Archive a page and its children", delete(h_archive_page))
        .route("GET", "/api/v1/pages/{id}/versions", "Docs", "Version history", get(h_versions))
        .route("GET", "/api/v1/pages/{id}/versions/{version}", "Docs", "One historical version", get(h_version))
        .route("POST", "/api/v1/pages/{id}/versions/{version}/restore", "Docs", "Restore a historical version as the newest", post(h_restore))
        .route("POST", "/api/v1/pages/{id}/attachments", "Attachments", "Upload a file to a page (raw body, ?filename=)", post(h_page_attach))
        .route("GET", "/api/v1/page-templates", "Docs", "List page templates", get(h_templates))
        .route("GET", "/api/v1/page-templates/{key}", "Docs", "A page template with its body", get(h_template))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titles_and_content_limits() {
        assert_eq!(check_title("  Runbook  ").unwrap(), "Runbook");
        assert!(check_title("   ").is_err());
        assert!(check_title(&"x".repeat(301)).is_err());
        assert!(check_content(&"x".repeat(2_000_001)).is_err());
        assert!(check_content("ok").is_ok());
    }
}
