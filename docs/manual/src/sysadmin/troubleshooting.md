---
title: Troubleshooting for system administrators
nav: Troubleshooting
summary: A routine for diagnosing problems, and the causes behind the common ones.
---

## Start here

```bash
systemctl status gantry postgresql nginx
gantry doctor
journalctl -u gantry -n 50 -o cat | jq -r '[.timestamp, .level, .target, .message] | @tsv'
curl -fsS http://127.0.0.1:8600/readyz
```

`gantry doctor` names the failing part in most cases. With a **reference** from a user's error message, go straight to that request:

```bash
journalctl -u gantry -o cat --since today | jq -c 'select(.request_id == "REFERENCE")'
```

## The service does not start

| `journalctl -u gantry` shows | Cause and fix |
| --- | --- |
| Nothing; `systemctl status` says the condition was not met | `/etc/gantry/secret.key` does not exist. Run `gantry-setup`. |
| `invalid configuration in /etc/gantry/…` | A mistake in the named file; the message names the key. Fix it and run `gantry config check`. |
| `… is readable by every user on this machine` | The key file's permissions are too open. `chmod 640 /etc/gantry/secret.key; chown root:gantry /etc/gantry/secret.key`. |
| `cannot connect to PostgreSQL: …` with *Connection refused* or *No such file or directory* | PostgreSQL is not running: `systemctl start postgresql` and read its journal. |
| `role "gantry" does not exist` or `database "gantry" does not exist` | The database was not prepared. Run `gantry-setup`. |
| `Peer authentication failed for user "gantry"` | `pg_hba.conf` no longer has the default `local all all peer` line. Restore it, or use a password in `database.url`. |
| `Address already in use` | Another process holds port 8600. Find it with `ss -ltnp \| grep 8600`. |
| `security.secret_key is not valid base64` or `must decode to 32 bytes` | The key file was edited or truncated. Restore it from a backup. |
| A migration error | Do not retry repeatedly. Keep the output, restore the pre-upgrade backup if service must resume, and report it. |

## Users see an error page from nginx

| Symptom | Cause and fix |
| --- | --- |
| *The application is not answering right now* (or, on a site maintained by hand, 502 Bad Gateway) | Gantry is not running or is restarting. If it is running, SELinux may be blocking nginx: `getsebool httpd_can_network_connect` must be `on`; `gantry-setup` sets it. Look for denials with `ausearch -m avc -ts recent`. |
| 504 Gateway Timeout | A request took longer than nginx allows. Find slow requests in the logs; check the database's load. |
| 413 Request Entity Too Large | `client_max_body_size` in the nginx site is lower than `server.max_body_mb`. |
| Certificate warnings | The self-signed certificate is still in use, or the certificate expired or does not match the name. |

## Live updates do not work

People see **Live updates paused** in the top bar; pages work, but nothing changes until they reload, and editors say *The live connection is not available, so you are editing on your own*.

| Check | How | Fix |
| --- | --- | --- |
| Is the feature on? | `gantry config check` shows `[realtime] enabled`. **Administration**, **Overview** shows the *Live updates* component with the number of open connections. | Set `enabled = true` and restart. |
| Does the web server pass the upgrade? | `curl -i -N -H "Connection: Upgrade" -H "Upgrade: websocket" -H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==" https://gantry.example.org/api/v1/ws` answers `401` with a JSON body when the request reaches Gantry (it is not signed in). An answer of `400`, `426` or an HTML page from nginx means the proxy did not pass the upgrade. | The nginx site needs `location = /api/v1/ws` with the `Upgrade` and `Connection "upgrade"` headers. Run `/usr/libexec/gantry/nginx-site refresh`, or see [what the web server must pass through](install.md#what-the-web-server-must-pass-through). A load balancer in front needs WebSocket support turned on too. |
| Do connections drop after a minute? | The browser reconnects again and again; the journal shows *connection opened* and *connection closed* in pairs, target `realtime`. | A proxy closes idle connections. Raise its idle timeout to an hour for `/api/v1/ws`. Gantry and the browser exchange a keep-alive every 25 seconds, so a timeout shorter than that is the usual cause. |
| *this connection must be opened from the application itself* in security events (`ws.origin_rejected`) | The browser's address does not match `server.public_url`, or a proxy rewrites `Origin` or `Host`. | Make the public address match what people type; pass `Host` unchanged. |
| One person only | Their browser's developer tools show the WebSocket failing. | A browser extension, antivirus or outbound proxy on their network blocks WebSockets. Everything else works for them. |
| Several nodes: people on different nodes do not see each other | Each node's journal, target `realtime`: *lost the connection that listens for other nodes*. | A connection pooler in transaction mode blocks `LISTEN`/`NOTIFY`. Use session mode. |
| *Too many requests* when opening another tab | One account has more than `realtime.max_connections_per_user` tabs connected. | Close tabs, or raise the limit. |

## Email is not delivered

Start at **Administration**, **Notifications and email**: it shows whether email is set up, counts for the last day and the mail server's answer for each failure. Then:

```bash
gantry doctor                                                   # the "email" line connects and signs in
journalctl -u gantry -u gantry-worker --since "1 hour ago" -o cat | jq -c 'select(.target == "email")'
```

Causes and fixes are listed in [Email](email.md#common-failures).

## Alerts

| Symptom | Cause and fix |
| --- | --- |
| No alert ever fires | Alerts are evaluated by a node that runs background work. With `worker.embedded = false`, a `gantry-worker` must be running. The journal, target `alerts`, shows each evaluation that fails. |
| *Slow responses* fires at night | Few requests make averages jumpy. Raise **Ignore periods with fewer requests than**. |
| *A storage or search component is down* | **Administration**, **Overview** names the component; `gantry doctor` gives the error. |
| *Server node stopped reporting* after removing a node on purpose | The alert resolves by itself a day after the node last reported. |
| Alerts arrive twice | Two installations share one database, or the same address is both a role holder and an extra address. |

## People cannot sign in

| Symptom | Cause and fix |
| --- | --- |
| Signing in appears to work, then returns to the sign-in page | `server.public_url` does not match the address in the browser, or is `https` while people reach the site over `http`, so the browser does not send the cookie. |
| Every change fails with *forbidden* but reading works | Same cause: the origin check. Also happens when a proxy rewrites the `Host` or `Origin` header. |
| Everyone appears to come from one address and is throttled together | The proxy is not listed in `server.trusted_proxies`, or does not send `X-Real-IP` / `X-Forwarded-For`. |
| No administrator can sign in | `gantry admin create --email you@example.org --name "You" --generate`, or `gantry admin reset-password --email …`. If local sign-in is off, set `auth.allow_local_login = true` and restart first. |

## Slowness

1. Is it the database? `runuser -u postgres -- psql -c "SELECT pid, now() - query_start AS running, state, left(query, 80) FROM pg_stat_activity WHERE datname = 'gantry' AND state <> 'idle' ORDER BY 2 DESC"`.
2. Is the connection pool exhausted? `gantry_db_pool_connections` at the limit with waiting requests: raise `database.max_connections` (and PostgreSQL's `max_connections` if needed).
3. Is one client hammering the API? Group `http` log lines by `user`.
4. Is semantic search slow? Install [pgvector](storage.md#pgvector).
5. Is the machine out of memory or disk? `free -h`, `df -h`.

## Background jobs pile up

`gantry_jobs_queued` grows and does not fall.

- Is a worker running? With `worker.embedded = false`, `gantry-worker.service` must run somewhere.
- Is one integration timing out? Its deliveries hold workers until the timeout. Look at target `integrations` in the logs, fix or disable that integration.
- For sustained volume, raise `worker.concurrency` or add a worker node.

## Search

| Symptom | Fix |
| --- | --- |
| Results missing after a restore or a change of search engine | `gantry reindex` |
| *Similar meaning* finds nothing | The embeddings provider is `none`, unreachable, or embeddings are still queued. Check **Overview** and the job queue. |
| OpenSearch is down | Gantry reports the search component as unhealthy. Fix OpenSearch, or remove the add-on to fall back to PostgreSQL and `gantry reindex`. |

## Disk full

PostgreSQL stops accepting writes when its disk is full, and Gantry reports not ready. Free space (old backups under `/var/backups/gantry` are the usual culprit, then the journal: `journalctl --vacuum-size=1G`), then `systemctl restart postgresql gantry`.

## Collecting information for a report

```bash
gantry --version
gantry doctor
rpm -qa 'gantry*' postgresql-server nginx
gantry config check                       # secrets are left out
journalctl -u gantry --since "1 hour ago" -o cat > gantry.log
```

Review `gantry.log` before sharing it: it contains email addresses and network addresses.
