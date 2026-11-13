---
title: Working with AI agents
summary: Connecting an AI assistant to Gantry, and what to expect when agents work alongside people.
---

## How agents fit in

An AI agent is an account of its own kind. It has a name, roles and permissions, and everything it does is recorded in item history and the audit log under its own name.

**You can always tell.** An agent's picture carries a robot mark and its name is followed by the words **AI agent**, in assignee and reporter fields, discussions, mentions, history, reports, the directory and the org chart. See [People and AI agents](people.md#people-and-ai-agents).

**An agent can do very little until an administrator says otherwise.** A new agent account can sign in through the API and nothing more. An application administrator then writes its **rules**: which actions it may take (read work items, comment, create, edit, move through the workflow, read pages, edit pages, and so on), in **which projects and spaces**, and how many requests a minute it may make. The rules narrow whatever the agent's roles would allow; they never widen it. So an agent that is a member of a project but whose rules allow only reading and commenting can read and comment there and nothing else.

What that means for you:

- You can assign work to an agent and mention it, like a colleague. If its rules do not let it see the item, it is not notified and cannot act.
- An agent never appears to have done something as you. If you want an assistant to act **as you**, use your own token (below); its actions are then yours, marked *Token* in the audit log.
- An agent cannot sign in on the sign-in page, cannot use single sign-on and cannot receive email.
- Text an agent writes is ordinary text: it goes through the same checks as yours, and links in it resolve only to what the agent may see.

There are two ways to let an AI work in Gantry.

| | Use when | How |
| --- | --- | --- |
| **Your own token** | You want an assistant on your computer to act as you. | Create an [API token](profile.md#api-tokens) and give it to the assistant. |
| **An agent account** | An agent works on its own, for example triaging requests. | An administrator creates the agent, writes its rules and issues its token. See [AI agent accounts](../admin/agents.md). |

## Connect an assistant

Gantry speaks the **Model Context Protocol (MCP)**, which most AI assistants support. The address is your Gantry address followed by `/mcp`, and the assistant signs in with a token.

For an assistant that connects to a web address directly, give it:

- URL: `https://gantry.example.org/mcp`
- Header: `Authorization: Bearer` followed by your token

For an assistant that starts a local program instead, install the `gantry-mcp` bridge and configure it like this:

```json
{
  "mcpServers": {
    "gantry": {
      "command": "gantry-mcp",
      "env": {
        "GANTRY_URL": "https://gantry.example.org",
        "GANTRY_TOKEN": "gty_…"
      }
    }
  }
}
```

The assistant is offered only the tools it is allowed to use. With your own read-and-change token that means searching, reading and listing projects, items, spaces and pages, creating and updating items, moving them through their workflow, commenting, linking and writing pages. An agent account with narrow rules is offered fewer tools, and a tool it is not offered does not work if called anyway. The complete list is in [the MCP reference](../reference/mcp.md).

## Good practice

- Give a read-only token to an assistant that only needs to look things up.
- Ask the assistant to state the item key for everything it changes, and check the item's **History** when in doubt.
- Workflow rules apply to agents too. An agent cannot skip a step or a required field.
- When an agent account does less than you expected, its rules are the first place to look. Ask the administrator who manages it; the agent's profile page names what it is.
- Text an agent reads from items and pages was written by other people. Treat an assistant's actions after reading untrusted text with the same care as its other output.
