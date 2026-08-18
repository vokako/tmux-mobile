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
each (`kiro_hooks` / `claude_hooks` / `codex_hooks`), shared by render and
refresh, so the two cannot disagree.

A CLI reads its config at launch, so patching the file cannot repair a RUNNING
agent — restart is the only path, which is what the roster's restart button is
for.

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
line with the agent's own half-typed text attached), and `sweep_deliveries`
reports the ones still pending after 45 s as a `warn` event. The sweep runs
when a client reads `hub_activity`, which is exactly when the answer is wanted.

Delivery reaches MANAGED windows only. `projects::managed_home` /
`is_managed_in` is the one definition of "an agent this app created" — the
isolated home `spawn` materialized, never the window name — and three gates
share it so they cannot drift: who counts as a chat participant (`hub_agents`),
whose stop-hook reply gets posted (`maybe_auto_post`), and whose pane we may
type into (`deliver_mentions`). Without the third one, `@all` would inject a
chat line into a kiro the user started by hand in that directory.

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
to. The brief in the system prompt is context; the KICK — passed as the CLI's
positional arg, so it becomes the first user prompt — is what makes it move.
Without it the agent sat at its prompt forever (observed live).

The kick is a MARKER, not an instruction: `[2026-08-18 16:41] (session
start)`. It used to be a sentence ("Start now: read your instructions…run
`tmm done` when complete"), and that was wrong for one reason that outranks
brevity — that channel is the OPERATOR's. The prompt echo is rendered in the
chat, so the app appeared to have typed instructions at the agent in the
user's name (owner report 2026-08-18), and standing instructions do not
belong in a per-launch message anyway: they are stated once, in the agent's
definition. `build_prompt` therefore explains what the marker MEANS (read the
brief, begin, `tmm done` when complete) and states that nothing in that line
is an operator request. The marker keeps the timestamp, which is the only
place an agent learns the wall time (a system prompt is replayed on every
restart, so a baked-in date would age into a lie).

Client side, `isSessionStart()` drops the echo from the transcript entirely —
it also matches the pre-change instruction kick, because rooms are persisted.
Measured after the change: a spawned kiro agent received the bare marker,
read its brief, answered, and called `tmm done` on its own.

## The activity feed (telemetry in the chat timeline)

The chat shows what agents SAID; around that, the Hub weaves in what we
OBSERVED. Mechanically it is `telemetry::recent_events` — an in-memory ring
(120/session) fed by the same recorders that drive status derivation, exposed
as `hub_activity { session, since_ts }` with ms timestamps so the client merges
it directly into the message timeline. It is deliberately NOT chat history:
nothing touches the bus db and the ring dies with the server.

Five event kinds: `tool`, `status`, `notif`, `prompt` (a prompt the agent
accepted, `via: app | local`) and `warn` (a line that was never echoed back).

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
- **Tool calls collapse, replies do not.** Consecutive `tool` events from the
  same window fold into one collapsible group ("N tool calls", last line as the
  preview); a message, a status declaration or a notification ends the run,
  which is what makes a group mean *between these two replies*. Groups are
  per-window, so two agents working at once never share one. Open while that
  window is working, closed when it stops, and an explicit click wins — the
  choice lives outside the row (`stepsChoice`, keyed by group) so a re-render
  cannot lose it.

A tool NAME is coloured by what the tool does — changes / runs / looks up /
reads, four buckets matched on substrings so `fs_write` and `Edit` land in the
same colour without an exhaustive table (`toolColor`). The name has to be split
off first: an older server shipped tool events with no `tool` field and the name
glued onto the text (`"shell tmm send …"`), which is why every name rendered
grey — the coloured column only exists when there IS a name. `toolEventParts`
normalizes both generations, and everything downstream (the colour column, the
collapsed peek, the self-report filter) reads through it. An expanded group shows
the last `STEPS_PREVIEW` steps with "show all N", because the tail is what tells
you where the agent is.

Self-report filtering is segment-wise: agents chain the report onto one shell
line (`tmm send "…" 2>&1; tmm status working "…"`), so a command is invisible
only when EVERY `;`/`&&`/`||` segment is a `tmm` self-report — the `tmm send`
the room already shows as a message never prints again as a tool row, while
`tmm send "done" && make deploy` keeps its row because the deploy has no other
trace. A `;` inside a quoted message body fails open (the row stays).

Because a run of tool calls is now one line instead of forty, `+ tools` is the
default detail level, and the level is reachable from the Hub head (a chip that
cycles chat → status → tools) as well as Settings → Appearance → Chat detail.

### Conversation visual language

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
(.chip-btn, .side-h, --ui-font-control) consumes the same tokens. Two
deliberate exceptions: the connect card's hero title, and its INPUTS at 16px
— below 16px iOS auto-zooms the page when an input focuses, which on the
phone-first connect card is worse than a size off the scale:

- Type scale (6 steps, nothing in between):
  `--fs-micro: 9px` (uppercase letterspaced tags only: `.p-tag`, `.sr-cap`,
  `.direct-tag`) · `--fs-meta: 10.5px` (times, hints, overlays, labels) ·
  `--fs-sub: 11.5px` (names, monospace paths/steps, raw view, chips) ·
  `--fs-ui: 12.5px` (menus, dialogs, empty states, previews) ·
  `--fs-body: 13.5px` (message text, composer) · `--fs-title: 15px`
  (page/dialog headings).
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
  `::after` overlay (send button, jump-to-tail). Dense secondary rows
  (statusline windows, roster, message actions) accept the WCAG-web 24px
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

Corner radii are one small scale, tuned down on owner feedback (大圆角 read as
toy-like): bubbles 12px with a 4px directional corner, the composer capsule
10px, roster chips 9px, system pills 8px, recipient chip and message actions
7px. The held mini-bubble's clip `round` and drawn frame follow the bubble
radius — change one, change all three.

With metadata inline, the sticky anchor `.msg` and the bubble are the same
box again, so the held windows are plain `100% − 33px` with no foot
compensation. Prose bubbles are
bounded to `min(76%, 760px)` on a wide screen and 91% on a phone, while tool runs,
observed prompts and status facts use the same width ceiling but stay visually
subordinate. The feed reuses `.subtle-scroll`; system events are centered frosted
pills — CONSECUTIVE lifecycle lines fold into one pill ("stopped lead ·
restarted lead": a stop followed by a restart is one fact, not two rows), and
at the chat-only level they disappear entirely, because they are the app's
record, not the conversation. The tap-revealed copy/raw actions are an absolutely-positioned OVERLAY on
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
accent CTA (send, statusline session block, connect, PWA install, git
commit, Team send/start actives) and strong selection border draws from
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
collapsed look starts only under `.held`, after it has actually touched the edge.
Styling it while it was still travelling made one DOM element LOOK like two
components.

**The held collapse is paint-only, never layout.** The first implementation
shrank the bubble to one line with `max-height`, and that closed a feedback loop:
collapsing changed the bubble's flow height, the browser's scroll anchoring
compensated `scrollTop` (measured: assigning 2261 landed on 2221↔2298), the
compensation flipped the boundary test, and the anchor blinked indefinitely —
the reported "一闪一闪". The fix is `clip-path: inset(… round r)`: it clips
painting and hit-testing but the element keeps its full flow height, so holding
an edge can never move the scroll position. A bubble shorter than the clip
window computes a negative inset and simply does not clip. Do not reintroduce
any held style that affects layout (height, padding, font-size, display).

Both edges show the SAME preview — head (with its time) + first line — in a
53px window. Top edge: clip the message to its top 53px. Bottom edge: the same
clip ON THE BUBBLE plus `translateY(calc(100% − 53px))` — transform moves
painting, not layout, so the bubble's head slides down into the bottom window;
without it the window showed the bubble's TAIL and the timestamp was cut off
(owner report). Both edges show the question's FIRST LINE: asks are the user's
own messages, which carry no head row (time lives in the foot), so the window
is 33px — first line box 9..29, next line's glyphs from ≈31, bar 29..32,
stroke 32..33. The frame of the held mini-bubble is DRAWN, not inherited: the
real border cannot survive the clip — its bottom edge lies below the window and
its side strokes are eaten by the window's corner rounding, and a first attempt
that only drew a floor line put that line at border-y 53..54, which the 53px
clip removed exactly (a pseudo-element's `top` is padding-box relative, 1px
lower than the border box the clip measures). While held, the real border-color
goes transparent (paint-only) and `::before` draws the complete frame — outer
edge exactly on the 53px window (`top:-1px; height:53px` in padding coords),
stroke safely inside the clip; `::after` is an opaque bar covering the next
line's ≈2px glyph sliver. The bubble's 140ms border-color transition makes the
swap a soft cross-fade — and also means a computed-style read right after the
class flips reports a MID-TRANSITION colour, which cost half an hour of chasing
a cascade bug that did not exist. Geometry is re-derived
whenever bubble padding, head presence or line-height change — current numbers
live beside the CSS.

Direction has hysteresis too: trackpads and touch momentum land 1–3px reversals
at rest, and at the held boundary a direction flip re-picks the anchor. A
reversal only commits after 16px of travel against the current direction.

One browser fact is part of the contract: Chromium reports `offsetTop` for a
sticky element at its HELD position, not its original flow position. Before the
scroll handler measures candidates, it synchronously overrides the current
anchor to `position: static`, reads all natural positions, then removes the
inline override before paint (`.held` no longer needs neutralizing — clip-path
does not change layout). Without that neutral measurement, the old anchor looks
naturally visible forever and transition state becomes stale. Programmatic jumps
to the tail call the same synchronization explicitly; setting `scrollTop` to the
same value does not emit a scroll event.

The held bubble is made opaque rather than replaced: bubble tints are rgba, so
reply text otherwise reads through them. Compositing the same tint over the page
colour preserves its ordinary appearance while the backdrop blur softens what
moves under it.
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
secondary behind a dot menu; the menu renders as a BAR under the roster, not a
popover inside the chip, because the roster scrolls horizontally and a scroll
container clips absolutely positioned children.

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
