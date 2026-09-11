/**
 * REVEAL, DO NOT RESIZE — the desktop Hub's partition motion (board #174,
 * owner 2026-09-11: "折叠展开最好是有动画，不是直接跳 … 点击展开最好也是有动画
 * 展开。这些要更丝滑一些").
 *
 * motion.md principle 3 forbids animating a width because surfaces measure
 * mid-frame, and principle 9 forbids any transition on an xterm ancestor
 * (the drawer holds the terminal). Both stay true by TECHNIQUE: the
 * partition's CONTENT is laid out at its final width and PINNED there
 * (`pinTrack`), and only the grid TRACK moves (`moveTrack`) — an animatable
 * `@property` factor in Hub.svelte's `grid-template-columns`, uncovering
 * content that never changes size. xterm is fitted once, to the final width;
 * the chat column in the middle does reflow, and it already owns width
 * changes through its ResizeObserver + retained reading anchor.
 *
 * The order is "placed first, then grows" (principle 8's popover rule): the
 * caller flips the REST state, lets Svelte flush, measures, pins, and only
 * then starts the move from the inline `from` value. All of that happens in
 * one task, so the final layout is never painted before the move begins.
 */
import { moveMs } from '../ui/motion.ts';

export type TrackProp = '--side-open' | '--drawer-open';

/** Freeze `track`'s content at the width it has RIGHT NOW (after the caller
 * flushed the rest state — the final width for an open, the current width
 * for a close), anchored to the edge that stays put while the grid track
 * moves (`end` for the left sidebar, `start` for the right drawer). Returns
 * the release. */
export function pinTrack(track: HTMLElement, side: 'start' | 'end'): () => void {
  track.style.width = `${track.offsetWidth}px`;
  track.classList.add('pinned', `pin-${side}`);
  return () => {
    track.style.width = '';
    track.classList.remove('pinned', 'pin-start', 'pin-end');
  };
}

/** How many moves are in flight per grid — the `.moving` gate drops only when
 * the LAST one ends, so a sidebar move ending cannot snap a drawer move. */
const inFlight = new WeakMap<HTMLElement, number>();

/** Move one grid track from `from` to the value its rest class now declares.
 * The start value is laid out synchronously (inline, no transition yet), the
 * `.moving` gate turns the transition on, and removing the inline value lets
 * the class value take over — the transition runs to it. Resolves when THAT
 * transition ends on the grid (the pin must not release a frame early: the
 * timer and the compositor clock differ by up to a frame, and releasing on
 * the timer alone let the content stretch to a not-yet-final track for one
 * frame — measured, board #174), with `moveMs() + 100` as the safety net for
 * an engine that never interpolates the factor (its change was a cut) and an
 * immediate resolve under reduced motion. The REST state is already applied
 * before this runs — state correctness never waits for transitionend
 * (principle 13); only the cosmetic release does. */
export function moveTrack(cols: HTMLElement, prop: TrackProp, from: 0 | 1): Promise<void> {
  cols.style.setProperty(prop, String(from));
  void cols.offsetWidth; // lay the start value out before the gate opens
  inFlight.set(cols, (inFlight.get(cols) ?? 0) + 1);
  cols.classList.add('moving');
  cols.style.removeProperty(prop);
  return new Promise((resolve) => {
    let done = false;
    const finish = () => {
      if (done) return;
      done = true;
      cols.removeEventListener('transitionend', onEnd);
      const left = (inFlight.get(cols) ?? 1) - 1;
      inFlight.set(cols, left);
      if (left <= 0) cols.classList.remove('moving');
      resolve();
    };
    const onEnd = (e: TransitionEvent) => { if (e.target === cols && e.propertyName === prop) finish(); };
    const ms = moveMs();
    if (ms === 0) { finish(); return; }
    cols.addEventListener('transitionend', onEnd);
    setTimeout(finish, ms + 100);
  });
}
