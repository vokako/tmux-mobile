# Terminal Sizing (cols × rows management)

## Context

xterm.js renders content into a fixed `(cols, rows)` grid. The grid must
match what will actually fit in the terminal's DOM container:

```
term.cols ≈ floor(termEl.clientWidth  / cellW)
term.rows ≈ floor(termEl.clientHeight / cellH)
```

If this equation breaks, the display degrades in a very visible way:

- `rows` too small → only the top half of the container shows content;
  the bottom half is empty terminal background because xterm has no
  lines to draw there.
- `rows` too large → xterm tries to render rows that are physically
  clipped, the cursor ends up off-screen, and input feels "wrong row".
- `cols` too small → visible wrapping that doesn't exist on the tmux
  side.
- `cols` too large → truncation, or last characters rendered past the
  visible edge.

Container size can change for many reasons on mobile:

- Orientation change / split-screen
- On-screen keyboard opening and closing
- Browser address bar retracting
- Safe-area insets updating
- User-triggered zoom in/out (`⌘+` or pinch)
- Font-size change (changes cell dimensions, not container, but the
  equation still needs re-evaluation)

## Earlier (removed) approach

The previous implementation had **six different code paths** all trying
to keep the equation satisfied:

1. `requestAnimationFrame(doResize)` at init.
2. `window.addEventListener('resize', ...)` with a 300ms debounce, with
   a mobile-specific "skip height-only changes" guard.
3. `window.addEventListener('keyboard-shift', ...)` with a 100ms delay
   that also reset a `lastFitCols/Rows` cache.
4. `window.addEventListener('terminal-refit', ...)` as an explicit
   "force refit" event, dispatched by `App.svelte` at 100ms and 500ms
   after keyboard close, and after zoom changes.
5. `$effect(() => { if (viewMode === 'terminal' && term) term.refresh(...) })`
   for the chat → terminal tab switch.
6. `ws-reconnected` event for post-reconnect refit.

Each was added to fix one observed bug. Together they drifted into a
state where:

- On Android, the IME open event is sometimes ignored by the guard in
  `App.svelte`'s `androidKeyboardHeight` handler (when the active
  element is a `<button>` instead of a textarea). `keyboard-shift` is
  not dispatched. The terminal stays at the pre-keyboard size while the
  keyboard physically covers the bottom half of the screen — resulting
  in the "only top half shows content" symptom users reported.
- The initial `requestAnimationFrame(doResize)` fires before xterm has
  actually rendered once, so `_renderService.dimensions.css.cell` is
  undefined and `calcFit` falls back to `fontSize * 0.6 / 1.2`. That
  estimate is off by a row or two for our font, so the first paint is
  sized slightly wrong and never self-corrects until something else
  (keyboard toggle, orientation change) triggers a re-fit.

Fixing this scenario-by-scenario kept producing "one more guard" style
patches — symptom fixes, not a root-cause fix.

## Decision

**One source of truth:** a `ResizeObserver(termEl)` is the only
trigger for re-fit.

The Android native height event still owns the upstream CSS viewport size.
Its stale-event guard must not discard an IME-open event merely because the
keyboard-toggle button is still the active element: Android can report the
height one task before xterm's hidden textarea receives focus. Such a height is
held for up to 500 ms and applied on the following text-input `focusin`; if no
focus arrives it is discarded as stale. A native height of zero always applies
immediately and clears any deferred open event. Once `--app-height` changes,
the `ResizeObserver(termEl)` remains the sole terminal re-fit trigger.

Rationale: every cause of container size change above ultimately
manifests as a change in `termEl.clientWidth` or `termEl.clientHeight`.
`ResizeObserver` observes exactly that, and its callback fires only
after layout is stable, so we don't hit the "clientHeight=0 mid-
transition" corner cases that the event-based approach drifted into.

**One extra trigger for the first render only:** xterm's real cell
metrics only become available after the first render. We attach
`term.onRender(...)` and re-fit once on its first fire. After that we
dispose the listener.

**What we keep, and why:**

- `keyboard-shift` event: still used, but ONLY to drive the keyboard
  lock/unlock state machine (see `terminal-gestures.md`). Sizing is
  not touched here — `--app-height` changes from `App.svelte` flow
  through normal CSS and show up as a `termEl` size change that the
  ResizeObserver catches.
- `ws-reconnected` event: this is not a sizing problem, it's a "we
  need to re-tell the server our cols/rows" problem. The server's
  resize tracker forgets per-connection state on disconnect. Kept as
  a distinct handler that re-sends `resize_pane`.
- The `pendingCols/Rows/Ts` server-echo reconciliation in
  `writeToXterm`. Unchanged.

## Implementation sketch

```js
function doResize() {
  const fit = calcFit();
  if (!fit) return;
  if (fit.cols === term.cols && fit.rows === term.rows) return;
  queuePaneResize(fit.cols, fit.rows);
  term.resize(fit.cols, fit.rows);
  if (lastContent) writeToXterm(lastContent, lastCursor);
}

const resizeObs = new ResizeObserver(() => doResize());
resizeObs.observe(termEl);

let firstRenderDone = false;
const onFirstRender = term.onRender(() => {
  if (firstRenderDone) return;
  firstRenderDone = true;
  doResize();
});
```

`queuePaneResize` updates xterm immediately but debounces the server RPC for
120 ms. A burst of WebView zoom/layout observations therefore sends only the
final tmux size, preventing stale intermediate resize echoes from replacing the
new grid. The explicit `app-zoom-change` event runs one final fit after Tauri's
native WebView zoom promise and two layout frames have settled.

Cleanup disposes the observer, listeners, and pending resize timer.

`doResize` is exposed via a module-level `doResizeRef` so the
`fontSize` $effect (which doesn't live inside the same $effect scope)
can trigger a re-fit after a font-size change.

## Feedback loop safeguard

`term.resize()` can cause a tiny geometric change to the xterm DOM
(scrollbar width, internal wrapper sizing) which ResizeObserver would
re-report. The `if (fit.cols === term.cols && fit.rows === term.rows) return`
early-out in `doResize` breaks the potential loop cleanly: once the
equation is satisfied, no more resize is sent.

## Alternatives considered

- **Keep the event spaghetti, just add more guards.** Rejected: the
  symptom is a coordination failure between six independent triggers,
  more triggers make it worse.
- **Use `term.fit()` from @xterm/addon-fit.** Considered briefly. The
  addon wraps the same `calcFit` + `term.resize` logic we already have
  and doesn't help with the "when to fire" coordination problem. Our
  `calcFit` also handles the `!cellSize ready` case, which the addon
  throws on in some versions. Not worth the dependency.
- **Use `MutationObserver` on the container.** Wrong tool — it observes
  subtree changes, not box size.

## Lessons learned

- When multiple unrelated code paths are all trying to keep the same
  invariant alive, prefer observing the thing the invariant is about
  directly, rather than subscribing to every upstream cause.
- `requestAnimationFrame` is not a safe substitute for "wait until
  layout has measured the element". For xterm specifically, wait for
  `term.onRender` instead.
- Screen symptoms like "only the top half shows content" on mobile
  almost always point to a cols/rows ↔ container mismatch, not to a
  rendering bug. Check the sizing path first.
