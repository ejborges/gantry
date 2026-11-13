---
title: Scaling out
summary: How far one server goes, what was measured, and how to add nodes, workers and dedicated engines when you need them.
---

## One server

A default installation, with everything on one machine, is meant for teams of 100 to 500 people, and the design leaves room beyond that. What we measured on the release build:

| | |
| --- | --- |
| Machine | 2 virtual CPUs, 7 GB memory, shared by Gantry, PostgreSQL 16 **and the load generator** |
| Data | 2,000 work items in one project |
| Light load, 2 clients | Open an item: median 4 ms, 95% under 7 ms. List 50 items: 11 ms and 17 ms. Keyword search: 12 ms and 20 ms. |
| Saturated, 32 clients without pauses | 650 requests per second on a mix of 80% item reads, 15% lists and 5% searches; 95% under 75 ms; no errors. |
| Writes | 2,000 items created one after another by a single client in 14 seconds. |
| Memory | The server process used under 60 MB. |

Five hundred people working normally produce tens of requests per second, with peaks of perhaps a hundred or two, so one small server has headroom. Treat the numbers as an indication, not a guarantee: they come from one small machine, a small data set and reads through the API; they do not cover browsers, large attachments or millions of items. Measure your own installation with the metrics in [Running and monitoring](operations.md#metrics).

**Starting sizes**

| People | CPU | Memory | Notes |
| --- | --- | --- | --- |
| up to 100 | 2 | 4 GB | |
| 100 to 500 | 4 | 8 GB | Give PostgreSQL `shared_buffers` of about a quarter of memory. |
| beyond | | | Split as below, starting with the database. |

Scale up before you scale out. A larger machine is simpler to run than a cluster.

## What can be separated

Gantry's application process keeps no state of its own: sessions, the job queue, counters and data are in the stores. So any number of application nodes can serve the same installation.

```text
                    ┌──────────────┐
   users, agents ──▶│ load balancer │  TLS, health checks on /readyz
                    └──┬────────┬──┘
              ┌────────▼──┐  ┌──▼────────┐      ┌──────────────┐
              │ app node 1 │  │ app node 2 │ …   │ worker nodes │
              │ gantry     │  │ gantry     │     │ gantry worker│
              └─────┬──────┘  └─────┬──────┘     └──────┬───────┘
                    └────────┬──────┴───────────────────┘
        ┌────────────────────┼─────────────────────┬───────────────┐
   ┌────▼───────┐     ┌──────▼──────┐       ┌──────▼─────┐   ┌─────▼──────┐
   │ PostgreSQL  │     │ object store │       │ OpenSearch  │   │ Valkey     │
   │ (primary +  │     │ (attachments)│       │ (optional)  │   │ (optional) │
   │  standby)   │     └─────────────┘       └────────────┘   └────────────┘
   └────────────┘
```

### Step 1: move the database to its own server

Usually the first and often the only step. Create the database and role there, enable `pg_trgm`, and on the application server:

```toml
[database]
url = "postgres://gantry:PASSWORD@db.internal:5432/gantry?sslmode=verify-full"
max_connections = 20
```

Run `gantry-setup --no-database …`. Use your usual PostgreSQL high-availability arrangement (a streaming standby with managed failover); Gantry needs only the address of the current primary.

### Step 2: more application nodes

Requirements for every node:

- the same Gantry version and the **same secret key** (`/etc/gantry/secret.key`);
- the same `database.url` and `server.public_url`;
- **shared attachments**: an [object store](storage.md#object-storage). Local disk is per node and will not do;
- `server.bind` on an address the load balancer reaches, and the balancer listed in `server.trusted_proxies`.

Install with `./install.sh`, copy `/etc/gantry` from the first node, and run `gantry-setup --no-database --no-nginx` on each. Point the load balancer at port 8600 of each node, check `/readyz`, and have it send `X-Forwarded-For` (or `X-Real-IP`) and the original `Host`. An example configuration is in `deploy/multi-node/` in the source tree.

Keep the total of `database.max_connections` across nodes and workers under PostgreSQL's `max_connections`, or put PgBouncer in session mode between them.

Things to know with several nodes:

- The per-account request rate limit is counted on each node, so the effective limit is the configured value times the number of nodes. Sign-in throttling and lockouts are shared and exact.
- Sticky sessions are not needed, for ordinary requests or for the live connection. A browser's WebSocket may land on any node: every node relays live messages to the others through the database (PostgreSQL `LISTEN`/`NOTIFY`), so people on different nodes see each other's cursors and changes. This needs nothing set up, but each node keeps one extra database connection open for listening, and a connection pooler between Gantry and PostgreSQL must be in **session** mode: `NOTIFY` does not pass through transaction pooling.
- The load balancer must pass WebSocket upgrades for `/api/v1/ws` with long idle timeouts: see [what the web server must pass through](install.md#what-the-web-server-must-pass-through).
- Each node reports once a minute. **Administration**, **Overview**, **Nodes** lists them with role, version and last report; add the alert **Server node stopped reporting** ([Alerts and monitoring](alerts.md)).
- Alerts and reminders are evaluated by one node at a time, chosen by a database lock, over the measurements of all nodes. Nothing is sent twice.
- Upgrade all nodes together: see [Upgrading](upgrade.md#several-application-nodes).

### Step 3: dedicated workers

Background jobs (deliveries, embeddings, imports, the service-level clock) run inside each server by default. To keep that work away from request handling, set on the application nodes

```toml
[worker]
embedded = false
```

and on one or more worker nodes install the `gantry` package, copy `/etc/gantry`, and run `systemctl enable --now gantry-worker`. Workers take jobs from the queue with row locks, so any number can run without doing the same job twice. A worker needs no inbound network access.

### Step 4: dedicated engines

Move search to OpenSearch, vectors to pgvector and counters to Valkey when each becomes the bottleneck, and not before: see [Storage back ends](storage.md).

## Containers and orchestration

The packages are the supported way to run Gantry on Enterprise Linux. Two places use containers, where they earn their keep:

- **Engines that Enterprise Linux does not ship** (OpenSearch, the object store) run under Podman as systemd services, installed by add-on packages.
- **The application itself** can run as a container when you already operate an orchestrator. `deploy/container/Containerfile` builds an image from the packages on a UBI base, and `deploy/kubernetes/` holds example manifests: a Deployment for the application, one for workers, a Service, an Ingress and the configuration. The application is stateless, so it scales with replicas; run PostgreSQL and the object store as you run your other stateful services.

In a container, configure through environment variables (`GANTRY_DATABASE_URL`, `GANTRY_SECRET_KEY`, `GANTRY_PUBLIC_URL`, `GANTRY_BIND=0.0.0.0:8600`) and a mounted configuration file for the rest; see [the configuration reference](../reference/configuration.md#environment-variables).

> **Note**: the container and Kubernetes files are examples to adapt. They were written with this release but, unlike the packages' contents, were not run as part of its test suite.

## High availability checklist

- [ ] PostgreSQL has a standby and a tested failover, and `database.url` follows the primary (a virtual address or a connection proxy).
- [ ] At least two application nodes behind a balancer that checks `/readyz`.
- [ ] Attachments in an object store with its own redundancy and backup.
- [ ] The secret key and configuration are in your configuration management, and in the backups.
- [ ] Workers on at least two nodes, or left embedded.
- [ ] Monitoring and alerts as in [Running and monitoring](operations.md#metrics).
