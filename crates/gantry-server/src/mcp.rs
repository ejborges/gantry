//! Model Context Protocol server at `POST /mcp`, for AI agents and MCP-capable clients.
//!
//! Serves two protocol eras on one endpoint:
//! * **2026-07-28** (stateless): each request carries `_meta["io.modelcontextprotocol/protocolVersion"]`
//!   and the `MCP-Protocol-Version` / `Mcp-Method` / `Mcp-Name` headers, which are checked against the body.
//! * **2025-11-25, 2025-06-18, 2025-03-26** (legacy): `initialize` handshake, no header checks.
//!
//! Authentication is an API token (`Authorization: Bearer gty_…`). Tools run with the token owner's
//! permissions, narrowed by the token's scopes, through the same functions as the REST API, so an
//! agent can never see or do more than its account. Every tool call is written to the audit log.

use crate::auth::Ctx;
use crate::error::AppError;
use crate::http::{Api, ReqInfo};
use crate::items::{self, ItemFilter, LinkBody, NewItem};
use crate::wiki::{self, NewPage};
use crate::{audit, projects, search, St};
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{header, HeaderMap, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::Json;
use base64::Engine;
use serde_json::{json, Map, Value};
use uuid::Uuid;

pub const MODERN: &str = "2026-07-28";
pub const LEGACY: &[&str] = &["2025-11-25", "2025-06-18", "2025-03-26"];
const META_VERSION: &str = "io.modelcontextprotocol/protocolVersion";
const INSTRUCTIONS: &str = "Gantry tracks work items (keys like OPS-12) in projects and documentation pages in spaces. \
Use `search` to find things, `list_projects` and `get_project` to learn a project's item types, fields and workflow before creating or transitioning items. \
Status changes go through `transition_work_item` and must follow the workflow; `get_work_item` lists the transitions currently allowed. \
Page edits need the `base_version` returned by `get_page`.";

struct Tool {
    name: &'static str,
    title: &'static str,
    description: &'static str,
    schema: fn() -> Value,
    read_only: bool,
}

/// The permission a tool needs. A caller whose rules (an AI agent's policy, a token's scopes)
/// rule the permission out is not offered the tool at all, so an agent is not led to try
/// things it will be refused. The check that matters is still made when the tool is called.
fn tool_permission(name: &str) -> &'static str {
    match name {
        "search" => "workspace.access",
        "list_projects" | "get_project" | "list_work_items" | "get_work_item" | "get_item_graph" => "project.view",
        "create_work_item" => "item.create",
        "update_work_item" => "item.edit",
        "transition_work_item" => "item.transition",
        "add_comment" => "item.comment",
        "link_work_items" => "item.link",
        "list_spaces" | "get_space" | "get_page" => "space.view",
        "create_page" | "update_page" => "page.edit",
        _ => "admin.settings",
    }
}

/// The tools this caller may be offered.
pub(crate) fn tools_for(ctx: &Ctx) -> Vec<Value> {
    let allowed = |t: &&Tool| {
        let p = tool_permission(t.name);
        ctx.token_allows(p) && ctx.policy.as_ref().is_none_or(|policy| policy.allows(p))
    };
    TOOLS.iter().filter(allowed).map(tool_json).collect()
}

fn tool_json(t: &Tool) -> Value {
    json!({"name": t.name, "title": t.title, "description": t.description, "inputSchema": (t.schema)(),
        "annotations": {"readOnlyHint": t.read_only, "destructiveHint": false, "idempotentHint": t.read_only, "openWorldHint": false}})
}

fn obj(props: Value, required: &[&str]) -> Value {
    json!({"type": "object", "properties": props, "required": required, "additionalProperties": false})
}

const TOOLS: &[Tool] = &[
    Tool { name: "search", title: "Search", description: "Search work items and documentation pages visible to you. Returns keys, titles, links and snippets.", read_only: true,
        schema: || obj(json!({"query": {"type": "string"}, "kind": {"type": "string", "enum": ["item", "page"], "description": "Restrict to one kind"}, "limit": {"type": "integer", "minimum": 1, "maximum": 50}}), &["query"]) },
    Tool { name: "list_projects", title: "List projects", description: "List the projects you can see, with open item counts.", read_only: true, schema: || obj(json!({}), &[]) },
    Tool { name: "get_project", title: "Get project", description: "A project's work item types, each with its custom fields and workflow (statuses and transitions), plus cycles and members.", read_only: true,
        schema: || obj(json!({"project": {"type": "string", "description": "Project key, e.g. OPS"}}), &["project"]) },
    Tool { name: "list_work_items", title: "List work items", description: "List work items with filters. All filters are optional and combine with AND.", read_only: true,
        schema: || obj(json!({
            "project": {"type": "string"}, "status": {"type": "string", "description": "Status key(s), comma separated"},
            "category": {"type": "string", "description": "todo, in_progress, done (comma separated)"}, "type": {"type": "string"},
            "assignee": {"type": "string", "description": "me, none, or a user id"}, "label": {"type": "string"}, "q": {"type": "string", "description": "Text in the title"},
            "overdue": {"type": "boolean"}, "sort": {"type": "string", "enum": ["updated", "created", "priority", "due", "rank", "key", "sla"]},
            "limit": {"type": "integer", "minimum": 1, "maximum": 200}, "offset": {"type": "integer", "minimum": 0}}), &[]) },
    Tool { name: "get_work_item", title: "Get work item", description: "One work item with description, custom fields, comments, links, history and the transitions allowed from its current status.", read_only: true,
        schema: || obj(json!({"key": {"type": "string", "description": "Item key, e.g. OPS-12"}}), &["key"]) },
    Tool { name: "create_work_item", title: "Create work item", description: "Create a work item. Use get_project first to learn valid types and custom field keys.", read_only: false,
        schema: || obj(json!({
            "project": {"type": "string"}, "title": {"type": "string"}, "type": {"type": "string", "description": "Item type key; defaults to the project's default"},
            "description": {"type": "string", "description": "Markdown"}, "priority": {"type": "integer", "minimum": 0, "maximum": 4, "description": "0 none, 1 low, 2 medium, 3 high, 4 critical"},
            "assignee_id": {"type": "string"}, "parent": {"type": "string", "description": "Key of the parent item"}, "labels": {"type": "array", "items": {"type": "string"}},
            "due_date": {"type": "string", "format": "date"}, "custom": {"type": "object", "description": "Custom field values by field key"}}), &["project", "title"]) },
    Tool { name: "update_work_item", title: "Update work item", description: "Change fields of a work item. Only the fields given are changed. Status is changed with transition_work_item.", read_only: false,
        schema: || obj(json!({"key": {"type": "string"}, "changes": {"type": "object", "description": "Any of: title, description, priority, assignee_id, labels, start_date, due_date, estimate, parent, cycle_id, custom"}}), &["key", "changes"]) },
    Tool { name: "transition_work_item", title: "Transition work item", description: "Move a work item to another status. The move must be one of the transitions listed by get_work_item.", read_only: false,
        schema: || obj(json!({"key": {"type": "string"}, "to": {"type": "string", "description": "Target status key"}}), &["key", "to"]) },
    Tool { name: "add_comment", title: "Add comment", description: "Comment on a work item (Markdown). Set internal=true for a service desk note hidden from the customer.", read_only: false,
        schema: || obj(json!({"key": {"type": "string"}, "body": {"type": "string"}, "internal": {"type": "boolean"}}), &["key", "body"]) },
    Tool { name: "link_work_items", title: "Link work items", description: "Create a typed link from one work item to another item or to a page (e.g. blocks, relates_to, verifies, derives_from, mitigates).", read_only: false,
        schema: || obj(json!({"key": {"type": "string"}, "rel": {"type": "string"}, "target_key": {"type": "string", "description": "Key of the target item"}, "target_page_id": {"type": "string", "description": "Id of the target page (instead of target_key)"}}), &["key", "rel"]) },
    Tool { name: "get_item_graph", title: "Get item graph", description: "The graph of links around a work item (dependencies, traceability), up to 4 hops.", read_only: true,
        schema: || obj(json!({"key": {"type": "string"}, "depth": {"type": "integer", "minimum": 1, "maximum": 4}}), &["key"]) },
    Tool { name: "list_spaces", title: "List spaces", description: "List documentation spaces and knowledge bases you can see.", read_only: true, schema: || obj(json!({}), &[]) },
    Tool { name: "get_space", title: "Get space", description: "A space with its page tree (ids and titles).", read_only: true, schema: || obj(json!({"space": {"type": "string", "description": "Space key"}}), &["space"]) },
    Tool { name: "get_page", title: "Get page", description: "A page's Markdown content, version and metadata.", read_only: true, schema: || obj(json!({"id": {"type": "string"}}), &["id"]) },
    Tool { name: "create_page", title: "Create page", description: "Create a documentation page (Markdown).", read_only: false,
        schema: || obj(json!({"space": {"type": "string"}, "title": {"type": "string"}, "content": {"type": "string"}, "parent_id": {"type": "string"}, "template": {"type": "string", "description": "Page template key to start from"}, "status": {"type": "string", "enum": ["draft", "published"]}}), &["space", "title"]) },
    Tool { name: "update_page", title: "Update page", description: "Replace a page's title and/or content. base_version must be the version you read; the call fails if someone else saved in between.", read_only: false,
        schema: || obj(json!({"id": {"type": "string"}, "base_version": {"type": "integer"}, "title": {"type": "string"}, "content": {"type": "string"}, "message": {"type": "string", "description": "Short note for the version history"}}), &["id", "base_version"]) },
];

pub(crate) fn tool_list() -> Vec<Value> {
    TOOLS.iter().map(tool_json).collect()
}

fn server_info() -> Value {
    json!({"name": "gantry", "title": "Gantry", "version": crate::VERSION})
}

fn arg_str<'a>(a: &'a Map<String, Value>, k: &str) -> Result<&'a str, AppError> {
    a.get(k).and_then(Value::as_str).ok_or_else(|| AppError::BadRequest(format!("argument {k:?} is required")))
}

fn arg_uuid(a: &Map<String, Value>, k: &str) -> Result<Uuid, AppError> {
    arg_str(a, k)?.parse().map_err(|_| AppError::BadRequest(format!("argument {k:?} must be an id")))
}

async fn call_tool(st: &St, ctx: &Ctx, name: &str, a: &Map<String, Value>) -> Result<Value, AppError> {
    fn from<T: serde::de::DeserializeOwned>(v: Value) -> Result<T, AppError> {
        serde_json::from_value(v).map_err(|e| AppError::BadRequest(format!("invalid arguments: {e}")))
    }
    match name {
        "search" => {
            let kinds = a.get("kind").and_then(Value::as_str).map(|k| vec![k.to_string()]).unwrap_or_default();
            let hits = search::search(st, ctx, arg_str(a, "query")?, kinds, None, a.get("limit").and_then(Value::as_i64).unwrap_or(10)).await?;
            Ok(json!({"results": hits.iter().map(search::hit_json).map(|mut h| { h.as_object_mut().map(|o| o.remove("snippet_parts")); h }).collect::<Vec<_>>()}))
        }
        "list_projects" => {
            let visible = crate::authz::visible_project_ids(st, ctx).await?;
            Ok(sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('key', p.key, 'name', p.name, 'kind', p.kind, 'description', p.description,
                    'open_items', (SELECT count(*) FROM items i WHERE i.project_id = p.id AND NOT i.archived AND i.status_category <> 'done')) ORDER BY p.key), '[]'::jsonb)
                 FROM projects p WHERE p.id = ANY($1)").bind(&visible).fetch_one(&st.db).await?)
        }
        "get_project" => {
            let (project, _) = projects::load_project(st, ctx, arg_str(a, "project")?).await?;
            let cycles: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'name', name, 'state', state)), '[]'::jsonb) FROM cycles WHERE project_id = $1").bind(project.id).fetch_one(&st.db).await?;
            Ok(json!({"key": project.key, "name": project.name, "kind": project.kind, "description": project.description, "default_type": project.default_type,
                      "types": projects::project_types_json(st, project.id).await?, "cycles": cycles}))
        }
        "list_work_items" => {
            let mut f: ItemFilter = from(Value::Object(a.clone()))?;
            f.limit = Some(f.limit.unwrap_or(50).clamp(1, 200));
            items::list_items(st, ctx, &f).await
        }
        "get_work_item" => {
            let (item, project, perms) = items::load_item(st, ctx, arg_str(a, "key")?).await?;
            items::detail_json(st, ctx, item.id, &project, &perms).await
        }
        "create_work_item" => {
            let mut body = a.clone();
            let project = body.remove("project").and_then(|v| v.as_str().map(str::to_string)).ok_or_else(|| AppError::BadRequest("argument \"project\" is required".into()))?;
            let n: NewItem = from(Value::Object(body))?;
            let v = items::create_item_checked(st, ctx, &project, n, "mcp").await?;
            Ok(json!({"key": v["key"], "status": v["status"], "url": format!("{}/i/{}", st.cfg.server.public_url.trim_end_matches('/'), v["key"].as_str().unwrap_or_default())}))
        }
        "update_work_item" => {
            let changes = a.get("changes").and_then(Value::as_object).cloned().ok_or_else(|| AppError::BadRequest("argument \"changes\" must be an object".into()))?;
            let v = items::update_item(st, ctx, arg_str(a, "key")?, changes).await?;
            Ok(json!({"key": v["key"], "updated_at": v["updated_at"]}))
        }
        "transition_work_item" => {
            let v = items::transition_item(st, ctx, arg_str(a, "key")?, arg_str(a, "to")?).await?;
            Ok(json!({"key": v["key"], "status": v["status"], "status_category": v["status_category"], "transitions": v["transitions"]}))
        }
        "add_comment" => items::add_comment(st, ctx, arg_str(a, "key")?, arg_str(a, "body")?, a.get("internal").and_then(Value::as_bool).unwrap_or(false)).await,
        "link_work_items" => {
            let target = match (a.get("target_key").and_then(Value::as_str), a.get("target_page_id").and_then(Value::as_str)) {
                (Some(k), None) => json!(k),
                (None, Some(p)) => json!({"type": "page", "id": p}),
                _ => return Err(AppError::BadRequest("give exactly one of target_key or target_page_id".into())),
            };
            items::add_link(st, ctx, arg_str(a, "key")?, &LinkBody { rel: arg_str(a, "rel")?.to_string(), target }).await
        }
        "get_item_graph" => items::item_graph(st, ctx, arg_str(a, "key")?, a.get("depth").and_then(Value::as_i64).unwrap_or(2) as i32).await,
        "list_spaces" => wiki::list_spaces(st, ctx).await,
        "get_space" => {
            let mut v = wiki::space_json(st, ctx, arg_str(a, "space")?).await?;
            v.as_object_mut().map(|o| o.remove("members"));
            Ok(v)
        }
        "get_page" => wiki::page_json(st, ctx, arg_uuid(a, "id")?).await,
        "create_page" => {
            let mut body = a.clone();
            let space = body.remove("space").and_then(|v| v.as_str().map(str::to_string)).ok_or_else(|| AppError::BadRequest("argument \"space\" is required".into()))?;
            let v = wiki::create_page(st, ctx, &space, from::<NewPage>(Value::Object(body))?).await?;
            Ok(json!({"id": v["id"], "version": v["version"], "title": v["title"]}))
        }
        "update_page" => {
            let mut changes = a.clone();
            changes.remove("id");
            let v = wiki::update_page(st, ctx, arg_uuid(a, "id")?, changes).await?;
            Ok(json!({"id": v["id"], "version": v["version"], "title": v["title"]}))
        }
        other => Err(AppError::BadRequest(format!("unknown tool {other:?}"))),
    }
}

/// Read a `gantry://item/<KEY>` or `gantry://page/<id>` resource as Markdown.
async fn read_resource(st: &St, ctx: &Ctx, uri: &str) -> Result<Value, AppError> {
    let text = if let Some(key) = uri.strip_prefix("gantry://item/") {
        let (item, project, perms) = items::load_item(st, ctx, key).await?;
        let v = items::detail_json(st, ctx, item.id, &project, &perms).await?;
        let comments: String = v["comments"].as_array().map(|c| c.iter().map(|c| format!("**{}** ({}):\n\n{}\n", c["author"]["name"].as_str().unwrap_or("Unknown"), c["created_at"].as_str().unwrap_or_default(), c["body"].as_str().unwrap_or_default())).collect::<Vec<_>>().join("\n")).unwrap_or_default();
        format!("# {} {}\n\n- Type: {}\n- Status: {}\n- Priority: {}\n- Assignee: {}\n\n{}\n\n## Comments\n\n{}", v["key"].as_str().unwrap_or_default(), item.title, item.type_key, item.status, item.priority, v["assignee"]["name"].as_str().unwrap_or("unassigned"), item.description, if comments.is_empty() { "None.".into() } else { comments })
    } else if let Some(id) = uri.strip_prefix("gantry://page/") {
        let (page, _, _) = wiki::load_page(st, ctx, id.parse().map_err(|_| AppError::NotFound)?).await?;
        format!("# {}\n\n{}", page.title, page.content)
    } else {
        return Err(AppError::NotFound);
    };
    Ok(json!({"contents": [{"uri": uri, "mimeType": "text/markdown", "text": text}]}))
}

fn rpc_error(id: &Value, code: i64, message: &str, data: Option<Value>) -> Value {
    let mut err = json!({"code": code, "message": message});
    if let Some(d) = data {
        err["data"] = d;
    }
    json!({"jsonrpc": "2.0", "id": id, "error": err})
}

/// Decode an `Mcp-Name` header value, which may use the `=?base64?…?=` form for non-ASCII names.
pub fn decode_header_value(v: &str) -> Option<String> {
    match v.strip_prefix("=?base64?").and_then(|r| r.strip_suffix("?=")) {
        Some(b64) => String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b64).ok()?).ok(),
        None => Some(v.to_string()),
    }
}

/// What the modern transport requires of the HTTP headers, given the request body.
/// Returns `(http status, JSON-RPC code, message, data)` on a mismatch.
pub fn check_modern_headers(headers: &HeaderMap, method: &str, version: &str, params: &Value) -> Result<(), (StatusCode, i64, String, Option<Value>)> {
    let h = |n: &str| headers.get(n).and_then(|v| v.to_str().ok());
    if version != MODERN {
        let supported: Vec<&str> = std::iter::once(MODERN).chain(LEGACY.iter().copied()).collect();
        return Err((StatusCode::BAD_REQUEST, -32022, "Unsupported protocol version".into(), Some(json!({"supported": supported, "requested": version}))));
    }
    if h("mcp-protocol-version") != Some(version) {
        return Err((StatusCode::BAD_REQUEST, -32020, "MCP-Protocol-Version header does not match the request body".into(), None));
    }
    if h("mcp-method") != Some(method) {
        return Err((StatusCode::BAD_REQUEST, -32020, "Mcp-Method header does not match the request body".into(), None));
    }
    let expected_name = match method {
        "tools/call" | "prompts/get" => params["name"].as_str(),
        "resources/read" => params["uri"].as_str(),
        _ => None,
    };
    if let Some(expected) = expected_name {
        if h("mcp-name").and_then(decode_header_value).as_deref() != Some(expected) {
            return Err((StatusCode::BAD_REQUEST, -32020, "Mcp-Name header does not match the request body".into(), None));
        }
    }
    Ok(())
}

fn json_response(status: StatusCode, body: Value) -> Response {
    (status, Json(body)).into_response()
}

async fn handle(State(st): State<St>, info: ReqInfo, crate::auth::MaybeCtx(ctx): crate::auth::MaybeCtx, headers: HeaderMap, body: Bytes) -> Response {
    // A browser must never be able to drive the MCP endpoint from another site.
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let host = headers.get(header::HOST).and_then(|v| v.to_str().ok()).unwrap_or_default();
        let ok = st.cfg.public_origin().map(|o| o == origin).unwrap_or_else(|| origin.split_once("://").map(|(_, h)| h == host).unwrap_or(false));
        if !ok {
            audit::security(&st, "mcp.origin_rejected", "warning", None, &info, json!({"origin": origin.chars().take(200).collect::<String>()})).await;
            return json_response(StatusCode::FORBIDDEN, rpc_error(&Value::Null, -32600, "Origin not allowed", None));
        }
    }
    let Some(ctx) = ctx else {
        let mut r = json_response(StatusCode::UNAUTHORIZED, rpc_error(&Value::Null, -32001, "Authentication required: send an API token as a Bearer token", None));
        r.headers_mut().insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("Bearer realm=\"gantry-mcp\""));
        return r;
    };
    let Ok(msg) = serde_json::from_slice::<Value>(&body) else { return json_response(StatusCode::BAD_REQUEST, rpc_error(&Value::Null, -32700, "Parse error", None)) };
    if msg.is_array() {
        return json_response(StatusCode::BAD_REQUEST, rpc_error(&Value::Null, -32600, "Batch requests are not supported", None));
    }
    let Some(method) = msg["method"].as_str().map(str::to_string) else {
        // A response or malformed message from the client: nothing to do.
        return if msg.get("result").is_some() || msg.get("error").is_some() { StatusCode::ACCEPTED.into_response() } else { json_response(StatusCode::BAD_REQUEST, rpc_error(&Value::Null, -32600, "Invalid request", None)) };
    };
    let Some(id) = msg.get("id").filter(|i| !i.is_null()).cloned() else { return StatusCode::ACCEPTED.into_response() };
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let version = params["_meta"][META_VERSION].as_str().map(str::to_string);
    let modern = version.is_some();
    if let Some(v) = &version {
        if let Err((status, code, message, data)) = check_modern_headers(&headers, &method, v, &params) {
            return json_response(status, rpc_error(&id, code, &message, data));
        }
    }
    // Shape a successful result for the era in use.
    let finish = |mut result: Value, cacheable: bool| -> Response {
        if modern {
            if let Some(o) = result.as_object_mut() {
                o.insert("resultType".into(), json!("complete"));
                if cacheable {
                    o.insert("ttlMs".into(), json!(60_000));
                    o.insert("cacheScope".into(), json!("public"));
                }
                o.insert("_meta".into(), json!({"io.modelcontextprotocol/serverInfo": server_info()}));
            }
        }
        json_response(StatusCode::OK, json!({"jsonrpc": "2.0", "id": id, "result": result}))
    };
    let capabilities = json!({"tools": {}, "resources": {}});
    match method.as_str() {
        "server/discover" => finish(json!({"supportedVersions": std::iter::once(MODERN).chain(LEGACY.iter().copied()).collect::<Vec<_>>(), "capabilities": capabilities, "serverInfo": server_info(), "instructions": INSTRUCTIONS}), false),
        "initialize" if !modern => {
            let asked = params["protocolVersion"].as_str().unwrap_or_default();
            let chosen = if LEGACY.contains(&asked) { asked } else { LEGACY[0] };
            finish(json!({"protocolVersion": chosen, "capabilities": capabilities, "serverInfo": server_info(), "instructions": INSTRUCTIONS}), false)
        }
        "ping" => finish(json!({}), false),
        // The list depends on who asks (an agent's rules, a token's scopes), so it is not shared between callers.
        "tools/list" => {
            let mut r = json!({"tools": tools_for(&ctx)});
            if modern {
                r["ttlMs"] = json!(60_000);
                r["cacheScope"] = json!("private");
            }
            finish(r, false)
        }
        "resources/list" => finish(json!({"resources": []}), true),
        "resources/templates/list" => finish(
            json!({"resourceTemplates": [
                {"uriTemplate": "gantry://item/{key}", "name": "work-item", "title": "Work item", "description": "A work item as Markdown, by key (e.g. OPS-12)", "mimeType": "text/markdown"},
                {"uriTemplate": "gantry://page/{id}", "name": "page", "title": "Documentation page", "description": "A page as Markdown, by id", "mimeType": "text/markdown"}]}),
            true,
        ),
        "resources/read" => match read_resource(&st, &ctx, params["uri"].as_str().unwrap_or_default()).await {
            Ok(v) => {
                audit::log(&st, &ctx, "mcp.resource_read", "mcp", params["uri"].as_str().unwrap_or_default(), json!({})).await;
                finish(v, false)
            }
            Err(_) => json_response(if modern { StatusCode::NOT_FOUND } else { StatusCode::OK }, rpc_error(&id, -32002, "Resource not found", Some(json!({"uri": params["uri"]})))),
        },
        "tools/call" => {
            let name = params["name"].as_str().unwrap_or_default().to_string();
            if !TOOLS.iter().any(|t| t.name == name) {
                return json_response(if modern { StatusCode::BAD_REQUEST } else { StatusCode::OK }, rpc_error(&id, -32602, &format!("Unknown tool: {name}"), None));
            }
            let args = params["arguments"].as_object().cloned().unwrap_or_default();
            // A tool the caller's rules exclude is refused before anything is looked up.
            let permission = tool_permission(&name);
            let outcome = if ctx.policy.as_ref().is_some_and(|p| !p.allows(permission)) {
                let _ = crate::authz::deny(&st, &ctx, permission, &format!("mcp_tool:{name}"));
                Err(AppError::Forbidden(format!("this AI agent account is not allowed to use {name}; an administrator sets what it may do")))
            } else {
                call_tool(&st, &ctx, &name, &args).await
            };
            // Argument values are not logged: they can contain page bodies. Keys identify what was asked.
            audit::log(&st, &ctx, "mcp.tool_call", "mcp_tool", &name, json!({"ok": outcome.is_ok(), "arguments": args.keys().collect::<Vec<_>>(), "target": args.get("key").or(args.get("project")).or(args.get("space")).or(args.get("id"))})).await;
            match outcome {
                Ok(v) => finish(json!({"content": [{"type": "text", "text": serde_json::to_string(&v).unwrap_or_default()}], "structuredContent": v, "isError": false}), false),
                // Tool failures are results the model can read and correct, not protocol errors.
                Err(e) => {
                    if e.status().is_server_error() {
                        tracing::error!(error = ?e, tool = %name, "MCP tool failed");
                    }
                    finish(json!({"content": [{"type": "text", "text": format!("Error ({}): {}", e.code(), e.public_message())}], "isError": true}), false)
                }
            }
        }
        _ => json_response(if modern { StatusCode::NOT_FOUND } else { StatusCode::OK }, rpc_error(&id, -32601, "Method not found", None)),
    }
}

async fn not_allowed() -> Response {
    let mut r = StatusCode::METHOD_NOT_ALLOWED.into_response();
    r.headers_mut().insert(header::ALLOW, HeaderValue::from_static("POST"));
    r
}

pub fn routes(api: Api) -> Api {
    // The handler authenticates itself so it can answer in JSON-RPC form.
    api.route("POST", "/mcp", "MCP", "Model Context Protocol endpoint (JSON-RPC over Streamable HTTP)", post(handle).get(not_allowed).delete(not_allowed))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn headers(pairs: &[(&'static str, &str)]) -> HeaderMap {
        let mut h = HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, HeaderValue::from_str(v).unwrap());
        }
        h
    }

    #[test]
    fn tool_catalog_is_well_formed() {
        let list = tool_list();
        assert!(list.len() >= 15);
        let mut names = std::collections::HashSet::new();
        for t in &list {
            let name = t["name"].as_str().unwrap();
            assert!(names.insert(name.to_string()), "duplicate tool {name}");
            assert!(name.chars().all(|c| c.is_ascii_lowercase() || c == '_'));
            assert_eq!(t["inputSchema"]["type"], "object");
            assert_eq!(t["inputSchema"]["additionalProperties"], false);
            for r in t["inputSchema"]["required"].as_array().unwrap() {
                assert!(t["inputSchema"]["properties"].get(r.as_str().unwrap()).is_some(), "{name}: required {r} is not a property");
            }
        }
    }

    #[test]
    fn writing_tools_are_not_marked_read_only() {
        for name in ["create_work_item", "update_work_item", "transition_work_item", "add_comment", "link_work_items", "create_page", "update_page"] {
            assert!(!TOOLS.iter().find(|t| t.name == name).unwrap().read_only, "{name}");
        }
        for name in ["search", "get_work_item", "list_work_items", "get_page", "get_item_graph"] {
            assert!(TOOLS.iter().find(|t| t.name == name).unwrap().read_only, "{name}");
        }
    }

    #[test]
    fn modern_requests_need_matching_headers() {
        let params = json!({"name": "search", "arguments": {}});
        let good = headers(&[("mcp-protocol-version", MODERN), ("mcp-method", "tools/call"), ("mcp-name", "search")]);
        assert!(check_modern_headers(&good, "tools/call", MODERN, &params).is_ok());
        let wrong_tool = headers(&[("mcp-protocol-version", MODERN), ("mcp-method", "tools/call"), ("mcp-name", "get_page")]);
        assert_eq!(check_modern_headers(&wrong_tool, "tools/call", MODERN, &params).unwrap_err().1, -32020);
        let wrong_method = headers(&[("mcp-protocol-version", MODERN), ("mcp-method", "tools/list"), ("mcp-name", "search")]);
        assert_eq!(check_modern_headers(&wrong_method, "tools/call", MODERN, &params).unwrap_err().1, -32020);
        let no_version = headers(&[("mcp-method", "tools/call"), ("mcp-name", "search")]);
        assert_eq!(check_modern_headers(&no_version, "tools/call", MODERN, &params).unwrap_err().1, -32020);
        let (status, code, _, data) = check_modern_headers(&good, "tools/call", "2031-01-01", &params).unwrap_err();
        assert_eq!((status, code), (StatusCode::BAD_REQUEST, -32022));
        assert_eq!(data.unwrap()["supported"][0], MODERN);
        let list = headers(&[("mcp-protocol-version", MODERN), ("mcp-method", "tools/list")]);
        assert!(check_modern_headers(&list, "tools/list", MODERN, &json!({})).is_ok(), "Mcp-Name is only needed where a name exists");
    }

    #[test]
    fn mcp_name_header_decoding() {
        assert_eq!(decode_header_value("search").as_deref(), Some("search"));
        assert_eq!(decode_header_value("=?base64?Z2FudHJ5Oi8vaXRlbS9PUFMtMQ==?=").as_deref(), Some("gantry://item/OPS-1"));
        assert_eq!(decode_header_value("=?base64?!!!?="), None);
    }
}
