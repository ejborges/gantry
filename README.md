# Gantry

Self-hosted, open-source project management, issue tracking, service desk, knowledge base
and wiki for teams of 100 to 500 people **and the AI agents that work with them**.

- **Projects**: boards, lists, configurable work item types and workflows with a visual workflow editor, cycles, links between items, a complete history of every change, and Gantt charts you edit in place.
- **Planning**: a hierarchy you define (programs, initiatives, projects, or your own levels), a live graph of how all work hangs together, and reports for the owners of each level.
- **Service desk**: customer portal, request types, queues, response and resolution targets, internal notes.
- **Docs**: spaces of Markdown pages with a simple (word-processor style) and an advanced (source with live preview) editor, a diagram editor, templates, history, and a color-coded comparison before publishing.
- **Working together**: a discussion on every work item and request, mentions, links by key, attachments, and several people editing the same text, fields, board or dashboard at once, each shown where they are.
- **Analytics**: dashboards from built-in visualizations or your own, written with Chart.js, Plotly, ECharts or D3 and run in a sealed frame that can only read what the viewer may read.
- **Finding things**: full-text search, and a search by any combination of fields, including your own, with results beside the item you open.
- **Notifications**: in the application and by email, chosen by each person within what administrators allow; alerts for application and system administrators on conditions they define, also to Slack and Teams.
- **People and teams**: profiles with avatars or generated identicons, status, teams made of positions with their history, teams of teams, an org chart.
- **For every discipline**: templates for software, engineering change, research, management, finance, legal, logistics, procurement, safety and service desks; everything is data you can change.
- **For AI agents**: an MCP endpoint and a REST API. Agents are accounts of their own, always marked as such wherever they appear, and can do nothing until an administrator writes their rules.
- **Integrations**: Slack, Microsoft Teams, GitHub, GitLab, Jira import, signed webhooks, single sign-on with OpenID Connect and SAML 2.0, SCIM provisioning.
- **Administration console**: users, agents and their rules, groups, roles, sign-in, types and workflows, notifications and email, alerts, add-ons, integrations, audit log, security events, system health.
- **Optional add-on**: NASA project management and systems engineering (NPR 7120.5, NPR 7123.1).
- **Runs on** Red Hat Enterprise Linux 9 and 10 and equivalents (Rocky, Alma, Oracle Linux), installed and upgraded with RPM, from one server to many.
- **In the browser**: Chromium-based browsers and Safari, on desktop, tablet and phone, in light, dark or the device's theme, following the WCAG 2.1 AA guidelines.

The server is one Rust program; the web application is React and TypeScript; data lives in
PostgreSQL, with optional dedicated engines for search, vectors, files and counters.
Licensed under Apache-2.0.

> **Status: 0.2.0.** Every module works end to end on an architecture meant to grow. See
> [What has and has not been verified](#what-has-and-has-not-been-verified) before relying on
> it, and the manual's *Upgrading* page, *Notes for each release*, when coming from 0.1.0.

## Contents

- [Repository layout](#repository-layout)
- [Development environment](#development-environment)
- [Build and run](#build-and-run)
- [Tests](#tests)
- [Building the RPMs](#building-the-rpms)
- [Installing](#installing)
- [Customizing and extending](#customizing-and-extending)
- [Add-ons](#add-ons)
- [Maintenance](#maintenance)
- [Security](#security)
- [Documentation](#documentation)
- [What has and has not been verified](#what-has-and-has-not-been-verified)

## Repository layout

```
crates/gantry-server/     the server and command-line tool (binary: gantry)
  src/                    one module per area: auth, authz, items, desk, wiki, search, mcp, …
  migrations/             database schema
  packs/core.json         built-in fields, workflows, types and templates
  tests/                  integration, security, external-system and regression suites
crates/gantry-mcp/        stdio bridge for MCP clients that start a local program
web/                      the web application (React, TypeScript, Vite)
  src/                    pages, components, editors, API and live-update clients, styles
  e2e/                    browser tests (Playwright)
  scripts/                the manual's site builder, screenshots, icons, license list
addons/nasa/              the NASA add-on: pack.json and document templates
docs/manual/              the web manual: Markdown sources, assets, generated reference data
docs/                     architecture notes and the research record
packaging/                gantry.spec, service units, setup and backup scripts, add-on files,
                          and tests of the scripts that run at install and upgrade
deploy/                   multi-node, container and Kubernetes examples
dev/                      scripts for local development and the Vagrant machine
Vagrantfile               Enterprise Linux development machine on VMware
```

## Development environment

### With Vagrant on VMware (recommended)

This gives you the same operating system Gantry is deployed on, with every build, test and
packaging tool installed.

Requirements: VMware Workstation or Fusion, [Vagrant](https://developer.hashicorp.com/vagrant),
the Vagrant VMware Utility and the plugin:

```bash
vagrant plugin install vagrant-vmware-desktop
vagrant up --provider vmware_desktop        # AlmaLinux 9 by default; first run takes a while
vagrant ssh
cd /vagrant
```

`dev/provision.sh` runs on first boot and installs: GCC, Rust (rustup), Node.js 22,
PostgreSQL 16 with pgvector, nginx, Valkey, Podman, the RPM build tools and the libraries the
test browser needs. Ports 8600 (Gantry), 5173 (Vite) and 8443 (nginx, for package tests) are
forwarded to your computer.

| Setting | Default | Change with |
| --- | --- | --- |
| Box | `almalinux/9` | `GANTRY_BOX=almalinux/10 vagrant up …` (any EL 9 or 10 box with a `vmware_desktop` build) |
| CPUs, memory | 4, 8 GB | `GANTRY_VM_CPUS`, `GANTRY_VM_MEMORY_MB` |
| Rust toolchain | current stable | `GANTRY_RUST_VERSION=1.90.0` |

The source tree is copied into the guest with rsync. After editing on the host, run
`vagrant rsync` (or keep `vagrant rsync-auto` running). Alternatively work inside the guest.

### Directly on a Linux machine

You need Rust 1.82 or newer, Node.js 20 or newer, and PostgreSQL 15 or newer server binaries
(16 recommended). On Enterprise Linux run `sudo dev/provision.sh $USER`; on other
distributions install the equivalents.

## Build and run

```bash
dev/test-db.sh start      # a throwaway PostgreSQL on 127.0.0.1:5433 (no system service needed)
dev/run-local.sh          # build everything, load sample data, serve on http://localhost:8600
```

`run-local.sh` builds the server and the web application, creates a fresh database, an
administrator and sample projects, installs the NASA add-on and starts the server.

| Sign in as | Password | Role |
| --- | --- | --- |
| `admin@example.org` | `correct horse battery staple` | System administrator |
| `ada@example.org`, `ben@example.org`, `chen@example.org` | same | Members |
| `pat@example.org` | same | Customer (help desk only) |

These accounts exist only in the development database.

Piece by piece:

```bash
cargo build                                   # server and bridge (debug)
cargo run -p gantry-server -- --config dev/gantry.dev.toml serve
cd web && npm ci && npm run build             # web application into web/dist
cd web && npm run dev                         # or: Vite dev server on :5173 with hot reload,
                                              #     proxying the API to :8600
cd web && npm run manual                      # the manual into docs/manual/_site
```

Configuration for development is `dev/gantry.dev.toml`. The command-line tool is the same
binary: `cargo run -p gantry-server -- --config dev/gantry.dev.toml doctor`.

## Tests

Every component has unit, integration, regression and security tests.

| Suite | Where | What it covers | Run |
| --- | --- | --- | --- |
| Server unit | `crates/gantry-server/src/**` (`#[cfg(test)]`) | Workflow rules, permission logic, cryptography, signatures, SAML canonicalization, calendars, parsers, mail composition, configuration | `cargo test -p gantry-server --lib` |
| Server integration | `tests/integration.rs` | Each module through the real HTTP API against PostgreSQL | `cargo test -p gantry-server --test integration` |
| People, agents, notifications | `tests/people.rs` | Profiles, avatars, status, teams and positions, the rules that limit AI agents (REST and MCP), notification choices, email, reminders, alerts | `cargo test -p gantry-server --test people` |
| Planning and analytics | `tests/planning.rs` | Hierarchy levels, owners' access, reports, search by criteria, saved searches, Gantt charts, the graph, dashboards, visualizations, the sealed frame, the about and error pages | `cargo test -p gantry-server --test planning` |
| Working together | `tests/collab.rs` | Discussions, mentions and links, history, project workflows, page stages, the live connection: who may join what, hints without content, several people editing one text, several server nodes | `cargo test -p gantry-server --test collab` |
| Server security | `tests/security.rs` | Authentication on every route, permission boundaries, information hiding, CSRF, lockout, throttling, upload safety, audit integrity | `cargo test -p gantry-server --test security` |
| Hostile input | `tests/hostile.rs` | SQL and other injection through every text and query field, access by guessed identifiers, header and mail injection, path traversal, XML attacks, oversized and malformed requests, mass assignment, cross-site requests, floods on the live connection | `cargo test -p gantry-server --test hostile` |
| SAML | `tests/saml.rs` | Signatures made by another implementation, signature wrapping, tampering, replay, conditions, certificate handling, account matching | `cargo test -p gantry-server --test saml` |
| External systems | `tests/external.rs` | Slack, Teams, webhooks, GitHub, GitLab, Jira Cloud and Data Center, OIDC, SCIM and MCP against local stand-ins | `cargo test -p gantry-server --test external` |
| Server regression | `tests/regression.rs` | Defects that were found and fixed, and the optional engines (S3, OpenSearch, Valkey, pgvector) | `cargo test -p gantry-server --test regression` |
| Bridge | `crates/gantry-mcp` | The stdio bridge | `cargo test -p gantry-mcp` |
| Web unit | `web/src/**/*.test.ts(x)` | Markdown rendering and sanitizing (GitHub, GitLab and Obsidian forms), editor round trip, search criteria, diagrams, workflows, date and time formatting, API client, components | `cd web && npm test` |
| Browser | `web/e2e/*.spec.ts` | Sign-in, projects, items, the editors, discussions and editing at the same time, service desk, planning, analytics, people and teams, administration, stored-XSS and CSRF defenses, accessibility (axe-core, WCAG 2.1 A and AA, light and dark), layout at desktop, tablet and phone sizes | `dev/e2e.sh` |
| Packaging | `packaging/tests/*.sh` | The script that writes and upgrades the nginx site, against a real nginx. The upgrade itself, on a real database: an installation created and filled by the 0.1.0 server from its RPM, then migrated, started and checked with this version | `packaging/tests/nginx-site.sh`; `packaging/tests/upgrade.sh PATH/TO/gantry-0.1.0-1.*.rpm` |

Everything at once:

```bash
dev/test-db.sh start
cargo test --workspace
(cd web && npm test)
dev/e2e.sh
packaging/tests/nginx-site.sh     # says SKIP where nginx is not installed
packaging/tests/upgrade.sh ../gantry-0.1.0/core/gantry-0.1.0-1.el9.x86_64.rpm   # with the previous release at hand
```

Server tests create a private database per test on the PostgreSQL named by
`GANTRY_TEST_DATABASE_URL` (default `postgres://postgres@127.0.0.1:5433/postgres`, which is
what `dev/test-db.sh` starts) and start a real server on a loopback port. Tests of optional
engines use what is available: pgvector if the extension is installed, Valkey or Redis if
a `valkey-server` or `redis-server` binary is on the machine (the test starts its own), and
local stand-ins for S3 and OpenSearch. A test that cannot run says `SKIPPED` and why.

Browser tests run in Chromium at three screen sizes. To add Safari's engine:

```bash
cd web && npx playwright install webkit       # supported on macOS, Windows, Debian and Ubuntu
GANTRY_E2E_WEBKIT=1 dev/e2e.sh
# or, from a Mac, against a server running elsewhere:
GANTRY_E2E_WEBKIT=1 GANTRY_E2E_URL=http://localhost:8600 npx playwright test
```

## Building the RPMs

```bash
packaging/build-rpms.sh               # clean build inside rpmbuild
packaging/build-rpms.sh --prebuilt    # build here first, then only package (faster)
```

The result is `dist/gantry-VERSION/`:

```
install.sh        installer for a freshly installed machine
core/             gantry  gantry-web  gantry-manual  gantry-mcp
addons/           gantry-addon-nasa  gantry-addon-pgvector  gantry-addon-valkey
                  gantry-addon-opensearch  gantry-addon-objectstore
SHA256SUMS
```

Build on Enterprise Linux 9 (the Vagrant machine): binaries built there run on 9 and 10.
One spec, `packaging/gantry.spec`, produces all packages; its header explains the options,
including an offline build from vendored dependencies.

## Installing

On a freshly installed Enterprise Linux 9 or 10 machine, as root:

```bash
./install.sh                                   # core packages and all dependencies
gantry-setup --hostname gantry.example.org --admin-email you@example.org
```

`install.sh --with nasa,pgvector` adds add-ons. `gantry-setup` prepares PostgreSQL, creates
the secret key and the first administrator, configures nginx with TLS, SELinux and the
firewall, and starts the service. Upgrades are the same `./install.sh` with the new packages:
the service migrates the database when it starts, and the package brings the nginx site it
manages up to date (live updates need a WebSocket location that 0.1.0 did not have).
Email is optional and set up afterward in `/etc/gantry/gantry.toml` (`[email]`).
The manual's *System administration* guide covers every step and option.

## Customizing and extending

| You want to | How | Where to read |
| --- | --- | --- |
| Change work item types, fields, workflows | Administration console, **Types and workflows**. Changes survive upgrades. | Manual: *Application administration* |
| Package your definitions for other installations | A **pack**: one JSON file. | Manual: *Reference, Packs*; `crates/gantry-server/packs/core.json` |
| Change settings | `/etc/gantry/gantry.toml` and drop-ins in `/etc/gantry/conf.d/`. | Manual: *Reference, Configuration* |
| Connect another system | Signed webhooks out; the REST API in; the MCP endpoint for agents. | Manual: *Reference* |
| Decide what an AI agent may do | Administration console, **Users and agents**, the agent's **Rules**. A new agent may do nothing. | Manual: *Application administration, AI agents* |
| Define your own hierarchy above projects | **Planning**, **Levels**. | Manual: *Using Gantry, Planning* |
| Build a dashboard or your own visualization | **Analytics**; scripts use Chart.js, Plotly, ECharts or D3 and read data through `gantry.api.get`. | Manual: *Using Gantry, Analytics*; *Reference, Visualizations* |
| Watch a condition and be told | Administration console, **Alerts**; rules for the application and for the system. | Manual: *Application administration, Notifications*; *System administration, Alerts* |
| Use a different engine for search, files, vectors or counters | Configuration, or implement the small trait in `src/storage.rs`. | `docs/architecture.md` |
| Add a kind of integration | One catalog entry and one function in `src/integrations.rs`. | `docs/architecture.md` |
| Change the look | `web/src/styles.css` is built on a small set of color, space and type tokens at the top of the file. | |

## Add-ons

Optional packages, installed with `dnf` like the core:

| Package | Adds |
| --- | --- |
| `gantry-addon-nasa` | NASA life-cycle phases, reviews, requirements, verification, RIDs, key decision points, waivers and plan templates. Content only. |
| `gantry-addon-pgvector` | An HNSW index for semantic search (pgvector). |
| `gantry-addon-valkey` | Valkey for counters and short-lived state. |
| `gantry-addon-opensearch` | OpenSearch for full-text search, as a Podman container under systemd. |
| `gantry-addon-objectstore` | An S3-compatible object store (SeaweedFS) for attachments, as a Podman container under systemd. |

## Maintenance

| Task | Command or place |
| --- | --- |
| Health | `gantry doctor`; `/readyz`; Administration, Overview |
| Logs | `journalctl -u gantry` (JSON lines; targets `http`, `audit`, `security`, `jobs`, `events`, `integrations`, `realtime`, `email`, `alerts`) |
| Alerts | Administration console, **Alerts**: error rate, slow responses, failing jobs, security events, undelivered email and more, to people, Slack or Teams |
| Metrics | `/metrics` (Prometheus) |
| Backup | `systemctl enable --now gantry-backup.timer`; `gantry-backup` |
| Upgrade | `./install.sh` from the new release directory; the service migrates the database on start |
| Verify the audit log | `gantry audit verify` |
| Rebuild search | `gantry reindex` |
| Recover administrator access | `gantry admin create …`, `gantry admin reset-password …` |

Details, schedules and troubleshooting are in the manual's *System administration* guide.

## Security

Users are always authenticated, act only within their configured privileges, are shown only
what they may see, and everything they do is logged.

- Deny by default; permissions are enforced on the server for every request; things a caller may not see answer *not found*. Counts, links, mentions, search results, notifications and live updates follow the same rule: nothing reaches an account that it could not have fetched itself.
- Sessions: `HttpOnly`, `SameSite`, `Secure` cookies; CSRF token and origin check on every change. API tokens: hashed, scoped, expiring, revocable.
- Passwords: Argon2id, lockout, per-address throttling of failed sign-ins. Single sign-on: OpenID Connect code flow with PKCE, state and nonce, asymmetric signatures only; SAML 2.0 with signed responses or assertions required, SHA-1 refused, and each response accepted once, only in answer to a request this server made.
- AI agents: separate accounts, marked in every place they appear, limited by rules an administrator writes (which permissions, which projects and spaces, how many requests a minute) on top of their roles. A new agent is allowed nothing.
- The live connection: authenticated like the API, opened only from the application's own origin, each subscription authorized like a read and checked again every minute; it carries hints, not content.
- Custom HTML and scripts in pages, messages and dashboards run in a sealed frame with no network access, storage or access to the page around it; they read data only through the viewer's own permissions and can change nothing.
- Stored secrets encrypted with AES-256-GCM. Outbound calls guarded against server-side request forgery. Inbound webhooks verified by signature.
- User content sanitized before display; strict content security policy; attachments are served as downloads, except images, audio, video and PDF of known types, which are shown or played in place under a policy that lets them run nothing.
- Append-only, hash-chained audit log; separate security event log; structured request logs with a request ID on every line; every component logs what it does, never secrets or message content.
- The service runs unprivileged inside a systemd sandbox and listens on loopback behind nginx.

Report vulnerabilities privately to the maintainers, not in a public tracker.

## Documentation

- **Web manual**: served by Gantry at `/manual/`, installed under `/usr/share/gantry/manual`, built from `docs/manual/src` with `cd web && npm run manual`. Guides for users, application administrators and system administrators, a reference (API, MCP, permissions, Markdown, visualizations, configuration, command line, events), and the NASA add-on guide. It follows the reader's light or dark setting and can be switched. Reference tables are generated from the server (`npm run manual:data`), and the build fails on a broken link.
- The manual's screenshots are taken from the sample data: with `dev/run-local.sh` running, `node web/scripts/manual-shots.cjs docs/manual/src/img`.
- **In the application**: **About** shows the version and build date of each component and add-on, the privacy, terms, cookies, data retention and data security statements (which an administrator can replace with the organization's own), and the license of every library the product is built from. Each error has a page that says what happened and whom to ask.
- `docs/architecture.md`: how the code is organized, for contributors.
- `docs/research.md`: the external facts this design rests on, with sources.

## What has and has not been verified

Verified in the environment where this release was produced (Ubuntu 24.04 container, Rust 1.97, Node.js 22, PostgreSQL 16 with pgvector, nginx 1.24):

- The server builds in debug and release; all server test suites pass: 152 unit tests, 131 tests through the real HTTP API and WebSocket against PostgreSQL in nine suites, and 2 for the bridge (see [Tests](#tests) for what they cover).
- The web application type-checks and builds; its 128 unit tests pass; the browser tests pass in **Chromium** at desktop, tablet and phone sizes (70 tests at desktop size and the 7 layout and theme tests again at tablet and at phone size: of those 84 runs, 81 passed and 3 were skipped by design, because they apply only to phones or only to wider screens), including the accessibility checks in light and dark themes and the keyboard-only tests.
- The manual builds with no broken links or anchors; its screenshots were retaken from this version.
- The script that writes the nginx site and brings an older site up to date on upgrade was run against a real nginx, starting from the site that 0.1.0 wrote.
- The RPM spec builds all nine packages; their file lists, dependencies, changelog and scriptlets were inspected, and the build itself runs the nginx site test. The packages were unpacked into a scratch directory and run from there: the packaged binary with the packaged configuration migrated a fresh database, loaded the NASA add-on, passed `gantry doctor`, served the web application, the manual, the chart libraries and the license list, and reported version 0.2.0 and a build date for every component. The same packaged binary passed the upgrade rehearsal below. The systemd units are unchanged since 0.1.0 (`systemd-analyze security` rates the service's sandbox 1.3, "OK"); the shell scripts pass `shellcheck` at warning level. `rpmlint` could not be run there.
- The upgrade from 0.1.0 was rehearsed on a real database with `packaging/tests/upgrade.sh`: an installation created and filled by the 0.1.0 server (from its package) was migrated and started by this version, and its work, discussions, pages, accounts, agent token, integration and audit chain came through as the upgrade notes describe, with a quiet log.

Written with care but **not executed** there, because the environment could not run them. Test these first in your own:

- **Installing and upgrading the RPMs on Enterprise Linux**, and therefore `install.sh`, `gantry-setup` as a whole, the package scriptlets (among them the one that refreshes the nginx site), the service actually running inside its systemd sandbox, the SELinux and firewalld steps, and the backup script and timer. Packages built in that environment are made on a host without the Enterprise Linux RPM macros, so they are for inspection only: **build the real ones in the Vagrant machine.**
- **Vagrant and VMware**: the `Vagrantfile` (syntax-checked) and `dev/provision.sh`.
- **Container pieces**: the Podman Quadlet units for OpenSearch and the object store, `gantry-objectstore-setup`, `deploy/container/Containerfile` and `deploy/kubernetes/`.
- **Safari**: the application avoids anything Safari 16.4 lacks and the build targets it, but no test ran in WebKit. `GANTRY_E2E_WEBKIT=1` runs the browser suite there.
- **Real third-party services**: Slack, Teams, GitHub, GitLab, Jira, OpenID Connect and SAML identity providers, mail servers, OpenSearch and S3 were exercised against local stand-ins that follow their documented protocols, not against the services themselves. SAML signatures were checked against responses signed by an independent implementation (xmlsec), not by a live identity provider. pgvector was tested for real; the Valkey back end was tested against a live Redis server, which speaks the same protocol, not against Valkey itself.
- **Assistive technology**: accessibility was checked with axe-core and with keyboard-only browser tests. Nobody has yet used this version with a screen reader from start to finish.
- **Load with many people editing at once**: the live connection was tested for correctness, permissions and abuse, and across two server nodes, not for hundreds of simultaneous editors.
- **PostgreSQL 15 and older**: only 16 was tested; the packages require 16.

Known limits of this release:

- Email goes out only. Replying to a notification by email does not add a message, and requests cannot be raised by email.
- The diagram editor is Gantry's own. It does not read or write draw.io files; diagrams written as Mermaid text are also drawn.
- Custom visualizations and embedded scripts can read data and cannot change it.
- Editing at the same time merges text (pages, descriptions, visualization scripts). Single fields, cards, tiles and bars are last change wins, with every change in the history.
- SAML: sign-in started from Gantry, with signed responses or assertions. Encrypted assertions, signed requests and single logout are not supported.
- Archived work items cannot be restored from the user interface.
- Jira is import only (re-runnable); there is no two-way synchronization.
- The request rate limit is counted per application node.
- The NASA add-on provides structure and templates; it does not establish compliance, and it is not a NASA product.
