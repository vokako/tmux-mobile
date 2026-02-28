# Architecture

## Overview
Tauri 2 cross-platform app (Rust backend + Svelte 5 frontend + xterm.js) that exposes tmux operations via WebSocket JSON-RPC. Includes a Chat view that parses CLI agent output (Kiro CLI) into a messaging UI.

## Project Layout
- `src/` — Svelte 5 frontend (Vite build)
  - `App.svelte` — page router (Settings → Sessions → Terminal/Chat), auto-reconnect, nav tabs
  - `lib/ws.js` — WebSocket client (auth, RPC calls, subscribe)
  - `lib/Settings.svelte` — connection form (host/port/token, persisted to localStorage)
  - `lib/Sessions.svelte` — session list, auto-expand, sort active first, kill confirmation, disconnect
  - `lib/Terminal.svelte` — xterm.js terminal + ChatView container, status line, input modes
  - `lib/ChatView.svelte` — chat bubble renderer (ANSI→HTML, markdown, diff, code blocks, tool cards)
  - `lib/parsers.js` — pluggable CLI output parser registry (Kiro CLI parser)
  - `lib/Icon.svelte` — SVG icon system (all icons, no emoji)
- `src-tauri/src/` — Rust backend
  - `lib.rs` — library crate exposing tmux + server + config modules
  - `main.rs` — Tauri entry, starts WS server as background task, contains tests
  - `bin/server.rs` — standalone WS server binary
  - `server.rs` — WebSocket server with token auth, JSON-RPC routing, subscribe/unsubscribe
  - `tmux.rs` — tmux CLI wrapper (capture-pane with -e -J flags for ANSI + joined lines)
  - `config.rs` — config file loader (~/.config/tmux-mobile/config.toml), auto-generates token

## Server Design
- Token auth: first message must be `{"method":"auth","params":{"token":"..."}}`
- Config: `~/.config/tmux-mobile/config.toml` with auto-generated persistent token
- Environment variables (TOKEN, HOST, PORT) override config file
- Subscription loop: polls `capture_pane` every 200ms, pushes on content change
- capture_pane flags: `-p` (stdout), `-e` (ANSI escapes), `-J` (join soft-wrapped lines), `-s` (all windows)

## Chat View Architecture
- **Parser registry** (`parsers.js`): pluggable system, `detectParser()` auto-selects parser
- **Kiro CLI parser**: uses ANSI color codes as semantic markers before stripping
  - Color 93 (purple) = user prompt `>`
  - Color 141 (light purple) = agent response `>`
  - Color 240 (gray) = system hint (skipped)
- **Message roles**: user, agent, system (slash command output)
- **Block types**: text (markdown), code (fenced), tool (collapsible), diff (red/green lines)
- **ANSI→HTML**: 256-color palette, dark color readability adjustment (`ensureReadable`)
- **Rendering**: Svelte 5 runes, `{@html}` for rendered content, auto-scroll with bottom detection

## Frontend Design
- Svelte 5 with runes ($state, $derived, $effect, $props)
- Terminal component kept alive (CSS hidden) when switching to Sessions
- Chat tab only shown when parser detects supported CLI tool
- SVG icon system (Icon.svelte) — no emoji anywhere
- Mobile: `100dvh`, fixed body, `overscroll-behavior: none`, safe area insets
- Auto-reconnect on page reload using saved localStorage credentials

## Mobile Support
- Android: cleartext traffic enabled (ws://), network security config
- Viewport: `interactive-widget=resizes-content` for keyboard handling
- Touch scrolling: terminal uses viewport.scrollTop proxy (known issue: no inertia)

## Phase Status
- [x] WebSocket server with auth, subscribe, error handling
- [x] Tauri 2 integration with Svelte frontend
- [x] xterm.js terminal with ANSI color rendering
- [x] Chat view with Kiro CLI parser
- [x] Config persistence (~/.config/tmux-mobile/config.toml)
- [x] SVG icon system
- [x] Mobile web support (viewport, touch, auto-reconnect)
- [x] Android build config (cleartext ws://)
- [x] File browser (browse, preview, edit, upload/download)
- [x] Light/Dark/System theme with CSS variables
- [x] PDF preview (pdf.js), image preview, syntax highlighting
- [x] /model interactive selector, /compact summary card
- [x] Settings panel (gear button → theme, connection, disconnect)
- [x] Tauri desktop auto-fills config from local file
- [x] Signed Android APK + macOS .dmg builds
- [ ] iOS target (needs Xcode + xcodegen)
- [ ] Terminal touch scrolling inertia (bug)
