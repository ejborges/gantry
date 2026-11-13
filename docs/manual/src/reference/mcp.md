---
title: MCP endpoint
summary: Connecting AI agents through the Model Context Protocol: address, authentication, tools and resources.
---

## Address and authentication

```
POST https://gantry.example.org/mcp
Authorization: Bearer gty_…
```

The endpoint uses the same API tokens, roles and scopes as the REST API. An agent sees and changes only what its account may, a `read` token cannot change anything, and every change is recorded in item history and the audit log under the agent's name. Items created through MCP are marked *via mcp*.

Requests carrying an `Origin` header from another website are refused, so a web page cannot drive a user's local MCP client into Gantry.

## Protocol

The endpoint speaks JSON-RPC 2.0 over streamable HTTP and supports both generations of the protocol:

- the stateless revision `2026-07-28`, in which each request stands alone and names its protocol version and method in headers (`Mcp-Protocol-Version`, `Mcp-Method`, and `Mcp-Name` for calls that name a tool or resource);
- the earlier revisions `2025-11-25`, `2025-06-18` and `2025-03-26`, which begin with `initialize`.

Clients negotiate this themselves; you do not need to choose.

## Connecting a client

**Clients that take a URL** (most current assistants and agent frameworks): give them the address above and the `Authorization` header.

**Clients that start a local program**: install `gantry-mcp` on the user's machine (package `gantry-mcp`, or the single binary) and configure:

```json
{
  "mcpServers": {
    "gantry": {
      "command": "gantry-mcp",
      "env": { "GANTRY_URL": "https://gantry.example.org", "GANTRY_TOKEN": "gty_…" }
    }
  }
}
```

The bridge forwards standard input and output to `/mcp` and adds nothing of its own. It verifies the server's TLS certificate.

Try the endpoint by hand:

```bash
curl -s https://gantry.example.org/mcp \
  -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -H "Mcp-Protocol-Version: 2026-07-28" -H "Mcp-Method: tools/list" \
  -d '{"jsonrpc": "2.0", "id": 1, "method": "tools/list"}'
```

## Tools

Each caller is offered **only the tools it may use**. The list returned by `tools/list` is made from the account's roles, for an AI agent account also from [the rules an administrator wrote for it](../admin/agents.md), and from the token's scope: a `read` token is not offered tools that change anything, and an agent allowed only to read and comment is offered the reading tools and the comment tool. A tool that is not offered also refuses when called by name, and each tool applies the same checks as the API for the specific project, item or page it is asked about.

The complete set:

{{mcp_tools}}

Tool results are returned both as text and as structured content. A call that breaks a rule (an unknown key, a workflow step that is not allowed, a missing required field) returns a tool error whose text explains what to do differently, so that the agent can correct itself.

## Resources

| Resource | Content |
| --- | --- |
| `gantry://item/{key}` | A work item with its fields, links and comments, as Markdown. |
| `gantry://page/{id}` | A documentation page, as Markdown. |

## Designing agent access

- Create one agent account per purpose and give it roles in only the projects and spaces it needs.
- Prefer a `read` token until the agent has proved itself; grant `write` for agents that should act.
- Workflow rules bind agents exactly as they bind people. Use required fields and review steps to keep a person in the loop where it matters, for example an *Approve* step only a human role may take.
- Content an agent reads was written by others and may contain instructions aimed at it. Limit what the agent's account can change to limit what such text could cause.
