# tmux-mobile - Project Status

**Created:** 2026-02-26
**Last updated:** 2026-04-04
**Status:** Production-ready (Desktop + Android + Browser)

## What Works

### Core Features
- **WebSocket Server** (Rust)
  - Token-based auth with optional E2E encryption (AES-GCM via HKDF key derivation)
  - Plain token fallback for HTTP contexts (no Web Crypto)
  - JSON-RPC API: auth, list_sessions, list_panes, capture_pane, send_keys, send_command, new_session, kill_session, new_window, kill_window, pane_command, resize_pane, subscribe/unsubscribe
  - Filesystem API: fs_cwd, fs_list, fs_stat, fs_read, fs_write, fs_mkdir, fs_delete, fs_rename, fs_download, fs_upload
  - Bookmarks: get_bookmarks, save_bookmarks
  - Real-time pane output via subscribe (200ms polling with cursor info)
  - Per-connection resize tracking with auto-restore on disconnect
  - Standalone server mode (`cargo run --bin server`)

- **Frontend** (Svelte 5 + Vite)
  - Settings — connect with host/port/token, address history, auto ws/wss detection
  - Sessions — list sessions, expand panes, AI tags (Kiro icon, Claude badge), create/kill sessions/windows
  - Terminal — xterm.js v6 with custom mobile touch handling:
    - Touch scroll with momentum physics (friction, distance-capped velocity)
    - Custom scrollbar drag (xterm.js v6 scrollbar is mouse-only)
    - Long-press word selection with drag-to-extend and tap-to-copy
    - Shortcut key rows (Esc, Tab, Ctrl-C/D, arrows, Home/End, Backspace)
    - Input box mode (toggle via chat icon) for weak network usage
    - Window switcher for multi-window sessions
  - Chat — structured message view for CLI agents (Kiro CLI parser, Claude Code parser stub)
  - Files — browser, editor, preview (syntax highlight, markdown, mermaid, katex, PDF), bookmarks, upload/download
  - Theme — dark/light/system with full theme support
  - Navigation — swipe between tabs, Android back gesture support, slide animations
  - Reconnect — auto-reconnect on disconnect, resume on app foreground
  - Debug overlay — draggable panel for mobile debugging

- **Tauri 2 Integration**
  - Desktop build: macOS .app + .dmg
  - Android build: APK (aarch64)
  - Android-specific: keyboard height detection via OnGlobalLayoutListener, file opening via FileProvider + Intent
  - Edge-to-edge display with safe area insets

### Testing
- Rust integration tests (`cargo test -- --test-threads=1`)
- Browser + Android app tested for terminal interactions
- Keyboard handling tested on both mobile browser and Android WebView

### Build Output
- **Desktop:** `tmux-mobile.app` + `.dmg`
- **Android:** APK via `npx tauri android build --target aarch64`
- **Web:** Static files in `dist/` (served via any HTTP server)

## Project Structure

```
src/
├── App.svelte              # Router, nav, reconnect, theme, keyboard detection (~730 lines)
├── main.js                 # Entry point
├── lib/
│   ├── ws.js               # WebSocket client + E2E encryption (~250 lines)
│   ├── Settings.svelte     # Connection settings (~344 lines)
│   ├── Sessions.svelte     # Session/pane browser with AI tags (~541 lines)
│   ├── Terminal.svelte     # xterm.js terminal + touch handling (~1010 lines)
│   ├── ChatView.svelte     # Chat bubble renderer (~811 lines)
│   ├── Files.svelte        # File browser/editor/preview (~1163 lines)
│   ├── parsers.js          # CLI output parsers (~483 lines)
│   └── Icon.svelte         # SVG icon system (~101 lines)
src-tauri/src/
├── lib.rs                  # Tauri commands, mobile entry
├── main.rs                 # Desktop entry + tests
├── bin/server.rs           # Standalone server binary
├── server.rs               # WebSocket server (~869 lines)
├── tmux.rs                 # tmux CLI wrapper (~392 lines)
├── fs.rs                   # Filesystem operations (~309 lines)
└── config.rs               # Config loader
```

## Key Technologies

- **Backend:** Rust + Tokio + tokio-tungstenite
- **Frontend:** Svelte 5 + Vite 6 + xterm.js v6
- **Desktop:** Tauri 2
- **Mobile:** Tauri 2 Android (WebView + native keyboard integration)

## Development

```bash
npm run dev              # Web dev server (0.0.0.0:5173)
npm run tauri:dev        # Desktop app + WS server
npm run build:mac        # macOS .app + .dmg
npm run build:android    # Android APK (aarch64)
cd src-tauri && cargo run --bin server   # Standalone WS server
cd src-tauri && cargo test -- --test-threads=1
```
