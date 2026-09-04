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

/** The stagger the `.reveal*` atoms add to their last rows (app.css: 7 × 30ms). */
export const REVEAL_STAGGER_MS = 210;

/**
 * How long a `.reveal` / `.reveal-tail` class may stay on a container after a
 * load lands: one move + the longest stagger + a margin. The class is then
 * dropped, because the atoms animate every child that MOUNTS while present
 * (motion.md principle 13 — an older page prepended later must not rise).
 */
export function revealMs(): number {
  return reducedMotion() ? 0 : T_MOVE_MS + REVEAL_STAGGER_MS + 50;
}
