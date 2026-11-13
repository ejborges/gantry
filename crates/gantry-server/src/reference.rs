//! The built-in catalog as data: routes, permissions, roles, event kinds, integration
//! kinds, MCP tools, default configuration and the content of the core pack. The manual's
//! reference pages are generated from this (`gantry config reference`), so they cannot
//! drift from what the server actually does.

use serde_json::{json, Value};

use crate::{authz, config::Config, events, http, integrations, mcp, packs};

pub fn catalog() -> Value {
    let routes: Vec<Value> = http::all_routes().docs.iter().map(|r| json!({"method": r.method, "path": r.path, "tag": r.tag, "summary": r.summary, "public": r.public})).collect();
    let permissions: Vec<Value> = authz::PERMISSIONS.iter().map(|(key, scope, description)| json!({"key": key, "scope": scope, "description": description})).collect();
    let roles: Vec<Value> = authz::BUILTIN_ROLES.iter().map(|r| json!({"key": r.key, "name": r.name, "scope": r.scope, "permissions": r.permissions, "description": r.description})).collect();
    let events: Vec<Value> = events::EVENT_KINDS.iter().map(|(kind, description)| json!({"kind": kind, "description": description})).collect();
    let kinds: Vec<Value> = integrations::KINDS
        .iter()
        .map(|k| {
            let fields = |list: &[(&str, &str, bool)]| list.iter().map(|(key, label, required)| json!({"key": key, "label": label, "required": required})).collect::<Vec<_>>();
            json!({"kind": k.kind, "name": k.name, "direction": k.direction, "description": k.description, "settings": fields(k.config), "secrets": fields(k.secrets)})
        })
        .collect();
    let core: Value = serde_json::from_str(packs::CORE_PACK).unwrap_or(Value::Null);
    let names = |section: &str, extra: &[&str]| -> Vec<Value> {
        core[section]
            .as_array()
            .map(|list| {
                list.iter()
                    .map(|entry| {
                        let mut out = json!({"key": entry["key"], "name": entry["name"], "description": entry["description"]});
                        for field in extra {
                            out[*field] = entry[*field].clone();
                        }
                        out
                    })
                    .collect()
            })
            .unwrap_or_default()
    };
    json!({
        "version": crate::VERSION,
        "routes": routes,
        "permissions": permissions,
        "roles": roles,
        "events": events,
        "integrations": kinds,
        "mcp_tools": mcp::tool_list(),
        "default_config": toml::to_string_pretty(&Config::default()).unwrap_or_default(),
        "status_texts": crate::pages::STATUS_TEXTS.iter().map(|t| json!({"code": t.code, "title": t.title, "meaning": t.meaning, "action": t.action})).collect::<Vec<_>>(),
        "notification_categories": crate::settings::CATEGORIES.iter().map(|c| json!({"key": c.0, "name": c.1, "description": c.2, "in_app": c.3, "email": c.4, "permission": c.5})).collect::<Vec<_>>(),
        "alert_kinds": crate::notify::ALERT_KINDS.iter().map(|k| json!({"kind": k.kind, "scope": k.scope, "name": k.name, "description": k.description,
            "params": k.params.iter().map(|(key, label, default)| json!({"key": key, "label": label, "default": default})).collect::<Vec<_>>()})).collect::<Vec<_>>(),
        "visualizations": {
            "libraries": crate::analytics::LIBRARIES.iter().map(|(key, name, _)| json!({"key": key, "name": name})).collect::<Vec<_>>(),
            "templates": crate::analytics::TEMPLATES.iter().map(|t| json!({"key": t.key, "name": t.name, "description": t.description, "library": t.library})).collect::<Vec<_>>(),
        },
        "core": {
            "item_types": names("item_types", &["workflow", "fields", "required"]),
            "workflows": names("workflows", &["initial", "statuses", "transitions"]),
            "fields": names("fields", &["type", "options"]),
            "project_templates": names("project_templates", &["domain", "kind", "item_types"]),
            "page_templates": names("page_templates", &["category"]),
        },
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn the_catalog_is_complete_enough_to_document() {
        let c = super::catalog();
        assert!(c["routes"].as_array().unwrap().len() > 80);
        assert!(c["permissions"].as_array().unwrap().len() > 25);
        assert!(c["roles"].as_array().unwrap().iter().any(|r| r["key"] == "system_admin"));
        assert!(c["mcp_tools"].as_array().unwrap().iter().all(|t| t["name"].is_string() && t["description"].is_string()));
        assert!(c["default_config"].as_str().unwrap().contains("[storage.blob]"));
        assert!(c["core"]["item_types"].as_array().unwrap().len() >= 20);
        assert!(c["status_texts"].as_array().unwrap().iter().any(|t| t["code"] == 404));
        assert!(c["notification_categories"].as_array().unwrap().len() >= 10);
        assert!(c["alert_kinds"].as_array().unwrap().iter().any(|k| k["scope"] == "system") && c["alert_kinds"].as_array().unwrap().iter().any(|k| k["scope"] == "application"));
        assert_eq!(c["visualizations"]["templates"].as_array().unwrap().len(), crate::analytics::TEMPLATES.len());
        // Nothing secret is part of the reference.
        assert!(!c.to_string().contains("secret_key ="));
    }
}
