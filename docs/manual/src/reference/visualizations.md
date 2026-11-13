---
title: Visualization scripts
summary: The data endpoints made for charts, the object a script is given, the libraries, parameters, limits and the sandbox, for people who write dashboards and embedded charts.
---

The guide is [Dashboards and visualizations](../user/analytics.md). This page is the detail.

## Where scripts run

| Place | How it is written |
| --- | --- |
| A saved visualization in the library | **Analytics**, **Visualizations**; used on dashboards and embedded with `![[viz:` *id* `]]` |
| A `viz` block in any text | ` ```viz chartjs ` … ` ``` ` in a page, description, message or request |
| An `embed` block in any text | ` ```embed ` … ` ``` ` with HTML, CSS, SVG and scripts |

All three run the same way: in a sealed frame in the reader's browser, with the object `gantry` in scope.

## Libraries

| Key | Library | Global |
| --- | --- | --- |
| `chartjs` | Chart.js | `Chart` |
| `echarts` | Apache ECharts | `echarts` |
| `plotly` | Plotly | `Plotly` |
| `d3` | D3 | `d3` |
| `html` | none | |

One library is loaded per visualization, from the Gantry server itself. The versions in your installation are listed under **About**, **Licenses**. An `embed` block and an `html` visualization load no library.

## The `gantry` object

| Member | Type | Description |
| --- | --- | --- |
| `gantry.root` | Element | Draw here. It fills the tile or block. |
| `gantry.size` | `{ width, height }` | Current size in pixels. |
| `gantry.params` | Object | Parameter values, all strings. A missing parameter is absent, not empty. |
| `gantry.theme` | `'light'` or `'dark'` | The reader's theme. The script runs again when it changes. |
| `gantry.colors.text`, `.muted`, `.grid`, `.axis`, `.background`, `.accent` | CSS colors | For text, secondary text, grid lines, axis lines, the tile's background and emphasis. |
| `gantry.colors.todo`, `.progress`, `.done` | CSS colors | The status colors used everywhere in Gantry. Use them only for those three meanings. |
| `gantry.colors.series` | Array of 8 colors | Categorical colors, chosen to stay distinguishable for color-blind readers in both themes. Use them in order; do not cycle past eight (group the rest as "Other"). |
| `gantry.colors.seriesInk` | Array of 8 colors | Text colors that are readable on each `series` color. |
| `gantry.api.get(path, query)` | Promise of JSON | A GET under `/api/v1/`, as the reader. `query` is an object of parameters; empty and missing values are left out. Rejects with an Error whose message is the server's explanation. |
| `gantry.formatDate(value, options)` | String | A date or time in the reader's language and time zone. `options` as for `Intl.DateTimeFormat`. |
| `gantry.formatNumber(value, options)` | String | A number in the reader's language. `options` as for `Intl.NumberFormat`. |
| `gantry.onResize(fn)` | | `fn({ width, height })` when the size changes. |
| `gantry.cleanup(fn)` | | `fn()` before the script runs again and when the tile is removed. |

A script is the body of an `async` function: `await` works at the top level, and a returned promise is awaited. An HTML visualization's `<script>` elements run in order after its markup is in place.

## Data for charts

Every endpoint returns only what the reader may see. All take `project` (a project key) **or** `container` (the ID of a program or initiative); with neither, they cover every project the reader can see.

### `GET /api/v1/analytics/summary`

`{ total, open, todo, in_progress, done, overdue, unassigned, created_30d, resolved_30d, projects }`

### `GET /api/v1/analytics/items`

Work items counted in groups.

| Parameter | Values |
| --- | --- |
| `group_by` | `status_category` (the default), `status`, `type`, `priority`, `project`, `assignee`, `reporter`, `label`, `cycle`, `created_week`, `created_month`, `resolved_month`, `due_month`. Up to three, separated by commas, give nested groups (`project,type,status_category`). |
| `metric` | `count` (the default) counts work items; `estimate` adds up their estimates. |
| `open` | `true` counts only unfinished work. |

`{ group_by, total, groups: [{ key, label, value, open, done, path }] }`. `value` is the chosen metric. With several groupings, there is one entry for each combination: `path` holds one `{ key, label }` for each level, outermost first, and `key` and `label` repeat the outermost. An item with several labels is counted once under each when grouping by `label`.

### `GET /api/v1/analytics/timeline`

| Parameter | Values |
| --- | --- |
| `interval` | `day`, `week` (default) or `month` |
| `periods` | Default 12. At most 366 days, 104 weeks or 60 months. |

`{ periods: [{ start, created, resolved, open_at_end }] }`, oldest first.

### `GET /api/v1/analytics/workload`

`{ people: [{ person, todo, in_progress, open, done_30d, overdue, estimate_open }] }`. `person` has `id`, `name`, `kind` (`human` or `agent`) and `avatar_url`; unassigned work is one entry whose `person` is `null`.

### `GET /api/v1/analytics/cycle-time`

| Parameter | Values |
| --- | --- |
| `days` | Look back up to 730 days; default 90 |

`{ items: [{ key, title, type_key, resolved_at, days }], stats: { count, average, median, p85 } }`, at most the 500 most recently resolved items.

### Anything else

`gantry.api.get` reaches every GET endpoint in [the API reference](api.md). Useful ones:

| Path | For |
| --- | --- |
| `/api/v1/items/query` with `{ q: JSON.stringify({ criteria, sort, limit }) }` | A list of work items by any criteria; see [Finding work by criteria](api.md#finding-work-by-criteria) |
| `/api/v1/projects` | The projects the reader can see |
| `/api/v1/projects/KEY/cycles` | A project's cycles |
| `/api/v1/containers` | Programs and initiatives |
| `/api/v1/reports/project/KEY`, `/api/v1/reports/container/ID` | The figures of a report, for readers who may read it |
| `/api/v1/graph/hierarchy` | The hierarchy as nodes and edges |
| `/api/v1/gantt/data` with `project` or `container` | Dated work, as on a Gantt chart |

## Parameters

A saved visualization declares parameters with a **name** (letters, digits and underscores, up to 40 characters) and a **default** (text of up to 500 characters, a number, or true or false). Values arrive in `gantry.params`, in this order of precedence:

1. the dashboard's **Show figures for** (sets `project`);
2. the tile's **Title and parameters…**;
3. the visualization's defaults.

A visualization embedded in text with `![[viz:` *id* `]]` runs with its defaults. To show the same chart for another project in a document, save a copy with other defaults, or write a `viz` block in place.

## The built-in visualizations

{{viz_templates}}

Each is an ordinary script. Open one in the library to read it, and **Make a copy to change** to start from it.

## Limits

| Limit | Value |
| --- | --- |
| Length of a script | 200,000 characters |
| Parameters per visualization | 30 |
| Tiles per dashboard | 40 |
| Height given with `height=` or after a bar | 80 to 1,200 pixels |
| Requests | Count toward the reader's ordinary [rate limit](api.md#rate-limits) |

## The sandbox

| Property | Value |
| --- | --- |
| Frame | `sandbox="allow-scripts"`: an origin of its own, no access to the page, cookies or storage; no forms, pop-ups, navigation or downloads |
| Network | The frame's policy is `default-src 'none'; connect-src 'none'`: no fetch, XMLHttpRequest, WebSocket, EventSource or beacon; no images, styles, fonts or frames from any address. Pictures and fonts can be embedded as `data:` URLs. |
| API | Only `gantry.api.get`, only GET, only under `/api/v1/`, as the reader. The server refuses other methods from a script. |
| Storage | None. Keep state in variables. |
| Workers, WebRTC | Not available. |
| Errors | An exception, or a rejected promise that is not handled, is shown in place of the chart. |

What this protects and what it does not is discussed for administrators in [Security](../sysadmin/security.md#custom-content-and-scripts).

## Accessibility checklist

- Give a `canvas` or `svg` `role="img"` and an `aria-label` that states the main numbers. ECharts does this with `aria: { enabled: true }`.
- Do not rely on color alone: label marks directly, or add a legend when there are two or more series.
- Use `gantry.colors` so that contrast holds in both themes; put text in `text` or `muted`, never in a series color.
- For anything a reader must be able to read exactly, offer a table: a `<table>` in an HTML visualization, or a second tile.
- Respect `prefers-reduced-motion`: turn animations off when it is set.
