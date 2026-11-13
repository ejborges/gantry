# Architecture notes for contributors

The manual's *System administration, Architecture* page describes Gantry from the outside.
This page describes the code.

## One request, end to end

1. `http::context_mw` (middleware) assigns a request ID, works out the client address (only
   trusting configured proxies), applies the rate limit, authenticates the caller from the
   session cookie or bearer token (`auth::authenticate`), enforces the CSRF token and origin
   on changes made with a session, and after the handler adds security headers and writes
   the request log line.
2. The handler receives a `Ctx` (who is calling, how they authenticated, their workspace
   permissions). It loads the project or space and asks `authz` for the caller's permissions
   there; `authz::require` refuses with 403, `authz::hide` with 404 when the object must not
   be revealed. List endpoints filter by `authz::visible_project_ids` / `visible_space_ids`.
3. Changes go through one function per operation (`items::create_item`, `transition_item`,
   `update_item`, `wiki::…`), shared by the REST API, the portal, MCP tools, integrations
   and imports, so that rules, history, audit and events cannot be bypassed by a new entry point.
4. The function writes history, calls `audit::log`, and `events::emit`. An event becomes
   notifications (`notify::deliver`: in the application, by email, each by the recipient's
   own choice), deliveries to integrations queued as jobs, and a hint on the live channel
   (`realtime::publish_work`).
5. `jobs` workers pick jobs up with `FOR UPDATE SKIP LOCKED` and run them with retries. The
   same workers run the minute-by-minute housekeeping: reminders, alert rules, saved-search
   alerts and the removal of expired rows.

An AI agent's request passes one more gate. `authz` resolves permissions from roles as for
anyone, then intersects them with the account's rules (`authz::AgentPolicy`: allowed
permissions, projects and spaces, and a request rate). The intersection happens where
permissions are resolved, so every entry point (REST, MCP, the live channel) gets it without
asking.

Every route is registered through `http::Api::route`, which records its method, path and
summary. That one table produces the OpenAPI document, the manual's API reference and the
security test that proves every non-public route refuses anonymous callers.

## Modules

| Module | Responsibility |
| --- | --- |
| `config` | Configuration file, drop-ins, environment overrides, validation. |
| `http` | Router, middleware, health, metrics, static files, OpenAPI. |
| `auth` | Sessions, password sign-in, API tokens, the `Ctx` extractor. |
| `oidc`, `saml`, `scim` | Single sign-on (OpenID Connect; SAML 2.0 with its own XML signature verification) and provisioning. |
| `authz` | Permission catalog, built-in roles, permission resolution, visibility filters, the rules that limit AI agents. |
| `audit` | Hash-chained audit log, security events. |
| `packs` | Pack format, validation and installation; workflow rules (`WorkflowDef`). |
| `projects`, `items`, `desk`, `wiki` | The product areas. |
| `planning` | Hierarchy levels and containers (programs, initiatives), the graph, reports, criteria search, saved searches, Gantt charts. |
| `analytics` | Aggregated figures, dashboards, saved visualizations, the frame that custom scripts run in. |
| `teams`, `accounts` | Teams, positions and their history, the org chart; profiles, avatars, identicons, status. |
| `refs` | Mentions and links in text: what a piece of text refers to, checked against what its author and each reader may see. |
| `realtime` | The WebSocket endpoint: rooms, presence, change hints, documents edited together. |
| `notify` | Notification choices, email, reminders, alert rules. |
| `settings` | Workspace settings with defaults and validation. |
| `pages`, `about` | Readable pages for failed requests; version, components, licenses and the statements shown to users. |
| `search` | Hybrid search over the search and vector ports; indexing. |
| `events`, `jobs`, `integrations` | Event fan-out, the job queue, outbound and inbound integrations, Jira import. |
| `mcp` | The Model Context Protocol endpoint. |
| `admin` | Administration API. |
| `storage` | The storage ports and their implementations. |
| `reference` | The built-in catalog as data, for generated documentation. |
| `crypto` | Password hashing, secret encryption, tokens, HMAC helpers. |

## Storage ports

`storage.rs` defines one trait per kind of data, each with a PostgreSQL implementation and,
where useful, a dedicated engine:

| Trait | Implementations |
| --- | --- |
| `BlobStore` | `FsBlob`, `S3Blob` (SigV4, path style) |
| `SearchIndex` | `PgSearch` (tsvector and trigram), `OpenSearch` |
| `VectorStore` | `PgArrayVector` (exact), `PgVector` (HNSW) |
| `Kv` | `PgKv`, `ValkeyKv` (RESP) |
| `GraphStore` | `PgGraph` (recursive CTE) |
| `Embedder` | `HashEmbedder`, `OpenAiCompatEmbedder` |

To add an engine: implement the trait, add a `backend` value to `config::Config::validate`
and to `Stores::connect`, and add a regression test beside the existing ones (they show how to
stand in for a remote service). PostgreSQL stays the source of truth for relational data;
search and vector stores are derived and can be rebuilt with `gantry reindex`.

## The live channel

`realtime.rs` serves one WebSocket per browser tab at `/api/v1/ws`. A connection subscribes
to rooms (`item:KEY`, `project:KEY`, `page:ID`, …); `realtime::authorize` answers each
subscription exactly as the REST API would answer a read of the same object, and the answer
is checked again every minute. Three kinds of traffic use it:

- **Hints.** `publish_work` and friends say *what changed*, never the content. The page
  refetches through the REST API, which applies the ordinary checks. A new endpoint that
  changes something visible should publish a hint; it must not put content in one.
- **Presence.** Who else has the object open and which field they are in.
- **Shared documents.** Text edited by several people at once is a Yjs document; the server
  stores and relays its updates without interpreting them (`collab_docs` and
  `collab_updates`; a client merges a long run of updates into one). Saving still goes through the ordinary REST call, which writes
  the version, the history and the audit entry.

With several application nodes, messages cross nodes over PostgreSQL `LISTEN`/`NOTIFY`
(`start_background`), so no extra broker is needed.

## Notifications and alerts

`notify::deliver` is the one way a person is told something. It takes recipients the caller
has already filtered by access, applies workspace and personal choices per category, writes
the in-application notice, and queues email as a job (`email_job`, with retries and a log
row per attempt). To add a kind of notification: add a category to `settings::CATEGORIES`
if none fits, and call `deliver`. Alert rules are evaluated once a minute under an advisory lock so
that one node does it; a new condition is one entry in `notify::ALERT_KINDS` and one branch
in `notify::evaluate`.

## Content people write

Text is stored as Markdown. `refs::scan` finds mentions and links to work items and pages
when text is saved; `refs::resolve` answers, for one reader, which of them exist and may be
seen, and the renderer links only those. Custom HTML, visualizations and embedded scripts
never run in the application's own page: they run in a frame with an opaque origin and a
content security policy that allows no network access (`analytics::frame_csp`,
`web/src/components/Sandbox.tsx`). A script asks the host page for data; the host makes the
request with the viewer's session, marked `X-Gantry-Sandbox: 1`, and the server refuses any
such request that would change something.

## Adding an integration

1. Add a `KindSpec` to `integrations::KINDS` (its settings and secrets appear in the console
   and the manual automatically).
2. Outbound: add a branch to `integrations::deliver` that builds the request. Always send
   through `guarded_client`, which blocks private addresses and pins DNS.
3. Inbound: add a route under `/api/v1/hooks/…`, marked `.public()`, that verifies a signature
   before doing anything else, records the delivery and acts through `auth::service_ctx` and
   the normal item functions.
4. Add a test to `tests/external.rs` using the `recorder()` stand-in.

## The web application

`web/src/api.ts` is the only place that talks to the server over HTTP, and `realtime.ts` the
only place that holds the WebSocket. Pages live in `web/src/pages`, shared components in
`ui.tsx`, `work.tsx` and `components/`, the editors in `editors/`. Markdown is the single
stored format: `lib/markdown.ts` renders it (markdown-it, then DOMPurify) and converts the
simple editor's HTML back to Markdown (turndown). One Markdown editor
(`editors/MarkdownEditor.tsx`) serves pages, descriptions and discussions; change it once
and it changes everywhere. The diagram and workflow editors share React Flow; the chart
libraries (Chart.js, Plotly, ECharts, D3) are copied to `public/vendor` before each build
(`scripts/prepare-assets.mjs`, which also writes the license list) and loaded only inside
the sandbox frame. Heavy pieces are separate chunks loaded on first use.
Styling is hand-written CSS on tokens defined at the top of `styles.css`, with light and dark
values for each.

## Conventions

- Deny by default: a new endpoint must call an `authz` check or be explicitly `.public()`.
- Never return 403 for an object the caller may not know exists; use `authz::hide`.
- No secrets in logs, audit details or problem documents.
- A defect fixed gets a test in `tests/regression.rs` named after what went wrong.
- Anything a hostile caller could try gets a case in `tests/hostile.rs` or `tests/security.rs`.
- Nothing is sent to a browser or an agent that its account could not have fetched itself:
  not in a list, a count, a hint on the live channel, a notification or an error message.
- American English in code, messages and documentation.
- User-facing text: plain words, sentence case, say what happened and what to do next.
