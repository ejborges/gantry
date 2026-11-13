---
title: Users, groups and roles
summary: Creating accounts for people and AI agents, and controlling what each may see and do.
---

## How access works

Nobody can do anything until a **role** says they can. A role is a named set of permissions, and it is given to a person, an AI agent or a group at one of three levels:

- for the **whole workspace**, under Administration;
- for **one project**, in the project's settings;
- for **one documentation space**, in the space's settings.

Someone's access is the sum of all their roles, including those of their groups. There is no way to subtract: to restrict a person, give them fewer or narrower roles. AI agents are the one exception: an agent's [rules](agents.md) narrow what its roles grant. A project set to *Everyone in the workspace* can be read by all members without listing them, and they can create items and comment in it; editing and moving items still needs a project role. A space with that setting can be read by all members. Anything set to *Only members listed* is invisible to everyone else, including in search. Two workspace roles see through that setting by design: System administrator and Auditor. Tell people who create private projects that these role holders can read them, and keep both roles to a few named people.

The full list is in [Permissions and roles](../reference/permissions.md).

## People

**New user** creates an account with a name, an email address, an initial password and a workspace role.

| Workspace role | Give it to |
| --- | --- |
| Member | Staff who work in projects and documentation. |
| Customer | People who only raise and follow requests in the help desk. |
| Application administrator, System administrator, Auditor | See [the console overview](index.md#who-is-an-administrator). |

Send the initial password by a private channel and ask the person to change it under Profile. With single sign-on you do not create people here at all: see [Sign-in](sign-in.md).

The **Manage** menu on each row lets you:

- add or remove a workspace role;
- **Unlock** an account that was locked after wrong passwords;
- **Sign out everywhere**, ending all of the person's sessions, for example when a laptop is lost;
- **Deactivate** the account. This ends its sessions and tokens at once and blocks sign-in, while keeping its name on everything it did. **Reactivate** reverses it. Accounts are never deleted, so history stays truthful.

### Profiles, pictures and handles

**Manage**, **Edit profile** opens an account's profile for you to change: **display name**, **handle** (what people type after `@`; lower-case letters, digits, dot, dash and underscore, 2 to 39 characters, unique), **job title**, **organization** and **organization code**. **Remove picture** takes away a picture someone uploaded and gives the account its generated pattern again.

Which of these fields people may change **themselves** is a rule for the whole site: see [Profiles](site.md#profiles). Whatever the rule, an administrator can always set them here, and your identity provider can set them through SCIM.

Every change you make to someone's profile is recorded in the audit log with what changed.

## AI agents

**New agent** creates an account of the kind *AI agent*. It cannot sign in with a password or single sign-on, acts only through API tokens, is marked as an AI agent everywhere, and is **allowed nothing** until you write its rules.

Creating agents, writing their rules and issuing their tokens needs the *admin.agents* permission, which application administrators have. The whole procedure is in [AI agent accounts](agents.md).

## Groups

A group is a list of accounts. Create one under **Groups**, add members, then give the group a role in a project or space instead of adding people one by one. When someone joins or leaves the team, change the group and every project follows.

If your identity provider supports SCIM, it can maintain groups for you.

## Custom roles

The built-in roles cannot be changed. When none fits, create one under **Roles**, **New role**:

1. Choose what it **applies to**: the whole workspace, one project at a time, or one documentation space at a time.
2. Select the permissions it grants.
3. Save. The role can now be chosen wherever roles of that level are assigned.

Example: a *Reviewer* project role with read, comment and transition permissions, for people who approve work but do not edit it.

## Good practice

- Give roles to groups, not individuals.
- Prefer project and space roles to workspace-wide ones.
- Keep administrator roles few and named.
- Deactivate leavers the same day; with SCIM this is automatic.
- Review the [audit log](audit.md) for role changes monthly: filter by actions starting with `role.`.
