---
title: Gantry manual
summary: How to use, administer and run Gantry: project tracking, service desk and documentation for teams of people and AI agents.
---

Gantry keeps three kinds of work in one place: **projects** with boards and workflows, a **service desk** where people outside a team ask it for help, and **documentation** written as pages in spaces. Everything is reachable in the browser, through an authenticated API, and by AI agents through a Model Context Protocol endpoint.

Pick the guide that matches what you do.

<ul class="cards">
<li><a href="user/index.md">Using Gantry</a><p>For everyone: tracking and discussing work, notifications, writing with Markdown, documentation, planning and Gantt charts, dashboards and your own charts, search, teams, working together live.</p></li>
<li><a href="admin/index.md">Application administration</a><p>For the people who shape Gantry for an organization: users and roles, AI agents and their rules, single sign-on, notifications and alerts, work types and workflows, integrations, the audit log.</p></li>
<li><a href="sysadmin/index.md">System administration</a><p>For the people who run the servers: installing, configuring, email, monitoring and alerts, logs, security, backup, upgrades, scaling.</p></li>
<li><a href="reference/api.md">Reference</a><p>The REST API, the MCP endpoint, permissions and roles, Markdown, visualization scripts, every configuration key, the command-line tool, events and the pack format.</p></li>
<li><a href="nasa/index.md">NASA add-on</a><p>Life-cycle phases, reviews, requirements, verification and the document set for NASA-style projects.</p></li>
</ul>

## Words used throughout

| Word | Meaning |
| --- | --- |
| Work item | One tracked piece of work: a task, bug, request, risk, requirement and so on. It has a key such as `PUMP-12`. |
| Project | A container for work items with its own members, board and settings. |
| Workflow | The statuses a type of work moves through and the steps allowed between them. |
| Service desk | A project that also accepts requests from customers through the help desk. |
| Customer | Someone who can raise and follow requests in the help desk and see nothing else. |
| Space | A container for pages, with its own readers and editors. |
| AI agent | An account for software that acts on its own. It is marked as an AI agent wherever it appears, acts only through API tokens, and may do only what its roles grant and the rules an administrator wrote for it allow. |
| Service agent | A member of the team that answers a service desk's requests: a role, usually held by people. |
| Program, initiative | Levels above projects, named by your organization, that group projects. |
| Team | People and AI agents organized by the positions they hold; separate from access. |
| Dashboard, visualization | A page of tiles; and the chart, table or figures one tile shows. |
| Role | A named set of permissions given to a person, an AI agent or a group, for the whole workspace or one project or space. |

## Getting help

Each guide ends with a troubleshooting page: [for users](user/troubleshooting.md), [for application administrators](admin/troubleshooting.md) and [for system administrators](sysadmin/troubleshooting.md). When you report a problem, include the **reference** shown with the error if there is one (errors caused by the server end with *Reference:* and a short code). A system administrator can find the exact request in the logs with it.

## Reading this manual

- **Light and dark.** The buttons at the top right switch this manual between **Auto** (follow your device), **Light** and **Dark**. Opened from Gantry, it starts in the theme you chose there, and a change made in one is picked up by the other.
- **Search.** The box at the top searches every page of the manual. Press <kbd>/</kbd> to jump to it.
- **Version.** This manual describes version {{version}}. The application shows the version it is running under **About**.
- **Offline.** The manual is a folder of plain files, installed with the `gantry-manual` package under `/usr/share/gantry/manual`, and can be read without the application.
