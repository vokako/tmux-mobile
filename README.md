<p align="center">
  <img src="assets/logo.svg" width="320" alt="⌘ tmux-mobile"><br>
  <strong>tmux<span>mobile</span></strong>
</p>

<p align="center">
  <strong>Remotely monitor and control your coding agents from your phone.</strong><br>
  <sub>Connect to tmux sessions running on your Mac/Linux over WebSocket — view terminal output, chat with AI agents, browse files, and send commands from any device.</sub>
</p>

<p align="center">
  <img src="https://img.shields.io/badge/Tauri_2-Rust-orange?style=flat-square" alt="Tauri 2">
  <img src="https://img.shields.io/badge/Svelte_5-Frontend-ff3e00?style=flat-square" alt="Svelte 5">
  <img src="https://img.shields.io/badge/xterm.js-Terminal-00d4ff?style=flat-square" alt="xterm.js">
</p>

---

## What is this?

You're running [Kiro CLI](https://kiro.dev), Claude Code, or any coding agent in a tmux session on your Mac. You walk away from your desk. **tmux-mobile** lets you keep watching and interacting from your phone:

- **Terminal view** — full xterm.js rendering with ANSI colors, scrollback, and special keys
- **Chat view** — AI agent conversations rendered as chat bubbles, with syntax-highlighted code blocks, collapsible tool calls, diff rendering, `/model` selector, and `/compact` summary cards
- **File browser** — browse, preview, edit, upload/download files from the server's filesystem
- **Sessions** — browse all tmux sessions/windows/panes, create or kill sessions
- **Light/Dark theme** — system-following or manual toggle, applies to all views including terminal

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

Open `http://<your-mac-ip>:5173` on your phone, enter the host/port/token, and you're in.

## Configuration

Config file: `~/.config/tmux-mobile/config.toml`

```toml
token = "auto-generated-uuid"
host = "0.0.0.0"    # optional
port = 9899          # optional
```

Environment variables override the config file:

```bash
TOKEN=my-secret PORT=8080 npm run tauri:dev
```

## Features

### Chat View

Auto-detects supported CLI tools (currently Kiro CLI) and renders output as a messaging UI:

- User messages → right-aligned bubbles with copy button
- Agent responses → left-aligned bubbles with bot avatar
- Code blocks → syntax-highlighted cards
- Tool calls → collapsible cards showing tool name and output
- Diffs → red/green line-by-line rendering
- `/compact` → styled summary card with markdown rendering
- `/model` → interactive model selector (tap to switch models)
- Thinking state → debounced spinner animation
- ANSI colors preserved throughout
- Markdown rendering (tables, bold, italic, inline code, links, lists)

The parser architecture is pluggable — add new CLI tools in `src/lib/parsers.js`.

### File Browser

Browse the server's filesystem starting from the session's working directory:

- Directory navigation with breadcrumbs, home, parent, refresh
- File preview: Markdown (rendered), CSV (table), code (syntax highlighted), HTML (iframe), PDF (pdf.js), images
- Text file editor with syntax highlighting, undo stack, save
- File operations: create, rename, delete, upload, download
- File info: path, type, size, modified, permissions
- Show/hide hidden files

### Terminal

- Full xterm.js with ANSI 256-color support
- Light and dark terminal themes
- Shortcut buttons (⌃C, ⌃D, ⌃Z, Tab, ↑, ↓)
- Multi-line input (Shift+Enter for newline)
- IME-aware (composing Enter doesn't send)
- Status bar overlay showing session:pane and command

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
│   ├── Settings.svelte     # Connection form (auto-fills in Tauri desktop)
│   ├── Sessions.svelte     # Session/pane browser with refresh
│   ├── Terminal.svelte     # Terminal + Chat container, status bar
│   ├── ChatView.svelte     # Chat bubble renderer (messages, model, compact)
│   ├── Files.svelte        # File browser, preview, editor
│   ├── Icon.svelte         # SVG icon system (Lucide-based)
│   ├── parsers.js          # Pluggable CLI output parsers
│   └── ws.js               # WebSocket client (tmux + filesystem RPC)
src-tauri/
├── src/
│   ├── lib.rs              # Library crate, Tauri commands, mobile entry point
│   ├── server.rs           # WebSocket server (JSON-RPC + auth + subscribe + fs)
│   ├── tmux.rs             # tmux CLI wrapper
│   ├── fs.rs               # Filesystem operations (list, read, write, upload, download)
│   ├── config.rs           # Config file loader + Tauri command
│   ├── main.rs             # Desktop entry point
│   └── bin/server.rs       # Standalone server binary
├── tauri.conf.json
└── Cargo.toml
```

## WebSocket Protocol

JSON-RPC over WebSocket. First message must authenticate:

```json
→ {"method": "auth", "params": {"token": "..."}}
← {"result": {"authenticated": true}}
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

## Prerequisites

- macOS or Linux with tmux installed
- Rust toolchain + Node.js
- Recommended: `set-option -g history-limit 50000` in tmux config

## Tailscale Integration

If you have Tailscale, serve the web UI over HTTPS:

```bash
tailscale serve --bg 5173
# Access from any device: https://your-machine.tailnet-name.ts.net/
# WebSocket: use Tailscale IP + port 9899
```

## Testing

```bash
tmux new-session -d -s test
cd src-tauri && cargo test -- --test-threads=1
```

## License

MIT
