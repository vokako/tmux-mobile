## Project Overview

tmux-mobile: Tauri 2 cross-platform app (Rust + Svelte 5) for remotely monitoring and controlling tmux sessions from a phone. WebSocket JSON-RPC protocol with token auth + optional E2E encryption.

Targets: Android (primary), macOS desktop, browser (web UI).

## Tech Stack

- **Frontend**: Svelte 5 (runes: `$state`, `$derived`, `$effect`, `$props`), Vite 6
- **Backend**: Rust (Tauri 2), tokio, tokio-tungstenite
- **Terminal**: xterm.js v6 (`@xterm/xterm`) — VS Code-based terminal emulator with virtual scrolling
- **Preview**: highlight.js, marked, mermaid, katex, pdfjs-dist
- **Tauri plugins**: plugin-fs, plugin-dialog, plugin-opener (opener is broken on Android — use AndroidFileOpener JS interface instead)

## Key Architecture Decisions

- Terminal rendering uses **xterm.js v6** with custom touch handling (scrolling, scrollbar drag, long-press word selection)
- Content updates: server captures tmux pane output (with `-e -J` flags for ANSI + joined lines), sends via WebSocket subscription. Client clears screen+scrollback (`\x1b[2J\x1b[3J`) and rewrites full content each cycle
- Chat view parses raw ANSI terminal output into structured messages using ANSI color codes as semantic markers
- File opening on Android uses a custom `@JavascriptInterface` (`AndroidFileOpener`) in `MainActivity.kt`, NOT `tauri-plugin-opener` (which has a serialization bug on Android)
- WebSocket server only starts on desktop (`#[cfg(desktop)]`); mobile connects to a remote server
- All Tauri plugin imports are lazy-loaded via dynamic `import()` and gated behind `isTauri` / `isAndroid` checks
- Downloaded files on Android go to `/storage/emulated/0/Download/TmuxMobile/`, opened via FileProvider + Intent
- Keyboard handling: visualViewport API for mobile browser, native `OnGlobalLayoutListener` event for Android WebView
- Touch interactions are fully custom (xterm.js v6's VS Code scrollbar is mouse-only, no touch support)

## Commands

```bash
npm run dev              # Vite dev server (web UI on 0.0.0.0:5173)
npm run tauri:dev        # Desktop app + WS server (dev mode)
npm run build:mac        # macOS .app + .dmg
npm run build:android    # Android APK (aarch64)
cd src-tauri && cargo run --bin server   # Standalone WS server
cd src-tauri && cargo test -- --test-threads=1   # Run tests (needs tmux running)
```

## Project Structure

```
src/
├── App.svelte              # Router, nav, reconnect, theme, swipe tabs, keyboard detection
├── main.js                 # Entry point, imports xterm.css
├── lib/
│   ├── ws.js               # WebSocket client (E2E encryption, JSON-RPC, subscribe)
│   ├── Settings.svelte     # Connection form with address history, auto ws/wss detection
│   ├── Sessions.svelte     # Session/pane browser with AI tags (Kiro/Claude)
│   ├── Terminal.svelte     # xterm.js terminal, touch scrolling/selection, input bar, shortcuts
│   ├── ChatView.svelte     # Chat bubble renderer for CLI agents
│   ├── Files.svelte        # File browser, preview, editor, bookmarks, upload/download
│   ├── parsers.js          # Pluggable CLI output parsers (Kiro CLI, Claude Code)
│   └── Icon.svelte         # SVG icon system (30+ icons)
src-tauri/src/
├── lib.rs                  # Tauri commands (save/list/delete downloads), mobile entry
├── main.rs                 # Desktop entry + integration tests
├── bin/server.rs           # Standalone server binary
├── server.rs               # WebSocket server (JSON-RPC, auth, E2E encryption, subscribe, fs ops)
├── tmux.rs                 # tmux CLI wrapper (-e -J flags for ANSI + joined lines)
├── fs.rs                   # Filesystem ops (list, stat, read, write, upload, download)
└── config.rs               # Config loader (~/.config/tmux-mobile/config.toml)
```

## Important Patterns

- **Platform checks in frontend**: `isTauri` (Tauri app vs browser), `isAndroid` (Android vs macOS). Always check `isAndroid` before falling back to Tauri plugin APIs.
- **Tauri plugin readiness**: Always `await tauriReady` before using `tauriFs`, `tauriDialog`, `tauriOpener`.
- **Android file access**: Use `sanitize_filename()` in `lib.rs` for all filename inputs to prevent path traversal.
- **Base64 encoding for large data**: Use chunked `String.fromCharCode` (8192 bytes per chunk) to avoid stack overflow. Never use `btoa(String.fromCharCode(...spreadAllBytes))`.
- **HTML preview**: iframe sandbox must NOT combine `allow-scripts` + `allow-same-origin` (sandbox escape).
- **WebSocket lifecycle**: `connect()` cleans up any existing connection first. `onclose` rejects all pending promises. `doDisconnect()` clears reconnect timers.
- **Terminal touch handling**: All touch interactions (scroll, scrollbar drag, word selection) are custom-implemented because xterm.js v6's VS Code scrollbar only supports mouse events. Content updates are paused during touch interactions (`touchScrolling` flag) and catch up via `endTouchScroll()` on release.
- **Keyboard shift**: On mobile, when keyboard opens, terminal shifts up via negative `marginTop` so the cursor stays visible. Uses `maxContainerH` to prevent terminal resize on keyboard open/close.
- **xterm DA filtering**: Device attribute responses from xterm.js (`\x1b[?62;22c` etc.) are filtered in `onData` before forwarding to tmux.

## Testing

Tests require a running tmux server (`tmux` command available):
```bash
tmux new-session -d -s test   # Ensure at least one session exists
cd src-tauri && cargo test -- --test-threads=1
```

Tests are sequential (shared tmux state), in `src-tauri/src/main.rs`.

## Config

File: `~/.config/tmux-mobile/config.toml`
```toml
token = "auto-generated-uuid"
host = "0.0.0.0"
port = 9899
tmux_socket = ""
```
Env vars `TOKEN`, `HOST`, `PORT` override config. Server intentionally exposes full filesystem access (design choice for remote management).
