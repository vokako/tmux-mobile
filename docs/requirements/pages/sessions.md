# Sessions Page

## Purpose
Browse all tmux sessions, windows, and panes. Create or kill sessions. Entry point for selecting which pane to view.

## Components
- Session list with expand/collapse
- Pane entries showing command name, AI agent icons (Kiro/Claude)
- New session / new window buttons
- Kill session / kill window buttons with confirmation
- Pull-to-refresh (mobile)

## Interactions
- Tap session → expand/collapse pane list
- Tap pane → navigate to Terminal view, subscribe to pane
- Tap new session → `new_session` RPC (supports optional path and command)
- Tap new window → `new_window` RPC
- Tap kill session → confirmation → `kill_session` RPC
- Tap kill window → confirmation → `kill_window` RPC
- Pull down → refresh session list

## API Calls
- `list_sessions` — list all tmux sessions
- `list_panes(session)` — list panes in session
- `new_session(name?, path?, command?)` — create session
- `kill_session(name)` — kill session
- `new_window(session)` — create window in session
- `kill_window(target)` — kill window
- `pane_command(target)` — get running command (for AI agent detection)

## State Management
- Sessions array with expanded state
- Active session/pane selection
- Sort order: (1) the session currently being viewed on the Terminal tab,
  (2) sessions sorted by `last_opened` descending (unix seconds of the last
  time the session was opened via tmux-mobile; persisted server-side),
  (3) sessions that have never been opened from the app, in the server's
  baseline order (tmux `session_activity` descending).

## Edge Cases
- Auto-expand sessions on load
- AI agent detection by command name (kiro, claude)
- `last_opened` survives across server and client restarts (stored in
  `~/.config/tmux-mobile/session_usage.json`). A session deleted and
  recreated with the same name will inherit the old record; acceptable.
- First-time use (no timestamps yet): all sessions fall back to the server's
  baseline order, matching the previous behavior.
