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
- Each team is an isolated chat **room** whose stable id is derived from the
  canonical workspace + selected template. Conversation, roster, Agent runtime
  files, and tmux session are all scoped by that same id. Different templates
  may therefore run concurrently in one project directory.
- **Header dropdown** lists active teams (room · live agent count); pick one to
  switch the chat + agent grid to it. Live agent status chips sit on the same
  header row (dot + name; wrap on overflow; tap → preview pane).
- **+ New team** opens the workspace picker (folder browser) + a **roster
  template** picker → `team_start_team(workspace, template)`.
- **× Close team** stops the active team's agents (`team_close_team`, kills its
  tmux session); its chat log persists, so re-starting the same
  workspace+template pair resumes history.

## Roster templates
- A team launches from a **named roster template** — a folder
  `<config>/tmux-mobile/teams/<name>/team.yaml` (legacy flat `<name>.json` is
  auto-migrated). Schema: optional **team-wide** `env` / `mcp` / `skills` /
  `prompt` (applied to every agent) plus `agents:[{ name, backend, role, goal,
  model, manage, env?, mcp?, skills? }]`. Built-ins (`default`, `software-dev`,
  `financial-research`, `deep-research`, `content-studio`, `data-analysis`,
  `mixed-engineering`) are seeded on first run. `mixed-engineering` is the
  built-in mixed-backend roster: Kiro lead, Claude architect/reviewer, and
  Codex builder/verifier.
- The new-team panel picks which template to use (with a folder browser that can
  create + select a new workspace folder).
- An **editor** (edit button beside the picker) adds/renames/deletes templates
  (tap-to-confirm), edits every agent field with clear labels, has a per-agent
  **advanced** section (env / extra MCP servers / skills) and a **Team-wide**
  section (env/mcp/skills/prompt). On phones it is a near-fullscreen sheet: the
  template list collapses into a labelled dropdown and the global system prompt
  collapses. (`team_templates` / `team_template_save` / `team_template_delete`.)
- An empty `model` uses the backend launcher's default: the server's Team model
  for Kiro, `sonnet` for Claude, and the global Codex configuration for Codex.

## Global system prompt
- A single shared prompt at `<config>/tmux-mobile/system_prompt.md`, edited at
  the top of the template editor (`team_system_prompt_save`).
- It is **prepended to the brief every agent reads at startup**, across all
  teams/roles — for project-wide conventions, tone, language preference, etc.
  Empty by default (no-op).
- Final prompt order is `system_prompt.md` → user-visible `config.toml`
  `team_rules` → template prompt → role/goal. Each layer has one owner:
  `team_rules` is the cross-template collaboration contract, template text
  defines roster-specific routing/workflow, and role/goal defines one Agent's
  responsibility.
- Kiro receives that prompt through its custom Agent `prompt`; Claude through
  `--append-system-prompt`; Codex through `developer_instructions` after any
  existing user developer instructions. The initial user message is only the
  visible `config.toml` `team_kick` lifecycle command.

## Components

### Roster strip (top, when a team is running)
- One chip per **present** agent (offline agents hidden; the human `human` is
  never shown as an addressable agent).
- Each chip: a status dot (idle = green, thinking = blue, working = amber,
  hardworking = orange, stalled = red, sleeping = muted), the agent name, and a
  terminal glyph.
- **Tap a chip → preview that agent's tmux pane** in the Terminal tab. The agent
  runs in a window named after it in `tmm-team-<team-id>`; the tab maps
  name → pane via `window_name` and opens it through the normal terminal path.

### Start panel (until a team is running)
- **Workspace field** — the agents' shared working directory. Defaults to the
  current terminal session's cwd (else the server's home); tap to edit before
  starting. This is the directory the team is limited to.
- **Start team** button → `team_start_team(workspace, template)`: the desktop
  server seeds the selected roster and launches each agent into its own named
  window of `tmm-team-<team-id>`, all in-process. Shows "agents coming online…"
  until they join.
- A workspace+template pair is idempotent and resumes the same Team history;
  another template in that workspace gets a separate Team.

### Message log (middle)
- Group-chat transcript, oldest at top, newest at bottom (auto-scrolls).
- The human's own messages are right-aligned with accent styling; others are
  left-aligned with the sender name shown.
- `join`/`leave`/`system` messages render as centered system notices.
- Loaded from `team_history` on open; new messages arrive live via the
  `team_message` push (de-duplicated by message id).
- The complete room log remains authoritative in the Team SQLite database
  across close/relaunch. It is also mirrored as JSON Lines at
  `<workspace>/.tmm/teams/<team-id>/team-history.jsonl`, so replacement agents
  can inspect that Team's decisions and handoffs without seeing sibling Teams.
- Agents recover missing context through the Team MCP
  `read_history(limit, before_seq)` tool instead of preloading that complete
  file. The default page is 20 messages, a page is capped at 100, and responses
  provide an exclusive sequence cursor only when older messages remain. This
  bounded recovery behavior lives in the tool's own description rather than a
  second hidden runtime prompt.

### Compose (bottom)
- A row of `@name` quick-mention chips (one per present agent) above the input.
- A growing textarea + a round send button. The empty textarea is always at
  least one complete text row tall and shows no horizontal or vertical
  scrollbar. It grows with content and scrolls internally only after its cap.
- On desktop, a slim drag handle on the composer's top edge adjusts the
  textarea's base height from 40–320 px (keyboard-accessible with Up/Down). The
  height is remembered locally; mobile ignores it and retains auto-grow.
- **Enter sends** (desktop); Shift+Enter inserts a newline. The send button is
  the primary path on a soft keyboard.
- Human `@name` posts default to requiring that agent's reply. `@all` always
  requires every other registered agent to reply, even if an agent-side caller
  omits `requires_reply`. Plain messages are informational broadcasts; wording
  such as "everyone" is not inferred as dispatch. Mentioning a creditor,
  including the human, discharges the reply obligation.
- The sent message is NOT appended locally — it echoes back via the live push,
  so there is never a duplicate.

## Behavior notes
- The human always posts as `human` (matches the bus's dashboard/CLI convention).
- Reconnects re-probe availability and re-load history.
- Launching is **in-process** (the Start team button); the optional `team/`
  Python scripts exist only for advanced/headless custom rosters (see the design
  doc).
