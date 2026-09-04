/**
 * `use:hoverInfo={() => info}` — a rich hover card for an element (motion.md
 * §2, board #86). Mouse and keyboard only: a finger has no hover, and a
 * long-press already means "menu" (ui/longpress). Opens after a short
 * dwell (immediately when the pointer hops from a neighbouring card — the
 * reader is scanning), closes on leave, press, scroll, Escape and blur. The
 * getter runs at open time, so the card always shows the CURRENT state.
 */
import { hoverCard, type HoverInfo } from './hover.svelte.ts';
import { anchorOf } from './placement.ts';
import { layout } from '../app/layout.svelte.ts';
import { dwellFor } from './hover-dwell.ts';

export type HoverGetter = (() => HoverInfo | null | undefined) | null | undefined;

export function hoverInfo(node: HTMLElement, get: HoverGetter) {
  let getter = get;
  let timer: ReturnType<typeof setTimeout> | null = null;
  let open = false;
  const clear = () => { if (timer) { clearTimeout(timer); timer = null; } };
  const hide = () => { clear(); if (open) { open = false; hoverCard.hide(); } };
  const show = () => {
    const info = getter?.();
    if (!info) return;
    const r = node.getBoundingClientRect();
    const align = r.left > window.innerWidth / 2 ? 'right' : 'left';
    open = true;
    hoverCard.show(anchorOf(node), info, align);
  };
  const arm = () => {
    if (layout.isTouchDevice || !getter) return;
    clear();
    timer = setTimeout(show, dwellFor(hoverCard.warm));
  };
  const onEnter = (e: PointerEvent) => { if (e.pointerType === 'mouse') arm(); };
  const onFocus = () => { if (node.matches(':focus-visible')) arm(); };
  node.addEventListener('pointerenter', onEnter);
  node.addEventListener('pointerleave', hide);
  node.addEventListener('pointerdown', hide);
  node.addEventListener('focus', onFocus);
  node.addEventListener('blur', hide);
  return {
    update(g: HoverGetter) { getter = g; },
    destroy() { hide(); node.removeEventListener('pointerenter', onEnter); node.removeEventListener('pointerleave', hide); node.removeEventListener('pointerdown', hide); node.removeEventListener('focus', onFocus); node.removeEventListener('blur', hide); },
  };
}
