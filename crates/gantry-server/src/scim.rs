//! SCIM 2.0 provisioning (RFC 7643/7644) so an identity provider can create, update and
//! deactivate accounts and maintain groups. Covers what Microsoft Entra ID and Okta send:
//! `userName eq` / `displayName eq` filters, PUT, and PATCH with add / replace / remove.
//!
//! Authenticate with an API token that has the `admin` scope and belongs to an account holding
//! the `scim.provision` permission (the built-in "Provisioning client" role).
//! Deleting a user deactivates the account and ends its sessions; history is kept.

use crate::auth::Ctx;
use crate::authz;
use crate::error::{bad, AppError, AppResult};
use crate::http::Api;
use crate::{audit, St};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::routing::get;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Value};
use sqlx::Row;
use uuid::Uuid;

const USER_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:User";
const GROUP_SCHEMA: &str = "urn:ietf:params:scim:schemas:core:2.0:Group";
const LIST_SCHEMA: &str = "urn:ietf:params:scim:api:messages:2.0:ListResponse";

fn guard(st: &St, ctx: &Ctx) -> AppResult<()> {
    authz::require_global(st, ctx, "scim.provision")
}

#[derive(Deserialize)]
struct ListQuery {
    filter: Option<String>,
    #[serde(rename = "startIndex")]
    start_index: Option<i64>,
    count: Option<i64>,
}

/// Parse `attr eq "value"`; the only filter form identity providers use for matching.
pub fn parse_eq_filter(filter: &str, attr: &str) -> Option<String> {
    let rest = filter.trim().strip_prefix(attr).or_else(|| filter.trim().strip_prefix(&attr.to_lowercase()))?;
    let rest = rest.trim_start().strip_prefix("eq").or_else(|| rest.trim_start().strip_prefix("EQ"))?.trim();
    Some(rest.strip_prefix('"')?.strip_suffix('"')?.replace("\\\"", "\""))
}

fn list_response(resources: Vec<Value>, total: i64, start: i64) -> Value {
    json!({"schemas": [LIST_SCHEMA], "totalResults": total, "startIndex": start, "itemsPerPage": resources.len(), "Resources": resources})
}

fn user_json(r: &sqlx::postgres::PgRow) -> Value {
    let id: Uuid = r.get("id");
    let (email, name): (String, String) = (r.get("email"), r.get("name"));
    json!({"schemas": [USER_SCHEMA], "id": id, "externalId": r.get::<Option<String>, _>("external_id"), "userName": email, "displayName": name,
           "name": {"formatted": name}, "emails": [{"value": email, "primary": true}], "active": r.get::<bool, _>("active"),
           "meta": {"resourceType": "User", "created": r.get::<chrono::DateTime<chrono::Utc>, _>("created_at"), "lastModified": r.get::<chrono::DateTime<chrono::Utc>, _>("updated_at"), "location": format!("/scim/v2/Users/{id}")}})
}

const USER_COLS: &str = "id, email, name, active, external_id, created_at, updated_at";

fn display_name(v: &Value) -> Option<String> {
    let name = v["displayName"].as_str().map(str::to_string).or_else(|| v["name"]["formatted"].as_str().map(str::to_string)).or_else(|| {
        let (g, f) = (v["name"]["givenName"].as_str().unwrap_or_default(), v["name"]["familyName"].as_str().unwrap_or_default());
        let full = format!("{g} {f}").trim().to_string();
        (!full.is_empty()).then_some(full)
    })?;
    // A directory can hold anything; a name here is one line that displays as it is stored.
    let clean: String = name.chars().filter(|c| !crate::accounts::deceptive(*c)).take(200).collect();
    Some(clean.trim().to_string()).filter(|n| !n.is_empty())
}

fn user_email(v: &Value) -> Option<String> {
    let from_list = v["emails"].as_array().and_then(|a| a.iter().find(|e| e["primary"] == true).or(a.first())).and_then(|e| e["value"].as_str());
    v["userName"].as_str().filter(|u| u.contains('@')).or(from_list).map(|e| e.trim().to_lowercase())
}

async fn list_users(State(st): State<St>, ctx: Ctx, Query(q): Query<ListQuery>) -> AppResult<Json<Value>> {
    guard(&st, &ctx)?;
    let start = q.start_index.unwrap_or(1).max(1);
    let count = q.count.unwrap_or(100).clamp(0, 500);
    let email = match &q.filter {
        Some(f) => Some(parse_eq_filter(f, "userName").ok_or_else(|| bad("only filters of the form userName eq \"value\" are supported"))?.to_lowercase()),
        None => None,
    };
    let rows = sqlx::query(&format!("SELECT {USER_COLS} FROM users WHERE kind = 'human' AND ($1::text IS NULL OR lower(email) = $1) ORDER BY created_at, id LIMIT $2 OFFSET $3")).bind(&email).bind(count).bind(start - 1).fetch_all(&st.db).await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM users WHERE kind = 'human' AND ($1::text IS NULL OR lower(email) = $1)").bind(&email).fetch_one(&st.db).await?;
    Ok(Json(list_response(rows.iter().map(user_json).collect(), total, start)))
}

async fn fetch_user(st: &St, id: Uuid) -> AppResult<Value> {
    Ok(user_json(&sqlx::query(&format!("SELECT {USER_COLS} FROM users WHERE id = $1 AND kind = 'human'")).bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?))
}

async fn create_user(State(st): State<St>, ctx: Ctx, Json(b): Json<Value>) -> AppResult<(StatusCode, Json<Value>)> {
    guard(&st, &ctx)?;
    let email = user_email(&b).ok_or_else(|| bad("userName must be an email address"))?;
    let name = display_name(&b).unwrap_or_else(|| email.clone());
    let active = b["active"].as_bool().unwrap_or(true);
    let id: Uuid = sqlx::query_scalar("INSERT INTO users (email, name, active, external_id) VALUES ($1,$2,$3,$4) RETURNING id").bind(&email).bind(name.chars().take(200).collect::<String>()).bind(active).bind(b["externalId"].as_str()).fetch_one(&st.db).await?;
    sqlx::query("INSERT INTO role_bindings (role_id, user_id, scope_type) SELECT id, $1, 'global' FROM roles WHERE key = 'member'").bind(id).execute(&st.db).await?;
    audit::log(&st, &ctx, "scim.user_create", "user", &id.to_string(), json!({"email": email, "active": active})).await;
    Ok((StatusCode::CREATED, Json(fetch_user(&st, id).await?)))
}

async fn get_user(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    guard(&st, &ctx)?;
    Ok(Json(fetch_user(&st, id).await?))
}

async fn apply_user(st: &St, ctx: &Ctx, id: Uuid, email: Option<String>, name: Option<String>, active: Option<bool>, external_id: Option<String>, action: &str) -> AppResult<Value> {
    let n = sqlx::query("UPDATE users SET email = coalesce($2, email), name = coalesce($3, name), active = coalesce($4, active), external_id = coalesce($5, external_id), updated_at = now() WHERE id = $1 AND kind = 'human'")
        .bind(id).bind(&email).bind(name.map(|n| n.chars().take(200).collect::<String>())).bind(active).bind(&external_id)
        .execute(&st.db).await?.rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    if active == Some(false) {
        sqlx::query("DELETE FROM sessions WHERE user_id = $1").bind(id).execute(&st.db).await?;
        sqlx::query("UPDATE api_tokens SET revoked_at = now() WHERE user_id = $1 AND revoked_at IS NULL").bind(id).execute(&st.db).await?;
    }
    audit::log(st, ctx, action, "user", &id.to_string(), json!({"email": email, "active": active})).await;
    fetch_user(st, id).await
}

async fn put_user(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Value>) -> AppResult<Json<Value>> {
    guard(&st, &ctx)?;
    Ok(Json(apply_user(&st, &ctx, id, user_email(&b), display_name(&b), b["active"].as_bool(), b["externalId"].as_str().map(str::to_string), "scim.user_replace").await?))
}

fn truthy(v: &Value) -> Option<bool> {
    // Microsoft Entra ID sends the strings "True" / "False".
    v.as_bool().or_else(|| v.as_str().and_then(|s| s.to_ascii_lowercase().parse().ok()))
}

async fn patch_user(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Value>) -> AppResult<Json<Value>> {
    guard(&st, &ctx)?;
    let (mut email, mut name, mut active, mut ext) = (None, None, None, None);
    for op in b["Operations"].as_array().map(Vec::as_slice).unwrap_or_default() {
        if !op["op"].as_str().map(|o| o.eq_ignore_ascii_case("replace") || o.eq_ignore_ascii_case("add")).unwrap_or(false) {
            continue;
        }
        match op["path"].as_str() {
            Some(p) if p.eq_ignore_ascii_case("active") => active = truthy(&op["value"]),
            Some(p) if p.eq_ignore_ascii_case("userName") => email = op["value"].as_str().map(|s| s.trim().to_lowercase()),
            Some(p) if p.eq_ignore_ascii_case("displayName") || p.eq_ignore_ascii_case("name.formatted") => name = op["value"].as_str().map(str::to_string),
            Some(p) if p.eq_ignore_ascii_case("externalId") => ext = op["value"].as_str().map(str::to_string),
            Some(_) => {}
            None => {
                let v = &op["value"];
                active = truthy(&v["active"]).or(active);
                email = user_email(v).or(email);
                name = display_name(v).or(name);
                ext = v["externalId"].as_str().map(str::to_string).or(ext);
            }
        }
    }
    Ok(Json(apply_user(&st, &ctx, id, email, name, active, ext, "scim.user_patch").await?))
}

async fn delete_user(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    guard(&st, &ctx)?;
    apply_user(&st, &ctx, id, None, None, Some(false), None, "scim.user_deactivate").await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn group_json(st: &St, id: Uuid) -> AppResult<Value> {
    let g = sqlx::query("SELECT id, name, external_id, created_at FROM groups WHERE id = $1").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    let members: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('value', u.id, 'display', u.name)), '[]'::jsonb) FROM group_members m JOIN users u ON u.id = m.user_id WHERE m.group_id = $1").bind(id).fetch_one(&st.db).await?;
    Ok(json!({"schemas": [GROUP_SCHEMA], "id": id, "externalId": g.get::<Option<String>, _>("external_id"), "displayName": g.get::<String, _>("name"), "members": members,
              "meta": {"resourceType": "Group", "created": g.get::<chrono::DateTime<chrono::Utc>, _>("created_at"), "location": format!("/scim/v2/Groups/{id}")}}))
}

fn member_ids(v: &Value) -> Vec<Uuid> {
    v.as_array().map(|a| a.iter().filter_map(|m| m["value"].as_str().and_then(|s| s.parse().ok())).collect()).unwrap_or_default()
}

async fn add_members(st: &St, group: Uuid, ids: &[Uuid]) -> AppResult<()> {
    sqlx::query("INSERT INTO group_members (group_id, user_id) SELECT $1, u.id FROM users u WHERE u.id = ANY($2) ON CONFLICT DO NOTHING").bind(group).bind(ids).execute(&st.db).await?;
    Ok(())
}

async fn list_groups(State(st): State<St>, ctx: Ctx, Query(q): Query<ListQuery>) -> AppResult<Json<Value>> {
    guard(&st, &ctx)?;
    let start = q.start_index.unwrap_or(1).max(1);
    let name = match &q.filter {
        Some(f) => Some(parse_eq_filter(f, "displayName").ok_or_else(|| bad("only filters of the form displayName eq \"value\" are supported"))?),
        None => None,
    };
    let ids: Vec<Uuid> = sqlx::query_scalar("SELECT id FROM groups WHERE ($1::text IS NULL OR name = $1) ORDER BY name LIMIT $2 OFFSET $3").bind(&name).bind(q.count.unwrap_or(100).clamp(0, 500)).bind(start - 1).fetch_all(&st.db).await?;
    let total: i64 = sqlx::query_scalar("SELECT count(*) FROM groups WHERE ($1::text IS NULL OR name = $1)").bind(&name).fetch_one(&st.db).await?;
    let mut out = vec![];
    for id in ids {
        out.push(group_json(&st, id).await?);
    }
    Ok(Json(list_response(out, total, start)))
}

async fn create_group(State(st): State<St>, ctx: Ctx, Json(b): Json<Value>) -> AppResult<(StatusCode, Json<Value>)> {
    guard(&st, &ctx)?;
    let name = b["displayName"].as_str().map(str::trim).filter(|n| !n.is_empty() && n.len() <= 200).ok_or_else(|| bad("displayName is required"))?;
    let id: Uuid = sqlx::query_scalar("INSERT INTO groups (name, external_id) VALUES ($1, $2) RETURNING id").bind(name).bind(b["externalId"].as_str()).fetch_one(&st.db).await?;
    add_members(&st, id, &member_ids(&b["members"])).await?;
    audit::log(&st, &ctx, "scim.group_create", "group", &id.to_string(), json!({"name": name})).await;
    Ok((StatusCode::CREATED, Json(group_json(&st, id).await?)))
}

async fn get_group(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<Json<Value>> {
    guard(&st, &ctx)?;
    Ok(Json(group_json(&st, id).await?))
}

async fn put_group(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Value>) -> AppResult<Json<Value>> {
    guard(&st, &ctx)?;
    let n = sqlx::query("UPDATE groups SET name = coalesce($2, name) WHERE id = $1").bind(id).bind(b["displayName"].as_str()).execute(&st.db).await?.rows_affected();
    if n == 0 {
        return Err(AppError::NotFound);
    }
    sqlx::query("DELETE FROM group_members WHERE group_id = $1").bind(id).execute(&st.db).await?;
    add_members(&st, id, &member_ids(&b["members"])).await?;
    audit::log(&st, &ctx, "scim.group_replace", "group", &id.to_string(), json!({"members": member_ids(&b["members"]).len()})).await;
    Ok(Json(group_json(&st, id).await?))
}

/// `members[value eq "<id>"]` → the id.
fn member_path_id(path: &str) -> Option<Uuid> {
    path.strip_prefix("members[")?.strip_suffix(']').and_then(|f| parse_eq_filter(f, "value")).and_then(|s| s.parse().ok())
}

async fn patch_group(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>, Json(b): Json<Value>) -> AppResult<Json<Value>> {
    guard(&st, &ctx)?;
    if !sqlx::query_scalar::<_, bool>("SELECT EXISTS (SELECT 1 FROM groups WHERE id = $1)").bind(id).fetch_one(&st.db).await? {
        return Err(AppError::NotFound);
    }
    for op in b["Operations"].as_array().map(Vec::as_slice).unwrap_or_default() {
        let kind = op["op"].as_str().unwrap_or_default().to_ascii_lowercase();
        let path = op["path"].as_str().unwrap_or_default();
        match (kind.as_str(), path) {
            ("add", p) if p.eq_ignore_ascii_case("members") => add_members(&st, id, &member_ids(&op["value"])).await?,
            ("replace", p) if p.eq_ignore_ascii_case("members") => {
                sqlx::query("DELETE FROM group_members WHERE group_id = $1").bind(id).execute(&st.db).await?;
                add_members(&st, id, &member_ids(&op["value"])).await?;
            }
            ("remove", p) if p.eq_ignore_ascii_case("members") => {
                let ids = member_ids(&op["value"]);
                if ids.is_empty() {
                    sqlx::query("DELETE FROM group_members WHERE group_id = $1").bind(id).execute(&st.db).await?;
                } else {
                    sqlx::query("DELETE FROM group_members WHERE group_id = $1 AND user_id = ANY($2)").bind(id).bind(&ids).execute(&st.db).await?;
                }
            }
            ("remove", p) if p.starts_with("members[") => {
                let uid = member_path_id(p).ok_or_else(|| bad("unsupported member filter"))?;
                sqlx::query("DELETE FROM group_members WHERE group_id = $1 AND user_id = $2").bind(id).bind(uid).execute(&st.db).await?;
            }
            ("replace" | "add", p) if p.eq_ignore_ascii_case("displayName") => {
                sqlx::query("UPDATE groups SET name = $2 WHERE id = $1").bind(id).bind(op["value"].as_str().ok_or_else(|| bad("displayName must be text"))?).execute(&st.db).await?;
            }
            ("replace" | "add", "") => {
                if let Some(n) = op["value"]["displayName"].as_str() {
                    sqlx::query("UPDATE groups SET name = $2 WHERE id = $1").bind(id).bind(n).execute(&st.db).await?;
                }
                if op["value"]["members"].is_array() {
                    add_members(&st, id, &member_ids(&op["value"]["members"])).await?;
                }
            }
            _ => return Err(bad(format!("unsupported operation {kind} on {path:?}"))),
        }
    }
    audit::log(&st, &ctx, "scim.group_patch", "group", &id.to_string(), json!({"operations": b["Operations"].as_array().map(Vec::len)})).await;
    Ok(Json(group_json(&st, id).await?))
}

async fn delete_group(State(st): State<St>, ctx: Ctx, Path(id): Path<Uuid>) -> AppResult<StatusCode> {
    guard(&st, &ctx)?;
    let name: String = sqlx::query_scalar("DELETE FROM groups WHERE id = $1 RETURNING name").bind(id).fetch_optional(&st.db).await?.ok_or(AppError::NotFound)?;
    audit::log(&st, &ctx, "scim.group_delete", "group", &id.to_string(), json!({"name": name})).await;
    Ok(StatusCode::NO_CONTENT)
}

async fn service_provider_config(State(st): State<St>, ctx: Ctx) -> AppResult<Json<Value>> {
    guard(&st, &ctx)?;
    Ok(Json(json!({
        "schemas": ["urn:ietf:params:scim:schemas:core:2.0:ServiceProviderConfig"],
        "patch": {"supported": true}, "bulk": {"supported": false, "maxOperations": 0, "maxPayloadSize": 0},
        "filter": {"supported": true, "maxResults": 500}, "changePassword": {"supported": false}, "sort": {"supported": false}, "etag": {"supported": false},
        "authenticationSchemes": [{"type": "oauthbearertoken", "name": "API token", "description": "Gantry API token with the admin scope"}]
    })))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/scim/v2/ServiceProviderConfig", "SCIM", "SCIM capabilities", get(service_provider_config))
        .route("GET", "/scim/v2/Users", "SCIM", "List or look up users", get(list_users))
        .route("POST", "/scim/v2/Users", "SCIM", "Provision a user", axum::routing::post(create_user))
        .route("GET", "/scim/v2/Users/{id}", "SCIM", "Read a user", get(get_user))
        .route("PUT", "/scim/v2/Users/{id}", "SCIM", "Replace a user", axum::routing::put(put_user))
        .route("PATCH", "/scim/v2/Users/{id}", "SCIM", "Update a user (for example deactivate)", axum::routing::patch(patch_user))
        .route("DELETE", "/scim/v2/Users/{id}", "SCIM", "Deactivate a user", axum::routing::delete(delete_user))
        .route("GET", "/scim/v2/Groups", "SCIM", "List or look up groups", get(list_groups))
        .route("POST", "/scim/v2/Groups", "SCIM", "Create a group", axum::routing::post(create_group))
        .route("GET", "/scim/v2/Groups/{id}", "SCIM", "Read a group", get(get_group))
        .route("PUT", "/scim/v2/Groups/{id}", "SCIM", "Replace a group", axum::routing::put(put_group))
        .route("PATCH", "/scim/v2/Groups/{id}", "SCIM", "Change group membership or name", axum::routing::patch(patch_group))
        .route("DELETE", "/scim/v2/Groups/{id}", "SCIM", "Delete a group", axum::routing::delete(delete_group))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eq_filters() {
        assert_eq!(parse_eq_filter(r#"userName eq "ada@example.org""#, "userName").as_deref(), Some("ada@example.org"));
        assert_eq!(parse_eq_filter(r#"displayName eq "Site \"A\" crew""#, "displayName").as_deref(), Some("Site \"A\" crew"));
        assert_eq!(parse_eq_filter(r#"userName co "ada""#, "userName"), None);
        assert_eq!(parse_eq_filter(r#"emails eq "x""#, "userName"), None);
        assert_eq!(parse_eq_filter(r#"userName eq "x' OR '1'='1""#, "userName").as_deref(), Some("x' OR '1'='1"), "the value is data, bound as a parameter");
    }

    #[test]
    fn member_paths() {
        let id = Uuid::new_v4();
        assert_eq!(member_path_id(&format!("members[value eq \"{id}\"]")), Some(id));
        assert_eq!(member_path_id("members[display eq \"x\"]"), None);
        assert_eq!(member_path_id("members"), None);
    }

    #[test]
    fn user_attribute_extraction() {
        let v = json!({"userName": "Ada@Example.org", "name": {"givenName": "Ada", "familyName": "Lovelace"}});
        assert_eq!(user_email(&v).as_deref(), Some("ada@example.org"));
        assert_eq!(display_name(&v).as_deref(), Some("Ada Lovelace"));
        let v = json!({"userName": "ada", "emails": [{"value": "x@y.org"}, {"value": "ada@example.org", "primary": true}], "displayName": "Ada"});
        assert_eq!(user_email(&v).as_deref(), Some("ada@example.org"));
        assert_eq!(display_name(&v).as_deref(), Some("Ada"));
        assert_eq!(user_email(&json!({"userName": "ada"})), None);
        assert_eq!(truthy(&json!("False")), Some(false));
        assert_eq!(truthy(&json!(true)), Some(true));
        assert_eq!(truthy(&json!("maybe")), None);
    }
}
