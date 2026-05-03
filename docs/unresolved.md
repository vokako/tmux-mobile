# Unresolved Issues

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

## Adaptive per-attempt reconnect timeout
- **Priority**: Low
- **Area**: App / Reconnect
- **Details**: HEAD's `tryReconnect` uses the default `connect()` timeout (5 s) for every attempt regardless of address class. Once the pending HTTP/parallel-probe reconnect rework lands, scale per-attempt timeout with `classifyAddress()` — e.g. LAN 2 s, Tailscale 3 s, WAN 5 s. LAN-local retries should fail fast; public-internet retries legitimately need more headroom (TLS + routing on cellular). Affects: `src/App.svelte:tryReconnect`, `src/lib/ws.js:connect`.

## Reconnect success does not immediately re-capture pane
- **Priority**: Low
- **Area**: Terminal / Reconnect
- **Details**: On reconnect, `onReconnectSuccess` calls `wsSubscribe(terminalTarget)` but does not call `capturePane`. The terminal then shows stale content until the next server-side 200ms tick pushes a `pane_output`. Usually imperceptible but on very slow networks the gap is visible. Consider dispatching a one-shot `capturePane` right after subscribe.

## Color adaptation: FG+BG not re-balanced as pair
- **Priority**: Low
- **Area**: Terminal / Color
- **Details**: `adaptColors` in `Terminal.svelte` reshapes each ANSI color independently, so hand-picked FG/BG combos can lose contrast in extreme cases (e.g. purple bg `rgb(128,0,128)` with yellow fg `rgb(255,255,0)` in light mode: both get moved toward mid luminance and end up with ~1.6:1 contrast). Fixing this requires a small SGR state machine that tracks current FG/BG and adjusts them jointly — meaningful work, low real-world impact because typical AI CLI output uses FG-only colors on the default terminal bg.

## Color adaptation: ANSI palette colors 0-15 not adapted
- **Priority**: Low
- **Area**: Terminal / Color
- **Details**: Basic 16 ANSI colors (`\x1b[31m` etc.) are left to xterm.js's theme mapping (`red`, `brightRed`, etc.). If a CLI relies on basic palette codes and the theme's chosen red/green clashes with the terminal bg in either mode, `adaptColors` won't help. Tune the palette in `darkTheme` / `lightTheme` objects, or extend the adaptation to post-process the xterm canvas (much harder).

## cargo test is broken at HEAD
- **Priority**: Low (no user impact; blocks running the automated test suite)
- **Area**: Build / Testing
- **Details**: `cargo test` fails to compile in `src-tauri/src/main.rs` because the tests still call `tmux::new_session(TEST_SESSION)` with a single argument, but the function signature was widened to `new_session(name, path: Option<&str>, command: Option<&str>)`. The bin builds fine (`cargo check`), so this only blocks the test runner. Fix: update the test call sites to pass `None, None`. Pre-existing when I started the color/reconnect/sort work — not introduced by any of my commits.
