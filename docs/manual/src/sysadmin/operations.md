---
title: Running and monitoring
summary: Starting and stopping, health checks, metrics, background jobs and routine maintenance.
---

## Services

| Unit | Purpose | Default |
| --- | --- | --- |
| `gantry.service` | The application. | Enabled by `gantry-setup`. |
| `gantry-worker.service` | A separate background worker. | Off. Only for [larger installations](scaling.md). |
| `gantry-backup.timer` | Nightly backup. | Off until you enable it. |
| `postgresql.service`, `nginx.service` | The database and the proxy. | Enabled by `gantry-setup`. |

```bash
systemctl status gantry
systemctl restart gantry       # a few seconds; requests in flight are allowed to finish
journalctl -u gantry -f
```

Each start first runs `gantry migrate`, which brings the database schema and built-in content up to date and does nothing when they already are.

The service is confined by systemd: it runs without privileges, sees the file system read-only except for `/var/lib/gantry`, has no access to home directories or devices, and may use only network and local sockets.

## Health checks

| Address | Answers | Use it for |
| --- | --- | --- |
| `/healthz` | `ok` when the process is up. | Restart decisions (liveness). |
| `/readyz` | `ready` when the database is reachable and the schema is current; otherwise status 503. | Load balancer membership (readiness). |

Both are reachable without signing in and reveal nothing else.

```bash
curl -fsS http://127.0.0.1:8600/readyz
```

`gantry doctor` checks more: configuration, key, database, schema, file storage, search, cache, that an administrator exists and that the audit chain is intact. It exits non-zero when any check fails, so it can be run from a monitoring system.

## Metrics

`/metrics` serves Prometheus text. It answers on loopback, or anywhere with `Authorization: Bearer` and the configured `server.metrics_token`. nginx does not forward it.

| Metric | Meaning | Watch for |
| --- | --- | --- |
| `gantry_http_requests_total` | Requests served. | |
| `gantry_http_request_duration_seconds_sum` | Time spent serving. Divide by the request count for the mean. | A rising mean. |
| `gantry_http_server_errors_total` | Responses with status 500 and above. | Any sustained rate. |
| `gantry_http_client_errors_total` | Responses with status 400 to 499. | Sudden jumps. |
| `gantry_auth_failures_total` | Failed sign-ins and rejected tokens. | Spikes: see [Security](security.md). |
| `gantry_security_events_total` | Security events recorded. | Spikes. |
| `gantry_rate_limited_total` | Requests refused by the rate limit. | A client in a loop. |
| `gantry_jobs_queued` | Background jobs waiting. | Growth that does not drain. |
| `gantry_jobs_dead` | Jobs that used all their retries. | Anything above zero. |
| `gantry_jobs_done_total`, `gantry_jobs_failed_total` | Job outcomes. | |
| `gantry_db_pool_connections` | Open database connections. | Staying at `database.max_connections`. |
| `gantry_audit_write_failures_total` | Audit entries that could not be written. | **Anything above zero. Alert on this.** |
| `gantry_uptime_seconds` | Time since start. | Unexpected resets. |

A minimal Prometheus job, run on the server itself:

```yaml
scrape_configs:
  - job_name: gantry
    static_configs:
      - targets: ["127.0.0.1:8600"]
```

Suggested alerts: `/readyz` failing for one minute; any increase of `gantry_audit_write_failures_total`; `gantry_jobs_dead > 0`; server errors above one per minute for five minutes.

Gantry also watches most of these itself and tells system administrators in the application, by email and in chat: see [Alerts and monitoring](alerts.md). Use both: Gantry's own alerts need Gantry to be running.

## Background jobs

Email, deliveries to chat and webhooks, search embeddings, imports and the service-level clock run as jobs in a queue in PostgreSQL. Once a minute one node also sends due reminders, evaluates alerts and checks saved searches for new matches. A failed job is retried with growing delays; when its attempts are used up it is kept as *dead* for an administrator to inspect and retry in the console under **Background jobs**. Jobs survive restarts.

## When the application is unavailable

While Gantry is stopped, restarting or being upgraded, nginx cannot reach it. People then see `/usr/share/gantry/web/unavailable.html` in place of a bare *502 Bad Gateway*: a plain page, in the device's light or dark theme, that says the application is not answering, that work stored on the server is not affected, and that the page **tries again by itself every 30 seconds**. A tab left open returns to where it was when Gantry is back.

The page is a static file served by nginx itself, so it works when nothing else does. It contains no script and loads nothing else.

**Add your support contact to it.** The page ends with a general sentence about whom to contact, because it cannot read the contact you entered in the administration console while the application is down. To name your help desk:

```bash
install -d /usr/local/share/gantry
cp /usr/share/gantry/web/unavailable.html /usr/local/share/gantry/unavailable.html
vi /usr/local/share/gantry/unavailable.html      # replace the sentence under "Getting help"
```

Then in `/etc/nginx/conf.d/gantry.conf` change `root /usr/share/gantry/web;` inside `location = /unavailable.html` to `root /usr/local/share/gantry;`, **delete the first line of the file** (the line that says it is managed by `gantry-setup`, so that upgrades leave your change alone), and run `nginx -t && systemctl reload nginx`. From then on the site file is yours: after an upgrade, compare it with `/usr/share/gantry/nginx/gantry.conf.in` for new locations.

With a separate load balancer, put the file on the balancer; `deploy/multi-node/nginx-lb.conf` shows where.

When Gantry is running, errors are shown by Gantry itself, with the support contact from **Administration**, **Settings**: see [the user guide](../user/troubleshooting.md#error-pages). A browser that opens an address which fails (not found, not permitted, a server error) gets a readable page with the same status code; programs that call the API get the same error as JSON.

## What is removed automatically

Gantry removes its own short-lived records once a minute. Nothing here needs a scheduled task.

| Record | Kept for |
| --- | --- |
| Sessions | Until they expire or have been idle for `auth.session_idle_minutes` |
| Notifications in people's inboxes | 90 days |
| Email delivery log | 90 days |
| Integration delivery log | 30 days |
| Alert history | 180 days |
| Per-minute measurements behind alerts | 7 days |
| Finished background jobs | 7 days. Failed jobs are kept until someone deals with them. |
| Records of server nodes that stopped reporting | 7 days |
| Unsaved working copies of text being edited together | 30 days after the last change. Saved content is not affected. |
| Single sign-on requests in progress | 15 minutes |

Kept until you remove them: work items, pages and their versions, attachments, the **audit log** and **security events**.

## Routine maintenance

| How often | Task |
| --- | --- |
| Daily, automatic | Backup (`gantry-backup.timer`). Expired sessions are removed by the application itself. |
| Weekly | Look at **Administration**, **Overview** and **Background jobs**; check disk space for `/var/lib/pgsql` and `/var/lib/gantry`. |
| Monthly | Apply operating system updates (`dnf upgrade`), which include PostgreSQL and nginx fixes. Review the security events. Test that a backup restores. |
| Before the TLS certificate expires | Replace it and `systemctl reload nginx`. |
| As released | Upgrade Gantry: see [Upgrading](upgrade.md). |

PostgreSQL's autovacuum keeps the database healthy without intervention at this scale. The audit log grows without limit by design; plan disk for it (a few hundred bytes per action).

## Command-line tasks

```bash
gantry admin create --email you@example.org --name "Your Name" --generate
gantry admin reset-password --email you@example.org     # reads the new password from standard input
gantry reindex                                          # rebuild the search index
gantry audit verify                                     # check the audit chain
gantry pack list                                        # add-on packs on disk and installed
```

Run them from a root shell. When started by root, `gantry` switches to the unprivileged `gantry` account by itself before it does anything, so that it can reach PostgreSQL the same way the service does and never runs with root's privileges. All of them are recorded in the audit log as done from the command line. The full list is in [the command-line reference](../reference/cli.md).
