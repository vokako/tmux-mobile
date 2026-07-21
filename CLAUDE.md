# AGENTS.md — tmux-mobile

## Project Overview
Tauri 2 cross-platform app (Rust + Svelte 5) for remotely monitoring and controlling tmux sessions from a phone. WebSocket JSON-RPC protocol with token auth + optional E2E encryption.

Targets: Android (primary), macOS desktop, browser (web UI).

## Tech Stack
- **Frontend**: Svelte 5 (runes: `$state`, `$derived`, `$effect`, `$props`), Vite 6
- **Backend**: Rust (Tauri 2), tokio, tokio-tungstenite
- **Terminal**: xterm.js v6 (`@xterm/xterm`)
- **Preview**: highlight.js, marked, mermaid, katex, pdfjs-dist

## Commands
```bash
npm run dev              # Vite dev server (web UI on 0.0.0.0:5173)
npm run tauri:dev        # Desktop app + WS server (dev mode)
npm run tauri:dev:release # Release-mode desktop app + WS server
npm run build:mac        # macOS .app + .dmg
npm run build:android    # Android APK (aarch64)
cd src-tauri && cargo run --bin server   # Standalone WS server
cd src-tauri && cargo test -- --test-threads=1   # Tests (needs tmux running)
```

Run Tauri through these project scripts. Do not use `pnpx tauri`: `pnpx` is
`pnpm dlx` and downloads the unrelated `tauri` package instead of invoking the
installed `@tauri-apps/cli`. Release mode is the `--release` option, not a
positional `release` argument.

On a memory-constrained machine, lower Cargo concurrency per invocation without
changing the project default:
`CARGO_BUILD_JOBS=2 npm run tauri:dev:release`.

## Documentation Map

### Requirements (the WHAT)
- [Terminal page](docs/requirements/pages/terminal.md)
- [Chat View page](docs/requirements/pages/chat-view.md)
- [File Browser page](docs/requirements/pages/file-browser.md)
- [Sessions page](docs/requirements/pages/sessions.md)
- [Settings page](docs/requirements/pages/settings.md)
- [Team page (multi-agent)](docs/requirements/pages/team.md)
- [i18n / Localization](docs/requirements/features/i18n.md)
- [WebSocket RPC API](docs/requirements/api-contracts/websocket-rpc.md)
- [WebSocket Server](docs/requirements/backend/services/websocket-server.md)
- [tmux Wrapper](docs/requirements/backend/services/tmux-wrapper.md)
- [Filesystem Service](docs/requirements/backend/services/filesystem.md)

### Design Docs (the WHY & HOW)
- [Fonts (system stack + symbol bundles + custom override)](docs/design-docs/features/fonts.md)
- [Chat Parser Architecture](docs/design-docs/features/chat-parser.md)
- [Terminal Touch Handling](docs/design-docs/pages/terminal-touch.md)
- [Terminal Gesture State Machine](docs/design-docs/pages/terminal-gestures.md)
- [Terminal Sizing (cols × rows)](docs/design-docs/pages/terminal-sizing.md)
- [Sessions Page Density & Navigation](docs/design-docs/pages/sessions-density.md)
- [Android Platform Integration](docs/design-docs/features/android-platform.md)
- [WebSocket Client Robustness](docs/design-docs/features/websocket-client.md)
- [Concurrent WS RPC (server)](docs/design-docs/features/concurrent-ws-rpc.md)
- [Disconnect Grace (server)](docs/design-docs/features/disconnect-grace.md)
- [Desktop Split-Screen](docs/design-docs/features/split-screen.md)
- [PWA Install Offer (web)](docs/design-docs/features/pwa-install.md)
- [File Handling & Security](docs/design-docs/features/file-handling.md)
- [Terminal Color Adaptation](docs/design-docs/features/color-adaptation.md)
- [Team / multi-agent bus](docs/design-docs/features/team.md)

### Other
- [Unresolved Issues](docs/unresolved.md)

## Key Patterns
- **Platform checks**: `isTauri` (Tauri vs browser), `isAndroid` (Android vs macOS). Always check `isAndroid` first.
- **Tauri plugins**: Always `await tauriReady` before use. Dynamic imports gated behind platform checks.
- **Android file opening**: Use `AndroidFileOpener` JS interface, NEVER `tauri-plugin-opener`.
- **Base64 large data**: Chunked 8192 bytes per chunk. Never spread all bytes.
- **HTML preview**: iframe `allow-same-origin` only, NO `allow-scripts`.
- **WebSocket lifecycle**: `connect()` cleans up existing. `onclose` rejects pending. `doDisconnect()` clears timers. Heartbeat ping every 15s; 2 consecutive RPC timeouts → auto-close → reconnect.
- **Terminal touch**: All custom (xterm.js v6 has no touch support). Pause updates during touch. Selection is an *object* (`{anchor, head}`, both inclusive buffer-row/col) that persists until explicitly copied or cancelled; both endpoints are draggable via handles; copy is via an explicit floating toolbar (no tap-to-copy heuristics). See `docs/design-docs/pages/terminal-gestures.md` for the full state machine.
- **Terminal keyboard**: Double-tap to open (NOT single-tap). `kbLocked` flag + `inputmode` attribute. `endTouchScroll` must NEVER change `kbLocked` (race condition with delayed timers). Only `unlockKeyboard()`, blur timer, keyboard-shift, and pane switch may change it.
- **Printable keys bypass xterm's keydown**: unmodified printable keydowns return `false` from `attachCustomKeyEventHandler` and flow through the textarea input pipeline; a CAPTURE-phase `input` listener on termEl claims non-composition `insertText` events (`stopImmediatePropagation`) and forwards them. It must claim, not just forward: xterm v6 handles `insertText` itself when no keydown preceded it (`!e.composed || !_keyDownSeen`) — exactly the no-keydown commits WKWebView IMEs produce for CJK punctuation — so two live handlers sent those characters TWICE. Reason for the bypass: CJK IMEs convert punctuation (`,`→`，`) at the input stage with NO composition events — xterm's keydown fast path would emit raw ASCII and preventDefault, killing the conversion. Applies to ALL platforms; composition stays with xterm. Paste is detected via a CAPTURE-phase `paste` listener (xterm's own paste handler fires onData synchronously before same-phase listeners) and routed to the `paste_text` RPC — tmux `paste-buffer -p` adds bracketed-paste markers iff the pane app enabled `?2004`, so multi-line pastes don't execute line by line.
- **Auto-pair textarea force-clear** (all platforms, was mobile-only): Force-clear xterm's hidden textarea after keyboard input (NOT paste, NOT mid-IME-composition). Use `paste` event flag to distinguish — NEVER use `data.length` (auto-paired `""` `()` have length 2, gets misclassified as paste). Composition needs TWO signals: `compositionstart/end` listeners AND per-event `insertCompositionText` inputType — some Android IMEs (Samsung/pad suggestion-bar keyboards) compose without ever firing compositionstart. `compositionend` must reset BOTH flags: Chromium commits as input(insertCompositionText) → compositionend with no trailing input event, so a sticky per-event flag would permanently suppress the clear for standard IMEs (GBoard).
- **Tab swipe priority**: App-level left/right tab swipe is lowest priority. Suppressed when any child gesture is active (`defaultPrevented` or vertical movement > 10px).
- **Chat parsing**: Use ANSI color codes as semantic markers BEFORE stripping.
- **Ctrl keys must be tmux named keys**: with `extended-keys on`, tmux DROPS raw C0 bytes (`send-keys -l $'\x03'`) sent to panes in extended key mode (`#{pane_key_mode}`=`Ext` — every modern agent TUI). `tmux::send_keys` literal mode therefore splits C0 bytes into named keys (`C-c`, `M-C-x`); don't bypass it.
- **xterm DA filtering**: Filter device attribute responses before forwarding to tmux.
- **Team is desktop-only + JSON-gated**: everything we built + the user sees is **Team** (`team_*` RPCs, `TEAM_*` config, `tmm-team-<team-id>` sessions, `team.rs`/`team_bridge.rs`); the **vendored library crate stays `agora`** (faithful upstream copy — `use agora::bus::Bus`). It's a target-gated dep (`cfg(not(android|ios))`); `server.rs` must NEVER name an agora type — it talks to the bus only through the JSON-only `server::TeamBridge` trait (concrete impl `team_bridge::TeamManager`, desktop-only). Mobile passes `None`; `team_*` RPCs then return method-not-found and the Team tab hides itself. The bus runs in-process; the MCP daemon (`:8787`, external agents) and the phone's WS path share it. **Multiple teams = isolated rooms** (room id = stable canonical-workspace + template slug): room-aware via a `BusProvider` trait (agents pick a room with an `x-room` header, the phone passes `room` per RPC, pushes carry `room`); `TeamManager` is the room registry over ONE shared SQLite connection. Each Team runs in `tmm-team-<team-id>` with agents as named windows; new runtime homes live under `<workspace>/.tmm/teams/<team-id>/` and are self-gitignored. On startup the manager recovers teams still alive in tmux; legacy workspace-only rooms retain their old `.tmm/` layout. The Team tab has a team switcher (new/close). See `docs/design-docs/features/team.md`.

## Workflow
- **Commit after every verified change** (owner's standing instruction): once a fix/feature is tested and its docs are updated, commit it right away — one logical change per commit. Don't let verified work sit uncommitted in the tree. Never commit `agent-team-page/` or other unrelated in-progress work without being asked.

## Testing
```bash
tmux new-session -d -s test
cd src-tauri && cargo test -- --test-threads=1
```
Tests are sequential (shared tmux state), in `src-tauri/src/main.rs`.

## Config
File: `$XDG_CONFIG_HOME/tmux-mobile/config.toml` (fallback `~/.config/tmux-mobile/`) — token, host, port, tmux_socket, tls_cert, tls_key, scrollback, disconnect_grace_secs, team_bind, team_db, team_room, team_model.
Env vars `TOKEN`, `HOST`, `PORT`, `TMUX_SOCKET`, `TLS_CERT`, `TLS_KEY`, `SCROLLBACK`, `DISCONNECT_GRACE_SECS`, `TEAM_BIND`, `TEAM_DB`, `TEAM_ROOM`, `TEAM_MODEL` override config (legacy `CREW_*`/`AGORA_*` env vars + `crew_*`/`agora_*` config keys still accepted as aliases).
Default scrollback: 500 lines.
Default disconnect_grace_secs: 600 (10 min). Delay before a disconnected client's tmux window is auto-resized back; set to 0 for legacy immediate restore. See `docs/design-docs/features/disconnect-grace.md`.
Default team_bind: `127.0.0.1:8787` (in-process MCP daemon + dashboard for the Team feature, desktop only). team_db default: `<config>/tmux-mobile/team.db`; team_room default: `main`; team_model default: `claude-sonnet-4.6` (kiro-backed agents). Team launching is in-process (`src-tauri/src/team.rs`), per-workspace, triggered by the Team tab's Start button. room→workspace persisted to `teams.json` for restart recovery. See `docs/design-docs/features/team.md`.
