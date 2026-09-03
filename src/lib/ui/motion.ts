/**
 * The motion helpers of the design language (design-language.md §1
 * "Micro-motion"). CSS carries almost all motion (app.css: .chev/.flip,
 * .appear*, .state-ctl); this module exists for the one thing CSS cannot do —
 * a keyed list REORDERING — via Svelte's `animate:flip`, and for the
 * reduced-motion gate every JS-driven duration must pass through.
 *
 *   <div animate:flip={{ duration: moveMs() }}>
 *
 * `svelte/transition` is deliberately NOT wrapped here: intros are the
 * `.appear*` classes, and exits are a snap on purpose (an outro keeps a dead
 * element in flow while the list underneath has already moved).
 */

/** The --t-move token (200ms) — kept in sync with app.css by a source test. */
export const T_MOVE_MS = 200;
/** The --t-fast token (120ms). */
export const T_FAST_MS = 120;

/** True when the user asked the OS for less motion. Safe without a window. */
export function reducedMotion(): boolean {
  return typeof matchMedia === 'function' && matchMedia('(prefers-reduced-motion: reduce)').matches;
}

/** Duration for a MOVE (reorder, slide): --t-move, or 0 under reduced motion. */
export function moveMs(): number {
  return reducedMotion() ? 0 : T_MOVE_MS;
}

/** Duration for micro feedback: --t-fast, or 0 under reduced motion. */
export function fastMs(): number {
  return reducedMotion() ? 0 : T_FAST_MS;
}
