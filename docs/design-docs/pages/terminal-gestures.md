# Terminal Gesture & Keyboard Architecture

## Context
xterm.js v6 has no mobile touch support. All touch interactions are custom. The keyboard must be explicitly controlled because Android IME aggressively re-shows keyboard on any textarea focus. Multiple gesture types must coexist without interference.

## First Principles
1. **A selection is an object, not a transient state.** Once made, it persists across scrolling, content updates, and finger lifts — until the user explicitly copies it (toolbar) or cancels it (tap outside, new long-press, pane switch, app background).
2. **Both endpoints are independently draggable.** Touch UIs since iPhoneOS 3.0 use draggable handles at each end of the selection. We do the same — there is no "one-shot select" path.
3. **Copy is explicit, never implicit.** A floating toolbar above the selection has a "Copy" button. There is no tap-to-copy heuristic, because heuristics produce clipboard pollution from stray taps.
4. **The selection's coordinate space is the buffer, not the viewport.** Endpoints are stored as absolute buffer rows. Scrolling moves the on-screen handle position, not the selection.

## Touch Modes
The touch handler is a single state machine driven by `touchMode`:

| Mode | How entered | What it does |
|------|-------------|--------------|
| `idle` | default | nothing |
| `down` | touchstart on terminal body (no selection-handle, no scrollbar) | starts long-press timer |
| `scrollbar` | touchstart on right 30px edge | proportional scroll-by-drag |
| `scroll` | `down` → vertical move > 1 line | inertial content scroll |
| `longpress-select` | `down` → 500ms hold | word-select; touchmove extends head |
| `handle-drag` | touchstart on a selection handle's 22px-radius hit zone | moves that endpoint |

## Touchstart Hit-Test Order
1. **Toolbar button**: bow out — the button has its own pointer handler.
2. **Selection handle**: enter `handle-drag` if a handle exists and is within 22px of the touch.
3. **Scrollbar edge** (right 30px): `scrollbar`.
4. **Anywhere else**: `down`. Long-press timer arms.

## Selection Lifecycle

### Creation
- **Long-press (500ms)** anywhere outside an existing selection → word-select at that cell. Same touch can extend the selection by dragging the head.
- **xterm-native** (mouse double/triple-click on desktop, programmatic Cmd+A) is adopted via `term.onSelectionChange`. The handler converts xterm's *exclusive* end to our *inclusive* end and pins content updates.

### Persistence
While a selection exists:
- Content updates are **paused** (`touchScrolling = true`) so incoming tmux output doesn't wipe the visible selection.
- Scrolling is allowed; on each `term.onScroll` we recompute pixel positions of handles + toolbar. If the selection scrolls fully out of view, the toolbar hides; handles hide independently per side.
- The keyboard cannot open via tap (it only opens via the toolbar button — separate concern).
- Resize re-applies the selection to xterm via `term.select()` so the visual highlight tracks the new geometry.

### Endpoint Adjustment
Drag a handle within its hit zone. At **grab time** (`beginEndpointDrag`) the selection is rewritten so the grabbed endpoint becomes `head` and the stationary one becomes `anchor`; every subsequent touchmove rewrites *head only*. Dragging past the other endpoint flips the selection's direction naturally — the anchor physically cannot move. (Earlier code addressed endpoints by geometric role per-move; after a crossover the roles swapped under a stale `dragHandle`, so the next move perturbed the far endpoint and both ends jumped.)

Drag mapping details (all in `applyHandleDragAt` / grab-offset capture):
- **Grab-offset compensation in both axes**: at grab, record finger − endpoint-cell-centre delta (X and Y); subtract it on every move. First frame maps to exactly the cell the endpoint is already on — zero snap. No artificial "lift" above the finger.
- **Horizontal edge snap**: within `max(10px, 0.6·cellW)` of the container's left edge → col 0; symmetric on the right (excluding the scrollbar zone) → last col. Matches OS selection where dragging past the text edge reaches the line boundary.
- **Vertical edge auto-scroll**: holding a drag within 36 px of the top/bottom edge scrolls the viewport (rAF loop, speed ramps 0.25→2 rows/frame with proximity) and re-maps the endpoint each scrolled line, so selections extend beyond the visible screen — same as native text views.

### Termination
- **Toolbar Copy**: copy text, then clear.
- **Tap outside the selection** (clean tap on `down`, no scroll): clear.
- **Long-press outside the selection**: clear, then create new selection.
- **Pane switch / app background / xterm-native clear**: clear.

## Coordinate Model
- `selection = { anchor: {row, col}, head: {row, col} }`
- `row` is **absolute buffer row** (not viewport-relative). Stable across scrolling.
- `col` is 0..cols-1, **inclusive** on both endpoints.
- `selStart(s)` / `selEnd(s)` derive the geometric (top-left, bottom-right) ordering from `anchor`/`head`.

The xterm.js API is converted at the boundary:
- `applySelectionToXterm()` calls `term.select(start.col, start.row, length)` where `length` spans the inclusive range.
- `onSelectionChange` reads `term.getSelectionPosition()` whose `pos.end.x` is exclusive, and converts to inclusive (`max(0, pos.end.x - 1)`).

## Handle UI
- Visual: 14px filled circle in `var(--accent)`, stem extending 14px up to the selection edge.
- Hit zone: 44px square centered on the visual anchor (matches Apple/Google touch-target guidelines), positioned via `left/top` on a 44×44 wrapper with negative margins.
- Position: leading handle anchored at the bottom-left **outside** the start cell; trailing handle at the bottom-right **outside** the end cell. Stem points up into the selection. This matches both iOS and Material conventions.

## Toolbar UI
- Single "Copy" button (one job, one button).
- Default: above the selection's first row, horizontally centered between start and end (or roughly above the start cell when the selection spans multiple rows).
- If too close to the top (< 8px), flips to below the selection's last row.
- X is clamped to `[48, container_width - 48]` so the toolbar never escapes its container.
- `pointerdown` handler stops propagation and calls `copySelection()` — the touchstart hit-test on the underlying `termEl` would otherwise treat it as a tap and try to cancel the selection.

## Keyboard Control (unchanged from earlier design)

### States
- `kbLocked = true` + `inputmode="none"` → keyboard cannot show
- `kbLocked = false` + `inputmode="text"` → keyboard allowed

### Transitions
| From | Event | To | Action |
|------|-------|----|--------|
| locked | keyboard toggle button | unlocked | inputmode=text, focus textarea, 1.5s grace |
| locked | tap on terminal | locked | no-op |
| unlocked | tap on terminal | unlocked | no-op |
| unlocked | textarea blur (150ms timer) | locked (or retry focus if in grace) | grace → re-focus; post-grace → inputmode=none |
| unlocked | keyboard-shift kbH=0 (was >0, post-grace) | locked | inputmode=none, blur |
| unlocked | keyboard toggle button | locked | blur |
| unlocked | pane switch | locked | reset |

### Key Rules
1. **Keyboard toggle button is the only way to open the keyboard.** Tapping the terminal never opens or closes it.
2. **`endTouchScroll` does NOT change `kbLocked`** (was causing race conditions with delayed timers).
3. **`endTouchScroll` is a no-op while a selection exists** — releasing `touchScrolling=false` would let writeToXterm clear+rewrite, wiping xterm's native highlight.
4. **keyboard-shift kbH=0** locks only on the open→close falling edge.
5. **Nav buttons have tabindex=-1** — prevents focus stealing.

## Tab Swipe Suppression (App level)
The App-level horizontal tab swipe is suppressed when:
- `e.defaultPrevented` on touchmove (terminal scroll, selection drag, handle drag, scrollbar drag all call `preventDefault`)
- Vertical movement > 10px

## Auto-pair Textarea Clearing
Mobile keyboards auto-pair quotes/brackets (`""`, `()`, `[]`). Force-clear textarea after each `onData` on mobile, EXCEPT during paste (detected via paste event flag, NOT `data.length`) and during active IME composition.

## Lessons Learned
- `endTouchScroll` via setTimeout can fire after `pointerdown` unlock → removed kbLocked manipulation.
- `data.length <= 1` misclassifies auto-paired input as paste → use paste event flag.
- Android `OnGlobalLayoutListener` can fire stale keyboard heights → guard with activeElement check.
- "Tap inside selection to copy" looked clever but was ambiguous — users couldn't tell whether the selection was "live", and a stray tap could wipe their clipboard. Replaced with an explicit toolbar.
- One-shot selection (no handles) made tmux text capture frustrating: misjudge by one cell and you re-select from scratch. Handles let users do the rough cut at long-press, then nudge.
- `pos.end.x` from xterm is exclusive while our long-press path stored inclusive — mixing the two caused intermittent "tap copies anywhere" because the hit-test sometimes used a 1-cell-too-wide rect. Now `selection` is canonically inclusive everywhere; only the xterm boundary translates.
