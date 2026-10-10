// One list over several servers (board #335 ②a-5).
//
// A union list is where two servers become visible at once, and it has to be
// a PROJECTION rather than a new source of truth: each server's own ordering
// rule still decides its rows, and this module only interleaves them.
//
// So it MERGES already-ordered slices with the domain's own comparator — the
// very function that domain sorts by (`projects.compareRows`) — and never
// sorts anything itself. The first version took a `rank` instead, composed
// from `projectUpdatedMs`, and that was a different clock from the one
// `sortRows` uses: `projectUpdatedMs` falls back to `last_seen_at`, which is
// right for the age LABEL and wrong for ordering (the capturer rewrites it on
// every tick). A union built on it could reorder a SINGLE server's list, which
// is the one thing this module must never do (reviewer P1, 2026-10-10).
//
// With one slice a merge of one list IS that list, by construction rather than
// as a property of some rank function — which is why the Single-mode guarantee
// is now structural.
//
// Three properties are the whole point:
//
//   - INDEPENDENT ARRIVAL. The function is pure over whatever slices it is
//     given, so a server that has not answered yet simply is not in them, and
//     the list is complete for the servers that have.
//   - ONE FAILURE DOES NOT ERASE THE OTHERS. A slice that failed contributes
//     no rows and is reported in `missing`, so the view can say which server
//     is unreachable instead of showing a shorter list with no explanation.
//   - EVERY ROW CARRIES ITS REF. A union row is the only place a name is
//     ambiguous, so each one comes out with the `Ref` that names its object on
//     its server. ②b's row actions take their runtime FROM that ref rather
//     than from a single "current" api, which is what keeps a click on B's
//     row from acting on A.
//
// Not wired into production: no component changes in ②a. The server TAG below
// is the data a view would show; the visual atom for it is ②b's, because a new
// visual species needs the design-language review that ②a cannot do from here.
import type { Ref, ServerId } from './refs.ts';
import { refKey } from './refs.ts';

/** What one server contributed. */
export interface ServerSlice<T> {
  serverId: ServerId;
  /** A short label for the server — its entry name. Shown only when more than
   * one server contributes to the list. */
  name: string;
  /** The server's position in the saved registry. It breaks ties, so a union
   * of two servers whose rows rank equally is STABLE across renders instead of
   * flickering with whichever answered first. */
  order: number;
  /** Did this server answer? `false` means "asked and failed", which is not
   * the same as an empty list. */
  ok: boolean;
  /** This server's rows, in the order ITS own rule put them. */
  items: readonly T[];
}

/** A row of a union list. */
export interface UnionRow<T, R extends Ref> {
  serverId: ServerId;
  item: T;
  /** Names this object on its own server. */
  ref: R;
  /** `refKey(ref)` — an `{#each}` key that cannot collide across servers. */
  key: string;
  /** The server label to show, or null when only one server contributes.
   * Null in Single mode is what makes the union invisible there. */
  tag: string | null;
}

export interface UnionList<T, R extends Ref> {
  rows: UnionRow<T, R>[];
  /** Servers that were asked and failed. The view owes the user a word about
   * these; it must not quietly show a shorter list. */
  missing: ServerId[];
}

export interface UnionSpec<T, R extends Ref> {
  /** How to name an item of this slice on its server. */
  ref: (serverId: ServerId, item: T) => R;
  /**
   * The DOMAIN's own comparator — the function it sorts its own list by
   * (`projects.compareRows(talk)`). Used ONLY to decide which slice's head
   * comes next; a slice is never re-sorted, so a comparator that disagreed
   * with the given order would still not change any single server's list.
   *
   * Omit it for a list whose order IS the server's answer (tmux's session
   * order): the slices are then concatenated in server order.
   */
  compare?: (a: T, b: T) => number;
  /**
   * Show the source tag? Defaults to "more than one server is in this list",
   * which is the SOURCE SET and not how many answered — a server dropping out
   * must not make the remaining rows stop saying where they are from
   * (reviewer P1, 2026-10-10). ②b passes the mode explicitly where the mode,
   * rather than the count, is what decides.
   */
  showSource?: boolean;
}

/**
 * Merge several servers' already-ordered rows into one list.
 *
 * With one slice the result is that slice, in its given order — a merge of one
 * list cannot reorder it.
 */
export function unionRows<T, R extends Ref>(
  slices: readonly ServerSlice<T>[],
  spec: UnionSpec<T, R>,
): UnionList<T, R> {
  const missing = slices.filter((s) => !s.ok).map((s) => s.serverId);
  // Source order is the saved registry order, so ties and concatenation are
  // stable across renders rather than following whoever answered first.
  const live = slices.filter((s) => s.ok).slice().sort((a, b) => a.order - b.order);
  const tagged = spec.showSource ?? slices.length > 1;

  const make = (slice: ServerSlice<T>, item: T): UnionRow<T, R> => {
    const ref = spec.ref(slice.serverId, item);
    return { serverId: slice.serverId, item, ref, key: refKey(ref), tag: tagged ? slice.name : null };
  };

  if (!spec.compare) {
    return { rows: live.flatMap((slice) => slice.items.map((item) => make(slice, item))), missing };
  }

  // A k-way merge: each step takes the head that the DOMAIN's comparator puts
  // first, ties going to the earlier server. Each slice is consumed in its own
  // order, so no server's list can be reordered by this.
  const at = live.map(() => 0);
  const rows: UnionRow<T, R>[] = [];
  const total = live.reduce((n, s) => n + s.items.length, 0);
  for (let taken = 0; taken < total; taken++) {
    let pick = -1;
    for (let i = 0; i < live.length; i++) {
      if (at[i]! >= live[i]!.items.length) continue;
      if (pick < 0) { pick = i; continue; }
      // Strictly less only: an equal head leaves the earlier server in front.
      if (spec.compare(live[i]!.items[at[i]!]!, live[pick]!.items[at[pick]!]!) < 0) pick = i;
    }
    rows.push(make(live[pick]!, live[pick]!.items[at[pick]!]!));
    at[pick]!++;
  }
  return { rows, missing };
}

/** Every server contributing to a list, in `order`. Useful to a view that
 * needs a per-server section header or an "unreachable" notice. */
export function contributors<T>(slices: readonly ServerSlice<T>[]): ServerSlice<T>[] {
  return slices.slice().sort((a, b) => a.order - b.order);
}
