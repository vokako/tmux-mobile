# Unresolved Issues

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

### rmcp client reconnect behavior on server restart unverified
- **Priority**: Medium · **Area**: Team / agora MCP
- The in-process MCP daemon's session ids are in-memory (LocalSessionManager);
  on server restart agents get stale-session errors and must re-handshake.
  Whether kiro/claude/codex reconnect silently vs error to the model is
  unverified (needs a real agent + restart). DB-backed history means no messages
  are lost (cursor replay), but the reconnect smoothness is unknown.

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
