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
npm run build:mac        # macOS .app + .dmg
npm run build:android    # Android APK (aarch64)
cd src-tauri && cargo run --bin server   # Standalone WS server
cd src-tauri && cargo test -- --test-threads=1   # Tests (needs tmux running)
```

## Documentation Map

### Requirements (the WHAT)
- [Terminal page](docs/requirements/pages/terminal.md)
- [Chat View page](docs/requirements/pages/chat-view.md)
- [File Browser page](docs/requirements/pages/file-browser.md)
- [Sessions page](docs/requirements/pages/sessions.md)
- [Settings page](docs/requirements/pages/settings.md)
- [i18n / Localization](docs/requirements/features/i18n.md)
- [WebSocket RPC API](docs/requirements/api-contracts/websocket-rpc.md)
- [WebSocket Server](docs/requirements/backend/services/websocket-server.md)
- [tmux Wrapper](docs/requirements/backend/services/tmux-wrapper.md)
- [Filesystem Service](docs/requirements/backend/services/filesystem.md)

### Design Docs (the WHY & HOW)
- [Chat Parser Architecture](docs/design-docs/features/chat-parser.md)
- [Terminal Touch Handling](docs/design-docs/pages/terminal-touch.md)
- [Terminal Gesture State Machine](docs/design-docs/pages/terminal-gestures.md)
- [Android Platform Integration](docs/design-docs/features/android-platform.md)
- [WebSocket Client Robustness](docs/design-docs/features/websocket-client.md)
- [File Handling & Security](docs/design-docs/features/file-handling.md)
- [Terminal Color Adaptation](docs/design-docs/features/color-adaptation.md)

### Other
- [Unresolved Issues](docs/unresolved.md)

## Key Patterns
- **Platform checks**: `isTauri` (Tauri vs browser), `isAndroid` (Android vs macOS). Always check `isAndroid` first.
- **Tauri plugins**: Always `await tauriReady` before use. Dynamic imports gated behind platform checks.
- **Android file opening**: Use `AndroidFileOpener` JS interface, NEVER `tauri-plugin-opener`.
- **Base64 large data**: Chunked 8192 bytes per chunk. Never spread all bytes.
- **HTML preview**: iframe `allow-same-origin` only, NO `allow-scripts`.
- **WebSocket lifecycle**: `connect()` cleans up existing. `onclose` rejects pending. `doDisconnect()` clears timers. Heartbeat ping every 15s; 2 consecutive RPC timeouts → auto-close → reconnect.
- **Terminal touch**: All custom (xterm.js v6 has no touch support). Pause updates during touch. See `docs/design-docs/pages/terminal-gestures.md` for full state machine.
- **Terminal keyboard**: Double-tap to open (NOT single-tap). `kbLocked` flag + `inputmode` attribute. `endTouchScroll` must NEVER change `kbLocked` (race condition with delayed timers). Only `unlockKeyboard()`, blur timer, keyboard-shift, and pane switch may change it.
- **Mobile auto-pair textarea**: Force-clear xterm's hidden textarea after keyboard input (NOT paste). Use `paste` event flag to distinguish — NEVER use `data.length` (auto-paired `""` `()` have length 2, gets misclassified as paste).
- **Tab swipe priority**: App-level left/right tab swipe is lowest priority. Suppressed when any child gesture is active (`defaultPrevented` or vertical movement > 10px).
- **Chat parsing**: Use ANSI color codes as semantic markers BEFORE stripping.
- **xterm DA filtering**: Filter device attribute responses before forwarding to tmux.

## Testing
```bash
tmux new-session -d -s test
cd src-tauri && cargo test -- --test-threads=1
```
Tests are sequential (shared tmux state), in `src-tauri/src/main.rs`.

## Config
File: `~/.config/tmux-mobile/config.toml` — token, host, port, tmux_socket, tls_cert, tls_key, scrollback.
Env vars `TOKEN`, `HOST`, `PORT`, `TMUX_SOCKET`, `TLS_CERT`, `TLS_KEY`, `SCROLLBACK` override config.
Default scrollback: 500 lines.
