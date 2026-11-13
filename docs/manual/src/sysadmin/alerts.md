---
title: Alerts and monitoring
summary: Be told about performance problems, errors, failing components and security incidents, in the application, by email, and in Slack or Microsoft Teams.
---

Gantry watches itself. A set of **system alerts** measures the server every minute and tells system administrators when something needs attention. They are on from the first start, and you can change every one of them.

This chapter covers alerts inside Gantry. They need Gantry to be running; to learn that Gantry is *not* running, also watch it from outside. See [Monitoring from outside](#monitoring-from-outside).

## What is watched from the start

Open **Administration**, **Alerts**, group **System: performance, errors and security** (needs *admin.settings*).

| Alert | Fires when | Severity | Repeats at most every |
| --- | --- | --- | --- |
| **Server errors** | More than 5% of requests ended in a server error over 10 minutes (at least 50 requests) | Critical | 60 min |
| **Slow responses** | The average response took more than 1,500 ms over 10 minutes | Warning | 120 min |
| **Background job backlog** | More than 500 jobs are waiting | Warning | 120 min |
| **Background jobs failing** | 5 jobs failed for good in an hour | Warning | 120 min |
| **Database connections nearly exhausted** | More than 90% of the connection pool was in use on a node | Warning | 60 min |
| **A storage or search component is down** | File storage, search or the cache does not answer its health check | Critical | 30 min |
| **Audit log could not be written** | Any audit entry could not be stored in 10 minutes | Critical | 30 min |
| **Repeated failed sign-ins** | 20 failed, throttled or locked-out sign-ins in 15 minutes | Warning | 60 min |
| **Critical security event** | Any security event of critical severity in 5 minutes | Critical | 15 min |

Each tells accounts with the **System administrator** role, in the application and by email.

## Everything you can watch

{{alert_kinds_system}}

Application administrators have a second group for conditions in the work itself (requests without an owner, missed targets, overdue work); see [Notifications, email and alerts](../admin/notifications.md#alerts).

## Change an alert or add one

Select an alert to change it, or press **New alert** in the system group.

- **Thresholds and periods**: the measurement's own settings. Tune them to your installation: a small site may want *Server errors* at 1%; a site with slow analytic queries may raise *Slow responses*.
- **Severity**: *Information*, *Warning* or *Critical*.
- **Do not repeat within … minutes**: how often the same alert may announce itself.
- **Who is told**: workspace roles (each holder is told in the application and by email, as far as their own choices allow), **extra email addresses** such as an on-call mailbox or a paging gateway, and **Slack, Microsoft Teams or webhook integrations**.
- **Also say when it is back to normal**.
- **Measure now and send a test**: measures at once, shows the value, and sends a marked test message through every channel. Use it whenever you change who is told.

Recommended additions:

| Add | Why |
| --- | --- |
| **Server node stopped reporting** | With more than one node: a node that reported in the last day and has been silent for 5 minutes. |
| **Email delivery failures** | A mail server that stopped accepting messages. Send it to chat or the application, not by email. |
| **Requests refused by the rate limit** | A client in a loop, or an attack. |
| A second **Security events** alert sent to chat | So that incidents reach the people on call wherever they are. |

Built-in alerts can be changed and turned off but not deleted, so an upgrade never brings back what you turned off, and you can always return to them.

## How alerts are measured

- Every node records its own counters once a minute: requests, server errors, client errors, time spent, requests refused by the rate limit, audit write failures, the share of database connections in use, and live connections.
- One node at a time evaluates the alerts, once a minute, over the records of **all** nodes. With several nodes an alert therefore describes the installation, not one machine.
- An alert **fires** when its condition becomes true and **resolves** when it becomes false. Both are written to **Recent alerts** with the measured value, kept for 180 days. The per-minute records behind them are kept for 7 days.
- A measurement that cannot be taken (the database is busy) is logged and tried again the next minute; it does not fire the alert.

Alerts are sent as notifications of the kinds **System alerts** (performance, errors, components) and **Security incidents** (the two security alerts and anything else measuring security events). Each administrator chooses in their profile whether those reach the bell, email or both; an application administrator can lock the choice for everyone. Lock *Security incidents* on.

## Security incidents

Security events are recorded as they happen: failed and throttled sign-ins, lock-outs, rejected tokens, denied actions, refused single sign-on responses, rejected webhook signatures, blocked outgoing requests, refused uploads.

- **Administration**, **Security events** lists them with who, from where, what and how severe. AI agents are named and marked.
- The two built-in alerts turn bursts and critical events into notifications.
- The server log carries each event under the target `security`, for forwarding to your log system.
- `gantry_security_events_total` and `gantry_auth_failures_total` are exported for Prometheus.

What to do about each kind of event is in [Security](security.md#responding).

## Monitoring from outside

An alert from Gantry cannot tell you that Gantry, its database or its mail server is down. Watch these from another system:

| Check | How | Alert when |
| --- | --- | --- |
| The application answers | `GET /readyz` through the public address | Not `200` for one minute |
| The process is alive | `GET /healthz`, or `systemctl is-active gantry` | Failing |
| The whole path works | `gantry doctor` from a timer; it exits non-zero on any failed check | Non-zero |
| Metrics | Scrape `/metrics`; see [Running and monitoring](operations.md#metrics) | `gantry_audit_write_failures_total` rising; `gantry_jobs_dead > 0`; error rate |
| The audit chain | `gantry audit verify` from a timer | Non-zero |
| Certificates | Your usual expiry check on the public certificate, and the SAML signing certificates shown under **Sign-in** | 30 days before expiry |
| Backups | That last night's backup exists and restores | Missing |

When Gantry is down, people see a plain page from the web server that says so and retries by itself; see [When the application is unavailable](operations.md#when-the-application-is-unavailable).
