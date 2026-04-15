<p align="center">
  <img src="assets/icon.svg" width="128" alt="tmuxmobile"><br>
  <img src="assets/logo.svg" width="220" alt="tmuxmobile">
</p>

<p align="center">
  <strong>Remotely monitor and control your coding agents from your phone.</strong><br>
  <sub>Connect to tmux sessions running on your Mac/Linux over WebSocket — view terminal output, chat with AI agents, browse files, and send commands from any device.</sub>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Tauri_2-Rust-orange?style=flat-square" alt="Tauri 2">
  <img src="https://img.shields.io/badge/Svelte_5-Frontend-ff3e00?style=flat-square" alt="Svelte 5">
  <img src="https://img.shields.io/badge/ansi--to--html-Terminal-00d4ff?style=flat-square" alt="Terminal">
</p>

---

## What is this?

You're running [Kiro CLI](https://kiro.dev), Claude Code, or any coding agent in a tmux session on your Mac. You walk away from your desk. **tmux-mobile** lets you keep watching and interacting from your phone:

- **Terminal view** — xterm.js with theme-aware colors, touch scrolling with iOS-like momentum, shortcut keys with long-press repeat, collapsible window switcher with AI agent icons
- **Chat view** — AI agent conversations rendered as chat bubbles, with markdown rendering, syntax-highlighted code blocks, collapsible tool calls, diff rendering, `/model` selector, and `/compact` summary cards
- **File browser** — browse, preview, edit, upload/download files with bookmarks, git integration (status, diff, log, add, commit, push)
- **Sessions** — browse all tmux sessions/windows/panes, create or kill sessions, pull-to-refresh
- **Settings** — font size control, light/dark/auto theme with smooth transitions, language switching (EN/中文), server connection info (hostname, machine ID), debug toggle
- **Multi-address reconnect** — server machine ID tracks alternate addresses, auto-failover on disconnect

The server runs on your Mac, the UI runs in any browser or as a native app (macOS, Android).

## Quick Start

```bash
npm install

# Option 1: Desktop app (Tauri window + WS server, auto-fills config)
npm run tauri:dev

# Option 2: Server + browser
cd src-tauri && cargo run --bin server   # starts WS server on :9899
npm run dev                               # starts web UI on :5173
```

On first launch, a token is auto-generated and saved to `~/.config/tmux-mobile/config.toml`. The Tauri desktop app auto-fills connection settings from this config.

Open `http://<your-mac-ip>:5173` on your phone, enter the address (`ws://host:port`) and token, and you're in.

## Configuration

Config file: `~/.config/tmux-mobile/config.toml`

```toml
token = "auto-generated-uuid"
host = "0.0.0.0"    # optional
port = 9899          # optional
tmux_socket = ""     # optional, -S path
tls_cert = ""        # optional, path to PEM cert for wss://
tls_key = ""         # optional, path to PEM private key for wss://
```

Environment variables override the config file:

```bash
TOKEN=my-secret PORT=8080 npm run tauri:dev
# WSS mode:
TLS_CERT=/path/to/cert.pem TLS_KEY=/path/to/key.pem npm run tauri:dev
```

## Features

### Terminal

- xterm.js with theme-aware color schemes (light/dark)
- Touch scrolling with velocity smoothing and iOS-like momentum physics
- Shortcut buttons (Esc, ^C, ^D, Tab, arrows) with long-press repeat
- Keyboard toggle button to show/hide on-screen keyboard
- Collapsible window switcher — shows AI agent icons (Kiro/Claude) or command name, persists state
- Floating buttons (scroll-to-bottom, window switcher) with frosted glass style
- Configurable font size and font family (Maple Mono NF CN)
- Status bar showing session:pane and running command

### Chat View

Auto-detects supported CLI tools (currently Kiro CLI) and renders output as a messaging UI:

- User messages → right-aligned bubbles with copy button
- Agent responses → left-aligned bubbles with bot avatar, markdown rendered
- Code blocks → syntax-highlighted cards
- Tool calls → compact collapsible cards
- Diffs → red/green line-by-line rendering
- `/compact` → styled summary card with markdown rendering
- `/model` → interactive model selector (tap to switch models)
- Thinking state → debounced spinner animation
- ANSI colors preserved throughout
- Markdown rendering (tables, bold, italic, inline code, links, lists)
- Immediate chat detection on session open

The parser architecture is pluggable — add new CLI tools in `src/lib/parsers.js`.

### File Browser

Browse the server's filesystem starting from the session's working directory:

- Unified toolbar: all actions in one compact icon row
- Directory navigation with breadcrumbs on separate path row
- Bookmarks: star current directory, bookmark panel with scrollable paths
- File preview: Markdown (rendered), CSV (table), code (syntax highlighted with Maple Mono font), HTML (iframe), PDF (pdf.js), images
- Text file editor with syntax highlighting, undo stack, save, unsaved changes confirmation
- File operations: create file/folder, rename, delete, upload, download
- File info: path (tap to copy), type, size, modified, permissions
- Show/hide hidden files, pull-to-refresh (mobile)
- Swipe right from left edge to go back
- **Git integration**: status view with per-file stage/unstage, GitHub-style diff viewer, commit log, add all/commit/push actions

### Connection

- Address field: `ws://host:port` or `wss://host:port`
- Address history: cached recent connections with token, quick switching
- Auto-reconnect on network disconnect with multi-address failover (same machine ID)
- Server info display: hostname, machine ID, address
- State restore on reload (page, session, view mode)
- tmux socket support (`-S` path)

## npm Scripts

| Script | Description |
|--------|-------------|
| `npm run dev` | Vite dev server (web UI on 0.0.0.0:5173) |
| `npm run build` | Production web build |
| `npm run build:mac` | macOS desktop app (.app + .dmg) |
| `npm run build:android` | Android APK (aarch64) |
| `npm run build:all` | Web + macOS + Android |
| `npm run tauri:dev` | Desktop app + WS server (dev mode) |

## Build Targets

### macOS

```bash
npm run build:mac
# Output: src-tauri/target/release/bundle/dmg/tmux-mobile_*.dmg
```

### Android

```bash
rustup target add aarch64-linux-android
npm run build:android
# Output: src-tauri/gen/android/app/build/outputs/apk/universal/release/app-universal-release.apk
```

Signed APK with bundled keystore. Cleartext WebSocket (`ws://`) enabled.

Requires: Android SDK, NDK 28+, Java 17+.

### iOS

```bash
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
npx tauri ios init && npx tauri ios dev
```

Requires: Xcode, Apple Developer account for device builds.

## Project Structure

```
src/
├── App.svelte              # Main app, routing, nav, settings panel, theme
├── lib/
│   ├── Settings.svelte     # Connection form with address history
│   ├── Sessions.svelte     # Session/pane browser with refresh
│   ├── Terminal.svelte     # ANSI terminal + Chat container, input bar
│   ├── ChatView.svelte     # Chat bubble renderer (messages, model, compact)
│   ├── Files.svelte        # File browser, preview, editor, bookmarks
│   ├── Icon.svelte         # SVG icon system (Lucide-based)
│   ├── parsers.js          # Pluggable CLI output parsers
│   ├── i18n.js             # Lightweight i18n (EN/中文), auto-detect locale
│   └── ws.js               # WebSocket client (tmux + filesystem RPC)
src-tauri/
├── src/
│   ├── lib.rs              # Library crate, Tauri commands, mobile entry point
│   ├── server.rs           # WebSocket server (JSON-RPC + auth + subscribe + fs)
│   ├── tmux.rs             # tmux CLI wrapper with socket support
│   ├── fs.rs               # Filesystem operations (list, read, write, upload, download)
│   ├── config.rs           # Config file loader, bookmarks
│   ├── main.rs             # Desktop entry point
│   └── bin/server.rs       # Standalone server binary
├── tauri.conf.json
└── Cargo.toml
```

## WebSocket Protocol

JSON-RPC over WebSocket. Connect with `ws://` or `wss://`. First message must authenticate:

```json
→ {"method": "auth", "params": {"token": "..."}}
← {"result": {"authenticated": true, "machine_id": "uuid", "hostname": "my-mac"}}
```

### Methods

| Method | Params | Description |
|--------|--------|-------------|
| `list_sessions` | — | List all tmux sessions |
| `list_panes` | `session` | List all panes across all windows |
| `capture_pane` | `target`, `lines?` | Capture pane content with ANSI colors |
| `send_keys` | `target`, `keys`, `literal` | Send keystrokes |
| `send_command` | `target`, `command` | Send text + Enter |
| `new_session` | `name?` | Create session |
| `kill_session` | `name` | Kill session |
| `subscribe` | `target` | Stream pane updates (200ms polling) |
| `unsubscribe` | `target` | Stop streaming |
| `pane_command` | `target` | Get current command running in pane |
| `set_socket` | `path` | Set tmux socket path at runtime |
| `get_bookmarks` | — | Get saved directory bookmarks |
| `save_bookmarks` | `bookmarks` | Save directory bookmarks |
| `fs_cwd` | `session` | Get session working directory |
| `fs_list` | `path`, `show_hidden?` | List directory contents |
| `fs_stat` | `path` | File metadata |
| `fs_read` | `path` | Read text file (≤512KB) |
| `fs_write` | `path`, `content` | Write text file |
| `fs_mkdir` | `path` | Create directory |
| `fs_delete` | `path` | Delete file or directory |
| `fs_rename` | `from`, `to` | Rename/move |
| `fs_download` | `path` | Download file as base64 (≤50MB) |
| `fs_upload` | `path`, `data` | Upload file (base64) |
| `git` | `subcmd`, `args[]`, `cwd?` | Git operations (whitelisted: status, diff, log, show, branch, rev-parse, push, add, commit, restore) |

## Prerequisites

- macOS or Linux with tmux installed
- Rust toolchain + Node.js
- Recommended: `set-option -g history-limit 50000` in tmux config

## Tailscale Integration

If you have Tailscale, serve the web UI over HTTPS:

```bash
tailscale serve --bg 5173
# Access from any device: https://your-machine.tailnet-name.ts.net/
# WebSocket: use wss:// with Tailscale domain or ws:// with Tailscale IP:9899
```

## Testing

```bash
tmux new-session -d -s test
cd src-tauri && cargo test -- --test-threads=1
```

## License

MIT
