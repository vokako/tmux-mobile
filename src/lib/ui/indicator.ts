/**
 * The sliding indicator (motion.md §2, board #86): a tab bar, a rail or a
 * segmented control keeps ONE moving highlight that glides to the chosen
 * item instead of each item lighting up in place.
 *
 * Usage — the container is `position: relative` (or fixed) and holds one
 * indicator element wearing `.slide-ind` (a 2px bar) or `.slide-pill` (a
 * filled pill behind the item); the action measures the active descendant
 * and writes the geometry as CSS variables the atom's transform reads:
 *
 *   <nav class="tabbar" use:slideIndicator={{ key: page, active: '.active' }}>
 *     <span class="slide-ind" aria-hidden="true"></span>
 *     <button class:active={…}>…</button>
 *   </nav>
 *
 * `key` is any value that changes when the selection does (the action
 * re-measures on update); `active` is the selector of the chosen descendant
 * (default `[aria-current], .active, .on, .sel`) — it need not be a direct
 * child (the rail's buttons sit in a `.rail-slot` wrapper). `hidden` collapses
 * the indicator while the container is being rearranged (the rail's icon drag)
 * and re-measures when it turns false again. A ResizeObserver re-measures on
 * layout change (rotation, split drag, font size). The atom only transitions
 * once `.ready` is set — after the first measure — so the indicator is born in
 * place and never slides in from the corner.
 *
 * Geometry comes from client rects (the active box relative to the
 * container's box, divided by the root's CSS zoom), never from offsetLeft/Top:
 * an offset is relative to the nearest positioned ancestor, which for a
 * nested button is its wrapper, not the container. Because a rect includes a
 * transform in flight (a neighbour mid-`animate:flip`), an update measures
 * twice — now, and once the move tempo has elapsed — so the marker settles
 * where the item actually lands.
 */
import { moveMs } from './motion.ts';
import { uiZoom } from './placement.ts';

export interface IndicatorParams { key?: unknown; active?: string; hidden?: boolean }

export interface IndicatorBox { offsetLeft: number; offsetTop: number; offsetWidth: number; offsetHeight: number }

export interface RectLike { left: number; top: number; width: number; height: number }

export const DEFAULT_ACTIVE = '[aria-current], .active, .on, .sel, [aria-selected="true"]';

/** The four variables for an active box (pure, tested). `null` collapses the indicator. */
export function indicatorVars(box: IndicatorBox | null): Record<string, string> {
  if (!box) return { '--ind-x': '0px', '--ind-y': '0px', '--ind-w': '0px', '--ind-h': '0px' };
  return {
    '--ind-x': `${box.offsetLeft}px`,
    '--ind-y': `${box.offsetTop}px`,
    '--ind-w': `${box.offsetWidth}px`,
    '--ind-h': `${box.offsetHeight}px`,
  };
}

/**
 * The active item's box in the container's padding-box coordinates (where an
 * absolutely positioned indicator's `top: 0; left: 0` sits), from the two
 * client rects. `zoom` is the root's CSS zoom (client rects are visual pixels,
 * the indicator's variables are the container's own CSS pixels); `border` is
 * the container's top/left border width, which a client rect includes and the
 * padding box does not. Pure, so the arithmetic is tested without a DOM.
 */
export function boxFromRects(
  container: RectLike,
  item: RectLike,
  zoom = 1,
  border: { left: number; top: number } = { left: 0, top: 0 },
): IndicatorBox {
  const z = zoom || 1;
  return {
    offsetLeft: (item.left - container.left) / z - border.left,
    offsetTop: (item.top - container.top) / z - border.top,
    offsetWidth: item.width / z,
    offsetHeight: item.height / z,
  };
}

export function slideIndicator(node: HTMLElement, params: IndicatorParams = {}) {
  let sel = params.active ?? DEFAULT_ACTIVE;
  let hidden = !!params.hidden;
  let settle: ReturnType<typeof setTimeout> | null = null;
  const ind = () => node.querySelector<HTMLElement>(':scope > .slide-ind, :scope > .slide-pill');
  const measure = () => {
    const el = hidden ? null : node.querySelector<HTMLElement>(sel);
    const box = el && el.offsetParent
      ? boxFromRects(node.getBoundingClientRect(), el.getBoundingClientRect(), uiZoom(), { left: node.clientLeft, top: node.clientTop })
      : null;
    for (const [k, v] of Object.entries(indicatorVars(box))) node.style.setProperty(k, v);
    // Enable the glide only after the first placement (the next frame, so the
    // first vars are painted without a transition).
    const i = ind();
    if (i && !i.classList.contains('ready')) requestAnimationFrame(() => i.classList.add('ready'));
  };
  const ro = typeof ResizeObserver === 'function' ? new ResizeObserver(() => measure()) : null;
  ro?.observe(node);
  requestAnimationFrame(measure);
  return {
    update(p: IndicatorParams = {}) {
      sel = p.active ?? DEFAULT_ACTIVE;
      hidden = !!p.hidden;
      measure();
      // A second read after the move tempo: a rect measured mid-flip is the
      // OLD place (the inverse transform is still on), so the marker would
      // otherwise glide to where the item was.
      if (settle) clearTimeout(settle);
      settle = setTimeout(() => { settle = null; measure(); }, moveMs() + 20);
    },
    destroy() {
      if (settle) clearTimeout(settle);
      ro?.disconnect();
    },
  };
}
