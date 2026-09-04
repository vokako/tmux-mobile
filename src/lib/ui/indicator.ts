/**
 * The sliding indicator (motion.md §2, board #86): a tab bar, a rail or a
 * segmented control keeps ONE moving highlight that glides to the chosen
 * item instead of each item lighting up in place.
 *
 * Usage — the container is `position: relative` and holds one indicator
 * element wearing `.slide-ind` (a 2px bar) or `.slide-pill` (a filled pill
 * behind the item); the action measures the active child and writes the
 * geometry as CSS variables the atom's transform reads:
 *
 *   <nav class="tabbar" use:slideIndicator={{ key: page, active: '.active' }}>
 *     <span class="slide-ind" aria-hidden="true"></span>
 *     <button class:active={…}>…</button>
 *   </nav>
 *
 * `key` is any value that changes when the selection does (the action
 * re-measures on update); `active` is the selector of the chosen child
 * (default `[aria-current], .active, .on, .sel`). A ResizeObserver
 * re-measures on layout change (rotation, split drag, font size). The atom
 * only transitions once `.ready` is set — after the first measure — so the
 * indicator is born in place and never slides in from the corner.
 */

export interface IndicatorParams { key?: unknown; active?: string }

export interface IndicatorBox { offsetLeft: number; offsetTop: number; offsetWidth: number; offsetHeight: number }

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

export function slideIndicator(node: HTMLElement, params: IndicatorParams = {}) {
  let sel = params.active ?? DEFAULT_ACTIVE;
  const ind = () => node.querySelector<HTMLElement>(':scope > .slide-ind, :scope > .slide-pill');
  const measure = () => {
    const el = node.querySelector<HTMLElement>(sel);
    const vars = indicatorVars(el && el.offsetParent ? el : null);
    for (const [k, v] of Object.entries(vars)) node.style.setProperty(k, v);
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
      measure();
    },
    destroy() {
      ro?.disconnect();
    },
  };
}
