# Terminal Gesture & Keyboard Architecture

## Context
xterm.js v6 has no mobile touch support. All touch interactions are custom. The keyboard must be explicitly controlled because Android IME aggressively re-shows keyboard on any textarea focus. Multiple gesture types must coexist without interference.

## Gesture Priority (highest → lowest)

| Priority | Gesture | Trigger | Blocks |
|----------|---------|---------|--------|
| 1 | Selection drag | touchmove during `isSelecting` | scroll, tab swipe |
| 2 | Scrollbar drag | touchstart on right 30px edge | scroll, tab swipe |
| 3 | Vertical scroll | touchmove vertical > 1 line | long-press, tab swipe |
| 4 | Long-press select | 500ms hold without scroll | — |
| 5 | Tap on selection | touchstart while `isSelecting` | keyboard open |
| 6 | Double-tap | two taps within 300ms | — (opens keyboard) |
| 7 | Tab swipe (App) | horizontal swipe > 120px | — (lowest priority) |

## Gesture State Machine

```
IDLE
  ├─ touchstart on scrollbar edge → SCROLLBAR_DRAG
  ├─ touchstart while isSelecting → TAP_ON_SELECTION → copy/clear → IDLE
  ├─ touchstart (normal) → TOUCH_DOWN
  │   ├─ touchmove vertical > 1 line → SCROLLING (cancel long-press timer)
  │   │   └─ touchend → MOMENTUM → IDLE (after coast)
  │   ├─ 500ms hold → SELECTING (long-press)
  │   │   ├─ touchmove → SELECTION_DRAG
  │   │   └─ touchend → SELECTED (wait for tap to copy)
  │   └─ touchend (no scroll, no long-press) → check double-tap
  │       ├─ second tap within 300ms → open keyboard
  │       └─ single tap → IDLE (no action)
  └─ (App level) horizontal swipe not consumed → TAB_SWITCH
```

## Keyboard Control

### States
- `kbLocked = true` + `inputmode="none"` → keyboard cannot show
- `kbLocked = false` + `inputmode="text"` → keyboard allowed

### Transitions
| From | Event | To | Action |
|------|-------|----|--------|
| locked | double-tap terminal | unlocked | set inputmode=text, focus textarea |
| locked | keyboard toggle button | unlocked | set inputmode=text, focus textarea |
| unlocked | textarea blur (150ms timer) | locked | set inputmode=none |
| unlocked | keyboard-shift kbH=0 | locked | set inputmode=none, blur textarea |
| unlocked | keyboard toggle button | locked | blur textarea |
| unlocked | pane switch | locked | reset state |

### Key Rules
1. **Double-tap to open keyboard** — single tap does nothing (prevents accidental keyboard popup during scroll/selection)
2. **Keyboard toggle button** — always works as explicit open/close
3. **endTouchScroll** — does NOT change kbLocked (was causing race conditions with delayed timers)
4. **keyboard-shift kbH=0** — always locks (catches Android system keyboard dismiss)
5. **Nav buttons have tabindex=-1** — prevents focus stealing from textarea

## Tab Swipe Suppression (App level)
The App-level left/right swipe to switch tabs is suppressed when:
- `e.defaultPrevented` on any touchmove (terminal scroll, selection drag, scrollbar drag all call `preventDefault`)
- Vertical movement > 10px (any vertical gesture)

## Auto-pair Textarea Clearing
Mobile keyboards auto-pair quotes/brackets (`""`, `()`, `[]`). This breaks xterm.js textarea clearing. Fix: force-clear textarea after each `onData` on mobile, EXCEPT during paste (detected via `paste` event flag, NOT `data.length`).

## Lessons Learned
- `endTouchScroll` via setTimeout can fire after `pointerdown` unlock → removed kbLocked manipulation from endTouchScroll
- `data.length <= 1` misclassifies auto-paired input as paste → use paste event flag
- Android `OnGlobalLayoutListener` can fire stale keyboard heights → guard with activeElement check
- Nav buttons without `tabindex=-1` steal focus from textarea on blur
- Single-tap to open keyboard causes accidental popups during scroll → changed to double-tap
