---
title: Permissions and roles
summary: Every permission, and what each built-in role grants.
---

Access is decided by permissions. A role is a named set of them, granted to a person, an AI agent or a group for the whole workspace, for one project or for one space. Anything not granted is refused.

## Permissions

{{permissions}}

Workspace-level permissions are granted by workspace roles. Project and space permissions are granted by a role in that project or space, or by a workspace role that includes them for everything.

## Built-in roles

{{roles}}

Built-in roles cannot be edited or deleted. Create custom roles for other combinations under **Administration**, **Roles**.

## Rules that roles cannot change

- A project with access *Everyone in the workspace* lets every account that has `workspace.access` view it, create items in it and comment (`project.view`, `item.create`, `item.comment`). Editing, moving and assigning items there still need a role in the project. A space with that access lets them read it (`space.view`); writing needs a role in the space.
- A project or space set to *Only members listed* gives nothing to anyone without a role in it.
- Customers (`customer`, `desk_customer`) reach work only through the help desk endpoints, and only their own requests. They never see internal notes.
- API token scopes can only narrow what the account's roles allow.
- An **AI agent** may use a permission only if a role grants it *and* the agent's rules allow it, and only in the projects and spaces its rules name. A new agent's rules allow nothing. See [AI agent accounts](../admin/agents.md).
- The **owner** of a program or initiative can view every project beneath it and read its reports (`project.view`, `project.reports`), without being a member. The **lead** of a project can read that project's reports.
- A **team's leads**, and the leads of teams above it, can edit that team without holding `team.manage`.
- The **owner** of a dashboard, visualization, saved search or Gantt chart, and the editors the owner names, can change it. Sharing one with the whole workspace needs `analytics.share` for dashboards and visualizations.
- Only accounts with access to a project can be made assignee of its work.
- Links, mentions and embedded cards in text are resolved for each reader: no permission lets text reveal something its reader may not see.
- The last active system administrator cannot be deactivated or lose the role.
- Deactivated accounts can do nothing, whatever roles they hold.
