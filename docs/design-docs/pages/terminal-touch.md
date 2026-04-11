# Terminal Touch Handling

## Context
xterm.js v6 (VS Code-based) only supports mouse events for its scrollbar. Mobile devices need touch scrolling, scrollbar drag, and word selection.

## Decision
All touch interactions are custom-implemented on top of xterm.js.

## How It Works
- Custom touch handlers for scroll, scrollbar drag, long-press word selection
- Content updates paused during touch (`touchScrolling` flag), caught up via `endTouchScroll()` on release
- iOS-like momentum physics with velocity smoothing
- Keyboard controlled via `inputmode` attribute on xterm's hidden textarea: default `"none"` prevents keyboard; set to `"text"` only on terminal tap or keyboard toggle
- `visualViewport` API for mobile browser, native `OnGlobalLayoutListener` for Android WebView

## Alternatives Considered
- **Native xterm.js touch**: Not available in v6 — VS Code scrollbar is mouse-only
- **CSS overflow scroll on ANSI→HTML**: Works but lacks xterm.js features (cursor, colors, virtual scrolling)

## Trade-offs
- Complex custom code to maintain
- Must pause/resume content updates around touch interactions
- Platform-specific keyboard handling (browser vs Android WebView)

## Lessons Learned
- xterm.js DA responses (`\x1b[?62;22c`) must be filtered before forwarding to tmux
- `maxContainerH` is essential to prevent terminal resize flicker on keyboard open/close
- **Android keyboard control** uses three layers:
  1. `inputmode="none"` on xterm's textarea by default — browser-level hint to suppress keyboard.
  2. `kbLocked` flag + `focus` event guard — immediately blurs textarea if focused while locked.
  3. `keyboard-shift kbHeight=0` listener — catches keyboard dismissed by Android back/system (textarea stays focused but keyboard closes; without this, any subsequent touch re-shows IME).
  - **Unlock flow**: terminal area `pointerdown` capture (only when locked) or keyboard toggle sets `kbLocked=false` + `inputmode="text"`. The toggle also calls `ta.focus()`.
  - **Lock flow**: `blur` event schedules delayed lock (150ms, canceled on refocus for blur→refocus cycles); `keyboard-shift kbHeight=0` locks + blurs immediately; `endTouchScroll` locks after scroll.
  - **Shortcut button press visual**: `nonPassiveShortcuts` action adds `.pressed` CSS class on `touchstart`, removes on `touchend`/`touchcancel` (`:active` doesn't fire because `preventDefault()` is called).
- **Svelte 5 registers `touchstart`/`touchmove` as passive** — `e.preventDefault()` in `ontouchstart` is silently ignored. The `nonPassiveShortcuts` Svelte action registers a non-passive `touchstart` handler via `addEventListener` that calls `preventDefault()` on ALL shortcut buttons (including kb-toggle, to prevent synthetic `mousedown` from stealing focus after `ta.focus()`).
