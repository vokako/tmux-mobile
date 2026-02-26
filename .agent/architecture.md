# Architecture

## Overview
Tauri 2 cross-platform app (Rust backend + Svelte 5 frontend + xterm.js) that exposes tmux operations via WebSocket JSON-RPC.

## Project Layout
- `src/` — Svelte 5 frontend (Vite build)
  - `App.svelte` — page router (Settings → Sessions → Terminal)
  - `lib/ws.js` — WebSocket client (auth, RPC calls, subscribe)
  - `lib/Settings.svelte` — connection form (host/port/token, persisted to localStorage)
  - `lib/Sessions.svelte` — session list with expandable panes, create/kill
  - `lib/Terminal.svelte` — xterm.js terminal with FitAddon, real-time pane output via subscribe
- `src-tauri/src/` — Rust backend
  - `lib.rs` — library crate exposing tmux + server modules
  - `main.rs` — Tauri entry, starts WS server as background task, contains all tests
  - `bin/server.rs` — standalone WS server binary (no Tauri dependency at runtime)
  - `server.rs` — WebSocket server with token auth, JSON-RPC routing, subscribe/unsubscribe
  - `tmux.rs` — tmux CLI wrapper (list sessions/panes, capture, send keys, session management)
- `test.html` — browser-based test client for exercising all WS methods

## Server Design
- Token auth: first message must be `{"method":"auth","params":{"token":"..."}}`, connection closed on failure
- Request routing: `handle_request()` dispatches to tmux functions, `subscribe`/`unsubscribe` managed per-connection
- Subscription loop: polls `capture_pane` every 200ms per subscribed target, only pushes when content changes
- tmux calls are blocking → wrapped in `spawn_blocking`
- Error codes follow JSON-RPC conventions (-32700, -32601, -32602, -32603, -32000)

## Frontend Design
- Svelte 5 with runes ($state, $effect, $props)
- No framework router — simple page state variable in App.svelte
- WebSocket client in ws.js handles auth, request/response correlation, and server push
- Settings persisted to localStorage
- Terminal subscribes on mount, unsubscribes on cleanup

## Phase Status
- [x] Phase 1: WebSocket server with auth, subscribe, error handling
- [x] Phase 2: Tauri 2 integration with Svelte frontend (3 pages)
- [x] xterm.js terminal rendering with ANSI support
- [x] Desktop build verified (.app + .dmg)
- [x] Android target initialized (src-tauri/gen/android/)
- [ ] iOS target (needs Xcode full install + xcodegen)
