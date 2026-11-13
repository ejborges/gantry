---
title: Logs
summary: What Gantry logs, the format of each kind of line, and the queries that answer common questions.
---

## Where logs go

Gantry writes to standard output and systemd's journal collects it. There are no log files of Gantry's own to rotate; the journal's limits apply (`SystemMaxUse=` in `/etc/systemd/journald.conf`).

```bash
journalctl -u gantry -f                 # follow
journalctl -u gantry --since "1 hour ago"
journalctl -u gantry -o cat             # only the JSON, one object per line
```

nginx keeps its own access and error logs for the site in `/var/log/nginx/gantry.access.log` and `gantry.error.log`, rotated by the nginx package.

To forward logs to a central system, forward the journal (`systemd-journal-upload`, rsyslog's `imjournal`, or your collector's journald input). Each line is already structured.

## Format

Every line is one JSON object with `timestamp` (UTC), `level`, `message`, `target` and the fields of that kind of line. The `target` says which kind it is.

| Target | One line for every | Level |
| --- | --- | --- |
| `http` | Request served. | INFO; DEBUG for health checks and static files |
| `audit` | Action recorded in the audit log. | INFO |
| `security` | Refusal or suspicious event. | WARN; ERROR when critical |
| `jobs` | Background job outcome and worker lifecycle. | INFO; WARN on failure |
| `events` | Event emitted to notifications and integrations. | INFO |
| `integrations` | Outbound delivery and inbound webhook. | INFO; WARN on failure |
| `gantry` | Start, stop, configuration warnings, unexpected errors. | any |

### A request

```json
{"timestamp":"2026-11-10T21:44:32.502708Z","level":"INFO","message":"request","request_id":"f1cd1338e1a578d7","method":"POST","path":"/api/v1/auth/login","status":200,"duration_ms":36.437,"user":"","ip":"127.0.0.1","target":"http"}
```

`request_id` is unique per request, is returned to the client in the `X-Request-Id` header, and is shown to users as the *Reference* on server errors. `user` is the account's ID once authenticated. Query strings and bodies are never logged, so secrets and personal data in them do not reach the journal.

### An audited action

```json
{"timestamp":"2026-11-10T21:44:32.346914Z","level":"INFO","message":"audit","action":"system.start","actor":"system","actor_kind":"system","auth":"","target_type":"node","target_id":"vm-15f12d","outcome":"success","ip":"","request_id":"","detail":"{\"version\":\"0.2.0\"}","target":"audit"}
```

The same entry is stored in the database's audit log, which is the authoritative, tamper-evident copy; the journal line lets a log system alert on it. `auth` says how the actor was authenticated: `session` for a browser, `token` for an API token, `cli` for the command line, `sso:NAME` for an account created by single sign-on. `actor` is the account's email address, or `system`.

### A security event

```json
{"timestamp":"2026-11-10T21:44:34.587448Z","level":"WARN","message":"security event","kind":"auth.login_failed","severity":"warning","actor":"","ip":"127.0.0.1","request_id":"5c853c34c3261551","path":"/api/v1/auth/login","detail":"{\"email\":\"x@example.org\"}","target":"security"}
```

| `kind` | Meaning |
| --- | --- |
| `auth.login_failed` | Wrong password or unknown account. |
| `auth.lockout` | An account was locked after repeated failures. |
| `auth.login_throttled` | An address exceeded the failed sign-in limit. |
| `auth.password_change_failed` | Someone signed in gave a wrong current password when changing it. |
| `auth.sso_failed` | A single sign-on response failed validation; the detail names the check. |
| `auth.token_rejected` | An API token was unknown, revoked or expired. |
| `csrf.rejected` | A change was attempted without a valid anti-forgery token, or from another origin. |
| `authz.denied` | An authenticated caller attempted something their roles do not allow. |
| `webhook.signature_invalid` | An inbound webhook had a missing or wrong signature. |
| `mcp.origin_rejected` | A browser on another website tried to call the MCP endpoint. |
| `rate_limit.exceeded` | A caller exceeded the request rate limit. |
| `audit.chain_broken` | An integrity check of the audit log failed. Severity critical. |

## Answering questions from the logs

These use `jq`; install it with `dnf install jq`.

**What happened to the request a user reported?**

```bash
journalctl -u gantry -o cat --since today | jq -c 'select(.request_id == "f1cd1338e1a578d7")'
```

**Which requests failed in the last hour?**

```bash
journalctl -u gantry -o cat --since "1 hour ago" | jq -c 'select(.target == "http" and .status >= 500)'
```

**Which requests were slow?**

```bash
journalctl -u gantry -o cat --since "1 hour ago" | jq -c 'select(.target == "http" and .duration_ms > 1000) | {timestamp, path, duration_ms, user}'
```

**Who is failing to sign in, and from where?**

```bash
journalctl -u gantry -o cat --since "24 hours ago" \
  | jq -r 'select(.target == "security" and .kind == "auth.login_failed") | .ip' | sort | uniq -c | sort -rn | head
```

**What did one account do?**

```bash
journalctl -u gantry -o cat --since "7 days ago" | jq -c 'select(.target == "audit" and .actor == "ada@example.org") | {timestamp, action, target_type, target_id, outcome}'
```

For anything older than the journal keeps, use the audit log in the console or query the `audit_log` table.

**Why did a delivery to chat fail?**

```bash
journalctl -u gantry -o cat --since "1 hour ago" | jq -c 'select(.target == "integrations" and .level != "INFO")'
```

## Verbosity

```toml
[logging]
level = "info"      # error, warn, info, debug or trace
format = "json"     # or "text" when reading at a terminal
```

`debug` adds health-check and static-file requests and more detail from background jobs. Use it while diagnosing and return to `info`. The level also accepts per-target filters, for example `info,integrations=debug`.

## What is never logged

Passwords, session and API tokens, the secret key, integration secrets, request and response bodies, and the content of items and pages are not written to the logs. Email addresses appear in audit and security lines because they identify who acted; treat the journal as personal data accordingly.
