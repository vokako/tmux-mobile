# Team wait timeout and workspace history

## Problem

Team agents long-poll the in-process HTTP MCP server through `wait`, while chat
messages are stored in the global Team SQLite database. Two gaps need closing:

1. Codex's documented default MCP tool timeout is 60 seconds, only 10 seconds
   above the bus's 50-second wait cap. Normal localhost calls fit, but the
   intended relationship is implicit.
2. `close_team` and a later explicit start call `reset_room`, which deletes the
   SQLite message log despite the Team requirements saying history survives.
   There is also no workspace-local transcript that a newly launched agent can
   read to recover the previous team's context.

Relevant sources read: `docs/design-docs/features/team.md`,
`docs/requirements/pages/team.md`, `docs/unresolved.md`,
`src-tauri/crates/agora/src/{bus,mcp,store,web}.rs`,
`src-tauri/src/{team,team_bridge,server}.rs`, and the current Codex manual's MCP
configuration reference.

## Approaches considered

1. Shorten every wait to 20-30 seconds. This creates more empty tool turns and
   token churn, while not fixing recovery after a dead server connection.
2. Keep the 50-second server cap and explicitly configure clients whose timeout
   is known and close to it. Use the existing recovery nudge for dead
   connections. Initially chosen, then refined below because each empty slice
   still caused another model turn.
3. Move the authoritative database into each workspace. Rejected: one
   TeamManager intentionally shares one WAL connection across rooms, and
   per-workspace databases would complicate routing and migrations.
4. Keep SQLite authoritative and mirror each room into
   `<workspace>/.tmm/team-history.jsonl`. Chosen: SQLite retains transactional
   state, while agents get a durable, readable, self-gitignored transcript next
   to their private runtime files.

## Done when

- Closing or explicitly restarting a team clears roster/obligation/employee
  runtime state but preserves its SQLite messages.
- Registering a room rebuilds `.tmm/team-history.jsonl` from all stored messages;
  live messages append once, and a lagged broadcast rebuilds the mirror.
- Every launched agent is told to read the transcript before acting when it
  contains prior context.
- Codex's generated Team MCP configuration sets a tool timeout above the
  bus's 50-second maximum.
- Focused tests and `cd src-tauri && cargo test -- --test-threads=1` pass.

## Files

- `src-tauri/crates/agora/src/store.rs`
- `src-tauri/crates/agora/src/bus.rs`
- `src-tauri/src/team_bridge.rs`
- `src-tauri/src/team.rs`
- `docs/requirements/pages/team.md`
- `docs/design-docs/features/team.md`

## Follow-up: coalescing idle wait slices

A stdio MCP proxy was considered as a way to retry timed-out HTTP waits without
waking the agent. It does not remove the outer Agent CLI's MCP tool timeout and
would add another process, protocol hop, and lifecycle to supervise.

Instead, the MCP `wait` handler treats the bus's 50-second timeout as an
internal liveness slice. It ignores `Idle` outcomes and starts another slice
inside the same tool call, returning only when a message/obligation arrives or
after a 180-second total budget. This reduces a five-minute idle period from
roughly six model/tool turns to two, with the second cancelled by the existing
idle-sleep. Codex and Claude receive 210-second outer tool timeouts.

The first implementation used 240 seconds. A live Kiro incident then stopped
refreshing `last_seen` almost exactly four minutes after its preceding post:
the server deadline and Kiro's apparent client boundary raced, and the TUI
remained stuck even though the server handler had ended. A 180-second budget
keeps most of the resource win over 50-second calls while leaving 60 seconds
before that observed boundary.

Additional proof:

- A short-budget test must span multiple idle slices before returning `Idle`.
- A message posted after an idle slice must complete the original tool call,
  not require the agent to issue another `wait`.

## Follow-up: dead client-side wait recovery

### Problem

A live incident in room `260717-lingting-687470` showed human messages persisted
in SQLite and delivered to two agents, while the lead's cursor and `last_seen`
stopped advancing. Its TUI displayed one `wait` call for over 50 minutes. The
lead's liveness stopped almost exactly four minutes after its preceding post,
matching the then-240-second server budget and exposing a client/server deadline
race. The MCP client transport can also remain hung after a connection is gone.

### Approaches considered

1. Add a stdio MCP proxy that retries HTTP calls. Rejected for the same reason
   as above: another process and protocol hop still cannot force a wedged Agent
   CLI tool call to consume the retry result.
2. Revert every wait to 50 seconds. Rejected: it restores roughly six empty
   model/tool turns per five idle minutes.
3. Use the existing presence distinction to recover dead waits. Chosen:
   `idle` waits refresh every 15 seconds and become `stalled` after 90 seconds,
   while real work becomes `hardworking` and retains the 30-minute protection.
4. Move the coalesced server deadline from 240 to 180 seconds. Chosen: it keeps
   most of the call-reduction benefit while leaving 60 seconds before Kiro's
   observed boundary.

### Done when

- A stale `stalled` wait is eligible for a reconnect nudge after 90 seconds.
- A `hardworking` agent is not eligible before the 30-minute backstop.
- Sleeping/offline agents and recently nudged panes are excluded.
- A normal empty wait returns at 180 seconds, before Kiro's 240-second boundary;
  Codex and Claude retain a further 30 seconds of configured transport margin.
- Focused tests and the complete Rust test suite pass.

### Files

- `src-tauri/src/team.rs`
- `docs/design-docs/features/team.md`
- `docs/exec-plans/team-wait-history.md`

## Follow-up: non-interrupting restart recovery

### Problem

Backend restart recovery previously sent `Esc` and a reconnect prompt to every
adopted Agent window after two seconds. That repairs a dead `wait`, but also
cancels healthy thinking, tool execution, or editing that happened to span the
restart.

### Chosen approach

Snapshot each adopted Agent's persisted `status + last_seen`, allow one
15-second wait heartbeat plus margin, and compare again after 20 seconds. Nudge
only idle-like (`idle`/`online`/`stalled`) Agents whose timestamp did not
advance. Active, sleeping, offline, unknown, and already-recovered Agents are
left untouched. Trust confirmation remains independent of presence, and the
recovery text is the neutral `Continue.` rather than an instruction to abandon
work and enter a wait-only loop.

### Done when

- Restart recovery never sends `Esc` to working/thinking/hardworking Agents.
- An unchanged dead idle wait still reconnects.
- A wait that heartbeats during the grace period is not touched.
- Trust prompts can still be confirmed without roster presence.
- The complete room transcript remains mirrored at
  `.tmm/team-history.jsonl`. The later bounded-history follow-up supersedes
  direct preload instructions in Agent prompts.

## Follow-up: client boundary probes and zero-turn idle

### Problem

The 180-second workaround still completed one empty tool turn before the
then-five-minute idle-sleep threshold, and the earlier Kiro boundary was inferred
from an incident rather than its actual timeout configuration.

### Approaches considered

1. Use day- or week-scale waits for every client. Rejected as a common value:
   Codex and Claude support it, but Kiro does not.
2. Add a retrying stdio proxy per agent. Rejected again: it adds one process and
   protocol hop per agent while remaining subject to the outer client timeout.
3. Use the largest coordinated Kiro-safe budget. Chosen: ten-minute client
   timeouts, a nine-minute server budget, and an eight-minute idle-sleep derived
   from that budget. Normal idle teams are cancelled into sleep one minute
   before the first empty wait completes, so they consume no repeated model
   turns.

### Evidence

- Kiro 2.12.x source defines `MAX_MCP_TIMEOUT_MS = 600000` and uses the
  per-server `timeout` field. A 330-second silent HTTP tool completed at that
  value; values above the cap were rejected before the request reached the
  server.
- Codex 0.144.4 completed the 330-second probe with
  `tool_timeout_sec=600.0`; its duration parser also accepted week-scale values.
- Claude Code 2.1.201 completed the probe with a 600000 ms per-server timeout,
  `MCP_TOOL_TIMEOUT=600000`, and
  `CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT=0`.

### Done when

- Kiro, Claude, and Codex generated Team configurations all carry a ten-minute
  tool timeout.
- Claude's independent network MCP idle timeout is disabled.
- The server wait budget is nine minutes, leaving one minute of delivery margin.
- Idle-sleep is derived as eight minutes, one minute before that wait completes.
- The focused timeout/config tests and complete Rust test suite pass.

## Follow-up: runtime path and Terminal notification cleanup

### Problem

The Kiro private runtime directory is still named `.tmm/kiro-home`, unlike the
`.tmm/claude` and `.tmm/codex` directories. Team Agent lifecycle notifications
also render attention dots in Terminal chrome, where the Team page already owns
the collaboration status and the duplicate marker is distracting.

### Approaches considered

1. Rename every legacy Kiro directory during server startup. Rejected: an
   adopted Kiro process may still be using that path, so startup migration could
   move files underneath a live Agent.
2. Use `.tmm/kiro` for new launches and ignore `.tmm/kiro-home`. Rejected:
   existing Kiro state would be stranded.
3. Migrate immediately before launching a Kiro Agent, and only when the legacy
   directory exists and `.tmm/kiro` does not. Chosen: it preserves state,
   never overwrites a canonical directory, and does not touch adopted panes.
4. Drop Team notifications at ingestion. Rejected: hooks, persisted unread
   state, system notifications, and non-Terminal consumers must remain intact.
   The chosen implementation filters only Terminal switcher and pane-picker
   presentation.

### Done when

- A legacy `.tmm/kiro-home` is renamed to `.tmm/kiro` before a new Kiro launch,
  preserving its contents.
- An existing `.tmm/kiro` is never overwritten and leaves a legacy directory
  untouched for manual reconciliation.
- Team notification dots are absent from Terminal session/window chrome and the
  all-session pane picker; ordinary session dots are unchanged.
- `Continue.` remains the exact recovery prompt.
- Focused Node/Rust tests, the complete Rust suite, `npm run build`, and
  `git diff --check` pass without restarting the server.

### Files

- `src-tauri/src/team.rs`
- `src/lib/terminal/Terminal.svelte`
- `src/lib/sessions/PanePicker.svelte`
- `src/lib/terminal/terminal-team-notifications.test.js`
- `docs/requirements/pages/terminal.md`
- `docs/design-docs/features/team.md`
- `docs/design-docs/features/agent-notifications.md`
- `docs/unresolved.md`

## Follow-up: bounded on-demand Agent history

### Problem

The internal `TEAM_ADDRESSING_CONTRACT` repeats collaboration policy already
present in the user-visible `config.toml` `team_rules`, with slightly different
rules for unsolicited action. Its resume instruction also tells every Agent to
read the complete JSONL transcript, so a long-running room can consume a large
provider context before the Agent knows whether old history is relevant.

### Approaches considered

1. Keep telling Agents to read `.tmm/team-history.jsonl`. Rejected: the file is
   intentionally complete and has no context-size bound.
2. Return a fixed recent history window. Better, but an Agent cannot recover an
   older decision without increasing the one-shot context load.
3. Expose a bounded, cursor-paginated `read_history` MCP tool. Chosen: default
   pages are small, sequence cursors make older-page traversal stable, and the
   Agent loads additional context only when needed. The old `history` route
   remains callable but is omitted from tool discovery for compatibility.

The editable `team_rules` remains the sole collaboration-policy source.
Hardcoded prompt text is reduced to runtime facts: use `read_history` only when
earlier context is missing, request only enough pages, and do not preload the
complete transcript.

### Done when

- `read_history` returns the newest 20 messages by default, accepts at most 100,
  and supports exclusive `before_seq` pagination toward older messages.
- Responses include message sequence numbers and an exact next-page hint only
  when older messages exist.
- Tool discovery exposes `read_history`, not the compatibility `history` alias.
- Agent startup no longer instructs providers to read the full JSONL archive.
- Focused pagination/tool-list/prompt tests and the complete Rust suite pass.

### Files

- `src-tauri/crates/agora/src/store.rs`
- `src-tauri/crates/agora/src/bus.rs`
- `src-tauri/crates/agora/src/mcp.rs`
- `src-tauri/src/team.rs`
- `docs/requirements/pages/team.md`
- `docs/design-docs/features/team.md`
