//! Packs: declarative bundles of fields, workflows, work item types, project templates and
//! page templates. The core pack is compiled into the binary; add-ons live on disk at
//! `<addons.dir>/<id>/pack.json` (installed by optional RPMs) and are applied from the
//! administration console or with `gantry pack install <id>`.
//!
//! Everything a pack defines is plain data in the database afterward, so administrators can
//! adjust it. Reinstalling or upgrading a pack refreshes only the objects that pack still owns
//! (`source = <pack id>`); anything an administrator has edited is marked `source = 'custom'`
//! and is left alone, as is anything owned by another pack.

use crate::error::{bad, AppError, AppResult};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

pub const CORE_PACK: &str = include_str!("../packs/core.json");
const FIELD_TYPES: &[&str] = &["text", "textarea", "number", "date", "select", "multiselect", "user", "url", "checkbox"];
const CATEGORIES: &[&str] = &["todo", "in_progress", "done"];

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pack {
    pub id: String,
    pub name: String,
    pub version: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub fields: Vec<FieldDef>,
    #[serde(default)]
    pub workflows: Vec<WorkflowDef>,
    #[serde(default)]
    pub item_types: Vec<TypeDef>,
    #[serde(default)]
    pub project_templates: Vec<TemplateDef>,
    #[serde(default)]
    pub page_templates: Vec<PageTemplateDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldDef {
    pub key: String,
    pub name: String,
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StatusDef {
    pub key: String,
    pub name: String,
    pub category: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransitionDef {
    pub name: String,
    /// Source statuses; `["*"]` means any status.
    pub from: Vec<String>,
    pub to: String,
    /// Field keys that must be filled in before the transition is allowed.
    #[serde(default)]
    pub requires: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDef {
    #[serde(default)]
    pub key: String,
    #[serde(default)]
    pub name: String,
    pub initial: String,
    pub statuses: Vec<StatusDef>,
    pub transitions: Vec<TransitionDef>,
    /// Where the visual editor drew each status: `{"<status key>": {"x": 0, "y": 0}}`.
    /// Has no effect on how the workflow behaves.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub layout: Option<Value>,
}

impl WorkflowDef {
    pub fn status(&self, key: &str) -> Option<&StatusDef> {
        self.statuses.iter().find(|s| s.key == key)
    }
    /// Transitions available from `from`, excluding no-op moves to the same status.
    pub fn available(&self, from: &str) -> Vec<&TransitionDef> {
        self.transitions.iter().filter(|t| t.to != from && t.from.iter().any(|f| f == "*" || f == from)).collect()
    }
    pub fn transition(&self, from: &str, to: &str) -> Option<&TransitionDef> {
        self.available(from).into_iter().find(|t| t.to == to)
    }
    pub fn first_in_category(&self, category: &str) -> Option<&StatusDef> {
        self.statuses.iter().find(|s| s.category == category)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.statuses.is_empty() {
            return Err("a workflow needs at least one status".into());
        }
        let mut keys = HashSet::new();
        for s in &self.statuses {
            if !valid_key(&s.key) {
                return Err(format!("status key {:?} must be lowercase letters, digits and underscores", s.key));
            }
            if !keys.insert(s.key.as_str()) {
                return Err(format!("duplicate status {:?}", s.key));
            }
            if !CATEGORIES.contains(&s.category.as_str()) {
                return Err(format!("status {:?} has unknown category {:?}", s.key, s.category));
            }
            if s.name.trim().is_empty() {
                return Err(format!("status {:?} needs a name", s.key));
            }
        }
        if !keys.contains(self.initial.as_str()) {
            return Err(format!("initial status {:?} is not defined", self.initial));
        }
        if self.statuses.len() > 60 || self.transitions.len() > 400 {
            return Err("a workflow can have at most 60 statuses and 400 transitions".into());
        }
        if let Some(l) = &self.layout {
            let ok = l.as_object().is_some_and(|o| o.len() <= 60 && o.iter().all(|(k, v)| keys.contains(k.as_str()) && v["x"].is_number() && v["y"].is_number() && v.as_object().is_some_and(|p| p.len() == 2)));
            if !ok {
                return Err("layout must give an x and y position for statuses of this workflow".into());
            }
        }
        if !self.statuses.iter().any(|s| s.category == "done") {
            return Err("a workflow needs at least one status in the done category".into());
        }
        for t in &self.transitions {
            if !keys.contains(t.to.as_str()) {
                return Err(format!("transition {:?} leads to unknown status {:?}", t.name, t.to));
            }
            if t.from.is_empty() {
                return Err(format!("transition {:?} has no source status", t.name));
            }
            for f in &t.from {
                if f != "*" && !keys.contains(f.as_str()) {
                    return Err(format!("transition {:?} starts from unknown status {f:?}", t.name));
                }
            }
        }
        // Every status must be reachable from the initial one, or work could never get there.
        let mut reached: HashSet<&str> = [self.initial.as_str()].into();
        loop {
            let before = reached.len();
            for t in &self.transitions {
                if t.from.iter().any(|f| f == "*" || reached.contains(f.as_str())) {
                    reached.insert(t.to.as_str());
                }
            }
            if reached.len() == before {
                break;
            }
        }
        if let Some(s) = self.statuses.iter().find(|s| !reached.contains(s.key.as_str())) {
            return Err(format!("status {:?} cannot be reached from the initial status", s.key));
        }
        Ok(())
    }

    /// The JSON stored in `workflows.definition`.
    pub fn definition(&self) -> Value {
        let mut d = json!({"initial": self.initial, "statuses": self.statuses, "transitions": self.transitions});
        if let Some(l) = &self.layout {
            d["layout"] = l.clone();
        }
        d
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeField {
    pub key: String,
    #[serde(default)]
    pub required: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TypeDef {
    pub key: String,
    pub name: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(default = "default_color")]
    pub color: String,
    #[serde(default)]
    pub description: String,
    pub workflow: String,
    #[serde(default)]
    pub fields: Vec<TypeField>,
}
fn default_icon() -> String {
    "circle".into()
}
fn default_color() -> String {
    "neutral".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestTypeDef {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "default_icon")]
    pub icon: String,
    #[serde(rename = "type")]
    pub type_key: String,
    #[serde(default)]
    pub fields: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SlaDef {
    pub priority: i16,
    pub response_minutes: i32,
    pub resolution_minutes: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedItem {
    #[serde(rename = "type")]
    pub type_key: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub custom: serde_json::Map<String, Value>,
    #[serde(default)]
    pub children: Vec<SeedItem>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedPage {
    pub title: String,
    #[serde(default)]
    pub body: Option<String>,
    /// Key of a page template to copy the body from.
    #[serde(default)]
    pub template: Option<String>,
    #[serde(default)]
    pub customer_visible: bool,
    #[serde(default)]
    pub children: Vec<SeedPage>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SeedSpace {
    pub name: String,
    #[serde(default = "default_space_kind")]
    pub kind: String,
    #[serde(default)]
    pub pages: Vec<SeedPage>,
}
fn default_space_kind() -> String {
    "wiki".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TemplateDef {
    pub key: String,
    pub name: String,
    #[serde(default = "default_domain")]
    pub domain: String,
    #[serde(default = "default_kind")]
    pub kind: String,
    #[serde(default)]
    pub description: String,
    pub item_types: Vec<String>,
    pub default_type: String,
    #[serde(default)]
    pub portal_open: bool,
    #[serde(default)]
    pub request_types: Vec<RequestTypeDef>,
    #[serde(default)]
    pub sla: Vec<SlaDef>,
    #[serde(default)]
    pub cycles: Vec<String>,
    #[serde(default)]
    pub items: Vec<SeedItem>,
    #[serde(default)]
    pub space: Option<SeedSpace>,
}
fn default_domain() -> String {
    "General".into()
}
fn default_kind() -> String {
    "standard".into()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PageTemplateDef {
    pub key: String,
    pub name: String,
    #[serde(default = "default_domain")]
    pub category: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub body: Option<String>,
    /// Path of a Markdown file relative to the pack directory, as an alternative to `body`.
    #[serde(default)]
    pub file: Option<String>,
}

pub fn valid_key(k: &str) -> bool {
    let mut chars = k.chars();
    matches!(chars.next(), Some('a'..='z')) && k.len() <= 63 && chars.all(|c| matches!(c, 'a'..='z' | '0'..='9' | '_'))
}

impl Pack {
    pub fn parse(text: &str) -> Result<Pack, String> {
        let pack: Pack = serde_json::from_str(text).map_err(|e| format!("pack is not valid: {e}"))?;
        pack.validate(&HashSet::new(), &HashSet::new(), &HashSet::new())?;
        Ok(pack)
    }

    /// Check internal consistency. The `known_*` sets hold keys that already exist in the
    /// database (from other packs), which this pack is allowed to reference.
    pub fn validate(&self, known_fields: &HashSet<String>, known_workflows: &HashSet<String>, known_types: &HashSet<String>) -> Result<(), String> {
        if !self.id.chars().all(|c| matches!(c, 'a'..='z' | '0'..='9' | '-')) || self.id.is_empty() || self.id.len() > 40 {
            return Err("pack id must be 1-40 lowercase letters, digits or hyphens".into());
        }
        let mut fields: HashSet<String> = known_fields.clone();
        for f in &self.fields {
            if !valid_key(&f.key) {
                return Err(format!("field key {:?} is not valid", f.key));
            }
            if !FIELD_TYPES.contains(&f.kind.as_str()) {
                return Err(format!("field {:?} has unknown type {:?}", f.key, f.kind));
            }
            if matches!(f.kind.as_str(), "select" | "multiselect") && f.options.is_empty() {
                return Err(format!("field {:?} needs options", f.key));
            }
            fields.insert(f.key.clone());
        }
        let mut workflows: HashSet<String> = known_workflows.clone();
        for w in &self.workflows {
            if !valid_key(&w.key) {
                return Err(format!("workflow key {:?} is not valid", w.key));
            }
            w.validate().map_err(|e| format!("workflow {:?}: {e}", w.key))?;
            for t in &w.transitions {
                if let Some(missing) = t.requires.iter().find(|r| !fields.contains(*r)) {
                    return Err(format!("workflow {:?}: transition {:?} requires unknown field {missing:?}", w.key, t.name));
                }
            }
            workflows.insert(w.key.clone());
        }
        let mut types: HashSet<String> = known_types.clone();
        for t in &self.item_types {
            if !valid_key(&t.key) {
                return Err(format!("item type key {:?} is not valid", t.key));
            }
            if !workflows.contains(&t.workflow) {
                return Err(format!("item type {:?} uses unknown workflow {:?}", t.key, t.workflow));
            }
            if let Some(missing) = t.fields.iter().find(|f| !fields.contains(&f.key)) {
                return Err(format!("item type {:?} uses unknown field {:?}", t.key, missing.key));
            }
            types.insert(t.key.clone());
        }
        let page_templates: HashSet<&str> = self.page_templates.iter().map(|p| p.key.as_str()).collect();
        for t in &self.project_templates {
            if !["standard", "service_desk"].contains(&t.kind.as_str()) {
                return Err(format!("project template {:?} has unknown kind {:?}", t.key, t.kind));
            }
            if !t.item_types.contains(&t.default_type) {
                return Err(format!("project template {:?}: default type {:?} is not among its item types", t.key, t.default_type));
            }
            if let Some(missing) = t.item_types.iter().find(|k| !types.contains(*k)) {
                return Err(format!("project template {:?} uses unknown item type {missing:?}", t.key));
            }
            for r in &t.request_types {
                if !t.item_types.contains(&r.type_key) {
                    return Err(format!("project template {:?}: request type {:?} maps to a type the project does not use", t.key, r.name));
                }
                if let Some(missing) = r.fields.iter().find(|f| !fields.contains(*f)) {
                    return Err(format!("project template {:?}: request type {:?} uses unknown field {missing:?}", t.key, r.name));
                }
            }
            for s in &t.sla {
                if !(0..=4).contains(&s.priority) || s.response_minutes <= 0 || s.resolution_minutes <= 0 {
                    return Err(format!("project template {:?} has an invalid SLA row", t.key));
                }
            }
            fn check_items(items: &[SeedItem], allowed: &[String], tpl: &str) -> Result<(), String> {
                for i in items {
                    if !allowed.contains(&i.type_key) {
                        return Err(format!("project template {tpl:?}: seed item {:?} has a type the project does not use", i.title));
                    }
                    check_items(&i.children, allowed, tpl)?;
                }
                Ok(())
            }
            check_items(&t.items, &t.item_types, &t.key)?;
            fn check_pages(pages: &[SeedPage], known: &HashSet<&str>, tpl: &str) -> Result<(), String> {
                for p in pages {
                    if let Some(k) = &p.template {
                        if !known.contains(k.as_str()) {
                            return Err(format!("project template {tpl:?}: page {:?} refers to unknown page template {k:?}", p.title));
                        }
                    }
                    check_pages(&p.children, known, tpl)?;
                }
                Ok(())
            }
            if let Some(space) = &t.space {
                if !["wiki", "knowledge_base"].contains(&space.kind.as_str()) {
                    return Err(format!("project template {:?} has unknown space kind {:?}", t.key, space.kind));
                }
                check_pages(&space.pages, &page_templates, &t.key)?;
            }
        }
        for p in &self.page_templates {
            if p.body.is_some() == p.file.is_some() {
                return Err(format!("page template {:?} needs exactly one of body or file", p.key));
            }
        }
        Ok(())
    }
}

/// Read a page-template file, refusing anything that escapes the pack directory.
fn read_pack_file(dir: &Path, rel: &str) -> Result<String, String> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() || rel_path.components().any(|c| !matches!(c, std::path::Component::Normal(_))) {
        return Err(format!("pack file path {rel:?} must be relative and stay inside the pack directory"));
    }
    std::fs::read_to_string(dir.join(rel_path)).map_err(|e| format!("cannot read pack file {rel:?}: {e}"))
}

async fn keys(db: &sqlx::PgPool, table: &str) -> sqlx::Result<HashSet<String>> {
    Ok(sqlx::query_scalar::<_, String>(&format!("SELECT key FROM {table}")).fetch_all(db).await?.into_iter().collect())
}

/// Apply a pack to the database. Idempotent.
pub async fn install(db: &sqlx::PgPool, pack: &Pack, dir: Option<&Path>) -> AppResult<()> {
    pack.validate(&keys(db, "fields").await?, &keys(db, "workflows").await?, &keys(db, "item_types").await?).map_err(bad)?;
    let mut tx = db.begin().await?;
    for f in &pack.fields {
        sqlx::query("INSERT INTO fields (key, name, type, options, description, source) VALUES ($1,$2,$3,$4,$5,$6)
             ON CONFLICT (key) DO UPDATE SET name = EXCLUDED.name, type = EXCLUDED.type, options = EXCLUDED.options, description = EXCLUDED.description WHERE fields.source = EXCLUDED.source")
            .bind(&f.key).bind(&f.name).bind(&f.kind).bind(json!(f.options)).bind(&f.description).bind(&pack.id)
            .execute(&mut *tx).await?;
    }
    for w in &pack.workflows {
        sqlx::query("INSERT INTO workflows (key, name, definition, source) VALUES ($1,$2,$3,$4)
             ON CONFLICT (key) DO UPDATE SET name = EXCLUDED.name, definition = EXCLUDED.definition, updated_at = now() WHERE workflows.source = EXCLUDED.source")
            .bind(&w.key).bind(&w.name).bind(w.definition()).bind(&pack.id)
            .execute(&mut *tx).await?;
    }
    for t in &pack.item_types {
        sqlx::query("INSERT INTO item_types (key, name, icon, color, description, workflow_key, fields, source) VALUES ($1,$2,$3,$4,$5,$6,$7,$8)
             ON CONFLICT (key) DO UPDATE SET name = EXCLUDED.name, icon = EXCLUDED.icon, color = EXCLUDED.color, description = EXCLUDED.description,
               workflow_key = EXCLUDED.workflow_key, fields = EXCLUDED.fields WHERE item_types.source = EXCLUDED.source")
            .bind(&t.key).bind(&t.name).bind(&t.icon).bind(&t.color).bind(&t.description).bind(&t.workflow).bind(json!(t.fields)).bind(&pack.id)
            .execute(&mut *tx).await?;
    }
    for t in &pack.project_templates {
        sqlx::query("INSERT INTO project_templates (key, name, domain, kind, description, definition, source) VALUES ($1,$2,$3,$4,$5,$6,$7)
             ON CONFLICT (key) DO UPDATE SET name = EXCLUDED.name, domain = EXCLUDED.domain, kind = EXCLUDED.kind, description = EXCLUDED.description, definition = EXCLUDED.definition WHERE project_templates.source = EXCLUDED.source")
            .bind(&t.key).bind(&t.name).bind(&t.domain).bind(&t.kind).bind(&t.description).bind(serde_json::to_value(t).map_err(|e| AppError::Other(e.into()))?).bind(&pack.id)
            .execute(&mut *tx).await?;
    }
    for p in &pack.page_templates {
        let body = match (&p.body, &p.file) {
            (Some(b), _) => b.clone(),
            (None, Some(f)) => read_pack_file(dir.ok_or_else(|| bad("this pack refers to files but has no directory"))?, f).map_err(bad)?,
            (None, None) => String::new(),
        };
        sqlx::query("INSERT INTO page_templates (key, name, category, description, body, source) VALUES ($1,$2,$3,$4,$5,$6)
             ON CONFLICT (key) DO UPDATE SET name = EXCLUDED.name, category = EXCLUDED.category, description = EXCLUDED.description, body = EXCLUDED.body WHERE page_templates.source = EXCLUDED.source")
            .bind(&p.key).bind(&p.name).bind(&p.category).bind(&p.description).bind(body).bind(&pack.id)
            .execute(&mut *tx).await?;
    }
    let summary = json!({"fields": pack.fields.len(), "workflows": pack.workflows.len(), "item_types": pack.item_types.len(), "project_templates": pack.project_templates.len(), "page_templates": pack.page_templates.len()});
    sqlx::query("INSERT INTO packs (id, name, version, description, manifest) VALUES ($1,$2,$3,$4,$5)
         ON CONFLICT (id) DO UPDATE SET name = EXCLUDED.name, version = EXCLUDED.version, description = EXCLUDED.description, manifest = EXCLUDED.manifest, installed_at = now()")
        .bind(&pack.id).bind(&pack.name).bind(&pack.version).bind(&pack.description).bind(summary)
        .execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn install_core(db: &sqlx::PgPool) -> anyhow::Result<()> {
    let pack = Pack::parse(CORE_PACK).map_err(|e| anyhow::anyhow!("built-in core pack: {e}"))?;
    install(db, &pack, None).await.map_err(|e| anyhow::anyhow!("installing core pack: {e}"))
}

pub struct Discovered {
    pub dir: PathBuf,
    pub pack: Result<Pack, String>,
    pub id: String,
}

/// Find add-on packs on disk. A pack that fails to parse is reported, not hidden.
pub fn discover(addons_dir: &Path) -> Vec<Discovered> {
    let Ok(entries) = std::fs::read_dir(addons_dir) else { return vec![] };
    let mut found: Vec<Discovered> = entries
        .filter_map(|e| e.ok())
        .filter(|e| e.path().join("pack.json").is_file())
        .map(|e| {
            let dir = e.path();
            let id = e.file_name().to_string_lossy().into_owned();
            let pack = std::fs::read_to_string(dir.join("pack.json")).map_err(|e| e.to_string()).and_then(|t| serde_json::from_str::<Pack>(&t).map_err(|e| e.to_string())).and_then(|p| {
                if p.id == id {
                    Ok(p)
                } else {
                    Err(format!("pack id {:?} does not match its directory name {id:?}", p.id))
                }
            });
            Discovered { dir, pack, id }
        })
        .collect();
    found.sort_by(|a, b| a.id.cmp(&b.id));
    found
}

pub async fn install_addon(db: &sqlx::PgPool, addons_dir: &Path, id: &str) -> AppResult<Pack> {
    let d = discover(addons_dir).into_iter().find(|d| d.id == id).ok_or(AppError::NotFound)?;
    let pack = d.pack.map_err(bad)?;
    install(db, &pack, Some(&d.dir)).await?;
    Ok(pack)
}

/// One add-on pack brought up to date by [`sync_addons`].
pub struct Synced {
    pub pack: Pack,
    /// The version that was installed before, if any. `None` means newly installed.
    pub previous: Option<String>,
}

impl Synced {
    /// Whether this run changed what is installed (a new pack or a different version).
    pub fn changed(&self) -> bool {
        self.previous.as_deref() != Some(self.pack.version.as_str())
    }
}

/// Install or upgrade every valid add-on pack found on disk. Returns what was installed and
/// what could not be read, so the caller can report both. Used after upgrades: an add-on
/// package replaces the files, and this brings the definitions in the database up to date.
pub async fn sync_addons(db: &sqlx::PgPool, addons_dir: &Path) -> (Vec<Synced>, Vec<(String, String)>) {
    let (mut done, mut failed) = (vec![], vec![]);
    for d in discover(addons_dir) {
        match d.pack {
            Ok(pack) => {
                let previous: Option<String> = sqlx::query_scalar("SELECT version FROM packs WHERE id = $1").bind(&pack.id).fetch_optional(db).await.ok().flatten();
                match install(db, &pack, Some(&d.dir)).await {
                    Ok(()) => done.push(Synced { pack, previous }),
                    Err(e) => failed.push((d.id, e.public_message())),
                }
            }
            Err(e) => failed.push((d.id, e)),
        }
    }
    (done, failed)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn core() -> Pack {
        Pack::parse(CORE_PACK).expect("core pack must be valid")
    }

    /// The packs that ship with a release say so. Their content changes with it (names,
    /// templates, wording), and administrators read the version under About and in the log.
    #[test]
    fn the_packs_that_ship_with_a_release_carry_its_version() {
        assert_eq!(core().version, env!("CARGO_PKG_VERSION"), "packs/core.json");
        let nasa = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../addons/nasa/pack.json")).expect("addons/nasa/pack.json");
        assert_eq!(serde_json::from_str::<Pack>(&nasa).expect("the NASA pack parses").version, env!("CARGO_PKG_VERSION"), "addons/nasa/pack.json");
    }

    #[test]
    fn core_pack_is_valid() {
        let p = core();
        assert_eq!(p.id, "core");
        assert!(p.item_types.len() >= 20 && p.workflows.len() >= 10 && p.project_templates.len() >= 10);
    }

    #[test]
    fn core_pack_covers_the_required_domains() {
        let domains: HashSet<String> = core().project_templates.iter().map(|t| t.domain.clone()).collect();
        for d in ["Software engineering", "General engineering", "Science", "Management", "Finance", "Legal", "Logistics", "Procurement", "Safety", "Service desk", "General"] {
            assert!(domains.contains(d), "no project template for domain {d}");
        }
    }

    #[test]
    fn nasa_addon_pack_is_valid_against_core() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../addons/nasa/pack.json");
        let text = std::fs::read_to_string(&path).expect("addons/nasa/pack.json exists");
        let nasa: Pack = serde_json::from_str(&text).expect("NASA pack parses");
        let c = core();
        let fields = c.fields.iter().map(|f| f.key.clone()).collect();
        let workflows = c.workflows.iter().map(|w| w.key.clone()).collect();
        let types = c.item_types.iter().map(|t| t.key.clone()).collect();
        nasa.validate(&fields, &workflows, &types).expect("NASA pack must be valid");
        assert_eq!(nasa.id, "nasa");
        for t in &nasa.page_templates {
            if let Some(f) = &t.file {
                read_pack_file(path.parent().unwrap(), f).unwrap_or_else(|e| panic!("{e}"));
            }
        }
    }

    fn wf(statuses: &[(&str, &str)], initial: &str, transitions: &[(&[&str], &str)]) -> WorkflowDef {
        WorkflowDef {
            key: "w".into(),
            name: "W".into(),
            initial: initial.into(),
            statuses: statuses.iter().map(|(k, c)| StatusDef { key: k.to_string(), name: k.to_string(), category: c.to_string() }).collect(),
            transitions: transitions.iter().map(|(f, t)| TransitionDef { name: format!("to {t}"), from: f.iter().map(|s| s.to_string()).collect(), to: t.to_string(), requires: vec![] }).collect(),
            layout: None,
        }
    }

    #[test]
    fn workflow_validation_catches_structural_errors() {
        assert!(wf(&[("a", "todo"), ("b", "done")], "a", &[(&["a"], "b")]).validate().is_ok());
        assert!(wf(&[("a", "todo"), ("b", "done")], "zzz", &[(&["a"], "b")]).validate().unwrap_err().contains("initial"));
        assert!(wf(&[("a", "todo"), ("b", "in_progress")], "a", &[(&["a"], "b")]).validate().unwrap_err().contains("done"));
        assert!(wf(&[("a", "todo"), ("b", "done")], "a", &[(&["a"], "c")]).validate().unwrap_err().contains("unknown status"));
        assert!(wf(&[("a", "todo"), ("b", "done"), ("c", "done")], "a", &[(&["a"], "b")]).validate().unwrap_err().contains("cannot be reached"));
        assert!(wf(&[("a", "todo"), ("a", "done")], "a", &[]).validate().unwrap_err().contains("duplicate"));
        assert!(wf(&[("a", "weird"), ("b", "done")], "a", &[(&["a"], "b")]).validate().unwrap_err().contains("category"));
    }

    #[test]
    fn transitions_respect_source_status() {
        let w = wf(&[("open", "todo"), ("work", "in_progress"), ("done", "done")], "open", &[(&["open"], "work"), (&["work"], "done"), (&["*"], "open")]);
        assert!(w.transition("open", "work").is_some());
        assert!(w.transition("open", "done").is_none(), "cannot skip a step");
        assert!(w.transition("done", "open").is_some(), "wildcard source");
        assert!(w.transition("open", "open").is_none(), "no-op transitions are not offered");
        assert_eq!(w.available("work").len(), 2);
        assert_eq!(w.first_in_category("done").unwrap().key, "done");
    }

    #[test]
    fn pack_rejects_dangling_references() {
        let mut p = core();
        p.item_types[0].workflow = "nope".into();
        assert!(p.validate(&HashSet::new(), &HashSet::new(), &HashSet::new()).unwrap_err().contains("unknown workflow"));
        let mut p = core();
        p.project_templates[0].default_type = "bug".into();
        assert!(p.validate(&HashSet::new(), &HashSet::new(), &HashSet::new()).unwrap_err().contains("default type"));
        assert!(Pack::parse("{\"id\":\"x\",\"name\":\"X\",\"version\":\"1\",\"surprise\":1}").is_err(), "unknown keys are rejected");
    }

    #[test]
    fn pack_files_cannot_escape_the_pack_directory() {
        let dir = std::env::temp_dir();
        assert!(read_pack_file(&dir, "../etc/passwd").is_err());
        assert!(read_pack_file(&dir, "/etc/passwd").is_err());
        assert!(read_pack_file(&dir, "templates/../../x.md").is_err());
    }

    #[test]
    fn key_format() {
        assert!(valid_key("change_request") && valid_key("a1"));
        assert!(!valid_key("Change") && !valid_key("1a") && !valid_key("a-b") && !valid_key(""));
    }
}
