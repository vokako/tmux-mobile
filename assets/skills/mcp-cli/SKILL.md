---
name: mcp-cli
description: Call MCP tools through the tmm CLI instead of native MCP — useful when your backend has no MCP support, when you want to try a server without editing your agent config, or when you need to add a tool server DYNAMICALLY mid-task and use it on the very next command. Use when you need an MCP tool that is not natively available in your session.
---

# MCP tools through the tmm CLI

Your session's native MCP tools (if any) are the normal way to call MCP —
prefer them. This skill is the ESCAPE HATCH: a way to reach any MCP server
through a shell command, including servers added seconds ago.

## Why this exists

Native MCP configs load when your CLI starts, so adding a server normally
means a restart. The `tmm mcp` subtree reads its config file on EVERY call:
add a server, and the next command already has it. It works identically from
every backend (kiro, claude, codex, grok) because it is just a shell command.

## The progressive ladder — pull only the tier you need

Never dump every schema into your context. Walk down:

```bash
tmm mcp servers                      # 1. configured servers (names only)
tmm mcp tools <server>               # 2. ONE line per tool: name — description
tmm mcp schema <server> <tool>       # 3. one tool's full input schema
tmm mcp call <server> <tool> key=value ...   # 4. call it
```

Values in `key=value` are JSON-coerced (`count=1` becomes a number,
`flag=true` a boolean). To pass the whole argument object verbatim:

```bash
tmm mcp call <server> <tool> --args-json '{"zip":"10001"}'
```

## Adding a server on the fly

```bash
tmm mcp add fetch --def '{"command":"mcp-server-fetch"}'
tmm mcp add remote --def '{"url":"https://mcp.example.com/mcp"}'
```

The config is a standard `{"mcpServers": {...}}` file at
`<workspace>/.tmm/mcp.json` (`$TMM_MCP_CONFIG` points at it; without the env
var the CLI walks up from your cwd). Editing the file directly is equally
valid — `add` is sugar. Either way the NEXT call reads it: no restart.

## Notes

- Exit codes are the MCP Inspector's stable contract: 0 ok, 4 server
  unreachable, 5 tool error / tool not found. On failure it also writes one
  JSON error line to stderr.
- Each call spawns the server fresh (~3s with globally installed binaries).
  Fine inside an agent turn; for a HOT or STATEFUL server, run it long-lived
  yourself (`tmm task start my-mcp -- <server command with HTTP transport>`)
  and register its `url` — then calls only connect, and state survives.
- Prefer direct binaries over `npx -y ...` in server defs: npx re-resolves
  against the registry on every spawn (measured 12s of a 15s call).
