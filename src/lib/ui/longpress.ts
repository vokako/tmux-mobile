// A long press is the touch equivalent of a right-click.
//
// Owner ask, 2026-08-20: "还有很多地方增加右键点击操作，和手机长按". Desktop gets
// `oncontextmenu`, which the platform already provides; a phone has no such event,
// so a hold has to be measured. This is the one place that measures it.
//
// Touch ONLY. A mouse already has a right button, and treating a held left button
// as a long press would fire a menu in the middle of a text selection.

/** How long a stationary finger becomes a press. Android's own long-press is
 * ~500ms; matching it makes ours feel native rather than sluggish or trigger-happy. */
const HOLD_MS = 500;
/** Movement that means "this is a scroll, not a press". The same 10px the app's
 * other gesture code uses as its slop, and it is the whole reason a list can still
 * be flicked while its rows are long-pressable. */
const SLOP_PX = 10;

export interface LongPressOptions {
  /** Fired with the touch point once the hold completes. */
  onlongpress?: (p: { x: number; y: number }) => void;
  ms?: number;
}

/**
 * Svelte action: `<div use:longpress={{ onlongpress: (p) => open(p) }}>`.
 *
 * Three things it has to get right, each of which is a way to make a list feel
 * broken:
 *  - a SCROLL is not a press (cancel past the slop, and on a second finger);
 *  - the press must not also be a TAP — the click that follows the release is
 *    swallowed once, or a row would both open its menu and activate itself;
 *  - the browser's own long-press behaviours (text selection, the link callout)
 *    are suppressed for the element while the finger is down, not globally: the
 *    page stays selectable, this row does not fight the menu it just opened.
 */
export function longpress(node: HTMLElement, options: LongPressOptions = {}) {
  let opts = options;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let start: { x: number; y: number } | null = null;
  let fired = false;

  const clear = () => {
    if (timer !== null) {
      clearTimeout(timer);
      timer = null;
    }
    start = null;
  };

  const onTouchStart = (e: TouchEvent) => {
    if (e.touches.length !== 1) {
      clear();
      return;
    }
    const t = e.touches[0]!;
    start = { x: t.clientX, y: t.clientY };
    fired = false;
    timer = setTimeout(() => {
      timer = null;
      if (!start) return;
      fired = true;
      opts.onlongpress?.({ ...start });
    }, opts.ms ?? HOLD_MS);
  };

  const onTouchMove = (e: TouchEvent) => {
    if (!start || e.touches.length !== 1) {
      clear();
      return;
    }
    const t = e.touches[0]!;
    if (Math.abs(t.clientX - start.x) > SLOP_PX || Math.abs(t.clientY - start.y) > SLOP_PX) clear();
  };

  const onTouchEnd = () => clear();

  // The tap that would otherwise follow the press. Capture phase so it never
  // reaches the row's own handler.
  const onClick = (e: MouseEvent) => {
    if (!fired) return;
    fired = false;
    e.preventDefault();
    e.stopPropagation();
  };

  node.addEventListener('touchstart', onTouchStart, { passive: true });
  node.addEventListener('touchmove', onTouchMove, { passive: true });
  node.addEventListener('touchend', onTouchEnd);
  node.addEventListener('touchcancel', onTouchEnd);
  node.addEventListener('click', onClick, true);

  return {
    update(next: LongPressOptions) {
      opts = next ?? {};
    },
    destroy() {
      clear();
      node.removeEventListener('touchstart', onTouchStart);
      node.removeEventListener('touchmove', onTouchMove);
      node.removeEventListener('touchend', onTouchEnd);
      node.removeEventListener('touchcancel', onTouchEnd);
      node.removeEventListener('click', onClick, true);
    },
  };
}

/**
 * Did this hold travel far enough to be a scroll? Pure, so the slop rule is
 * testable without a browser — it is the difference between "a list you can
 * flick" and "a list that pops a menu when you try".
 */
export function isScroll(
  from: { x: number; y: number },
  to: { x: number; y: number },
  slop = SLOP_PX,
): boolean {
  return Math.abs(to.x - from.x) > slop || Math.abs(to.y - from.y) > slop;
}
