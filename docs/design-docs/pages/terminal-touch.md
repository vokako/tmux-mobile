# Terminal Touch Handling

## Context
xterm.js v6 (VS Code-based) only supports mouse events for its scrollbar. Mobile devices need touch scrolling, scrollbar drag, and word selection.

## Decision
All touch interactions are custom-implemented on top of xterm.js. Keyboard is controlled via double-tap (not single-tap) to prevent accidental popups during scroll/selection.

## Gesture Architecture
See [Terminal Gestures Design Doc](terminal-gestures.md) for the full gesture priority table, state machine, and keyboard control flow.

## How It Works
- Custom touch handlers for scroll, scrollbar drag, long-press word selection
- Content updates paused during touch (`touchScrolling` flag), caught up via `endTouchScroll()` on release
- iOS-like momentum physics with velocity smoothing
- **Double-tap to open keyboard** — single tap does nothing on terminal area
- Keyboard toggle button as explicit open/close alternative
- Keyboard controlled via `inputmode` attribute on xterm's hidden textarea + `kbLocked` flag
- `visualViewport` API for mobile browser, native `OnGlobalLayoutListener` for Android WebView

## Alternatives Considered
- **Native xterm.js touch**: Not available in v6 — VS Code scrollbar is mouse-only
- **CSS overflow scroll on ANSI→HTML**: Works but lacks xterm.js features (cursor, colors, virtual scrolling)
- **Single-tap to open keyboard**: Caused accidental keyboard popups during scroll, selection, and general terminal interaction. Changed to double-tap.
- **pointerdown for keyboard unlock**: Fired before touch gesture classification was complete, causing race conditions with delayed `endTouchScroll` timers. Replaced with touchend-based double-tap detection.

## Trade-offs
- Complex custom code to maintain
- Must pause/resume content updates around touch interactions
- Platform-specific keyboard handling (browser vs Android WebView)
- Double-tap is slightly less discoverable than single-tap (mitigated by keyboard toggle button)

## Lessons Learned
- xterm.js DA responses (`\x1b[?62;22c`) must be filtered before forwarding to tmux
- **endTouchScroll must NOT manipulate kbLocked** — it's called via setTimeout (200-500ms delay) and can fire after a pointerdown/double-tap unlock, overriding the user's intent. Keyboard state is managed only by: double-tap, toggle button, blur timer, keyboard-shift event.
- **Android keyboard control** uses three layers:
  1. `inputmode="none"` on xterm's textarea by default — browser-level hint to suppress keyboard.
  2. `kbLocked` flag + `focus` event guard — immediately blurs textarea if focused while locked.
  3. `keyboard-shift kbHeight=0` listener — catches keyboard dismissed by Android back/system.
  - **Unlock flow**: double-tap terminal or keyboard toggle → `unlockKeyboard()` (clears timers, sets `kbLocked=false`, `inputmode=text`, focuses textarea).
  - **Lock flow**: `blur` event schedules delayed lock (150ms); `keyboard-shift kbHeight=0` locks immediately; pane switch resets to locked.
- **Svelte 5 registers `touchstart`/`touchmove` as passive** — `e.preventDefault()` in `ontouchstart` is silently ignored. The `nonPassiveShortcuts` Svelte action registers non-passive handlers.
- **Mobile auto-pair textarea accumulation** — Mobile keyboards auto-pair quotes/brackets. Fix: force-clear textarea after each `onData`, skip paste (detected via `paste` event flag, NOT `data.length`).
- **Tab swipe vs child gestures** — App-level swipe suppressed when `e.defaultPrevented` or vertical movement > 10px.
- **Nav buttons must have `tabindex="-1"`** — otherwise Android moves focus to nav buttons when textarea blurs, interfering with keyboard control.
- **Android `OnGlobalLayoutListener` stale heights** — can fire keyboard-open events when no text input is focused. Guard with `activeElement` check.
