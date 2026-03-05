# Architecture

## Overview
Tauri 2 cross-platform app (Rust backend + Svelte 5 frontend) that exposes tmux operations via WebSocket JSON-RPC. Terminal uses ANSI→HTML rendering with native CSS scrolling. Includes a Chat view that parses CLI agent output (Kiro CLI) into a messaging UI, and a full file browser with preview/edit/upload/download.

## Project Layout
- `src/` — Svelte 5 frontend (Vite 6 build)
  - `App.svelte` — page router (Settings → Sessions → Terminal/Chat/Files), auto-reconnect, nav tabs, theme, swipe navigation, Android back gesture via history API
  - `lib/ws.js` — WebSocket client (auth, JSON-RPC calls, subscribe/unsubscribe, pending promise cleanup on disconnect)
  - `lib/Settings.svelte` — connection form (host/port/token, address history in localStorage)
  - `lib/Sessions.svelte` — session list, auto-expand, sort active first, kill confirmation, new session/window
  - `lib/Terminal.svelte` — ANSI→HTML terminal with ansi-to-html, light/dark color maps, ChatView container, input bar with shortcuts
  - `lib/ChatView.svelte` — chat bubble renderer (ANSI→HTML, markdown, diff, code blocks, tool cards, model selector, compact cards)
  - `lib/Files.svelte` — file browser, bookmarks, preview (Markdown/CSV/HTML/PDF/image/code), text editor, upload/download, local downloads management
  - `lib/parsers.js` — pluggable CLI output parser registry (Kiro CLI parser)
  - `lib/Icon.svelte` — SVG icon system (Lucide-based, no emoji)
- `src-tauri/src/` — Rust backend
  - `lib.rs` — Tauri commands (`save_to_downloads`, `list_downloads`, `delete_download`, `get_download_path` with `sanitize_filename`), plugin init, mobile entry point. WS server only starts on desktop (`#[cfg(desktop)]`).
  - `main.rs` — Desktop entry point + integration tests (sequential, need running tmux)
  - `bin/server.rs` — standalone WS server binary (no Tauri, just websocket)
  - `server.rs` — WebSocket server: token auth, JSON-RPC routing, subscribe/unsubscribe with 200ms polling, filesystem operations, bookmarks
  - `tmux.rs` — tmux CLI wrapper (`capture-pane -p -e -J`, send_keys, send_command, new_session, kill_session, list_panes, pane_command). Supports `-S` custom socket path via RwLock global.
  - `fs.rs` — filesystem operations: list_dir, stat_file, read_file (512KB), write_file, create_dir, delete_path, rename_path, download_file (50MB limit, base64), upload_file. `resolve_path` handles `~` expansion.
  - `config.rs` — config file loader (`~/.config/tmux-mobile/config.toml`), auto-generates token, env var overrides, bookmarks persistence
- `src-tauri/gen/android/` — Android-specific code
  - `MainActivity.kt` — custom `FileOpener` JS interface (`@JavascriptInterface`), edge-to-edge, keyboard height detection, safe area insets to WebView
  - `AndroidManifest.xml` — permissions, FileProvider, cleartext traffic
  - `res/xml/file_paths.xml` — FileProvider shared paths (Download/TmuxMobile)

## Server Design
- Token auth: first message must be `{"method":"auth","params":{"token":"..."}}`
- Config: `~/.config/tmux-mobile/config.toml` with auto-generated persistent token
- Environment variables (TOKEN, HOST, PORT) override config file
- Subscription loop: polls `capture_pane` every 200ms per connection, pushes `pane_output` on content change
- capture_pane flags: `-p` (stdout), `-e` (ANSI escapes), `-J` (join soft-wrapped lines)
- Full filesystem access by design (remote management tool)

## Chat View Architecture
- **Parser registry** (`parsers.js`): pluggable system, `detectParser()` auto-selects parser
- **Kiro CLI parser**: uses ANSI color codes as semantic markers before stripping
  - Color 93 (purple) = user prompt `>`
  - Color 141 (light purple) = agent response `>`
  - Color 240 (gray) = system hint (skipped)
- **Message roles**: user, agent, system (slash command output)
- **Block types**: text (markdown), code (fenced), tool (collapsible), diff (red/green lines)
- **ANSI→HTML**: 256-color palette, dark color readability adjustment (`ensureReadable`)

## File Browser
- Browse remote FS starting from session's working directory
- Preview: Markdown (rendered with resolved images, mermaid diagrams), CSV (table), HTML (sandboxed iframe), PDF (pdf.js), images, code (highlight.js with 14 languages)
- Text editor with save, inline editing
- Upload: Tauri file picker on desktop/Android, `<input>` fallback in browser. Base64 encoded, chunked for large files.
- Download: Android saves to `/storage/emulated/0/Download/TmuxMobile/`, desktop uses save dialog, browser uses blob download
- File opening on Android: custom `AndroidFileOpener` JS interface → FileProvider URI → ACTION_VIEW Intent
- Bookmarks: per-connection directory bookmarks stored server-side

## Frontend Design
- Svelte 5 with runes ($state, $derived, $effect, $props)
- Terminal and Files components kept alive (CSS visibility:hidden) when switching tabs
- Chat tab only shown when parser detects supported CLI tool
- SVG icon system (Icon.svelte) — no emoji anywhere
- Mobile: `100dvh`, fixed body, `overscroll-behavior: none`, safe area insets via CSS custom properties
- Auto-reconnect with exponential backoff (max 20 attempts, up to 5s delay)
- State restore on reload using localStorage
- Swipe left/right between tabs, swipe right from edge for Files back navigation
- Android back gesture via history.pushState/popstate

## Android Platform
- File downloads: `/storage/emulated/0/Download/TmuxMobile/`
- File opening: `MainActivity.kt` → `FileOpener` inner class → `@JavascriptInterface` → `FileProvider.getUriForFile` → `Intent.ACTION_VIEW`
- `tauri-plugin-opener` has a serialization bug on Android (`OpenArgs` deserialization) — always use `AndroidFileOpener` instead
- `attachFileOpener` retries up to 50 times (5s) waiting for WebView to appear in view hierarchy
- Cleartext traffic (ws://) enabled via network security config
- Keyboard height detection via `OnGlobalLayoutListener`, passed to WebView as CSS custom property
- Safe area insets (status bar, nav bar) via `WindowInsetsCompat`, passed as `--sat`/`--sab` CSS properties

## Phase Status
- [x] WebSocket server with auth, subscribe, error handling
- [x] Tauri 2 integration with Svelte frontend
- [x] ANSI→HTML terminal with theme-aware colors
- [x] Chat view with Kiro CLI parser
- [x] Config persistence (~/.config/tmux-mobile/config.toml)
- [x] SVG icon system
- [x] Mobile web support (viewport, touch, auto-reconnect)
- [x] Android build (cleartext ws://, FileProvider, download/open)
- [x] File browser (browse, preview, edit, upload/download, bookmarks)
- [x] Light/Dark/System theme with CSS variables
- [x] PDF preview (pdf.js), image preview, syntax highlighting (highlight.js)
- [x] Markdown preview with mermaid diagrams, KaTeX math, resolved images
- [x] /model interactive selector, /compact summary card
- [x] Settings panel (gear button → theme, connection, disconnect)
- [x] Tauri desktop auto-fills config from local file
- [x] Signed Android APK + macOS .dmg builds
- [x] Android back gesture, swipe tab navigation
- [ ] iOS target (needs Xcode + xcodegen)
