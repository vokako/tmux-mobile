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
tmm project create <path> [--name n] [--session s] [--with-agent kiro|claude|codex]
tmm project up|down|archive <session>
tmm registry list                    centrally-defined agents
tmm registry save --name <n> --backend <b> [--system <text>] [--skills a,b] [--mcp <json>] [--can-hire]
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
  `✔ done` line to the room: the chat is the record.
- `hub_agents { session }` — one row per live window: name, command, agent
  detection (`projects::agents::detect`), derived state.

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
faith (§4.3). The store is in-memory, keyed `(session, window_index)` — the
same granularity as a hook notification and a project slot.

| fact | source | verdict |
|---|---|---|
| tool event < 30s ago | hooks (isolated homes, Phase B+) | `working` + activity line |
| explicit `tmm status` | agent | as declared, expires after 30 min |
| `tmm done` | agent | `idle`, summary as detail |
| notification `permission_required` / `input_required` | hooks | `waiting` (fresh pane activity resolves it — an answered prompt self-corrects) |
| notification `completed` (Stop) | hooks | `idle` — a finished turn is REST, not distress. Direct agents fire Stop after every exchange and never call `tmm done`; the old stop-without-done→stuck rule (Team-supervisor era) branded every long-idle window as broken |
| notification `failed` (StopFailure) | hooks | `failed` — the one distress signal we can honestly observe |
| pane activity < 30s | tmux `window_activity` | `working` (hook-poor backends degrade here, not to a lie) |
| nothing | — | `idle` |

Precedence is "latest fact wins", with fresh tool activity beating a stale
declaration and `done` superseding both. The explicit-declaration TTL exists
so a crashed agent cannot stay `working` on its own last words. Notifications
feed the store from the hub's inbox consumer *before* dedupe (dedupe is a
notification-UI concern; telemetry wants every fact). Window records are
dropped when the window disappears (`retain_windows` on every `hub_agents`).

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

## Spawn: the starter pistol

An agent CLI boots into an interactive prompt and does nothing until spoken
to. The brief in the system prompt is context; the KICK — a fixed first user
message passed as the CLI's positional arg ("Start now… run `tmm done` when
complete") — is what makes it move. Without it the agent sat at its prompt
forever (observed live before the fix).

## The activity feed (telemetry in the chat timeline)

The chat shows what agents SAID; between those, the Hub can weave in what we
OBSERVED — status declarations, lifecycle notifications, individual tool
calls — as dim mono one-liners. The detail level is a Settings knob
(Appearance → Chat detail): `chat` (messages only) / `+ status` (default) /
`+ tools`. Mechanically it is `telemetry::recent_events` — an in-memory ring
(120/session) fed by the same recorders that drive status derivation,
exposed as `hub_activity { session, since_ts }` with ms timestamps so the
client merges it directly into the message timeline. It is deliberately NOT
chat history: nothing touches the bus db, the ring dies with the server, and
consecutive duplicate tool lines collapse client-side (pre+post per call).

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
