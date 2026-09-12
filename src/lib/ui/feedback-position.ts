import { anchorOf, menuHeightLimit, menuPlacement, popOrigin, viewBox, POPOVER_EDGE, type AnchorRect } from './placement.ts';

/** Translate a scrollport to the placement owner's origin; flip/clamp remain
 * menuPlacement's job, rather than a second set of feedback geometry rules. */
export function feedbackLayout(
  trigger: AnchorRect, bounds: AnchorRect, size: { w: number; h: number }, viewport: { w: number; h: number },
  limits: { scrollport?: AnchorRect; keepClear?: AnchorRect } = {},
) {
  const port = limits.scrollport ?? bounds;
  const area = {
    left: Math.max(0, bounds.left, port.left), top: Math.max(0, bounds.top, port.top),
    right: Math.min(viewport.w, bounds.right, port.right), bottom: Math.min(viewport.h, bounds.bottom, port.bottom),
  };
  const view = { w: area.right - area.left, h: area.bottom - area.top };
  if (trigger.right <= area.left || trigger.left >= area.right || trigger.bottom <= area.top || trigger.top >= area.bottom) return null;
  // Adjacent controls may be taller than the triggering text/glyph. Preserve
  // its horizontal anchor while protecting the whole control row vertically.
  const clear = limits.keepClear ?? trigger;
  const anchor = {
    left: Math.max(trigger.left, area.left) - area.left,
    right: Math.min(trigger.right, area.right) - area.left,
    top: Math.max(Math.min(trigger.top, clear.top), area.top) - area.top,
    bottom: Math.min(Math.max(trigger.bottom, clear.bottom), area.bottom) - area.top,
  };
  if (view.w <= 2 * POPOVER_EDGE || view.h <= 2 * POPOVER_EDGE
    || anchor.right <= anchor.left || anchor.bottom <= anchor.top) return null;
  const maxWidth = view.w - 2 * POPOVER_EDGE;
  const maxHeight = menuHeightLimit(anchor, view);
  if (!maxHeight) return null;
  const pos = menuPlacement(anchor, { w: Math.min(size.w, maxWidth), h: Math.min(size.h, maxHeight) }, view);
  return { x: area.left + pos.x, y: area.top + pos.y, maxWidth, maxHeight, origin: popOrigin(anchor, pos) };
}

interface FeedbackAnchor {
  trigger: Element | null;
  bounds: Element | null;
  scrollport?: Element | null;
  keepClear?: Element | null;
}

/** Local measurement only. The caller retains the feedback/dismissal state. */
export function feedbackPosition(node: HTMLElement, initial: FeedbackAnchor) {
  let options = initial;
  let frame = 0;
  let disposed = false;
  const ready = (value: boolean) => {
    node.classList.toggle('ready', value);
    node.toggleAttribute('inert', !value);
  };
  ready(false);
  function place() {
    if (disposed) return;
    const { trigger, bounds, keepClear } = options;
    const scrollport = options.scrollport ?? bounds;
    if (!trigger?.isConnected || !bounds?.isConnected) { ready(false); return; }
    const anchor = anchorOf(trigger), area = anchorOf(bounds), viewport = viewBox();
    const limits = { scrollport: scrollport ? anchorOf(scrollport) : undefined, keepClear: keepClear ? anchorOf(keepClear) : undefined };
    const limit = feedbackLayout(anchor, area, { w: 0, h: 0 }, viewport, limits);
    if (!limit) { ready(false); return; }
    node.style.position = 'fixed';
    node.style.width = 'max-content';
    node.style.maxWidth = `${limit.maxWidth}px`;
    node.style.setProperty('--feedback-max-height', `${limit.maxHeight}px`);
    const pos = feedbackLayout(anchor, area, { w: node.offsetWidth, h: node.offsetHeight }, viewport, limits)!;
    node.style.left = `${pos.x}px`;
    node.style.top = `${pos.y}px`;
    node.style.setProperty('--pop-origin', pos.origin);
    ready(node.offsetWidth > 0 && node.offsetHeight > 0);
  }
  const schedule = () => {
    if (disposed || frame) return;
    frame = requestAnimationFrame(() => { frame = 0; place(); });
  };
  const observer = new ResizeObserver(schedule);
  let unwatch = () => {};
  function watch() {
    const { trigger, bounds, keepClear } = options;
    const scrollport = options.scrollport ?? bounds;
    observer.observe(node);
    if (trigger) observer.observe(trigger);
    if (trigger && scrollport?.contains(trigger)) {
      for (let parent = trigger.parentElement; parent && parent !== scrollport; parent = parent.parentElement) observer.observe(parent);
    }
    if (bounds) observer.observe(bounds);
    if (keepClear) observer.observe(keepClear);
    if (scrollport) { observer.observe(scrollport); scrollport.addEventListener('scroll', schedule, { capture: true, passive: true }); }
    unwatch = () => { observer.disconnect(); scrollport?.removeEventListener('scroll', schedule, true); };
    schedule();
  }
  watch();
  return {
    update(next: FeedbackAnchor) { unwatch(); options = next; watch(); },
    destroy() { disposed = true; unwatch(); if (frame) cancelAnimationFrame(frame); },
  };
}
