# Team instance isolation

## Problem

The workspace slug currently identifies both a project directory and a Team.
Starting a second template in the same directory therefore reuses the first
Team's SQLite room, desired roster, tmux session, backend homes, hooks, and
history mirror.

## Approaches considered

1. Add only the template to the SQLite room. Rejected because tmux and `.tmm`
   state would still collide.
2. Allocate a random ID for every launch. Rejected because closing and
   restarting the same workspace/template would no longer recover its history.
3. Derive one stable Team ID from canonical workspace + template. Chosen because
   distinct templates are isolated while the same pair remains resumable and
   idempotent.

Legacy live rooms keep their workspace-only ID and root `.tmm` paths during
recovery. New rooms use `.tmm/teams/<team-id>/`, preventing an upgrade from
moving files under a running process.

## Done when

- Two templates started in one workspace receive different room/session IDs.
- Their SQLite messages, employee definitions, backend homes, hooks, and JSONL
  history mirrors are disjoint.
- Starting an already-active workspace/template returns that existing Team,
  including a recovered legacy room.
- Existing workspace-only tmux sessions still recover without moving their
  runtime files.
- Closing and relaunching a legacy or new Team retains its persisted room
  identity and history while closed Teams remain absent from the active UI.
- Focused isolation tests, the complete Rust suite, frontend tests, and the
  production build pass.

## Files

- `src-tauri/src/team.rs`
- `src-tauri/src/team_bridge.rs`
- `src-tauri/src/server.rs`
- `src/lib/Team.svelte`
- `docs/requirements/pages/team.md`
- `docs/design-docs/features/team.md`

## Status

Implemented and verified. Same-workspace rooms now have isolated bus/runtime
state, close/relaunch retains each room identity, and the live legacy Team on
this machine remains on its original session and `.tmm` layout.
