---
title: Integrations
summary: Connecting Slack, Microsoft Teams, GitHub, GitLab, Jira and anything that accepts or sends webhooks.
---

Open **Administration**, **Integrations**, **Add integration**. Each integration has a name, a kind, and optionally **one project** it is limited to; without that it applies to every project.

{{integrations}}

Secrets are encrypted when stored and never shown again. To change one, enter a new value. Each integration lists its recent **activity**: every delivery and every received call, with the result, so you can see whether it works without reading server logs. **Send test** sends a sample event.

> **Note**: what leaves Gantry never includes internal notes on service desk requests, and events about private projects go only to integrations limited to that project.

Slack, Microsoft Teams and webhook integrations can carry two kinds of message: **project events**, described below, and **alerts**: conditions an administrator watches, such as *requests waiting without an owner* or *server errors*. An alert is sent to an integration when the integration is selected under **Post to** in the alert. See [Notifications and alerts](notifications.md#alerts). Personal notifications (assigned to you, mentioned) are not sent to chat; they go to the bell and to email.

## Slack

**Post events to a channel**

1. In Slack, create an app for your workspace and add an **incoming webhook** for the channel.
2. Add a Slack integration in Gantry and paste the webhook URL as its secret.
3. Choose the events to send.

**The `/gantry` command**

1. In the Slack app, add a slash command `/gantry`. As its request URL, enter the *URL to enter in the other system* that Gantry shows for the integration.
2. Copy the app's **signing secret** into the integration.
3. Enter the ID of the Gantry account the command acts as. Create an agent for this with a role in the projects it should reach, so that Slack users get exactly that access and no more.

In Slack: `/gantry show OPS-12`, `/gantry create OPS Replace the badge reader`, `/gantry find badge reader`. Every call is verified with Slack's signature and timestamp; unsigned or replayed calls are refused and recorded as security events.

## Microsoft Teams

Microsoft retired the old Office 365 connectors; Gantry uses the replacement, **Workflows**.

1. In the Teams channel choose **Workflows** and create one from the template *Post to a channel when a webhook request is received*.
2. Copy the address the workflow gives you.
3. Add a Teams integration in Gantry with that address as its secret and choose the events.

Messages arrive as cards with the item, what happened and a button to open it.

## GitHub and GitLab

These link code to work. When a commit message, pull request or merge request mentions an item key such as `PUMP-12`, the item gains a link to it. With *complete on merge* turned on, a merged request whose description says `fixes PUMP-12`, `closes PUMP-12` or `resolves PUMP-12` also moves that item to its first *Done* status.

1. Add a GitHub or GitLab integration and enter a long random **secret**.
2. Copy the *URL to enter in the other system*.
3. **GitHub**: in the repository or organization settings add a webhook with that URL, content type `application/json`, the same secret, and the events *Pushes* and *Pull requests*.
   **GitLab**: in the project or group settings add a webhook with that URL and the push and merge request events. On GitLab 19.0 or newer use a **signing token**; on older versions use the **secret token**.

Calls without a valid signature are refused. A delivery received twice is applied once.

## Jira

Jira is an **import**, for moving work into Gantry or keeping a copy up to date.

1. Add a Jira integration with the base URL, whether it is *cloud* or *datacenter*, and credentials: for Cloud an account email and API token; for Data Center a personal access token.
2. Press **Import issues**. Choose the Gantry project to import **into**, **which issues** as a JQL query (for example `project = OPS AND updated >= -30d`), a limit, and the type new items get when Jira's type has no match.
3. The import runs in the background; follow it under the integration's activity and **Background jobs**.

Each imported item keeps its Jira key as a reference. Running the same import again updates those items instead of creating duplicates. Summary, description and comments (converted to Markdown), status, priority, labels, due date and the parent relation are carried over. Assignee, reporter and comment authors are matched by email address when an account exists.

## Webhooks to anything else

A **webhook** integration posts each chosen event as JSON to an HTTPS address of yours. Requests are signed in the [Standard Webhooks](https://www.standardwebhooks.com/) format, so the receiver can verify them with an off-the-shelf library; the payload and headers are described in [Events and webhooks](../reference/events.md).

For anything that should *change* Gantry (a monitoring system opening an incident, a script updating a field), use the [API](../reference/api.md) with an agent's token.

## When a delivery fails

A failed delivery is retried several times with growing delays, then listed under **Background jobs** as failed. The integration's activity shows the response each attempt received.

| Activity shows | Usually means |
| --- | --- |
| 404 or 410 from Slack or Teams | The webhook was removed. Create a new one and update the secret. |
| *blocked address* | The address points at a private network. By default Gantry refuses to call internal addresses; a system administrator can allow it (`security.block_private_networks`). |
| *signature mismatch* on inbound calls | The secret differs between the two systems. Enter it again on both sides. |
| Timeouts | The receiver is slow or unreachable from the server. |
