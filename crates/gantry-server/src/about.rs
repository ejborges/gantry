//! What the application tells its users about itself: the organization's privacy, terms,
//! cookie, retention and security statements (written by an administrator), how to get help,
//! and the installed components and add-ons with their versions.
//!
//! The list of third-party software and its licenses is a static file built with the web
//! application (`/licenses.json`), so it always describes exactly what was shipped.

use crate::auth::{Ctx, MaybeCtx};
use crate::error::AppResult;
use crate::http::Api;
use crate::settings::{self, LEGAL_DOCS};
use crate::St;
use axum::extract::State;
use axum::routing::get;
use axum::Json;
use serde_json::{json, Value};

/// When this binary was built, if the build recorded it (the RPM build does).
pub const BUILD_DATE: Option<&str> = option_env!("GANTRY_BUILD_DATE");

/// Public: the statements an administrator has written, and how to reach support. Shown on
/// the sign-in page as well, so it must not need a session.
async fn about(State(st): State<St>, MaybeCtx(ctx): MaybeCtx) -> AppResult<Json<Value>> {
    let site = settings::get(&st, "site").await;
    let legal = settings::get(&st, "legal").await;
    let documents: Vec<Value> = LEGAL_DOCS.iter().map(|(key, title)| json!({"key": key, "title": title, "text": legal[*key].as_str().unwrap_or_default()})).collect();
    let mut out = json!({"site": {"name": site["name"], "support": site["support"]}, "documents": documents, "product": {"name": "Gantry", "license": "Apache-2.0"}});
    // The exact version helps an attacker pick an exploit, so only signed-in people see it.
    if ctx.is_some() {
        out["product"]["version"] = json!(crate::VERSION);
    }
    Ok(Json(out))
}

/// The parts of this installation: server, web application, manual, and each add-on pack,
/// with version, date and description.
async fn components(State(st): State<St>, _ctx: Ctx) -> AppResult<Json<Value>> {
    let packs: Value = sqlx::query_scalar("SELECT coalesce(jsonb_agg(jsonb_build_object('id', id, 'name', name, 'version', version, 'description', description, 'installed_at', installed_at) ORDER BY (id <> 'core'), name), '[]'::jsonb) FROM packs").fetch_one(&st.db).await?;
    let read = |name: &str| std::fs::read_to_string(st.cfg.server.web_root.join(name)).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok());
    let web = read("build.json").unwrap_or(json!({}));
    let manual = std::fs::read_to_string(st.cfg.server.manual_root.join("build.json")).ok().and_then(|t| serde_json::from_str::<Value>(&t).ok()).unwrap_or(json!({}));
    Ok(Json(json!({
        "components": [
            {"key": "server", "name": "Gantry server", "version": crate::VERSION, "date": BUILD_DATE, "summary": "API, background worker and command line",
             "description": "Serves the REST API, the real-time connection and the MCP endpoint; runs background jobs; applies database migrations."},
            {"key": "web", "name": "Web application", "version": web["version"], "date": web["date"], "summary": "The interface in your browser",
             "description": "A single-page application served as static files. It talks only to this server."},
            {"key": "manual", "name": "Manual", "version": manual["version"], "date": manual["date"], "summary": "Guides for users and administrators",
             "description": "A static site served at /manual, built from the same release as the server."},
        ],
        "addons": packs,
        "storage": {"files": st.stores.blob.name(), "search": st.stores.search.name(), "vectors": st.stores.vector.name(), "cache": st.stores.kv.name()},
        "license": {"name": "Apache License 2.0", "third_party": "/licenses.json"},
    })))
}

pub fn routes(api: Api) -> Api {
    api.route("GET", "/api/v1/about", "About", "Privacy, terms, cookie, retention and security statements, and how to reach support", get(about))
        .public()
        .route("GET", "/api/v1/about/components", "About", "Installed components and add-ons with versions, dates and descriptions", get(components))
}
