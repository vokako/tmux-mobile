/** The note action row's state (board #46 review blocker): `gen` is a
 * MONOTONIC token — every transition bumps it, and the Copy beat's timeout
 * captures the gen of ITS OWN copy, so a stale timeout (anything happened
 * since: another copy, a switched row, a different issue, a project change)
 * is a no-op instead of closing somebody else's row or truncating their
 * Copied feedback. Pure transitions; Board.svelte holds one $state triple. */
export interface MessageActsState { open: number; copied: boolean; gen: number }
export const MESSAGE_ACTS_IDLE: MessageActsState = { open: -1, copied: false, gen: 0 };
/** Any context change — toggle/switch/outside/Escape/issue open/project
 * switch — sets which row is open (−1 = none) and orphans in-flight beats. */
export function messageActsSet(s: MessageActsState, open: number): MessageActsState {
  return { open, copied: false, gen: s.gen + 1 };
}
/** A copy ATTEMPT resolves (board #46, second blocker): the clipboard write
 * is an await, and the context can move underneath it — the user switches
 * issue/note while the promise is pending, and an unguarded "copied = true"
 * would stamp Copied onto whatever is open NOW and arm a timer against it.
 * The attempt's identity is the gen captured BEFORE the await; the stamp
 * lands only if the state still IS that context (any transition since —
 * open/switch/reset — bumped gen). A stale resolve returns the SAME object,
 * which is also the caller's signal not to arm the dismiss timer. */
export function messageActsCopyLanded(s: MessageActsState, attemptGen: number): MessageActsState {
  return attemptGen === s.gen ? { ...s, copied: true, gen: s.gen + 1 } : s;
}
/** The beat's timeout fires: it may put away only ITS OWN copy — a stale
 * gen leaves the state exactly as it found it. */
export function messageActsExpired(s: MessageActsState, gen: number): MessageActsState {
  return gen === s.gen ? { open: -1, copied: false, gen: s.gen } : s;
}
