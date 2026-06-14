# Team Page

## Purpose
Collaborate with multiple coding agents (Kiro / Claude Code / Codex) from the
phone as one group chat, and jump from any agent to its live tmux execution
state. Backed by the **Team** multi-agent bus running in-process on the desktop
server. See `docs/design-docs/features/team.md` for architecture.

## Availability
- **Desktop server only.** The team bus is desktop-only; the phone reaches it
  through its existing WebSocket connection.
- The tab appears **only when** the connected server has the bus running (probed
  once per connection via the `team_status` RPC; method-not-found → hidden).
- A server with the bus present but no teams shows the "New team" panel.

## Multiple teams
- Each team is an isolated chat **room** (id = workspace slug) — separate
  conversation, roster, and agents. Many teams (one per project) coexist.
- **Header dropdown** lists active teams (room · live agent count); pick one to
  switch the chat + agent grid to it.
- **+ New team** opens the workspace picker → `team_start_team(workspace)`.
- **× Close team** stops the active team's agents (`team_close_team`, kills its
  tmux session); its chat log persists, so re-starting the same workspace
  resumes history.

## Components

### Roster strip (top, when a team is running)
- One chip per **present** agent (offline agents hidden; the human `human` is
  never shown as an addressable agent).
- Each chip: a status dot (waiting = green, working = amber, online = accent),
  the agent name, and a terminal glyph.
- **Tap a chip → preview that agent's tmux pane** in the Terminal tab. The agent
  runs in a window named after it in `tmm-team-<workspace-slug>`; the tab maps
  name → pane via `window_name` and opens it through the normal terminal path.

### Start panel (until a team is running)
- **Workspace field** — the agents' shared working directory. Defaults to the
  current terminal session's cwd (else the server's home); tap to edit before
  starting. This is the directory the team is limited to.
- **Start team** button → `team_start_team(workspace)`: the desktop server seeds
  the built-in roster (manager / worker / reviewer) and launches each agent into
  its own named window of `tmm-team-<slug>`, all in-process. Shows "agents coming
  online…" until they join.
- Multiple workspaces → multiple independent teams (`tmm-team-<slug>` each).

### Message log (middle)
- Group-chat transcript, oldest at top, newest at bottom (auto-scrolls).
- The human's own messages are right-aligned with accent styling; others are
  left-aligned with the sender name shown.
- `join`/`leave`/`system` messages render as centered system notices.
- Loaded from `team_history` on open; new messages arrive live via the
  `team_message` push (de-duplicated by message id).

### Compose (bottom)
- A row of `@name` quick-mention chips (one per present agent) above the input.
- A growing textarea + a round send button.
- **Enter sends** (desktop); Shift+Enter inserts a newline. The send button is
  the primary path on a soft keyboard.
- Posting with an `@mention` requires that agent to reply (the obligation rule);
  a plain message is an informational broadcast. You can always discharge a debt
  to anyone you `@`-mention, including the human.
- The sent message is NOT appended locally — it echoes back via the live push,
  so there is never a duplicate.

## Behavior notes
- The human always posts as `human` (matches the bus's dashboard/CLI convention).
- Reconnects re-probe availability and re-load history.
- Launching is **in-process** (the Start team button); the optional `team/`
  Python scripts exist only for advanced/headless custom rosters (see the design
  doc).
