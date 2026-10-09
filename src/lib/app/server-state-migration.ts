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
// being a location. This function is the one-time fold that gets there:
//
//   - IDEMPOTENT. Running it twice is running it once. It is also safe on a
//     client that has already migrated, and on a fresh client with nothing to
//     move.
//   - the ACTIVE value WINS. A live unprefixed value is what the user was
//     looking at a moment ago; the park under the current id, if any, is from
//     the last time they left that server, which is older by construction.
//   - old parks are KEPT. Every other server's `<key>::<id>` is already in its
//     final home, so the fold must not touch it — and a park for the current
//     server is only overwritten by the live value, never deleted on its own.
//   - a resolved CURRENT is required. With no current server there is no slot
//     to fold INTO, and guessing one would file A's drafts under B. It is a
//     no-op and reports so, so the caller can run it again once a connect has
//     resolved which server this is.
//
// It does NOT remove the unprefixed keys. A client that rolls back to a build
// before #335 reads them and finds exactly the state it left, and the ②b
// commit that switches every live read and write over is what makes them dead
// weight. Clearing them is a later, separate decision with its own reason.
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

export interface MigrationReport {
  /** Did the fold run? False only when there is no current server. */
  ran: boolean;
  /** The server the live values were filed under. */
  serverId: ServerId;
  /** The keys whose live value was written into the current server's slot. */
  folded: string[];
  /** Keys that already had the same value in the slot, so nothing was written
   * (the second run of an idempotent fold reports all of them here). */
  unchanged: string[];
}

/**
 * Fold the live unprefixed per-server keys into the CURRENT server's slot.
 *
 * Returns what it did, which is what the ②b caller logs and what the tests
 * assert on. A key with no live value is left alone: an absent live value is
 * not evidence that the server has nothing — before #315 it may never have
 * been written at all, and the park under its own id is then the only copy.
 */
export function migrateServerState(storage: Store): MigrationReport {
  const serverId = currentServerId(storage);
  if (!serverId) return { ran: false, serverId: '', folded: [], unchanged: [] };
  const folded: string[] = [];
  const unchanged: string[] = [];
  for (const key of RESIDENT_KEYS) {
    const live = storage.getItem(key);
    if (live == null) continue;
    const slot = residentKey(key, serverId);
    // The second run sees the slot already holding the live value and writes
    // nothing. That is what makes the fold idempotent without a "migrated"
    // flag, which would itself be a value that can go stale or be cleared.
    if (storage.getItem(slot) === live) { unchanged.push(key); continue; }
    storage.setItem(slot, live);
    folded.push(key);
  }
  return { ran: true, serverId, folded, unchanged };
}
