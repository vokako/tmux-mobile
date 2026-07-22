# Sessions Page

## Purpose
Fast, scannable entry point for selecting which pane to view. Optimized for
a user running many parallel coding-agent sessions ("is Kiro done in `proj-A`?
what's Claude doing in `proj-B`?") — each row should reveal its identity at a
glance without requiring interaction.

## Components

- Session rows, MRU chips, and expanded pane rows show a compact attention dot when the corresponding session or window has an unread coding-agent lifecycle notification.

### Top row
- **Single row combining MRU chips and search**. Chips fill the left, a
  round search icon button sits on the right. Tapping search swaps the
  entire row into full-width input mode (chips hide, input grows, × on
  the right closes / clears). Escape also closes.
- **MRU chips** (shown in the default row state). Up to 5 most-recently-
  opened **AI sessions** — sessions where at least one pane is running a
  known coding-agent CLI (Kiro/Claude/OpenClaw), excluding the currently-
  open one. The filter is deliberate: plain zsh/node/vim sessions clutter
  a "fast switch" surface and are still reachable via search or the full
  list. Horizontally scrollable. One tap on a chip opens that session at
  its primary AI pane.

### Grouping: Teams vs Sessions
When the server has the team bus (`teamState.available`) and at least one
team session (`tmm-team-<room>`) exists, the list splits into two labelled
groups — **Teams** first, then **Sessions** — each with an icon + count
header. Without team sessions (or on a busless server) the list stays flat
and headerless, exactly as before.

- Classification is by the `tmm-team-` name prefix, **gated on the shared
  `teamState.available`** (`src/lib/core/team.svelte.ts`) so a busless server
  shows these as ordinary sessions on every surface (Sessions, PanePicker).
- A team row displays the workspace basename (the `-<6hex>` slug suffix is
  stripped for display; the full room stays in the row `title`). Two
  workspaces with the same basename therefore display alike — a known,
  accepted trade-off (the title attribute disambiguates on desktop).
- Tapping a team row opens the **Team chat** for that room (not a raw
  terminal); the trailing affordance is a chat glyph instead of the kill
  button, and team rows never expand a pane list.

### Session row (single line, dense)
Left-to-right:
- **Status dot** — accent color + glow when tmux `attached == true`,
  muted otherwise.
- **Session name** (bold, truncates at 40% of row width).
- **Inline summary** — the row's identity, chosen as:
  - If a pane in the session runs a known AI CLI → the AI's icon.
  - Otherwise → the primary pane's `current_command` (monospace).
  - The cwd path segment was removed from the session row (it was squeezed
    to unreadability in the cramped line). Full paths live on the expanded
    window rows (right-aligned, horizontally scrollable).

  The "primary pane" is: the pane matching `activeTarget` if open, else the
  first pane with an AI tag, else the first pane returned by the server.
- **Trailing cluster** (right-aligned, tight):
  - Relative time of last open (`now`, `5m`, `3h`, `2d`, or month/day).
    Only shown when the session has a `last_opened` timestamp.
  - Window count badge `Nw` — only when `windows > 1`.
  - Kill button (`×` → "tap to kill" on first tap; 3-second confirm window).

### Pane list (expanded only when relevant)
Default: collapsed for every session. Shown when:
- The session has > 1 window AND the user taps the session row once, OR
- A search query is active AND the session matches but some panes match
  more specifically (search auto-expands).

Each pane row shows: `W.P` index (monospace, accent) · `current_command` ·
`cwd segment` · AI icon (if any) · `×` kill button.

Plus a `+ Window` button at the end of the pane list.

### Bottom bar
- **New Session** — expands an inline form with session name, working-dir
  picker, optional startup command with Kiro/Claude presets.
- **Refresh** icon. Pull-to-refresh was removed: it was a custom
  touch-handler implementation and on top of a scrolling list it conflicted
  too often with ordinary vertical scrolling near the top edge. A tap on
  the refresh button is explicit, reliable, and hits the same code path.

## Interactions

### Opening a session
- **Single pane session** → tap the session row anywhere (except kill) →
  navigates directly to Terminal with that pane.
- **Multi-window session** → tap the session row → expands the pane list in
  place. Tap again to collapse. Tap any pane to navigate.
- **MRU chip** → single tap → opens the session at its *primary AI pane*
  (the first pane running a known agent CLI, falling back to the first
  pane). **Chips never toggle the inline pane list** — the chip strip is
  the fast-switch surface, so a chip tap must move the user to the
  terminal, not leave them on the Sessions page with an unexpected row
  expansion elsewhere.

### Searching
- Type in the search box → list filters instantly. Matches highlight by
  virtue of appearing at all (no per-match highlighting; density already
  makes matches visible).
- Empty state while searching shows the query verbatim:
  `No matches for "foo"`.
- MRU chips hide while searching to focus attention on results.

### Kill
- Session kill: tap `×` → row shows `tap to kill`. Tap again within 3s →
  `kill_session` RPC → refresh.
- Window kill: same pattern inside the pane list.
- Clicking the row instead of the kill button during confirm state activates
  the session, **not** kill. Kill must be an explicit second tap on the kill
  button itself.

## API Calls
- `list_sessions` — sessions with `last_opened` annotation.
- `list_panes(session)` — called for every session on load (needed for
  inline summary). One call per session; cheap.
- `new_session(name?, path?, command?)`.
- `kill_session(name)`.
- `new_window(session)`.
- `kill_window(target)`.
- `fs_list(path)` — for the working-dir picker in the new-session form.

## State Management
- `sessions`: array of `TmuxSession` sorted as: (1) active session, then
  (2) sessions with `last_opened` descending (MRU), then (3) never-opened
  sessions in server's baseline order (tmux `session_activity` desc).
- `panes[sessionName]`: `TmuxPane[]`, loaded eagerly for summary rendering.
- `expanded[sessionName]`: bool, user-controlled per-session expansion for
  multi-window sessions.
- `query`: current search string, `$derived` `isSearching` gates chip
  visibility and auto-expansion.

## Derived rendering rules

### `sessionSummary(session)`
Picks the "primary pane" as described above and returns `{ ai, cmd }`.
The session row uses this to render inline context without opening the list.

### `relTime(unixSec)`
`< 45s` → `now`, `< 1h` → `Nm`, `< 24h` → `Nh`, `< 7d` → `Nd`, otherwise
`M/D`. Tabular numerals so the column is steady.

## Edge Cases
- **No sessions**: shows a friendly empty state at the list position.
- **Sessions without `last_opened`**: still listed (at the bottom of the
  MRU tail), but no time chip.
- **Long session names / cwd**: truncated with ellipsis; inline summary
  sacrifices cwd first, then cmd, keeping name and trailing cluster intact.
- **Recreated session with same name**: inherits previous `last_opened`
  (persisted by name); acceptable for MRU.
- **Search query exact match on cwd but no visible cwd in row**: the row
  still appears — search runs against full data, not rendered text.
