---
title: Notifications
summary: What Gantry tells you about, in the application and by email, and how to choose.
---

Gantry tells you when something involves you: in the application (the bell), by email, or both. You choose per kind of notification. Your own actions never notify you.

While Gantry is open, a new notification also appears for a few seconds at the bottom of the page as it arrives, and the number on the bell goes up. Nothing is lost if you miss it: the bell keeps the list.

## Choose what you are told

Open **Profile** and find **Notifications**, or choose **Choose what you are told about** at the bottom of the bell's list.

Each row is a kind of notification with two check boxes: **In the application** and **By email**.

| Kind | You are told when | Starts as |
| --- | --- | --- |
| **Assigned to you** | A work item or request is assigned to you. | Application and email |
| **Mentions** | Someone writes `@` and your handle in a discussion, a description or a published page that you are able to open. | Application and email |
| **New messages** | There is a new message on something you reported, are assigned to or follow. | Application and email |
| **Status changes** | Something you reported, are assigned to or follow moves to another status. | Application |
| **Other changes** | Fields, dates, files or links change on something you are assigned to or follow. | Application |
| **Reminders** | Work assigned to you is due soon, due today or overdue. | Application and email |
| **New service desk requests** | A request is raised in a service desk you work in. | Application and email |
| **Service-level warnings** | A request you own misses its response or resolution target. | Application and email |
| **Saved search alerts** | New work starts to match a saved search you asked to be alerted about. | Application and email |
| **Teams** | You are put in a team position, or your time in one ends. | Application |

Administrators see three more rows: **Application alerts**, **System alerts** and **Security incidents**. They are described in the administration guides.

The *Starts as* column shows the defaults Gantry ships with. Your application administrator can change the defaults, and can fix a row for everyone; a fixed row is grayed out and says *Set by an administrator for everyone*.

> **Note**: the email column is grayed out when your organization has not connected a mail server, or when your account has no real email address.

## Follow something

You are told about work you reported or are assigned to. To hear about anything else, open it and press **Follow**; press **Stop following** to end it. The item shows how many people follow it, not who.

## Reminders

Once a day, at an hour your administrator chose (8 in the morning unless changed), Gantry sends **one** message that lists your open work that needs attention:

- work due **today**;
- work due **soon** (two days ahead unless your administrator changed it);
- work that is **overdue**, repeated every few days until it is finished or its date is moved.

The hour is counted in your own time zone when you set one in your profile, otherwise in your organization's. The message links to a search of your own unfinished work, soonest due date first.

## Alerts you define yourself

Any search can watch for new matches. In [Search](search.md#save-a-search), build the search, save it, and turn on **Tell me when new work matches**. Examples: *critical bugs with no assignee*, *requests from my project that missed a target*, *anything labeled `safety` that changed today*.

Gantry checks saved searches every few minutes and tells you only about work that newly matches.

## What an email looks like

- The **subject** starts with the name of your site in brackets and names what the message is about, for example `[Gantry] PUMP-12 Replace pump seal`.
- The **body** says what happened in one sentence (*Ada Okafor mentioned you in a message on PUMP-12*), quotes the message when there is one, and has one button, **Open in Gantry**, that takes you there.
- Messages about the same work item are threaded together by mail programs.
- The **last line** names the kind of notification and links to the place in your profile where you turn it off. There is nothing to unsubscribe from elsewhere.
- Email goes only to people who are allowed to open the thing it is about. An internal note on a service desk request is mailed only to the people who work that desk.

Email comes as plain text and as HTML; your mail program shows whichever it prefers. Automatic replies such as out-of-office messages are not read by Gantry: answer in the application.

## Slack and Microsoft Teams

When your organization has connected Slack or Microsoft Teams, project events and administrators' alerts can also be posted to a channel. That is set up per project or per alert by an administrator, not per person. See [Integrations](../admin/integrations.md).

## Not receiving something?

| Symptom | Check |
| --- | --- |
| Nothing in the bell | The row's **In the application** box in your profile. For messages and changes: are you the reporter, the assignee, or following? |
| In the bell but no email | The row's **By email** box. Then your spam folder. Then ask an administrator whether email is turned on. |
| No reminders | Reminders are sent for work that is assigned to you, has a due date and is not finished. |
| Too much | Clear the boxes for **Status changes** and **Other changes** first; they are the most frequent. |
