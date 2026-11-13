//! Authorization. Deny by default; every decision goes through this module.
//!
//! A *role* is a named set of permissions with a scope (global, project or space).
//! A *binding* grants a role to a user or a group, either globally or on one project/space.
//! Effective permissions on an object = permissions from global bindings ∪ bindings on that object
//! ∪ the small implicit set that "internal" visibility grants to workspace members.
//! Built-in roles are reinstalled on every migration; administrators can add custom roles.

use crate::audit;
use crate::auth::Ctx;
use crate::error::{AppError, AppResult};
use crate::{AppState, St};
use serde_json::json;
use std::collections::HashSet;
use uuid::Uuid;

/// (key, scope, description). The scope is where the permission is meaningful.
pub const PERMISSIONS: &[(&str, &str, &str)] = &[
    ("workspace.access", "global", "Use the workspace: internal projects and spaces, the people directory"),
    ("project.create", "global", "Create projects"),
    ("space.create", "global", "Create documentation spaces"),
    ("admin.access", "global", "Open the administration console and see system status"),
    ("admin.users", "global", "Manage users, groups and role assignments"),
    ("admin.roles", "global", "Define custom roles"),
    ("admin.security", "global", "Manage sign-in providers, other people's tokens, and read security events"),
    ("admin.audit", "global", "Read the audit log"),
    ("admin.schemes", "global", "Manage work item types, fields, workflows, templates and add-on packs"),
    ("admin.integrations", "global", "Manage integrations and webhooks"),
    ("admin.settings", "global", "Change system settings; define system alerts"),
    ("admin.agents", "global", "Create AI agent accounts, set what they may do and issue their tokens"),
    ("admin.notifications", "global", "Set workspace notification defaults and application alerts"),
    ("scim.provision", "global", "Provision users and groups through SCIM"),
    ("team.create", "global", "Create teams"),
    ("team.manage", "global", "Edit any team, its positions and its members"),
    ("planning.manage", "global", "Define hierarchy levels; create programs and initiatives and assign their owners"),
    ("analytics.share", "global", "Share dashboards and visualizations with the workspace"),
    ("project.view", "project", "See the project and its work items"),
    ("project.admin", "project", "Change project settings, members, workflows, request types and SLAs"),
    ("project.reports", "project", "See statistics and workload reports for the project"),
    ("item.create", "project", "Create work items"),
    ("item.edit", "project", "Edit work item fields"),
    ("item.transition", "project", "Move work items through the workflow"),
    ("item.assign", "project", "Assign work items"),
    ("item.comment", "project", "Take part in the discussion on work items"),
    ("comment.moderate", "project", "Remove other people's messages from a discussion"),
    ("item.link", "project", "Link work items to other items, pages and commits"),
    ("item.delete", "project", "Archive work items"),
    ("desk.agent", "project", "Work service desk queues; read and write internal notes"),
    ("desk.request", "project", "Raise requests through the portal and follow one's own requests"),
    ("space.view", "space", "Read pages in the space"),
    ("space.admin", "space", "Change space settings, members and the page workflow"),
    ("page.edit", "space", "Create and edit pages"),
    ("page.delete", "space", "Archive pages"),
];

pub struct BuiltinRole {
    pub key: &'static str,
    pub name: &'static str,
    pub scope: &'static str,
    pub permissions: &'static [&'static str],
    pub description: &'static str,
}

pub const BUILTIN_ROLES: &[BuiltinRole] = &[
    BuiltinRole { key: "system_admin", name: "System administrator", scope: "global", permissions: &["*"], description: "Everything, including security and system settings." },
    BuiltinRole {
        key: "app_admin",
        name: "Application administrator",
        scope: "global",
        permissions: &["workspace.access", "project.create", "space.create", "admin.access", "admin.users", "admin.roles", "admin.schemes", "admin.integrations", "admin.audit", "admin.agents", "admin.notifications", "team.create", "team.manage", "planning.manage", "analytics.share"],
        description: "Runs the application day to day. Cannot change sign-in providers or system settings.",
    },
    BuiltinRole {
        key: "auditor",
        name: "Auditor",
        scope: "global",
        permissions: &["workspace.access", "admin.access", "admin.audit", "admin.security", "project.view", "project.reports", "space.view"],
        description: "Read-only access to all content, the audit log and security events.",
    },
    BuiltinRole { key: "member", name: "Member", scope: "global", permissions: &["workspace.access", "project.create", "space.create", "team.create", "analytics.share"], description: "Regular workspace user." },
    BuiltinRole { key: "customer", name: "Customer", scope: "global", permissions: &[], description: "Portal-only account. Sees nothing until granted access to a service desk." },
    BuiltinRole { key: "provisioner", name: "Provisioning client", scope: "global", permissions: &["scim.provision"], description: "For identity-provider SCIM tokens only." },
    BuiltinRole {
        key: "project_admin",
        name: "Project administrator",
        scope: "project",
        permissions: &["project.view", "project.admin", "project.reports", "item.create", "item.edit", "item.transition", "item.assign", "item.comment", "comment.moderate", "item.link", "item.delete", "desk.agent", "desk.request"],
        description: "Full control of one project.",
    },
    BuiltinRole {
        key: "project_member",
        name: "Project member",
        scope: "project",
        permissions: &["project.view", "item.create", "item.edit", "item.transition", "item.assign", "item.comment", "item.link", "desk.request"],
        description: "Does the work in a project.",
    },
    BuiltinRole { key: "project_viewer", name: "Project viewer", scope: "project", permissions: &["project.view"], description: "Read-only access to one project." },
    BuiltinRole {
        key: "service_agent",
        name: "Service agent",
        scope: "project",
        permissions: &["project.view", "item.create", "item.edit", "item.transition", "item.assign", "item.comment", "item.link", "desk.agent", "desk.request"],
        description: "Handles requests in a service desk.",
    },
    BuiltinRole { key: "desk_customer", name: "Service desk customer", scope: "project", permissions: &["desk.request"], description: "May raise and follow their own requests in one service desk." },
    BuiltinRole { key: "space_admin", name: "Space administrator", scope: "space", permissions: &["space.view", "space.admin", "page.edit", "page.delete"], description: "Full control of one space." },
    BuiltinRole { key: "space_editor", name: "Space editor", scope: "space", permissions: &["space.view", "page.edit"], description: "Reads and writes pages." },
    BuiltinRole { key: "space_viewer", name: "Space viewer", scope: "space", permissions: &["space.view"], description: "Reads pages." },
];

/// What "internal" visibility grants to anyone holding `workspace.access`.
pub const INTERNAL_PROJECT_PERMS: &[&str] = &["project.view", "item.create", "item.comment"];
pub const INTERNAL_SPACE_PERMS: &[&str] = &["space.view"];

pub fn is_known_permission(p: &str) -> bool {
    p == "*" || PERMISSIONS.iter().any(|(k, _, _)| *k == p)
}

#[derive(Clone, Debug, Default)]
pub struct Perms(pub HashSet<String>);

impl Perms {
    pub fn has(&self, p: &str) -> bool {
        self.0.contains("*") || self.0.contains(p)
    }
    pub fn grant(&mut self, ps: &[&str]) {
        self.0.extend(ps.iter().map(|s| s.to_string()));
    }
    /// Keep only the permissions that are also in `allowed`. `*` is expanded first, so the
    /// result never contains the wildcard.
    pub fn narrow(&self, allowed: &HashSet<String>) -> Perms {
        if self.0.contains("*") {
            Perms(PERMISSIONS.iter().map(|(k, _, _)| k.to_string()).filter(|k| allowed.contains(k)).collect())
        } else {
            Perms(self.0.iter().filter(|p| allowed.contains(*p)).cloned().collect())
        }
    }
    /// Sorted list for API responses. `*` expands to every known permission.
    pub fn list(&self) -> Vec<String> {
        let mut v: Vec<String> = if self.0.contains("*") { PERMISSIONS.iter().map(|(k, _, _)| k.to_string()).collect() } else { self.0.iter().cloned().collect() };
        v.sort();
        v
    }
}

/// What an AI agent account may do, set by an administrator. An agent's effective permissions
/// are the permissions of its roles *and* this allow-list *and*, for anything inside a project
/// or space, the list of projects and spaces it is confined to. A new agent has an empty
/// policy: it can authenticate and nothing else.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AgentPolicy {
    pub unrestricted: bool,
    pub permissions: HashSet<String>,
    pub all_projects: bool,
    pub projects: HashSet<Uuid>,
    pub all_spaces: bool,
    pub spaces: HashSet<Uuid>,
    /// Requests per minute; `None` uses the server-wide limit.
    pub rate_limit: Option<u32>,
}

impl AgentPolicy {
    /// Read a stored policy. Anything missing or malformed is read as "not allowed".
    pub fn from_json(v: Option<&serde_json::Value>) -> AgentPolicy {
        let Some(v) = v.filter(|v| v.is_object()) else { return AgentPolicy::default() };
        let ids = |k: &str| v[k].as_array().map(|a| a.iter().filter_map(|x| x.as_str()?.parse().ok()).collect()).unwrap_or_default();
        AgentPolicy {
            unrestricted: v["mode"] == "unrestricted",
            permissions: v["permissions"].as_array().map(|a| a.iter().filter_map(|x| x.as_str()).filter(|p| *p != "*" && is_known_permission(p)).map(str::to_string).collect()).unwrap_or_default(),
            all_projects: v["all_projects"] == true,
            projects: ids("projects"),
            all_spaces: v["all_spaces"] == true,
            spaces: ids("spaces"),
            rate_limit: v["rate_limit_per_minute"].as_u64().map(|n| n.clamp(1, 100_000) as u32),
        }
    }

    pub fn to_json(&self) -> serde_json::Value {
        let sorted = |s: &HashSet<Uuid>| {
            let mut v: Vec<String> = s.iter().map(Uuid::to_string).collect();
            v.sort();
            v
        };
        let mut perms: Vec<&String> = self.permissions.iter().collect();
        perms.sort();
        json!({"mode": if self.unrestricted { "unrestricted" } else { "restricted" }, "permissions": perms,
               "all_projects": self.all_projects, "projects": sorted(&self.projects),
               "all_spaces": self.all_spaces, "spaces": sorted(&self.spaces), "rate_limit_per_minute": self.rate_limit})
    }

    pub fn allows(&self, permission: &str) -> bool {
        self.unrestricted || self.permissions.contains(permission)
    }
    pub fn allows_project(&self, id: Uuid) -> bool {
        self.unrestricted || self.all_projects || self.projects.contains(&id)
    }
    pub fn allows_space(&self, id: Uuid) -> bool {
        self.unrestricted || self.all_spaces || self.spaces.contains(&id)
    }
    /// Apply the policy to a set of role permissions.
    pub fn narrow(&self, perms: Perms) -> Perms {
        if self.unrestricted {
            perms
        } else {
            perms.narrow(&self.permissions)
        }
    }
}

pub async fn seed_roles(db: &sqlx::PgPool) -> sqlx::Result<()> {
    for r in BUILTIN_ROLES {
        let perms: Vec<String> = r.permissions.iter().map(|s| s.to_string()).collect();
        sqlx::query(
            "INSERT INTO roles (key, name, scope, permissions, builtin, description) VALUES ($1,$2,$3,$4,true,$5)
             ON CONFLICT (key) DO UPDATE SET name = EXCLUDED.name, scope = EXCLUDED.scope, permissions = EXCLUDED.permissions, builtin = true, description = EXCLUDED.description",
        )
        .bind(r.key)
        .bind(r.name)
        .bind(r.scope)
        .bind(perms)
        .bind(r.description)
        .execute(db)
        .await?;
    }
    Ok(())
}

/// Permissions from global bindings, plus bindings on `scope` when given.
pub async fn load_perms(db: &sqlx::PgPool, user_id: Uuid, scope: Option<(&str, Uuid)>) -> sqlx::Result<Perms> {
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT p FROM role_bindings b JOIN roles r ON r.id = b.role_id, unnest(r.permissions) AS p
         WHERE (b.user_id = $1 OR b.group_id IN (SELECT group_id FROM group_members WHERE user_id = $1))
           AND (b.scope_type = 'global' OR (b.scope_type = $2 AND b.scope_id = $3))",
    )
    .bind(user_id)
    .bind(scope.map(|s| s.0))
    .bind(scope.map(|s| s.1))
    .fetch_all(db)
    .await?;
    Ok(Perms(rows.into_iter().collect()))
}

pub async fn project_perms(st: &AppState, ctx: &Ctx, project_id: Uuid, access: &str, portal_open: bool) -> AppResult<Perms> {
    if let Some(policy) = &ctx.policy {
        if !policy.allows_project(project_id) {
            return Ok(Perms::default());
        }
    }
    let mut perms = load_perms(&st.db, ctx.user_id, Some(("project", project_id))).await?;
    if access == "internal" && ctx.global.has("workspace.access") {
        perms.grant(INTERNAL_PROJECT_PERMS);
    }
    if portal_open {
        perms.grant(&["desk.request"]);
    }
    // The project lead, and the owner of a program or initiative the project belongs to, can
    // always read the project and its reports. Ownership is assigned by an administrator.
    if !(perms.has("project.view") && perms.has("project.reports")) {
        let (lead, owner): (bool, bool) = sqlx::query_as(
            "SELECT coalesce(p.lead_id = $2, false), EXISTS (
               WITH RECURSIVE up(id, parent_id, owner_id, depth) AS (
                 SELECT c.id, c.parent_id, c.owner_id, 0 FROM containers c WHERE c.id = p.container_id
                 UNION ALL SELECT c.id, c.parent_id, c.owner_id, up.depth + 1 FROM containers c JOIN up ON c.id = up.parent_id WHERE up.depth < 20)
               SELECT 1 FROM up WHERE owner_id = $2)
             FROM projects p WHERE p.id = $1",
        )
        .bind(project_id)
        .bind(ctx.user_id)
        .fetch_optional(&st.db)
        .await?
        .unwrap_or((false, false));
        if owner || (lead && perms.has("project.view")) {
            perms.grant(&["project.view", "project.reports"]);
        }
    }
    Ok(match &ctx.policy {
        Some(policy) => policy.narrow(perms),
        None => perms,
    })
}

pub async fn space_perms(st: &AppState, ctx: &Ctx, space_id: Uuid, access: &str) -> AppResult<Perms> {
    if let Some(policy) = &ctx.policy {
        if !policy.allows_space(space_id) {
            return Ok(Perms::default());
        }
    }
    let mut perms = load_perms(&st.db, ctx.user_id, Some(("space", space_id))).await?;
    if access == "internal" && ctx.global.has("workspace.access") {
        perms.grant(INTERNAL_SPACE_PERMS);
    }
    Ok(match &ctx.policy {
        Some(policy) => policy.narrow(perms),
        None => perms,
    })
}

/// IDs of every project the caller may view. Used to filter lists, search and graph results.
pub async fn visible_project_ids(st: &AppState, ctx: &Ctx) -> AppResult<Vec<Uuid>> {
    if ctx.policy.as_ref().is_some_and(|p| !p.allows("project.view")) {
        return Ok(vec![]);
    }
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "WITH RECURSIVE owned(id, depth) AS (
            SELECT id, 0 FROM containers WHERE owner_id = $1 AND NOT archived
            UNION ALL SELECT c.id, owned.depth + 1 FROM containers c JOIN owned ON c.parent_id = owned.id WHERE owned.depth < 20)
         SELECT p.id FROM projects p WHERE NOT p.archived AND ($2 OR (p.access = 'internal' AND $3)
           OR p.container_id IN (SELECT id FROM owned)
           OR EXISTS (
            SELECT 1 FROM role_bindings b JOIN roles r ON r.id = b.role_id
            WHERE b.scope_type = 'project' AND b.scope_id = p.id
              AND (b.user_id = $1 OR b.group_id IN (SELECT group_id FROM group_members WHERE user_id = $1))
              AND r.permissions && ARRAY['project.view', '*']))",
    )
    .bind(ctx.user_id)
    .bind(ctx.global.has("project.view"))
    .bind(ctx.global.has("workspace.access"))
    .fetch_all(&st.db)
    .await?;
    Ok(match &ctx.policy {
        Some(policy) => ids.into_iter().filter(|id| policy.allows_project(*id)).collect(),
        None => ids,
    })
}

pub async fn visible_space_ids(st: &AppState, ctx: &Ctx) -> AppResult<Vec<Uuid>> {
    if ctx.policy.as_ref().is_some_and(|p| !p.allows("space.view")) {
        return Ok(vec![]);
    }
    let ids: Vec<Uuid> = sqlx::query_scalar(
        "SELECT s.id FROM spaces s WHERE NOT s.archived AND ($2 OR (s.access = 'internal' AND $3) OR EXISTS (
            SELECT 1 FROM role_bindings b JOIN roles r ON r.id = b.role_id
            WHERE b.scope_type = 'space' AND b.scope_id = s.id
              AND (b.user_id = $1 OR b.group_id IN (SELECT group_id FROM group_members WHERE user_id = $1))
              AND r.permissions && ARRAY['space.view', '*']))",
    )
    .bind(ctx.user_id)
    .bind(ctx.global.has("space.view"))
    .bind(ctx.global.has("workspace.access"))
    .fetch_all(&st.db)
    .await?;
    Ok(match &ctx.policy {
        Some(policy) => ids.into_iter().filter(|id| policy.allows_space(*id)).collect(),
        None => ids,
    })
}

const HOLDS: &str = "EXISTS (SELECT 1 FROM role_bindings b JOIN roles r ON r.id = b.role_id
    WHERE (b.user_id = u.id OR b.group_id IN (SELECT group_id FROM group_members WHERE user_id = u.id))";

/// Of `users`, the active accounts that hold `permission` on the project: through a role on the
/// project or everywhere, a group, internal visibility, leading the project, or owning a
/// program the project belongs to. Used to decide who may be told about something.
pub async fn project_audience(st: &AppState, project_id: Uuid, permission: &str, users: &[Uuid]) -> sqlx::Result<Vec<Uuid>> {
    if users.is_empty() {
        return Ok(vec![]);
    }
    let internal_grant = INTERNAL_PROJECT_PERMS.contains(&permission);
    let read_grant = matches!(permission, "project.view" | "project.reports");
    let rows: Vec<(Uuid, String, Option<serde_json::Value>)> = sqlx::query_as(&format!(
        "WITH RECURSIVE up(id, parent_id, owner_id, depth) AS (
            SELECT c.id, c.parent_id, c.owner_id, 0 FROM containers c JOIN projects p ON p.container_id = c.id WHERE p.id = $2
            UNION ALL SELECT c.id, c.parent_id, c.owner_id, up.depth + 1 FROM containers c JOIN up ON c.id = up.parent_id WHERE up.depth < 20)
         SELECT u.id, u.kind, u.agent_policy FROM users u WHERE u.id = ANY($1) AND u.active AND (
            {HOLDS} AND (b.scope_type = 'global' OR (b.scope_type = 'project' AND b.scope_id = $2)) AND r.permissions && ARRAY[$3, '*'])
            OR ($4 AND (SELECT access = 'internal' FROM projects WHERE id = $2) AND {HOLDS} AND b.scope_type = 'global' AND r.permissions && ARRAY['workspace.access', '*']))
            OR ($5 AND (u.id IN (SELECT owner_id FROM up) OR u.id = (SELECT lead_id FROM projects WHERE id = $2)))
            OR ($3 = 'desk.request' AND (SELECT kind = 'service_desk' AND coalesce((settings->>'portal_open')::boolean, false) FROM projects WHERE id = $2)))"
    ))
    .bind(users)
    .bind(project_id)
    .bind(permission)
    .bind(internal_grant)
    .bind(read_grant)
    .fetch_all(&st.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|(_, kind, policy)| {
            kind != "agent" || {
                let p = AgentPolicy::from_json(policy.as_ref());
                p.allows(permission) && p.allows_project(project_id)
            }
        })
        .map(|r| r.0)
        .collect())
}

/// Of `users`, the active accounts that may read the space.
pub async fn space_audience(st: &AppState, space_id: Uuid, users: &[Uuid]) -> sqlx::Result<Vec<Uuid>> {
    if users.is_empty() {
        return Ok(vec![]);
    }
    let rows: Vec<(Uuid, String, Option<serde_json::Value>)> = sqlx::query_as(&format!(
        "SELECT u.id, u.kind, u.agent_policy FROM users u WHERE u.id = ANY($1) AND u.active AND (
            {HOLDS} AND (b.scope_type = 'global' OR (b.scope_type = 'space' AND b.scope_id = $2)) AND r.permissions && ARRAY['space.view', '*'])
            OR ((SELECT access = 'internal' FROM spaces WHERE id = $2) AND {HOLDS} AND b.scope_type = 'global' AND r.permissions && ARRAY['workspace.access', '*'])))"
    ))
    .bind(users)
    .bind(space_id)
    .fetch_all(&st.db)
    .await?;
    Ok(rows
        .into_iter()
        .filter(|(_, kind, policy)| {
            kind != "agent" || {
                let p = AgentPolicy::from_json(policy.as_ref());
                p.allows("space.view") && p.allows_space(space_id)
            }
        })
        .map(|r| r.0)
        .collect())
}

/// People who hold `permission` through a role on this project itself (not through a role
/// that applies everywhere). These are the people who work in it.
pub async fn project_members_with(st: &AppState, project_id: Uuid, permission: &str) -> sqlx::Result<Vec<Uuid>> {
    sqlx::query_scalar(
        "SELECT DISTINCT u.id FROM users u JOIN role_bindings b ON (b.user_id = u.id OR b.group_id IN (SELECT group_id FROM group_members WHERE user_id = u.id))
         JOIN roles r ON r.id = b.role_id
         WHERE b.scope_type = 'project' AND b.scope_id = $1 AND r.permissions && ARRAY[$2, '*'] AND u.active",
    )
    .bind(project_id)
    .bind(permission)
    .fetch_all(&st.db)
    .await
}

/// Build a 403 and record the denial as a security event.
pub fn deny(st: &St, ctx: &Ctx, permission: &str, target: &str) -> AppError {
    let (st2, ctx2, perm, tgt) = (st.clone(), ctx.clone(), permission.to_string(), target.to_string());
    tokio::spawn(async move {
        audit::security(&st2, "authz.denied", "warning", Some((ctx2.user_id, &ctx2.email)), &ctx2.req, json!({"permission": perm, "target": tgt})).await;
    });
    AppError::Forbidden(format!("you do not have the '{permission}' permission for this"))
}

/// Like [`deny`], but answers 404 so the existence of the object is not revealed.
pub fn hide(st: &St, ctx: &Ctx, permission: &str, target: &str) -> AppError {
    let _ = deny(st, ctx, permission, target);
    AppError::NotFound
}

pub fn require(st: &St, ctx: &Ctx, perms: &Perms, permission: &str, target: &str) -> AppResult<()> {
    if perms.has(permission) && ctx.token_allows(permission) {
        Ok(())
    } else {
        Err(deny(st, ctx, permission, target))
    }
}

pub fn require_global(st: &St, ctx: &Ctx, permission: &str) -> AppResult<()> {
    if ctx.has(permission) {
        Ok(())
    } else {
        Err(deny(st, ctx, permission, "global"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wildcard_grants_everything() {
        let p = Perms(["*".to_string()].into_iter().collect());
        assert!(p.has("admin.settings") && p.has("item.delete"));
        assert_eq!(p.list().len(), PERMISSIONS.len());
    }

    #[test]
    fn empty_grants_nothing() {
        let p = Perms::default();
        assert!(PERMISSIONS.iter().all(|(k, _, _)| !p.has(k)));
    }

    #[test]
    fn builtin_roles_reference_only_known_permissions_in_their_scope() {
        for r in BUILTIN_ROLES {
            for perm in r.permissions {
                assert!(is_known_permission(perm), "{} references unknown permission {perm}", r.key);
                if *perm != "*" && r.scope != "global" {
                    let scope = PERMISSIONS.iter().find(|(k, _, _)| k == perm).unwrap().1;
                    assert_eq!(scope, r.scope, "{} ({}) holds {perm} ({scope})", r.key, r.scope);
                }
            }
        }
    }

    #[test]
    fn narrowing_never_adds_and_never_keeps_the_wildcard() {
        let allowed: HashSet<String> = ["project.view", "item.comment", "not.a.permission"].iter().map(|s| s.to_string()).collect();
        let admin = Perms(["*".to_string()].into_iter().collect());
        let n = admin.narrow(&allowed);
        assert!(n.has("project.view") && n.has("item.comment"));
        assert!(!n.has("admin.users") && !n.0.contains("*") && !n.has("not.a.permission"));
        let viewer = Perms(["project.view".to_string(), "item.edit".to_string()].into_iter().collect());
        let n = viewer.narrow(&allowed);
        assert!(n.has("project.view") && !n.has("item.edit") && !n.has("item.comment"), "narrowing is an intersection");
    }

    #[test]
    fn a_missing_or_malformed_agent_policy_allows_nothing() {
        for v in [None, Some(json!(null)), Some(json!("unrestricted")), Some(json!([])), Some(json!({})), Some(json!({"mode": "Unrestricted"})), Some(json!({"permissions": "*"}))] {
            let p = AgentPolicy::from_json(v.as_ref());
            assert!(!p.unrestricted, "{v:?}");
            assert!(PERMISSIONS.iter().all(|(k, _, _)| !p.allows(k)), "{v:?}");
            assert!(!p.allows_project(Uuid::nil()) && !p.allows_space(Uuid::nil()));
        }
    }

    #[test]
    fn agent_policy_reads_lists_and_ignores_unknown_permissions_and_the_wildcard() {
        let id = Uuid::new_v4();
        let p = AgentPolicy::from_json(Some(&json!({"mode": "restricted", "permissions": ["project.view", "*", "made.up"], "projects": [id.to_string(), "nonsense"], "all_spaces": true, "rate_limit_per_minute": 30})));
        assert!(p.allows("project.view") && !p.allows("item.edit") && !p.allows("*") && !p.allows("made.up"));
        assert!(p.allows_project(id) && !p.allows_project(Uuid::new_v4()));
        assert!(p.allows_space(Uuid::new_v4()));
        assert_eq!(p.rate_limit, Some(30));
        assert_eq!(AgentPolicy::from_json(Some(&p.to_json())), p, "a policy survives a round trip");
        let all = Perms(["*".to_string()].into_iter().collect());
        assert_eq!(p.narrow(all).list(), vec!["project.view"]);
    }

    #[test]
    fn role_keys_are_unique() {
        let keys: HashSet<_> = BUILTIN_ROLES.iter().map(|r| r.key).collect();
        assert_eq!(keys.len(), BUILTIN_ROLES.len());
    }

    #[test]
    fn customer_role_has_no_workspace_access() {
        let c = BUILTIN_ROLES.iter().find(|r| r.key == "customer").unwrap();
        assert!(c.permissions.is_empty());
        let viewer = BUILTIN_ROLES.iter().find(|r| r.key == "project_viewer").unwrap();
        assert_eq!(viewer.permissions, &["project.view"]);
    }
}
