# tmux-mobile

Remote tmux session management over WebSocket. Control your Mac's tmux sessions from any device — phone, tablet, or another computer.

Built with Tauri 2 (Rust backend) + Svelte 5 (frontend) + xterm.js (terminal rendering).

## Quick Start

```bash
npm install
npm run tauri:dev
```

The app opens a desktop window and starts the WebSocket server on `ws://0.0.0.0:9876` with a generated auth token printed to the console.

### Server-only mode (no GUI)

```bash
cd src-tauri && cargo run --bin server
```

### Frontend dev (without Tauri)

Start the WS server in one terminal, Vite dev server in another:

```bash
cd src-tauri && TOKEN=mytoken cargo run --bin server
# in another terminal:
npm run dev
# open http://localhost:5173
```

### Browser test client

Open `test.html` in a browser to exercise all WebSocket methods interactively.

### Desktop build

```bash
npm run tauri:build
```

Produces `.app` and `.dmg` in `src-tauri/target/release/bundle/`.

### Configuration

| Variable | Default | Description |
|----------|---------|-------------|
| `HOST` | `0.0.0.0` | Bind address |
| `PORT` | `9876` | Listen port |
| `TOKEN` | auto-generated UUID | Auth token |

```bash
TOKEN=my-secret-token PORT=8080 npm run tauri:dev
```

## npm Scripts

| Script | Description |
|--------|-------------|
| `npm run dev` | Vite dev server only (frontend) |
| `npm run build` | Vite production build |
| `npm run tauri:dev` | Tauri dev mode (desktop window + WS server + Vite) |
| `npm run tauri:build` | Tauri production build (desktop bundles) |
| `npm run tauri:android` | Tauri Android dev mode |
| `npm run tauri:android:build` | Tauri Android production build |

## Mobile Targets

### Android — Ready

Android project initialized in `src-tauri/gen/android/`. To build:

```bash
# Install Rust Android targets (one-time)
rustup target add aarch64-linux-android armv7-linux-androideabi i686-linux-android x86_64-linux-android

# Dev on connected device/emulator
npm run tauri:android

# Production build
npm run tauri:android:build
```

Requires: Android SDK, NDK, Java 17+. Set `ANDROID_HOME` and `JAVA_HOME`.

### iOS — Manual Setup Required

iOS target not yet initialized. To set up:

```bash
# Install xcodegen (required by Tauri)
brew install xcodegen

# Install Rust iOS targets
rustup target add aarch64-apple-ios aarch64-apple-ios-sim

# Initialize iOS project
npx tauri ios init

# Dev on simulator
npx tauri ios dev

# Build
npx tauri ios build
```

Requires: Xcode (full install, not just Command Line Tools), Apple Developer account for device builds.

## App Pages

- **Settings** — Enter server host, port, and token to connect
- **Sessions** — View all tmux sessions, expand to see panes, create/kill sessions
- **Terminal** — Real-time pane output via xterm.js with ANSI color support, send commands and special keys (Ctrl-C, Tab, arrows)

## Project Structure

```
├── src/                  # Svelte 5 frontend
│   ├── App.svelte        # Main app with page routing
│   ├── lib/
│   │   ├── Settings.svelte   # Connection settings page
│   │   ├── Sessions.svelte   # Session list page
│   │   ├── Terminal.svelte    # Terminal view (xterm.js)
│   │   └── ws.js             # WebSocket client library
│   └── main.js
├── src-tauri/            # Rust backend (Tauri 2)
│   ├── src/
│   │   ├── lib.rs        # Library crate (tmux + server modules)
│   │   ├── main.rs       # Tauri entry + tests
│   │   ├── bin/server.rs # Standalone WS server binary
│   │   ├── server.rs     # WebSocket server (JSON-RPC + auth + subscribe)
│   │   └── tmux.rs       # tmux CLI wrapper
│   ├── gen/android/      # Generated Android project
│   ├── tauri.conf.json
│   └── Cargo.toml
├── test.html             # Browser-based WS test client
├── index.html
├── package.json
└── vite.config.js
```

## WebSocket Protocol

JSON-RPC style messages over WebSocket. Every request/response is a JSON object.

### Authentication

The first message on any connection **must** be an auth request. The server closes the connection on failure.

```json
→ {"method": "auth", "params": {"token": "your-token"}}
← {"id": null, "result": {"authenticated": true}}
```

### Methods

All methods require authentication first.

#### list_sessions

```json
→ {"id": 1, "method": "list_sessions"}
← {"id": 1, "result": [{"name": "main", "windows": 2, "attached": true, "created": "..."}]}
```

#### list_panes

```json
→ {"id": 2, "method": "list_panes", "params": {"session": "main"}}
← {"id": 2, "result": [{"session": "main", "window": 0, "pane": 0, "width": 80, "height": 24, "current_command": "zsh"}]}
```

#### capture_pane

```json
→ {"id": 3, "method": "capture_pane", "params": {"target": "main:0.0", "lines": 50}}
← {"id": 3, "result": {"output": "$ ls\nfile1.txt\nfile2.txt\n"}}
```

`lines` is optional (defaults to 200).

#### send_keys

```json
→ {"id": 4, "method": "send_keys", "params": {"target": "main:0.0", "keys": "ls -la", "literal": true}}
← {"id": 4, "result": {"ok": true}}
```

Set `literal: true` for text input, `false` for special keys like `Enter`, `C-c`, etc.

#### send_command

Sends text + Enter in one call.

```json
→ {"id": 5, "method": "send_command", "params": {"target": "main:0.0", "command": "ls -la"}}
← {"id": 5, "result": {"ok": true}}
```

#### new_session / kill_session

```json
→ {"id": 6, "method": "new_session", "params": {"name": "dev"}}
→ {"id": 7, "method": "kill_session", "params": {"name": "dev"}}
```

### Real-time Pane Output (Subscribe)

Subscribe to a pane to receive content updates pushed from the server every 200ms (only when content changes).

```json
→ {"method": "subscribe", "params": {"target": "main:0.0"}}
← {"id": null, "result": {"subscribed": "main:0.0"}}

// Server pushes when pane content changes:
← {"id": null, "method": "pane_output", "params": {"target": "main:0.0", "content": "full pane content..."}}
```

Unsubscribe:

```json
→ {"method": "unsubscribe", "params": {"target": "main:0.0"}}
```

### Error Codes

| Code | Meaning |
|------|---------|
| -32700 | Parse error (invalid JSON) |
| -32601 | Method not found |
| -32602 | Invalid params (missing required field) |
| -32603 | Internal error (tmux command failed) |
| -32000 | Auth error |

## Prerequisites

- macOS with tmux installed and running
- Rust toolchain
- Node.js
- For Android: Android SDK + NDK + Java 17
- For iOS: Xcode + xcodegen

## Testing

```bash
# Ensure tmux is running with at least one session
tmux new-session -d -s test

# Run tests (must be serial due to shared tmux state)
cd src-tauri && cargo test -- --test-threads=1
```

## License

MIT
