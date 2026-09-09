# tmm CLI — the agent's hands

## What this is

`tmm` (binary at `src-tauri/src/bin/tmm.rs`) is the CLI front for the project
hub: one chat room per project and hook-derived agent states. It is the
**only active interface an agent has** to the rest of the
system — the CLI-only substrate decision from
`docs/exec-plans/agents-v2.md` §4.1: what an agent *says* goes through `tmm`,
what we *observe* arrives through hooks into `projects::telemetry`, and the
two channels are joined at read time.

There is deliberately no MCP tool surface for this. MCP requires per-backend
config materialization and three different schema dialects; `tmm` requires
one line in a system prompt. (The agora MCP daemon that once backed the Team
feature was deleted with it, board #100.)

Two subtrees break the client-of-the-hub shape on purpose: `tmm task` and
`tmm mcp` are purely local and never open a socket — see their sections below.

Every command answers tenet 6's question — "what can an agent not do without
it": send/log are the room, board is the shared task state, spawn/registry/
teams are delegation, project is workspace lifecycle, prompt/skills/mcp are
the identity assets, task is background work, agent is teammate control. A
command that no agent needs is not added; `--help` is the complete list and
this reference stays one screen.

## Commands

```
# the room (context from $TMM_PROJECT / $TMM_AGENT, exported by the launcher)
tmm send "@name <text>"              send to one or more recipients (@all, @human)
tmm send "<text>" --status           ambient progress; room-only, interrupts nobody
tmm send … --image <path|url>        attach an image by REFERENCE (repeatable)
tmm log [--since <ts>] [--limit N] [-f]      read chat; --since exclusive (ms), -f follows
tmm log --grep <text> [--grep …] [--global]  search FULL history (any-match, body+sender)

# the task board
tmm board [list] | show <id>         the kanban; one issue + its note thread
tmm board add "title" [--body <t>] [--assignee <n>]
tmm board take <id>                  claim: assignee = you, status = doing
tmm board move <id> <todo|doing|review|done>
tmm board note <id> <text>           record progress/decisions ON the issue

# teammates and projects
tmm agent list                       windows + agent detection + derived state
tmm agent interrupt|stop|restart|remove <name>
tmm spawn <agent> [--brief <text>]   spawn a registry agent into this project
tmm spawn --team <team> [--brief <text>]     start a configured team (all members)
tmm project list                     ● live / ○ down, session + path
tmm project create <path> [--name n] [--session s] [--with-agent <backend>]
tmm project up|down|archive|delete|rename <session>

# identity assets (central registry)
tmm registry list|save|delete        agent definitions (--backend --system --model
                                     --effort --skills --mcp --can-hire)
tmm teams list|save|delete           named agent teams (members + roles)
tmm prompt show|path|set|clear       the app-wide instructions (<config>/AGENTS.md)
tmm skills list|save|delete|refresh|import   app-owned skill store
tmm mcp list|save|delete             central MCP server defs (RPC half)

# background tasks — LOCAL tmux only, no server, never exits 2
tmm task start <name> [--session <s>] [--replace] -- <cmd...>
tmm task list | status <name> | logs <name> [--limit N] [--grep <t>] | stop | rm

# local MCP runtime (the mcp-cli skill; progressive tiers)
tmm mcp servers | tools [<server>] | schema <server> <tool> | call … | add …

# local helper
tmm claude-statusline                render Claude Code statusLine JSON from stdin
```

Global flags: `--project <session>`, `--agent <name>`, `--server <ws://…>`,
`--output json` (every read). Token from `config.toml`, overridable with
`$TMM_TOKEN`; server from `$TMM_SERVER`, default `ws://127.0.0.1:<port>`.

Central assets (skills, MCP defs, registry agents, teams) are the app's own
store; how they materialize into agents — built-in skills, one-url import,
isolated homes — is [agents-overview.md](agents-overview.md).

## Self-management: the app operates itself through its own CLI

Everything the UI can do to projects and the registry, `tmm` can do — and the
spawn prompt tells agents so. A lead can set up a whole project
(`tmm project create` → `up`), define a NEW kind of agent
(`tmm registry save`) and then spawn it: definition → instantiation →
delegation, all inside one conversation. This adds no authority: an agent
already holds a shell (it can run tmux or edit files directly), so first-class
commands only replace ad-hoc power with a documented, observable interface.
`can_hire` stays a resource gate on spawn — it is about fan-out control, not
security. `project up/down/archive` accept the SESSION NAME (resolved to the
project id via project_list), because the session is what agents and humans
actually see.

Verified with a real agent: a spawned lead briefed to "create a project at
/tmp/evolve" ran `tmm project create /tmp/evolve --session evolve`, verified
with `tmm project list`, and reported done — 21s end to end.

## The two hard rules

**Fail soft, never block.** The server is optional (agents-v2 principle 4):
an agent is a plain CLI process in a tmux window and must keep working when
the server is down. `tmm` enforces this with a 2s connect timeout, a 10s RPC
timeout, no retries: a dead server is one stderr line and exit 2, measured at
~20ms for connection-refused. Anything that calls `tmm` from a hook or a
prompt can treat it as fire-and-forget.

**Tiered exit codes** (multica's convention, adopted after reading its CLI
docs): `0` ok · `1` local/tmux failure (`tmm task` only) · `2` server
unreachable · `3` auth rejected · `4` not found (method missing on this
server — mobile or old build; or no such task) · `5` usage/params.
Agents and scripts branch on the class without parsing error prose.
`tmm task *` opens no socket, so it can never return 2 — see below.

## Background tasks (`tmm task`) — local tmux, no server

The rest of `tmm` is a thin WS client: the hub subcommands go through
`rpc()` and `state.db` is owned by the server process. `tmm task` is the one
subtree that is purely local (`src-tauri/src/tasks.rs`, over `tmux.rs`), and it
has to be: **what an agent most often wants to run in the background is the
server itself.** A task manager that needed the hub to be up could not start
the hub. So `task` is dispatched in `main()` before `Config::load()` — which is
not just a read, it seeds a token and a machine id into `config.toml`, and a
command that only talks to tmux has no business doing that.

### `tmm mcp` — the CLI door that became a skill

The history matters because it explains the shape: for a few hours on
2026-08-28 this subtree REPLACED native MCP entirely ("可以不用各种 agent 内部
的 mcp 工具调用了，可以用 MCP Inspector CLI 来统一来做"), and the same day the
owner reversed the call ("mcp 工具还是用原生的方式调用吧，给 kiro 的 mcp 工具
开启 toolsearch。我们这个 cli 方式调用 mcp，只作为另一个 skill 就好"). The
final shape:

- **Native MCP is the invocation path**: registry defs materialize into each
  backend's own config at spawn (kiro `mcpServers`, claude `mcp.json` +
  `--strict-mcp-config`, codex `-c mcp_servers.*`, grok `[mcp_servers.*]`
  toml), exactly as before the detour.
- **kiro gets toolSearch**: managed kiro homes carry `toolSearch.enabled=true`
  with `minPct=0`/`minTokens=0` (`kiro_cli_settings` — written at spawn,
  backfilled by `refresh_hooks`), so MCP schemas are always DEFERRED into a
  compact list and loaded on demand through kiro's built-in `tool_search`.
  That is the progressive-loading behavior, natively.
- **`tmm mcp` survives as a SKILL** (`assets/skills/mcp-cli/SKILL.md`,
  imported into the store as `mcp-cli`): the system prompt does NOT teach it
  (a spawn.rs test pins that); an agent gets it only by listing the skill.
  Its niche is what native cannot do: per-call config reads, so `tmm mcp add`
  makes a server callable on the NEXT command with no restart, uniformly on
  every backend.

The subtree itself is the second purely-local one (like `task`, dispatched
before `Config::load()`):

- The verbs shell out to the **MCP Inspector CLI** (`$TMM_MCP_CLI`, default
  `npx -y @modelcontextprotocol/inspector --cli`), which connects, invokes one
  method, prints, and exits — its exit codes are a stable contract (4
  unreachable, 5 tool error) and pass straight through.
- Discovery is **progressive, like skills** (owner, 2026-08-28: "渐进式加载…
  避免一次性加载太多上下文，有点像 toolsearch"): each tier loads only what the
  previous one made you want. `servers` prints names; `tools <server>` prints
  ONE LINE per tool (name — first line of the description; the reshaping is
  `mcp_cli::compact_tools` over the inspector's `--format json` output, so a
  50-tool server costs 50 lines, not pages of schema); `schema <server> <tool>`
  prints ONE tool's full record, read just before calling; `call` invokes it.
  The SKILL tells its reader to walk the ladder and never dump every schema up
  front (the system prompt deliberately says nothing about any of this).
- **Dynamic**: `tmm mcp add <name> --def '{"command":…}'` merges one server
  into the config NOW (creating `.tmm/mcp.json` in a fresh workspace), and
  because the inspector reads the file per call, the next call has it. Editing
  the file directly is equally valid — `add` is sugar.
- The config is a STANDARD `{"mcpServers": …}` file at
  `<workspace>/.tmm/mcp.json` — **the agent's file**: `spawn` seeds missing
  entries from the registry defs (`seed_mcp_config`) but an existing entry
  always wins and unknown entries are kept, so an agent can edit its own tool
  set and the NEXT call reads it — no restart, which the native path could
  never offer. `$TMM_MCP_CONFIG` (set at spawn) names it from any cwd; without
  the env var the CLI walks up from cwd like git does.
- `tmm mcp list|save|delete` (the central registry defs) stay RPC — they are
  the catalog; `servers|tools|schema|call|add` is the runtime.

### Why this belongs to an agent at all

An agent's constraints differ from a human's. Every tool call is a fresh,
TTY-less, one-shot shell; the only state shared between calls is the
filesystem and the process table; and the agent's own context gets compacted.
So:

- **The handle must be discoverable, not remembered.** A PID noted in the
  conversation is exactly the thing that rots. `tmm task list` is one
  `tmux list-windows -a` call that enumerates every task in every session, so
  an agent that lost its context can rediscover what it left running.
- **Output must be bounded.** Context is the scarce resource; `cat`-ing a
  500 MB log destroys the caller. `logs` scans the whole scrollback but returns
  a bounded tail (50 lines by default).
- **A real TTY matters more than it looks.** Two reasons. When stdout is a pipe
  or file, libc switches from line to block buffering, so a Python/Node task
  can write nothing to its log for minutes — an agent polling it concludes the
  task hung and kills a healthy process. And a TTY is what makes `C-c` reach
  the whole foreground process group, so `stop` collapses the process tree
  instead of orphaning its children (a `nohup`-ed `npm → tauri → vite + server`
  chain cannot be given that signal; `scripts/preflight.mjs` exists because
  those orphans really happened). The TERM/KILL escalation follows the same
  rule (2026-09-03 review): it goes to the pane's process GROUP (`killpg`),
  not to `#{pane_pid}` — that pid is the `sh -c` wrapper, and a TERM to it
  alone let `npm run dev` or a pipeline run on headless while the pane went
  dead and the task said `killed:term`. tmux starts every pane as a session
  leader, so the wrapper's pgid is its own pid and the group is exactly the
  tree the task started; `stop_ends_the_whole_process_group_not_just_the_wrapper`
  pins it with a wrapper that ignores INT and a `nohup`-ed child — the kernel's
  SIGHUP to the foreground group when the wrapper dies is exactly what such a
  child shrugs off, which is why the pid-only version looked fine in casual use.

`pm2` was rejected, not just as an extra dependency: **auto-restart lies to an
agent.** From a log tail you cannot tell "running fine" from "crashed five
times and retrying", and for a build task the restart loop is pure harm. A
standalone shell wrapper was rejected because `tmux.rs` already solves socket
discovery (`-S`), tmux binary location, and `capture-pane -J` wrap
normalization — bash would duplicate all three, badly.

### The three tmux facts it rests on (verified, tmux 3.7b)

1. **`remain-on-exit on` is what makes a finished task observable.** The pane
   goes `#{pane_dead}=1` with the code in `#{pane_dead_status}`, and the
   scrollback stays readable. Status *and* log retention from one native
   mechanism — no pidfiles, no sentinel files, no log files. A task that
   auto-vanished would be evidence destroyed: the agent could never find out
   why it failed.
2. **It must be set with `-w`.** Session scope would turn it on for every
   window the user has open, so their shells would stop closing on exit. Not
   ours to change. Verified: with a task running in the current session, a
   sibling window still auto-closes and global `remain-on-exit` is still `off`.
3. **The registry is a window option, not a file.** `@tmm_task` marks the
   window; `@tmm_cmd` and `@tmm_started` ride along so `list` needs no second
   lookup. The options are set *before* `respawn-window -k` runs the command,
   otherwise a command that exits in milliseconds takes its window down first.

Task names are globally unique — the name is the handle — so lookups scan all
sessions, and `start` refuses a name a live task holds (`--replace` to take it
over). Refusing rather than clobbering matches preflight's philosophy and keeps
parallel subagents from silently stealing each other's tasks.

### Two things that read as bugs and are not

**`logs` filters tmux's own `Pane is dead (…)` line.** tmux writes it into the
pane, on the bottom row, padding the gap above with blank rows. Left in, a
bounded `--limit 5` returned five blank lines and the real output fell out of
view. So `logs` returns task output only (and only strips the marker for dead
tasks), while `status` stays the single place that reports how it ended.

**A signal death is not an exit code.** `State::Killed(String)` is fed from
`#{pane_dead_signal}` (tmux names it: `kill`, `int`, `term`). The first cut
reported a SIGKILL as `exited:-1`, which is a lie an agent would then act on;
JSON now carries `exit_code` and `signal` as separate fields, one of which is
always null. `Exited(-1)` survives only as the "dead and tmux told us nothing"
fallback.

### Naming

`task`, not `bg`: the existing management surface is `<noun> <verb>`
(`project up`, `registry save`), a noun is what `list` can enumerate, and
shell `bg` actually means "resume a *stopped* job", which is the wrong
semantics. `spawn` was unavailable — it already spawns agents. `start`, not
`run`, because `run` implies it blocks and returns output, and an agent holding
that mental model waits forever. No aliases: they double the surface an agent
can get wrong to save three characters.

### Known limits

- Output lives in the tmux scrollback, so anything past `history-limit` is
  gone. Deliberate: writing a log file would bring back the CR/ANSI sludge
  that makes `capture-pane` output nice to read in the first place.
- Tasks are tmux windows, so `tmux kill-server` takes them with it.
- The `tmm-tasks` fallback session keeps one idle shell window (the one tmux
  creates with the session). Harmless, and it keeps the session alive between
  tasks.

## Server side — where each verb's design lives

`hub_*` RPCs in `src-tauri/src/server/hub_rpc.rs`, dispatched by prefix in
`connection.rs`. The room is `proj:<session>` in state.db's `hub_msgs`
(`projects/rooms.rs`, board #107); a room is implicit — it exists exactly when
it has messages. Mobile has no projects store, so every `hub_*` method answers
method-not-found there (`tmm` maps it to exit 4).

The rules behind each surface live next to their design, not here:

| surface | design doc |
|---|---|
| chat store, feed, paging, drafts, images | [hub-feed.md](hub-feed.md) |
| delivery, receipts, status derivation, vitals, activity log | [agent-status.md](agent-status.md) |
| recipients, `/commands`, the palette, interrupt | [hub-composer.md](hub-composer.md) |
| the task board (`hub_board_*`, `tmm board`) | [board.md](board.md) |
| spawn, isolated homes, registry, per-backend knowledge, AGENTS.md | [agents-overview.md](agents-overview.md) |
| project lifecycle, restart/resume | [projects.md](projects.md) |

## CLI/UI parity

Every agent and project verb is reachable from both the chat UI and `tmm` —
the CLI is not a subset (owner: "所有的 Agent 也可以通过 TMM 命令直接交互所有
的操作"), because an agent that can only be managed by a human cannot manage
a teammate:

| | CLI | UI |
|---|---|---|
| Agent | `tmm agent interrupt\|stop\|restart\|remove <name>` | roster card/context menu: Watch / Interrupt / Restart / Stop / Remove |
| Project | `tmm project up\|down\|archive\|delete <session>` | header: Open / Close / Delete (archive is the list's own action) |

`remove` is the eject button next to stop's pause button: it kills the window,
DROPS THE SLOT (so `up` never recreates it) and deletes the isolated home (so
`is_managed_in` stops recognising it). `delete` is archive's irreversible
sibling: it closes the session, removes every `<path>/.tmm/agents/<name>/` the
app created, and forgets the row (slots cascade; the Board's issues go with a
permanent delete, board #41). Two things both verbs leave alone on purpose:
**your files** — we only ever delete inside `.tmm/agents/` — and **the chat
history**, because the room is the record. In the chat UI, delete is a recycle
bin, not destruction (hub-feed.md); the raw CLI keeps the sharper verbs as-is.

## Verified

Against the live server: `project list` (6 projects, ● markers), `send` →
`log` roundtrip (message in `proj:tmux` with ts cursor working), `--output
json` on all reads. Dead server → exit 2 in 21ms; wrong token → exit 3.
Unit tests: derivation table (9 cases), hub dispatch (4 cases), all in
`cargo test --lib`.

`tmm task`, end to end against the real binary: a task started in the current
session reports `running`, then `exited:7` with the code from
`pane_dead_status`; `logs --limit 3` and `logs --grep error` both return
bounded, already-rendered text; `stop` on a process trapping INT and TERM
escalates and reports `killed:kill` with `exit_code: null`; `rm` on a running
task exits 5, `start` on a taken name exits 5, `status` on an unknown task
prints `missing` and exits 4. Scope: global `remain-on-exit` still `off`,
session-level unset, a sibling window still auto-closes. `-- printf %s|%s|%s
--release --limit -f` reached the command verbatim, proving flags after `--`
never touch the parser. With `TMUX`/`TMUX_PANE` unset the task lands in
`tmm-tasks` and stays fully operable from outside tmux. 13 unit tests cover the
pure helpers (quoting, row parsing incl. signal vs status, bounded tail, grep,
dead-marker strip, name validation, ages).
