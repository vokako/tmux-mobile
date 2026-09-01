# tmm CLI — the agent's hands

## What this is

`tmm` (binary at `src-tauri/src/bin/tmm.rs`) is the CLI front for the project
hub: one chat room per project, agent status declarations, derived agent
states. It is the **only active interface an agent has** to the rest of the
system — the CLI-only substrate decision from
`docs/exec-plans/agents-v2.md` §4.1: what an agent *says* goes through `tmm`,
what we *observe* arrives through hooks into `projects::telemetry`, and the
two channels are joined at read time.

There is deliberately no MCP tool surface for this. MCP requires per-backend
config materialization and three different schema dialects; `tmm` requires
one line in a system prompt. The agora MCP daemon still exists for Team
(legacy) and is not extended.

One subtree breaks the client-of-the-hub shape on purpose: `tmm task` manages
background tasks with local tmux only and never opens a socket. See
"Background tasks" below for why that split has to exist.

## Commands

```
# agent-facing (context from $TMM_PROJECT / $TMM_AGENT, exported by the launcher)
tmm send "<text>"                    post to the project chat; @name addresses
tmm log [--since <ts>] [--limit N] [-f]   read chat; --since is exclusive (ms)
tmm status <working|waiting|blocked> [note]
tmm done [summary]                   completion; also posts "✔ done — summary"

# human-facing AND agent-facing — self-management
tmm agent list                       windows + agent detection + derived state
tmm project list                     ● live / ○ down, session + path
tmm project create <path> [--name n] [--session s] [--with-agent kiro|claude|codex|grok]
tmm project up|down|archive <session>
tmm project rename <session> --name "New name"   the LABEL only (session unchanged)
tmm registry list                    centrally-defined agents
tmm registry save --name <n> --backend <b> [--system <text>] [--model m] [--effort <level>] [--skills a,b] [--mcp <json>] [--can-hire]
tmm registry delete <name>
tmm skills list|save|delete          central skill assets (name → ref)
tmm mcp list|save|delete             central MCP server defs
tmm spawn <agent> [--brief <text>]   spawn a registry agent into this project

# background tasks — LOCAL tmux only, no server, never exits 2
tmm task start <name> -- <cmd...>    detached in its own tmux window
                 [--session <s>]     default: the session you are in, else "tmm-tasks"
                 [--replace]         take over a name a live task holds
tmm task list                        every task in every session + state + age
tmm task status <name>               running | exited:<code> | killed:<signal>
tmm task logs <name> [--limit N] [--grep <text>]   last 50 lines by default
tmm task stop <name>                 C-c → TERM → KILL; keeps the log
tmm task rm <name>                   close a finished task's window
```

Central assets: agents reference skills and MCP servers by NAME and pick
them in the UI (chips over the defined assets — no free-text names; the
config loop closes inside the app). MCP defs live whole in the db. Skills
are APP-OWNED (state.db v7): importing COPIES the files into
`<state dir>/skills/<name>/` — agents load from there, never from the
source. The source (absolute local dir or github url) is recorded as sync
metadata with a synced_at stamp; Refresh re-syncs from it (git sources get
their clone cache invalidated first, so a refresh sees the remote's current
state). Deleting a skill removes the row AND the managed files. Legacy
inline mcp objects in old agent defs are preserved and keep working.

ONE url installs whatever it contains (`skills_import` / `tmm skills
import <url|abs dir>`; owner, 2026-08-28: "对claude plugin的支持…输入一个
url就能装上 不需要下载下来"): instead of teaching each layout,
`discover_skills` walks the fetched tree (depth ≤ 5, ≤ 50 hits, dotdirs
and node_modules skipped) for every directory holding a SKILL.md — so a
bare skill dir imports as itself, a claude PLUGIN imports each
`skills/*` entry, and a marketplace repo imports every plugin's skills.
Discovery needs the WHOLE tree, so `fetch_git_full` clones once and
disables the sparse checkout (the per-subpath sparse fetch is for a
known skill dir). Each imported row records a source pointing at ITS OWN
directory (a `tree/<ref>/<subpath>` url, or an absolute path), which is
what keeps per-skill Refresh working; names come from the skill's own
frontmatter (sanitized to the store's [a-zA-Z0-9_-]), a name collision
inside one import suffixes `-2`, and a built-in name is SKIPPED, never
stolen. In the UI, leaving the name EMPTY on a new skill routes the
save through the importer. The editor also previews the whole managed
dir, not just SKILL.md (`skills_files`/`skills_file`, same owner
request): a chip per file, `.md` renders, anything else shows as
monospace text; the server caps previews at 256 KB, refuses non-text,
and rejects path escapes.

Three skills are BUILT-IN (owner, 2026-08-28: "应该有一个默认的内置的
skill…来源就是内置的 比如tmm可能就得有一个skill  mem命令也有一个skill"):
`tmm-cli` (the full CLI reference beyond the system-prompt paragraph),
`mem` (the hierarchical `_MEMORY.md` memory-tree CLI) and `mcp-cli` (the
MCP escape hatch below). Their SKILL.md files live in `assets/skills/*`
and are embedded in the binary (`projects::BUILTIN_SKILLS`,
`include_str!`); `seed_builtin_skills()` materializes them into the
managed store at every server start with `source = "builtin"`. Refresh
re-syncs from the RUNNING BUILD (not a path or remote), save/delete
refuse the reserved names — a deleted built-in would silently resurrect
at the next restart, so the UI hides delete and locks the source field
instead. Agents still OPT IN by listing the skill; nothing is injected
unasked.

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

Global flags: `--project <session>`, `--agent <name>`, `--server <ws://…>`,
`--output json` (every read). Token from `config.toml`, overridable with
`$TMM_TOKEN`; server from `$TMM_SERVER`, default `ws://127.0.0.1:<port>`.

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

The rest of `tmm` is a thin WS client: all 21 hub subcommands go through
`rpc()` and `state.db` is owned by the server process. `tmm task` is the one
subtree that is purely local (`src-tauri/src/tasks.rs`, over `tmux.rs`), and it
has to be: **what an agent most often wants to run in the background is the
server itself.** A task manager that needed the hub to be up could not start
the hub. So `task` is dispatched in `main()` before `Config::load()` — which is
not just a read, it seeds a token, a machine id and the team defaults into
`config.toml`, and a command that only talks to tmux has no business doing
that.

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
  those orphans really happened).

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

## Server side

`hub_*` RPCs in `src-tauri/src/server/hub_rpc.rs`, dispatched by prefix in
`connection.rs` exactly like `team_*`:

- `hub_post { session, from, body, requires_reply? }` — room auto-opens.
- `hub_log { session, since_ts?, limit? }` — incremental cursor filters on
  message `ts` (epoch ms) in our layer.
- `hub_status { session, agent, state, note? }` — resolves the agent NAME to
  a window index (telemetry's key) via the window-name match; rejects unknown
  states/names with invalid-params.
- `hub_done { session, agent, summary? }` — records completion AND posts a
  `✔ done` line to the room: the chat is the record. A non-empty summary is
  also DELIVERED into the pane of the agent that spawned this one (the
  `spawned_by` field `tmm spawn` records in the launch recipe): a record-only
  room line wakes nobody, so a lead that spawned two builders never learned
  they finished (owner, 2026-08-29). The line lands as
  `[tmm chat <ts>] <name>: [done] <summary>` with the same `record_delivery`
  bookkeeping as an @mention; targeted, one recipient, once per turn end —
  the chain terminates at the human (empty `spawned_by`), so it cannot loop,
  and hook-sourced posts stay record-only.
- `hub_agents { session }` — one row per live window: name, command, agent
  detection (`projects::agents::detect`), derived state.
- `hub_board_list/get/save/note/delete` — the project TASK BOARD (owner,
  2026-08-29: "引入一个新的看板功能…借助软件工程，可以把我们的任务管理的更好"):
  session-scoped issues in four fixed columns (`todo/doing/review/done`,
  `projects::BOARD_STATUSES` — free-text statuses would fork the vocabulary
  per agent). Public numeric `id` / `#N` is a PROJECT-LOCAL durable sequence
  (board #41's final owner ruling): every session starts at 1, advances
  independently, and never reuses a deleted number. All UI, room and `tmm
  board` operations resolve `session + id`, so project A and B may both have
  `#1` without ambiguity. `issues.id` remains a hidden database-wide row key
  solely so `issue_notes.issue_id` needs no rebuild. Schema v16 adds
  `project_number`, a unique `(session, project_number)` index, and
  `issue_sequences`; migration ranks old rows by their former global id within
  each session, rename moves rows + sequence transactionally, archive retains
  them, and permanent delete removes both before releasing the alias. `save` creates (no id) or PATCHES (id + only the changed
  fields, COALESCE in SQL — an agent's `move` must not erase a body the
  human edited meanwhile); `note` appends to the issue's own thread and
  bumps `updated_at`; every write records WHO acted. The HUMAN's half is the
  board PAGE (`src/lib/hub/Board.svelte` — its own tab, not a Hub drawer
  partition; the Hub header's layout icon jumps to it); the AGENT's half is
  `tmm board list|show|add|take|move|note|delete` — `take` = assignee+doing
  in one move, and the tmm-cli skill carries the conventions (take before
  you start; note decisions ON the issue; only the acceptor moves to done).
  UI assignment is a real `hub_post` dispatch. Its brief carries the original
  title/body AND `assignNotes()`'s chronological authored note thread (board
  #42), separated under `Notes (N):`; note lines share a 1200-character budget,
  and a partial/omitted tail points to `tmm board show <id>`, so the discussion
  reaches the Agent without one giant note flooding its pane. A freshly
  created issue has no thread and appends nothing.
  Schema v12 (`issues` + `issue_notes`, cascade); v15 adds the durable
  `agent_touched` edit lock (board #43). **The original brief becomes
  history**: `hub_board_get.editable` is true only while the current assignee
  is empty and `agent_touched` is false. Any non-human save/note sets the bit
  permanently, and `issue_save` rejects later title/body patches even if the
  issue is unassigned or moved back; status/assignee/note remain writable so
  workflow never freezes. Existing rows backfill conservatively from
  non-todo status, Agent authorship, or Agent notes. The detail renders locked
  title/body and every note as selectable static text, not fake editors.
  **Two axes, joined at
  events**: the board is the issue's lifecycle, `tmm status` the window's
  live turn. Every status change posts a `[tmm] board #N a → b` room line,
  and a move TO `review` is a handoff — the REPORTER gets the line typed
  into its pane (`deliver_chat_line`, shared with the done-summary edge;
  a human reporter reads the board itself, the actor is never told of its
  own move, `done` posts a room line only — no delivery loop is possible).
  A change somebody else makes to an ASSIGNED issue reaches the assignee the
  same way (`board_change_notice`, pure) — with one carve-out (board #30,
  owner 2026-08-31: "如果任务我标记为done可以不用给agent发送提示了"): a save
  that MOVES the issue to done wakes nobody, even when title/body ride along
  with the acceptance — closing the work leaves the executor nothing to act
  on, and the room's `[tmm] board … → done` line still records it. A later
  edit to an already-done issue (a reopen included) is ordinary news again.
  A Board NOTE is a direct reply as well as durable thread history: after the
  note is stored, an author other than the current non-human assignee wakes
  that assignee through the same targeted delivery + receipt path, carrying
  issue id/title and a bounded excerpt. Unassigned, human-assigned, self-note,
  unmanaged or offline cases degrade to the persisted note — never an RPC
  failure, shell injection, self-prompt, or duplicate delivery. On the Board
  page a note bubble also wears Chat's ONE shared `.m-acts` overlay (board
  #46): tap reveals Copy, raw body goes to the clipboard, text selection wins,
  and the timestamp is the accessible trigger. The overlay is absolute and
  transient (outside/Escape/context/Copied close). Its pure
  `{open,copied,gen}` transitions guard both sides of the async boundary — the
  attempt gen is captured before `writeText` awaits and the beat's gen is
  captured before its timer — so deferred resolves and stale timeouts are
  no-ops in a later note/issue.
  **The title is OPTIONAL** (board #31): `issue_save` refuses only a
  CONTENTLESS issue (create and patch alike — a patch may clear the title
  while a body remains, and the cleared title persists EMPTY, verbatim);
  `tmm board add --body "…"` files a body-only issue. Every surface that
  NAMES an issue — the room's `board #N a → b — {ref}` line, review/change/
  note notices, `tmm board list|show`, the Board page's cards and delete
  confirm — speaks ONE fallback, `projects::issue_ref` (client mirror
  `issueRef` in `board.ts`): trimmed title, else the body whitespace-squashed
  and cut on a char boundary at `ISSUE_REF_CHARS` with a `…` marker, else
  `#id`. One helper on each side of the wire, so the same issue never wears
  two names and no message ever renders an empty head or dangling separator.

The room is `proj:<session>` on the same agora bus that Team uses —
`TeamBridge::open_room` provisions it with **no tmux session, no roster, no
supervisor, no workspace history mirror** (empty workspace skips the `.tmm/`
mirror on purpose: a project dir should not grow dot-dirs because someone
chatted). `teams()` filters `proj:*` rooms out so they never appear in the
Team switcher; `recover()` skips them (no tmux session to find) and they
re-open lazily on the next post/read.

Mobile: no bus → every `hub_*` method answers method-not-found (`tmm` maps it
to exit 4). Same degradation contract as `team_*`.

## Status derivation (`projects/telemetry.rs`)

Status is **derived from observed facts**, never self-reported state kept on
faith. The store is in-memory, keyed `(session, window_index)` — the same
granularity as a hook notification and a project slot.

A turn is a bracket, and the hooks now report all four of its edges:
`userPromptSubmit` opens it, tool calls happen inside it, a permission prompt
suspends it, `stop` / `tmm done` closes it. So the whole machine is *which
boundary is the most recent fact*, and there are exactly four states:

| newest fact | state | `since` |
|---|---|---|
| a failed stop (StopFailure) | `failed` | the stop |
| an explicit `tmm status waiting\|blocked`, still fresh (30 min TTL) | `waiting` | the claim |
| a turn end (`stop` / `tmm done`) | `idle` | the end (detail = done summary) |
| an ask (`permission_required` / `input_required`) | `waiting` | the ask |
| a turn start (`userPromptSubmit`, or a tool call) | `running` | **the START** |
| no hook has ever spoken for this window | pane activity < 30 s | `running` else `idle` |

`since` for `running` is the turn's start, not the newest event, so a client
renders "running 2m14s" and means the turn's age.

**What `tmm status` is still for, now that turns are observed.** Only the part
we cannot see: being stuck on something outside the agent's control (a
credential, an answer, another agent). `waiting` and `blocked` both set
`waiting` and keep the note. A claim of `working` sets NOTHING — the turn
bracket already answers "is it running" — and contributes only its note as the
detail line. That removes a whole class of contradiction where an agent declared
itself busy while its own stop hook said the turn was over, and it is why the
seeded system prompt now tells agents not to announce that they are working.

Four words is the whole set. A state nobody can point at an observation for is a
state nobody should trust.

## What lives in `<workspace>/.tmm` — three generations, one directory

`.tmm/` is per-workspace runtime state, self-gitignored (`.tmm/.gitignore` is
`*`, written by `ensure_gitignore` on the first spawn). It looks messy on an old
checkout because three features have written into it, and only two of them are
current:

| Path | Owner | Status |
| --- | --- | --- |
| `.tmm/agents/<window_name>/` | **Projects / agents-v2** (`projects::spawn`) | current |
| `.tmm/teams/<team-id>/{kiro,claude,codex}/` | **Team** (`team::workspace`) | current |
| `.tmm/kiro-home/`, `.tmm/heartbeat.sh`, `.tmm/keepalive.sh` | Team, pre-multi-team layout | legacy, read-only |

An **agent home** (`.tmm/agents/<window_name>/`) is the whole identity of one
managed agent, and its shape is dictated by the backend it wraps:

```
.tmm/agents/builder-2/
  agents/builder-2.json     # kiro agent config: prompt, model, tools, mcpServers, hooks
  settings/cli.json         # kiro CLI settings (trust-all confirmation off)
  sessions/cli/*.jsonl      # kiro's own conversation store, inside OUR home
  launch-builder-2.sh       # the launch line, sourced by the pane (never send-keys'd — >2KB gets eaten)
  launch.json              # the recipe a restart replays: env + identity command
```

The directory name is the **tmux window name**, not the registry def name:
spawning `builder` twice gives windows `builder` and `builder-2`, hence two
homes. That name is the agent's identity everywhere — telemetry, `tmm status`,
`@mentions` — and `projects::managed_home` / `is_managed_in` is the ONE function
that turns it back into this path (see "Managed vs direct windows").

For claude the same directory holds `mcp.json` + `settings.json` (passed with
`--mcp-config` / `--settings`); for codex it is `codex/` as `CODEX_HOME` plus
`codex/hooks.json`.

Nothing here is hand-written and nothing needs backing up: `spawn` materializes
it from the registry def, `refresh_hooks` repairs it on every start,
`agent_remove` deletes one home, and `project_delete` deletes all of them. The
legacy row in the table is the only part that is neither written nor migrated
any more — `team::workspace` still *reads* `.tmm/kiro-home` so an old team keeps
working, and new teams go to `.tmm/teams/<team-id>/`. Deleting the legacy paths
on a workspace whose teams have been re-created costs nothing.

## The model belongs in the agent config, not on the launch line

A registry def's `model` used to be pasted onto kiro's launch line as
`--model <id>`, with an empty def falling back to a hardcoded
`claude-sonnet-4.6`. Two failures came out of that, both silent:

* **A wrong id downgraded instead of failing.** kiro-cli's TUI prints
  `Model 'x' does not exist` above the splash screen and then starts on its
  DEFAULT model, so `claude-sonnet-4-5` — one character off the real
  `claude-sonnet-4.5` — produced an agent that answered normally on the wrong
  model. (In `--no-interactive` mode the same flag is a hard error, which is why
  this was never seen in a script.) Owner report, 2026-08-19: "kiro 里配置的模型
  好像没有生效".
* **The model was invisible where the owner looked for it.** It was in
  `launch.json` and in the pane's scrollback, not in
  `.tmm/agents/<name>/agents/<name>.json` — the file that otherwise IS the
  agent.

So `render_kiro` writes `"model": "<id>"` into the agent config (a first-class
field of kiro's agent schema — verified: a bogus value there is reported as a
real error on the first turn, not swallowed) and the launch line carries only
`--agent`. Three things follow:

* Every start reads the same field, because they all pass `--agent`: first
  launch, `up`, `hub_agent_restart`, `--resume-id`. The pre-recipe backfill in
  `refresh_hooks` used to drop the model entirely on restart; there is nothing
  left to drop.
* `refresh_hooks` migrates one `--model` off an old recipe into the config and
  strips it from the line, so the two can never hold different opinions. The id
  is read off the line that really ran — nothing is guessed — and an id the
  backend REJECTS is dropped rather than migrated: it was never the agent's
  model (kiro was running its default), so carrying the typo forward would turn
  a working agent into a mute one on its next restart.
* An empty `model` means what the editor's placeholder says, the BACKEND's
  default, and the key is omitted. The old hardcoded `claude-sonnet-4.6`
  contradicted that placeholder and would have outlived the model.

Validation is where the app can still say something useful: `projects::models`
asks `kiro-cli chat --list-models -f json` (cached 10 min) and both
`registry_save` and `spawn` reject an id the backend does not know, listing the
ones it does. The list is never hardcoded — model ids change weekly — and
everything degrades soft: no CLI, no login, or unparsable output all mean
"cannot know", and an unknown id is accepted rather than blocking a save.
`models_list` exposes the same list to the agent editor as a datalist. claude
and codex take aliases nobody can enumerate, so their field stays free text.

**Reasoning effort is the same kind of identity field** (owner, 2026-08-22:
"agent配置里应该有thinking effort的配置选项"): `reg_agents.effort` (state.db
v11), empty = the backend's default, exactly the model's contract. Unlike
models the levels are a FIXED enum per backend, measured per CLI
(`models::effort_values`): kiro/claude `low|medium|high|xhigh|max`, grok
`low|medium|high|xhigh`, codex `minimal|low|medium|high|xhigh` — so the
editor offers a Select, not free text, and `registry_save`/`spawn` reject a
value the CLI would answer with a warning and a silent default. Delivery is
each backend's own knob: a `--effort` launch flag for kiro/claude/grok
(measured on each CLI; recorded in the recipe so restarts keep it), a
`-c model_reasoning_effort=…` config override for codex.

## Config drift: the app owns managed agent configs

Hooks are how we observe an agent at all, so a config on disk must never be
older than the build reading its events. It was: agents spawned before
`userPromptSubmit` existed kept a three-hook config, and because that hook is the
only reset of the same-turn dedup flag, their first `tmm send` silently killed
the stop-hook auto-post for the rest of the window's life — the owner-visible
symptom being "the agent's final reply never shows up" (2026-08-16, three live
agents on the dev machine all had `[postToolUse, preToolUse, stop]`).

`spawn::refresh_hooks(project_path, window_name)` rewrites the `hooks` key in
place — kiro's `agents/<name>.json`, claude's `settings.json`, codex's
`codex/hooks.json` — and nothing else, because the prompt carries the brief the
agent was given once at spawn and that cannot be rebuilt. It is a no-op when
already current, and it is called on every start: `hub_agent_restart` and
`reconcile` when a project comes up. The hook sets themselves live in ONE place
each (`kiro_hooks` / `claude_hooks` / `codex_hooks` / `grok_hooks`), shared by render and
refresh, so the two cannot disagree.

**Settings drift is config drift too.** A managed kiro home carries
`settings/cli.json` (read because the pane launches with `KIRO_HOME=<home>`),
and its canonical content lives in ONE function (`kiro_cli_settings`), written
fail-loud at spawn and backfilled fail-soft by `refresh_hooks` on every start —
a file that predates a key gains it, a key the app does not own (say, the
user's `chat.editMode`) survives untouched, and an already-canonical file is
not rewritten. Two keys today: `chat.disableTrustAllConfirmation = true` (an
agent has nobody at its keyboard to answer a confirmation), and
`chat.defaultInterruptBehavior = "queue"` — an owner decision, 2026-08-20
("所有 Agent 在 kiro 里边发送指令的模式 默认给我设计成 Queue 队列模式吧 不要
steer 模式"): a line typed at a BUSY agent waits and arrives whole as the next
prompt instead of steering the running turn, which is also the contract the
delivery pipeline already assumes (`delivery_overdue` pauses the ack clock
while a turn is open precisely because a busy kiro queues what we type).
Team's kiro renderer (`team/backends.rs`) mirrors the same two keys, as it
mirrors the rest of the rendering.

A CLI reads its config at launch, so patching the file cannot repair a RUNNING
agent — restart is the only path, which is what the roster's restart button is
for.

## The grok backend (grok 1.0.5, added 2026-08-21)

The fourth backend, aligned with kiro's shape and verified live (an isolated
home spawned from the hub answered a real turn on the owner's Bedrock custom
model, hooks fired, `tmm done` posted, state derived):

- **Isolation**: `GROK_HOME=<ws>/.tmm/agents/<name>/`, like KIRO_HOME.
- **Identity**: `agents/<name>.md` — YAML frontmatter (`name`, `description`,
  and the MODEL, honored from frontmatter; same lesson as kiro, nothing on
  the launch line) with the system prompt as the body; launched as
  `grok --always-approve --agent <name>`. Skills ride the prompt as the
  compact index, like claude.
- **Telemetry**: `hooks/tmux-mobile.json` in the home — that home's "global"
  hook scope, always trusted. Five events: UserPromptSubmit (prompt +
  same-turn-dedup reset; payload wraps the text in `<user_query>` tags),
  Pre/PostToolUse (camelCase keys: `toolName`/`toolInput`), Stop,
  StopFailure. **A grok `stop` is a completion ONLY with
  `reason: "end_turn"`** — a second observe-only stop fires at session
  teardown (`shutdown`/`channel_closed`) and reading it as a completion
  would double-post; the reply rides `lastAssistantMessage`.
- **Auth carries, prefs do not**: grok auth is HOME-scoped (`auth.json`,
  plus custom `[model.*]` entries whose keys ride env vars), so
  `grok_config_toml` copies the user's `[models]`/`[model.*]` catalog (and
  `auth.json` when present) into the isolated home — without it the agent is
  a login screen. User hooks/UI/MCP deliberately do NOT carry. TRAP, paid
  for once: toml 1.x parses a DOCUMENT via `toml::Table`; `Value::from_str`
  fails on any real config ("expected nothing") and the catalog silently
  vanished. `[folder_trust] enabled = false` keeps the TUI from parking at
  a trust prompt nobody can see.
- **Resume**: `--continue` is cwd-scoped (safe, unlike codex `--last`);
  `--resume <id>` exact, id from the hooks' `sessionId`.
- **Models**: `grok models` enumerates (bullet list, `*` marks the default),
  so grok ids validate like kiro's; custom models appear by name.
- **Vitals**: its own dialect — a header line ends in the context RATIO
  (`47K / 500K`; the percentage is computed, K/M suffix required on the
  total, used ≤ total) and the input box's bottom border names the model
  (`╰── Grok 4.6 (Bedrock) · always-approve ─╯`, first `·`-segment; the
  approval mode is not a vital). `sniff_remembered` dispatches on the
  detected backend, because reading one CLI's screen with another's grammar
  yields confident nonsense.

**Pane activity is not a work signal for a window that has hooks**, and that
correction is the point of this rewrite. It used to be: `window_activity` newer
than the last stop, within 30 s, meant `working`. But an agent TUI repaints
after it answers — spinner, status line, cursor — so activity was *always*
newer than the stop and every finished agent read `working` forever (owner
report, 2026-08-16). Windows with no hook coverage at all still fall back to it,
because for them the alternative is no signal.

Records are dropped when the window disappears (`retain_windows` on every
`hub_agents`), and notifications feed the store *before* dedupe (dedupe is a
notification-UI concern; telemetry wants every fact).

## The claude backend on Bedrock (claude 2.1.239, wired 2026-08-22)

Installed and verified live the same day the owner asked for it ("都用bedrock
渠道…复用我们全局定义的配置 但是自己管理好类似kirohome这种"): a managed claude
agent spawned from the hub answered a real turn on
`global.anthropic.claude-sonnet-4-6` via Bedrock, hooks fired, `tmm done`
posted, the reply auto-posted.

- **Channel**: the USER's `~/.claude/settings.json` carries an `env` block
  (`CLAUDE_CODE_USE_BEDROCK=1`, `AWS_REGION`, `ANTHROPIC_MODEL`,
  `ANTHROPIC_SMALL_FAST_MODEL`) — SigV4 from the normal AWS credential chain,
  no API key, same idea as codex's `amazon-bedrock` provider.
- **Isolation**: `CLAUDE_CONFIG_DIR=<ws>/.tmm/agents/<name>/` — claude's
  KIRO_HOME. History, session state and `.claude.json` stay in the agent's
  home. The relocation also unhooks the user settings layer, so the channel
  is INHERITED: `render_claude` copies the user file's `env` block into the
  isolated `settings.json` (grok's "auth carries, prefs do not" in claude's
  dialect; plugins/marketplaces deliberately do not carry), and
  `refresh_hooks` backfills a missing `env` on every start.
- **First-run furniture, both measured**: a fresh config dir parks the TUI at
  the theme-onboarding picker before anything else, so `.claude.json` is
  pre-seeded with `hasCompletedOnboarding` (never clobbered — it records
  folder trust); the bypass-permissions acceptance dialog is suppressed by
  `skipDangerousModePermissionPrompt: true`; the folder-trust prompt is
  answered by the spawn confirmation machinery
  (`CLAUDE_FOLDER_TRUST_MARKERS`, wording re-verified on 2.1.239).
- **Model**: empty means the BACKEND default — the inherited env's
  `ANTHROPIC_MODEL` — so no `--model` is passed (the old hardcoded `sonnet`
  alias overrode the env and does not resolve on Bedrock). A configured model
  rides `--model`, which wins over env.
- **Hook payloads verified** (they were claude-documented, codex-measured
  until now): `hook_event_name`/`prompt`/`session_id` on UserPromptSubmit,
  `last_assistant_message` on Stop — the normalizer's claude arm matches.

Two plumbing lessons came out of the live test, both general:

- **A managed window's backend comes from its RECIPE, not the pane sniff**
  (`agents::detect_managed`): the npm codex runs as `node` — pane command
  `node`, title = project name, window name = agent name, nothing says
  "codex" — so a spawned codex fell out of delivery, the roster, vitals and
  recovery entirely. We wrote the backend into `launch.json` at spawn; for
  our own windows the record beats the sniff, and hand-started windows still
  sniff.
- **`send_command` sleeps 200 ms between the text and the Enter**: a TUI that
  receives the burst back-to-back treats the Enter as paste content and
  parks the delivered line in its composer unsubmitted (measured on codex;
  Team's `nudge_pane` had learned the same beat earlier).

## Backend parity (2026-08-22)

The owner asked for claude/codex/grok to match kiro's feature set ("都要和kiro
我们现在支持的特性能对齐"). What was actually missing, and what was done:

- **Turn-start hooks**: `claude_hooks` and `codex_hooks` registered
  Pre/PostToolUse + Stop but NOT `UserPromptSubmit` — the only reset of the
  same-turn dedup flag and the only carrier of the submitted `prompt`. Their
  agents therefore lost the stop-hook auto-post forever after their first
  `tmm send`, and every line `deliver_mentions` typed stayed "unconfirmed".
  Both now register it, `is_user_prompt_submit` recognizes their spelling
  (snake key, `"UserPromptSubmit"` value), and `refresh_hooks` backfills
  every already-spawned agent on its next start. Codex payloads were measured
  live (codex-cli 0.148.0, isolated CODEX_HOME): same schema family as
  claude — `hook_event_name`, `prompt`, `session_id`, `tool_name`/`tool_input`,
  `last_assistant_message` on stop. Codex has NO StopFailure event (binary
  checked), so `failed` cannot be derived for it.
- **Vitals**: codex has its own dialect now (`sniff_codex`, measured): the
  persistent footer `<model> [<effort>] · <cwd>` (second segment must be an
  absolute path, model token must carry a digit) and context as
  `NN% context left` / the `/status` card's `NN% left (… used / …)` — codex
  says LEFT where kiro says USED, so the vital is `100 − NN`. claude gets the
  EMPTY reading on purpose: the CLI is not installed here, its furniture
  cannot be measured, and the old `_ => sniff_kiro` fallback read its pane
  with the wrong grammar.
- **Resume**: managed codex restarts now resume — `relaunch_line` splices the
  SUBCOMMAND in (`command codex resume <id> <flags>`; verified that `resume`
  accepts the recipe's own flags). Appending like the flag backends would
  have handed `resume` to the CLI as a prompt. Still never `--last`
  (machine-wide).
- **Palette**: per-backend command tables — see the slash-command section.
- **Not aligned, with reasons** (also in `docs/unresolved.md`): claude vitals
  and palette (the CLI is installed and on Bedrock since later that day — see
  the claude backend section — but its status furniture and `/` popup are
  still untranscribed);
  auto-continue recovery patterns for claude/codex/grok (their transient
  error texts have not been captured; detection is deliberately narrow and a
  guessed pattern types into working agents); codex `failed` state (no
  StopFailure event exists).

## Images: a reference, never bytes

`tmm send --image <path|url>` (repeatable) attaches an image by REFERENCE. The
CLI resolves it for a reader who is somewhere else — a URL passes through, `~`
expands, a relative path is made absolute against the agent's cwd — and appends
it to the body as `![](src)`. The room stays a log: no base64 ever enters a
message.

The client splits those references out of the markdown (`splitImages`) instead
of letting the renderer emit `<img>`, because a filesystem path is not a URL a
webview can load. `http(s)` / `data:` / `blob:` go straight into the tag;
anything else is fetched through the same signed `/dl` endpoint the file browser
uses, so a screenshot streams rather than arriving base64'd through the RPC
channel. A reference that cannot be resolved renders as the reference itself —
"it sent /tmp/x.png" is still information.

## Managed vs direct windows

Two kinds of windows, presented apart (owner decision, 2026-08-01). MANAGED
agents were spawned from the registry: isolated home, tmm-wired, kicked —
they are chat participants (cards, DM targets). DIRECT windows are everything
else: shells and agents the user started by hand — they are terminal things
and appear only inside the terminal drawer's window list (tagged "direct").
The marker is the isolated home itself: `hub_agents` sets `managed` iff
`<workspace>/.tmm/agents/<window_name>/` exists. Shells never get chat
affordances; a hand-started kiro can still be @-delivered to (useful), but
the chat roster does not advertise it.

## Delivery: how a chat line reaches an agent

The bus stores the record, but an interactive CLI only reacts to what lands in
its input box. On `hub_post`, every `@name` (or `@all`) whose name matches an
AGENT window in the session gets the line typed into its pane as
`[tmm chat] from: body` — an idle agent wakes and acts on it, a busy agent
sees it queued in its input. Shells never receive delivery (typing into a
shell would EXECUTE the message), and the sender's own window is skipped.

The line carries local wall time — `[tmm chat 2026-08-17 16:31] human: …` — for
the reader, not for us. A CLI reads that line inside a conversation that may have
been idle for hours, and "when was this said" is context it cannot recover: its
own clock only tells it *now*. Same reason the spawn KICK is stamped
(`kick_now()`), and the same reason the SYSTEM PROMPT is not: a prompt is
replayed every time the window is restored, so a date baked into it becomes a lie
a few days later. `tmm log` renders stored timestamps the same way instead of
printing raw epoch millis, which told an agent nothing it could reason about.

`send-keys` succeeding proves only that the pane existed. Whether the CLI
accepted the text as a *prompt* is a different question, and the
`userPromptSubmit` hook is the only thing that answers it: the payload carries
the submitted `prompt`, so a typed line that comes back is delivered. So
`deliver_mentions` records the line as pending (`telemetry::record_delivery`),
the echo clears it (`record_prompt`, containment match — the CLI may submit our
line with the agent's own half-typed text attached; the containment runs
whitespace-BLIND on both sides (`strip_ws`, mirrored client-side by `squashWs`):
a newline in the body never survives the round trip — the composer renders it
as a space or its own wrap (owner, 2026-08-22), and tmux in extended-keys mode
dropped the raw `\n` byte outright so the echo came back with the lines glued
together, which a squash-to-one-space canon could never contain (owner,
2026-08-24, measured live: `AgenticAI\nAgentic…` echoed as
`AgenticAIAgentic…`). The delivery is fixed at the source too:
`tmux::send_command` routes any text containing `\n` through `paste_text`,
because bracketed paste is the one channel that carries a newline INTO a
composer — send-keys loses the byte and a named `Enter` would submit the
half-typed line), and `sweep_deliveries`
reports the ones still pending after 45 s as a `warn` event. The sweep runs
when a client reads `hub_activity`, which is exactly when the answer is wanted.

**A typed line joins a QUEUE, it does not replace one.** `Rec.pending` was a
single slot, so a second message typed at a busy agent erased the first one's
record — and when the agent finally submitted that first queued line, nothing
matched it, so it came back as a prompt the user had typed *locally*: rendered as
an "input" row, while the message it belonged to kept its hollow ring for ever
(owner, 2026-08-20: "在对列里后续隔很久才响应的消息 … 显示到 input 上了，但是没有当
成已读的消息，给跳过了"). It is a `Vec` now: `record_prompt` matches on CONTENT and
removes exactly what it found (so a queue may be worked through in any order, and
ONE submitted prompt carrying several of our lines acknowledges all of them),
`overdue_lines` reports per line rather than per window, and re-typing the same line
replaces its entry instead of queueing a duplicate that could never be acked twice.
The client side of the same bug: `feedBlocks` marked only the NEWEST message an echo
contained, so a batch submission left the earlier ones hollow.

**The ack clock does not run while a turn is open.** A queued line is not a lost
line: an agent that is mid-turn holds what we typed in its input queue — kiro says
so on screen ("Type to queue") — and submits it when the turn ends, which for a
long turn is many minutes later. Sweeping on typing-time alone therefore warned
about the system working as designed, and the owner watched perfectly good
messages turn unconfirmed ("有一些queue的指令，没办法马上confirm，这个不用立刻就
unconfirm", 2026-08-19). `delivery_overdue` (pure, tested) skips a window whose
derived state is `running` or `waiting` — both mean the queue is holding the line,
the second because a permission prompt suspends the turn — and otherwise measures
from the LATER of the typing time and the turn's end, so the CLI gets the full
window from the moment it could actually submit. Two cases must keep their old
behaviour, and both are tests: a line typed at an IDLE agent keeps its own clock
(it had its chance), and a window with no hook facts at all is still swept (it can
never ack, and pane repaints must not keep a line pending forever). Client side the
hollow ring now says what it means — "a busy agent queues it and accepts it when
its turn ends" — because hollow never meant failed.

**The queue is DURABLE, because the agent outlives our restart.** Everything else
in `Rec` is an observation, and an observation we lost is one the next hook
re-establishes. A pending delivery is not: it is a promise made to a client ("this
line was typed into the pane; its receipt is coming"), and the party that will keep
it is a separate process our restart does not touch — the agent holds our line in
its own input queue and submits it whenever its turn ends, which can be after we
came back. With the queue in process memory only, that echo arrived with nothing to
match: filed as a prompt the human typed at the keyboard (`via: local`, its own
input row) while the message kept its hollow ring for ever — the same symptom as
the single-slot bug, from a different cause (owner, board #5, 2026-08-29: "发送了
一条消息，然后后端的服务有重启了，然后agent又收到指令确认hooks，这个hooks没有正确把
之前的未确认的消息变成已读状态，被单独写出来了"). So `record_delivery` mirrors the
line into `deliveries` (state.db v13, keyed by the line so a re-typed line upserts
exactly as the memory queue replaces it), and `hydrate` folds a window's rows back
in lazily — once per window per process, at the echo path (`record_prompt`) and at
the sweep, which is the one that has to see queues belonging to windows this
process never heard from. Four rules keep the recovery honest: memory WINS on a
line it already holds (a line typed by this process keeps its own clock), a
recovered line's ack clock starts at the recovery (nothing was listening for its
echo while we were down, so sweeping it on the first feed read after a restart
would warn seconds before the echo we are trying to catch), a settled line is
deleted — acked or reported alike, so a restart cannot re-warn about it — and a
window that no longer exists has its whole queue dropped rather than left for a
recycled window index to inherit. Lines older than 24 h are pruned once per
process: the agent that would have echoed them is long gone. Whitespace-blind
containment and the multi-line-per-prompt match are untouched — the recovery hands
the same `pending` vector to the same matcher, which is what the regression tests
assert (record → drop every in-process record → echo → `via: app`), including one
through the real hook path in `agent_notifications.rs`. The table is also created
IDEMPOTENTLY on every open (`Store::heal`), not only in the v13 step: a binary
built in the seconds between the version bump and its migration block stamps the
database at v13 without the table and every later build then skips the step for
ever — measured on the dev host while this was being written.

Delivery reaches MANAGED windows only. `projects::managed_home` /
`is_managed_in` is the one definition of "an agent this app created" — the
isolated home `spawn` materialized, never the window name — and three gates
share it so they cannot drift: who counts as a chat participant (`hub_agents`),
whose stop-hook reply gets posted (`maybe_auto_post`), and whose pane we may
type into (`deliver_mentions`). Without the third one, `@all` would inject a
chat line into a kiro the user started by hand in that directory.

## A slash command is for the CLI, not for the model

`/model`, `/clear`, `/compact`, `/tools` are interpreted by the agent's own TUI,
and only when they are the whole line. Sent the normal way they are not commands
at all: `deliver_mentions` types `[tmm chat 16:38] human: @builder-2 /model` into
the pane, the CLI sees prose, and the model answers a question about slash
commands instead of the CLI running one (owner, 2026-08-19: "支持 /命令 这个直接
发送 不加消息时间戳之类的").

So the composer routes them through `hub_command { session, agent, text }`, which
types the text VERBATIM — no stamp, no sender, no @address — and answers with the
windows it reached. Four rules:

- **A path is not a command.** `slashCommand()` (`hub.ts`, pure and tested)
  requires the first token to be `/word` with no second slash, so `/model
  claude-opus-5` is a command while `/tmp/foo` and `/usr/bin/env node` stay
  messages. Without that discriminator a one-line path would be typed into an
  agent's pane as a command.
- **It needs a target**, an explicit leading `@name` or the composer's recipient
  (`@all` → every managed agent). With neither — a room note, which reaches nobody
  live — it stays an ordinary message rather than vanishing.
- **Managed windows only**, the same gate as delivery and auto-post. Typing into a
  window the user started by hand is not ours to do, and `/clear` typed into a
  SHELL would run `/clear` as a path.
- **The room records it as a lifecycle line** (`[tmm] /help → builder-2`), not as a
  message: it is an instruction to a program, so it renders as a folded `sys` row
  and disappears at the chat-only level. Recording it as a message would also feed
  it back to the mention scanner.

### Completion: two stages, one palette

Typing `/` in the composer offers the agent CLI's commands; choosing one that
takes an argument offers ITS values next ("比如我打/ 就会出现compact之类的让我选，
还有model 如果支持两个参数的，可以多次选择", 2026-08-19). `commandPalette()`
(`hub.ts`, pure and tested) is the whole decision: which stage, which items, and
which slice of the input a chosen item replaces.

`KIRO_COMMANDS` is TRANSCRIBED from kiro-cli's own TUI table — the same names,
the same descriptions, the same sub-commands — not invented. A made-up command is
worse than no completion at all: it looks authoritative in the list and then does
nothing in the pane. Cloud-only and hidden entries are left out, and `/quit` sits
last because it ends the agent. `/model`'s values are the exception that cannot be
transcribed: they are fetched through `models_list`, the same server call the agent
editor uses (cached ten minutes, asked at most once per backend per Hub visit).

**The palette speaks the ADDRESSEE's dialect** (2026-08-22): each CLI has its
own command table, so `offeredCommands(backend)` picks it by the explicit
`@name`'s backend, else the composer's recipient — `GROK_COMMANDS` transcribed
from grok 1.0.5's own docs (its `/model` completes inline and grok models ARE
enumerable, so the dynamic values work; `/resume`, `/dashboard`, `/mcps` are
pickers/modals and carry `view: true`), `CODEX_COMMANDS` transcribed live from
codex-cli 0.148.0's `/` popup (there `/model`, `/permissions`, `/review` are
PICKERS — views — while `/compact`, `/diff`, `/status`, `/mcp` act in the
pane; `/delete` stays in the table flagged as destructive and is never
offered). claude gets NO palette: the CLI is not installed on this machine, so
its table cannot be transcribed, and `@all` over a mixed roster gets none
either — one command line cannot be right in two dialects at once.

**Only commands that DO something are offered.** kiro marks the rest
`inputType: "panel"` — a list or table that takes over the pane and needs a key to
dismiss — and a few others open `$EDITOR` or a recorder. Sending one from the chat
parks the agent inside something nobody here can see, so `/tools`, `/help`,
`/mcp`, `/context`, `/code`, `/hooks`, `/knowledge`, `/memories`, `/tangent`,
`/rewind`, `/goal`, `/workflow`, `/prompts`, `/feedback`, `/upgrade-agent`,
`/usage`, `/reply` and `/voice` carry `view: true`: they stay in the table, with
the reason, and `OFFERED_COMMANDS` filters them out (owner, 2026-08-19: "有一些命令
输入后是交互式查看的 这种就先去掉吧 以后再想办法支持"). Re-enabling one is deleting
a flag, once there is a way to show a panel — the terminal drawer is the obvious
candidate. Ten remain: `/model`, `/compact`, `/clear`, `/effort`, `/agent swap`,
`/chat`, `/spec`, `/plan`, `/paste`, `/quit`. `/agent` offers only `swap` for the
same reason: `create` and `edit` open an editor in the pane.

Only the LAST token is completed, and only the FIRST argument: these commands take
one, and what follows it — a path, a prompt, a free-text name — is not ours to
guess, so a filled argument closes the palette instead of re-offering the same
list. The match is FUZZY, in three strict tiers — prefix, then substring, then
subsequence (`fuzzyRank`, pure + tested; stable table order within a tier) —
because demanding the first characters made the palette useless exactly where it
matters most: the discriminating part of a model id like `claude-sonnet-4.5`
never comes first ("不一定从第一个字符开始匹配，可以模糊匹配", owner 2026-08-24).
The tiers are the safety: Enter accepts the TOP item, so `/co` must keep meaning
`/compact` even while `/mdl` finds `/model` — a looser match may join the list
but never outrank a tighter one. The palette owns ArrowUp/Down, Tab and Enter
while it is open (a menu that
ignores the keyboard is a menu you have to reach for the mouse to use), Escape
dismisses it until the text changes, and hover shares ONE highlight with the
keyboard cursor because two would read as two selections.

**The composer speaks readline.** Ctrl-A/E (line start/end), Ctrl-U/K/W (kill to
line start / to line end / previous word), Ctrl-Y (yank), Ctrl-D/H
(delete/backspace), Ctrl-T (transpose), Ctrl-F/B (move) — the fingers that live in
this app already know these from every shell, macOS text views honour half the set
natively, and the half macOS lacks (U, W, Y) is the half a Linux browser lacks
entirely ("其中有一些 mac 系统好像就已经支持了 … 适用性更好一些", owner
2026-08-20). Implemented OURSELVES on every platform so none of them drift:
`readlineEdit()` (pure + tested, `hub.ts`) owns the arithmetic — kill semantics are
readline's (one buffer; consecutive kills accumulate, backward kills prepend so
`Ctrl-W Ctrl-W Ctrl-Y` restores word order; any other key breaks the chain;
Ctrl-D deletes without saving), A/E move within the LINE of a multi-line draft,
Ctrl-K at a line's end eats the newline (join), and an empty-buffer Ctrl-Y is a
handled no-op because Chromium's default Ctrl-Y is REDO. Ctrl-C/V/X/Z return null
and stay the browser's. The component keeps only the buffer and the chain flag;
the caret is set after Svelte writes the value back, or the update would reset it
to the text's end.

**The composer shows what will be RUN.** While the draft parses as a command with
a target — the same `slashCommand` + recipient branch `send()` takes, so the look
can never promise a command that sends as prose — the capsule takes an accent tint
and the text flips to the tool lane's monospace ("如果是指令的话在输入框里样式改变
一下", owner 2026-08-20): `/tmp/foo` stays proportional, `/model` with an empty
recipient stays proportional, `@builder-2 /compact` turns mono. The measuring
mirror flips WITH the input (one CSS rule covers both) — `growComposer` re-lays-out
the text to find the last line, and measuring mono text in a proportional font
misplaces the send button's collision zone — and the height re-measures on the
flip itself, since a font change rewraps.

## Vitals: reading the agent's own status line

An agent CLI already publishes its live state at the bottom of its pane — the
model, the share of context used, the reasoning effort, the cwd and branch. We
were showing none of it and asking the owner to go read the terminal ("从输出的最后
几行原始文本内容 加一下当前状态的嗅探，比如模型名 上下文长度 effort 之类的",
2026-08-19). There is no API for it: a CLI's live state lives in its own process.

So `projects::vitals` reads the last lines of the pane, and
`hub_agents` attaches a `vitals` object per agent. It is a SNIFFER, with a
sniffer's contract: every field is optional, an unreadable pane yields the empty
reading and never an error, the object is omitted entirely when nothing could be
read, and nothing downstream may assume a value is present. It sits on top of the
hook-derived state, never in place of it — hooks are facts, this is a reading of
somebody else's screen. Each backend is its own dialect: `sniff_remembered`
dispatches on the backend `agents::detect` reported — `sniff_kiro` for kiro's
`·`-joined status line, `sniff_grok` for grok's header ratio + boxed footer
(see the grok backend notes above) — because reading one CLI's screen with
another CLI's grammar yields confident nonsense, which is worse than nothing.

What makes it more than guesswork is that kiro's layout is documented by kiro's
own source: the status line is a fixed order of segments joined by `·` — `agent ·
autonomous · model · effort · context · tangent · codeIntel · goal` on the left,
`location · branch` on the right — and the context segment is defined there as
"Share of the context used" (a pie glyph plus `N%`, or `N% ctx` in lite mode). Four
rules follow from reading a screen rather than an API:

- **The agent's own name is the anchor.** The model is positional (it follows the
  name, and the optional `Autonomous` flag), so a line that does not start with
  this window's name contributes no model. Without that anchor a cwd, a tangent
  name or a neighbour's status line becomes "the model this agent is running".
- **Fields that can identify themselves do — and effort cannot, so it is
  anchored.** Context and branch are found by shape wherever they sit, because a
  narrow pane wraps the right-hand segments onto their own lines. Effort has no
  shape of its own — it is a bare word (`low`…`max`), and reading it wherever it
  sat turned ordinary output (a table cell, a priority column) into a confident
  effort reading (owner, 2026-08-26: "effort 显示好像有的显示不对") — so it is
  matched by the status line's own PATTERN: on the line anchored by the agent's
  name, the segment immediately BEFORE the context segment (`… · model ·
  effort · ◔ N%` — owner, same day: "你要按照这样的模式去匹配 不要直接全文匹
  配"). And absence there is a VERDICT, not a miss: kiro omits the
  segment entirely when the effort is the backend default ("effort 这个参数不
  是百分之百都会显示的"), so when the anchored line carries its context segment
  (the full left side is on screen) `effort_definitive` is set and backfill may
  not resurrect a remembered value — without that, a stale or once-misread
  effort was re-inserted with a fresh timestamp on every poll, a permanent ghost.
  The WIDE pane is the mirror image of the narrow one: with room to spare, kiro
  right-aligns `location · branch` on the SAME line, joined to the left segments
  by a padding run of spaces rather than a `·` — so the context segment arrived
  glued to the cwd (`◔ 5%       /local/home/cfu/temp`) and the percent was
  refused (owner, 2026-08-26: "上下文长度也没嗅探出来"). A run of two or more
  spaces never occurs inside a segment, so it is treated as a segment boundary
  too.
- **A bare `69%` is never the context.** Only a percentage carrying the pie glyph
  or the `ctx` suffix counts. A terminal is full of percentages, and a confident
  wrong number on the card is worse than a missing one.
- **The newest paint wins**, so the tail is read bottom-up: a pane keeps every
  earlier status line in its scrollback.

Managed agents only (their status line's shape is one we materialized), one
`capture-pane` per agent, capped at four per project.

**A miss is normal, so the reading REMEMBERS.** Sniffing looks at somebody else's
screen at an arbitrary instant: the pane may be mid-repaint, a tool's output may
have pushed the status line up, a panel may be open. Treating each miss as "no
information" made the card blink empty (owner, 2026-08-19: "context window 和模型状
态信息，有时候会闪没了，是不是中间心跳失败了？我觉得对于心跳的状态可以多维持缓存一会
儿"). `sniff_remembered` keeps the last good reading per (session, window) and fills
gaps FIELD BY FIELD — a pane often shows the wrapped branch line while the status
line itself has scrolled off, and half a reading is still half a reading. A fresh
value always wins, so a `/model` swap shows up at once; the memory expires after
`VITALS_TTL_SECS` (1 h) and `retain_windows` drops it with the window, so a new
agent cannot inherit the last one's numbers. The per-backend sniffers stay pure and
tested; the
memory is a thin layer over it.

And polling is no longer the only thing that FILLS the memory. The poll reads
whatever the pane happens to show at poll time, so a fresh page often showed
nothing until some later poll caught the status line (owner, 2026-08-25:
"在每次有消息发送的或者 hooks 的时候嗅探上下文用量，并且服务端记录这个状态，
客户端随时能获取到，现在经常看到没有信息，过了一会儿才出来").
`vitals::sniff_window_soon` sniffs at the EVENTS that make a pane fresh — every
hook edge (`userPromptSubmit`, a tool row, stop/ask) and every chat line
`deliver_mentions` types — delayed ~1.2 s because the TUI needs a beat to repaint
its footer, throttled to one capture per window per 3 s so a burst of tool hooks
costs one read, on a throwaway thread because telemetry may never block what it
observes, and behind the same managed-only gate as `hub_agents` (a no-op under
`cfg(test)`). The reading lands in the same memory the poll answers from, so the
client sees the last known state immediately instead of waiting for a lucky
capture.

The owner's other half was right too, and it was a client bug: `loadAgents` did
`catch { agents = [] }`, so a dropped socket or one timed-out RPC emptied the whole
roster — cards, readings and all — until the next successful poll. "I could not ask"
is not "there is nobody": the roster is a last-known state now, and `selectProject`
is the one place that clears it, because that is the one time it is genuinely
unknown.

**On the card, permanently, and the percentage is a line rather than a number**
("这个直接常驻显示吧 可以字号小一点 百分比用一个细长会变颜色的进度条示意 一个细横线
就行", 2026-08-19). Each roster card carries the model and effort on a second line
at the smallest step of the type scale (`--fs-micro`, monospace so an id or a
percentage does not reflow as it changes), and the context usage is a 2px line at
the card's own bottom edge — absolute, so it costs no height and cannot make the
roster taller as agents appear. The branch and cwd stay in the tooltip and the menu
header: they are the same for every agent in a project, so on a card they would be
chrome rather than data, and the exact number belongs where a number is what you
came for.

`ctxColor()` (pure, tested) returns a THEME EXPRESSION, not a colour: every stop is
one of the app's four status tokens and the ramp is a `color-mix` between them, so
it is correct in both themes and follows the palette if it changes. The two anchors
are kiro's own — green until 20%, amber by 60% (its warning threshold) — and past
that it continues into `hot` and `danger`, because a context above 85% is about to
force a compact, which is a thing to see coming rather than discover.

### The composer's draft is part of the project

An unsent line belongs to the conversation it was being written for. It used to be
one box shared by every project, so switching projects carried a half-typed line in
front of the wrong agents, and a reload threw it away ("前端消息框的消息应该和项目
绑定，比如我切换项目了，但是消息框没有切换，这里内容最好是浏览器有缓存的，正在输入
的内容刷新也还在", 2026-08-19).

`hubPrefs.draft/setDraft` keys the text by tmux session next to the lead and the
read marker (so `renameSession` moves it too, or a rename would silently eat what
you were typing). `selectProject` parks the outgoing draft and loads the incoming
one; the write happens on every keystroke, because one small JSON string is cheap
and a debounce loses the last characters exactly when the tab goes away.
`draftUpdate()` (pure, tested) holds the two rules that would regress in silence:
an empty draft REMOVES its key — otherwise every project ever visited leaves a row
behind for good — and the text is capped at `DRAFT_MAX`, because a draft is a
convenience, not a document, and a pasted file would fill localStorage and take the
rest of the Hub's preferences down with it.

## Messages are not deletable in the UI (the archive was tried, and retired)

The room is the record. A two-step archive existed for two days (2026-08-19 →
21): tap a message → Archive hid it (`hub_msg_archive`, restore free), a bar
above the feed opened the archive view, and deleting it THERE (`hub_msg_purge`)
was the confirmed, irreversible step. The owner then asked what it was for and
had it removed ("没有消息删除 不需要这个功能，彻底去掉吧", 2026-08-21): the
extra Archive verb on every message read as clutter, and once projects got
their own recycle bin the message-level one had no job left. A tapped message
now offers exactly Copy and Raw.

What remains, deliberately:

- **The server API is intact** — `hub_msg_archive` / `hub_msg_restore` /
  `hub_msg_purge` / `hub_archive`, the `msg_archive` table (state.db v10, with
  its message snapshot), and `hub_log`'s filter against
  `projects::archived_ids`. Hiding a message is still something the API can do;
  this build's client just never calls it. Purge still reaches team.db only
  through the JSON-only `TeamBridge::delete_messages`, deleting messages FIRST
  and dropping archive rows after, so a failure stays retryable.
- Anything archived by an older client stays hidden from `hub_log` — retiring
  the UI must not resurface what someone chose to hide.

## Right-click, and the phone's long press

"还有很多地方增加右键点击操作，和手机长按" (owner, 2026-08-20). Both are the same
gesture — "tell me what I can do to THIS" — so they share one mechanism rather than
growing a menu per surface:

- `ui/ContextMenu.svelte` is the menu. Same popover dialect as the agent dot menu
  and the shared `Select` (a fixed layer placed by `menuPlacement`, dismissed by an
  outside pointerdown / Escape / any ancestor scroll / a resize, hover and the
  keyboard cursor sharing ONE highlight), because a second menu language would read
  as a second kind of menu. It is anchored on a POINT: `pointAnchor(x, y)` makes the
  pointer a zero-size rect so the existing placement rule — right edge on the
  anchor, flip above when there is no room, clamp to the viewport — applies
  unchanged. All of these layers assume the VIEWPORT is their containing block,
  which is a rule about everything ABOVE them: a permanent `will-change: transform`
  on the App's `.page` made each page a containing block for its fixed descendants,
  and on desktop — where the rail pads `.page` 46px right — every popover rendered
  46px right of its math ("桌面版…下拉框整体往右偏了", owner 2026-08-25; the
  phone's `.page` starts at x=0, so it never showed there). The hint now applies
  only during the 120ms tab slide, when no popover can be open — and with fixed
  sheets anchored to the true viewport, one that must clear the status bar pads
  with `var(--sat)` (the APK's MainActivity-fed inset), never raw `env()`, which
  reads 0 there.
- `ui/longpress.ts` measures the hold for NON-TEXT subjects. Touch only: a
  mouse already has a right button, and treating a held left button as a press
  would fire a menu in the middle of a text selection. Three rules keep it
  from making a list feel broken — a hold that travels more than 10px is a
  SCROLL (`isScroll`, pure and tested), a second finger cancels it, and the
  click that follows the release is swallowed once so a row does not both
  open its menu and activate itself. Android does emit `contextmenu` for a
  long-press on selectable text; message bubbles therefore use
  `touchContextMenu(pointerType)` at their native handler instead: touch/pen
  return BEFORE `preventDefault` and belong to system selection, while mouse
  right-click and the keyboard menu key keep the app menu (board #48).

Wired to the agent card and sidebar project row, each offering the verbs it
already has elsewhere — a context menu with its own action set is a second
source of truth waiting to disagree. The message bubble is the deliberate
text exception: mouse right-click offers Copy / Raw, but touch/pen long-press
is native selection and a non-collapsed selection also swallows the following
click so no action row appears. `.m-body` explicitly enables text selection;
its head/meta stay chrome. Board notes intercept no contextmenu at all, keep
selectable `.n-text`, and use the same selection-before-tap rule for their Copy
overlay.

## Who a message goes to

Three ways a message can land, and they are NOT shades of one thing:

| recipient | what happens | cost |
|---|---|---|
| a name (the default: the lead) | typed into that agent's input | one agent starts a turn |
| `@all` | typed into EVERY managed agent's input | every agent starts a turn at once |
| nobody | recorded in the room, delivered to no pane | nobody is interrupted; agents see it at their next `tmm log` |

The third one used to be labelled "everyone", which was backwards — it is the
one that reaches nobody live. The composer now names all three for what they
cost, and only broadcast wears a warning colour.

A room has ONE default recipient, so talking to your lead agent costs no `@`.
`pickLead()` (client, pure) resolves it: the remembered choice for this project
while that agent is still present → the only managed agent → one whose registry
definition `can_hire` (that IS the lead role) → lowest window index. Choosing a
recipient IS choosing the project's lead, so it persists per session.
`addressed()` prefixes `@name` on send and leaves any hand-written `@` alone;
an empty recipient posts to the room. An empty room offers a preset start
instead of a composer: tap one agent to begin, or pick several as a team — each
is the same `hub_spawn`, and the lead of the new roster follows the same rule.
"Empty" is a VERDICT, not a default. `selectProject` used to clear the arrays
and let the panel judge them while `hub_log` was still in flight, so every
switch flashed the "add an agent" pitch in front of rooms full of history
(owner, 2026-08-25: "先看到添加 agent 一个 agent list那个页面闪了一下，然后再
出来消息"). Two rules end that: a visited room is PARKED in an in-memory
per-session cache (`roomCache` — feed, activity, roster, cursors) and restored
whole on return, with the pollers merging on top (the cached `lastTs` makes the
refresh incremental) and the cached roster seating the recipient at once via
the same `pickLead`; and a room with no cache entry renders nothing at all
until its first `hub_log` answer flips `roomReady` — only then may the feed
call itself empty. Entering a room lands at its tail either way: a parked
scrollTop from the last room would point at arbitrary content in this one.

## Stopping and starting one agent

`hub_agent_stop` / `hub_agent_restart` (managed-only, same `managed_home` gate)
act on the tmux WINDOW, because a window is the agent's life. Stop kills it and
keeps the declaration; the isolated home and the conversation id stay on disk, so
the agent has not left the project — the roster shows it greyed with a start
action (`stoppedAgents`, computed from the slots `project_list` already returns).

Restart is kill + `projects::up`, which is deliberate reuse: `up` matches windows
BY NAME, creates only what is missing, and prefers `--resume-id` / `--resume` /
`codex resume <id>`, so the agent comes back to its own conversation instead of a
blank prompt. A window younger than the capture loop's 120 s rule may not be in
the declaration yet, so there is a fallback to a fresh `spawn` — a new
conversation, but better than an agent that does not come back. The reply carries
`resumed` so the caller can tell which happened.

Restart also works when nothing is running, and that is the only way the UI uses
it: a running agent gets ONE control (stop), and a stopped chip gets "start
again" (owner call — a restart button on a running agent just combines two steps
nobody asked to combine). Stopping asks first, because the process may be
mid-task and on a phone the button is a thumb away from the chip you meant to
tap; starting something that is not running destroys nothing, so it just
happens. Both post a `[tmm] stopped <name>` / `[tmm] restarted <name>` line,
because the room is the record.

## Spawn: the starter pistol

An agent CLI boots into an interactive prompt and does nothing until spoken
to. For a while we exploited that with a synthetic KICK passed as the CLI's
positional arg — first an instruction ("Start now: read your instructions…"),
then a bare `(session start)` marker. Both are gone, and the rule now is:

**Nothing is sent unless there is something to consume.** A spawn with no
brief passes NO positional arg at all: the agent opens and waits, costing
nothing, until a real message arrives. Two reasons, and the second is the one
that killed the marker: that channel is where the OPERATOR's words land (the
prompt echo renders in the chat, so an invented line reads as something the
user typed), and an agent handed a contentless prompt starts reasoning about
nothing — "多此一举" (owner, 2026-08-18).

A brief IS something to consume: `tmm spawn <agent> --brief "…"` delivers it
as the first prompt, stamped `[YYYY-MM-DD HH:MM]` like every later message
(`first_prompt()`). The stamp is also how an agent learns the wall time —
`build_prompt` says so, because a system prompt cannot carry a date (it is
replayed on every restart, so a baked-in "today" ages into a lie). The system
prompt also tells a brief-less agent explicitly to WAIT.

A side effect worth keeping: `write_launch_recipe` no longer has to strip a
trailing quoted argument to keep the kick out of the restart line — the
command it is handed never contains a first prompt, so it stores it verbatim
(the old strip was a guess that would have eaten a legitimate quoted flag).
Client side, `isSessionStart()` still filters the OLD kicks out of persisted
rooms.

Measured after the change: a brief-less kiro agent's launch line ends at
`--trust-all-tools` and its pane sits idle with no turn at all; the same agent
answered immediately when a real `tmm send` arrived; a briefed agent received
`[2026-08-18 17:10] 回一句：收到 brief` as its first prompt, answered, and
called `tmm done` itself.

## The activity feed (telemetry in the chat timeline)

The chat shows what agents SAID; around that, the Hub weaves in what we
OBSERVED. Mechanically it is `telemetry::recent_events`, fed by the same
recorders that drive status derivation and exposed as
`hub_activity { session, since_ts }` with ms timestamps so the client merges it
directly into the message timeline.

**It is DURABLE.** It used to be an in-memory ring (120/session) that died with
the server, which meant a restart erased every tool lane in the conversation while
the messages around them survived — a feed with holes in it (owner, 2026-08-19:
"后台的工具调用 status之类的是不是没有持久化，好像重启就没了"). The `activity`
table (state.db v9) is now the record, and the ring is what is left when there is
no database. Three rules keep an observer from becoming a burden:

- **Writing is FAIL-SOFT, but not silent.** `push` inserts and does not block on
  the outcome. Telemetry may never break the thing it observes, so a locked or
  read-only database costs you the history, not the tool call — but a lost write is
  a hole in the very thing that is supposed to be complete, so the first failure
  and every hundredth after it are reported on the server log.
- **Reading takes the TAIL, and it PAGES.** A first load (`since_ts` 0) returns the
  newest `LOAD_EVENTS` rows, oldest-first, because what the user is looking at is
  the end of the conversation; a cursor read returns everything after it; and
  `before_ts` + `before_id` returns the page strictly OLDER than that cursor, which
  is how a client reaches history it never loaded.
- **It does NOT prune itself.** See below.

Both halves are OFF under `cfg(test)`: unit tests exercise the ring and the derive
rules, and the SQL is tested directly in `store.rs`, so `cargo test` cannot write
rows for invented sessions into the developer's real state.db.

### The trace is COMPLETE; the READ is what is bounded (board #9)

The owner's ask was two-sided: the trace must be complete enough to analyse
afterwards, and the phone must not pay for that completeness on every project
switch ("后端：要记录完整，性能完好。前端：要显示通信流畅，同时不要过多地占用前端和
网络通信的资源", 2026-08-29). Those pull in opposite directions only if the same
number bounds both, which is what the old design did.

**The storage audit that started it**, measured on the dev host:

| what | where | complete? |
|---|---|---|
| chat messages (human, agents, `[tmm]` lifecycle lines, status notes, done summaries) | `team.db` `messages`, indexed `(room, seq)` | **YES.** Nothing prunes it — 1483 messages / 18 rooms / 10 days at the time of the audit, 414 KB of bodies. The only removals are explicit: `hub_msg_purge` and the admin `clear_room` |
| observed telemetry (tool calls, prompts + receipts, notifications, warns) | `state.db` `activity`, indexed `(session, ts)` | **NO, until this change** — 5309 rows, the busiest session 4046 of them, against a 2000-row-per-session prune |
| outstanding deliveries | `state.db` `deliveries` | YES (board #5) |
| derived agent state, vitals readings, the recovery tracker | process memory | By design — each is a CURRENT reading that the next hook re-establishes, not history |

Two truncations remain deliberate and are worth knowing when analysing: a prompt
event keeps its first 1024 characters and a tool event 2048 of its argument. The
full text of what an agent SAID is never truncated — that is a message.

**Retention is GONE, not configurable-and-defaulted-off.** `KEEP_EVENTS` was 2000
per session, pruned every 256 inserts; the busiest session already held 4046 rows,
so the cap stood to delete the older HALF of it, and the only reason it had not is
that the prune counter is per process and every restart postponed it — an accident,
not a policy. Worse, a deleted row is indistinguishable from an event that never
happened, which is exactly what makes a trace unanalysable. So the recorder prunes
nothing at all, and a `telemetry.rs` source test stands guard over that (no prune
call in the write path, no private env knob). If a retention is ever wanted it
belongs in `Config` with a documented key and a doc entry, like every other
operational setting; `Store::prune_activity` survives as the primitive it would
call.

**The cost moved to the read, where it can be bounded without destroying
anything.** Both feeds page backwards, and both keep their old newest-page
semantics when the new parameters are absent, so an older client is unaffected:

- `hub_log` pages on the bus's own `seq` (a message's log position — stable,
  gapless per log, already on every message the client holds; a millisecond
  timestamp is none of those, since two messages can share one). The 1000 in
  `hub_log` is a PER-PAGE cap, never a history horizon: `history_before` in the bus
  query layer takes the cursor into SQL (`WHERE room=? AND seq < ?` over the
  `(room, seq)` index), so a room of any size is walkable page by page. Pinned by a
  test that appends 1200 messages — past that ceiling — and reassembles the whole
  conversation from 12 pages of 100, asserting every message came back exactly once
  and in order.
- Exact lookups no longer live inside a page either: `message_by_id` (indexed, id
  is UNIQUE) replaced the archive path's scan of `history(room, 1000)`, which made
  anything older than the newest 1000 unarchivable. Now that a client can scroll to
  any message, "the newest 1000" is not a place where correctness may live.
- `hub_activity` pages on `(ts, id)`. The id is load-bearing: a busy turn writes
  several events inside one millisecond, so a ts-only cursor either skips them or
  loops on them for ever. The server hands the exact pair back as `oldest`. Same
  volume proof: 2500 rows at 5 per millisecond, walked in 10 pages of 250, every
  row exactly once.
- `has_more` is measured, not guessed — each store call asks for one row more than
  the caller wanted. Without it a client cannot tell "you have everything" from
  "your page ended exactly at the limit".
- **A page that loses every row still hands back a cursor.** `oldest_seq` prefers a
  surviving message's seq (what the user can actually see) and falls back to the
  RAW page's oldest position, because the archive filter and `since_ts` can empty a
  page completely: `has_more: true` with no cursor is a walk that stops dead at a
  hidden stretch, with the older visible messages behind it unreachable. Pinned by
  a test that archives a whole page's worth in the middle of a room and keeps
  walking to the older visible ones.
- Limits are CAPPED server-side (`MAX_PAGE_EVENTS`, and 1000 for messages), so one
  RPC can never become a multi-megabyte frame however loudly a client asks.
- The delivery sweep runs on the LIVE page only. Walking back through history must
  not make the app re-warn about deliveries it already reported.
- Cross-source merge is untouched: both feeds still come back oldest-first with ms
  timestamps, and the two cursors are independent, so a client may poll the tail
  (`since_ts`) while paging backwards.

Verified live against the real databases: walking the whole activity log of the
busiest session in pages of 200 returned 22 pages and exactly 4202 distinct
events — the server's own total — with no duplicate and no gap, which is the
property a scroll-to-load depends on; and paging `hub_log` by 30 gave contiguous
`seq` ranges with zero overlap. The payload difference is the point: the current
first load is ~96 KB of messages + ~254 KB of events, while a 30-message + 40-event
first page is ~28 KB + ~19 KB.

A tool row keeps up to `MAX_TOOL_DETAIL_CHARS` (2 KB) of its argument. It kept 80,
which is shorter than the paths this app's own agents work with: every row ended in
`…` after a third of a line with the rest of a wide screen blank beside it, and the
argument is the half of a tool call worth reading (owner, 2026-08-20: "工具调用的参数
没有显示全，后边被压缩成 ... 了，屏幕的宽度没有有效利用"). Length costs no layout —
the lane pans — and no unbounded storage, because the log is capped by row count.

**One row per call.** Both `preToolUse` and `postToolUse` are subscribed, because a
backend may only send one of them, and they carry the same tool and the same
argument — so the lane showed every call twice, milliseconds apart. `record_tool`
collapses the pair (same window, same line, within `TOOL_DEDUPE_SECS`) rather than
the hook config dropping one, because an already-spawned agent keeps the config it
was started with: fixing it at the source would only have helped agents spawned
later.

### Opening the drawer keeps the reader's place

The terminal drawer regrids the columns: the feed narrows, every message rewraps to
a new height, and the same `scrollTop` now points at different content — the
message being read drifts away ("点击右侧 terminal 按钮后，当前消息变窄，导致当前消
息位置漂移", owner 2026-08-20). The browser's own fix, scroll anchoring, is off on
purpose (`overflow-anchor: none` is what ended the held-ask blink), so
`withReadingAnchor` does it by hand: remember the topmost visible block and its
offset from the feed's top edge, mutate, and after the DOM settles put the SAME
element back at the SAME offset. Identity is the DOM node itself (Svelte's keyed
each preserves it); sticky variants (`held`/`ask-top`/`ask-bottom`) are skipped as
references because a pinned rect does not move with the flow, so anchoring to one
restores nothing. At the tail it just stays at the tail — that is what "where I
was" means there. Both drawer paths (open, and the one `closeDrawer` behind the
toggle, the Esc key and the ✕) go through it; the source test counts the bare
`termOpen =` writes so a new trigger cannot bypass the anchor unnoticed.

### The lane is three columns, and the middle one is the only scroller

The tool name is fixed left, the time is fixed right, and the argument between them
is the only thing that moves ("时间的信息应该固定保持在最右侧，工具明固定保持在最左
侧，相当于三列，中间参数是可以左右滑动查看的", owner 2026-08-20). In the MARKUP:
`.st-scroll` wraps the text, and the name and time are flex siblings BESIDE it —

```
.step (flex)
├── .tname        flex: none
├── .st-scroll    flex: 1; min-width: 0; overflow-x: auto   ← the only scroller
│   └── .st-text  nowrap, no ellipsis
└── .st-ts        flex: none
```

That structure is the whole guarantee: the panning text is clipped by its own box,
so it *cannot* show through the name or slide past the lane's edge. The first build
tried the other shape — one scroller for the whole lane with the name and time as
`position: sticky` layers painted over it — and failed three times in a row: the
columns jumped 30px on the first pan (sticky offsets are measured from the
scrollport's padding box, not the border), then were 97% transparent (`--surface`
is a 3% wash, unusable for anything that must COVER something), and still leaked
into the lane's padding beside the name, because a sticky column covers its own box
and never the area next to it ("参数穿模到工具名左侧了"). Structure beats paint;
`hub/Hub.source.test.ts` pins the markup shape and the CSS that keeps it.

Costs accepted knowingly: rows pan independently (acceptable — each argument is
read on its own), and the middle cell hides its scrollbar (a lane of rows each drawing a
ruler; the cut-off text is the affordance). `min-width: 0` on the scroller is
load-bearing — without it a flex child refuses to shrink below its content and the
time is pushed off the row. `.st-text` stays `nowrap` and must never become `pre`:
a tool detail routinely contains real newlines, and `pre` would turn one call into
a three-line row, breaking both "one row per call" and the row cap, whose height
is single-line math. The lane's `--lane-indent`/`--lane-pad-r`/`--lane-bg` stay on
`.steps` with one value each; `.s-body` is `overflow-x: hidden`.

Four event kinds: `tool`, `notif`, `prompt` (a prompt the agent accepted,
`via: app | local`) and `warn` (a line that was never echoed back). A `tmm status`
note is NOT among them — see below.

**A `tmm status` note is a MESSAGE from the agent.** The hooks bracket a turn but
say nothing about what it is FOR, and the owner's symptom was exactly that: "经常
一直在做但是没有同步状态" (2026-08-19). So the note — not the state word — is the
payload, and the form it takes is the agent speaking: `hub_status` posts
`[tmm status <state>] <note>` to the room from the AGENT ("status要用agent发送消息
的形式显示", 2026-08-19). That is not only what it looks like, it is what makes it
last: the room is the record, so a note outlives a restart the way a reply does,
and there is exactly ONE copy of it (it is no longer also an event, which would
have shown the same sentence twice).

Three things make the form safe. The post is **record-only**, so `deliver_mentions`
never runs on it — an `@name` inside a note must not type into a peer's pane, which
is invariant 2 of the hook-sourced posts. A **note-less claim posts nothing**:
`running`/`idle` is derived from observation and beats a word the agent typed, so a
bare state word would be an empty message. And the marker is deliberately not
`[tmm] `: that prefix means "the app is narrating" and folds into a grey `sys` row,
which is the treatment this note was moved out of.

**A `tmm done` SUMMARY is the same kind of thing**, and it was the worse case:
`[tmm] done — <summary>` folded into a grey `sys` row and the chat-only level drops
those entirely, so the agent's own account of what it finished vanished exactly
where a reader looks (owner, 2026-08-19: "返回的状态信息要用消息的形式展示在对话
里"). It is now `[tmm done] <summary>` from the agent. A done with NO summary stays
a lifecycle line, because nothing was said.

Client side the marker comes off (`statusNote()`, pure, tested) and the bubble is
exactly the bubble any other message gets ("status 消息的样式要和普通消息一样就
行", 2026-08-19). A first cut rendered it a notch quieter with an attention border
for `waiting`/`blocked`, which was the same mistake in a new costume — a second
visual species for the same thing is what made these read as telemetry to begin
with. The one adornment lives in the HEADER, where a name already goes: the bubble
head reads the name plus a state BADGE at the row's RIGHT edge — the bubble's
top-right corner ("放到这个消息的右侧 往右上角放", owner 2026-08-20) — a dot +
the state word in the app's state-pill dialect, coloured by `noteStateColor()`
(pure + tested) in the ONE
progressive status language the owner asked for ("不同的颜色应该是渐进式的",
2026-08-20), defined once above `stateDotColor` in hub.ts and spoken by both:
**accent = in motion** (running/working — a spinner is never green; this is
also the colour the progress row's lane bar and the tool lane's pulse use),
**green = ended well** (done), **amber = paused on a person**
(waiting/blocked), **red = failed** (the only distress signal), **grey = at
rest / a word we do not know**. `working` used to be green, which made every
busy agent look already finished; a test now pins the badge and the roster
dots to the same table so the two readers cannot fork. The live/off dots in
the sidebars are a DIFFERENT domain on purpose — green there is the power
convention ("on"), not a turn state. The badge answers the owner's two asks in
order: point at the state and colour it ("在消息框的 agent name 后面加一个箭头
指向具体的状态 … 为不同状态定义不同的色彩", 2026-08-20), then say it
UNAMBIGUOUSLY — the first cut was a literal `name → state` arrow, which read as
an addressee ("像是这个 Agent 给另外一个 working 的人发的", same day), and a
dotted pill reads as "entered this state" instead. What the note is ABOUT
belongs to the header, and the words stay an ordinary message. The raw view
still shows the stored body, marker included, because raw means exact.
`record_status` keeps the explicit claim in the status record: that is the part
only it can answer, and `derive_from` needs it for `waiting`.

The other half is the prompt, since a channel nobody is told to use stays empty.
`build_prompt` opens with the communication TOPOLOGY itself (owner, 2026-08-29:
"说明一下人和agent通信以及agent和agent之间通信的方式，让信息可以自由流动") — a
"How messages MOVE" section spelling out the five flows: what arrives INTO the
agent (every message is a stamped prompt typed into its pane, queued if
mid-turn), what leaves it AUTOMATICALLY (the captured final reply, the done
summary delivered to its briefer), an ADDRESSED `tmm send "@name …"` (types
into that pane — it interrupts, so it is for something the reader must act
on), an UNADDRESSED send (room-only, interrupts nobody, read at the next
`tmm log`), and the room's memory (`tmm log` / `tmm agent list` — an agent
only ever RECEIVES what is addressed to it; the log is how it catches up on
the rest). Then `tmm status working "<what you are doing right now>"` with
when to send one (at the start, when the work moves to a different part, when
a step runs long). That ordering is the convention: progress is ambient,
messages are addressed. NOTE: an agent
already spawned keeps the prompt it was given — see the def-drift entry in
`docs/unresolved.md` — so the new convention reaches existing windows only when
they are re-spawned.

Ordering is the whole point of a transcript, and it took three things to be
right. An event is stamped when the inbox file is CONSUMED (250 ms poll, so
close enough to when the hook fired), which makes the CONSUME order the render
order — so the listing sorts the inbox by the epoch prefix in each filename
instead of trusting `read_dir`, whose order is arbitrary. Timestamps are real
milliseconds, not `secs * 1000`, or every event in the same second would tie
while chat messages carry true millis. And the client breaks a genuine tie by
putting the observation first: a reply is what ENDS a turn, so the tool calls
that share its millisecond happened before it. Get any of the three wrong and a
turn's tool calls render after the answer they produced (owner report,
2026-08-16).

`prompt` is the input half of the transcript and the reason the hook is worth
installing twice over: text typed at the agent's own keyboard exists in NO
other channel — the room only ever held the output side.

`feedBlocks()` (`src/lib/hub/hub.ts`, pure and unit-tested) turns messages +
events into rows, and three rules shape what the user sees:

- **A receipt is not a row.** An `app`-origin prompt is the echo of a line we
  typed, so it marks that message *delivered* instead of printing the same text
  a second time. This happens at every detail level, because "did what I just
  sent arrive" is not a detail anyone opts into — same for `warn`.
- **A local prompt is a row.** Nothing else records it.
- **A tool call is a name plus an argument.** The two travel apart from the hook
  onward (`ActivityEvent.tool` / `.text`) so the client can render the name as
  the scannable column and never has to re-split a string on a space that a path
  or a shell command can contain.
- **The agent's own `tmm send/status/done/log/spawn` is not a tool row.** Its
  effect is already a row — the message, the status change, the completion — so
  showing the call that produced it would print the same event twice
  (`isSelfReport`). `tmm task`, `project`, `agent`, `skill` have no other trace
  in the chat and stay visible.
- **A finished turn is not a row.** The `completed` notification used to print
  "finished a turn" after every answer, next to the answer itself. The reply IS
  the event; the chip going idle is the state. Lifecycle rows are now only the
  ones where a human is needed (permission, input, failure).
- **Tool calls collapse per AGENT, replies do not.** A window's `tool` events
  fold into ONE group ("N tool calls") that stays open for that window's whole
  turn. What ends the run is that window's OWN rows — its reply, its local
  prompt, the (invisible) echo of a line delivered to it, a note about it — so a
  group still means *between these two replies*. Another agent's rows are a
  different lane and never break it: folding only CONSECUTIVE events turned two
  agents working at once into one group per call (w1, w2, w1, w2 …) and the feed
  read as churn (owner report, 2026-08-19). A reply is attributed to its lane by
  the `windowOf` map the Hub passes in; with no map (the pure-function default)
  a reply conservatively ends every run. Because a delivery echo is consumed as
  a receipt it renders nothing, but it is still a turn boundary — otherwise a
  new turn's calls poured into the previous turn's group.
- **A group is open by default, and its height is a SETTING.** The open
  default replaced "open only while the agent is working", which made a
  finished run need a click to read (owner, 2026-08-19). The body scrolls past
  a configurable row count — `hubPrefs.stepsRows`, a Settings stepper beside
  the feed-level control, default `STEPS_ROWS` = 5 (owner, 2026-08-24: "最大显
  示的行数应该也变成一个可配置的参数。现在默认把这个参数配置成 5 行" — it was
  10, a fixed constant). `clampStepsRows` (pure + tested) pins 3–30 on both
  the setter and whatever an old localStorage entry hands back: one or two
  rows cannot show a run's shape, and past ~30 the cap caps nothing. "Show all
  N" still lifts the cap for one group. Every call is in the DOM — the cap is
  a viewport on it, which is what keeps a live run from growing the
  conversation while the inner tail sticks to the newest call (`stickBottom`,
  released as soon as the user scrolls up inside it). The open/closed choice
  lives outside the row (`stepsChoice`, keyed by group) so a re-render cannot
  lose it.

A tool NAME is coloured by what the tool does — changes / runs / looks up /
reads, four buckets matched on substrings so `fs_write` and `Edit` land in the
same colour without an exhaustive table (`toolColor`). The name has to be split
off first: an older server shipped tool events with no `tool` field and the name
glued onto the text (`"shell tmm send …"`), which is why every name rendered
grey — the coloured column only exists when there IS a name. `toolEventParts`
normalizes both generations, and everything downstream (the colour column, the
collapsed peek, the self-report filter) reads through it.

Self-report filtering is segment-wise: agents chain the report onto one shell
line (`tmm send "…" 2>&1; tmm status working "…"`), so a command is invisible
only when EVERY `;`/`&&`/`||` segment is a `tmm` self-report — the `tmm send`
the room already shows as a message never prints again as a tool row, while
`tmm send "done" && make deploy` keeps its row because the deploy has no other
trace. A `;` inside a quoted message body fails open (the row stays).

Because a run of tool calls is now one group instead of forty rows, `+ tools` is
the default detail level, and the level is reachable from the Hub head (a chip
that cycles chat → status → tools) as well as Settings → Appearance → Chat
detail.

### Conversation visual language

#### The chat/terminal divider is draggable, and a tool group is one card

The divider between the conversation and the terminal drawer is a real
splitter: the drawer column is `var(--hub-drawer-w)` (320–900, default 520,
persisted as `tmux_hub_drawer_w`) and the chat column takes the rest with a
280px floor. It reuses `SideHandle`, which is now parametric (variable name,
storage key, bounds, and which edge it rides — a `left` handle inverts the
drag delta) rather than forked: ui-unification says there is ONE resize
affordance, and that has to survive a second consumer. It was a fixed
`0.8fr / 1.2fr` grid before, so the divider looked draggable and was not
(owner report).

A folded tool-call group is ONE card, full feed width. Two things were wrong:
the head owned its own border while the body was indented with
`margin-left + border-left`, so opening the group jogged the left edge (the
body box started at 11px, the head's text at 30px — measured); and the group
was capped at bubble width (76%), which truncated exactly the paths one opens
it to read. Now `.steps` carries the border and radius, `.s-head` is
borderless inside it, and `.s-body` is separated by a top line with
`padding-left: 30px` = the head's padding (10) + chevron (12) + gap (7), so
every row lines up under the head's TEXT rather than under its chevron.

#### The terminal drawer has ONE switcher

Opening a terminal inside the chat gives a drawer with a single bar: window
pills (state dot, name, `direct` tag for a window the app did not start) plus
the roster count and the open-full / close actions. It used to carry TWO — the
pills on top and a tmux-style statusline underneath — which listed the same
windows and called the same `pickWindow` (owner: "上面和下面有两个 bar…可以把
它们合并一下"). The pills survived because they carry the state and the
actions; the statusline's only unique content was the roster count, which
moved into the bar. The embedded `Terminal` stays `chromeless`, so its own
window-switcher never appears here either — one bar, one place to switch.
`statuslineWindows()` and its test went with the footer rather than lingering
as dead code.

#### Deleting, and CLI/UI parity

Four verbs act on one agent and four on a project, and EVERY one of them is
reachable from both the chat UI and `tmm` — the CLI is not a subset (owner:
"所有的 Agent 也可以通过 TMM 命令直接交互所有的操作"), because an agent that
can only be managed by a human cannot manage a teammate:

| Agent | `tmm agent interrupt\|stop\|restart\|remove <name>` | roster dot menu (context menu): Watch / Interrupt / Stop / Remove |
| Project | `tmm project up\|down\|archive\|delete <session>` | header: Open / Close / Delete (archive is the list's own action) |

`remove` is the eject button next to stop's pause button: it kills the window,
DROPS THE SLOT (so `up` never recreates it) and deletes the isolated home (so
`is_managed_in` stops recognising it). `delete` is archive's irreversible
sibling: it closes the session, removes every `<path>/.tmm/agents/<name>/` the
app created, and forgets the row (slots cascade). Two things both verbs leave
alone on purpose: **your files** — we only ever delete inside `.tmm/agents/` —
and **the chat history**, because the room is the record of what happened and
rooms are keyed by session name. The Board is different: it is PROJECT task
state, not the durable conversation record. Archive keeps it (restore must be
lossless); irreversible `project_delete` removes its issues + note threads
before releasing the session name, so a later project with that name cannot
inherit somebody else's tasks (board #41).

**In the chat UI, delete is a RECYCLE BIN, not destruction** (owner,
2026-08-21: "把project里删掉进入archive … 相当于回收站的功能，在archive里可以
彻底删除project" — the same two-step rule messages already follow: hide first,
destroy there). The header's Delete closes a live session and ARCHIVES the
declaration (`project_down` + `project_archive`); the project drops into a
folded "回收站 · n" section at the sidebar's bottom, which exists only while
non-empty. Restore is one tap and asks nothing (it destroys nothing — the
declaration comes back, Open rebuilds the session, agents resume their
conversations). The irreversible `project_delete` is reachable ONLY from
inside the bin, behind the one confirmation that means it. Archived projects
are never re-adopted by the capturer, which is what keeps the bin stable. The
raw CLI keeps the sharper verbs as-is: `tmm project archive|delete` map to the
same two steps.

Interrupt moved server-side (`hub_agent_interrupt`) when the CLI gained it:
one implementation types the named `Escape` key, so the button and the command
cannot drift and the managed-agent gate is enforced in the same place as
stop/restart. **The derived state is reset BEFORE the key goes in**
(`telemetry::record_interrupt` — `end = ("completed", now)`, `ask` and the
agent's explicit claim cleared, so `derive` answers `idle` at once). Two
reasons, and the order is the whole point (owner, 2026-08-29): a turn cancelled
from OUTSIDE has no edge of its own — the backend fires no stop hook for it — so
the newest fact would stay the `userPromptSubmit` that opened the turn and the
card would read `running` for as long as the agent stayed alive; and an
interrupted agent is usually given something else to do within seconds, whose
own `userPromptSubmit` re-derives `running` — a reset that raced that new turn
would be indistinguishable from no reset at all, i.e. an interrupt that looked
like it never landed. Resetting first makes the effect visible in the gap, and a
real new turn afterwards is a fact of its own. It leaves a mark too: a
successful interrupt posts
`[tmm] interrupted <name>` to the room — the sys grammar already had the word
(amber: a turn cut short, not an ending) and the owner asked to SEE the act in
the conversation ("发送 interrupt 的状态在消息列表里也要展示出来", 2026-08-24).
The composer carries the verb too, behind two beats: with the box EMPTY the
grey send button stays clickable — the first tap ARMS it (amber,
a caption pill naming the target, since a phone has no hover to read a title
from) and the second fires; double Ctrl+C on the empty composer is the same
arm/fire pair, and with text present Ctrl+C remains the browser's copy. While
the recipient is MID-TURN (hook-derived running/waiting; any managed agent for
`@all`) the resting button already says so: a stop square inside a slowly
circling arc — 2.2s, because a fast spin says "loading" while this says "a
turn is open"; accent on the resting grey, stilled under
`prefers-reduced-motion`. The armed state keeps that same glyph on the amber
ground: one object moving through hotter states, replacing the `zap` bolt that
read as nothing ("我看打断是闪电，看着好像不是那么容易理解", owner
2026-08-25). Idle and failed recipients keep the plain grey arrow — an ended
turn has nothing to interrupt. The
interrupt goes to whoever the composer is addressing — the recipient, or all
managed agents for `@all`; an unaddressed room note arms nothing, because a
room note interrupts nobody. An armed button stands down on its own: 3 s
unfired, any typing, Escape, or switching projects — an armed cancel must not
lie in wait or follow the user into another room.

#### The sidebar row is a summary

A project row answers three questions before it is tapped — is it up (the
live dot), when did it last speak, and who is in it doing what (owner,
2026-08-24: "目前显示的内容稍微有一点点过于简单了 … 上次回复的时间 … 当前几个
Agent 的简单 logo 状态"). The last-reply time comes from the SAME `hub_rooms`
map that orders the list, so the time on the row explains the row's position;
it renders through `agoShort` (pure + tested) — ONE unit, "5m"/"2h"/"3d",
because `fmtElapsed`'s two-unit "2h14m" is running-timer language and a row is
not a timer. Under the name, the project's agents appear as quiet mono chips —
11px backend logo + window name — each wearing a chat-only state dot in the
one progressive status language. The LOOK is the Terminal sidebar's, by
construction rather than by imitation (owner, same day: "应该和terminal侧边栏
一样，调整的好看一些，我觉得甚至这两个可以共用，唯一区别是terminal会多显示一
些window"): the atoms — `.side-age`, `.side-wins`, `.side-win`,
`.side-win-name`, `.side-win-dot` — live in app.css beside `.side-h` and
`.side-row`, the Terminal sidebar's dense mode swaps its scoped `.win`/`.age`
classes for the shared ones, and `ui/sidebar.source.test.ts` forbids either
component from restyling them (a scoped rule outranks app.css at 0,2,0 vs
0,1,0 — the same silent drift that once split the two section headers). A
component may only POSITION the shared containers — the Terminal keeps its
`.wins-indent` that tucks chips under the name; the Chat row nests them inside
its two-line body, so alignment is structural. The difference between the two
sidebars is exactly what the owner named: the Terminal shows every window,
the Chat shows the agents. The states for EVERY project arrive on `hub_rooms`
itself: `telemetry::all_states` is a pure-memory derive over every hook-known
window — no tmux call, no pane sniff, so the 20 s sidebar poll costs nothing
new — and a window with no hook facts is simply absent, which the client reads
as idle because that is what no facts honestly means. A LIVE row detects its
real agent windows with the window switcher's own `paneAgent`; a CLOSED row
shows its DECLARED agent slots dimmed and dotless — what `up` would restore,
not anything running now.

#### Opening and closing a project

The chat header carries exactly ONE of two buttons, whichever is true right
now: **Open** (`project_up`) when the project has no live tmux session, and
**Close** (`project_down`) when it does — the owner could not find a way to
close a project from the chat, because only Open existed. Close goes through
the same confirmation dialog as stopping an agent (it kills every pane in the
session), and the copy says what survives: the project stays in the list, Open
brings it back, and each agent resumes its own conversation.

Beside the title, the header shows the project's FULL path, not `shortPath`'s
`…/last/two` stub: it renders whole when it fits, and when the header's buttons
leave it too little room it becomes a horizontal scroller instead of
ellipsizing — the owner's three rules verbatim ("展示得下完整展示 / 展示不下再
隐藏 / 支持滑动查看，不要省略号", 2026-08-20). The scrollbar is hidden (one
quiet line), a `wheelX` action pans it with a mouse's vertical wheel (a
non-passive listener, because it has to preventDefault; it lets the wheel
through when the path fits), and the full path stays in `title`. Desktop only,
as before — the phone header never showed the path.

`project_create` also had to change: the session name follows the NAME when
one is given (`session ?? name ?? basename(path)`). It previously fell
straight through to the folder, so `tmm project create /tmp --name closetest`
made a session called `tmp` — the same folder-name-wins bug the Hub dialog hit
from the other side. An explicit `--session` still overrides.

#### The phone's back gesture peels the Hub, layer by layer

A back swipe on the chat used to reach App's do-nothing fallthrough, and what
the user saw was Chrome's back-navigation flash — the app looking like a
reloading web page (owner, 2026-08-24: "chat agent配置等页面 对于返回手势适配
不太好 像是网页刷新了。像文件管理页面就很好"). The Files page was named the
reference, so the Hub now speaks the exact contract Files defined: App owns
the history stack (seeded `{app:true}` entries, `popstate` routed to the
visible page's registered `onGoBack` closure, re-push at the floor so the
browser never actually leaves), and the page answers "did I consume this?".
The Hub consumes by peeling its topmost transient layer — the same order a
tap outside or Escape would: context menu, agent menu, recipient picker, `/`
palette, armed interrupt, the confirm dialogs, team picker, create dialog, an
in-progress rename, then the terminal drawer (through `closeDrawer`, because
every drawer toggle goes through the reading anchor), and finally — compact
only — a bare conversation lifts the project list. The list is the FLOOR,
Files' `/`: with it open, back falls through unconsumed to App's re-push,
which is what keeps back from cycling the list open and closed forever. The
Agents page does the same with its two layers (pending delete dialog, then
whichever editor is open — on compact the editor takes the whole screen, so
back-to-list is the gesture's plain meaning). A consumed pop re-pushes in App
so the next back always has an entry to spend. Pages register through the
`onGoBack` prop; nobody installs a second `popstate` listener.

#### Starting a project, and interrupting an agent

Creating a project asks for a NAME and a PATH, in that order, and the name
is REQUIRED: it names the project and seeds its tmux session name. Left
empty, the server falls back to the directory's basename, which produced
projects called "src-tauri" (owner report) — and no better default exists,
because the folder name is exactly what was wrong. The path field still
accepts a typed path, but `Browse…` opens `DirPicker` — the same `fs_list`
RPC the file browser uses, directories only, read-only. It is a chooser,
not a second file manager: no preview, edit or upload, 40px rows for a
thumb, and the path label keeps its TAIL visible (`direction: rtl`).

Interrupting is a THIRD verb, between "say something" and "stop the
agent": it types `Escape` into the agent's own pane. That is the only
channel that reaches a BUSY agent — a `tmm` message is read between turns,
so it cannot cancel the turn in progress — and Escape is how the supported
TUIs cancel. It must be the NAMED key, never a raw `\x1b`: with
`extended-keys on` tmux drops raw C0 bytes sent to a pane in extended mode.
Interrupt cancels output and leaves the agent alive; stop/restart remain
the heavier, separately-confirmed actions.

#### User-facing vocabulary (the contract)

One noun per concept, everywhere the USER reads: the tab is **Chat**
(中文 "对话") — it was "Hub", a name that described the architecture, not
the page; a **Project** (项目) is the container entity in the left column,
and each project has one chat; the things that speak are **agents**.
"Room" is the bus's term (`proj:<session>`) and NEVER appears in UI copy —
the no-recipient send is "leave a note in the chat" (中文 already said
"只记录/不打断任何人"). Internal identifiers (`hub_*` RPCs, component
names, i18n keys) intentionally keep their names: they are API contracts,
and renaming them buys migration risk, not clarity.

#### Design tokens (the contract — do not reintroduce ad-hoc values)

An audit (2026-08-18, ui-ux-pro-max guidelines) found ELEVEN font sizes and
FIVE transition durations accumulated in the Hub, and the settings/connect
surfaces had grown their own (raw 10–15px sizes, `transition: all`). The
tokens now live on `:root` in app.css — APP-WIDE, not Hub-scoped — and every
component rule must reference them; a raw `font-size: 12px` or
`transition: … 160ms` anywhere is a regression. The shared UI vocabulary
(.chip-btn, .side-h, --ui-font-control) consumes the same tokens.

A second audit (2026-08-19, owner: "对我们全部的 ui 里的字号系统做一个梳理，不要
出现太多 hardcode") found the rule had only ever been enforced in the Hub: **185
raw px font sizes** remained across 18 other components, almost all a half pixel
from a step (53×12px, 36×13px, 33×11px, 20×10px). 168 were rounded onto the
scale mechanically; the rest were judgement calls, listed below. The guard
against a third audit is `src/lib/ui/tokens.source.test.ts`, which scans every
`.svelte`/`.css` file under `src/` for a raw px font size and fails with
file:line — with an allowlist that forces each exception to be argued for in one
place instead of hiding in a stylesheet:

- Type scale (6 steps, nothing in between):
  `--fs-micro: 9px` (uppercase letterspaced tags only: `.p-tag`, `.sr-cap`,
  `.direct-tag`) · `--fs-meta: 10.5px` (times, hints, overlays, labels) ·
  `--fs-sub: 11.5px` (names, monospace paths/steps, raw view, chips, MENU ROWS
  via `--ui-font-control`) · `--fs-ui: 12.5px` (dialogs, empty states,
  previews) · `--fs-body: 13.5px` (message text, composer) · `--fs-title: 15px`
  (page/dialog headings).
- Two DISPLAY steps above the chrome scale, for the one surface that is a poster
  rather than an interface: `--fs-hero: 36px` (the connect card's brand icon)
  and `--fs-display: 22px` (its headline). Not for reuse inside a page.
- One BEHAVIOUR constant: `--fs-input-touch: 16px`, the threshold below which
  iOS auto-zooms the page when an input takes focus. It is not a type step, and
  the phone-first connect card and template editor both need it.
- A rendered DOCUMENT scales with its own base, not with the chrome scale: the
  markdown preview's headings are `em` multiples of `--file-font-size` (the
  file browser's own setting). They were absolute px, so the reader's font size
  moved the body text and left every heading behind.
- Two raw values survive, both non-typographic and both allowlisted in the test:
  `CollabGraph .lbl` (SVG user units inside a viewBox — a CSS px token there
  would be measured against the viewport, not the graph) and TeamTemplates
  `.ag-mono` (deliberately below the iOS threshold because mono glyphs are
  wider; changing it is a behaviour change, not a cleanup).
- Metadata ink: `--meta-ink` (a text2/text3 mix). The old stack — text3 AND
  10px AND 0.78 opacity — triple-attenuated timestamps into decoration;
  opacity is no longer used to dim metadata TEXT (state icons may still use
  it for their empty/filled distinction).
- Motion: `--t-fast: 120ms` for surface feedback (color, border, shadow,
  filter, opacity), `--t-move: 200ms` for anything that moves or resizes
  (transform, height). One duration per PURPOSE, not per author.
- Colour semantics: green (`--status-ok`) means RUNNING/CONFIRMED state and
  nothing else; accent means selection and interaction. The roster capsule
  already complies (accent border = selected, dot colour = state); keep it
  that way.
- Touch targets: PRIMARY actions get a ≥44px hit area on phone — visual size
  stays in the small-radius design language, the extension is an invisible
  pseudo-element overlay. The back-to-tail control is the shared global
  `.to-tail` atom (board #49): Chat and Terminal wear the same 38px circle,
  token surface/ink/border/shadow, hover/press motion, 44px `::before`, and
  token-red `.news::after`; components may declare only placement. Dense secondary rows
  (the drawer's window pills, roster, message actions) accept the WCAG-web 24px
  minimum with ≥8px gaps instead: inflating them to 44px would destroy the
  density that page exists for. The recipient chip cannot expand upward or
  rightward — it would steal taps from the textarea's first line.
- Screen readers: a message bubble is TEXT, not a control. The copy/raw
  toggle rides the meta trailer (a real `<button>` with an i18n aria-label);
  the bubble's own click handler is a pointer convenience, not the accessible
  path — do not put `role="button"` back on the bubble (it made every message
  announce as one giant button and Tab walk through the whole transcript).


The Hub uses one adaptive chat surface rather than separate desktop/mobile
markup. Its visual hierarchy follows the useful parts of Telegram without
copying a second application: a quiet FLAT canvas derived from the existing
theme tokens (an earlier accent radial glow read as a faint blue shadow and was
removed on owner feedback); opaque incoming and outgoing bubbles; asymmetric lower corners that make
direction legible without labels; and a restrained border plus one-pixel shadow
instead of nested panels. Incoming bubbles use `--bubble-in`, outgoing human
messages use the accent-derived `--bubble-out`; both are opaque so content moving
under a sticky bubble never reads through it. No light/dark colour is hard-coded:
the component variables are `color-mix()` derivatives of `--bg`, `--bg2`,
`--accent`, `--border` and `--text3`.

One label rule: an agent's bubble is headed by its name (several agents speak
in one room); your own carries none — the right-aligned accent bubble already
says "yours". Time — and on your own messages the delivery ring, to the
time's right — is a Telegram-style INLINE TRAILER floated at the end of the
content: it shares the last text line when it fits and drops to its own
right-aligned line when it doesn't; never a separate row or column outside
the bubble (a fixed foot row under the bubble read as detached furniture, and
in-bubble fixed rows made bubbles read bigger than their words). Two CSS
pieces carry it: the last content element (when it is a `<p>`) turns
`display: inline` so the float can share its line box — safe because `.md`
paragraph margins are symmetric, so the PREVIOUS block's bottom margin still
separates them — and `.m-body` is `flow-root` so the bubble's height contains
the float. Both sides hug their content (`align-self: flex-start/flex-end`);
column-flex default STRETCH made every agent bubble 76% wide with a short
line's time stranded at the far right: the first design put it in the head next to a
delivery chip whose `margin-left: auto` shoved the time from right to left the
moment the receipt arrived (owner report). Your own messages also carry a
status ring in that foot, ALWAYS: an empty circle until the agent's prompt
hook echoes the line back, a green check once it does — the receipt is a state
change, not an appearing element.

Corner radii are one small scale, now TOKENIZED after two owner rounds (first
tuned down on plain arcs — 大圆角 read as toy-like; then up twice once corners
became continuous, 2026-08-21: a squircle's curvature concentrates at the
corner, so the same radius reads tighter, and elements across pages had
drifted apart — "很多页面里的一些元素，圆角统一下"). Three tiers in app.css +
two specials, and a swept rule REFERENCES the token instead of restating the
number: `--ui-radius-control: 10px` (buttons, inputs, chips, menu items),
`--ui-radius-row: 12px` (list rows, roster cards, toasts, small bars),
`--ui-radius-panel: 14px` (cards, menus, tool lanes, big panels); specials are
bubbles 18px with a 6px directional corner + the 16px composer capsule (15px
compact), and dialogs 18px. Micro tags (4–6px) stay hardcoded — at that size
the arc and the superellipse are the same pixels. On engines that know
`corner-shape` (Chromium 139+), every rounded RECTANGLE draws those radii as
CONTINUOUS iOS-style corners —
`corner-shape: squircle`, ONE policy block in app.css listing the surfaces by
component ("有质感一些，类似于 iPad、iOS 或 Mac 上的应用。尤其是标签的圆角过渡",
owner 2026-08-20; it lives there because svelte-check's CSS service does not
know the property yet, and global selectors reach scoped components because
Svelte keeps the original class names). Everywhere else the plain circular arc
stays — same radius, no polyfill. Full-round shapes (999px capsules, toggles,
50% dots) are excluded on purpose, since a superellipse flattens a stadium's
caps and squares off a circle; a listed class that grows a pill variant gets an
explicit `corner-shape: round` reset next to the policy block.

With metadata inline, the sticky anchor `.msg` and the bubble are the same
box again, and a held bubble is simply that box at full height. Prose bubbles are
bounded to `var(--msg-max)` = `min(84%, 1360px)` on a wide screen (one variable
shared by bubble, prompt row and tool lane — the %-term is what rules, so a wide
monitor gets wide bubbles; owner, 2026-08-28) and 91% on a phone, while tool runs,
observed prompts and status facts use the same width ceiling but stay visually
subordinate. The feed reuses `.subtle-scroll`; system events are centered frosted
pills — CONSECUTIVE lifecycle lines fold into one pill (a stop followed by a
restart is one fact, not two rows), and
at the chat-only level they disappear entirely, because they are the app's
record, not the conversation. Inside the pill each line gets its OWN ROW: they
used to be joined by a `·` on one nowrap line, so "removed k" and "spawned k"
ran together into a single grey run-on string ("removed k spawned … 单独一行，
有一些高亮的样式", 2026-08-24). Every row then speaks ONE grammar — WHO it is
about, WHAT happened, and the DETAIL ("slash 命令，以及 agent 的 remove 等状态
更新，都用统一的 ui 来展示…包括 agent 的名字，状态，或者发送的指令", 2026-08-24)
— and each atom reuses a dialect the feed already speaks instead of inventing a
third: `sysParts()` parses every shape into `{who, verb, text, cmd}` (`removed
k`, `spawned dev — brief` with the brief as detail, `/model x → dev, rev` where
the split is the LAST ` → ` — the one `hub_command` appended, so a typed arrow
survives). The NAME wears the bubble header's ink (`.m-head`'s 650-weight
accent), the ACTION is the status dot + state word (tinted by `sysVerbColor()`
in the ONE progressive status language every other coloured state word in the
Hub uses: accent = something started moving, green = ended well, grey = at
rest, red = destructive, amber = interrupted; both pure + tested, and the value
is always a token so the two themes read the same), and a `/command`'s typed
line stays ONE object — name and arguments together in the rendered-markdown
INLINE CODE dialect (monospace, soft `--code-bg` wash, the composer's accent
lean). Two wrong shapes preceded it: a micro-pill name beside loose arguments
at another size read as fragments of two dialects ("带参数的渲染好像不是很好",
2026-08-24), and drawn frames on the inner atoms — a bordered verb pill, a
bordered command capsule — read as chrome rather than content ("不用这种边框的",
same day). The targets stand as the who.
The per-row ellipsis sits on the TEXT, so a long detail can never eat the
badge or the name. An unknown line shape keeps no verb or name at all and
renders whole: guessing either would truncate the remainder. Subordinate is not
ILLEGIBLE: the pill wears reading ink (`--text2`, `--fs-sub`), because
`--text3` fine print was reported as unreadable ("不要只用灰色小字，让我看不太清",
2026-08-20). The feed also marks calendar days in the same pill dialect: a
centred date pill (`Today` / `Yesterday` / a local date) before the first block
of each new LOCAL day (`sameDay()`, pure + tested — the times alone never said
which day a message was from, 2026-08-20); it is not sticky, because a pinned
rect would fight the ask-anchor's edge math. The tap-revealed copy/raw actions are an absolutely-positioned OVERLAY on
the bubble's bottom-right corner, not a row in the flow: opening them must not
push the conversation around or change the scroll height the anchor math
depends on. The bubble itself carries a text cursor, not a pointer — it is
selectable prose first; the tap affordance needs no hand.

The composer is one rounded capsule at every width, and everything lives inside
it. The recipient chip is pinned to the capsule's top-left; the textarea's FIRST
line starts beside it via a measured `text-indent` (the chip width is bound with
`bind:clientWidth`, so a long agent name still works) and wrapped lines reclaim
the full capsule width beneath the chip. The send button is a small rounded
SQUARE in the capsule's bottom-right corner — same design language as the
capsule, flat, no shadow physics of its own (the floating accent circle read as
a foreign element). Its glyph is a BOLD UP-ARROW (the iMessage/ChatGPT shape):
symmetric, so it optically centres where a diagonal paper plane always sat
crooked in a small square — the plane was the owner's "太丑了". Ink is crisp,
not washed: near-white over the light theme's deep blue; in the dark theme the
accent is ELECTRIC CYAN and a full-strength block of it read as a light source
(owner report), so the fill tones to a 60% accent/background mix with
near-white ink — promoted to app-wide tokens `--accent-fill` /
`--accent-fill-ink` / `--accent-line` in app.css (2026-08-18): every solid
accent CTA (send, connect, PWA install, git commit, Team send/start actives) and strong selection border draws from
them, so the "glowing block" class of bug is fixed in ONE place. Disabled recedes into the surface instead of ghosting the
accent. The button reserves NO column (owner: text may run directly above
it): the textarea is full width, and growComposer measures the value in a
hidden mirror div to find the LAST line's right edge — only when that edge
would collide with the button zone does the box gain bottom padding, and the
pad clears the button's FULL height (34px): a one-line pad still left the
button's top strip over the glyph descenders (owner report). When the box is
at max height and SCROLLING, every line passes under the button's corner, so
the avoidance flips axis: a 40px right padding shortens all lines clear of
the button for as long as the scroll state lasts, and releases with it. Same
"share the last line, else drop below" semantics as the bubble's meta
trailer; a textarea cannot flow around a float, so the mirror is the only
honest way to know where the last line ends (the mirror must mirror font
metrics, width, wrapping AND the chip text-indent). The indent is a re-measure dependency of the
auto-grow, since it changes where text wraps. On a phone the chip drops its
redundant “TO” prefix and caps its width; safe-area padding remains on the outer
composer.

Scrolling follows four rules, all of them about not losing your place. New
content scrolls the feed only while it is parked AT the tail (`following`) —
yanking someone back down while they read history is worse than a missed
autoscroll — and sending forces it, because you plainly want to see what you just
sent. Parked away from the tail, a round button offers the way back and carries a
dot when a MESSAGE arrived meanwhile (telemetry rows extend the tail but are not
news). There is exactly **one** user-message anchor in the whole viewport, and
it is never a second pin component. The real message bubble must enter and move
with the feed first; only when that SAME DOM element is about to leave does
`position: sticky` catch it. Scrolling down selects the newest naturally visible
user message and prepares its top edge; scrolling up selects the oldest naturally
visible one and prepares its bottom edge. Through a long reply with no user
message naturally visible, that active bubble remains the same — it MUST NOT swap
to the next/previous message at an invisible midpoint. When the next real bubble
enters, it takes over in its natural location. This is what makes second, third and
later questions work without ever stacking two anchors.

The difference between “active” and “held” is deliberate. `ask-top` /
`ask-bottom` puts the same bubble into the sticky flow while it travels; the
floating treatment starts only under `.held`, after it has actually touched the
edge. Styling it while it was still travelling made one DOM element LOOK like two
components.

**The TEXT folds; the bubble is never cut.** Four positions in three days, and
the fourth is the one that separates the two ideas that kept getting confused. It
began clipped to a 33px window — one line, which read as a truncated one-liner.
Then it showed all of itself, which is right for a two-line ask and swallows the
conversation for a twenty-line one. Then it was capped with `max-height` +
`overflow: hidden`, which is the same mistake as the clip wearing different
clothes: "我希望是消息内容自己内部折叠 不是框截断 … 气泡什么的都要完整的不要任何裁
切" (2026-08-19).

So: no clip, no cap, no `overflow: hidden` anywhere near `.held`. The bubble
keeps its whole box — border, radius, padding, meta trailer — and is exactly as
tall as the text it is showing. **Every long user message folds by default**, not
only the held one: `elideTail` cuts the BODY at the rear before Markdown render,
glues the full-width `……` marker onto the last kept line, and an in-bubble `Show
the whole message` control releases it. Expanded messages are never pinned: they
rejoin the feed so the whole body is reachable; `Fold it away again` reverses the
choice, and an expanded message that leaves the viewport refolds through the
reading anchor.

The budget is VISUAL rows, not source newlines (board #53). Each source line costs
`ceil(visualUnits / perLine)` rows (at least one); the line that exhausts the
budget is cut at the units still available, backing up to a Latin word boundary.
CJK/fullwidth codepoints cost two units. This prevents one 3,000-character source
paragraph from riding through whole while preserving the simple short-line case.
A cut that strands a Markdown fence appends its closing fence, and text that fits
comes back by identity.

Both dimensions are measured from the real chat, never guessed. `measureHeld`
takes 20% of the stable CHAT COLUMN height (the feed's parent, not the composer-
shrunk feed) and divides by the bubble line box for `heldLines`. For `perLine`, it
computes the bubble's maximum content width from the feed content box and the
shared `--msg-max` (`min(84%, 1360px)` minus bubble padding), then divides by a
cached canvas measurement of the current bubble font's representative Latin
glyph width (`perLineOf`, clamped 16–240; 80 only before measurement). Thus the
same text keeps roughly the promised rows at 1280px and 420px, while CJK's 2-unit
pricing preserves the same physical height. A line of error only makes the bubble
a line taller — nothing is clipped and unfold always holds the source.

A width-changing layout mutation is part of the same reading-anchor transaction:
mutate → settle the new grid → `measureHeld` → settle the re-cut messages → only
then restore the reader's reference offset or the live tail. This matters for the
right drawer, which changes chat width without a window resize; waiting for a
future poll left stale wide-screen folds in the narrow column.

**Folding changes the box's height, so one browser guard stays.** The original
`max-height` collapse closed a feedback loop: Chromium's scroll anchoring
compensated `scrollTop`, the compensation flipped the boundary test, and the
anchor blinked indefinitely. The feed therefore keeps `overflow-anchor: none`
and follows its tail explicitly. The old second guard (`naturalH`, an unfolded
height fed to the boundary test) retired when fold-on-hold retired: folding no
longer toggles at the sticky boundary, so the real `offsetHeight` is honest.

The bubble is made opaque rather than replaced: bubble tints are rgba, so reply
text otherwise reads through them. Compositing the same tint over the page colour
preserves its ordinary appearance while the backdrop blur softens what moves
under it.

Direction has hysteresis too: trackpads and touch momentum land 1–3px reversals
at rest, and at the held boundary a direction flip re-picks the anchor. A
reversal only commits after 16px of travel against the current direction.

One browser fact is part of the contract: Chromium reports `offsetTop` for a
sticky element at its HELD position, not its original flow position. Before the
scroll handler measures candidates, it synchronously overrides the current
anchor to `position: static`, reads all natural positions, then removes the
inline override before paint (`.held` needs no neutralizing — it is paint-only).
Without that neutral measurement, the old anchor looks naturally visible forever
and transition state becomes stale. Programmatic jumps to the tail call the same
synchronization explicitly; setting `scrollTop` to the same value does not emit a
scroll event.
And when the keyboard opens, the feed re-parks at the tail: `--app-height` shrinks
the box while the scroll position stays, which otherwise leaves the newest line
below the fold — App already broadcasts `keyboard-shift`, so the Hub listens
instead of measuring anything itself.

Inside that shell the composer remains a textarea, not an input: a message you
are still writing has to be readable, so it wraps, grows to fit (height measured
from `scrollHeight` — wrapping depends on font, width and text, so it cannot be
guessed) and starts scrolling at a `max-height` ceiling. Growing it shrinks the feed, which is the
keyboard problem again, so it re-parks the tail as it grows. Enter sends where
there are modifiers to distinguish with, and inserts a newline on a touch device —
there the return key is the only way to get one and the send button is right
there; Shift+Enter is always a newline.

Tapping a message reveals what you can do with it — copy the source, or switch
between rendered Markdown and raw source — rather than parking two buttons on
every bubble forever. Raw is always the exact `m.body`. Rendered view applies
Markdown, including one deliberate chat convention: a complete ` ```markdown `
or ` ```md ` fence is a transparent wrapper and its contents are rendered again.
Agents commonly wrap a requested `.md` document that way; showing it as `<pre>`
made “rendered” indistinguishable from raw (the real `proj:test` seq=52 failure).
Other language fences and unclosed Markdown fences remain code. Fence length is
respected, so a four-backtick Markdown wrapper may contain ordinary triple-
backtick code blocks.

The roster is one
line per agent (avatar, name, state dot, elapsed, unread dot) with everything
secondary behind a dot menu. That menu is a CONTEXT MENU next to the chip, in a
`position: fixed` layer placed from the trigger's client rect — fixed because the
roster scrolls horizontally and a popover positioned inside that scroll container
is clipped by it, which is why the first version was a full-width bar under the
roster instead ("是不是应该类似右键菜单一样，在旁边会比较好", 2026-08-19). The
placement is a pure function (`menuPlacement`): right-aligned to the trigger and
below it, flipped above when the room underneath is smaller than the menu,
clamped to the viewport on both axes, and skipping the flip while the height is
still unmeasured — the menu stays invisible for that one frame rather than
jumping. Under CSS `zoom` the trigger's rect is divided by `--ui-zoom` first: a
client rect is in visual pixels while a fixed child's `left` is in its own zoomed
pixels. It dismisses itself on an outside pointerdown, Escape, a roster scroll or
a resize, and every action closes it as its first act — a menu you have to close
by hand is a menu you forget to close. A stopped agent's menu carries the only
two verbs that apply to it (Start again / Remove); Watch, Interrupt and Stop all
need a live pane.

No emoji anywhere in this surface: state is carried by colour, a rotating
chevron, a pulsing dot and stroked SVG icons. Lifecycle lines the server posts
into the room (`spawned`, `done`) carry the machine marker `[tmm] ` and the
client decides how they look; `systemLine()` still recognizes the older `⚡`/`✔`
spelling because rooms are persisted and old messages must not regress into
chat bubbles.

## Verified

Against the live server: `project list` (6 projects, ● markers), `send` →
`log` roundtrip (message in `proj:tmux` with ts cursor working), `status
waiting "note"` → `agent list` shows `waiting — note`, `done` → `idle`,
`--output json` on all reads. Dead server → exit 2 in 21ms; wrong token →
exit 3; `team_status` shows `[]` teams (proj rooms filtered). Unit tests:
derivation table (9 cases), hub dispatch (4 cases), all in `cargo test --lib`.

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
