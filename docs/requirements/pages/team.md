# Team Page

## Purpose
Collaborate with multiple coding agents (Kiro / Claude Code / Codex) from the
phone as one group chat, and jump from any agent to its live tmux execution
state. Backed by the **agora** multi-agent bus running in-process on the desktop
server. See `docs/design-docs/features/team-agora.md` for architecture.

## Availability
- **Desktop server only.** The agora bus is desktop-only; the phone reaches it
  through its existing WebSocket connection.
- The tab appears **only when** the connected server has the bus running (probed
  once per connection via the `agora_roster` RPC; method-not-found → hidden).
- A server with the bus present but no agents shows the tab with an empty roster.

## Components

### Roster strip (top)
- One chip per **present** agent (offline agents hidden; the human `human` is
  never shown as an addressable agent).
- Each chip: a status dot (waiting = green, working = amber, online = accent),
  the agent name, and a terminal glyph.
- **Tap a chip → preview that agent's tmux pane** in the Terminal tab. The agent
  runs in a tmux window named after it; the tab maps name → pane via
  `window_name` and opens it through the normal terminal path.

### Message log (middle)
- Group-chat transcript, oldest at top, newest at bottom (auto-scrolls).
- The human's own messages are right-aligned with accent styling; others are
  left-aligned with the sender name shown.
- `join`/`leave`/`system` messages render as centered system notices.
- Loaded from `agora_history` on open; new messages arrive live via the
  `agora_message` push (de-duplicated by message id).

### Compose (bottom)
- A row of `@name` quick-mention chips (one per present agent) above the input.
- A growing textarea + a round send button.
- **Enter sends** (desktop); Shift+Enter inserts a newline. The send button is
  the primary path on a soft keyboard.
- Posting with an `@mention` in the body makes the bus require that agent to
  reply (the obligation rule); a plain message is an informational broadcast.
- The sent message is NOT appended locally — it echoes back via the live push,
  so there is never a duplicate.

### Start team (empty-roster state)
- When the bus is up but no agents are online and none have been launched, the
  roster strip shows a **Start team** button.
- Tapping it calls `agora_start_team`: the desktop server seeds the built-in
  roster (manager / worker / reviewer) and launches each agent into its own
  named tmux window — all in-process, no separate script. The button then shows
  "agents coming online…" until they join.

## Behavior notes
- The human always posts as `human` (matches agora's dashboard/CLI convention).
- Reconnects re-probe availability and re-load history.
- Launching is **in-process** (the Start team button); the optional `team/`
  Python scripts exist only for advanced/headless custom rosters (see the
  design doc).
