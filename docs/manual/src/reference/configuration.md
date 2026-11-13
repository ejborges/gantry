---
title: Configuration reference
summary: Every configuration key with its default, and the environment variables that override them.
---

How the file, drop-ins and environment fit together is explained in [Configuration](../sysadmin/configuration.md). This page lists what exists.

## Defaults

This is the configuration Gantry runs with when nothing is set. Keys holding secrets (`security.secret_key`, `storage.blob.secret_key`, `storage.search.password`, `storage.kv.url`, `ai.embeddings.api_key`) exist but are never printed, so they are absent here.

{{default_config}}

## Keys

### `[server]`

| Key | Meaning |
| --- | --- |
| `bind` | Address and port the application listens on. Keep it on loopback behind a proxy on the same machine. |
| `public_url` | The `https` address users type. Drives cookie security, the accepted origin, and links in notifications and redirects. |
| `web_root`, `manual_root` | Directories holding the web application and the manual. |
| `trusted_proxies` | Peers allowed to state the client address through `X-Real-IP` or `X-Forwarded-For`: addresses, or networks such as `10.0.0.0/8`. |
| `max_body_mb` | Largest accepted request body, attachments included. |
| `rate_limit_per_minute` | Requests per minute per account, or per address when not signed in. |
| `metrics_token` | Bearer token for `/metrics`. Empty: loopback clients only. |

### `[database]`

| Key | Meaning |
| --- | --- |
| `url` | PostgreSQL address. The default uses the local socket and the `gantry` system account. For a remote server: `postgres://gantry:PASSWORD@db.internal:5432/gantry?sslmode=verify-full`. |
| `max_connections` | Size of the connection pool of this process. |

### `[security]`

| Key | Meaning |
| --- | --- |
| `secret_key_file` | File holding the key that encrypts stored secrets. Must not be readable by other users. |
| `secret_key` | The key itself, base64 of 32 bytes, as an alternative to the file. |
| `block_private_networks` | Refuse outbound integration calls to private, loopback and link-local addresses. |
| `allow_insecure_outbound` | Permit `http` integration addresses. For laboratories only. |

### `[auth]`

| Key | Meaning |
| --- | --- |
| `allow_local_login` | Allow sign-in with a password stored in Gantry. |
| `password_min_length` | Minimum password length; at least 8. |
| `session_idle_minutes` | Sign out after this long without activity. |
| `session_absolute_hours` | Sign out after this long regardless. |
| `lockout_threshold`, `lockout_minutes` | Wrong passwords in a row before an account locks, and for how long. |
| `login_attempts_per_ip` | Failed sign-ins allowed from one address in five minutes. |

### `[logging]`

| Key | Meaning |
| --- | --- |
| `format` | `json` or `text`. |
| `level` | `error`, `warn`, `info`, `debug`, `trace`, or filters such as `info,integrations=debug`. |

### `[storage.blob]`, `[storage.search]`, `[storage.vector]`, `[storage.kv]`

| Key | Meaning |
| --- | --- |
| `storage.blob.backend` | `fs` or `s3`. |
| `storage.blob.path` | Directory for attachments when `fs`. |
| `storage.blob.endpoint`, `bucket`, `region`, `access_key`, `secret_key` | The S3-compatible store when `s3`. |
| `storage.search.backend` | `pg` or `opensearch`. |
| `storage.search.url`, `index`, `username`, `password` | The OpenSearch cluster. |
| `storage.vector.backend` | `pg_array` or `pgvector`. |
| `storage.kv.backend` | `pg` or `valkey`. |
| `storage.kv.url` | `valkey://[:password@]host:port[/db]`; `redis://` is accepted. |

### `[ai.embeddings]`

| Key | Meaning |
| --- | --- |
| `provider` | `none`, `hash` or `openai_compatible`. |
| `base_url`, `model`, `api_key` | The embeddings service, when `openai_compatible`. |
| `dimensions` | Vector length the model produces; 16 to 4096. |
| `min_similarity` | Semantic matches scoring below this are dropped. |

### `[worker]`, `[addons]`, `[integrations]`

| Key | Meaning |
| --- | --- |
| `worker.embedded` | Run background jobs inside the server process. |
| `worker.concurrency` | Jobs run at once per process. |
| `worker.poll_seconds` | How often an idle worker looks for work. |
| `addons.dir` | Directory scanned for add-on packs. |
| `integrations.timeout_seconds` | Timeout for outbound integration calls. |

## Environment variables

Set these in a systemd drop-in (`systemctl edit gantry`) or a container definition. They override the files.

| Variable | Overrides |
| --- | --- |
| `GANTRY_CONFIG` | Path of the configuration file. |
| `GANTRY_DATABASE_URL` | `database.url` |
| `GANTRY_SECRET_KEY` | `security.secret_key` |
| `GANTRY_BIND` | `server.bind` |
| `GANTRY_PUBLIC_URL` | `server.public_url` |
| `GANTRY_WEB_ROOT`, `GANTRY_MANUAL_ROOT` | `server.web_root`, `server.manual_root` |
| `GANTRY_ADDONS_DIR` | `addons.dir` |
| `GANTRY_BLOB_PATH` | `storage.blob.path` |
| `GANTRY_LOG_FORMAT`, `GANTRY_LOG_LEVEL` | `logging.format`, `logging.level` |
