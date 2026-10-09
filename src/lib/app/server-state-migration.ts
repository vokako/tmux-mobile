// Moving the per-server keys from "live plus parked" to "one slot per server"
// (board #335 ②a-3, reviewer r1 §C).
//
// Until now a per-server key had TWO homes: the unprefixed one, which holds
// whichever server is current, and `<key>::<id>`, where the switch files the
// server you are leaving (`servers.ts` parkFrom/pointTo). That works for one
// server at a time and cannot work for two: with A and B both on screen there
// is no "the" live value, and the first write from B would overwrite A's.
//
// So the `::<id>` slot becomes the ONLY home, and the unprefixed key stops
// being a location. This function is the one-time fold that gets there.
//
// It is a ONE-TIME fold recorded in storage, not a function that happens to be
// safe to repeat. The first version compared values instead of recording
// completion, and that is only idempotent while nothing else writes the slot —
// the moment the slot becomes the live home, a second run overwrites a NEWER
// resident value with the stale unprefixed one (reviewer P1-a; the test for it
// even asserted the overwrite). Two markers, because completion has two sides:
//
//   `tmux_state_layout` (unprefixed)  the INPUTS have been consumed. No later
//                                     run reads them again, so a CURRENT that
//                                     changes afterwards can never
//                                     re-attribute A's leftover live values
//                                     to B.
//   `tmux_state_layout::<id>`         THIS server's slot is in the new layout
//                                     and must never be folded into again.
//
// The rest of the contract:
//
//   - the ACTIVE value WINS, on the one run that reads it. A live unprefixed
//     value is what the user was looking at a moment ago; the park under the
//     current id, if any, is from the last time they left that server.
//   - old parks are KEPT. Every other server's `<key>::<id>` is already in its
//     final home.
//   - a resolved CURRENT is required. With no current server there is no slot
//     to fold INTO, and guessing one would file A's drafts under B. It is a
//     no-op that says so, and nothing is marked, so the caller can run it
//     again once a connect has resolved which server this is.
//   - the unprefixed keys are left READABLE. A client rolled back to a build
//     from before #335 finds exactly the state it left. Deleting them belongs
//     to the ②b commit that stops writing them, where it is a decision about
//     a delegate that no longer exists rather than about this fold.
//
// Not wired into production yet: ②a ships the function and its tests, and the
// ②b commit that switches every live read/write and the park/point/reset
// entry points over calls it once, before any page mounts.
import { PARKED_KEYS, currentServerId } from './servers.ts';
import type { ServerId } from './refs.ts';

type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

/** The keys this fold owns. `servers.ts` already names them: they are exactly
 * the ones the switch parks, because "parked per server" and "resident per
 * server" are the same set seen from two sides. Keeping one list means a key
 * added to one and forgotten in the other cannot exist. */
export const RESIDENT_KEYS = PARKED_KEYS;

/** Where a per-server key lives after the fold. The ONE place that spells the
 * layout, so nothing reconstructs `${key}::${id}` by hand. */
export function residentKey(key: string, serverId: ServerId): string {
  return `${key}::${serverId}`;
}

/** The completion marker's name. Deliberately NOT one of RESIDENT_KEYS: the
 * unprefixed spelling is a fact about the inputs, the suffixed one a fact
 * about a server's slot, and a key that the server store rewrote on access
 * could not say both. */
export const LAYOUT_KEY = 'tmux_state_layout';
/** The layout this fold produces. A version rather than a boolean so a later
 * layout change can tell "folded by #335" from "folded by whatever comes
 * next" instead of seeing an opaque true. */
export const LAYOUT_VERSION = '335';

export interface MigrationReport {
  /** Did the fold run? */
  ran: boolean;
  /** Why not, when it did not. `no-current`: nothing to fold into, ask again
   * after a connect. `already-done`: the inputs were consumed by an earlier
   * run, and re-reading them would overwrite newer resident values. */
  reason: 'folded' | 'no-current' | 'already-done';
  /** The server the live values were filed under. */
  serverId: ServerId;
  /** The keys whose live value was written into the current server's slot. */
  folded: string[];
}

/**
 * Fold the live unprefixed per-server keys into the CURRENT server's slot,
 * once and for all.
 *
 * Returns what it did, which is what the ②b caller logs and what the tests
 * assert on. A key with no live value is left alone: an absent live value is
 * not evidence that the server has nothing — before #315 it may never have
 * been written at all, and the park under its own id is then the only copy.
 */
export function migrateServerState(storage: Store): MigrationReport {
  const serverId = currentServerId(storage);
  if (!serverId) return { ran: false, reason: 'no-current', serverId: '', folded: [] };
  // Either side of the marker is enough to stop: the inputs were consumed, or
  // this server's slot has already received them. Both mean the unprefixed
  // keys are no longer inputs, and reading them again is the data loss this
  // check exists to prevent.
  if (storage.getItem(LAYOUT_KEY) != null || storage.getItem(residentKey(LAYOUT_KEY, serverId)) != null) {
    return { ran: false, reason: 'already-done', serverId, folded: [] };
  }
  const folded: string[] = [];
  for (const key of RESIDENT_KEYS) {
    const live = storage.getItem(key);
    if (live == null) continue;
    storage.setItem(residentKey(key, serverId), live);
    folded.push(key);
  }
  // Marked after the fold, so a crash halfway leaves the inputs readable and
  // the next run redoes the whole thing rather than half of it.
  storage.setItem(residentKey(LAYOUT_KEY, serverId), LAYOUT_VERSION);
  storage.setItem(LAYOUT_KEY, LAYOUT_VERSION);
  return { ran: true, reason: 'folded', serverId, folded };
}
