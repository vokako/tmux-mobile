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
// being a location. This function is the one-time fold that gets there, and
// three of its rules were each a bug first.
//
// COMPLETION IS RECORDED, IN ONE WRITE. The first version compared values,
// which is only idempotent while nothing else writes the slot — the moment the
// slot is the live home, a second run overwrites a NEWER resident value with
// the stale unprefixed one. The second version recorded completion in two
// markers, and two writes always leave a window: the per-server one lands, the
// global one fails, and then the server it folded into skips while the NEXT
// server folds the same leftover inputs into its own slot (reviewer P1-a,
// second round). So there is ONE marker and ONE write, carrying both facts —
// which layout, and which server received the inputs.
//
// THE LIVE SET IS AUTHORITATIVE IN FULL, not just its present keys. An absent
// live key means the value was CLEARED, and for the current server that is a
// definite conclusion rather than a guess: arriving at a server surfaces its
// park as the live key (`pointTo`), so from that moment live and slot agree,
// and any later divergence is a live-side change. Copying only the present
// keys made a cleared draft or a reset read-mark come back from the old park
// the moment the slot became the live home. The fold therefore IS
// `parkFrom` — the switch's own "file this server's live keys under its id,
// an absent one clears the slot" — rather than a second loop that has to be
// taught the same rule.
//
// CURRENT MUST RESOLVE. An empty CURRENT has no slot to fold into; a CURRENT
// naming an id that is not in the saved registry (an entry removed, a value
// written by an older build) has a slot nothing will ever read. Both are a
// reported no-op with no writes and no marker, so a later run still folds once
// a connect has resolved which server this is.
//
// What it still does NOT do: clear the unprefixed keys. A client rolled back to
// a build from before #335 finds exactly the state it left. Deleting them
// belongs to the ②b commit that stops writing them.
//
// Not wired into production yet: ②a ships the function and its tests, and the
// ②b commit that switches every live read/write and the park/point/reset
// entry points over calls it once, before any page mounts.
import { PARKED_KEYS, currentServerId, loadServers, parkFrom } from './servers.ts';
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

/** The completion marker. Unprefixed and single: its subject is the INPUTS,
 * which there is only one set of, and a second marker would mean a second
 * write and a window between them. */
export const LAYOUT_KEY = 'tmux_state_layout';
/** The layout this fold produces. A version rather than a boolean so a later
 * layout change can tell "folded by #335" from "folded by whatever comes
 * next" instead of seeing an opaque true. */
export const LAYOUT_VERSION = '335';

/** What the marker records: the layout, and which server received the inputs
 * (the per-server half of completion, in the same write). */
export interface LayoutMark {
  v: string;
  into: ServerId;
}

/** The marker, or null when the inputs have not been consumed. Anything
 * unparseable still counts as consumed: the one thing worse than not knowing
 * which fold ran is reading the inputs a second time. */
export function layoutMark(storage: Store): LayoutMark | null {
  const raw = storage.getItem(LAYOUT_KEY);
  if (raw == null) return null;
  try {
    const parsed = JSON.parse(raw);
    if (parsed && typeof parsed === 'object') {
      return { v: String(parsed.v ?? ''), into: String(parsed.into ?? '') };
    }
  } catch { /* an older or hand-edited marker still means "consumed" */ }
  return { v: String(raw), into: '' };
}

export interface MigrationReport {
  /** Did the fold move anything? */
  ran: boolean;
  /**
   * `folded`: it ran and is recorded.
   * `unrecorded`: it ran but the marker could not be written — storage is
   *   failing. The inputs still read as unconsumed, so the next server to
   *   become current would receive them.
   *
   *   ②b must treat this — and a thrown error — as BOOT MIGRATION NOT
   *   COMPLETE, which is stronger than "do not switch servers" (reviewer,
   *   2026-10-09): it must also not bring up the pages that WRITE resident
   *   state and retry afterwards. A writable page plus an unconsumed input set
   *   is the half-migrated client this whole design exists to avoid — the
   *   page's new writes land in the slot, and the retry then folds the stale
   *   unprefixed values over them. Retry before anything can write, or come
   *   up read-only.
   * `no-current` / `unknown-current`: nothing to fold into. No writes, no
   *   marker; ask again once a connect has resolved the server.
   * `already-done`: the inputs were consumed by an earlier run, and reading
   *   them again would overwrite newer resident values.
   */
  reason: 'folded' | 'unrecorded' | 'no-current' | 'unknown-current' | 'already-done';
  /** The server the live values were filed under — or, for `already-done`,
   * the one an earlier run filed them under. */
  serverId: ServerId;
  /** Keys whose live value was copied into the slot. */
  folded: string[];
  /** Keys whose slot was REMOVED because the live value was gone: the user
   * cleared that draft, drawer or read-mark, and the old park must not come
   * back as this server's state. */
  cleared: string[];
}

/**
 * Fold the live unprefixed per-server keys into the CURRENT server's slot,
 * once and for all.
 *
 * Returns what it did, which is what the ②b caller logs and what the tests
 * assert on.
 */
export function migrateServerState(storage: Store): MigrationReport {
  const none = (reason: MigrationReport['reason'], serverId = ''): MigrationReport =>
    ({ ran: false, reason, serverId, folded: [], cleared: [] });

  const id = currentServerId(storage);
  if (!id) return none('no-current');
  // A dangling CURRENT must not guess a slot: filing the live values under an
  // id no saved server has is losing them quietly.
  if (!loadServers(storage).some((s) => s.id === id)) return none('unknown-current');

  const done = layoutMark(storage);
  if (done) return none('already-done', done.into);

  const folded: string[] = [];
  const cleared: string[] = [];
  for (const key of RESIDENT_KEYS) {
    if (storage.getItem(key) != null) folded.push(key);
    else if (storage.getItem(residentKey(key, id)) != null) cleared.push(key);
  }
  // The switch's own rule, not a second copy of it.
  parkFrom(storage, id);
  // Marked AFTER the move, so a failure part-way leaves the inputs readable
  // and the next run redoes the whole fold rather than half of it.
  try {
    storage.setItem(LAYOUT_KEY, JSON.stringify({ v: LAYOUT_VERSION, into: id } satisfies LayoutMark));
  } catch {
    // Two writes cannot be made atomic in `localStorage`. What is possible is
    // one write, and saying so when even that fails.
    return { ran: true, reason: 'unrecorded', serverId: id, folded, cleared };
  }
  return { ran: true, reason: 'folded', serverId: id, folded, cleared };
}
