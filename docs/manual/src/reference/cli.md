---
title: Command-line tool
summary: The gantry command: every subcommand and when to use it.
---

`gantry` is both the server and its administration tool. It reads `/etc/gantry/gantry.toml` (or the file named by `--config` or `GANTRY_CONFIG`). Run it from a root shell: started by root it switches to the unprivileged `gantry` account before doing anything.

Every command that changes something is recorded in the audit log with the method `cli`.

| Command | What it does |
| --- | --- |
| `gantry serve` | Run the server. Used by `gantry.service`. |
| `gantry worker` | Run a background worker. Used by `gantry-worker.service`. `--once` processes the queue and exits. |
| `gantry migrate` | Create or upgrade the database schema, built-in definitions and add-on packs found on disk. Runs before every service start; safe to run at any time. |
| `gantry doctor` | Check configuration, key, database, schema, storage, search, cache, administrators and the audit chain. Exits non-zero if any check fails. |
| `gantry reindex` | Rebuild the search index and queue embeddings for every item and page. |
| `gantry admin create --email E --name N [--generate]` | Create a system administrator. The password is read from standard input, or generated and printed once with `--generate`. |
| `gantry admin reset-password --email E` | Set a new password, read from standard input. Unlocks the account and ends its sessions. |
| `gantry config check` | Validate the configuration and print the effective settings without secrets. |
| `gantry config generate-key` | Print a new random secret key. |
| `gantry config reference` | Print the built-in catalog as JSON: API routes, permissions, roles, events, MCP tools, defaults. This manual's reference pages are generated from it. |
| `gantry audit verify` | Recompute the audit log's hash chain. Exits non-zero and names the first altered entry if it is broken. |
| `gantry pack list` | List add-on packs on disk and whether each is installed. |
| `gantry pack install ID` | Install or update one add-on pack. |

## Scripts installed with the server

| Script | What it does |
| --- | --- |
| `gantry-setup` | First-time setup of one server; see [Installing](../sysadmin/install.md#set-up). Safe to run again. `--help` lists its options. |
| `gantry-backup [DIR]` | Back up the database, attachments and configuration; see [Backup and restore](../sysadmin/backup.md). |
| `gantry-objectstore-setup` | With the object store add-on: create credentials and the bucket. |

## `gantry-mcp`

A separate small program for users' machines; see [MCP endpoint](mcp.md#connecting-a-client).

| Variable | Meaning |
| --- | --- |
| `GANTRY_URL` | Address of the Gantry server. |
| `GANTRY_TOKEN` | API token to act with. |

## Examples

Create a break-glass administrator and store the password in a sealed envelope:

```bash
gantry admin create --email breakglass@example.org --name "Break glass" --generate
```

Set a password without it appearing in the shell history:

```bash
read -rs PW && printf '%s' "$PW" | gantry admin reset-password --email you@example.org; unset PW
```

Health check for a monitoring system:

```bash
gantry doctor >/dev/null || echo "gantry needs attention"
```
