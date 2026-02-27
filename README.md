<p align="center">
  <img src="https://img.shields.io/badge/⌘_tmux-mobile-00d4ff?style=for-the-badge&labelColor=0a0a0f&color=00d4ff" alt="tmux-mobile">
</p>

<p align="center">
  <strong>Remotely monitor and control your coding agents from your phone.</strong><br>
  <sub>Connect to tmux sessions running on your Mac over WebSocket — view terminal output, chat with AI agents, and send commands from any device.</sub>
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
- **Chat view** — AI agent conversations rendered as chat bubbles (like WhatsApp), with syntax-highlighted code blocks, collapsible tool calls, and diff rendering
- **Sessions** — browse all tmux sessions/windows/panes, create or kill sessions

The server runs on your Mac, the UI runs in any browser or as a native app.

## Quick Start

```bash
npm install

# Option 1: Desktop app (Tauri window + WS server)
npm run tauri:dev

# Option 2: Server + browser
cd src-tauri && cargo run --bin server   # starts WS server on :9876
npm run dev                               # starts web UI on :5173
```

On first launch, a token is auto-generated and saved to `~/.config/tmux-mobile/config.toml`. The token persists across restarts.

Open `http://<your-mac-ip>:5173` on your phone, enter the host/port/token, and you're in.

## Configuration

Config file: `~/.config/tmux-mobile/config.toml`

```toml
token = "auto-generated-uuid"
host = "0.0.0.0"    # optional
port = 9876          # optional
```

Environment variables override the config file:

```bash
TOKEN=my-secret PORT=8080 npm run tauri:dev
```

## Chat View

The Chat view auto-detects supported CLI tools (currently Kiro CLI) and renders their output as a messaging UI:

- User messages → right-aligned blue bubbles
- Agent responses → left-aligned bubbles with bot avatar
- Code blocks → syntax-highlighted cards with language labels
- Tool calls → collapsible cards showing tool name and output
- Diffs → red/green line-by-line rendering
- System output (`/usage`, `/context`) → full-width cards preserving terminal formatting
- Thinking state → spinner animation
- ANSI colors preserved throughout
- Markdown rendering (tables, bold, italic, inline code, links, lists)

The parser architecture is pluggable — add new CLI tools in `src/lib/parsers.js`.

## npm Scripts

| Script | Description |
|--------|-------------|
| `npm run dev` | Vite dev server (web UI on 0.0.0.0:5173) |
| `npm run build` | Production build |
| `npm run tauri:dev` | Desktop app + WS server |
| `npm run tauri:build` | Desktop production build |
| `npm run tauri:android` | Android dev mode |
| `npm run tauri:android:build` | Android production build |

## Mobile Targets

### Android

```bash
rustup target add aarch64-linux-android
npm run tauri:android:build -- --target aarch64
```

Cleartext WebSocket (`ws://`) is enabled for both debug and release builds.

Requires: Android SDK, NDK, Java 17+.

### iOS

```bash
brew install xcodegen
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
npx tauri ios init
npx tauri ios dev
```

Requires: Xcode, Apple Developer account for device builds.

## Project Structure

```
src/
├── App.svelte              # Main app, routing, nav
├── lib/
│   ├── Settings.svelte     # Connection settings
│   ├── Sessions.svelte     # Session/pane browser
│   ├── Terminal.svelte     # Terminal + Chat container
│   ├── ChatView.svelte     # Chat bubble renderer
│   ├── Icon.svelte         # SVG icon system
│   ├── parsers.js          # Pluggable CLI output parsers
│   └── ws.js               # WebSocket client
src-tauri/
├── src/
│   ├── server.rs           # WebSocket server (JSON-RPC + auth + subscribe)
│   ├── tmux.rs             # tmux CLI wrapper
│   ├── config.rs           # Config file loader
│   ├── main.rs             # Tauri entry
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

## Prerequisites

- macOS with tmux installed
- Rust toolchain + Node.js
- Recommended: `set-option -g history-limit 50000` in tmux config

## Testing

```bash
tmux new-session -d -s test
cd src-tauri && cargo test -- --test-threads=1
```

## License

MIT
