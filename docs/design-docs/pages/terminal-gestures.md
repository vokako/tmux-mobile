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
| 6 | Single-tap | clean tap (no scroll / scrollbar / selection) | — (opens keyboard; never closes it) |
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
  │   └─ touchend (no scroll, no long-press, not on selection) → open keyboard if locked
  │       (single tap never closes — closing is only via the toggle button)
  └─ (App level) horizontal swipe not consumed → TAB_SWITCH
```

## Keyboard Control

### States
- `kbLocked = true` + `inputmode="none"` → keyboard cannot show
- `kbLocked = false` + `inputmode="text"` → keyboard allowed

### Transitions
| From | Event | To | Action |
|------|-------|----|--------|
| locked | single tap on terminal | unlocked | set inputmode=text, focus textarea, start 1.5s grace |
| locked | keyboard toggle button | unlocked | set inputmode=text, focus textarea, start 1.5s grace |
| unlocked | single tap on terminal | unlocked | no-op (intentionally does not close) |
| unlocked | textarea blur (150ms timer) | locked (or retry focus if in grace) | grace → re-focus; post-grace → inputmode=none |
| unlocked | keyboard-shift kbH=0 (was >0, post-grace) | locked | set inputmode=none, blur textarea |
| unlocked | keyboard toggle button | locked | blur textarea |
| unlocked | pane switch | locked | reset state |

### Key Rules
1. **Single tap opens the keyboard; it never closes it.** Closing is only via the explicit keyboard toggle button. This matches user intent: a stray tap while reading the terminal should not hide the keyboard the user is actively typing into.
2. **Tap is guarded against scroll, scrollbar drag, and active selection** — those branches return early so an accidental tap during those gestures cannot trigger unlock.
3. **Keyboard toggle button** — always works as explicit open/close.
4. **endTouchScroll** — does NOT change kbLocked (was causing race conditions with delayed timers).
5. **keyboard-shift kbH=0** — locks on the **open→close falling edge** only (`lastKbHeight > 0`) and only when past the unlock grace window. A bare kbH=0 event (e.g., Android pad where IME never rose) MUST NOT re-lock, otherwise a freshly-pressed keyboard toggle would be immediately cancelled.
6. **Nav buttons have tabindex=-1** — prevents focus stealing from textarea.

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
- Double-tap to open keyboard proved unreliable on Android (two-tap latency + occasional stray selection on the first tap) → reverted to single tap. Accidental popups during scroll/selection are prevented by the `didScroll / touchScrolling / onScrollbar / isSelecting` guards in `onTermTapEnd` instead of by requiring a second tap. Closing is never tied to tap so a stray tap while typing cannot hide the keyboard.
