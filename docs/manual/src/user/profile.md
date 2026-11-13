---
title: Your profile
nav: Profile, status and tokens
summary: Your picture, name, organization, time zone, status, theme, notification choices, password and API tokens.
---

Open **Profile** from the bottom of the rail or from the account menu.

## You

### Your picture

Press **Upload a picture** and choose a PNG, JPEG, WebP or GIF file.

- A **square picture of about 256 × 256 pixels** works best; it is shown as a circle.
- There is a limit on the file's size and on its longer side (512 KB and 1,024 pixels unless your administrator changed them). The page states the limits in force, and a picture that is too big is refused with a message that gives its size and the limit. Pictures smaller than 32 pixels are refused too.
- **Remove** takes your picture away again.

Until you upload a picture you have a **pattern generated just for your account**: a geometric design in your own colors that nobody else has. **Draw a different pattern** gives you another one.

AI agent accounts always carry a robot mark on their picture, whichever picture they have.

### Your details

| Field | Notes |
| --- | --- |
| **Display name** | How you appear everywhere. |
| **Handle** | What people type after `@` to mention you. It is made from your email address; ask an administrator to change it. |
| **Job title** | Shown on your profile and in the org chart. |
| **Organization** | Your company, agency or department. |
| **Organization code** | A short code such as a department or cost center. |
| **Time zone** | See below. |

Your administrator decides which of these you may change yourself. A field you cannot change says *Set by an administrator*. Your email address comes from your account and is changed by an administrator.

Press **Save profile** after changing anything here.

### Time zone

Every date and time in Gantry (when a message was posted, when an item changed, when a request is due) is stored once, in UTC, and shown to each person in **their** time zone.

- **Automatic (this device's time zone)** follows the device you are using. Travel with your laptop and the times follow you. This is the default.
- **A named zone**, such as *America/Chicago*, is used on every device, wherever you are.

Hover over a time to see how long ago it was. Calendar dates, such as a due date, are days and do not shift with time zones.

Reminders are sent at an hour of your own time zone when you chose a named zone, and at an hour of your organization's time zone when you left it on Automatic.

## Status

Tell colleagues whether to expect an answer.

1. Under **Status**, choose **I am**: *Available*, *Away*, *Busy* or *Out of office*.
2. Optionally add a **message** of up to 140 characters: *In the lab until 3*, *Back Monday*.
3. Choose how long it lasts: **Until I change it**, **For an hour**, **For four hours**, **Until tomorrow** or **For a week**. When the time is up, your status returns to Available and the message is cleared.
4. Press **Set status**.

Your status appears as a dot on your picture and in words next to your name in the directory, on your profile, in lists of people to choose from and in team pages. **Clear status** removes it at once. The account menu at the top right switches between the four statuses with one click; the message and the duration are set here.

## Notifications

Choose what you are told about, in the application and by email. See [Notifications](notifications.md).

## Theme

Choose **Light**, **Dark** or **Match device**. *Match device* follows the setting of your computer or phone and changes with it, for example at night. The choice is remembered on each device separately. The same control is in the account menu.

This manual has the same three choices in its header (**Auto**, **Light**, **Dark**). When you open it from Gantry it starts in the theme you chose there.

## Password

Enter your current password and a new one. Changing it signs out your other devices. Accounts that use single sign-on have no password here.

## API tokens

A token lets a script, a command-line tool or an AI assistant use Gantry **as you**, with your access and never more.

1. Under **API tokens** press **New token**.
2. Give it a **name** that says what uses it, for example *Laptop: Claude Code*.
3. Choose what it is **allowed to** do: *Read only*, *Read and change content*, or *Read, change and administer*. Choose the least that works.
4. Choose when it **expires**.
5. Press **Create**. The token is shown **once**; copy it now.

The table shows each token's name, what it may do, when it was last used and when it expires. **Revoke** a token the moment it is no longer needed or might have been seen by someone else; it stops working immediately.

> **Important**: a token is a password. Do not paste it into chat, tickets or source code. Put it in your tool's secret store or an environment variable.

How to use a token is described in [the API reference](../reference/api.md) and, for AI clients, in [AI agents](agents.md).
