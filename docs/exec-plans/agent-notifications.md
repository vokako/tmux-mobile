# Agent Notifications Execution Plan

## Problem

Coding agents running in ordinary tmux windows can finish or require attention
while the user is viewing another window or using the mobile client. Pane output
polling carries bytes but no reliable completion or permission semantics, so the
UI cannot show a durable attention marker.

## Candidate Approaches

1. Parse terminal output. Rejected because prompts and messages are unstable and
   localized.
2. Use tmux activity/bell hooks. Rejected as the primary mechanism because they
   cannot distinguish completion, input, permissions, and ordinary output.
3. Normalize native agent lifecycle hooks through a local inbox. Chosen because
   all three supported agents expose completion hooks, Claude and Codex expose
   attention hooks, and `TMUX_PANE` gives an exact tmux identity without adding a
   network listener.

## Acceptance Criteria

- Claude, Codex, and Kiro hook payloads normalize to one notification schema.
- A valid event for a live tmux pane persists as unread at window scope.
- Authenticated clients receive new events and a reconnecting client can list
  existing unread state.
- Opening a notified window clears it for all connected clients.
- Settings reports hook installation state and installs/removes only entries
  owned by tmux-mobile.
- Team-generated agent configurations include notification hooks.
- Existing hooks and unrelated agent settings remain unchanged.
- Hooks are inert outside tmux and never report failure when the desktop server
  or local inbox is unavailable.
- Reinstalling repairs stale tmux-mobile-owned helper commands.

## Files

- `src-tauri/src/agent_notifications.rs`
- `src-tauri/src/server.rs`
- `src-tauri/src/lib.rs`
- `src-tauri/src/bin/server.rs`
- `src-tauri/src/team.rs`
- `src/lib/core/agent-notifications.svelte.js`
- `src/lib/core/ws.js`
- `src/App.svelte`
- `src/lib/sessions/Sessions.svelte`
- `src/lib/terminal/Terminal.svelte`
- `src/lib/app/Settings.svelte`
- relevant requirements and design documentation

## Proof

```bash
node --test src/lib/*.test.js
npm run build
cd src-tauri && cargo test -- --test-threads=1
```

Focused Rust tests cover payload normalization, deduplication/persistence, and
additive hook configuration merging before the full suite runs.
