---
title: How Gantry is put together
nav: Architecture
summary: The components you run, where their files live and how they talk to each other.
---

## Components

```text
Browser / API client / AI agent
            │ HTTPS 443
        ┌───▼────┐
        │ nginx  │  TLS, redirects http to https
        └───┬────┘
            │ HTTP 127.0.0.1:8600
   ┌────────▼──────────┐        ┌──────────────────────┐
   │ gantry serve      │───────▶│ PostgreSQL 16         │
   │  web app, manual  │ socket │  data, search, queue, │
   │  REST API, /mcp   │        │  sessions, audit log  │
   │  background jobs  │        └──────────────────────┘
   └───┬───────────────┘
       │ files                     optional, by add-on
       ▼                          ┌──────────────────────┐
 /var/lib/gantry/blobs            │ Valkey · OpenSearch   │
                                  │ S3 object store       │
                                  └──────────────────────┘
```

| Component | What it does | Package |
| --- | --- | --- |
| `gantry serve` | One program: serves the web application and this manual, the REST API and the MCP endpoint, and runs background jobs. | `gantry` |
| PostgreSQL | Holds everything: work, pages, accounts, sessions, the audit log, the search index, vectors and the job queue. | `postgresql-server` (dependency) |
| nginx | Terminates TLS and forwards to the application on loopback. | `nginx` (dependency) |
| `gantry worker` | Optional separate process for background jobs on larger installations. | `gantry` |
| `gantry-mcp` | Optional bridge for AI clients that talk to a local program instead of a URL. Runs on the user's machine. | `gantry-mcp` |

A default installation is **one server** running the first three. Nothing else is needed for a team of a few hundred people. Larger installations add application nodes and move specific kinds of data to dedicated engines; see [Scaling out](scaling.md).

## Different stores for different data

Gantry reaches each kind of data through its own interface, and each interface has a built-in PostgreSQL implementation and, where it pays off at scale, a dedicated engine.

| Data | Default | Optional engine | Add-on package |
| --- | --- | --- | --- |
| Relational: items, pages, accounts, audit | PostgreSQL | | |
| Documents: custom fields, definitions, event payloads | PostgreSQL `jsonb` | | |
| Files: attachments | Local directory | S3-compatible object store | `gantry-addon-objectstore` |
| Full-text search | PostgreSQL text search | OpenSearch | `gantry-addon-opensearch` |
| Vectors: semantic search | PostgreSQL arrays, exact | pgvector HNSW index | `gantry-addon-pgvector` |
| Graph: links between items | PostgreSQL recursive queries | | |
| Counters and short-lived state | PostgreSQL unlogged table | Valkey | `gantry-addon-valkey` |
| Job queue | PostgreSQL | | |

Switching an engine is a configuration change; see [Storage back ends](storage.md).

## Where things are

| Path | Contents |
| --- | --- |
| `/usr/bin/gantry` | The server and command-line tool. |
| `/etc/gantry/gantry.toml` | Configuration. Yours to edit; upgrades never replace it. |
| `/etc/gantry/conf.d/*.toml` | Drop-ins applied on top, in name order. Add-on packages put theirs here. |
| `/etc/gantry/secret.key` | The key that encrypts stored secrets. Readable by root and the `gantry` group only. **Back it up.** |
| `/var/lib/gantry/blobs` | Attachments, when stored on local disk. |
| `/usr/share/gantry/web`, `/usr/share/gantry/manual` | The web application and this manual. |
| `/usr/share/gantry/addons` | Content packs installed by add-on packages. |
| `/etc/nginx/conf.d/gantry.conf` | The nginx site, written by `gantry-setup`. |
| `/var/backups/gantry` | Backups made by `gantry-backup`. |
| The journal | All logs: `journalctl -u gantry`. |

## Accounts and ports

- The application runs as the unprivileged system account `gantry` and connects to PostgreSQL through the local socket as the database role of the same name. There is no database password to manage on a single server.
- The application listens on `127.0.0.1:8600` only. nginx listens on 80 (redirect) and 443.
- Outbound connections are made only for what you configure: identity providers, chat and webhook integrations, Jira import, an embeddings service.

## Security model in brief

- Every request is authenticated, except sign-in itself, health checks and signed webhook receivers. Permissions are checked on the server for every action; the web application hides what a user may not do but is never what enforces it.
- Things a user may not see are answered with *not found*, not *forbidden*, so their existence is not revealed.
- Passwords are stored as Argon2id hashes; sessions and API tokens as SHA-256 hashes; integration and single sign-on secrets encrypted with AES-256-GCM under the secret key.
- Every action is written to an append-only, hash-chained audit log, and every refusal to a security event log.

The pages that follow cover [installing](install.md), [configuration](configuration.md), [day-to-day operation](operations.md), [logs](logs.md), [security incidents](security.md), [backup](backup.md), [upgrades](upgrade.md) and [troubleshooting](troubleshooting.md).
