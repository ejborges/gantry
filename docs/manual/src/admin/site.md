---
title: Site settings, support contact and statements
nav: Site, support and statements
summary: Name the installation, tell people where to get help, publish your privacy, terms, cookie, retention and security statements, and decide which profile fields people may change.
---

Open **Administration**, **Settings**. Changing these needs the *admin.settings* permission, except **Profiles**, which needs *admin.users*.

## This site

| Setting | Effect |
| --- | --- |
| **Site name** | Replaces "Gantry" in the navigation, on the sign-in page, in the browser tab and in email subjects. |
| **Time zone of the organization** | An IANA name such as `America/New_York`. Decides the hour and the day of reminders for people who have not chosen a time zone of their own. |
| **Announcement** | Shown to everyone in a bar at the top until you clear it. Use it for planned maintenance. |
| **Sign-in notice** | Text on the sign-in page, for example an acceptable-use statement. |

## Where people get help

This is the contact shown on **every error page**, on the **sign-in page**, under **About**, and on the page that lists error messages. Fill in what applies and leave the rest empty.

| Field | Example |
| --- | --- |
| **Who to ask** | IT service desk |
| **Email** | `help@example.org` |
| **Web address** | `https://help.example.org` (must begin with `http://` or `https://`) |
| **Anything else** | Hours, a phone extension, a chat channel |

When nothing is filled in, error pages say *Contact the people who run this installation in your organization*. Fill this in before you invite users: an error page is exactly where someone needs to know whom to ask.

The contact is shown to people who are not signed in, so do not put anything confidential in it.

## Statements people can read

Gantry gives you a place to publish five statements, and shows them to everyone, **signed in or not**, under **About** and from the links beneath the sign-in form.

| Statement | Write here |
| --- | --- |
| **Privacy** | What personal data you process in this installation, why, on what basis, who can see it, and how people exercise their rights. |
| **Terms** | The conditions for using the installation: acceptable use, who may have an account. |
| **Cookies** | Your statement on cookies. The software's own list is shown beneath it automatically. |
| **Data retention** | How long you keep data and backups, and how deletion is requested. |
| **Data security** | Your organization's measures: encryption at rest, backup locations, incident contact. |

- Write each one in Markdown. **How it will read** shows the result beside the text.
- A statement left **empty is not listed** as your organization's text; the page then says that your organization has not written one, and shows only the facts about the software.
- **See them as readers do** opens the public pages.
- Press **Save statements**.

> **Important**: these are your organization's words. Gantry does not supply legal text, and nothing here is legal advice. What Gantry adds beneath each statement are verifiable facts about the software (which cookie it sets, what it logs, how long it keeps notification and delivery records, how passwords are stored) so that your statements can refer to them. The retention periods of the software's own records are listed in [Operations](../sysadmin/operations.md#what-is-removed-automatically).

**Licenses** and the list of **components and add-ons** under About need no setup: they are produced from the installed software.

## Profiles

Decide which of their own details people may change. What is not selected is set by administrators under **Users and agents**, or by your identity provider.

| People may change their own | Ships as |
| --- | --- |
| **Display name** | Allowed |
| **Organization** | Allowed |
| **Organization code** | Not allowed |
| **Job title** | Allowed |

Lock the fields that your identity provider or HR system is the source of truth for, so that they cannot drift. People can always change their picture, time zone, status and notification choices.

**Profile pictures**

| Setting | Ships as | Range |
| --- | --- | --- |
| **Largest profile picture, in KB** | 512 | 16 to 5,120 |
| **Largest side, in pixels** | 1,024 | 64 to 4,096 |

People are advised to upload a square of 256 × 256 pixels. A picture over a limit is refused with a message that states the limit; nothing is resized on the server. Accepted formats are PNG, JPEG, WebP and GIF; the server checks the file's real type and dimensions, not its name. Accounts without a picture get a generated pattern.

Press **Save profile rules**.

## Server configuration

The last panel shows the settings the server is running with, read-only. They are changed in `/etc/gantry/gantry.toml` by a system administrator, followed by a restart. Passwords, keys and tokens are never shown.
