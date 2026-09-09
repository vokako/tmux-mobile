import { createDoubleTapDetector } from './terminal-keyboard.ts';
import { scrollSamples, scrollStep, releaseVelocity, coastStep, edgeDirection, edgeStep } from './terminal-gesture-motion.ts';
import type { VelocitySample } from './terminal-gesture-motion.ts';

type HandleSide = 'start' | 'end';
type TouchMode = 'idle' | 'down' | 'scrollbar' | 'scroll' | 'longpress-select' | 'handle-drag';

export interface GestureHost {
  available(): boolean;
  hasSelection(): boolean;
  isPinned(): boolean;
  pinUpdates(): void;
  requestRenderRelease(ms: number): void;
  hitHandle(x: number, y: number): HandleSide | null;
  grabHandle(which: HandleSide, x: number, y: number): { dx: number; dy: number };
  dragHeadAt(x: number, y: number): void;
  extendHeadAt(x: number, y: number): void;
  tryWordSelection(x: number, y: number): boolean;
  clearSelectionOutside(x: number, y: number): void;
  isScrollbarPoint(x: number): boolean;
  scrollPosition(): number;
  dragScrollbar(startY: number, startViewport: number, y: number): void;
  lineHeight(): number;
  edgeBounds(): { top: number; bottom: number };
  scrollLines(lines: number): void;
  openFromDoubleTap(): void;
}

export interface GestureEnvironment {
  now(): number;
  setDelay(callback: () => void, ms: number): number;
  clearDelay(id: number): void;
  requestFrame(callback: FrameRequestCallback): number;
  cancelFrame(id: number): void;
  vibrate(ms: number): void;
}

export interface TerminalGestures {
  onTouchStart(event: TouchEvent): void;
  onTouchMove(event: TouchEvent): void;
  onTouchEnd(event: TouchEvent): void;
  onTouchCancel(): void;
  isIdle(): boolean;
  isCoasting(): boolean;
  stopMomentum(): void;
  resetAfterVisibility(): void;
  cancelHold(): void;
  dispose(): void;
}

const LONG_PRESS_MS = 500;
const TOUCH_END_DELAY_MS = 500;

export function createTerminalGestures(host: GestureHost, environment: GestureEnvironment): TerminalGestures {
  // Mobile touch: scrolling, scrollbar drag, long-press word selection
  let touchId: number | null = null; // track the initial touch to ignore extra fingers
  let touchY = 0, touchStartY = 0, accumulatedDy = 0, longPressTimer: number | null = null, didScroll = false;
  let lastMoveTime = 0, momentumId: number | null = null, totalDist = 0;
  let velocitySamples: VelocitySample[] = []; // recent velocity samples for smoothing

  let onScrollbar = false, scrollbarStartY = 0, scrollbarStartViewport = 0;
  // Touch mode: 'idle' | 'down' | 'scrollbar' | 'scroll' | 'longpress-select' | 'handle-drag'
  let touchMode: TouchMode = 'idle';
  let dragHandle: HandleSide | null = null; // 'start' | 'end' when touchMode === 'handle-drag'
  let handleGrabDx = 0;  // finger minus dragged endpoint cell-centre at grab time
  let handleGrabDy = 0;
  let edgeScrollId: number | null = null; // rAF loop for drag-at-edge auto-scroll
  let edgeScrollDir = 0;   // -1 up / +1 down / 0 none
  let lastDragX = 0, lastDragY = 0; // latest compensated drag point (px)
  const stopMomentum = () => { if (momentumId) { environment.cancelFrame(momentumId); momentumId = null; } };
  // Double-tap -> keyboard (terminal-keyboard.md). Fed ONLY from the clean-tap
  // branch of onTouchEnd; every other gesture end resets it.
  const doubleTap = createDoubleTapDetector();

  // Auto-scroll while dragging a handle near the top/bottom edge - the
  // native way to extend a selection beyond the visible screen. Speed
  // ramps with proximity to the edge (1 px/frame deep in the zone is
  // ~1 row per 3 frames; pressed against the edge it's ~4 rows/frame...
  // we keep it gentle: 1 row per N frames scaling to 2 rows/frame).
  function updateEdgeScroll(clientY: number) {
    const rect = host.edgeBounds();
    const dir = edgeDirection(clientY, rect.top, rect.bottom);
    edgeScrollDir = dir;
    if (dir !== 0 && !edgeScrollId) {
      let acc = 0;
      const tick = () => {
        if (edgeScrollDir === 0 || touchMode !== 'handle-drag' || !host.available()) {
          edgeScrollId = null;
          return;
        }
        const rect2 = host.edgeBounds();
        const step = edgeStep(acc, edgeScrollDir, lastDragY + handleGrabDy, rect2.top, rect2.bottom);
        acc = step.accumulated;
        const lines = step.lines;
        if (lines !== 0) {
          host.scrollLines(lines);
          acc = step.remainder;
          // Viewport moved under the stationary finger - re-map the
          // endpoint so the selection keeps extending row by row.
          host.dragHeadAt(lastDragX, lastDragY);
        }
        edgeScrollId = environment.requestFrame(tick);
      };
      edgeScrollId = environment.requestFrame(tick);
    }
  }
  function stopEdgeScroll() {
    edgeScrollDir = 0;
    if (edgeScrollId) { environment.cancelFrame(edgeScrollId); edgeScrollId = null; }
  }

  // Hit-test the toolbar copy button (handled by the button's own pointer
  // events; we just need to know to skip terminal-touch handling when the
  // touch lands on the toolbar).
  function isOnToolbar(target: EventTarget | null) {
    const element = target as Element | null;
    return !!(element && element.closest && element.closest('.sel-toolbar'));
  }

  const onTouchStart = (e: TouchEvent) => {
    stopMomentum();
    touchId = e.touches[0]!.identifier; // track this finger
    const cx = e.touches[0]!.clientX;
    const cy = e.touches[0]!.clientY;

    // Toolbar / handle hit-tests come first - they're tiny UI surfaces and
    // the rest of the terminal-touch logic must not run for them. Toolbar
    // buttons handle their own clicks; we just bow out.
    if (isOnToolbar(e.target)) {
      touchMode = 'idle';
      return;
    }
    if (host.hasSelection()) {
      const which = host.hitHandle(cx, cy);
      if (which) {
        touchMode = 'handle-drag';
        dragHandle = which;
        // Re-anchor so the grabbed endpoint is `head` - all subsequent
        // moves rewrite head only (see beginEndpointDrag).
        // Record the finger's offset from the dragged endpoint's CELL
        // CENTRE in both axes, so the first touchmove maps to exactly the
        // cell the endpoint is already on - zero snap. The old code only
        // compensated Y (the end-handle dot sits at the cell's right edge,
        // so X was off by up to a full column) and then "lifted" the point
        // one row above the finger, which guaranteed a one-row jump on the
        // first frame of every drag.
        const offset = host.grabHandle(which, cx, cy);
        handleGrabDx = offset.dx;
        handleGrabDy = offset.dy;
        // Pin content updates while dragging. preventDefault on touchmove
        // (which is non-passive) blocks the page from scrolling.
        host.pinUpdates();
        return;
      }
    }

    // Scrollbar drag (right edge)
    onScrollbar = host.isScrollbarPoint(cx);
    if (onScrollbar) {
      touchMode = 'scrollbar';
      host.pinUpdates();
      scrollbarStartY = cy;
      scrollbarStartViewport = host.scrollPosition();
      return;
    }

    touchY = cy;
    touchStartY = touchY;
    accumulatedDy = 0;
    velocitySamples = [];
    totalDist = 0;
    lastMoveTime = environment.now();
    didScroll = false;
    touchMode = 'down';
    // Selection lives on. We DO allow scrolling within a selection - the
    // selection follows buffer rows, so scrolling just moves it. We do
    // NOT, however, kick off a new long-press while a selection exists;
    // long-press inside the selection is no-op (use handle to refine),
    // long-press outside cancels and starts a new selection.
    const startCX = cx, startCY = cy;
    longPressTimer = environment.setDelay(() => {
      if (touchMode !== 'down' || didScroll || !host.available()) return;
      // If a selection exists and the long-press lands inside it, ignore
      // (avoid surprising users who are aiming at handles).
      if (!host.tryWordSelection(startCX, startCY)) return;
      touchMode = 'longpress-select';
      environment.vibrate(15);
    }, LONG_PRESS_MS);
  };
  // Find the tracked touch by identifier (ignore extra fingers)
  const findTouch = (list: TouchList): Touch | null => { for (let i = 0; i < list.length; i++) if (list[i]!.identifier === touchId) return list[i]!; return null; };
  const onTouchMove = (e: TouchEvent) => {
    if (!host.available()) return;
    const t0 = findTouch(e.touches);
    if (!t0) return; // not our finger
    // Scrollbar drag: map touch delta proportionally to scroll position
    if (touchMode === 'scrollbar') {
      host.dragScrollbar(scrollbarStartY, scrollbarStartViewport, t0.clientY);
      if (e.cancelable) e.preventDefault();
      return;
    }
    // Handle drag: the grabbed endpoint is `head` (re-anchored at grab
    // time); just track the finger. Grab-offset compensation in BOTH axes
    // means the mapped cell starts exactly where the endpoint already is.
    if (touchMode === 'handle-drag' && host.hasSelection()) {
      lastDragX = t0.clientX - handleGrabDx;
      lastDragY = t0.clientY - handleGrabDy;
      host.dragHeadAt(lastDragX, lastDragY);
      updateEdgeScroll(t0.clientY);
      if (e.cancelable) e.preventDefault();
      return;
    }
    // Long-press selection: extend from anchor word to current cell
    if (touchMode === 'longpress-select' && host.hasSelection()) {
      host.extendHeadAt(t0.clientX, t0.clientY);
      if (e.cancelable) e.preventDefault();
      return;
    }
    // Normal content scroll
    const now = environment.now();
    const y = t0.clientY;
    const dy = touchY - y;
    const previousMoveTime = lastMoveTime;
    touchY = y;
    lastMoveTime = now;
    accumulatedDy += dy;
    totalDist += Math.abs(dy);
    const lh = host.lineHeight();
    velocitySamples = scrollSamples(velocitySamples, dy, now, previousMoveTime);
    const step = scrollStep(accumulatedDy, lh);
    const lines = step.lines;
    if (lines !== 0) {
      didScroll = true;
      cancelHold();
      if (touchMode === 'down') touchMode = 'scroll';
      host.pinUpdates();
      host.scrollLines(lines);
      accumulatedDy = step.remainder;
      if (e.cancelable) e.preventDefault();
    }
  };
  const onTouchEnd = (e: TouchEvent) => {
    const endedMode = touchMode;
    cancelHold();
    // Only a clean tap can be half of a double-tap; a scroll, drag or
    // long-press between two taps breaks the pair.
    if (endedMode !== 'down') doubleTap.reset();
    if (endedMode === 'scrollbar') {
      touchMode = 'idle';
      onScrollbar = false;
      host.requestRenderRelease(TOUCH_END_DELAY_MS);
      return;
    }
    if (endedMode === 'handle-drag') {
      touchMode = 'idle';
      dragHandle = null;
      stopEdgeScroll();
      // Selection persists; pinning persists
      return;
    }
    if (endedMode === 'longpress-select') {
      touchMode = 'idle';
      // Selection persists with current head; pinning persists
      return;
    }
    // 'down' (clean tap) or 'scroll' (released after scroll)
    if (endedMode === 'down') {
      touchMode = 'idle';
      const t0 = e.changedTouches?.[0];
      // Clean tap. If a selection exists and the tap was outside it (and
      // not on a handle/toolbar - those bailed at touchstart), cancel the
      // selection. That tap is spent on the cancel: it never starts a
      // double-tap pair, so dismissing a selection cannot pop the keyboard.
      if (host.hasSelection()) {
        doubleTap.reset();
        if (t0) {
          host.clearSelectionOutside(t0.clientX, t0.clientY);
        }
        return;
      }
      // A single tap does nothing. The second clean tap of a double-tap
      // opens the keyboard - the ONE terminal-area gesture that may.
      if (t0 && doubleTap.tap({ x: t0.clientX, y: t0.clientY, t: environment.now() })) {
        // Cancel the browser's synthetic mouse events for this touch
        // (mousedown/mouseup/click/dblclick): xterm turns a dblclick into a
        // word selection, which onSelChange would adopt as a stray
        // selection right under the keyboard. touchend is registered
        // non-passive for exactly this call.
        if (e.cancelable) e.preventDefault();
        host.openFromDoubleTap();
      }
      return;
    }
    // 'scroll' or anything that left touchScrolling=true
    touchMode = 'idle';
    if (host.isPinned() && velocitySamples.length > 0) {
      const lh = host.lineHeight();
      let v = releaseVelocity(velocitySamples, lh);
      if (Math.abs(v) > 0.1) {
        let acc = 0;
        const coast = () => {
          const step = coastStep(v, acc);
          v = step.velocity;
          acc = step.accumulated;
          const lines = step.lines;
          if (lines !== 0) {
            host.scrollLines(lines);
            acc = step.remainder;
          }
          if (step.running) {
            momentumId = environment.requestFrame(coast);
          } else {
            momentumId = null;
            host.requestRenderRelease(200);
          }
        };
        momentumId = environment.requestFrame(coast);
      } else {
        host.requestRenderRelease(TOUCH_END_DELAY_MS);
      }
    } else if (host.isPinned() && !host.hasSelection()) {
      host.requestRenderRelease(TOUCH_END_DELAY_MS);
    }
  };
  const onTouchCancel = () => {
    cancelHold();
    doubleTap.reset();
    onScrollbar = false;
    touchMode = 'idle';
    dragHandle = null;
    stopMomentum();
    stopEdgeScroll();
    // Don't blow away the selection on a stray cancel - but if we were
    // mid-handle-drag the user expects the partial drag to commit, which
    // it already has via moveHead() on the last touchmove.
    if (!host.hasSelection()) host.requestRenderRelease(100);
  };

  function cancelHold() {
    if (longPressTimer) { environment.clearDelay(longPressTimer); longPressTimer = null; }
  }

  return {
    onTouchStart, onTouchMove, onTouchEnd, onTouchCancel,
    isIdle: () => touchMode === 'idle',
    isCoasting: () => momentumId !== null,
    stopMomentum,
    resetAfterVisibility() {
      onScrollbar = false;
      touchMode = 'idle';
      dragHandle = null;
      stopMomentum();
      stopEdgeScroll();
    },
    cancelHold,
    dispose() {
      cancelHold();
      stopMomentum();
      stopEdgeScroll();
    },
  };
}
