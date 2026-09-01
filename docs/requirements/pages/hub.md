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
- The header chevron beside the project name opens the project-actions
  menu as the NAME expanding downward (board #32): its left edge aligns
  with the visible name's left edge (anchored on the name element's real
  rect, zoom-corrected), and the shared placement clamp keeps it whole
  inside the viewport — never clipped at the right edge. Every other
  context menu keeps the right-aligned pointer placement.
- The composer's scrollbar exists exactly while there is something to
  scroll (board #34): hidden at rest and while the text fits, auto only
  when the natural height exceeds the max — decided in growComposer's one
  measurement, so shrinking or sending flips it back at once. Placeholders
  name the reach and stop ("Message every agent…", "Leave a note…"); the
  recipient menu's small hints keep the delivery semantics.
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
- **Board detail is a selectable history, not a mutable chat log** (board
  #43): note bodies and a locked issue's original title/body explicitly allow
  text selection. The original brief is editable only while the issue is
  currently unassigned AND no Agent has ever saved or noted on it; the server
  persists that activity bit and rejects later title/body patches, so clearing
  an assignee or moving back to todo cannot rewrite history. Status, assignee
  and new notes remain active workflow controls.

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
- Tail intent survives a page switch (board #38): "at the tail" is measured
  as the BOTTOM GAP (`scrollHeight − scrollTop − clientHeight < TAIL_GAP`),
  never absolute scrollTop; a hidden page's scroll/layout events cannot flip
  `following` in either direction; messages arriving while hidden update the
  data with the physical scroll deferred; on return the feed settles, then
  forces the tail and re-seeds the ask anchor + seen marker — but ONLY for a
  reader who left at the tail. One parked in history returns exactly where
  they were. Entering a room still lands at its tail.
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
  visible `[img:n]` token swapped for the real ref at send. Pasting into the
  composer stages files the same way (board #25): a clipboard carrying files
  (screenshot, copied file) goes through the identical pipeline and the
  default text insertion is suppressed — a copied file's path-as-text would
  say the same thing twice; a text-only paste is untouched.

## Drawer (three partitions, one width handle)
- **Terminal**: the selected agent's live pane (embedded xterm), window
  pills with state dots. Esc inside `.xterm` goes to the pane.
- **Files**: the real Files component in single-pane mode, per-project
  parked cwd, maximize hands off to the Files tab. Esc inside is the
  browser's own.
- On the phone/compact: terminal and files toggles JUMP to their tabs.
- The open partition is remembered PER PROJECT (board #23: "切换不同的
  project 回来原来的视图还在"): opening/closing records it
  (`hubPrefs.drawer`, localStorage), switching projects restores it — a room
  where the drawer was closed comes back closed, and the terminal partition
  re-seats its pane from the NEW room's roster (a stale target from the old
  room never leaks in). Survives reload; renameSession migrates the key.
- The header also carries a board shortcut — the task board is its own
  PAGE (tab between Files and Agents), never a drawer partition.
- Every drawer toggle re-anchors the reading position (`withReadingAnchor`).

## Board page (task management — its own tab)
- Four fixed columns: **todo / doing / review / done**. Issues carry title,
  body, assignee, opened-by, and their own **note thread** (progress and
  decisions live ON the issue, not only in chat). The **title is OPTIONAL**
  (board #31): an issue needs title or body — never neither — and an empty
  title is STORED empty (no fabricated fallback in persistence). Everything
  that NAMES an issue (cards, delete confirm, room lines, notices, the CLI's
  list/show) speaks ONE fallback — `issueRef`/`issue_ref`: trimmed title,
  else the body squashed to one line and cut Unicode-safe with a `…`
  marker, else `#id` — so a titleless card wears its body excerpt as the
  title and the preview line stays empty (the same text twice reads as a
  bug). The note reply box
  soft-wraps and grows with its content (board #28: a long line used to
  scroll horizontally, hiding what came before) — one line at rest, Enter
  sends, Shift+Enter inserts a newline, an IME composition's Enter never
  sends, and a sent note shrinks the box back; the send button rides the
  last line. Tapping a historical note reveals the same absolute `.m-acts`
  action row Chat bubbles use, with one **Copy** verb that writes the note's
  raw body; selection drags do not trigger it, the time is the accessible
  action trigger, and outside/Escape/issue switch or the brief Copied beat
  dismisses it without changing detail scroll height (board #46). Copy state
  is generation-scoped before and after the async clipboard write, so an old
  timeout or deferred resolve cannot affect another note or issue. The
  CREATE form submits from the keyboard too (board #36:
  "我在 board 填写完 issue 描述后，可以 cmd+enter 直接提交确认"): the
  title's Enter creates, and the multi-line body creates on **Cmd+Enter /
  Ctrl+Enter** (both modifiers, cross-platform) — a bare or Shift Enter in
  the body stays a real newline, an IME composition's Enter commits the
  candidate text and never the issue (on the title too), and every trigger
  is the same createIssue the ✓ button calls. The DETAIL editor takes no
  chord — its save is the diffed, guarded Save button.
- The four areas tile as **1, 2 or 4 columns — never 3** (board #27: with
  four areas, three across orphans the fourth on its own row). The count is
  decided by the BOARD's own container width (CSS container queries), so
  the standalone page and the Hub drawer's embedded board obey the same
  thresholds by construction — the viewport plays no part. Movement order
  and per-column scrolling hold in every shape; the page itself never
  scrolls. The 1-column stack is ADAPTIVE (board #33): a **sparse** area
  (0–1 cards) takes header + content height only, **dense** areas (2+)
  flex-share the remaining height with their own internal scroll, and when
  all four are sparse the leftover blank stays at the bottom — empty areas
  are never inflated. The ≥2-column grids keep splitting the height
  equally (rows must not content-size, or the page would scroll).
- The human writes/edits here; agents use `tmm board` (`take` = claim +
  doing; only the acceptor moves to done). Same rows, same vocabulary —
  the board is session-scoped like the room. `#N` is a database-wide issue
  handle from the shared `issues` table, **not** a per-project sequence, so
  a project's first visible issue need not be `#1` and gaps are normal. The
  number is not the isolation boundary: every list/get/save/note/delete uses
  `session + id`, and a guessed id from another project matches nothing.
  Renaming a project moves its Board rows to the new session key in the same
  transaction as the declaration; archive keeps the Board, while permanent
  project deletion removes the Board and its note threads before releasing
  the session name (board #41).
- The sidebar lists only projects whose board HAS issues (board #39: "如果
  该项目完全为空 则直接不显示该 project"), fed by ONE bulk `hub_board_counts`
  read alongside `project_list`/`hub_rooms` — never a per-project
  `hub_board_list` walk. Each row is the Chat sidebar's two-line shape (the
  shared `.proj-row` skeleton: dot + name + last-reply age) with the four
  column counts as the quiet second line — fixed todo/doing/review/done
  order, zeros included, each chip coloured by the one board status
  language. Because the set is always four, Board alone adds the shared
  `.side-wins.grid` modifier and a zero-height hidden mirror row: it measures
  the available row, localized labels, and the widest count across boards;
  `chipCols` then selects equal **4 / 2 / 1** columns (4×1, 2×2, or 1×4).
  Two columns are the safe pre-measure fallback. Three is never offered, so
  there is no ragged 3+1; equal tracks align each column's leading dots, and
  the one-column fallback prevents truncation at very narrow widths. Chat
  keeps the bare flex-wrap for its variable agent count. The CURRENT board's
  counts refresh from its own `hub_board_list`
  the moment it answers, so deleting the last issue hides the project and
  creating the first shows it at once — no poll wait. An empty CURRENT
  board stays usable while hidden: the page-head still names it (the name
  lookup runs over the full project list) and its main area creates the
  first issue; with no session to follow, the first NON-EMPTY board is the
  default selection.
- Cards name the REPORTER (`by human` / `by lead`) and the assignee.
  Assigning from the detail view picks a managed agent and DELIVERS the
  assignment (`hub_post` @message typed into that agent's pane, with the
  take/note/move instructions) — assignment is a dispatch, not a label. The
  dispatch reads as a HANDOFF (board #51): the SUBJECT leads (`{who}
  assigned this to you`, the operator's name at the front), then the issue
  (original title/body), then the note thread, and the tmm take/note/move
  instructions ride LAST — never between the issue and its thread. Notes
  come chronological with authors preserved (board #42) under their own
  explicit character budget, and any cut points to `tmm board show <id>`
  so one giant note cannot flood the pane while discussion never disappears.
- Status changes are recorded in the room (`[tmm] board #N a → b`), and a
  move to review NOTIFIES the reporter (line typed into their pane) — the
  handoff loop: file → assign (delivered) → take → review (delivered) →
  done by the reviewer. Board columns and agent live states are separate
  axes joined at these events.
- Destructive and discarding actions confirm through the app's ONE
  ConfirmDialog, phone-sheet aware (board #29): deleting an issue is a
  danger confirm that CAPTURES its target at request time (a selection or
  project change while the dialog stands cannot redirect it; success cleans
  only the matching view); dropping a dirty edit OR typed-but-uncreated
  form data is a neutral confirm reached from every exit (cancel, Escape,
  back, sidebar pick), a confirmed discard truly clears the form, and clean
  navigation never asks.
- Carries the shared project sidebar (every project has its own board);
  the followed last-touched session is the default, a pick overrides it
  until that moves again. Compact is the standard drill-down (project list
  ⇄ board). Polls while visible; a failed poll keeps the last board; back
  gesture peels detail → list, then LIFTS the project drawer as the floor
  (Hub's compact rule — back with it open falls through and re-pushes; it
  never dumps the reader on the terminal, board #47). Only when the page
  was jumped into from the chat's board icon does back return to the
  conversation instead (App's one-deep return slot outranks the lift).

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
