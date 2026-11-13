---
title: When something goes wrong
nav: Troubleshooting
summary: What common messages, slow pages and error pages mean, how to get past them, and how to ask for help.
---

## Messages while you work

| What you see | Why | What to do |
| --- | --- | --- |
| *This does not exist, or you cannot see it* | The address is wrong, the item was archived, or you are not a member of its project or space. Gantry gives the same answer in all three cases on purpose. | Check the key. Ask the project's administrator to add you. |
| *You do not have permission to do that* | Your role in this project or space does not include the action. | Ask the project or space administrator for a role that does. |
| A step button is missing | The workflow does not allow that step from the current status, or your role may not move items. | Look at the steps offered; you may need an intermediate one. |
| *… is required* when moving an item | The workflow needs a field filled in before this step. | Fill the named field in under the item's details, then try again. |
| A person is missing from the assignee list | Only accounts with access to the project can be assigned its work. | Ask the project's administrator to add them. |
| *The due date cannot be before the start date* | Dates must be in order. | Move the start date first, or clear it. |
| Your page will not save: *the page changed* | Someone saved the page after you opened it, while live editing was unavailable. | Your text is still in the editor. Open the latest version in a new tab, copy over what you need, and reload. |
| **Live updates paused** in the top bar | The live connection dropped. Everything else works. | It reconnects by itself. Reload to see the latest at once. See [Working at the same time](together.md#when-the-connection-drops). |
| A name or key in gray with a dashed underline | A mention or link to something that does not exist or that you cannot see. | Nothing is wrong on your side. |
| A chart tile shows an error message | The script behind the visualization failed. | Tell its owner the message. See [Dashboards and visualizations](analytics.md#when-a-script-fails). |
| *Too many requests* | You, or a script using your token, sent a great many requests in a minute. | Wait a minute. If a script causes it, slow the script down. |
| *The file is too large* | The attachment is over the limit set by your administrator. | Compress or split the file, or link to where it is stored. |
| *That picture is … The limit is …* | A profile picture is over the size or dimension limit. | Resize it; about 256 × 256 pixels is plenty. |
| *Your session has ended* | You were signed out after inactivity, or an administrator ended your sessions. | Sign in again. |
| The simple editor is unavailable for a page | The page contains something the simple editor would lose: a diagram, footnotes, a chart, raw HTML. | Use the advanced editor for that page. |
| *This diagram could not be drawn* | The diagram text has a mistake. | Open the editor with the result beside the text; the message names the problem. |
| No email arrives | Email is off for that kind of notification, or for the whole site. | See [Notifications](notifications.md#not-receiving-something). |
| Search does not find something you know exists | You may not have access, it may be archived, or it was created seconds ago and is still being indexed. | Try again in a moment, or open it by its key. |
| The page looks unstyled or half-loaded after an upgrade | Your browser kept part of the old version. | Reload the page (<kbd>Ctrl</kbd> <kbd>Shift</kbd> <kbd>R</kbd>). |

## Slow pages

While a page loads you see gray placeholder lines where the content will be. If the wait passes about **ten seconds**, they give way to a moving bar and a sentence that names what is being loaded:

- up to 25 seconds: *This is taking longer than usual. The server is still working on it; there is no need to reload.*
- up to a minute: *Still waiting after … seconds. Large projects and slow connections can take a while.*
- after a minute: *No answer after a minute. Check your connection. If other pages open normally, tell an administrator what you were opening*, with a **Reload the page** button.

Reloading earlier rarely helps: it starts the same work again.

## Error pages

When a whole page cannot be shown, Gantry shows an error page in place of it. Every error page has the same parts:

- **what happened**, in a heading, with the code (for example *Error 404*);
- **what it means**, in a sentence or two;
- **what you can do**;
- buttons: **Go to my work** (or **Sign in**), **Reload** and **Go back**;
- **Getting help**: how to reach the people who run your installation, as your administrator has set it up: a name, a note, an email address, a web address;
- a **reference** when there is one. Quote it when you ask for help.

{{status_codes}}

The same list is in the application: open **About** from the account menu or the foot of the rail and choose **Error messages and help**. Each code also has an address of its own, such as `/status/404`, which support staff can send you; it works without signing in.

If Gantry is stopped or restarting, the web server in front of it shows a plain page that says the application is unavailable. That page reloads itself every 30 seconds, so you do not need to keep pressing reload: leave the tab open and it returns to where you were when Gantry is back.

## Reporting a problem

Tell your administrator:

1. what you were doing and the address of the page;
2. the exact message;
3. the time, and your time zone;
4. the **reference**, if the message ends with one (*Reference: f1cd1338e1a578d7*).

With the reference a system administrator can find exactly what happened in the server's logs.

## Supported browsers

Gantry runs in current versions of browsers built on Chromium (Chrome, Edge, Brave and others) and in Safari 16.4 or newer on macOS, iOS and iPadOS, at desktop, tablet and phone sizes.
