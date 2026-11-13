---
title: Email
summary: Connect a mail server so that Gantry can send notifications, reminders and alerts; test it, watch deliveries, and keep messages out of spam folders.
---

Gantry sends email through a mail server you already have: your organization's relay, a hosted service, or a mail server on the same machine. It speaks SMTP to it. It never receives mail and needs no mailbox.

Until a mail server is configured, everything else works and people are notified in the application only.

## Configure

Add an `[email]` section to `/etc/gantry/gantry.toml`.

```toml
[email]
enabled = true
host = "smtp.example.org"
port = 587
encryption = "starttls"
username = "gantry"
password_file = "/etc/gantry/smtp.password"
from = "Gantry <gantry@example.org>"
reply_to = ""
timeout_seconds = 20
```

| Setting | Meaning | Default |
| --- | --- | --- |
| `enabled` | Turns sending on. | `false` |
| `host` | The mail server's name. | none |
| `port` | 587 with STARTTLS, 465 with TLS from the start, 25 for an internal relay. | `587` |
| `encryption` | `starttls`, `tls` or `none`. With `starttls` and `tls` the server's certificate is verified. Use `none` only for a relay on the same machine or a trusted private network: the message, and the password if there is one, travel in the clear. | `starttls` |
| `username` | Leave empty for a relay that accepts mail by address. | empty |
| `password`, `password_file` | The password, or better, a file that contains it. The file must not be readable by everyone: `chown root:gantry` and `chmod 640`. Gantry refuses to start otherwise. | empty |
| `from` | The sender, as `Name <address>`. Use an address in a domain you control. | none |
| `reply_to` | Where replies go. Gantry does not read replies, so point this at a person, a help desk, or leave it empty and use a sender address that says not to reply. | empty |
| `timeout_seconds` | How long to wait for the mail server. | `20` |

Two more conditions must hold, and `gantry config check` reports them:

- `server.public_url` must be set, because every message links back to the application;
- `from` must be a valid sender.

Create the password file, check, and restart:

```bash
install -m 640 -o root -g gantry /dev/null /etc/gantry/smtp.password
printf '%s' 'the-password' > /etc/gantry/smtp.password
gantry config check
systemctl restart gantry
```

If the mail server is not on the same machine and SELinux is enforcing, the service may already connect out: Gantry's own unit is not restricted from making outgoing connections. A local firewall that restricts outgoing traffic must allow the mail server's port.

## Test

Open **Administration**, **Notifications and email**. The panel shows the mail server and sender in use. **Send a test message** (needs *admin.settings*) sends one to your own address, or to one you type, and shows the mail server's answer on failure, for example *the mail server did not accept the message: … authentication failed*.

From the command line, the same test with a token of a system administrator:

```bash
curl -fsS -X POST https://gantry.example.org/api/v1/admin/email/test \
  -H "Authorization: Bearer $GANTRY_TOKEN" -H 'Content-Type: application/json' \
  -d '{"to":"you@example.org"}'
```

`gantry doctor` includes an **email** line: it connects to the mail server, greets it and signs in, without sending anything, and reports the answer. Run it after changing the `[email]` section.

## How sending works

- A notification that should go out by email becomes a **background job**. Web requests never wait for the mail server.
- If the mail server refuses a message or cannot be reached, the job is retried after 30 seconds, 2 minutes, 8 minutes, 32 minutes and 2 hours. After six attempts it is marked failed and stays visible under **Background jobs**, where it can be retried by hand.
- Each attempt is written to the **delivery log** (who, which kind of notification, the subject, the outcome and the mail server's answer on failure; never the body) and to the server log under the target `email`.
- With several application nodes, any node or worker may send. All of them need the same `[email]` settings and network access to the mail server.

What is sent, to whom, and people's own choices are described in [Notifications, email and alerts](../admin/notifications.md) and, for users, [Notifications](../user/notifications.md).

## What a message contains

- A subject that begins with the site name in brackets.
- A plain-text part and an HTML part with the same content: what happened, an excerpt, one link to the application, and a line that names the kind of notification and links to the person's settings.
- No images, no tracking pixels, no links other than to your own installation.
- Headers: `Auto-Submitted: auto-generated`, so that well-behaved mail systems do not answer with out-of-office messages; a `Message-ID` in your public address's domain; and `In-Reply-To` and `References` that group the messages about one work item into a thread.

Email goes only to active accounts of people. AI agent accounts never receive email, and neither do deactivated accounts. A person receives a message about something only if they are allowed to open it.

## Deliverability

Whether a message reaches an inbox is decided by the receiving mail system. To keep notifications out of spam folders:

| Do | Why |
| --- | --- |
| Send from a domain you control, through a mail server that is allowed to send for it. | Receivers check that the sending server is authorized. |
| Publish **SPF** for that domain, including the mail server. | Without it many receivers reject or junk the mail. |
| Have the mail server sign with **DKIM**. | Gantry does not sign messages itself; the relay does. |
| Publish a **DMARC** policy once SPF and DKIM pass. | It tells receivers what to do with mail that fails. |
| Use a `from` address that exists, or a `reply_to` that does. | Bounces and replies need somewhere to go. |

Bounces go to the mail server, not to Gantry. An address that bounces keeps being written to until the account's address is corrected or its email choices are turned off; watch your mail server's bounce reports during rollout.

## Watching it

| Where | What |
| --- | --- |
| **Administration**, **Notifications and email** | Sent, failed and skipped in the last 24 hours; the last hundred messages with outcome and detail. |
| **Administration**, **Overview** | The **Email** component: the mail server in use, or *Not configured*. |
| **Administration**, **Alerts** | Add an alert on **Email delivery failures** (5 refused messages in an hour unless changed) so that a broken mail server is noticed. Send it in the application or to chat: an email about email not working will not arrive. |
| The server log | Target `email`: one line per message sent or failed, with the recipient and the kind, never the content. |
| **Background jobs** | Kind `email`: waiting, retrying, failed. |

The delivery log is kept for 90 days.

## Turning it off

Set `enabled = false` and restart. Queued messages are then recorded as *skipped* when their job runs. Application administrators can also stop sending without a restart: **Notifications and email**, clear **Send notifications by email**.

## Common failures

| The test says | Cause |
| --- | --- |
| *email is not configured* | `enabled`, `host` or `from` is missing, or the server was not restarted. |
| A timeout | The host or port is wrong, or a firewall drops the connection. `nc -vz smtp.example.org 587` from the server tells you. |
| A certificate error | The mail server's certificate does not match `host`, or is signed by an authority the system does not trust. Use the name on the certificate; add a private authority to the system trust store (`update-ca-trust`). |
| *authentication failed* | Wrong user name or password, or the server requires an application password. |
| *relay access denied* | The mail server does not allow this machine or this sender to send to outside addresses. |
| Sent, but nothing arrives | See [Deliverability](#deliverability), then the mail server's own log. |
