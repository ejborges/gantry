---
title: Notifications, email and alerts
nav: Notifications and alerts
summary: Set what people are told about by default, when reminders go out, and define alerts on conditions in the application, delivered in the application, by email, and to Slack or Microsoft Teams.
---

Three groups of people configure notifications, each at their own level.

| Who | Decides | Where |
| --- | --- | --- |
| **System administrator** | Whether email can be sent at all: the mail server. System alerts on performance, errors and security. | The configuration file; **Alerts**, system group. See [Email](../sysadmin/email.md) and [Alerts and monitoring](../sysadmin/alerts.md). |
| **Application administrator** | The default for each kind of notification, which choices are the same for everyone, reminder timing, and application alerts. | **Notifications and email**; **Alerts**, application group. This chapter. |
| **Each person** | What they receive, within what you allow. | Their profile. See [Notifications](../user/notifications.md) in the user guide. |

Notification settings and application alerts need the *admin.notifications* permission, which application administrators have.

## Defaults for everyone

Open **Administration**, **Notifications and email**.

**Send notifications by email** is the master switch for email from the application's side. Each person can still turn email off for themselves. When it is off, or when no mail server is configured, the email column in everyone's profile is unavailable.

The table has one row for each kind of notification and three check boxes.

| Column | Meaning |
| --- | --- |
| **In the application** | People get this kind in the bell unless they turn it off. |
| **By email** | People get this kind by email unless they turn it off. |
| **Same for everyone** | Locks the row: everyone gets exactly what the row says, and the check boxes in their profile are grayed out with *Set by an administrator for everyone*. |

The kinds, and what Gantry ships with:

{{notification_categories}}

The last three are sent only to accounts that hold the permission named; other people do not see those rows at all.

A person's own choice wins over your default, unless you lock the row. Changing a default therefore changes what happens for people who never made a choice and leaves the others alone.

Suggestions:

- **Lock *Assigned to you* and *Mentions* on** in the application, so that nobody can miss work given to them directly.
- Leave **Status changes** and **Other changes** off for email. They are frequent, and people who want them can turn them on.
- **Lock *Security incidents* on** for both channels.

## Reminders about work

| Setting | Ships as | Notes |
| --- | --- | --- |
| **Remind this many days before something is due** | 2 | 0 turns the reminder off. Up to 30. |
| **Remind about overdue work every … days** | 3 | 0 turns the reminder off. The first reminder comes the day after the due date. |
| **Hour of the day** | 8 | 0 to 23, in each person's own time zone. People whose time zone is Automatic get the organization's, set under [Settings](site.md#this-site). |

Each person gets **one** message a day at most, listing their work that is due today, due soon and overdue. Work also always gets a reminder on the day it is due. Reminders go to the assignee, for work that is not finished and not archived.

Press **Save**.

## Email delivery

The panel beneath shows whether email is **Set up**: the mail server in use, the sender address, and how many messages were sent, failed and not sent in the last 24 hours. The table lists the last hundred messages with the time, the recipient, the subject and the **outcome**:

| Outcome | Meaning |
| --- | --- |
| **sent** | The mail server accepted the message. What happens after that (spam filtering, a full mailbox) is outside Gantry. |
| **failed** | The mail server refused it or could not be reached, after retries. The detail column has the server's answer. |
| **skipped** | Not sent on purpose: email was turned off, the account was deactivated, or its address changed after the message was queued. |

The log holds who was written to and the subject, never the body. It is kept for 90 days.

When the panel says **Not set up**, a system administrator needs to configure the mail server: see [Email](../sysadmin/email.md). If you also hold *admin.settings*, **Send a test message** sends one now and shows the mail server's answer.

## Alerts

An **alert** watches a condition and tells people when it is met.

Open **Administration**, **Alerts**. There are two groups.

- **System: performance, errors and security** is the system administrator's (*admin.settings*). It is described in [Alerts and monitoring](../sysadmin/alerts.md).
- **Application: work, requests and accounts** is yours (*admin.notifications*).

Every alert is measured **once a minute**. When its condition becomes true, the alert *fires* and the people and channels you chose are told; when it becomes false again, they can be told that too.

### What you can watch

{{alert_kinds_application}}

Three alerts are there from the start: **Integration deliveries failing**, **Service-level targets missed** and **Requests waiting without an owner**. They tell application administrators in the application. Change them to suit, or turn them off.

### Define an alert

Press **New alert** in the application group.

| Field | Meaning |
| --- | --- |
| **What to watch** | One of the measurements above. |
| **Name** | What people will see: *Critical bugs unassigned*, *IT desk falling behind*. |
| The measurement's own settings | The period and the threshold. *Overdue work* and *Stalled work* also take **Only in this project**: a project key, or empty for everywhere. |
| **Severity** | *Information*, *Warning* or *Critical*. Shown with the alert and sent to chat. |
| **Do not repeat within … minutes** | A condition that keeps flipping announces itself once in this period. Every change is still recorded under **Recent alerts**. |
| **Who is told** | **Accounts with the role**: select one or more workspace roles. Each person holding the role is told **in the application** and **by email**, as far as you select those two boxes and as far as their own notification choices allow. |
| **Also send email to** | Addresses outside the application, comma separated: an on-call mailbox, a pager gateway. |
| **Post to** | Slack, Microsoft Teams and webhook integrations that are set up under [Integrations](integrations.md). The message says what happened, with the severity and a link. |
| **Also say when it is back to normal** | Sends a second message, *Resolved: …*, when the condition ends. |
| **This alert is on** | Turn an alert off without deleting it. |

**Measure now and send a test** in the dialog measures the condition immediately, tells you the value, and sends a message marked as a test through every channel of the alert, so you can check that it arrives before you rely on it.

### Reading the list

Each alert shows its state: **Condition met now** when it is firing, the last measured value, and **Off** when it is turned off. **Recent alerts** beneath lists when each alert fired and resolved and what was measured; it is kept for 180 days.

People receive alerts under the notification kinds **Application alerts**, **System alerts** and **Security incidents**, which they can route to the bell, to email or both in their profile, unless you locked those rows.

### Alerts that people define for themselves

Alerts here are for administrators. Any user can define an alert of their own on work: a [saved search](../user/search.md#save-a-search) with **Tell me when new work matches**. Point people to that for conditions such as *anything labeled `safety` is created* or *my project has an unassigned critical item*.

## Slack and Microsoft Teams

Two things can post to a chat channel.

- **Project events** (work created, moved, commented on, requests raised, targets missed): set up once per channel under [Integrations](integrations.md), for all projects or one. This is how a team follows its project in chat.
- **Alerts**: select the integration under **Post to** in the alert.

Neither sends personal notifications to individuals in chat; those go to the bell and to email. Nothing posted to chat ever includes internal notes of service desk requests.
