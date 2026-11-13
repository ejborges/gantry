---
title: Installing
summary: From a freshly installed Enterprise Linux machine to a running Gantry in two commands.
---

## Requirements

| | |
| --- | --- |
| Operating system | Red Hat Enterprise Linux 9 or 10, or an equivalent: Rocky Linux, AlmaLinux, Oracle Linux. x86_64 or aarch64. |
| Machine | 2 CPU cores and 4 GB of memory to start; see [Scaling out](scaling.md) for larger teams. Disk for the database and attachments. |
| Network | A DNS name for the server, and ports 80 and 443 reachable by users. Access to your distribution's package repositories during installation. |
| Nothing else | Gantry's packages bring PostgreSQL and nginx with them as dependencies. |

## Install the packages

Copy the release directory to the server. It contains the packages in two groups and an installer script.

```
gantry-{{version}}/
  install.sh
  core/      gantry, gantry-web, gantry-manual, gantry-mcp
  addons/    gantry-addon-nasa, gantry-addon-pgvector, gantry-addon-valkey,
             gantry-addon-opensearch, gantry-addon-objectstore
```

As root:

```bash
./install.sh
```

The script does two things that a package cannot do for itself, then installs the core group with `dnf`:

- on Enterprise Linux 9 it selects PostgreSQL 16 (`dnf module enable postgresql:16`), because the default there is an older release;
- it installs every dependency: PostgreSQL server, nginx, OpenSSL and the SELinux tools.

To install by hand instead:

```bash
dnf module enable -y postgresql:16      # Enterprise Linux 9 only
dnf install -y ./core/*.rpm
```

## Set up

```bash
gantry-setup --hostname gantry.example.org --admin-email you@example.org
```

`gantry-setup` is safe to run more than once. It:

1. initializes PostgreSQL if this machine has no database cluster yet, and creates the `gantry` role and database;
2. creates the secret key in `/etc/gantry/secret.key`;
3. sets the public address in `/etc/gantry/gantry.toml`;
4. creates the database schema and the first system administrator, and prints a generated password **once**;
5. configures nginx with TLS and HTTP/2, on IPv4 and, where the machine has it, IPv6. Without `--cert` and `--key` it creates a self-signed certificate so that the service can start;
6. allows nginx to reach the application under SELinux, and opens ports 80 and 443 in firewalld;
7. starts Gantry and nginx and runs the health checks.

Open `https://gantry.example.org/`, sign in, and change the password under **Profile**.

### Use your own certificate

```bash
gantry-setup --hostname gantry.example.org \
  --cert /etc/pki/tls/certs/gantry.example.org.fullchain.pem \
  --key  /etc/pki/tls/private/gantry.example.org.key
```

Run this again whenever the files move. To renew in place, replace the files and run `systemctl reload nginx`.

### Other situations

| Situation | Option |
| --- | --- |
| The database is on another server | Set `database.url` in `/etc/gantry/gantry.toml` first, then run `gantry-setup --no-database …`. The database must exist, be owned by the role in the URL, and have the `pg_trgm` extension. |
| Another proxy or load balancer terminates TLS | `--no-nginx`. Forward to port 8600, set `server.bind` to an address the proxy can reach, and list the proxy under `server.trusted_proxies`. |
| You manage the firewall yourself | `--no-firewall` |

### What the web server must pass through

`gantry-setup` writes an nginx site that does all of this. If you put your own proxy or load balancer in front of Gantry, it must do the same.

| Requirement | Why | In the shipped nginx site |
| --- | --- | --- |
| Pass **WebSocket** upgrades for `/api/v1/ws`, with read and send timeouts of an hour | Live updates, presence and editing together | `location = /api/v1/ws` with `Upgrade` and `Connection "upgrade"` headers, `proxy_read_timeout 1h` |
| Send the original `Host`, and the client address in `X-Forwarded-For` or `X-Real-IP` | The origin check, rate limits and the audit log | `proxy_set_header` lines |
| Do not buffer request bodies, and allow bodies up to `server.max_body_mb` | Uploads | `proxy_request_buffering off`, `client_max_body_size` |
| Do not forward `/metrics` | It is for the monitoring system only | `location = /metrics { return 404; }` |
| Show `/usr/share/gantry/web/unavailable.html` for 502, 503 and 504 | People see a readable page, not a bare proxy error, while Gantry restarts | `error_page 502 503 504 /unavailable.html` |
| Do not rewrite or strip the `Content-Security-Policy` and other security headers | They are part of the protection | Nothing to do |

After changing a proxy, check live updates: sign in, open **Administration**, **Overview**, and look at the **Live updates** component. It shows the number of open connections; your own tab is one of them. In the browser, the badge **Live updates paused** appears in the top bar when the connection cannot be made.

## Optional add-ons

Install only what you need; each is one package from the `addons` directory.

```bash
./install.sh --with nasa,pgvector
# or: dnf install -y ./addons/gantry-addon-nasa-*.rpm
```

| Package | Adds | After installing |
| --- | --- | --- |
| `gantry-addon-nasa` | NASA project management and systems engineering content. | Nothing: it is loaded at once. See the [NASA add-on guide](../nasa/index.md). |
| `gantry-addon-pgvector` | Indexed vector search. | Nothing: Gantry restarts with the new setting. |
| `gantry-addon-valkey` | Valkey for counters and short-lived state. | Nothing. On Enterprise Linux 9 Valkey comes from EPEL; `install.sh` enables it. |
| `gantry-addon-opensearch` | OpenSearch in a Podman container. | `systemctl restart gantry`, then `runuser -u gantry -- gantry reindex`. |
| `gantry-addon-objectstore` | An S3-compatible store in a Podman container. | `gantry-objectstore-setup`. |

What each changes, and how to point Gantry at an engine you already run, is in [Storage back ends](storage.md).

## Verify

```bash
gantry doctor
```

```
Gantry {{version}} doctor
  ok    configuration: valid
  ok    secret key: set
  ok    public URL: https://gantry.example.org
  ok    web application: /usr/share/gantry/web
  ok    database: PostgreSQL 16.x
  ok    schema: up to date
  ok    file storage: fs
  ok    search: pg
  ok    cache: pg
  ok    email: not configured; notifications stay in the application
  ok    administrators: 1 active
  ok    audit log: 12 entries, chain intact
```

Email is optional and is set up afterward: see [Email](email.md).

Run `gantry` commands from a root shell; the tool switches to the unprivileged `gantry` account by itself and reads the same configuration as the service. Then enable the nightly backup: see [Backup and restore](backup.md).

## Remove

```bash
systemctl disable --now gantry
dnf remove 'gantry*'
```

Removing the packages leaves your data in place: the database, `/var/lib/gantry`, `/etc/gantry` and `/var/backups/gantry`. Delete those by hand, and drop the database, only when you are certain.
