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
| `scroll` | `down` → accumulated signed movement reaches one line | inertial content scroll |
| `longpress-select` | `down` → 500ms hold | word-select; touchmove extends head |
| `handle-drag` | touchstart in a selection handle's capsule hit zone | moves that endpoint |

## Touchstart Hit-Test Order
1. **Toolbar button**: bow out — the button has its own pointer handler.
2. **Selection handle**: enter `handle-drag` inside its capsule, with same-row overlap divided at the midpoint and edge handles extending to the container edge.
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
- **Any local input** (keystroke, shortcut button, paste): clear — see below.

## Input Returns to the Live Tail (`resumeLiveTail`)

Four states suppress rendering: a pinned selection, an unsettled touch scroll
(`touchScrolling`), a live momentum coast, and a viewport parked in scrollback
(`termAtBottom === false`, frames deferred as `hasNewContent`). None of them
used to end on input, which produced a display that looked broken: *you type
and nothing appears.*

Why it doesn't self-heal: the server only pushes a pane frame when the state
differs from the last one it **sent** (`state_key == prev → skip`). It has no
idea the client dropped a frame, so it never re-sends. `lastContent` keeps
advancing while the screen stays frozen, and the typed characters surface only
when some unrelated event (a resize, a visibility change, a pane switch)
happens to repaint — which is exactly why "switch to another tab and back"
appeared to fix it.

So every send path (`enqueueKeys`, and the paste branch that goes to
`paste_text` instead) calls `resumeLiveTail()` first: drop the selection, stop
the momentum coast, unpin `touchScrolling`, snap to the bottom, repaint from
`lastContent`. It early-returns when nothing is suppressed, so ordinary typing
costs one boolean check. This matches real terminals, where any keystroke
returns you to the live output and drops the selection.

Two traps worth keeping written down:

- **`unlockKeyboard()` used to `clearTimeout(endTouchScrollTimer)`.** That timer
  is the *only* pending reset of `touchScrolling` (armed for
  `TOUCH_END_DELAY_MS` = 500 ms after every scroll, longer with momentum).
  Tapping the keyboard toggle inside that window therefore pinned the display
  forever — the exact "I typed and the terminal froze" report. It now calls
  `resumeLiveTail()` instead of cancelling the reset.
- **Momentum outlives the snap.** Setting `termAtBottom = true` and
  `scrollToBottom()` while a coast is still running is pointless: the next
  coast frame scrolls straight back off the tail and re-arms the deferral.
  `resumeLiveTail` must `stopMomentum()` (measured, not theorised — typing
  mid-coast reproduced it).

## Coordinate Model
- `selection = { anchor: {row, col}, head: {row, col} }`
- `row` is **absolute buffer row** (not viewport-relative). Stable across scrolling.
- `col` is 0..cols-1, **inclusive** on both endpoints.
- `selStart(s)` / `selEnd(s)` derive the geometric (top-left, bottom-right) ordering from `anchor`/`head`.

The xterm.js API is converted at the boundary:
- `applySelectionToXterm()` calls `term.select(start.col, start.row, length)` where `length` spans the inclusive range.
- `onSelectionChange` reads `term.getSelectionPosition()` whose `pos.end.x` is exclusive, and converts to inclusive (`max(0, pos.end.x - 1)`).

### Selection Decision Ownership (#140, 2026-09-09)

`selection-model.ts` now owns the previously inline range containment,
inclusive length, endpoint re-anchoring, exclusive-end conversion and
word-boundary scan. `Selection` remains the one anchor/head definition;
`selStart` and `selEnd` are unchanged. The component still owns selection
state, guards before applying/re-anchoring or adopting a native selection,
xterm calls, the reentrancy guard, UI measurement and render pin, in the same
order. Range containment and absent-string fallback belong to the pure model.

This is a mechanical move, not a correction to boundary semantics:
length retains its one-cell floor, `end.x=0` retains the next-row col-0
clamp, and word scanning still indexes the already-translated UTF-16 string
and splits only at whitespace. Buffer-line reads stay in Terminal; the
model has no DOM, xterm, clock or Svelte dependency. Do not introduce a
second selection shape or a Unicode-width engine here.

Unit vectors cover absent ranges/strings, inclusive row/column edges,
resized-grid length flooring, equal/reversed endpoints and repeated
crossings, conversion and string boundaries. The source contracts follow
ownership: numeric rules live in the model tests, while Terminal tests pin
the real helper calls and unchanged side-effect guards.

The five lifted formulas/bodies match their originals after parameter/return
normalization. The #139 comparison preserves all 92 final-state/geometry
signatures and xterm selection arguments; 88 PNGs match byte-for-byte, with
the other four differing only in the unchanged 1.5s Copied toast's capture
timing. Reversing the grabbed endpoint fails the crossover unit vector and
real-component browser check. Android validation remains deferred.

## Handle UI
- Visual: 12px filled circle in `var(--accent)`, with a 2px-wide stem one cell high. The positioned wrapper has zero width/height; pseudo-elements draw the stem and dot.
- Position: the leading anchor is the start cell's top-left corner, its stem runs down through the cell, and its dot sits below. The trailing anchor is the end cell's bottom-right corner, its stem runs up, and its dot also sits below. Edge dots shift 6px inward without moving the stem.
- Hit zone: 28px half-width around each anchor. The leading Y interval is `startY - 11` through `startY + cellH + 22`; trailing is `endY - cellH - 11` through `endY + 22`. Same-row overlap divides at the horizontal midpoint (the leading handle is tested first). At column 0 / the final column, the respective X interval extends to the container edge. Handle hit testing precedes the scrollbar's 30px zone.

**Documentation correction (#139, 2026-09-09):** these are the existing
`hitHandle` and CSS rules from `7bf035c6`, not a new hit-target design. The
former 22px-radius/44px-wrapper description had drifted from the implementation.

### Numeric Geometry Ownership (#141, 2026-09-09)

`terminal-gesture-geometry.ts` first takes the point-to-cell, grab-offset and
horizontal-snap formulas. Inputs are measured numbers; the module does not
read DOM, xterm or a clock. Terminal still reads the actual client rectangle
and the one `cellSize` source at the original call sites, including the nested
cell mapping during a handle drag. Nothing stores geometry between gestures.
Buffer-row conversion and the existing writes to the drag state stay in
their adapters.

Boundary vectors pin flooring/clamping, nonzero origins, fractional and
changed measurements, compensation in both axes, inclusive snap edges and
the left-edge priority in overlapping zones. This is a mechanical move;
it does not retune thresholds or change coordinate systems.
The three formulas and metric-read order match the original AST. Moving
the 10px snap minimum to 11px fails the just-inside boundary vector;
restoring it returns green.

The second mechanical commit moves `selectionView` and `hitSelectionHandle`.
They reuse the canonical selection ordering and return the existing derived
UI coordinates or handle identity; `SelectionUI` is a projection, not another
selection store or a saved measurement. Missing selection/element and
zero-cell-metric guards remain in Terminal, before the same live reads.
The five formula bodies, the `cellSize` implementation and measurement-call
order match the pre-extraction code; markup/CSS and gesture scheduling stay
unchanged.

Projection/hit vectors cover independent offscreen endpoints, strict toolbar
flip boundaries, horizontal clamping, fractional metrics, capsule edges,
same-row midpoint ties and the handle occupying the scrollbar touch zone.
The existing row-3 Copy anchor (26px, producing the measured -16px clipping)
is pinned deliberately; #143 remains a separate behavior fix.

Chromium 152.0.7977.64 with real xterm 6.0.0 reproduces #140's 92
state/geometry signatures and selection-call arguments. 88 PNGs are
byte-identical; the four Copy captures differ only in the unchanged toast's
1.5s lifetime. A separate touch 3px inside the right edge still drags the
handle. The negative control excludes that scrollbar-width region from the
end capsule: the unit vector fails and the real component calls
`scrollToLine`, leaving column 40 instead of moving to 36. Restoring the
capsule restores the drag. These are off-device checks, not Android IME or
physical-touch verification.

## Toolbar UI
- Single "Copy" button (one job, one button).
- Default: above the selection's first row, horizontally centered between start and end (or roughly above the start cell when the selection spans multiple rows).
- If too close to the top (< 8px), flips to below the selection's last row.
- X is clamped to `[48, container_width - 48]` so the toolbar never escapes its container.
- `pointerdown` handler stops propagation and calls `copySelection()` — the touchstart hit-test on the underlying `termEl` would otherwise treat it as a tap and try to cancel the selection.

## Keyboard Control

### States
- `inputmode="text"` is pinned at mobile textarea initialization.
- `kbLocked = true` makes the focus handler immediately blur the textarea.
- `kbLocked = false` permits focus; explicit unlock blurs an already-focused
  textarea before focusing it again so a dismissed IME can reopen.

### Transitions
| From | Event | To | Action |
|------|-------|----|--------|
| either | keyboard toggle, IME hidden | unlocked | focus textarea, 1.5s grace |
| either | double-tap on terminal (two clean `down` taps ≤300ms, ≤40px apart, no selection) | unlocked | `unlockKeyboard()`; the second touchend is `preventDefault`ed so no synthetic dblclick reaches xterm |
| locked | single tap on terminal | locked | no-op |
| unlocked | single tap on terminal | unlocked | no-op |
| unlocked | textarea blur (150ms timer) | locked (or retry focus if in grace) | grace → re-focus, at most twice; otherwise lock |
| unlocked | keyboard-shift kbH=0 (was >0, post-grace) | locked | lock, blur |
| either | keyboard toggle, IME visible | locked | end grace, lock, blur |
| unlocked | pane switch | locked | reset |

**Documentation correction (#139, 2026-09-09):** `a228b41c` records why
none/text toggling was retired: Android's first InputConnection could cache
the original `none` value and ignore the first attempt to open the IME.
The toggle reads the actual `keyboard-open` class, not `kbLocked`, since a
system IME close can leave the textarea focused. This documents the current
focus-gated implementation; it does not claim a new Android verification.

### Key Rules
1. **Two ways to open the keyboard, both through `unlockKeyboard()`: the toggle button and a double-tap on the terminal.** A single tap never opens or closes it. The double-tap is detected on `touchend` by `createDoubleTapDetector` (`terminal-keyboard.ts`) and fed ONLY from the clean-tap (`down`) branch — a scroll, scrollbar drag, long-press or handle drag between two taps resets the pair, and a tap that cancels a selection is spent on the cancel. (Until 2026-09-03 the docs promised this gesture while the toggle was the only caller; `Terminal.source.test.ts` now pins both callers.)
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
- `endTouchScroll` via setTimeout can fire after explicit unlock → removed kbLocked manipulation.
- `data.length <= 1` misclassifies auto-paired input as paste → use paste event flag.
- Android `OnGlobalLayoutListener` can fire stale keyboard heights → guard with activeElement check.
- "Tap inside selection to copy" looked clever but was ambiguous — users couldn't tell whether the selection was "live", and a stray tap could wipe their clipboard. Replaced with an explicit toolbar.
- One-shot selection (no handles) made tmux text capture frustrating: misjudge by one cell and you re-select from scratch. Handles let users do the rough cut at long-press, then nudge.
- `pos.end.x` from xterm is exclusive while our long-press path stored inclusive — mixing the two caused intermittent "tap copies anywhere" because the hit-test sometimes used a 1-cell-too-wide rect. Now `selection` is canonically inclusive everywhere; only the xterm boundary translates.

## Fixed-Frame Motion

The existing release calculation weights up to five velocity samples from
the last 100ms, multiplies the average px/ms by 16 and caps it at
`MOMENTUM_MAX_PX = 240` before converting to lines/frame. Coast starts above
0.1 line/frame, multiplies velocity by 0.95 before accumulating each frame,
and stops at 0.05 or below. Both scroll paths retain signed fractional
remainders with `Math.trunc`. Edge dragging uses a 36px zone and ramps from
0.25 to 2 rows/frame. These are current fixed-frame rules, not a claim of
refresh-rate-independent physics.

Documentation correction (#139, 2026-09-09): the release comment said 120px
while the executable cap was already 240px. The comment now matches the
constant. Characterization pins that value; extraction must not tune it or
change the time model.

## Characterization Before Extraction

Board #139 (2026-09-09) adds source contracts and a real-Terminal Chromium
baseline before moving these decisions. Five additional source cases pin
text-mode focus gating, gesture priority/passivity/cleanup, inclusive selection
and its render pin, endpoint/capsule geometry, and fixed-frame motion.
They do not execute touch handlers or simulate layout.

The browser fixture uses actual xterm 6.0.0 with controlled RPC replies and
build-layer public-API trace wrappers, never replacement parsing/rendering
logic. Eight desktop/compact, theme/motion and DOM/WebGL combinations record
92 state signatures and viewport-sized PNGs: selection/viewport/handle/toolbar
rectangles, action traces, crossing and edge-cancel, explicit Copy, input
interrupting momentum, focus pairing, font/line-height/zoom, hidden replay
and pane disposal. Touch events are constructed in Chromium with controlled
timestamps; mouse selection/Copy and screenshots use Playwright. Both the
real DOM fallback and WebGL renderer were exercised. The fixture, logs and
images are referenced on #139.

Measured versions: Chromium 152.0.7977.64, Node 22.23.2, Svelte 5.53.5 and
Vite 6.4.1. Changing only the hit half-width from 28px to 8px fails the
source contract and the 24px-offset grab: the endpoint remains at column 14
instead of moving to 20. Restoring the original source returns green.
Production behavior, markup and CSS are unchanged.

The baseline deliberately retains a discovered defect (#143): a row-3
selection anchors Copy at 26px, but its 42px height and upward transform put
its top at -16px under the clipped container. Fix it separately rather than
silently changing toolbar placement during extraction.

No Android pass was performed. These results cannot establish physical
touch batching, fling feel, user activation, IME composition/height ordering,
haptics or OS suspension behavior; the owner pass in #138 remains a gate
for the later controller move, not for these off-device characterizations.
