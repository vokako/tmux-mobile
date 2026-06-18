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
- **File browser** — browse, preview, edit, upload/download files with bookmarks, git integration (status, diff, log, add, commit, push)
- **Sessions** — browse all tmux sessions/windows/panes, create or kill sessions, pull-to-refresh
- **Team (multi-agent)** — spin up a roster of coding agents (Kiro / Claude Code / Codex) that collaborate in a shared group chat; watch them work live, tap any agent to preview its pane
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
disconnect_grace_secs = 600  # optional, seconds to wait before restoring
                             # tmux window size after last client drops.
                             # 0 = restore immediately (legacy behavior).
```

Environment variables override the config file:

```bash
TOKEN=my-secret PORT=8080 npm run tauri:dev
# WSS mode:
TLS_CERT=/path/to/cert.pem TLS_KEY=/path/to/key.pem npm run tauri:dev
# Disable the grace period (restore window size immediately on disconnect):
DISCONNECT_GRACE_SECS=0 npm run tauri:dev
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

### Team (multi-agent)

Spin up a **roster of coding agents** that collaborate on a shared task and watch
them work from your phone. Desktop-server only (the agent bus runs in-process).

![Team architecture](docs/design-docs/features/team-architecture.svg)

- **One group chat, many agents** — the human and several agents (Kiro CLI /
  Claude Code / Codex) talk in a shared room over an append-only message bus.
  Address an agent with `@name`; `@all` reaches everyone. Each agent runs in its
  own tmux window, so you can **tap any agent to preview its live pane**.
- **Per-workspace teams** — a team is tied to a working directory; multiple teams
  run in parallel, each isolated in its own room and `tmm-team-<slug>` session.
- **Roster templates** (YAML) — built-ins for a `default` starter, `software-dev`
  (product / architect / frontend / backend / reviewer / tester / devops),
  `financial-research`, `deep-research`, `content-studio`, and `data-analysis`.
  Define your own under
  `~/.config/tmux-mobile/teams/<name>/team.yaml`: per-agent role/goal/model plus
  optional **extra MCP servers** and **skills** (a local path or a GitHub URL,
  fetched + cached), and **team-wide** `env` / `mcp` / `skills` / `prompt` that
  apply to every agent. Edit it all in the in-app template editor.
- **Live collaboration graph** — a ring of participants with status-coloured,
  breathing nodes and arcs that trace messages between them.
- **Liveness & self-heal** — agents report status (idle → thinking → working →
  hardworking → stalled) via a heartbeat; a wedged agent is nudged back into the
  loop automatically.

See `docs/design-docs/features/team.md` for the architecture (one-page diagram
of bus + agent loop + hooks: [`team-architecture.svg`](docs/design-docs/features/team-architecture.svg)).

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
│   ├── Terminal.svelte     # ANSI terminal, input bar, shortcut keys
│   ├── Files.svelte        # File browser, preview, editor, bookmarks
│   ├── Icon.svelte         # SVG icon system (Lucide-based)
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
| `fs_download` | `path` | Download file as base64 (≤50MB) — for inline preview |
| `fs_download_url` | `path` | Signed URL for the HTTP `/dl` streaming endpoint (no size limit); used by file download action |
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
