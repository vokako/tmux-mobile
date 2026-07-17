# Team — multi-agent collaboration

> A letter to the next agent. This explains **why** the Team feature is shaped
> the way it is and the traps we avoided. Code comments cover the **how**.

## 1. What it is

The **Team** tab lets the human, from the phone, drop into a group chat with
several coding agents (Kiro / Claude Code / Codex) and coordinate them — and tap
any agent to preview its live tmux execution state in the Terminal tab.

### Architecture overview

The diagram below shows what runs where: the phone connects over WebSocket to the
desktop server, which hosts the in-process message bus and supervises one
configured backend CLI (**Kiro / Claude Code / Codex**) per agent inside a
`tmm-team-<slug>` tmux session. Each agent runs the same loop — `wait` → reason
→ execute real tools (heartbeat fires where supported) → `post` → loop back.
The hooks (keepalive, heartbeat) and the supervisor's self-heal mechanism keep
the loop alive; external standalone agents can join the same bus over HTTP MCP.
Read this picture first; the prose below fills in the rationale.

![Team architecture](team-architecture.svg)


It is built on **agora**, an experimental group-chat message bus (originally a
standalone project at `~/agora`). agora is:

- An **append-only group chat** over SQLite. Everyone reads the same log.
- A **pull model**: agents call `wait` (blocking long-poll) to receive messages,
  `post` to speak. Addressing is by `@name` in the body; `requires_reply=true`
  makes the bus refuse an agent's `wait` until it answers (the "obligation
  graph"). Enforced in the bus, not per-agent — agent-agnostic.
- `@all` is stronger than a normal mention: it expands to every registered
  agent except the sender and always creates reply obligations, even if an MCP
  caller omits `requires_reply=true`. Plain broadcasts never infer recipients
  from phrases such as "everyone"; manager-led prompts require the lead to
  translate explicit group intent into `@all`.
- Served as an **HTTP MCP server** (`/mcp`) for agents (zero-touch: a role prompt
  + an `x-agent` header) plus a dashboard / SSE / JSON API for humans.

### Naming: "team" is ours, "agora" is the library

"agora" is the upstream project's codename. Everything **we** built and
everything the **user** sees is branded **Team** (the Team tab, `team_*` RPCs,
`TEAM_*` config, `tmm-team-*` tmux sessions, `team.rs`/`team_bridge.rs`,
`team/` scripts). The **vendored library crate keeps its real name `agora`**
(`src-tauri/crates/agora/`) — renaming a faithful third-party copy would only
obscure its origin. So `use agora::bus::Bus` inside `team_bridge.rs` is
deliberate; everything around it is `team`.

## 2. Integration shape — and why

The Team switcher follows Terminal's expanded switcher geometry: one 31px bar
made from 24px chips, 3px vertical padding, and the bottom divider. Agent chips
scroll inside their own middle strip; team selection and actions stay fixed,
and the bar never wraps taller as agents are added.

The decisive choice: **vendor agora as an in-process, desktop-only sub-crate and
share ONE `Bus` between the agents' MCP daemon and the phone's WS server.**

```
                tmux-mobile desktop server (one process)
   ┌───────────────────────────────────────────────────────────┐
   │  agora::Bus  (SQLite + tokio::broadcast)                    │
   │     ├── agora::web::serve → axum :8787  /mcp + dashboard ───┼──► kiro/claude/codex agents
   │     ├── TeamBridge (JSON trait) ──► tmux-mobile WS server ──┼──► phone Team tab
   │     └── team::supervisor (in-process) ─────────────────────┼──► launches agents into tmux
   └───────────────────────────────────────────────────────────┘
        ▲ agents each run in their OWN named window of a
          per-workspace session: tmm-team-<workspace-slug>
```

**Launching is in-process** (`src-tauri/src/team.rs`): the phone's "Start team"
button calls `team_start_team` with a chosen **workspace** (the agents' working
dir), and the desktop server itself seeds the default roster onto the bus and
reconciles it into real agent windows in tmux — no separate script, no extra
process. The agent CLIs still run as their own tmux processes (intrinsic, and
what enables pane preview), but all orchestration lives in the app. (The team's
Python launcher that predated this Rust port has been retired — see §"Retired".)

Why this shape, and what was rejected:

- **In-process, not a separate daemon the phone HTTP-hops to.** The phone already
  has an authenticated, encrypted WS link to the desktop server. Routing team
  through it (a handful of `team_*` RPC methods + a `team_message` push) means
  **no second auth surface, no second port the phone must reach, no CORS, reuse
  of the existing E2E encryption + reconnect logic.** External agents still get
  the real MCP endpoint on `:8787` because the *same* `Bus` also runs agora's
  axum router. One room, two front doors.

- **Desktop-only, target-gated.** agora pulls in axum + rmcp + rusqlite — heavy,
  and pointless on a phone (the phone is a *client*, it never hosts the bus). The
  dependency is gated:
  `[target.'cfg(not(any(target_os = "android", target_os = "ios")))'.dependencies]`.
  Verified: `cargo tree --target aarch64-linux-android -i agora` prints nothing;
  the host target shows it. The Android build is unaffected.

- **A JSON-only trait at the boundary (`server::TeamBridge`).** `server.rs`
  compiles on mobile too, so it must not name any agora type. The trait speaks
  only `serde_json::Value`; the concrete impl (`team_bridge::TeamManager`) lives in a
  desktop-only module. Mobile passes `None` and the `team_*` methods return
  method-not-found, which the Team tab reads as "unavailable" and hides itself.
  This seam keeps one codebase building for both targets.

- **Vendored, not a git submodule / path-dep on `~/agora`.** Per the repo creed,
  "not in the repo = doesn't exist." We copied the library source into
  `src-tauri/crates/agora/` (dropping its CLI `main.rs` — the daemon runs
  in-process) and kept its tests.

## 2b. Multiple teams = isolated rooms

The bus is **room-aware**: each team is its own chat **room** (id = the
workspace slug), fully isolated — separate message log, roster, and obligation
graph. One daemon serves all rooms.

- **agora crate**: a `BusProvider` trait (`room → Bus`) that `mcp.rs`/`web.rs`
  resolve per request — agents via an **`x-room` header**, the human API via a
  `?room=` query. `SingleRoom` wraps one `Bus` for the single-room/test path.
  The store was already fully room-keyed, so only routing changed.
- **Host (`team_bridge.rs`)**: `TeamManager` is a **registry** of rooms (each a
  `Bus` lazily opened on the shared db, with its own pump into one merged push
  channel) and *is* the `BusProvider` for the MCP daemon. Unknown rooms are
  refused until a team is started — a stray agent can't conjure a room by
  guessing a header; only the operator's "New team" creates one.
- **The phone** passes the active `room` with every `team_*` RPC and filters the
  `team_message` push by each message's `room`, so the view never mixes teams.
- **Agents** are launched with an `x-room` header for their team's room, so
  kiro/claude/codex each join the right chat.
- **Team tab UI**: a header dropdown lists active teams (room · agent count),
  "+ New team" opens the workspace picker, and "×" closes the active team
  (`team_close_team` kills its tmux session; the chat log persists in the db, so
  re-starting the same workspace resumes its history).

### Frontend session identity & availability (`src/lib/team.svelte.js`)

The ONE frontend module that knows team tmux sessions are named
`tmm-team-<room>` (room = `workspace_slug` = `<basename>-<6hex>`, see
`team.rs`). Exports `TEAM_PREFIX`, `isTeamSession`, `teamRoomOf`,
`teamSessionOf`, `teamLabel`, and the shared `teamState`
(`{available, probed}`) rune. Sessions, PanePicker, and Team import from
here — the prefix/slug scheme must never be re-derived locally (it drifted
once: PanePicker grouped ungated while Sessions gated, showing the same
session as "team" on one surface and "regular" on another).

- `isTeamSession` is **gated on `teamState.available`**: on a busless
  server, `tmm-team-*` names fall back to ordinary sessions everywhere.
- `teamState` is written only by App's `probeTeam()`, and only a definitive
  method-not-found (-32601, surfaced via `err.code` from `ws.js`) may flip
  `available` to false. Transient RPC failures keep the current value —
  flipping it unmounts the always-mounted Team component (below) and
  destroys exactly the state it exists to preserve.

### Team tab lifecycle (always-mounted)

The Team component stays mounted while `teamState.available` (a hidden
`page-layer`, like Files/Terminal) so switching tabs preserves `activeRoom`,
the loaded history, reading scroll position, and the embedded agent
terminals. Consequences, all deliberate:

- **Re-shows don't refetch.** The `team_message` push listener keeps
  `messages` current while hidden (appends only — no roster RPCs, no scroll
  nudge while hidden); the visible-effect full-`refresh()` runs once, later
  shows only tick the roster poll. This is what makes scroll position
  actually survive a tab switch.
- **Reconnect is the exception**: pushes are lost while disconnected, so a
  `ws-reconnected` listener re-runs the full `refresh()` (visible or not)
  to close the gap in the log.
- **`page === 'team'` needs a fallback**: the layer only mounts when
  available, so once the probe definitively answers "no bus", App resets
  `page` to sessions (otherwise the main area would render blank — restore
  sets `page` before the probe resolves).
- **Sessions → Team jump**: Sessions calls `openTeam(room)` → App invokes
  the exported `selectTeam(room)` via `bind:this` (imperative on purpose:
  re-selecting the same room must still close the switcher/new-team panel;
  `bind:this` nulls itself on unmount, unlike a hand-registered callback).
  If the room isn't in the manager's team list (stale tmux session that was
  never recovered), `refreshTeams`' keep-valid fallback re-points to a valid
  team and the UI shows a transient "Unknown team: <room>" banner instead of
  silently landing the user in a different team's chat.

## 3. Per-workspace teams (the "limited to a directory" requirement)

A team is tied to a **workspace** = the agents' shared working directory (their
real project). The phone defaults it to the current terminal session's cwd
(`fsCwd`), shows it, and lets the user edit it before starting. The room id IS
the workspace slug, so the team, its session `tmm-team-<slug>`, and its config
home all share one identifier.

- The tmux session is **`tmm-team-<slug>`** where `slug` is the sanitized
  workspace basename (`team::workspace_slug`, mirrored in `Team.svelte:slugify`
  — they MUST agree, or pane preview can't find windows). tmux names can't
  contain `:` or `.`, so the slug strips them. The `tmm-team-` prefix lets the
  app recognize team sessions (e.g. the PanePicker labels their panes by agent
  name, not `current_command`).
- Multiple teams coexist: different workspaces → different `tmm-team-<slug>`
  sessions, each with a self-gitignored runtime home at `<workspace>/.tmm/`.
- Team does not add tracked source or instructions to the user's project.
  Backend config and hooks live under `.tmm/`; prompts are passed inline.

### Workspace history and recovery

The complete append-only room log remains authoritative in the shared SQLite
database (`team_db`, default `~/.config/tmux-mobile/team.db`). Closing a team or
explicitly starting the same workspace clears only process-lifetime state
(roster, obligations, and desired employees); messages are retained. A backend
restart with a live tmux session retains both messages and runtime state.

Each room also mirrors its full transcript to
`<workspace>/.tmm/team-history.jsonl`, one serialized message envelope per line.
Room registration first rebuilds the file atomically from SQLite, then the live
message pump appends each higher sequence number exactly once. If the broadcast
receiver lags, it rebuilds from SQLite instead of accepting a gap. The mirror is
not a second source of truth and is not imported into SQLite; it is a durable,
agent-readable context file. Every generated agent prompt points to it so a new
CLI launched after close/reopen can inspect prior decisions and handoffs even
though its own provider conversation is new.

## 4. Agent ↔ tmux pane link (the "preview execution state" requirement)

Each agent runs in its own tmux window **named after the agent**
(`tmux::new_named_window`). Agent → pane mapping is `window_name == agent.name`
within `tmm-team-<slug>`. Two surfaces use it:

- **Mobile / narrow**: the roster chip → `Team.svelte:previewAgent` →
  `openTerminal(...)`, jumping to that agent's pane in the Terminal tab (same
  subscribe/snapshot path as any pane). The PanePicker dropdown also labels
  `tmm-team-*` panes by agent name.

- **Desktop (≥900px, non-touch)**: the Team tab is a **two-pane split** — a live
  agent terminal **grid** on the left, the chat on the right, with a draggable
  splitter (ratio persisted in `tmux_team_gridfrac`, default 0.6). See
  `AgentGrid.svelte`:
  - one cell per employee, laid out near-square: `cols = ceil(√n)`,
    `rows = ceil(n/cols)`;
  - each cell is a **chromeless** embedded `Terminal` (new prop — no
    window-switcher bar, window-list poll skipped, since the cell is pinned to
    one agent). Read-only until clicked; clicking activates it (xterm focus +
    full interaction), exactly like an active split cell;
  - cell font = app font − 2 (glanceable previews);
  - panes resolved via a 2 s `listSessionsWithPanes` poll, so cells fill in as
    agents launch; offline employees show a placeholder until their window
    appears.
  No quick-switch chrome — the grid is fixed by agent count, by design.

## 5. The in-process supervisor (`src-tauri/src/team.rs`)

`team::start(bridge, cfg, room, workspace, template)` runs as a tokio task. On
"Start team" (`team_start_team`, one-shot guarded) it:

1. derives `slug` + session `tmm-team-<slug>`, writes the embedded hooks into
   `<workspace>/.tmm/`, and composes each backend's prompt inline (a packaged
   `.app` needs no external runtime files);
2. seeds the selected template's fixed roster as employees, unless a team is
   already present (restart-safe);
3. runs a 3 s reconcile loop launching each employee into a named window;
   `disabled` employees' windows are killed. Same loop serves the initial team
   AND the manager's runtime `hire`/`fire`.

**Restart recovery: adopt the windows, nudge the agents to reconnect.** When the
backend restarts, `recover_running_teams` finds the surviving `tmm-team-*`
sessions and re-runs the supervisor, which **adopts** the existing agent windows
(`tmux::find_window_by_name`) instead of reopening them — so each agent keeps its
CLI conversation context and any in-progress work. But an adopted agent is hung:
its MCP **client** connection died with the old daemon, and (verified with
kiro-cli 2.7.0) the client neither times out nor retries on its own — it sits
forever inside a dead `wait` call. It *will* reconnect, though, once that call is
cancelled and a fresh turn starts. Recovery snapshots the existing agent windows
before restarting the supervisor, then calls `team::nudge_adopted_agents` on
that frozen list. Each adopted window receives `Esc` (cancel the in-flight call
→ back to the prompt) and a short re-prompt that makes it call `wait` again,
re-establishing the connection against the stateless daemon. The frozen list is
important: the supervisor may fill a missing roster window during the nudge
delay, and interrupting that fresh CLI can cancel its first-run setup before it
joins the bus.

This *reconnect* nudge lives in recovery, **not** in the reconcile loop's fast
path, because right after a restart the loop's presence check can't distinguish
a healthy agent from one hung on a dead socket — a just-restarted agent still
reports its last status (`idle`/`working`) until its 90 s presence mark lapses,
so an in-loop nudge gated on online-status would fire on healthy agents (a bug
found in testing). (The loop *does* carry a separate **30-min self-heal**, §5b,
but that fires only on genuinely long silence, well past this ambiguous window.)
Killing + relaunching the agents fresh would also recover them (the
stateless daemon makes the new handshake succeed), but it discards their context;
adoption + nudge preserves it.

**Why the daemon is stateless.** The MCP daemon is served with
`StreamableHttpServerConfig { stateful_mode: false }` (`crates/agora/src/web.rs`).
rmcp's default stateful mode keeps session ids in an in-memory
`LocalSessionManager`; because the daemon runs *in-process* with the server,
every restart wipes that map and any agent presenting an old `Mcp-Session-Id` is
rejected (`401 Session not found`; a no-session request gets `422 expect
initialize`) with no auto-re-handshake. Our tool surface is genuinely stateless —
identity is resolved per request from the `x-agent`/`x-room` headers and all
state is in SQLite — and the agent loop is pure request/response (`post`/`wait`),
so we need no server→client push. Stateless mode therefore costs nothing and lets
the nudged agents' fresh requests succeed with no `initialize`. See
`unresolved.md` (the resolved "rmcp reconnect" item) for the full root-cause
trail and the verified end-to-end test.

The agent config dialects (kiro agent JSON, claude
`--mcp-config`/`--settings`, Codex launch-time `-c` overrides), the keepalive
Stop-hook, and the role/goal prompts carry over from agora's verified launcher.
The MCP server the agents connect to is named **`team`** in their configs (so
MCP tool names are `mcp__team__hire` etc).

**Wait timeout boundary.** `wait` is one stateless Streamable HTTP request from
the agent CLI to the localhost MCP daemon; it does not pass through the phone's
WebSocket connection. The bus still caps each internal liveness slice at 50
seconds, but the MCP handler ignores empty slices and keeps the same tool call
parked for up to 240 seconds. A message broadcast wakes it immediately; 240
seconds is only the no-message ceiling. Codex's per-server `tool_timeout_sec`
and Claude's `MCP_TOOL_TIMEOUT` are both set to 270 seconds, leaving 30 seconds
of transport margin. Kiro's known failure mode is different: after the daemon
process dies, its old request may never time out; restart recovery therefore
cancels that call with `Esc` and starts a fresh stateless wait, as described
above.

This is deliberately implemented in the one shared in-process HTTP daemon, not
as one stdio proxy per agent. A stdio proxy would still be subject to the Agent
CLI's outer tool timeout while adding a process, pipes, memory, and another
restart lifecycle for every agent. The shared handler adds none of those. Each
waiting agent holds one lightweight async HTTP request. Broadcast provides
zero-poll message wakeup; the fallback presence/SQLite poll runs every 15
seconds instead of every second, remaining far below the 90-second stale
threshold while reducing idle database activity by about 15x.

All three backends run the keepalive command on their turn-complete/Stop
lifecycle. This is enforcement, not redundant prompting: Codex may correctly
reply, perform one 240-second `wait`, then end the turn when that wait is empty
despite an instruction to wait forever. Its Stop event therefore runs two
commands in parallel: `keepalive.sh` re-prompts the TUI into `wait`, while the
notification helper records completion. Kiro and Claude use the same keepalive
mechanism through their native hook dialects. The script sends literal prompt
text and Enter as separate tmux operations: Codex can keep the text in its
composer but drop an Enter sent in the same operation during the turn-complete
redraw. Hook scripts consume one JSON line from stdin rather than waiting for
EOF, avoiding a deadlock with CLIs that keep the hook pipe open until exit.
Generated commands run lifecycle scripts through `/bin/bash`; direct execution
can be killed by macOS provenance enforcement for app-created files.

Keepalive sustains the loop while the team is active. It does not prevent the
supervisor's deliberate all-idle sleep: once every agent is parked, the
supervisor interrupts their waits to stop token use and marks them `sleeping`;
the next human message nudges them back into `wait`. A Codex pane resting at its
prompt in that state is healthy, not disconnected.

**Authentication stays global even though runtime config is private.** Claude
Code keeps its normal home and receives Team config through command-line files,
so its global credentials/keychain remain available. Kiro authentication is
stored outside `KIRO_HOME` and likewise remains available when Team points that
variable at its private agent config. Codex authentication can depend on its
provider config, a provider `.env`, file login, or the OS keyring, all selected
through `CODEX_HOME`. Each private Codex home therefore links the system
`config.toml`, `.env`, and `auth.json` when present (`$CODEX_HOME`, otherwise
`~/.codex`); Team MCP settings are layered with CLI `-c` overrides instead of
rewriting that config. Links follow provider/token refreshes without copying
credential contents into the workspace. Missing files remain a no-op.

**Permission and first-use startup.** Team workspaces are explicitly selected by
the operator, so all three backends run without per-tool approval prompts:
Kiro uses `--trust-all-tools` plus
`chat.disableTrustAllConfirmation=true`; Claude uses
`--dangerously-skip-permissions` plus
`skipDangerousModePermissionPrompt=true`; Codex uses
`--dangerously-bypass-approvals-and-sandbox` and
`--dangerously-bypass-hook-trust`. Claude and Codex still have separate
folder-trust dialogs for a new workspace, and neither CLI exposes a public
interactive-mode flag that skips them. Their complete initial prompts are
therefore passed on the launch command, while a two-minute background watcher
confirms only when all backend-specific trust-dialog markers are visible. An
already-trusted workspace receives no synthetic startup keystroke. Recovery
uses the same strict marker sets before sending its normal Escape/reconnect
nudge, so a server restart can also release an agent that was parked at its
first-use folder-trust dialog. Detection uses a plain tmux capture: the normal
ANSI-preserving capture can insert color escapes inside visible prompt text and
break exact marker matching.

## 5b. Agent liveness, presence & self-heal

The first version conflated "is this agent's process alive?" with "did it touch
the bus recently?" — and a single 30 s presence TTL flipped any agent we hadn't
heard from to **offline**, which the UI filters out of the roster. The trap: a
*working* agent (it received messages and is now running tools / thinking) makes
**no** `wait`/`post` call for its whole turn, so a turn longer than 30 s — i.e.
any real coding turn — was misreported and the agent visibly **dropped from the
team** mid-task. Presence was only ever refreshed by bus calls, and the one
state most likely to outlast the TTL (working) was the state with no signal.

The fix is one **liveness clock** — `last_seen` — fed by three independent
sources, plus a tiered overlay that never silently removes a busy agent. The
display status is a five-rung ladder — **idle → thinking → working →
hardworking → stalled** (plus terminal `offline`):

1. **Parked in `wait`** → status `idle`; the wait loop refreshes `last_seen`
   about every 15 seconds, so an `idle` agent stays well inside the 90-second
   freshness window.
2. **`wait` delivers a message** → status `thinking`: it just received work and a
   quick reply is expected. **`post`** → `working` + `last_seen`.
3. **Heads-down working** → the agent's **tool hooks** POST `/api/heartbeat`
   (agora `web.rs`; resolves `x-agent`/`x-room` like `/mcp`, calls
   `Bus::heartbeat`, which sets status `working` + `last_seen`). Sustained tool
   activity is what promotes `thinking` → `working`. Wired on the per-tool /
   per-prompt hooks so a busy agent reports alive *between* `wait` calls. Kiro,
   Claude, and Codex all wire `PreToolUse`, `PostToolUse`, and
   `UserPromptSubmit` in their native hook dialect. The hook is
   `team/hooks/heartbeat.sh`: `pre` pulses immediately and starts one bounded
   per-agent lease that pulses every 30 seconds, `post` stops the lease and
   pulses once, and `pulse` handles a newly submitted prompt. This keeps a
   single legitimately long tool fresh for its entire run instead of only at
   its boundaries. Team bus tools are excluded so `wait` remains `idle` and the
   idle-sleep state machine still works. Curl has a 2-second cap and runs in the
   background. Stop on every backend and Claude's separate
   `PostToolUseFailure` event also clear the lease; a hard CLI crash that emits
   none of those events can retain it for at most 24 hours.
   The supervisor injects `TEAM_HB_URL`/`TEAM_AGENT`/`TEAM_ROOM` into every
   backend's launch env so the hook is self-contained.

**Tiered presence overlay** (`bus.rs::apply_presence`, read-time only —
`STALE_TTL_MS = 90 s` sits above a typical inter-tool/thinking gap so a
heartbeating agent never flaps; `STALLED_TTL_MS = 30 min`):

- stored `offline` (explicit `leave`/`fire`) → stays `offline` (UI hides it);
- fresh (`age ≤ 90 s`) → keep the real status (`idle`/`thinking`/`working`);
- `90 s < age ≤ 30 min` → **`hardworking`** — a `working`/`thinking` agent whose
  heartbeats stopped: head-down on a long tool / long think. An `idle`/`online`
  agent that goes stale here instead means its wait loop died, so it is surfaced
  as `stalled` directly. `hardworking` is **kept in the roster** (the chip/graph
  node turns orange, the pane preview still works) — explicitly *not* `offline`,
  so a quiet-but-busy agent is never "fired" by the UI again;
- `age > 30 min` → **`stalled`** (red) — needs a restart; the supervisor
  self-heals it (below).

Colors run a heat gradient: idle green → thinking blue → working amber →
hardworking orange (`--status-hot`) → stalled red.

**Self-heal backstop** (`team.rs::reconcile_loop`, `RECOVERY_STALE_MS = 30 min`).
Because a parked agent touches every 15 seconds and a working tool holds a
30-second heartbeat lease, `last_seen` older than 30 min means genuinely
*nothing* — a dead MCP socket, a crashed loop, a stop we never caught. For such
an agent (window still present) the supervisor runs the same `nudge_pane`
recovery uses — `Esc` to
cancel the wedged call, then a re-prompt to resume `wait` — **once**, then cools
down the same window for another 30 min so we never spam. Pure model thinking
between tools still has no lifecycle callback, but the pre/post/prompt pulses
reset the clock at every observable boundary; an active tool itself is never
interrupted merely for crossing the 30-minute threshold.

**Idle-sleep** (`team.rs::SleepState`, `IDLE_SLEEP_MS = 5 min`). The other
extreme: when **every** non-offline agent has been parked in `wait` (status
`idle`) for 5 min — the team has nothing to do — the supervisor sends `Escape`
to each pane, which cancels the in-flight `wait` MCP call. The CLI returns to
its shell prompt and stops thinking; without sleep the team would re-enter the
240-second coalesced `wait` forever, burning one fresh LLM turn on each
completion. In a normal idle cycle only one empty call completes; the next is
cancelled by five-minute sleep. Wake is anchored on bus seq: at sleep we
snapshot the latest message seq, and any strictly-greater seq on a later tick —
typically the human resuming the conversation — fires the standard `nudge_pane`
(Esc + reconnect re-prompt + Enter) at every pane to put them back in `wait`.
While slept the self-heal backstop is **off**: `last_seen` aging past 90 s into
`stalled` is *expected* (no live `wait` to refresh it) and must not be treated
as a wedged-agent signal, otherwise we would oscillate. Latency from a human
post to the team being live again is bounded by the 3 s reconcile tick.

When it sleeps, the supervisor also sets each agent's stored status to
`sleeping` (`Bus::set_status` → `store::set_status`); `apply_presence` treats
`sleeping` like `offline` — never aged into `stalled` — so the roster keeps the
label until wake (where the supervisor sets it back to `idle` and the agent's
own fresh `wait` refines it). The frontend renders `sleeping` as a sixth status
on the ladder (indigo `--status-sleep`, dimmed slow-breathing node in the
collab graph, a legend entry, and a roster-dot colour). It is **not** `offline`,
so a sleeping agent stays visible in the roster and graph rather than vanishing.

**Why `sleeping` is re-stamped every tick, not set once.** Our `Esc` takes
~1–2 s to actually cancel the agent's in-flight `wait`. In that window the wait
loop parks at least once more and writes `idle` (refreshing `last_seen`),
clobbering the `sleeping` we set on the Sleep tick. Set once, that stale `idle`
then ages into **`stalled`** (red) after 90 s — the exact bug observed: the team
*was* asleep but showed `stalled`. The fix: while `slept`, the reconcile loop
re-stamps `sleeping` on every 3 s tick. By the next tick the wait is truly
stopped, nothing else writes the row, and since `apply_presence` never ages
`sleeping`, the label sticks. The re-stamp is idempotent and cheap (one UPDATE
per agent); it refreshes `last_seen` harmlessly (sleeping is exempt from aging,
and wake keys off bus seq, not `last_seen`).

### Retired: the standalone `team/` Python launcher

`team/` once shipped a Python launcher (`run.py` + `supervise.py` +
`team_backends.py` + `team.yaml`) — the original agora demo path, kept for
headless use. It has been **removed**: `team.rs` is a faithful in-process Rust
port that does everything it did and more (multi-room via `x-room`, the
heartbeat hook, the private per-team runtime home), so the Python copy only
drifted out of sync and risked misleading the next reader. `team/` now holds only the
artifacts the app compiles in via `include_str!`: `AGENTS.md`, `hooks/`
(`keepalive.sh`, `heartbeat.sh`), and `templates/`. (The retired scripts live on
in this repo's gitignored `temp/team-standalone-py/` should anyone want the
reference.)

## 5c. Team definition: folders, YAML, env, MCP & skills

A team definition is a **folder** `~/.config/tmux-mobile/teams/<name>/` holding a
`team.yaml` (the roster + per-agent config) and optionally a `skills/` dir of
local skills. Built-ins ship the same shape in
`team/templates/<name>/team.yaml`, embedded via `include_str!` and seeded once
into the config dir (user edits never overwritten). The folder — not a flat
file — is the unit so a team can carry its own assets.

**Mixed engineering roster.** `mixed-engineering` is deliberately a lean fixed
team rather than another large all-purpose roster:

- **Kiro lead** owns requirements, acceptance criteria, decomposition, and
  handoffs. This follows Kiro's official
  [Specs](https://kiro.dev/docs/specs/),
  [Steering](https://kiro.dev/docs/steering/), and
  [custom-agent](https://kiro.dev/docs/cli/custom-agents/) model.
- **Claude architect/reviewer** explores before proposing the design, makes
  contracts explicit, then independently reviews the resulting diff. Claude
  Code's official [best practices](https://code.claude.com/docs/en/best-practices)
  and [subagent guidance](https://code.claude.com/docs/en/sub-agents) emphasize
  planning, context isolation, verification, and independent review.
- **Codex builder/verifier** receives a bounded implementation task with exact
  constraints and done criteria, then implements, tests, and reports command
  evidence. The current [Codex manual](https://developers.openai.com/codex/codex-manual.md)
  recommends explicit goals, context, constraints, done criteria, testing, and
  review, while reserving parallel writes for genuinely independent work.

This roster is fixed because runtime `hire()` currently creates Kiro agents
only; relying on dynamic hiring would silently lose the requested backend mix.
All three model fields stay blank so each CLI uses its configured system
default. Their global authentication is inherited as described in §5.

**Why a platform schema, adapted down.** We define ONE schema (`team.yaml`) and
translate it to each backend's dialect in `team.rs`, rather than exposing kiro/
claude/codex config directly. The top level carries **team-wide** fields that
apply to EVERY agent — `env`, `mcp`, `skills`, and `prompt` — and each agent adds
`name`/`backend`/`role`/`goal`/`model`/`manage` plus its own `env`/`mcp`/`skills`.
At seed time `seed_template` folds the team-wide config into each agent's spec:
env merges (agent overrides), and team `mcp`/`skills` are prepended so a per-agent
entry wins on a same-named MCP server (`merge_env` / `merge_list`). Team
`prompt` is passed to `build_agent_prompt` and reaches every agent through its
inline launch prompt. The separately editable global `system_prompt.md` is
loaded whenever a team starts and prepended before the shared rules and
team-specific prompt. Putting a shared tool (e.g. context7) at the team level
means writing it once instead of on every role. The full schema is documented at
the top of `default/team.yaml`, and the editor's per-template "Team-wide" section
edits env/mcp/skills/prompt.

- **model** — optional per agent. A non-empty value is passed through each
  backend's native `--model` flag. Blank uses the configured Team model for
  Kiro, `sonnet` for Claude, and the inherited Codex configuration. Values are
  not validated against a fixed list because all three CLIs can expose dynamic
  models or custom providers; the selected CLI reports unavailable model IDs.
- **env** — optional; default is none. Team-wide `env` is the base, per-agent
  `env` overrides it (`merge_env`). It's set on the agent's process at launch, so
  BOTH its MCP servers and skill use inherit it (backends do their own `$VAR`
  expansion; we don't interpolate, and we ship no secrets).
- **mcp** — extra MCP servers merged into the agent's config alongside the always-
  present `team` server. Remote (`url`+`headers`) or local (`command`+`args`+`env`).
  Header values may reference an environment variable as `$VAR` or `${VAR}`
  without exposing its value: Kiro/Claude receive `${VAR}` interpolation, while
  Codex receives `bearer_token_env_var` or `env_http_headers` overrides.
  Adapted per backend: `kiro_mcp_value` (kiro `{url,headers}` / `{command,args,env}`),
  `claude_mcp_value` (remote tagged `type:"http"`), `codex_mcp_overrides`
  (launch-time `-c mcp_servers.<name>...` values).
- **skills** — our own skill = a dir with a `SKILL.md` (YAML frontmatter
  name/description). A skill ref is either a **local path** (relative to the team
  folder) or a **GitHub URL**. `resolve_skills` turns each into a local dir:
  GitHub `tree/<ref>/<subpath>` URLs are **sparse-cloned** (`--depth 1
  --filter=blob:none --sparse` + `sparse-checkout set <subpath>`) into a shared
  cache `~/.config/tmux-mobile/skills-cache/<owner>/<repo>/<ref>/` — keyed so
  repeated refs reuse the clone, no re-pull. Adapted per backend: **kiro** gets a
  native `skill://<dir>/SKILL.md` resource (progressive load); **claude/codex**
  get a one-line skills index appended to their kick (name · description · path,
  "read the SKILL.md before a matching task") since they have no per-invocation
  skill flag in our launch path. Network/parse failures skip that skill, never
  fail the launch.

**Migration.** Legacy flat `teams/<name>.json` files are auto-migrated on startup
(`migrate_legacy_json`): converted to `teams/<name>/team.yaml`, the original
renamed `<name>.json.bak` (reversible). The frontend↔backend RPC contract is
unchanged — it still passes JSON; only the on-disk format is YAML
(`save_template` JSON→YAML, `read_team_def` YAML→JSON). The editor
(`TeamTemplates.svelte`) gained a per-agent "advanced" section for env/mcp/skills.

## 6. The obligation bug we hit and fixed

First live run spammed `@human 在线` dozens of times. Root cause: the human
operator is never a registered agent, so when the human's `@worker` message
(with `requires_reply`) obligated the worker, the worker's reply `@human …`
resolved to no registered recipient — `mentioned_names` dropped it — and the
debt **never cleared**. The worker stayed `Blocked` forever and re-replied every
tick (keepalive amplified it). Fix (`agora::bus::post`): debt discharge now
matches **any** raw `@`-mentioned creditor, registered or not, so an agent can
always answer the human. New obligations still only attach to registered agents.
Regression test: `can_discharge_debt_to_unregistered_human`.

## 7. Config

Desktop server reads (config.toml or env, `AGORA_*` accepted as legacy aliases):
`team_bind`/`TEAM_BIND` (default `127.0.0.1:8787`), `team_db`/`TEAM_DB` (default
`~/.config/tmux-mobile/team.db`), `team_room`/`TEAM_ROOM` (default `main`),
`team_model`/`TEAM_MODEL` (default `claude-sonnet-4.6`, used by Kiro agents whose
template model is blank). Bus startup is best-effort: on failure the terminal
server still runs and the Team tab stays hidden.

> Security: the MCP `/mcp` and `/api/*` have **no auth** — only the phone path is
> authenticated (it rides the token-authed WS). Keep `TEAM_BIND` on `127.0.0.1`
> unless you intend other LAN devices to reach the dashboard directly, and only
> on a trusted network.

## 8. Known edges / future

- **`requires_reply` defaulting.** The phone infers `requires_reply=true` from an
  `@mention`. After the §6 fix this is safe, but a casual "@worker 你在线吗" still
  creates a real obligation. Consider making it opt-in if teams feel too rigid.
- **Roster status colors**: idle / thinking / working / **hardworking** (orange)
  / **stalled** (red) — see §5b — plus human. There is still no "owes a reply" /
  deadlock indicator (the data is in `/api/quiescence`).
- **Backend verification**: the mixed Kiro / Claude / Codex roster and Codex
  wait recovery have been exercised locally. Actual provider availability still
  depends on each system CLI's global authentication.
- **History window**: the tab loads `team_history(200)`; the full log lives in
  SQLite and is mirrored to `<workspace>/.tmm/team-history.jsonl`.
