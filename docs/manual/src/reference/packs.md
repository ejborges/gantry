---
title: Packs
summary: The file format for packaging fields, workflows, types and templates, so that Gantry can be extended and customized as data.
---

A pack is one JSON file, `pack.json`, in a directory of its own under the add-ons directory (`/usr/share/gantry/addons/ID/pack.json`), optionally with Markdown files for page templates beside it. Gantry's own built-in content is a pack, and so is the NASA add-on; read either as a worked example (`crates/gantry-server/packs/core.json` and `addons/nasa/pack.json` in the source).

A pack contains **definitions only**. It cannot run code, read data or change permissions, so installing one is safe to review: what you read is all it does.

## Shape

```json
{
  "id": "quality",
  "name": "Quality management",
  "version": "1.0.0",
  "description": "Nonconformance reports and corrective actions.",
  "fields": [
    {"key": "root_cause", "name": "Root cause", "type": "textarea", "description": "What allowed this to happen."},
    {"key": "disposition", "name": "Disposition", "type": "select", "options": ["Use as is", "Rework", "Scrap", "Return to supplier"]}
  ],
  "workflows": [
    {
      "key": "ncr",
      "name": "Nonconformance",
      "initial": "open",
      "statuses": [
        {"key": "open", "name": "Open", "category": "todo"},
        {"key": "investigating", "name": "Investigating", "category": "in_progress"},
        {"key": "dispositioned", "name": "Dispositioned", "category": "in_progress"},
        {"key": "closed", "name": "Closed", "category": "done"}
      ],
      "transitions": [
        {"name": "Investigate", "from": ["open"], "to": "investigating"},
        {"name": "Disposition", "from": ["investigating"], "to": "dispositioned", "requires": ["root_cause", "disposition"]},
        {"name": "Close", "from": ["dispositioned"], "to": "closed"},
        {"name": "Reopen", "from": ["closed"], "to": "open"}
      ]
    }
  ],
  "item_types": [
    {"key": "ncr", "name": "Nonconformance report", "icon": "alert-triangle", "color": "red", "workflow": "ncr",
     "description": "A product or process that does not meet its requirement.",
     "fields": [{"key": "root_cause"}, {"key": "disposition"}, {"key": "severity", "required": true}]}
  ],
  "project_templates": [
    {"key": "quality", "name": "Quality management", "domain": "Quality", "kind": "standard",
     "description": "Nonconformance reports with dispositions.", "item_types": ["ncr", "task"], "default_type": "ncr"}
  ],
  "page_templates": [
    {"key": "ncr-review", "name": "Nonconformance review minutes", "category": "Quality", "description": "Agenda and record for the review board.", "file": "templates/ncr-review.md"}
  ]
}
```

## Rules

| Part | Rules |
| --- | --- |
| `id` | Lowercase letters, digits and hyphens. Must equal the directory name. |
| `version` | Any string; shown to administrators. Change it when the content changes. |
| Keys | Lowercase letters, digits and underscores, starting with a letter. Keys are global: prefix yours (`qm_root_cause`) to avoid clashing with other packs. |
| Field `type` | `text`, `textarea`, `number`, `date`, `select`, `multiselect`, `user`, `url`, `checkbox`. `select` and `multiselect` need `options`. |
| Status `category` | `todo`, `in_progress` or `done`. A workflow needs an `initial` status and at least one `done` status. |
| Transition `from` | A list of status keys, or `["*"]` for any. `requires` lists field keys that must be filled in before the step. |
| Type `fields` | A list of `{"key": …}` entries in form order. Add `"required": true` for fields that must be filled in when an item is created. |
| References | A type's `workflow` and `fields`, and a template's `item_types`, may refer to definitions in this pack or in the core pack (`severity` and `task` in the example are core definitions). |
| Project template `kind` | `standard` or `service_desk`. |
| Page template | Either inline `body` (Markdown) or `file`, a path inside the pack's directory. |

A pack that breaks a rule is rejected as a whole when it is installed, with a message naming the problem; nothing is half-installed. A file that cannot be read as a pack at all is listed as `INVALID` by `gantry pack list`.

## Installing and updating

```bash
install -d /usr/share/gantry/addons/quality
install -m 0644 pack.json /usr/share/gantry/addons/quality/
gantry pack install quality
```

or use **Administration**, **Add-ons**. To update, replace the file and install again. Definitions that an administrator has edited since are marked *customized* and are **not** overwritten.

To distribute a pack as a system package, model it on `gantry-addon-nasa` in `packaging/gantry.spec`: the package places the files and its scriptlet runs `gantry pack install`.

## Other ways to extend Gantry

| Need | Use |
| --- | --- |
| React to what happens in Gantry | A [webhook](events.md) to your own service. |
| Change Gantry from another system | The [REST API](api.md) with an agent token. |
| Give an AI agent access | The [MCP endpoint](mcp.md). |
| Bring accounts from your directory | [OpenID Connect and SCIM](../admin/sign-in.md). |
| Use a different engine for search, files, vectors or counters | [Storage back ends](../sysadmin/storage.md). Each is a small interface in `crates/gantry-server/src/storage.rs` with two implementations to follow. |
| Add a new kind of integration | `crates/gantry-server/src/integrations.rs`: one entry in the catalog of kinds plus a delivery or receiver function. |
