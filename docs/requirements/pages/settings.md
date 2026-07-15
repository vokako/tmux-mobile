# Settings Page

## Purpose
Configure server connection, appearance, language, and app behavior.

The gear button opens Settings as a full-window workspace below the app nav,
not a small popover. The workspace avoids redundant page/section headings and
uses the compact tab navigation as its only title row. The tab bar and choice
controls reuse Terminal's 24px outlined chip language, including its spacing,
rounded shape, muted idle state, and accent active state. Settings uses two tabs:

- **Appearance** — theme, language, responsive layout, desktop interface scale, terminal font and spacing
- **Connection** — current server/addresses, optimize/share/disconnect, debug

## Components
- Address field: `ws://host:port` or `wss://host:port`
- Token field
- Address history: cached recent connections with token, quick switching
- Server info: hostname, machine ID, address
- Language selector: EN / 中文 (pill buttons)
- Theme selector: Auto / Light / Dark (pill buttons)
- Desktop interface scale (60%–180%, persisted to localStorage `tmux_ui_zoom`; Cmd/Ctrl `+`, `-`, and `0` use the same value)
- Terminal font size control (+/−), independent from interface scale
- Terminal font family (editable list of common locally-installed fonts; accepts another family typed by the user; only a valid local font is applied and persisted to localStorage `tmux_font`; empty = system default)
- Terminal line spacing (0.60–1.60, persisted to localStorage `tmux_line_height`; applies live to every normal, split, and Team terminal)
- Line-spacing slider uses the themed surface track and accent thumb, never the browser's native white track
- Debug toggle (On/Off); the floating log panel is draggable from its header on touch and desktop pointer input, clamps to the visible viewport, and remembers its last position
- Disconnect button

## Interactions
- Enter address + token → connect
- Tap history entry → auto-fill and connect
- Switch language → immediate UI text update, persisted to localStorage
- Switch theme → immediate CSS variable transition
- Switch settings tab → remember the last tab in localStorage and restore it on the next open
- A previously saved Terminal tab migrates to Appearance
- Adjust interface scale → updates the complete Tauri desktop WebView; terminal grid refits after the native zoom settles
- Adjust terminal font size → updates Terminal view without changing the surrounding UI
- Choose or enter a font → validate it against fonts available on the current device, then apply and remember it; invalid input leaves the active font unchanged
- Tap disconnect → `doDisconnect()`

## API Calls
- `auth(token)` or encrypted auth (client_nonce + proof) — authenticate on connect (returns machine_id, hostname)
- `set_socket(socket)` — set tmux socket path at runtime

## State Management
- Connection state (disconnected/connecting/connected)
- Address history (localStorage)
- Locale / language preference (localStorage `tmux_locale`)
- Interface scale, terminal font size, theme, terminal font name preference (localStorage)
- Server info (hostname, machine_id)
- State restore on reload (page, session, view mode)

## Edge Cases
- Auto ws/wss detection based on address format
- Multi-address reconnect: server machine_id tracks alternate addresses, auto-failover on disconnect
- Tauri desktop auto-fills config from local `~/.config/tmux-mobile/config.toml`
- Language auto-detected from `navigator.language` on first visit (zh → Chinese, else English)
