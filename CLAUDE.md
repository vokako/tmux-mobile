# CLAUDE.md

## Project Overview

tmux-mobile: Tauri 2 cross-platform app (Rust + Svelte 5) for remotely monitoring and controlling tmux sessions from a phone. WebSocket JSON-RPC protocol with token auth.

Targets: Android (primary), macOS desktop, browser (web UI).

## Tech Stack

- **Frontend**: Svelte 5 (runes: `$state`, `$derived`, `$effect`, `$props`), Vite 6
- **Backend**: Rust (Tauri 2), tokio, tokio-tungstenite
- **Terminal**: ansi-to-html (NOT xterm.js — uses CSS scrolling with ANSI→HTML rendering)
- **Preview**: highlight.js, marked, mermaid, katex, pdfjs-dist
- **Tauri plugins**: plugin-fs, plugin-dialog, plugin-opener (opener is broken on Android — use AndroidFileOpener JS interface instead)

## Key Architecture Decisions

- Terminal rendering uses ANSI→HTML + native CSS scrolling (not xterm.js canvas)
- Chat view parses raw ANSI terminal output into structured messages using ANSI color codes as semantic markers
- File opening on Android uses a custom `@JavascriptInterface` (`AndroidFileOpener`) in `MainActivity.kt`, NOT `tauri-plugin-opener` (which has a serialization bug on Android)
- WebSocket server only starts on desktop (`#[cfg(desktop)]`); mobile connects to a remote server
- All Tauri plugin imports are lazy-loaded via dynamic `import()` and gated behind `isTauri` / `isAndroid` checks
- Downloaded files on Android go to `/storage/emulated/0/Download/TmuxMobile/`, opened via FileProvider + Intent

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
├── App.svelte              # Router, nav, reconnect, theme, swipe tabs
├── lib/
│   ├── ws.js               # WebSocket client (auth, JSON-RPC, subscribe)
│   ├── Settings.svelte     # Connection form with address history
│   ├── Sessions.svelte     # Session/pane browser
│   ├── Terminal.svelte     # ANSI terminal + ChatView container, input bar
│   ├── ChatView.svelte     # Chat bubble renderer
│   ├── Files.svelte        # File browser, preview, editor, bookmarks, upload/download
│   ├── parsers.js          # Pluggable CLI output parsers (Kiro CLI)
│   └── Icon.svelte         # SVG icon system
src-tauri/src/
├── lib.rs                  # Tauri commands (save/list/delete downloads), mobile entry
├── main.rs                 # Desktop entry + integration tests
├── bin/server.rs           # Standalone server binary
├── server.rs               # WebSocket server (JSON-RPC, auth, subscribe, fs ops)
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
