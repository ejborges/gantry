---
title: Troubleshooting for application administrators
nav: Troubleshooting
summary: Common administrative problems and where to look.
---

## Access

| Problem | Check |
| --- | --- |
| Someone cannot see a project | Is the project private? Then they, or a group they are in, need a role under the project's **Members**. Is their account a *Customer*? Customers see only the help desk. |
| Someone can see and comment but not edit or move items | They have no role in the project (it is visible to the whole workspace), or their role is Viewer. Give Member. |
| An AI agent's calls return 403 | Three things must all allow the action: a **role**, the agent's **rules**, and the **token**. Open **Rules: what it may do**; a permission marked *none of its roles grants it* needs a role, and one that is not selected needs selecting. A *Read only* token cannot change anything. |
| An AI agent gets empty lists, or 404 for things that exist | Its rules allow nothing there. A new agent is allowed nothing, and projects and spaces not selected under **Where** do not exist for it. |
| An AI agent is not offered an MCP tool | The tool list is made for each caller from its roles, rules and token. Allow the permission the tool needs. |
| An AI agent's calls return 401 | The token is revoked or expired, or the agent is deactivated. |
| A new person signs in through single sign-on and sees nothing | New accounts get the Member role and see workspace-wide projects only. Add them to the private projects they need, ideally through a group. |
| You removed your own administrator role | Ask another system administrator, or have the system administrator create one from the command line. |

## Work types and workflows

| Problem | Check |
| --- | --- |
| A type does not appear in **New item** | It must be selected under the project's **Settings**, **Work item types**. |
| A step button does not appear | The workflow has no step from the item's current status to the one you expect. |
| Removing a status asks where its items go | Items may be in it. Choose the status they move to; the move is recorded in each item's history. |
| A change to a shared workflow did not reach a project | That project has its own workflow for the type (its **Settings**, **Workflows** shows *Own workflow*). The project's administrator can return it to the shared one. |
| Someone cannot be chosen as assignee | Only accounts with access to the project can be assigned its work. Add them to the project. |
| An add-on update did not change a workflow | The workflow is marked *Customized*. Your version is kept on purpose. |

## Service desk

| Problem | Check |
| --- | --- |
| A customer does not see a desk | Either select *Any signed-in user may raise requests here*, or give them the Service desk customer role in that project. |
| No request types are offered | At least one request type must be marked *Offered*. |
| Targets are missed overnight | The clock is set to around the clock. Switch to working hours. |
| Customers see no suggested articles | Pages must be in the desk's knowledge base space, published, and marked *Shown in the help desk*. |

## Notifications and email

| Problem | Check |
| --- | --- |
| Nobody receives email | **Notifications and email**: is email delivery **Set up**? Is **Send notifications by email** selected? Is the server's public address set (the panel warns when it is not)? Then read the delivery table for *failed* rows and their detail. |
| One person receives no email | Their profile's **By email** boxes; whether the row is locked off; whether their account has a real address. AI agents never receive email. |
| The email column is grayed out in everyone's profile | Email is not set up, or **Send notifications by email** is off. |
| Messages are *sent* but do not arrive | The mail server accepted them. Look at its logs and at spam filtering; ask the system administrator about SPF and DKIM for the sender address. See [Email](../sysadmin/email.md#deliverability). |
| Reminders arrive at the wrong hour | The hour is counted in each person's own time zone, or the organization's for people on Automatic. Check both. |
| An alert never fires | **Measure now and send a test** shows the current value. Compare it with the threshold; check that the alert is on. |
| An alert fires but nobody is told | **Who is told**: at least one role, address or integration; the recipients' own choices for *Application alerts*; **Do not repeat within** may be holding it back. |
| An alert does not reach Slack or Teams | Open the integration's **activity** under Integrations. |

## People, teams and agents

| Problem | Check |
| --- | --- |
| Someone cannot change their job title or organization | **Settings**, **Profiles**: the field is not selected. Select it, or set the value for them under **Users and agents**, **Edit profile**. |
| A picture is refused | It is over the size or dimension limit under **Settings**, **Profiles**, smaller than 32 pixels, or not a PNG, JPEG, WebP or GIF. |
| A mention does not notify | The handle is wrong (see the person's profile), or the person cannot open the item or page. |
| Someone cannot edit a team | Only its leads, the leads of a team above it, and holders of *team.manage* can. |
| Someone cannot share a dashboard | Their role lacks *analytics.share*. |
| Someone cannot define levels or create a program | They lack *planning.manage*. The owner of a program can add initiatives beneath it. |
| A program's owner cannot see a project's report | The project is not placed beneath that program. The project's administrator places it. |

## Live updates

| Problem | Check |
| --- | --- |
| **Live updates paused** for everyone | The web server in front of Gantry is not passing WebSocket connections. Tell the system administrator; see [Troubleshooting](../sysadmin/troubleshooting.md#live-updates-do-not-work). The Overview shows the *Live updates* component and how many connections are open. |
| For one person | Their network or a browser extension blocks WebSockets. Everything else still works; they see changes when they reload. |

## Integrations

Open the integration and read its **activity** first: it shows each attempt and the response. Then see [the table of common failures](integrations.md#when-a-delivery-fails).

## Search

| Problem | Check |
| --- | --- |
| New content is not found for a few seconds | Indexing happens in the background; look at **Background jobs** for a backlog. |
| *Similar meaning* returns nothing | Semantic search is off, or embeddings are still being built. The Overview shows the *Embeddings* component. |
| Results seem stale after a restore or a change of search engine | Ask the system administrator to rebuild the index (`gantry reindex`). |

## When you need the system administrator

Give them the time of the problem, the affected user, and the **reference** from the error message or the request ID from the audit entry. The system administrator guide's [troubleshooting page](../sysadmin/troubleshooting.md) starts from there.
