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
after a 240-second total budget. This reduces a five-minute idle period from
roughly six model/tool turns to one before the existing idle-sleep cancels the
next wait. Codex and Claude receive 270-second outer tool timeouts; Kiro's
observed client has no shorter timeout and remains covered by restart recovery.

Additional proof:

- A short-budget test must span multiple idle slices before returning `Idle`.
- A message posted after an idle slice must complete the original tool call,
  not require the agent to issue another `wait`.

## Follow-up: dead client-side wait recovery

### Problem

A live incident in room `260717-lingting-687470` showed human messages persisted
in SQLite and delivered to two agents, while the lead's cursor and `last_seen`
stopped advancing. Its TUI displayed one `wait` call for over 50 minutes even
though the server caps the handler at four minutes. The MCP client transport
can therefore remain hung after the server future or connection is gone.

### Approaches considered

1. Add a stdio MCP proxy that retries HTTP calls. Rejected for the same reason
   as above: another process and protocol hop still cannot force a wedged Agent
   CLI tool call to consume the retry result.
2. Reduce every wait duration. Rejected: it increases model/tool churn and does
   not repair a client transport that never observes completion.
3. Use the existing presence distinction to recover dead waits. Chosen:
   `idle` waits refresh every 15 seconds and become `stalled` after 90 seconds,
   while real work becomes `hardworking` and retains the 30-minute protection.

### Done when

- A stale `stalled` wait is eligible for a reconnect nudge after 90 seconds.
- A `hardworking` agent is not eligible before the 30-minute backstop.
- Sleeping/offline agents and recently nudged panes are excluded.
- Focused tests and the complete Rust test suite pass.

### Files

- `src-tauri/src/team.rs`
- `docs/design-docs/features/team.md`
- `docs/exec-plans/team-wait-history.md`
