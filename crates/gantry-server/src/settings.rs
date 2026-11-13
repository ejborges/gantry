//! Settings stored in the database and changed from the administration console.
//!
//! Each key holds one JSON object. A stored object is laid over the defaults below, so a new
//! release can add a field without a migration. Every write is validated here and audited by
//! the caller. Settings that need a restart or carry secrets live in the configuration file
//! instead (`config.rs`).

use crate::error::{bad, AppResult};
use crate::AppState;
use serde_json::{json, Map, Value};

pub struct Key {
    pub key: &'static str,
    /// Permission needed to change it.
    pub permission: &'static str,
    pub summary: &'static str,
}

pub const KEYS: &[Key] = &[
    Key { key: "site", permission: "admin.settings", summary: "Site name, announcement, sign-in notice, default time zone and how people reach support" },
    Key { key: "legal", permission: "admin.settings", summary: "Privacy, terms, cookie, data retention and data security statements shown to users" },
    Key { key: "profile", permission: "admin.users", summary: "Which profile fields people may change themselves, and avatar limits" },
    Key { key: "notifications", permission: "admin.notifications", summary: "Workspace defaults for each notification category, and reminder timing" },
    Key { key: "hierarchy", permission: "planning.manage", summary: "Whether parent and child work items must follow the level order" },
];

pub const LEGAL_DOCS: &[(&str, &str)] = &[
    ("privacy", "Privacy"),
    ("terms", "Terms of use"),
    ("cookies", "Cookies"),
    ("retention", "Data retention"),
    ("security", "Data security"),
];

/// Notification categories: (key, label, description, default in-app, default email, permission
/// a person needs for the category to apply to them, or "" for everyone).
pub const CATEGORIES: &[(&str, &str, &str, bool, bool, &str)] = &[
    ("assigned", "Assigned to you", "A work item or request is assigned to you", true, true, ""),
    ("mention", "Mentions", "Someone mentions you in a discussion or a page", true, true, ""),
    ("comment", "New messages", "A new message on something you reported, are assigned to or watch", true, true, ""),
    ("status", "Status changes", "Something you reported, are assigned to or watch moves to another status", true, false, ""),
    ("update", "Other changes", "Fields, dates, files or links change on something you are assigned to or watch", true, false, ""),
    ("reminder", "Reminders", "Work assigned to you is due soon or overdue", true, true, ""),
    ("desk_new", "New service desk requests", "A request is raised in a service desk you work in", true, true, ""),
    ("desk_sla", "Service-level warnings", "A request you own misses its response or resolution target", true, true, ""),
    ("search_alert", "Saved search alerts", "New results appear in a saved search you asked to be alerted about", true, true, ""),
    ("team", "Teams", "You are added to or removed from a team position", true, false, ""),
    ("admin_app", "Application alerts", "Alerts defined by application administrators", true, true, "admin.access"),
    ("admin_system", "System alerts", "Performance, warnings, errors and component health", true, true, "admin.settings"),
    ("security", "Security incidents", "Suspicious sign-in activity and other security events", true, true, "admin.security"),
];

pub fn defaults(key: &str) -> Value {
    match key {
        "site" => json!({"name": "Gantry", "announcement": "", "login_notice": "", "timezone": "UTC",
                         "support": {"name": "", "email": "", "url": "", "text": ""}}),
        "legal" => json!({"privacy": "", "terms": "", "cookies": "", "retention": "", "security": ""}),
        "profile" => json!({"user_editable": {"name": true, "org_name": true, "org_code": false, "job_title": true},
                            "avatar_max_kb": 512, "avatar_max_px": 1024}),
        "notifications" => {
            let cats: Map<String, Value> = CATEGORIES.iter().map(|c| (c.0.to_string(), json!({"in_app": c.3, "email": c.4, "locked": false}))).collect();
            json!({"email_enabled": true, "categories": cats, "reminders": {"due_soon_days": 2, "overdue_every_days": 3, "hour": 8}})
        }
        "hierarchy" => json!({"enforce_item_levels": false}),
        _ => json!({}),
    }
}

/// Lay `over` on `base`, object by object.
pub fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(slot) if slot.is_object() && v.is_object() => merge(slot, v),
                    _ => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// The effective value of a setting: stored fields over defaults.
pub async fn get(st: &AppState, key: &str) -> Value {
    let mut v = defaults(key);
    match sqlx::query_scalar::<_, Value>("SELECT value FROM settings WHERE key = $1").bind(key).fetch_optional(&st.db).await {
        Ok(Some(stored)) => merge(&mut v, &stored),
        Ok(None) => {}
        Err(e) => tracing::error!(error = %e, key, "could not read setting; using defaults"),
    }
    v
}

/// A name: one line, with nothing that changes how it displays.
fn line(o: &Map<String, Value>, k: &str, max: usize) -> AppResult<()> {
    match o.get(k) {
        None => Ok(()),
        Some(Value::String(s)) if s.chars().count() <= max && !s.chars().any(crate::accounts::deceptive) => Ok(()),
        Some(_) => Err(bad(format!("{k} must be one line of at most {max} characters"))),
    }
}

fn text(o: &Map<String, Value>, k: &str, max: usize) -> AppResult<()> {
    match o.get(k) {
        None => Ok(()),
        Some(Value::String(s)) if s.chars().count() <= max && !s.chars().any(|c| c == '\0') => Ok(()),
        Some(_) => Err(bad(format!("{k} must be text of at most {max} characters"))),
    }
}

fn only(o: &Map<String, Value>, allowed: &[&str], what: &str) -> AppResult<()> {
    match o.keys().find(|k| !allowed.contains(&k.as_str())) {
        Some(k) => Err(bad(format!("unknown {what} setting {k:?}; available: {}", allowed.join(", ")))),
        None => Ok(()),
    }
}

pub fn valid_timezone(tz: &str) -> bool {
    tz.parse::<chrono_tz::Tz>().is_ok()
}

/// A contact link may be a web address or nothing. Never `javascript:` or the like.
fn web_url(s: &str) -> bool {
    s.is_empty() || url::Url::parse(s).map(|u| matches!(u.scheme(), "http" | "https")).unwrap_or(false)
}

/// Check a value before it is stored. Unknown keys and fields are refused, so a typo cannot
/// silently do nothing.
pub fn validate(key: &str, v: &Value) -> AppResult<()> {
    let o = v.as_object().ok_or_else(|| bad(format!("{key} must be an object")))?;
    match key {
        "site" => {
            only(o, &["name", "announcement", "login_notice", "timezone", "support"], "site")?;
            line(o, "name", 60)?;
            text(o, "announcement", 500)?;
            text(o, "login_notice", 2000)?;
            if let Some(tz) = o.get("timezone") {
                if !tz.as_str().map(valid_timezone).unwrap_or(false) {
                    return Err(bad("timezone must be an IANA time zone name such as America/Chicago or UTC"));
                }
            }
            if let Some(s) = o.get("support") {
                let s = s.as_object().ok_or_else(|| bad("support must be an object"))?;
                only(s, &["name", "email", "url", "text"], "support")?;
                line(s, "name", 100)?;
                line(s, "email", 320)?;
                line(s, "url", 500)?;
                text(s, "text", 1000)?;
                if let Some(e) = s.get("email").and_then(Value::as_str).filter(|e| !e.is_empty()) {
                    if !crate::accounts::valid_email(e) {
                        return Err(bad("support email is not a valid address"));
                    }
                }
                if !s.get("url").and_then(Value::as_str).map(web_url).unwrap_or(true) {
                    return Err(bad("support url must start with https:// or http://"));
                }
            }
        }
        "legal" => {
            let keys: Vec<&str> = LEGAL_DOCS.iter().map(|d| d.0).collect();
            only(o, &keys, "legal")?;
            for k in keys {
                text(o, k, 100_000)?;
            }
        }
        "profile" => {
            only(o, &["user_editable", "avatar_max_kb", "avatar_max_px"], "profile")?;
            if let Some(e) = o.get("user_editable") {
                let e = e.as_object().ok_or_else(|| bad("user_editable must be an object"))?;
                only(e, &["name", "org_name", "org_code", "job_title"], "user_editable")?;
                if e.values().any(|v| !v.is_boolean()) {
                    return Err(bad("user_editable values must be true or false"));
                }
            }
            for (k, lo, hi) in [("avatar_max_kb", 16, 5120), ("avatar_max_px", 64, 4096)] {
                if let Some(n) = o.get(k) {
                    if !n.as_i64().map(|n| (lo..=hi).contains(&n)).unwrap_or(false) {
                        return Err(bad(format!("{k} must be a whole number between {lo} and {hi}")));
                    }
                }
            }
        }
        "notifications" => {
            only(o, &["email_enabled", "categories", "reminders"], "notifications")?;
            if o.get("email_enabled").is_some_and(|v| !v.is_boolean()) {
                return Err(bad("email_enabled must be true or false"));
            }
            if let Some(c) = o.get("categories") {
                let c = c.as_object().ok_or_else(|| bad("categories must be an object"))?;
                for (cat, rule) in c {
                    if !CATEGORIES.iter().any(|k| k.0 == cat) {
                        return Err(bad(format!("unknown notification category {cat:?}")));
                    }
                    let r = rule.as_object().ok_or_else(|| bad("each category must be an object"))?;
                    only(r, &["in_app", "email", "locked"], "category")?;
                    if r.values().any(|v| !v.is_boolean()) {
                        return Err(bad("in_app, email and locked must be true or false"));
                    }
                }
            }
            if let Some(r) = o.get("reminders") {
                let r = r.as_object().ok_or_else(|| bad("reminders must be an object"))?;
                only(r, &["due_soon_days", "overdue_every_days", "hour"], "reminders")?;
                for (k, lo, hi) in [("due_soon_days", 0, 30), ("overdue_every_days", 0, 30), ("hour", 0, 23)] {
                    if let Some(n) = r.get(k) {
                        if !n.as_i64().map(|n| (lo..=hi).contains(&n)).unwrap_or(false) {
                            return Err(bad(format!("{k} must be a whole number between {lo} and {hi}")));
                        }
                    }
                }
            }
        }
        "hierarchy" => {
            only(o, &["enforce_item_levels"], "hierarchy")?;
            if o.values().any(|v| !v.is_boolean()) {
                return Err(bad("enforce_item_levels must be true or false"));
            }
        }
        _ => return Err(bad(format!("unknown setting; available: {}", KEYS.iter().map(|k| k.key).collect::<Vec<_>>().join(", ")))),
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_pass_their_own_validation() {
        for k in KEYS {
            assert!(validate(k.key, &defaults(k.key)).is_ok(), "{}", k.key);
            assert!(crate::authz::is_known_permission(k.permission), "{}", k.key);
        }
    }

    #[test]
    fn stored_values_are_laid_over_defaults() {
        let mut v = defaults("profile");
        merge(&mut v, &json!({"user_editable": {"org_code": true}, "avatar_max_kb": 100}));
        assert_eq!(v["user_editable"]["org_code"], true);
        assert_eq!(v["user_editable"]["name"], true, "fields that were not stored keep their default");
        assert_eq!(v["avatar_max_kb"], 100);
        assert_eq!(v["avatar_max_px"], 1024);
    }

    #[test]
    fn unknown_fields_and_bad_values_are_refused() {
        assert!(validate("site", &json!({"nmae": "x"})).is_err());
        assert!(validate("site", &json!({"timezone": "Mars/Olympus"})).is_err());
        assert!(validate("site", &json!({"timezone": "America/Chicago"})).is_ok());
        assert!(validate("site", &json!({"support": {"url": "javascript:alert(1)"}})).is_err());
        assert!(validate("site", &json!({"support": {"url": "https://help.example.org", "email": "help@example.org"}})).is_ok());
        assert!(validate("site", &json!({"support": {"email": "not an address"}})).is_err());
        assert!(validate("profile", &json!({"avatar_max_kb": 1})).is_err());
        assert!(validate("profile", &json!({"user_editable": {"email": true}})).is_err());
        assert!(validate("notifications", &json!({"categories": {"nonsense": {"email": true}}})).is_err());
        assert!(validate("notifications", &json!({"categories": {"mention": {"email": "yes"}}})).is_err());
        assert!(validate("notifications", &json!({"reminders": {"hour": 24}})).is_err());
        assert!(validate("nope", &json!({})).is_err());
        assert!(validate("site", &json!("text")).is_err());
    }

    #[test]
    fn notification_categories_are_unique_and_name_real_permissions() {
        let mut seen = std::collections::HashSet::new();
        for c in CATEGORIES {
            assert!(seen.insert(c.0), "duplicate {}", c.0);
            assert!(c.5.is_empty() || crate::authz::is_known_permission(c.5), "{}", c.0);
        }
    }
}
