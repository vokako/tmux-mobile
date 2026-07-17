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
  <img src="https://img.shields.io/badge/xterm.js_6-Terminal-00d4ff?style=flat-square" alt="xterm.js 6">
</p>

---

## What is this?

You're running [Kiro CLI](https://kiro.dev), Claude Code, or any coding agent in a tmux session on your Mac. You walk away from your desk. **tmux-mobile** lets you keep watching and interacting from your phone:

- **Terminal view** — watch and interact with your tmux session right from your phone, with touch scrolling, on-screen shortcut keys, and quick window switching
- **File browser** — browse, preview, and edit project files from the phone, bookmark the directories you visit often, and run common git actions in place
- **Sessions** — browse all tmux sessions/windows/panes, create or kill sessions, pull-to-refresh
- **Team (multi-agent)** — spin up a roster of coding agents (Kiro / Claude Code / Codex) that collaborate in a shared group chat; watch them work live, tap any agent to preview its pane
- **Settings** — terminal font/family/line-spacing controls, native desktop interface scaling, light/dark/auto theme, language switching (EN/中文), connection info, and diagnostics
- **Multi-address reconnect** — server machine ID tracks alternate addresses, auto-failover on disconnect; socket, encryption, and queued-send state are isolated across reconnects

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

The same tmux session you have at your desk, on your phone:

- Smooth touch scrolling with momentum, like a native app
- Shortcut keys (Esc, Tab, Ctrl-C, arrows) plus a one-shot Ctrl modifier for any letter, with long-press repeat where applicable
- Keyboard toggle so you can read without it in the way
- Window switcher with agent icons (Kiro / Claude / Codex) — jump between panes in one tap
- Floating "scroll to bottom" with a new-output indicator when you've scrolled up to look at history
- Light/dark theme follows your device; terminal font size, installed font family, and line spacing are adjustable
- Status bar shows the current session/pane and the running command

### File Browser

Browse the project on your Mac from the phone:

- Tap into folders, preview files in place — Markdown rendered, code with syntax colors, images, PDFs, CSV tables, HTML
- Edit text files right there, with undo and a confirm-before-losing changes
- Upload, download, rename, delete; star directories you visit often so they're one tap away
- Show or hide hidden files; pull to refresh; swipe in from the left edge to go back
- **Git in your pocket** — see status, stage files, browse the diff, scan the commit log, then commit and push, all without leaving the phone

### Connection

- Paste an address and a token, you're in — the server auto-generates the token on first launch
- Recent connections are remembered for quick switching between Macs
- If your IP or network changes, the app reconnects automatically using known alternate addresses for the same Mac
- On macOS, Cmd `+` / `-` / `0` scales the complete interface; terminal font size remains an independent setting
- Reload the page and you come back to where you were — same session, same view
- Works with a custom tmux socket if you use `-S`

### Team (multi-agent)

<img width="3232" height="1816" alt="longshot_2235-2248_2x" src="https://github.com/user-attachments/assets/095b0a83-2e09-4cec-8f00-0861a2a97cee" />

Spin up a **roster of coding agents** that collaborate on a shared task and watch
them work from your phone. Desktop-server only (the agent bus runs in-process).

![Team architecture](docs/design-docs/features/team-architecture.svg)

- **One group chat, many agents** — you and several agents (Kiro CLI / Claude Code / Codex) work in the same conversation. Talk to one with `@name`; use `@all` when every agent must reply. Each agent has its own pane, so you can tap any of them to watch what it's doing live.
- **One team per project folder** — bind a team to a working directory; run several teams in parallel for different projects, each kept neatly separate.
- **Ready-made and your own** — start from a built-in roster (`mixed-engineering` combines Kiro, Claude, and Codex; `software-dev` covers a larger delivery team; research/content/data rosters are also included), or design your own team — pick the roles, goals, tools, and skills you want — directly in the in-app template editor.
- **A live collaboration graph** — a ring of participants with breathing, status-coloured nodes; arcs trace the messages between them.
- **They keep themselves alive** — agents quietly report whether they're idle, thinking, working, or stuck; the system nudges a stuck one back on track automatically.

## npm Scripts

| Script | Description |
|--------|-------------|
| `npm run dev` | Vite dev server (web UI on 0.0.0.0:5173) |
| `npm run build` | Production web build |
| `npm run build:mac` | macOS desktop app (.app + .dmg) |
| `npm run build:android` | Android APK (aarch64) |
| `npm run build:all` | Web + macOS + Android |
| `npm run tauri:dev` | Desktop app + WS server (dev mode) |
| `npm run tauri:dev:release` | Release-mode desktop app + WS server |

Use the project scripts for Tauri. `pnpx tauri` invokes `pnpm dlx` and resolves
the unrelated `tauri` package, while positional `release` is passed to the
runner instead of enabling release mode. The supported command is
`npm run tauri:dev:release`.

Cargo uses the machine's normal parallelism by default. On a memory-constrained
machine, reduce it for one run with
`CARGO_BUILD_JOBS=2 npm run tauri:dev:release`.

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

Not implemented yet. The expected bootstrap is tracked in
[`docs/unresolved.md`](docs/unresolved.md); it still needs Tauri iOS project
initialization, Xcode/xcodegen integration, signing, and device validation.

## API

JSON-RPC over WebSocket. Full method reference + auth flow:
[`docs/requirements/api-contracts/websocket-rpc.md`](docs/requirements/api-contracts/websocket-rpc.md).

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

## License

MIT
