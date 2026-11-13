---
title: Upgrading
summary: Installing a new version with dnf, what happens during the upgrade, and how to go back.
---

## Before

1. Read the [notes for the release](#notes-for-each-release) for anything that needs attention.
2. Take a backup: `gantry-backup`.
3. If the installation matters and you have a spare machine, rehearse: restore that backup there ([Backup and restore](backup.md#restore)), upgrade it, and look around. That shows what the upgrade does to your own data before it does it.

## Upgrade

Copy the new release directory to the server and run, as root:

```bash
./install.sh
```

or equivalently `dnf upgrade -y ./core/*.rpm ./addons/gantry-addon-…rpm` for the packages you have installed.

What happens:

1. `dnf` replaces the program, the web application, the manual and add-on content. Your `/etc/gantry/gantry.toml` is kept. If a new version changes the packaged default file, the new one is written next to yours as `gantry.toml.rpmnew` for you to compare.
2. If `gantry-setup` wrote your nginx site, the package brings it up to the new version's template (`/usr/libexec/gantry/nginx-site refresh`): it keeps your host name, certificate and upload limit, tests the result with `nginx -t`, reloads nginx, and puts the old file back if nginx refuses the new one. A site you maintain by hand (first line removed) is never changed; the upgrade then prints what it would have added.
3. The service restarts. Before it serves anything, `gantry migrate` upgrades the database schema, the built-in definitions and installed add-on content. Definitions you customized are left as they are.
4. Requests in flight during the restart are completed; new ones wait a few seconds at nginx.

Check afterward:

```bash
gantry doctor
journalctl -u gantry --since "10 minutes ago" -o cat | jq -c 'select(.level != "INFO")'
```

People with the application open get the new version at their next page load. While the service restarts they see [a page that says the application is unavailable](operations.md#when-the-application-is-unavailable) and retries by itself.

## Several application nodes

All nodes must run the same version. Stop Gantry on every node, upgrade the packages everywhere, start one node and wait for its `/readyz` to answer (it migrates the schema, holding a database lock so that only one node ever does), then start the others. The interruption is as long as the package upgrade takes, typically under a minute.

## Going back

Database migrations move forward only. To return to the previous version:

1. `systemctl stop gantry gantry-worker`
2. `dnf downgrade ./core/*.rpm` from the previous release directory;
3. restore the backup taken before the upgrade: see [Backup and restore](backup.md#restore).

Work done since that backup is lost, so decide quickly whether an upgrade is good.

## Operating system upgrades

Minor releases (`dnf upgrade`) need nothing special. A move to a new PostgreSQL major version, including the one that comes with a new Enterprise Linux major release, follows the operating system's PostgreSQL upgrade procedure (`postgresql-setup --upgrade`) or a dump and restore; Gantry itself needs no change.

## Notes for each release

### 0.2.0

**Do before upgrading**

- Nothing, beyond the backup.

**Happens by itself**

- The database schema gains tables for notifications, email, alerts, profiles, teams, planning, analytics, saved searches, real-time editing and SAML. Existing data is kept. Every account gets a handle (from its email address) and a generated profile picture.
- The nginx site written by `gantry-setup` gains a **WebSocket location** (`/api/v1/ws`) and the page shown while the application is unavailable. If you maintain the site by hand, add both: see [what the web server must pass through](install.md#what-the-web-server-must-pass-through). Without the WebSocket location everything works, but nothing updates live and the top bar says *Live updates paused*.
- A set of system and application **alerts** is created and turned on. They tell holders of the System administrator and Application administrator roles. Review them under **Administration**, **Alerts**.

**Do after upgrading**

| Do | Why |
| --- | --- |
| **Write rules for every AI agent account.** | From this version an agent may do only what its roles grant *and* its rules allow. So that nothing stops working during the upgrade, agents that **already exist** are set to *Everything its roles grant* and are flagged **No rules** in the list; agents created from now on start with rules that allow **nothing**. Go through the flagged agents and give each the narrowest rules that fit its job. **Administration**, **Users and agents**, **Manage**, **Rules: what it may do**. See [AI agent accounts](../admin/agents.md). |
| Configure a mail server, if you want email. | Notifications are in the application only until `[email]` is set. See [Email](email.md). |
| Enter the support contact and your statements. | They are shown on error pages, the sign-in page and under About. **Administration**, **Settings**. See [Site, support and statements](../admin/site.md). |
| Decide who may share scripts. | Members can share dashboards and visualizations with the workspace. Read [Custom content and scripts](security.md#custom-content-and-scripts) and adjust *analytics.share* if needed. |
| Tell people. | Notification choices, profile pictures, status and time zone are in each person's profile. |

**Behavior that changed**

| Before | Now |
| --- | --- |
| An AI agent could do what its roles allowed. | Its rules must allow it too. A new agent is allowed nothing; agents from before the upgrade are unrestricted until you write their rules. See above. |
| Any account could be made assignee. | The assignee must have access to the project. Existing assignments are kept. |
| A due date could be before the start date. | It is refused. Existing items are kept as they are until their dates are next changed. |
| Titles and names could contain line breaks and direction-override characters. | They are single lines; such characters are refused. |
| Markdown removed all HTML. | A safe subset of HTML is kept; scripts, styles and frames run only inside the sealed frame of an `embed` or `viz` block. Pictures on other sites are shown as links. |
| All attachments were downloaded. | Pictures, sound, video and PDF files are displayed in the browser, under a policy that forbids script. Other files are downloaded as before. |
| Saving a page replaced it at once. | **Review and save** shows the changes first. Several people can edit the same text at once. |
| An item listed who watched it. | It shows how many people follow it. Who follows is private. |
| A browser that opened a failing address got a JSON error. | It gets a readable page with the same status code. API clients get JSON as before. |
| Single sign-on discovery could contact any address. | OpenID Connect and SAML addresses pass the same check as integrations: private and loopback addresses are refused unless allowed in `[integrations]`. If your identity provider is on a private address, allow it there. |
| The canceled status had the key `cancelled` in built-in workflows. | The key is `canceled`. Items are migrated; scripts that send the old key must be updated. |
| Roles, groups and types for pickers were at `/api/v1/catalogue`. | The address is `/api/v1/catalog`. The old one answers *not found*. |
| MCP listed every tool to every caller. | Each caller is offered the tools its roles, rules and token allow. |

**New settings**

`[email]` and `[realtime]`: see [Configuration](configuration.md). Both have working defaults; `gantry.toml.rpmnew` shows them with comments.

**Known limits of new features**

- SAML: no encrypted assertions, no signed requests, no single log-out, RSA certificates only. See [what is not supported](../admin/sign-in.md#what-is-not-supported).
- Time zone *Automatic* follows each device; reminders for such accounts use the organization's time zone.
- The diagram editor is Gantry's own and does not read draw.io files.
