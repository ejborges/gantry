---
title: Configuration
summary: How the configuration file, drop-ins and environment variables fit together, and the settings most installations change.
---

## Where settings come from

In order, later wins:

1. built-in defaults;
2. `/etc/gantry/gantry.toml`;
3. `/etc/gantry/conf.d/*.toml`, in name order;
4. environment variables, for a handful of settings (see [the reference](../reference/configuration.md#environment-variables)).

The file is TOML. An unknown key is an error, not ignored, so a typing mistake cannot silently leave a setting at its default. Check your work before restarting:

```bash
gantry config check
```

This prints the effective configuration with secrets left out, and names the file and key of the first mistake if there is one. Apply changes with `systemctl restart gantry`.

Put local changes in `gantry.toml`, or in your own drop-in such as `conf.d/90-site.toml` if you manage the server with a configuration tool. Leave the `50-*.toml` files to the add-on packages that own them.

## Settings most installations change

### The public address

```toml
[server]
public_url = "https://gantry.example.org"
```

This must be the `https` address people use. It marks cookies as secure-only, is the origin Gantry accepts changes from, and appears in notification links and sign-in redirects. `gantry doctor` reports a missing or non-https value.

### Behind a load balancer

```toml
[server]
bind = "0.0.0.0:8600"
trusted_proxies = ["10.0.0.10", "10.0.0.11"]
```

Only the listed peers are believed when they state the client's address. Get this right: rate limits, lockouts and every log line depend on the client address.

### Sign-in policy

```toml
[auth]
allow_local_login = true       # false once single sign-on covers everyone
password_min_length = 12
session_idle_minutes = 480     # signed out after this long without activity
session_absolute_hours = 168   # and in any case after this long
lockout_threshold = 10         # wrong passwords before an account locks
lockout_minutes = 15
login_attempts_per_ip = 30     # failed sign-ins per address in five minutes
```

### Upload size

```toml
[server]
max_body_mb = 25
```

If you raise it, raise `client_max_body_size` in `/etc/nginx/conf.d/gantry.conf` to one megabyte more.

### Semantic search

```toml
[ai.embeddings]
provider = "openai_compatible"
base_url = "http://inference.internal:8000/v1"
model = "your-embedding-model"
api_key = ""
dimensions = 1024
```

`provider` is `none`, `hash` or `openai_compatible`. *hash* is built in, needs no service and gives modest quality. *openai_compatible* calls any server that implements `POST {base_url}/embeddings`, for example one you run with NVIDIA NIM, vLLM, Text Embeddings Inference or Ollama, so text need not leave your network. After changing the provider, model or dimensions run `gantry reindex`.

### Email

Notifications, reminders and alerts are sent by email once a mail server is configured. All of it is in [Email](email.md).

```toml
[email]
enabled = true
host = "smtp.example.org"
port = 587
encryption = "starttls"
username = "gantry"
password_file = "/etc/gantry/smtp.password"
from = "Gantry <gantry@example.org>"
```

### Live updates

Live updates, presence and editing together use one WebSocket connection per browser tab. They are on by default and need no settings; the web server in front of Gantry must pass the connection through, which the shipped nginx site does (see [Install](install.md#what-the-web-server-must-pass-through)).

```toml
[realtime]
enabled = true
max_connections_per_user = 12
max_messages_per_second = 60
```

| Setting | Meaning |
| --- | --- |
| `enabled` | `false` turns the connection off. The application then works without live updates: people see changes when they reload, and text is edited by one person at a time with a check for conflicting saves. |
| `max_connections_per_user` | Tabs and devices one account may have connected at once. A further tab works without live updates. |
| `max_messages_per_second` | Averaged over ten seconds. A connection that sends more is closed; a browser never comes near it. |

### Monitoring access

```toml
[server]
metrics_token = "a-long-random-string"
```

Without a token, `/metrics` answers only on the loopback address. See [Running and monitoring](operations.md#metrics).

## Secrets

| Secret | Where it lives |
| --- | --- |
| The secret key | `/etc/gantry/secret.key`, mode 0640, owner root, group gantry. Gantry refuses to start if the file is readable by everyone. |
| The mail server's password | In the file named by `email.password_file`, mode 0640, owner root, group gantry. Gantry refuses to start if the file is readable by everyone. |
| Database password, when the database is remote | In `database.url`. Keep `gantry.toml` at mode 0640. It is replaced by `***` whenever configuration is printed. |
| Object store and embeddings keys | In the configuration file or a drop-in, same permissions. |
| Integration and single sign-on secrets | In the database, encrypted with the secret key. Entered in the administration console. |

Changing the secret key makes the stored integration and single sign-on secrets unreadable; they would have to be entered again. Do not rotate it casually; do back it up.

Every key and its default is listed in [the configuration reference](../reference/configuration.md).
