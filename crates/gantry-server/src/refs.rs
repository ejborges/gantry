//! References inside Markdown text: `@handle` mentions, work item keys such as `OPS-12`, and
//! `[[wikilinks]]` to items, projects and pages.
//!
//! A reference becomes a link only when two checks pass:
//!
//! 1. when the text is **saved**, the author must be allowed to see the target. What passes is
//!    stored next to the text (`refs` column) as `name → id`;
//! 2. when the text is **shown**, the viewer must be allowed to see the target too. Stored
//!    references the viewer may not see are left out, and the name stays plain text.
//!
//! So nobody can link to what they cannot see, and a link never tells a viewer more than the
//! API would tell them directly.

use crate::auth::Ctx;
use crate::error::AppResult;
use crate::http::Api;
use crate::{authz, projects, AppState, St};
use axum::extract::State;
use axum::routing::post;
use axum::Json;
use serde::Deserialize;
use serde_json::{json, Map, Value};
use sqlx::Row;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::OnceLock;
use uuid::Uuid;

const MAX_PER_KIND: usize = 60;

#[derive(Debug, Default, PartialEq)]
pub struct Found {
    pub handles: Vec<String>,
    pub item_keys: Vec<String>,
    /// Targets of `[[...]]`, without the `|label` and `#heading` parts.
    pub wikilinks: Vec<String>,
}

fn re(cell: &'static OnceLock<regex::Regex>, pattern: &str) -> &'static regex::Regex {
    cell.get_or_init(|| regex::Regex::new(pattern).expect("valid regex"))
}

/// Blank out fenced code blocks and inline code, where nothing is a reference.
pub fn without_code(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut fence: Option<(char, usize)> = None;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_start();
        let marker = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        let run = marker.map(|m| trimmed.chars().take_while(|c| *c == m).count()).unwrap_or(0);
        match fence {
            Some((m, n)) => {
                if marker == Some(m) && run >= n && trimmed[run..].trim().is_empty() {
                    fence = None;
                }
                out.push('\n');
            }
            None if run >= 3 && line.len() - trimmed.len() <= 3 => {
                fence = marker.map(|m| (m, run));
                out.push('\n');
            }
            None => {
                // Inline code: a run of backticks up to the next run of the same length.
                let chars: Vec<char> = line.chars().collect();
                let mut i = 0;
                while i < chars.len() {
                    if chars[i] == '`' {
                        let n = chars[i..].iter().take_while(|c| **c == '`').count();
                        let mut j = i + n;
                        let mut close = None;
                        while j < chars.len() {
                            if chars[j] == '`' {
                                let m = chars[j..].iter().take_while(|c| **c == '`').count();
                                if m == n {
                                    close = Some(j + m);
                                    break;
                                }
                                j += m;
                            } else {
                                j += 1;
                            }
                        }
                        match close {
                            Some(end) => {
                                out.push(' ');
                                i = end;
                            }
                            None => {
                                out.extend(&chars[i..i + n]);
                                i += n;
                            }
                        }
                    } else {
                        out.push(chars[i]);
                        i += 1;
                    }
                }
            }
        }
    }
    out
}

/// Find the references written in `text`. Purely textual: nothing is looked up.
pub fn scan(text: &str) -> Found {
    static MENTION: OnceLock<regex::Regex> = OnceLock::new();
    static KEY: OnceLock<regex::Regex> = OnceLock::new();
    static WIKI: OnceLock<regex::Regex> = OnceLock::new();
    let plain = without_code(text);
    let mut f = Found::default();
    let push = |v: &mut Vec<String>, s: String| {
        if v.len() < MAX_PER_KIND && !v.contains(&s) {
            v.push(s);
        }
    };
    for c in re(&WIKI, r"\[\[([^\[\]\n|#]{1,200})(?:#[^\[\]\n|]*)?(?:\|[^\[\]\n]*)?\]\]").captures_iter(&plain) {
        push(&mut f.wikilinks, c[1].trim().to_string());
    }
    for c in re(&MENTION, r"(?:^|[^\w@/.\-])@([A-Za-z0-9][A-Za-z0-9._\-]{1,38})").captures_iter(&plain) {
        let h = c[1].trim_end_matches(['.', '-', '_']).to_lowercase();
        if h.len() >= 2 {
            push(&mut f.handles, h);
        }
    }
    for c in re(&KEY, r"\b([A-Z][A-Z0-9]{1,9}-[0-9]{1,9})\b").captures_iter(&plain) {
        push(&mut f.item_keys, c[1].to_string());
    }
    f
}

fn ids_of(stored: &Value, kind: &str) -> Vec<(String, Uuid)> {
    stored[kind].as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_str()?.parse().ok()?))).collect()).unwrap_or_default()
}

/// The accounts a stored reference set mentions.
pub fn mentioned(stored: &Value) -> Vec<Uuid> {
    ids_of(stored, "users").into_iter().map(|u| u.1).collect()
}

/// Check the references in `text` against what the author may see and return the set to
/// store. `space` is where a page title in a wikilink is looked up first.
pub async fn check(st: &St, ctx: &Ctx, text: &str, space: Option<Uuid>) -> AppResult<Value> {
    let found = scan(text);
    let mut out = json!({"users": {}, "items": {}, "pages": {}, "projects": {}});
    if found.handles.is_empty() && found.item_keys.is_empty() && found.wikilinks.is_empty() {
        return Ok(json!({}));
    }
    // Mentions name accounts, so they need the directory.
    if !found.handles.is_empty() && ctx.has("workspace.access") {
        for r in sqlx::query("SELECT handle, id FROM users WHERE handle = ANY($1) AND active").bind(&found.handles).fetch_all(&st.db).await? {
            out["users"][r.get::<String, _>("handle")] = json!(r.get::<Uuid, _>("id"));
        }
    }
    let mut keys = found.item_keys.clone();
    let mut project_keys = vec![];
    let mut titles = vec![];
    for w in &found.wikilinks {
        let upper = w.to_uppercase();
        if crate::items::parse_key(w).is_some() {
            if !keys.contains(&upper) {
                keys.push(upper);
            }
        } else if projects::valid_project_key(w) {
            project_keys.push(w.clone());
        } else {
            titles.push(w.clone());
        }
    }
    if !keys.is_empty() || !project_keys.is_empty() {
        let visible = authz::visible_project_ids(st, ctx).await?;
        if !keys.is_empty() {
            for r in sqlx::query("SELECT p.key || '-' || i.number AS key, i.id FROM items i JOIN projects p ON p.id = i.project_id WHERE p.key || '-' || i.number = ANY($1) AND i.project_id = ANY($2) AND NOT i.archived").bind(&keys).bind(&visible).fetch_all(&st.db).await? {
                out["items"][r.get::<String, _>("key")] = json!(r.get::<Uuid, _>("id"));
            }
        }
        if !project_keys.is_empty() {
            for r in sqlx::query("SELECT key, id FROM projects WHERE key = ANY($1) AND id = ANY($2)").bind(&project_keys).bind(&visible).fetch_all(&st.db).await? {
                out["projects"][r.get::<String, _>("key")] = json!(r.get::<Uuid, _>("id"));
            }
        }
    }
    if !titles.is_empty() {
        let visible = authz::visible_space_ids(st, ctx).await?;
        let lowered: Vec<String> = titles.iter().map(|t| t.to_lowercase()).collect();
        let rows = sqlx::query("SELECT lower(title) AS t, id, space_id FROM pages WHERE lower(title) = ANY($1) AND space_id = ANY($2) AND NOT archived AND status = 'published' ORDER BY updated_at DESC").bind(&lowered).bind(&visible).fetch_all(&st.db).await?;
        for title in &titles {
            let matches: Vec<&sqlx::postgres::PgRow> = rows.iter().filter(|r| r.get::<String, _>("t") == title.to_lowercase()).collect();
            // A title in the same space wins; elsewhere it must be the only page with that title.
            let pick = matches.iter().find(|r| Some(r.get::<Uuid, _>("space_id")) == space).or(if matches.len() == 1 { matches.first() } else { None });
            if let Some(r) = pick {
                out["pages"][title] = json!(r.get::<Uuid, _>("id"));
            }
        }
    }
    Ok(out)
}

/// Who may appear when mentions are resolved for a viewer.
pub enum People<'a> {
    /// Anyone, if the viewer has the directory.
    Directory,
    /// Only these accounts (the people already taking part in a customer's request).
    Only(&'a [Uuid]),
}

/// Turn stored reference sets into what the viewer may see, one result per input, in order.
/// Each result maps the name as written to a small description of the target.
pub async fn resolve(st: &AppState, ctx: &Ctx, stored: &[&Value], people: People<'_>) -> AppResult<Vec<Value>> {
    let all = |kind: &str| -> Vec<Uuid> { stored.iter().flat_map(|s| ids_of(s, kind)).map(|x| x.1).collect::<HashSet<_>>().into_iter().collect() };
    let (user_ids, item_ids, page_ids, project_ids) = (all("users"), all("items"), all("pages"), all("projects"));
    let mut by_id: HashMap<Uuid, Value> = HashMap::new();
    if !user_ids.is_empty() {
        let allowed: Vec<Uuid> = match people {
            People::Directory if ctx.has("workspace.access") => user_ids,
            People::Directory => vec![],
            People::Only(list) => user_ids.into_iter().filter(|u| list.contains(u)).collect(),
        };
        for r in sqlx::query("SELECT u.id, person_json(u) AS p FROM users u WHERE u.id = ANY($1)").bind(&allowed).fetch_all(&st.db).await? {
            by_id.insert(r.get("id"), r.get("p"));
        }
    }
    if !item_ids.is_empty() || !project_ids.is_empty() {
        let visible = authz::visible_project_ids(st, ctx).await?;
        if !item_ids.is_empty() {
            let rows = sqlx::query(
                "SELECT i.id, jsonb_build_object('key', p.key || '-' || i.number, 'title', i.title, 'status_category', i.status_category, 'type', i.type_key, 'type_name', ty.name,
                    'status_name', item_status_name(i.project_id, i.type_key, i.status),
                    'url', '/i/' || p.key || '-' || i.number) AS j
                 FROM items i JOIN projects p ON p.id = i.project_id JOIN item_types ty ON ty.key = i.type_key WHERE i.id = ANY($1) AND i.project_id = ANY($2) AND NOT i.archived",
            )
            .bind(&item_ids)
            .bind(&visible)
            .fetch_all(&st.db)
            .await?;
            for r in rows {
                by_id.insert(r.get("id"), r.get("j"));
            }
        }
        if !project_ids.is_empty() {
            for r in sqlx::query("SELECT id, jsonb_build_object('key', key, 'name', name, 'url', '/projects/' || key) AS j FROM projects WHERE id = ANY($1) AND id = ANY($2)").bind(&project_ids).bind(&visible).fetch_all(&st.db).await? {
                by_id.insert(r.get("id"), r.get("j"));
            }
        }
    }
    if !page_ids.is_empty() {
        let visible = authz::visible_space_ids(st, ctx).await?;
        for r in sqlx::query("SELECT pg.id, jsonb_build_object('id', pg.id, 'title', pg.title, 'url', '/docs/' || s.key || '/' || pg.id) AS j FROM pages pg JOIN spaces s ON s.id = pg.space_id WHERE pg.id = ANY($1) AND pg.space_id = ANY($2) AND NOT pg.archived AND pg.status = 'published'").bind(&page_ids).bind(&visible).fetch_all(&st.db).await? {
            by_id.insert(r.get("id"), r.get("j"));
        }
    }
    Ok(stored
        .iter()
        .map(|s| {
            let mut out = Map::new();
            for kind in ["users", "items", "pages", "projects"] {
                let m: BTreeMap<String, Value> = ids_of(s, kind).into_iter().filter_map(|(name, id)| Some((name, by_id.get(&id)?.clone()))).collect();
                if !m.is_empty() {
                    out.insert(kind.into(), json!(m));
                }
            }
            Value::Object(out)
        })
        .collect())
}

#[derive(Deserialize)]
struct PreviewBody {
    text: String,
    /// Key of the space the text belongs to, so page titles resolve there first.
    space: Option<String>,
}

/// What the references in a draft would link to, for the editor's preview. Both checks are
/// made as the caller, who is the author and the viewer at once.
async fn preview(State(st): State<St>, ctx: Ctx, Json(b): Json<PreviewBody>) -> AppResult<Json<Value>> {
    if b.text.len() > 1_000_000 {
        return Err(crate::error::bad("text is too long"));
    }
    let space = match &b.space {
        Some(k) => sqlx::query_scalar("SELECT id FROM spaces WHERE key = $1").bind(k.to_uppercase()).fetch_optional(&st.db).await?,
        None => None,
    };
    let stored = check(&st, &ctx, &b.text, space).await?;
    let resolved = resolve(&st, &ctx, &[&stored], People::Directory).await?;
    Ok(Json(resolved.into_iter().next().unwrap_or(json!({}))))
}

pub fn routes(api: Api) -> Api {
    api.route("POST", "/api/v1/refs/preview", "References", "Resolve the mentions and links in draft text, as the caller may see them", post(preview))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_mentions_keys_and_wikilinks() {
        let f = scan("Thanks @ada.lovelace, see OPS-12 and [[ENG-3]]. cc @Ben-2 (@chen), [[Runbook: Pump|the runbook]] and [[OPS]].\n@pat");
        assert_eq!(f.handles, vec!["ada.lovelace", "ben-2", "chen", "pat"]);
        assert_eq!(f.item_keys, vec!["OPS-12", "ENG-3"]);
        assert_eq!(f.wikilinks, vec!["ENG-3", "Runbook: Pump", "OPS"]);
    }

    #[test]
    fn email_addresses_urls_and_code_are_not_references() {
        let f = scan("mail ada@example.org or https://x.test/@ben and a.b@c.d\n`@chen OPS-1`\n```\n@pat OPS-2 [[OPS-3]]\n```\n~~~\n@pat\n~~~\ntext ``code with ` tick @eve`` @real OPS-9");
        assert_eq!(f.handles, vec!["real"]);
        assert_eq!(f.item_keys, vec!["OPS-9"]);
        assert!(f.wikilinks.is_empty());
    }

    #[test]
    fn an_unclosed_backtick_does_not_hide_the_rest() {
        let f = scan("a ` lone tick then @ada and OPS-4");
        assert_eq!(f.handles, vec!["ada"]);
        assert_eq!(f.item_keys, vec!["OPS-4"]);
    }

    #[test]
    fn hostile_input_is_bounded_and_inert() {
        let many: String = (0..500).map(|i| format!("@user{i} OPS-{i} [[Page {i}]] ")).collect();
        let f = scan(&many);
        assert_eq!((f.handles.len(), f.item_keys.len(), f.wikilinks.len()), (MAX_PER_KIND, MAX_PER_KIND, MAX_PER_KIND));
        let f = scan("@'; DROP TABLE users;-- [[<script>alert(1)</script>]] OPS-1' OR '1'='1");
        assert!(f.handles.is_empty());
        assert_eq!(f.wikilinks, vec!["<script>alert(1)</script>"], "kept as text; it is only ever compared with page titles");
        assert_eq!(f.item_keys, vec!["OPS-1"]);
        // Deeply nested brackets and long runs finish quickly.
        let _ = scan(&"[".repeat(20_000));
        let _ = scan(&"`".repeat(20_000));
        let _ = scan(&"@".repeat(20_000));
    }

    #[test]
    fn stored_sets_are_read_defensively() {
        let id = Uuid::new_v4();
        assert_eq!(mentioned(&json!({"users": {"ada": id.to_string(), "bad": "nope", "n": 5}})), vec![id]);
        assert!(mentioned(&json!({})).is_empty() && mentioned(&json!(null)).is_empty() && mentioned(&json!({"users": []})).is_empty());
    }
}
