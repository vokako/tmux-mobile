# Terminal Page

## Purpose
Primary view for interacting with tmux panes. Renders terminal output with ANSI colors, provides touch-optimized scrolling and input for mobile devices.

## Components

- Window chips and the all-session pane picker show unread agent attention dots without changing chip height; the left session chip signals only unread notifications in other sessions; the picker omits a summary dot for the current session because its window chips already carry precise dots; opening that window marks its server-persisted notification read across connected clients.
- Expanded window-switcher chips and every mobile shortcut-key button share a 24px control height and pill geometry. Their text and icons use the same 1px optical baseline correction so visible glyphs are vertically centered. Shortcut rows add no vertical padding, preserving terminal viewport space.
- xterm.js v6 terminal emulator with theme-aware color schemes (light/dark)
- Shortcut buttons: Esc, Ctrl, ^C, Tab, arrows — with long-press repeat for repeatable keys
- Keyboard toggle button (show/hide on-screen keyboard)
- Collapsible window switcher:
  - **Collapsed**: a single chip in the top-right corner (uses the shared
    AgentChip component — same visual language as every other chip in the
    app). Shows the current window's agent icon or command name plus a
    small chevron-left indicator (pointing toward where the bar would
    expand from). Positioned absolute so it does not steal a row from
    the terminal viewport.
  - **Expanded**: horizontal tab bar pinned to the top of the Terminal
    view. The first chip is the current session name, rendered with the same
    text-only active style as the Sessions page chip strip; tapping it opens
    the all-session pane picker. The remaining scrollable chips contain only
    windows from the current session plus the `+ new window` button. A
    chevron-right button on the right collapses the bar back into the single
    chip (toward where the collapsed chip will live).
  - All chips in both states are the same `AgentChip` component — same
    height, same padding, same font. The switcher has one size in its
    expanded state and a smaller footprint in its collapsed state, not
    two different visual languages.
  - State (collapsed vs expanded) is persisted in localStorage.
- Floating scroll-to-bottom button (frosted glass style) with an accent state and red dot when deferred output is waiting
- Status bar: session:pane and running command

## Interactions (Mobile)
- **Double-tap terminal** → open keyboard (single tap does nothing)
- **Keyboard toggle button** → explicit open/close keyboard
- **Vertical swipe** → scroll terminal content (momentum physics)
- **Long-press (500ms)** → select a word, then drag either endpoint handle to extend or reverse the selection
- **Selection toolbar** → explicitly copy the selected text; tapping outside or switching panes cancels the selection
- **Swipe right edge** → scrollbar drag
- **Tap shortcut button** → sends key sequence; long-press repeats
- **Tap Ctrl** → arms a one-shot modifier; the next letter typed on the system keyboard sends Ctrl+letter, then Ctrl releases; tapping Ctrl again cancels it
- **Horizontal swipe (App level)** → switch tabs (lowest priority, suppressed by all above)

## Interactions (Desktop)
- App-configured shortcuts are consumed before terminal input
- Every unassigned hardware Ctrl / Option combination is encoded as terminal input and sent to tmux, including Ctrl+X, Ctrl+F, letters, punctuation, navigation keys, and F1–F12; touch-capable desktop browsers and devices with attached keyboards follow the same path
- Cmd combinations remain owned by the macOS app/browser unless explicitly configured as an app shortcut

## API Calls
- `subscribe(target)` — start streaming pane output (200ms polling, includes cursor position and command changes)
- `unsubscribe(target)` — stop streaming
- `send_keys(target, keys, literal)` — send keystrokes
- `send_command(target, command)` — send text + Enter
- `capture_pane(target, lines)` — initial content load
- `list_panes(session)` — populate window switcher (current session)
- `resize_pane(target, cols, rows)` — resize pane to match client viewport (auto-restores on disconnect)

## State Management
- Active pane target (session:window.pane)
- Terminal content (managed by xterm.js)
- Window switcher collapsed/expanded (persisted)
- `touchScrolling` flag — pauses content updates during touch
- `kbLocked` flag + `inputmode` attribute — keyboard control (see design doc)
- Persistent selection object `{anchor, head}` in inclusive buffer coordinates, plus derived handle/toolbar geometry
- Terminal font size, family, and line spacing (from settings); desktop interface scale is independent

## Edge Cases
- Content updates paused during touch interaction, caught up via `endTouchScroll()`
- `endTouchScroll` does NOT manipulate `kbLocked` — prevents race conditions with delayed timers
- Complete xterm.js Device Attribute response chunks (`\x1b[?62;22c`) are filtered before forwarding to tmux; split-chunk leakage remains tracked in `docs/unresolved.md`
- Mobile keyboard: double-tap to open. Three-layer control — `inputmode="none"` (browser hint), `kbLocked` focus guard (immediate blur), `keyboard-shift kbHeight=0` listener (catches system keyboard dismiss). Only `unlockKeyboard()`, blur timer, keyboard-shift, and pane switch may change `kbLocked`.
- Nav buttons have `tabindex="-1"` to prevent focus stealing from textarea
- All shortcut buttons use non-passive `touchstart: preventDefault()` to block synthetic `mousedown` focus stealing
- Uses `visualViewport` API for mobile browser, native `OnGlobalLayoutListener` for Android WebView
- Android stale keyboard height events guarded by `activeElement` check
- Mobile auto-pair input (quotes `""`, brackets `()[]`): xterm's hidden textarea is force-cleared via `requestAnimationFrame` after each keyboard input. Paste excluded via `paste` event flag (NOT `data.length`).
- Tab swipe suppressed when any child gesture is active (`defaultPrevented` or vertical movement > 10px)
