# Hub Page (project chat & agent collaboration)

## Purpose
The desktop workbench for **agents-v2**: one chat room per project where the
human and its managed agents collaborate, with the roster, live telemetry, a
terminal/files/board drawer, and every agent-management verb in one place.
This is the surface the `tmm` CLI talks to from the other side — CLI/UI
parity is a standing rule: every verb here exists as a `tmm` command, because
an agent that can only be managed by a human cannot manage a teammate.
See `docs/design-docs/features/tmm-cli.md` (architecture & rationale) and
`docs/design-docs/features/agent-lifecycle.md` (spawn → conversation →
completion).

## Availability
- **Desktop only** (`hubEligible`): the hub tab never renders on the phone;
  compact (≤760px) windows get the drill-down layout (project list ⇄
  conversation).
- Requires the desktop server (hub RPCs are method-not-found elsewhere).

## The model (business summary)
- **A project is a directory + a tmux session**; every session is a project
  (auto-adopted). The chat **room is recorded on the project** and survives
  renames. See `docs/design-docs/features/projects.md`.
- **Agents are windows** in the project session. A *managed* agent was
  spawned by the app into an isolated home (`<ws>/.tmm/agents/<name>/`) from
  a **registry definition** (backend, model, effort, system prompt, skills,
  MCP); a *direct* window (hand-started) is observed but never typed into.
- **What an agent SAYS** goes through the `tmm` CLI; **what we OBSERVE**
  arrives via hooks (turn open, tool calls, ask, stop) and status is
  **derived** (`running | waiting | idle | failed`).
- **Messages move five ways** (taught to every agent in its system prompt):
  into an agent as stamped prompts typed into its pane (queued mid-turn);
  out of it automatically (captured final reply + `tmm done` summary
  delivered to whoever briefed it); addressed sends (`@name` — interrupts);
  unaddressed sends (room-only, nobody interrupted); and the room memory
  (`tmm log` — an agent only receives what is addressed to it).
- **The task board** is the project's plan of record: the human writes
  issues here, agents keep them current via `tmm board`
  (todo/doing/review/done, fixed vocabulary).

## Sidebar (projects)
- Ordered by **conversation recency** (newest message per room), live-only
  projects without talk underneath; each row shows the last-reply age and
  the project's agents as quiet mono chips with state dots (shared atoms
  with the Terminal sidebar — `.side-*` in app.css).
- Create (`+`) requires a name; the path comes from the DirPicker (never
  typed from memory). Delete goes through the **recycle bin** (archive →
  restore is free; the confirmed delete is the only irreversible step).
- The open project persists (`tmux_hub_project`) and is re-verified against
  the list on load.

## Roster (agent cards)
- One card per live window; managed agents carry the hook-derived state dot
  (at-rest is achromatic grey; running wears the `.live-dot` halo + breathe),
  sniffed **vitals** (model + effort at `--fs-micro`, context% as a 2px
  bottom edge line), and a context menu (Watch / Interrupt / Stop / Restart /
  Remove / configure agent — rising consequence order).
- A **stopped** agent (slot, no window) keeps its card and menu (Start /
  Remove). The `+ agent` card is the strip's sticky last card and renders
  for every selected project — empty roster and closed session included
  (spawn into a down project opens it).
- Tapping a card makes that agent the composer's **recipient** (and the
  remembered project lead — `pickLead`).

## Feed (the conversation)
- Telegram-like bubbles: name header, floated time trailer, delivery ring on
  own messages; `[tmm status]` / `[tmm done]` markers render as ordinary
  bubbles with a state badge; `[tmm] ` lifecycle lines fold into one sys
  capsule (dropped at the chat-only detail level); tool calls fold into one
  lane per turn (configurable row cap, middle column scrolls, never
  truncated).
- Long user messages fold at the rear (`elideTail`) with an in-bubble
  expand; ONE user-message anchor pins the reading position (never while
  expanded). Images render as thumbnails and open the in-app Lightbox;
  markdown + LaTeX render in bubbles.
- Feed levels: chat-only / +tools / everything. Receipts and warnings are
  always visible.
- Room state is cached per session (`roomCache`) — switching projects
  restores instantly, "empty" is a verdict reached only after the first
  `hub_log` answer.

## Composer
- ONE capsule: recipient chip (`@name`, always prefixed on send), auto-grow
  textarea, up-arrow send. Drafts are per-project and survive reload.
- `/` opens the **command palette** in the addressee's dialect (transcribed
  from each CLI, view-commands filtered out; fuzzy 3-tier matching); a
  command-shaped draft styles as a command and goes verbatim into the pane
  (`hub_command`).
- Readline editing (Ctrl-A/E/U/K/W/Y/D/H/T/F/B, one kill buffer).
- **Interrupt**: empty-composer send button arms (amber) then fires Escape
  into the recipient's pane; double Ctrl-C same; while the recipient is
  mid-turn the resting button shows the stop-in-arc glyph. The act is
  recorded in the room (`[tmm] interrupted <name>`).
- Attachments (`+`): images downscale client-side and land in
  `<ws>/.tmm/uploads/`; any other file lands byte-identical; position is a
  visible `[img:n]` token swapped for the real ref at send.

## Drawer (three partitions, one width handle)
- **Terminal**: the selected agent's live pane (embedded xterm), window
  pills with state dots. Esc inside `.xterm` goes to the pane.
- **Files**: the real Files component in single-pane mode, per-project
  parked cwd, maximize hands off to the Files tab. Esc inside is the
  browser's own.
- On the phone/compact: terminal and files toggles JUMP to their tabs.
- The header also carries a board shortcut — the task board is its own
  PAGE (tab between Files and Agents), never a drawer partition.
- Every drawer toggle re-anchors the reading position (`withReadingAnchor`).

## Board page (task management — its own tab)
- Four fixed columns: **todo / doing / review / done**. Issues carry title,
  body, assignee, opened-by, and their own **note thread** (progress and
  decisions live ON the issue, not only in chat).
- The human writes/edits here; agents use `tmm board` (`take` = claim +
  doing; only the acceptor moves to done). Same rows, same vocabulary —
  the board is session-scoped like the room.
- Cards name the REPORTER (`by human` / `by lead`) and the assignee.
  Assigning from the detail view picks a managed agent and DELIVERS the
  assignment (`hub_post` @message typed into that agent's pane, with the
  take/note/move instructions) — assignment is a dispatch, not a label.
- Status changes are recorded in the room (`[tmm] board #N a → b`), and a
  move to review NOTIFIES the reporter (line typed into their pane) — the
  handoff loop: file → assign (delivered) → take → review (delivered) →
  done by the reviewer. Board columns and agent live states are separate
  axes joined at these events.
- Carries the shared project sidebar (every project has its own board);
  the followed last-touched session is the default, a pick overrides it
  until that moves again. Compact is the standard drill-down (project list
  ⇄ board). Polls while visible; a failed poll keeps the last board; back
  gesture peels detail → list → project list.

## Back gesture (compact)
Peels layers in tap-outside order: context menu → agent menu → recipient
picker → palette → armed interrupt → confirms → pickers → drawer → and a
bare conversation lifts the project list (the floor).

## Related
- Design: `tmm-cli.md` (the whole agents-v2 substrate), `projects.md`,
  `agent-lifecycle.md`, `design-language.md` (visual contract), `team.md`
  (the OTHER multi-agent surface — templated rosters, phone-first).
- API: `docs/requirements/api-contracts/websocket-rpc.md` (hub_*, board,
  registry/skills/MCP tables).
