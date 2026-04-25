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
| 6 | Tap | clean tap (no scroll / scrollbar / selection) | — (does nothing; keyboard is opened via the toggle button only) |
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
  │   └─ touchend (no scroll, no long-press, not on selection) → IDLE (no action)
  │       (keyboard is opened only via the toggle button — tapping the terminal never opens or closes it)
  └─ (App level) horizontal swipe not consumed → TAB_SWITCH
```

## Keyboard Control

### States
- `kbLocked = true` + `inputmode="none"` → keyboard cannot show
- `kbLocked = false` + `inputmode="text"` → keyboard allowed

### Transitions
| From | Event | To | Action |
|------|-------|----|--------|
| locked | keyboard toggle button | unlocked | set inputmode=text, focus textarea, start 1.5s grace |
| locked | tap on terminal | locked | no-op (intentionally does not open) |
| unlocked | tap on terminal | unlocked | no-op (intentionally does not close) |
| unlocked | textarea blur (150ms timer) | locked (or retry focus if in grace) | grace → re-focus; post-grace → inputmode=none |
| unlocked | keyboard-shift kbH=0 (was >0, post-grace) | locked | set inputmode=none, blur textarea |
| unlocked | keyboard toggle button | locked | blur textarea |
| unlocked | pane switch | locked | reset state |

### Key Rules
1. **Keyboard toggle button is the only way to open the keyboard.** Tapping the terminal never opens or closes it — stray taps while reading scrollback, adjusting selection handles, or aiming at the window switcher all failed to trigger the keyboard unintentionally.
2. **Tap on terminal is reserved for the non-keyboard paths**: tap-on-selection copies + clears; all other taps are intentionally no-ops.
3. **endTouchScroll** — does NOT change kbLocked (was causing race conditions with delayed timers).
4. **keyboard-shift kbH=0** — locks on the **open→close falling edge** only (`lastKbHeight > 0`) and only when past the unlock grace window. A bare kbH=0 event (e.g., Android pad where IME never rose) MUST NOT re-lock, otherwise a freshly-pressed keyboard toggle would be immediately cancelled.
5. **Nav buttons have tabindex=-1** — prevents focus stealing from textarea.

## Tab Swipe Suppression (App level)
The App-level left/right swipe to switch tabs is suppressed when:
- `e.defaultPrevented` on any touchmove (terminal scroll, selection drag, scrollbar drag all call `preventDefault`)
- Vertical movement > 10px (any vertical gesture)

## Auto-pair Textarea Clearing
Mobile keyboards auto-pair quotes/brackets (`""`, `()`, `[]`). This breaks xterm.js textarea clearing. Fix: force-clear textarea after each `onData` on mobile, EXCEPT:
- during paste (detected via `paste` event flag, NOT `data.length`)
- during an active IME composition (detected via `compositionstart` / `compositionend`). Clearing the textarea mid-composition drops the buffered pinyin/kana and prevents CJK input from completing correctly. The rAF callback re-checks the composing flag in case a composition started between scheduling and running.

## Lessons Learned
- `endTouchScroll` via setTimeout can fire after `pointerdown` unlock → removed kbLocked manipulation from endTouchScroll
- `data.length <= 1` misclassifies auto-paired input as paste → use paste event flag
- Android `OnGlobalLayoutListener` can fire stale keyboard heights → guard with activeElement check
- Nav buttons without `tabindex=-1` steal focus from textarea on blur
- Double-tap to open keyboard proved unreliable on Android (two-tap latency + occasional stray selection on the first tap) → reverted to single tap. Then single-tap to open was also removed because strays while reading scrollback or adjusting selection handles kept flipping the keyboard unexpectedly. Current model: **the keyboard toggle button is the only way to open it.** Tapping the terminal is reserved for tap-on-selection (copy+clear) and long-press paths.
