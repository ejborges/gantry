---
title: REST API
summary: Authenticating, conventions, and every endpoint. Everything the web application does, it does through this API.
---

## Basics

- Base address: your Gantry address, paths under `/api/v1`.
- Requests and responses are JSON (`Content-Type: application/json`), except attachment upload and download.
- A machine-readable description is at `/api/v1/openapi.json`.
- Times are ISO 8601 in UTC. Calendar dates such as due dates are `YYYY-MM-DD`.
- Work items are addressed by key (`PUMP-12`), projects and spaces by key (`PUMP`), everything else by UUID.

## Authentication

### API tokens

Scripts and agents send a token on every request:

```bash
curl -H "Authorization: Bearer gty_3f9a1c2b7d_…" https://gantry.example.org/api/v1/items/PUMP-12
```

Create tokens under **Profile**, or for AI agents under **Administration**. A token acts as its account, limited by the account's roles, for an AI agent also by [the rules an administrator wrote for it](../admin/agents.md), **and** by the token's scope:

| Scope | Allows |
| --- | --- |
| `read` | Reading. |
| `write` | Reading and changing content. |
| `admin` | Also administrative endpoints, if the account's roles allow them. |

Tokens are checked on every request; revoking one or deactivating its account takes effect at once.

### Browser sessions

The web application signs in with `POST /api/v1/auth/login` and receives an `HttpOnly` session cookie. Requests that change anything must also carry the header `X-Gantry-CSRF` with the value returned by `GET /api/v1/auth/me`, and must come from the site's own origin. This applies to cookie sessions only; token requests need no such header. Do not build integrations on cookie sessions; use tokens.

## Errors

Errors are problem documents (RFC 9457) with the matching HTTP status:

```json
{"type": "about:blank", "title": "Bad Request", "status": 400, "code": "invalid_request", "detail": "Severity is required for Bug"}
```

| Status | Meaning |
| --- | --- |
| 400 | The request is malformed or breaks a rule; `detail` says which. |
| 401 | Not authenticated: missing, invalid, expired or revoked credentials. |
| 403 | Authenticated, but not permitted; or a token whose scope is too narrow. |
| 404 | Does not exist, **or you may not see it**. The two are deliberately indistinguishable. |
| 409 | Conflict, for example saving a page that changed since you read it, or deleting a built-in alert. |
| 413 | Body larger than `server.max_body_mb`. |
| 429 | Rate limit reached; honor the `Retry-After` header. |
| 5xx | A fault on the server. Quote the `X-Request-Id` response header when reporting it. |

An account that may not see something gets the same answers as for something that does not exist: `404` for one object, and lists that simply do not contain it. An AI agent whose rules allow nothing therefore gets empty lists, not errors.

**Browsers get pages.** When a browser *navigates* to an address that fails (a `GET` whose `Sec-Fetch-Mode` is `navigate`, or whose `Accept` begins with `text/html`), the response has the same status code and an HTML page that explains it and names the support contact. Programs get the JSON above. The texts of those pages are public at `GET /api/v1/about/status-codes`:

{{status_codes}}

Every response carries `X-Request-Id`.

## Lists and paging

List endpoints accept `limit` (default 50, at most 200) and `offset`, and return the page with the `total`. Work items are filtered with query parameters:

```bash
curl -H "Authorization: Bearer $TOKEN" \
  "https://gantry.example.org/api/v1/items?project=PUMP&category=todo,in_progress&assignee=me&sort=priority&limit=100"
```

| Parameter | Values |
| --- | --- |
| `project` | A project key. |
| `type` | One or more type keys, comma separated. |
| `status` | One or more status keys. |
| `category` | `todo`, `in_progress`, `done`; comma separated. |
| `assignee`, `reporter` | A user ID, `me`, or `none` for unassigned. |
| `label`, `cycle`, `parent` | A label, a cycle ID, a parent item key. |
| `priority` | 0 (none) to 4 (critical). |
| `overdue` | `true` for open items past their due date. |
| `sla` | Service desk: `breached` or `at_risk`. |
| `q` | Words in the title. |
| `sort` | `rank`, `priority`, `due`, `updated`, `created`, `key`. |

## Examples

**Create an item**

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  https://gantry.example.org/api/v1/projects/PUMP/items \
  -d '{"type": "bug", "title": "Gauge reads zero after restart", "priority": 3, "custom": {"severity": "Major"}}'
```

**Move it through its workflow**

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  https://gantry.example.org/api/v1/items/PUMP-31/transition -d '{"to": "in_progress"}'
```

The item's `transitions` array lists the steps available from its current status and the fields each requires.

**Comment**

```bash
curl -X POST -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  https://gantry.example.org/api/v1/items/PUMP-31/comments -d '{"body": "Reproduced on rev C."}'
```

**Update a page without overwriting someone else's edit**

```bash
curl -X PATCH -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  https://gantry.example.org/api/v1/pages/PAGE-ID -d '{"content": "# Runbook\n…", "base_version": 7, "message": "Added rollback"}'
```

If the page is no longer at version 7 the answer is 409 and nothing is changed.

**Search**

```bash
curl -H "Authorization: Bearer $TOKEN" "https://gantry.example.org/api/v1/search?q=controller+resets&kind=item"
```

`kind` is `item`, `page` or both; `mode` is `keyword`, `semantic` or `hybrid` (the default when semantic search is turned on). Results contain only what the caller may open.

## Rate limits

Each account may make `server.rate_limit_per_minute` requests per minute (600 unless changed); unauthenticated callers are limited per address. Exceeding it returns 429.

## Finding work by criteria

`POST /api/v1/items/query` (or `GET` with the same JSON in the `q` parameter) is the search behind **Find work**. It returns work items and service desk requests the caller may see.

```bash
curl -s -X POST https://gantry.example.org/api/v1/items/query \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" -d '{
    "match": "all",
    "criteria": [
      {"field": "assignee", "op": "in", "value": ["me"]},
      {"field": "status_category", "op": "not_in", "value": ["done"]},
      {"field": "due", "op": "before", "value": {"days": 7}},
      {"field": "custom.severity", "op": "in", "value": ["critical", "high"]}
    ],
    "sort": [{"field": "due", "dir": "asc"}],
    "limit": 50
  }'
```

| Key | Meaning |
| --- | --- |
| `match` | `all` (every criterion must hold; the default) or `any`. |
| `criteria` | Up to 40 objects of `field`, `op` and, where the operator takes one, `value`. |
| `sort` | Fields to sort by, each `asc` or `desc`. |
| `limit`, `offset` | Paging. `limit` is 50 by default and at most 500. |

The response has `items`, `total`, `limit` and `offset`.

`GET /api/v1/items/query/fields` lists every field the caller can search, with its kind, its label, the operators it accepts and, for choices, the values: the built-in fields and every custom field of the types the caller can see (`custom.` followed by the field's key). That list is the authority. The built-in fields are:

| Fields | Operators | Value |
| --- | --- | --- |
| `key`, `title`, `description`, `text` (title or description) | `contains`, `not_contains`, `starts_with`, `eq`, `ne` | A string |
| `project`, `type`, `status`, `status_category`, `channel` | `in`, `not_in` | A list of keys. `status_category` is `todo`, `in_progress` or `done`. |
| `assignee`, `reporter` | `in`, `not_in`, `is_empty`, `is_not_empty` | A list of account IDs; `"me"` is the caller |
| `cycle`, `request_type`, `container` | `in`, `not_in`, `is_empty`, `is_not_empty` | A list of IDs |
| `priority`, `estimate` | `eq`, `ne`, `gt`, `gte`, `lt`, `lte`, `is_empty`, `is_not_empty` | A number. Priority runs from 0 (none) to 4 (critical). |
| `created`, `updated`, `resolved`, `started`, `due` | `on`, `before`, `after`, `on_or_before`, `on_or_after`, `between`, `is_empty`, `is_not_empty` | `"2026-03-31"`, or `{"days": -7}` for a date relative to today; `between` takes a list of two |
| `label` | `in` (any of), `all`, `not_in`, `is_empty`, `is_not_empty` | A list of labels |
| `parent` | `eq`, `is_empty`, `is_not_empty` | A work item key |
| `watcher` | `in` | `["me"]`: what the caller follows. Nobody can search by what someone else follows. |
| `overdue`, `sla_breached`, `has_parent`, `has_attachments` | `is` | `true` or `false` |

Custom fields take the operators of their kind: text (`contains`, `not_contains`, `eq`, `ne`), number, date, check box (`is`), choice (`in`, `not_in`) and person (`in`, `not_in`), each also with `is_empty` and `is_not_empty` except check boxes.

Sort by `key`, `title`, `status`, `priority`, `type`, `project`, `assignee`, `reporter`, `created`, `updated`, `resolved`, `started`, `due`, `estimate` or `rank`.

Field names and operators are checked against that list and values are always passed to the database as parameters, so a criterion cannot alter the query. An unknown field, an operator the field does not accept, or a value of the wrong shape is answered with `400` and a `detail` that names it.

Saved searches are at `/api/v1/searches`.

## Live updates

The web application learns about changes over one WebSocket connection, `GET /api/v1/ws`. A script can use it too, to react to changes without polling.

- **Authentication**: the session cookie (the connection must then come from the site's own origin) or `Authorization: Bearer` with an API token of scope `read` or wider.
- **Messages** are JSON objects with a type in `t`.

```
→ {"t": "sub", "room": "project:PUMP"}
← {"t": "joined", "room": "project:PUMP", "conn": "…", "write": true, "peers": []}
← {"t": "event", "room": "project:PUMP", "kind": "item.updated", "data": {"key": "PUMP-12"}}
→ {"t": "unsub", "room": "project:PUMP"}
```

| Room | Carries | Needs |
| --- | --- | --- |
| `item:KEY` | Changes to one work item and its discussion | To be able to read the item |
| `project:KEY` | Changes to any work item in the project, and to the project | To be able to read the project |
| `page:ID`, `space:KEY` | Changes to a page; to the pages of a space | To be able to read them |
| `dashboard:ID`, `viz:ID`, `gantt:ID` | Changes to a dashboard, a visualization, a saved Gantt chart | To be able to open it |
| `work` | A hint for every change in a project the caller can see | `workspace.access` |
| `planning`, `directory`, `teams`, `team:ID` | Changes to the hierarchy, the people directory, teams | `workspace.access` |
| `user:ID` | The caller's own notifications | To be that account |

Rules that make the connection safe to expose:

- A subscription is authorized exactly like reading the object, and is checked again every minute; `{"t": "denied", "room": …}` is the answer when it is not allowed, whether or not the object exists.
- An `event` is a **hint**: it names what changed and never contains content. Fetch the object through the ordinary API, which applies the ordinary checks.
- A notification arrives on the caller's own room as `{"t": "notify", "category": …, "title": …, "link": …}`: the line the bell shows and where it leads, never the text of a message. Every connection is in its own account's room from the start, and nobody can join another account's.
- A connection may hold a limited number of subscriptions and send `realtime.max_messages_per_second`; an account may hold `realtime.max_connections_per_user` connections.
- The server sends a WebSocket ping every 25 seconds.

The other message types (`presence`, and those beginning with `y`, which carry text being edited together) are the web application's own and may change between versions.

## Requests from embedded scripts

Custom visualizations and embedded HTML run in a sealed frame and cannot call the API themselves. The page that hosts them performs their requests with the viewer's session and marks each with the header `X-Gantry-Sandbox: 1`. The server refuses any request so marked that is not a `GET`, whatever the session would allow. See [the visualization reference](visualizations.md).

## Endpoints

Endpoints marked *no sign-in* are the sign-in flow, health checks and webhook receivers that authenticate by signature. Everything else needs a session or a token, and then the permission its action implies.

{{routes}}
