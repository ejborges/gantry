---
title: Storage back ends
summary: Moving search, vectors, counters and attachments from PostgreSQL to dedicated engines, with packages or with engines you already run.
---

Out of the box everything is in PostgreSQL and a directory. That is the right choice until a specific kind of data outgrows it. Each kind can then be moved on its own, by installing an add-on package (which runs the engine on this machine) or by pointing the configuration at an engine you already operate.

| Move | When it is worth it |
| --- | --- |
| Vectors to **pgvector** | Semantic search slows down: several hundred thousand indexed passages. |
| Counters to **Valkey** | Several application nodes under heavy API traffic; you want that write load off the database. |
| Search to **OpenSearch** | Millions of items and pages, or you need its language analysis. |
| Attachments to an **object store** | More than one application node (they must share attachments), or very large files. |

The **Overview** in the administration console and `gantry doctor` show which engine provides each component.

## pgvector

```bash
dnf install -y ./addons/gantry-addon-pgvector-*.rpm
```

The package pulls in the `pgvector` extension from the distribution, enables it in the database, and adds `/etc/gantry/conf.d/50-pgvector.toml`:

```toml
[storage.vector]
backend = "pgvector"
```

Gantry restarts and builds an HNSW index over the existing vectors; nothing needs re-embedding. Removing the package returns to exact search.

With a remote database, install the extension on the database server instead, run `CREATE EXTENSION vector` in the Gantry database as a superuser, and set the two lines above yourself.

## Valkey

```bash
dnf install -y ./addons/gantry-addon-valkey-*.rpm
```

Installs Valkey from the distribution (from EPEL on Enterprise Linux 9), starts it with Gantry, and adds:

```toml
[storage.kv]
backend = "valkey"
url = "valkey://127.0.0.1:6379"
```

Only short-lived data goes there: sign-in throttling counters and similar. Sessions stay in PostgreSQL, so losing Valkey signs nobody out. To share one Valkey between nodes, set `url` to it on every node, with a password: `valkey://:PASSWORD@valkey.internal:6379`. `redis://` addresses and Redis servers work too.

## OpenSearch

```bash
dnf install -y ./addons/gantry-addon-opensearch-*.rpm
systemctl is-active gantry-opensearch      # wait for "active"; the first start downloads the image
systemctl restart gantry
gantry reindex
```

OpenSearch is not part of Enterprise Linux, so the package runs it as a container under **Podman**, managed by systemd through a Quadlet unit (`/usr/share/containers/systemd/gantry-opensearch.container`). It listens on loopback only and keeps its data in the Podman volume `gantry-opensearch-data`.

```bash
systemctl status gantry-opensearch
journalctl -u gantry-opensearch
podman volume inspect gantry-opensearch-data
```

The unit follows the image tag for the current major version. To pin an exact version or use an internal registry, copy the file to `/etc/containers/systemd/` and edit `Image=` there; that copy takes precedence.

To use a cluster you already run:

```toml
[storage.search]
backend = "opensearch"
url = "https://search.internal:9200"
index = "gantry"
username = "gantry"
password = "…"
```

and disable the local container: `systemctl disable --now gantry-opensearch`. A cluster reached over the network must use TLS and its own accounts; the local container has its security plugin off only because nothing but this machine can reach it.

PostgreSQL remains the source of truth. The search index can always be rebuilt with `gantry reindex`, so OpenSearch needs no backup of its own.

## Object storage

```bash
dnf install -y ./addons/gantry-addon-objectstore-*.rpm
gantry-objectstore-setup
```

The package runs **SeaweedFS**, an Apache-licensed S3-compatible store, as a Podman container (`gantry-objectstore.service`). The setup command creates credentials, starts the store, creates the bucket and writes `/etc/gantry/conf.d/50-objectstore.toml`.

To use a store you already run (Ceph RGW, Garage, SeaweedFS, a cloud provider):

```toml
[storage.blob]
backend = "s3"
endpoint = "https://objects.internal"
bucket = "gantry"
region = "us-east-1"
access_key = "…"
secret_key = "…"
```

Requests use path-style addressing and AWS Signature Version 4.

**Moving existing attachments.** Switching the back end does not move files. Before switching, copy the content of `/var/lib/gantry/blobs` into the bucket keeping the same relative names, for example with `aws s3 sync /var/lib/gantry/blobs s3://gantry/ --endpoint-url …` or `rclone`. Then switch and restart.

Unlike the search index, attachments in an object store are primary data: back the bucket up.

## Embeddings service

Semantic search needs vectors, which come from the provider set under `[ai.embeddings]`: see [Configuration](configuration.md#semantic-search). Text is sent only to the address you configure there, so a model served inside your own network keeps content on site.

## Going back

Each move is reversed by removing the package (or the settings) and restarting. Search then needs `gantry reindex`; attachments need copying back. Counters and vectors need nothing.
