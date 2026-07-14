# Unresolved Issues

## Prefs/bookmarks: cross-client last-writer-wins
- **Priority**: Low
- **Area**: Filesystem service / Files page
- **Details**: `save_bookmarks` / `set_pref('recentFiles')` persist the whole
  array; the server (`config.rs`) blindly replaces it. The client now guards
  its own races (generation counter + never-write-before-first-load, see
  `docs/requirements/pages/file-browser.md`), but two clients connected at
  once (phone + desktop) can still clobber each other's writes: both load at
  t0, phone stars A, desktop stars B from its pre-A snapshot → A is silently
  dropped. Deeper fix: server-side add/remove RPCs (`bookmark_toggle`,
  `fs_add_recent`) or merge semantics in `set_prefs`, after which the client
  guards become unnecessary.

## Team label collisions on same-basename workspaces
- **Priority**: Low
- **Area**: Sessions / PanePicker / team.svelte.js
- **Details**: `teamLabel()` strips the `-<6hex>` slug suffix for display, so
  `/a/demo` and `/b/demo` both render "demo" (the full room only lives in the
  row `title`, unreachable on touch). Accepted for now (rare case). Fix would
  be joining against `team_status().teams[].workspace` and disambiguating with
  a parent-dir segment when basenames collide.

## Team feature — open issues (review 2026-06-15)

### ~~close_team leaks the room's bus + re-broadcast pump~~ — FIXED
- The `Team` now stores its pump `JoinHandle`; `close_team` aborts it (and the
  double-checked-insert path aborts the duplicate pump). No leak, no duplicate
  pushes on reopen.

### ~~Supervisor retries a failing launch every 3s forever~~ — FIXED
- The reconcile loop now counts per-agent launch failures and stops retrying
  after `MAX_LAUNCH_FAILURES` (3). (Still not surfaced in the UI — see below.)

### ~~A closed team that wasn't fully launched never stops its supervisor~~ — FIXED
- The loop now exits when `!bridge.room_exists(room)` too, not only on
  `launched_any && session gone`.

### Launch failures / fired agents not surfaced in the UI
- **Priority**: Low · **Area**: Team
- After the failure cap (above) the supervisor stops trying, but the UI shows no
  "failed" state — the agent just never appears as a chip/grid cell. Likewise a
  `fire`d or crashed agent. Fix: include a per-agent state (failed/offline) in
  the teams/roster payload and badge it in the roster + grid.

### Fired/offline agents keep a dead cell + stale pane in the grid
- **Priority**: Low · **Area**: Team / AgentGrid.svelte
- The desktop grid renders one cell per *employee* (all states), so a `fire`d or
  crashed agent leaves a cell pointing at a dead/closed pane (spinner or last
  frame). No visual "offline/failed" treatment. Fix: badge offline cells, or
  drop disabled employees from the grid.

### Manager hire() launches on a hardcoded backend, no model/x-room nuance
- **Priority**: Low · **Area**: Team / agora hire + supervisor
- `agora::bus::hire` seeds an employee with `backend` absent; our supervisor
  defaults hires to "kiro" (the recovery/seed path) — a hire can't pick
  claude/codex, and the hired spec has no `model`. Acceptable for now; revisit
  if runtime hiring is used heavily.

### System prompt / template edits don't affect already-running teams
- **Priority**: Low · **Area**: Team
- The system prompt + roster are baked into a team's brief/seed at launch.
  Editing them only affects teams started afterward; running agents must be
  restarted to pick up changes. By design, but not surfaced to the user.

### ~~rmcp client reconnect behavior on server restart~~ — FIXED + VERIFIED
- **Root cause (confirmed on the live system):** the in-process MCP daemon ran
  in rmcp's default **stateful** mode (`LocalSessionManager`, in-memory session
  ids). A backend restart wipes the session map, so a recovered agent still
  presenting its old `Mcp-Session-Id` is rejected (`401 Session not found`; a
  no-session request gets `422 expect initialize`). rmcp 0.3.2 does **not**
  auto-re-handshake, so the agent hangs on `wait` forever. Its `last_seen` goes
  stale → `apply_presence` marks the whole roster `offline` → the Team UI's
  "coming online" spinner (gated on `agents.length === 0`) spins indefinitely.
- **Fix 1 — stateless daemon** (`crates/agora/src/web.rs`): serve MCP with
  `StreamableHttpServerConfig { stateful_mode: false }`. Our tool surface is
  genuinely stateless (identity per-request from `x-agent`/`x-room` headers, all
  state in SQLite) and the agent loop is pure request/response (`post`/`wait`),
  so we lose nothing (no server→client push needed) and any *fresh* request now
  works with no init/session. Verified by `tests/stateless_probe.rs` and a live
  curl (bare `tools/call` → `200` + result).
- **Fix 2 — adopt + nudge on recovery** (`team_bridge::recover_running_teams`
  → `team::nudge_session_agents`): recovery **keeps** the surviving agent windows
  (so each agent's conversation context + in-flight work is preserved) and nudges
  each one to reconnect. The agent's MCP *client* lost its socket to the old
  daemon and is hung inside a `wait` call; verified with kiro-cli 2.7.0, the
  client neither times out nor retries on its own **but reconnects fine once the
  dead call is cancelled and a new turn starts**. The nudge is `Esc` (cancel the
  in-flight call → back to the prompt) + a short re-prompt that calls `wait`
  again. Harmless if an agent was healthy (just restarts its wait loop). Done
  once from recovery, NOT in the reconcile loop — the loop's presence check can't
  tell a healthy agent from one hung on a dead socket (a just-restarted agent
  still looks "online" for ~30 s until its presence TTL lapses, so an in-loop
  nudge gated on online-status never fires).
  - **Why not kill+relaunch?** Killing the windows and relaunching fresh agents
    would also work (Fix-1 makes the fresh handshake succeed), but it throws away
    each agent's CLI conversation + any in-progress task. Adoption keeps them.
- **Verified end-to-end**: kill server with 3 agents online → agents hang on a
  dead `wait` → restart server → recovery adopts the windows + nudges → all 3
  back to `idle`, last_seen <1s, no duplicate windows, context intact.

### Stale Chinese default.json on existing installs
- **Priority**: Low · **Area**: Team / templates
- `ensure_templates_seeded` only writes `default.json` when absent, so installs
  created before the English rewrite keep the Chinese roster. The English roster
  ships as `default-en.json` (added to the user config) but `default.json` is
  not migrated. Fix: offer a "reset to built-in" action, or version the builtin.

## iOS Target
- **Priority**: Low
- **Area**: Build / Platform
- **Details**: iOS build not yet implemented. Needs Xcode + xcodegen + Apple Developer account. Basic setup: `rustup target add aarch64-apple-ios aarch64-apple-ios-sim && npx tauri ios init && npx tauri ios dev`

## Terminal scrollback reset on full rewrite
- **Priority**: Medium
- **Area**: Terminal
- **Details**: `writeToXterm` calls `term.clear()` when `buf.baseY > 0`, dropping all xterm-side scrollback on each full rewrite. tmux only returns ~500 lines per capture, so scrollback effectively caps at the server snapshot length, and any extra history xterm had built up is lost after each push. Fix needs delta-based content updates instead of full rewrite, or server-side scrollback streaming. Affects: `src/lib/Terminal.svelte:writeToXterm`.

## Chat parser runs on every pane output
- **Priority**: Low (was Medium; chat UI now disabled so CPU cost has no user impact)
- **Area**: Chat View / Parser
- **Details**: `waitingForInput` and `statusInfo` are `$derived.by(paneContent)`, so the parser re-scans the entire pane content on every output event. For fast log streams this is CPU-heavy. Options: tail-only scan, debounce paneContent into a separate $state, or poll on an interval. Affects: `src/lib/Terminal.svelte:waitingForInput/statusInfo`, `src/lib/parsers.js`. **Note**: chat UI is currently disabled (`chatSupported = false`), so the derived values are computed but nothing visible depends on them. Revisit when re-enabling chat.

## ANSI 256-color adaptation in light theme
- **Priority**: Low
- **Area**: Terminal / Theme
- **Details**: `adaptColorsForLight` only inverts 24-bit RGB sequences (`\x1b[38;2;r;g;bm` / `\x1b[48;2;r;g;bm`). 256-color palette (`\x1b[38;5;Nm`) and basic 16-color output is passed through unchanged, so TUIs using indexed colors (htop, vim themes) may have poor contrast in light mode. Either map the 256-color palette or rely on `term.options.minimumContrastRatio`.

## xterm helper textarea listeners on font change
- **Priority**: Low
- **Area**: Terminal / Keyboard
- **Details**: If xterm.js rebuilds its hidden helper textarea on font-size change (unverified), our `kbTa` reference and the `blur`/`focus` listeners become stale, breaking the keyboard lock guard. Needs confirmation with a test; if true, re-bind listeners after each font change. Affects: `src/lib/Terminal.svelte` fontSize $effect.

## newWindow picks last pane by listPanes order
- **Priority**: Low
- **Area**: tmux RPC / Window switcher
- **Details**: `newWindow` in `Terminal.svelte` relies on `listPanes(session)` returning the new pane at the end, which is not guaranteed to be race-free. Have the `new_window` RPC return the new `{session, window, pane}` directly so the UI can switch without re-querying.

## Window switcher may show non-active pane info
- **Priority**: Low
- **Area**: Window switcher
- **Details**: The `windows` derived in `Terminal.svelte` dedupes by window id using the first pane it encounters, not the active one. `current_command` / `pane_title` / AI icon badge may therefore come from a background pane. Prefer `pane_active` when choosing the representative pane.

## paneCommand polling 3s lag
- **Priority**: Low (no user impact while chat UI is disabled)
- **Area**: Chat detection
- **Details**: `paneCommand` is polled every 3s, so chat-mode detection (Kiro start/exit) lags up to 3s. Could piggyback on pane output push for near-instant detection, or tighten poll to 1s as a minimum-effort fix. **Note**: chat UI is currently disabled so this lag has no visible effect; revisit when re-enabling chat.

## Scroll-to-bottom button lacks new-content indicator
- **Priority**: Low
- **Area**: Terminal / UX
- **Details**: When the user scrolls up to read history and new output arrives, the current `scroll-btn` stays visually identical. A red dot / highlight on the button when `hasNewContent` is true would make it obvious that new output is waiting. This only becomes useful once the main-branch defer-rendering behavior lands (currently HEAD writes immediately on output, which re-clears scrollback and pulls termAtBottom back to true). Revisit after the defer-render change is merged. Affects: `src/lib/Terminal.svelte:setOnPaneOutput`, `.scroll-btn` in `<style>`.

## Color adaptation: FG+BG not re-balanced as pair
- **Priority**: Low
- **Area**: Terminal / Color
- **Details**: `adaptColors` in `Terminal.svelte` reshapes each ANSI color independently, so hand-picked FG/BG combos can lose contrast in extreme cases (e.g. purple bg `rgb(128,0,128)` with yellow fg `rgb(255,255,0)` in light mode: both get moved toward mid luminance and end up with ~1.6:1 contrast). Fixing this requires a small SGR state machine that tracks current FG/BG and adjusts them jointly — meaningful work, low real-world impact because typical AI CLI output uses FG-only colors on the default terminal bg.

## Color adaptation: ANSI palette colors 0-15 not adapted
- **Priority**: Low
- **Area**: Terminal / Color
- **Details**: Basic 16 ANSI colors (`\x1b[31m` etc.) are left to xterm.js's theme mapping (`red`, `brightRed`, etc.). If a CLI relies on basic palette codes and the theme's chosen red/green clashes with the terminal bg in either mode, `adaptColors` won't help. Tune the palette in `darkTheme` / `lightTheme` objects, or extend the adaptation to post-process the xterm canvas (much harder).

## DA-response leakage: `?62;22;52c` appears as text in the prompt
- **Priority**: Medium
- **Area**: Terminal / xterm input filter
- **Details**: Occasionally the literal string `?62;22;52c` shows up in the terminal as if the user typed it. This is xterm.js's reply to a DA1 (Primary Device Attributes) query — full sequence `\x1b[?62;22;52c`. The query reaches xterm because `tmux capture-pane -e` re-emits ANSI sequences captured from programs that printed `\x1b[c`. xterm replies via `term.onData(...)`, our existing filter at `Terminal.svelte:605` drops `^\x1b\[[\?>=]?[\d;]*c$` — but the reply can be split across two `onData` invocations (e.g. `\x1b[?62;22` then `;52c`), defeating the regex. Fix candidates: (a) accumulate `onData` chunks across a short window and re-test the joined string before forwarding; (b) strip DA-related sequences server-side before pushing the snapshot, since they're never useful as visible content; (c) configure xterm to not auto-reply DA queries at all. Need a real-world repro to pick the cleanest path. Affects: `src/lib/Terminal.svelte:onData`.
