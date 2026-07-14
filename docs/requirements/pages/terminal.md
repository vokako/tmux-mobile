# Terminal Page

## Purpose
Primary view for interacting with tmux panes. Renders terminal output with ANSI colors, provides touch-optimized scrolling and input for mobile devices.

## Components
- Expanded window-switcher chips and every mobile shortcut-key button share a 24px control height and pill geometry.
- xterm.js v6 terminal emulator with theme-aware color schemes (light/dark)
- Shortcut buttons: Esc, ^C, ^D, Tab, arrows — with long-press repeat
- Keyboard toggle button (show/hide on-screen keyboard)
- Collapsible window switcher:
  - **Collapsed**: a single chip in the top-right corner (uses the shared
    AgentChip component — same visual language as every other chip in the
    app). Shows the current window's agent icon or command name plus a
    small chevron-left indicator (pointing toward where the bar would
    expand from). Positioned absolute so it does not steal a row from
    the terminal viewport.
  - **Expanded**: horizontal tab bar pinned to the top of the Terminal
    view. Holds chips for (1) every window in the current session,
    (2) a `+ new window` button, (3) a vertical separator, (4) up to 5
    most-recently-opened AI sessions (Kiro/Claude/OpenClaw) as chips.
    Scrolls horizontally. A chevron-right button on the right collapses
    the bar back into the single chip (toward where the collapsed chip
    will live).
  - All chips in both states are the same `AgentChip` component — same
    height, same padding, same font. The switcher has one size in its
    expanded state and a smaller footprint in its collapsed state, not
    two different visual languages.
  - State (collapsed vs expanded) is persisted in localStorage.
- Floating buttons: scroll-to-bottom (frosted glass style)
- Floating buttons: scroll-to-bottom (frosted glass style)
- Status bar: session:pane and running command

## Interactions (Mobile)
- **Double-tap terminal** → open keyboard (single tap does nothing)
- **Keyboard toggle button** → explicit open/close keyboard
- **Vertical swipe** → scroll terminal content (momentum physics)
- **Long-press (500ms)** → select word at touch point, drag to extend
- **Tap on selection** → copy to clipboard (tap outside clears)
- **Swipe right edge** → scrollbar drag
- **Tap shortcut button** → sends key sequence; long-press repeats
- **Horizontal swipe (App level)** → switch tabs (lowest priority, suppressed by all above)

## API Calls
- `subscribe(target)` — start streaming pane output (200ms polling, includes cursor position)
- `unsubscribe(target)` — stop streaming
- `send_keys(target, keys, literal)` — send keystrokes
- `send_command(target, command)` — send text + Enter
- `capture_pane(target, lines)` — initial content load
- `pane_command(target)` — get running command for status bar
- `list_panes(session)` — populate window switcher (current session)
- `list_sessions` — populate cross-session AI chips (expanded switcher only)
- `list_panes(other_session)` — per AI-candidate session, to find the primary
  pane for chip targets (expanded switcher only)
- `resize_pane(target, cols, rows)` — resize pane to match client viewport (auto-restores on disconnect)

## State Management
- Active pane target (session:window.pane)
- Terminal content (managed by xterm.js)
- Window switcher collapsed/expanded (persisted)
- `touchScrolling` flag — pauses content updates during touch
- `kbLocked` flag + `inputmode` attribute — keyboard control (see design doc)
- `isSelecting` — word selection mode active
- Font size (from settings)

## Edge Cases
- Content updates paused during touch interaction, caught up via `endTouchScroll()`
- `endTouchScroll` does NOT manipulate `kbLocked` — prevents race conditions with delayed timers
- xterm.js Device Attribute responses (`\x1b[?62;22c`) filtered before forwarding to tmux
- Mobile keyboard: double-tap to open. Three-layer control — `inputmode="none"` (browser hint), `kbLocked` focus guard (immediate blur), `keyboard-shift kbHeight=0` listener (catches system keyboard dismiss). Only `unlockKeyboard()`, blur timer, keyboard-shift, and pane switch may change `kbLocked`.
- Nav buttons have `tabindex="-1"` to prevent focus stealing from textarea
- All shortcut buttons use non-passive `touchstart: preventDefault()` to block synthetic `mousedown` focus stealing
- Uses `visualViewport` API for mobile browser, native `OnGlobalLayoutListener` for Android WebView
- Android stale keyboard height events guarded by `activeElement` check
- Mobile auto-pair input (quotes `""`, brackets `()[]`): xterm's hidden textarea is force-cleared via `requestAnimationFrame` after each keyboard input. Paste excluded via `paste` event flag (NOT `data.length`).
- Tab swipe suppressed when any child gesture is active (`defaultPrevented` or vertical movement > 10px)
