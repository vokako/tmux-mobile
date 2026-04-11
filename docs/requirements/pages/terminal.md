# Terminal Page

## Purpose
Primary view for interacting with tmux panes. Renders terminal output with ANSI colors, provides touch-optimized scrolling and input for mobile devices.

## Components
- xterm.js v6 terminal emulator with theme-aware color schemes (light/dark)
- Input bar with text field and send button
- Shortcut buttons: Esc, ^C, ^D, Tab, arrows — with long-press repeat
- Keyboard toggle button (show/hide on-screen keyboard)
- Collapsible window switcher — AI agent icons (Kiro/Claude) or command name, persists state
- Floating buttons: scroll-to-bottom, window switcher (frosted glass style)
- Status bar: session:pane and running command

## Interactions
- Type in input bar → `send_keys` to tmux pane
- Tap shortcut button → sends corresponding key sequence
- Long-press shortcut → repeats key at interval
- Touch scroll → custom touch handler (xterm.js v6 has no native touch scroll)
- Tap window in switcher → switches active pane via `subscribe`
- Tap scroll-to-bottom → scrolls terminal to end

## API Calls
- `subscribe(target)` — start streaming pane output (200ms polling, includes cursor position)
- `unsubscribe(target)` — stop streaming
- `send_keys(target, keys, literal)` — send keystrokes
- `send_command(target, command)` — send text + Enter
- `capture_pane(target, lines)` — initial content load
- `pane_command(target)` — get running command for status bar
- `list_panes(session)` — populate window switcher
- `resize_pane(target, cols, rows)` — resize pane to match client viewport (auto-restores on disconnect)

## State Management
- Active pane target (session:window.pane)
- Terminal content (managed by xterm.js)
- Window switcher collapsed/expanded (persisted)
- Touch scrolling flag (`touchScrolling`) — pauses content updates during touch
- Font size (from settings)
- Keyboard lock state (`kbLocked` flag + `inputmode` attribute on xterm textarea)

## Edge Cases
- Content updates paused during touch interaction, caught up via `endTouchScroll()`
- xterm.js Device Attribute responses (`\x1b[?62;22c`) filtered before forwarding to tmux
- Mobile keyboard: three-layer control — `inputmode="none"` (browser hint), `kbLocked` focus guard (immediate blur), `keyboard-shift kbHeight=0` listener (catches system keyboard dismiss without blur). Unlocked only by terminal tap (`pointerdown` capture, skipped if already unlocked) or keyboard toggle. All shortcut buttons use non-passive `touchstart: preventDefault()` to block synthetic `mousedown` focus stealing.
- Uses `visualViewport` API for mobile browser, native `OnGlobalLayoutListener` for Android WebView
