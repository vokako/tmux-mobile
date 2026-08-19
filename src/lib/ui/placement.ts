// Where a popover goes next to the control that opened it.
//
// UI-level on purpose: both the Hub's agent menu and the shared Select need it,
// and a `ui/` component importing from `hub/` would be a layering inversion.
// Pure, so the clamp/flip is testable without a browser.

export interface AnchorRect { left: number; right: number; top: number; bottom: number }

/**
 * Right-aligned to the trigger and below it, because the triggers are dot menus
 * and field-width buttons whose right edge is where the chevron sits; flipped
 * above when the menu is taller than the room left underneath; clamped to the
 * viewport on both axes so it is never partly off screen. A zero height means
 * "not measured yet", and then the flip is skipped rather than guessed — the
 * caller keeps the menu invisible for that one frame.
 *
 * Everything is in CSS pixels of the fixed layer's coordinate space; a caller
 * under CSS `zoom` must divide the trigger's client rect first (a client rect is
 * in visual pixels, a fixed child's `left` is in its own zoomed pixels).
 */
export function menuPlacement(
  anchor: AnchorRect,
  size: { w: number; h: number },
  view: { w: number; h: number },
  gap = 6,
  edge = 8,
): { x: number; y: number } {
  const x = Math.max(edge, Math.min(anchor.right - size.w, view.w - size.w - edge));
  let y = anchor.bottom + gap;
  if (size.h && y + size.h > view.h - edge) y = Math.max(edge, anchor.top - size.h - gap);
  return { x, y };
}

/** The root's CSS `zoom` (the web/Android interface scaling). 1 on the Tauri
 * desktop path, where the webview zooms instead and rects need no correction. */
export function uiZoom(): number {
  if (typeof document === 'undefined') return 1;
  return parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--ui-zoom')) || 1;
}

/** The trigger's rect in the fixed layer's coordinate space. */
export function anchorOf(el: Element): AnchorRect {
  const r = el.getBoundingClientRect();
  const z = uiZoom();
  return { left: r.left / z, right: r.right / z, top: r.top / z, bottom: r.bottom / z };
}

/** The viewport in the same space. */
export function viewBox(): { w: number; h: number } {
  const z = uiZoom();
  return { w: window.innerWidth / z, h: window.innerHeight / z };
}
