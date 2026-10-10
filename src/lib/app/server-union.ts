// One list over several servers (board #335 ②a-5).
//
// A union list is where two servers become visible at once, and it has to be
// a PROJECTION rather than a new source of truth: each server's own ordering
// rule still decides its rows, and this module only interleaves them. That is
// why it takes slices that are ALREADY ORDERED by their domain's own function
// (`projects.sortRows`, the sessions list's own order) — restating "open
// first, then by activity" here would be a second definition of a rule that
// already exists, and the two would drift.
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
   * The value the DOMAIN's own rule orders by, read from that domain's own
   * functions — not re-derived here. Higher sorts first; an array compares
   * element by element, so "open first, then newest activity" is
   * `[live ? 1 : 0, updatedMs]`.
   *
   * Omit it to keep each slice's given order and interleave by server only
   * (a list whose order is the server's answer, like tmux's session order).
   */
  rank?: (item: T, serverId: ServerId) => number | readonly number[];
}

const asList = (v: number | readonly number[]): readonly number[] => (typeof v === 'number' ? [v] : v);

/** Compare two rank values: higher first, element by element. */
function byRank(a: readonly number[], b: readonly number[]): number {
  for (let i = 0; i < Math.max(a.length, b.length); i++) {
    const d = (b[i] ?? 0) - (a[i] ?? 0);
    if (d) return d;
  }
  return 0;
}

/**
 * Interleave several servers' already-ordered rows into one list.
 *
 * With one contributing server the result is that server's own list in its own
 * order, with no tags — the Single-mode shape, unchanged.
 */
export function unionRows<T, R extends Ref>(
  slices: readonly ServerSlice<T>[],
  spec: UnionSpec<T, R>,
): UnionList<T, R> {
  const missing = slices.filter((s) => !s.ok).map((s) => s.serverId);
  const live = slices.filter((s) => s.ok);
  // A tag only once there is something to tell apart. An empty slice still
  // counts as a contributor: its server is on screen, and a list that grows a
  // tag column the moment that server's first row arrives would be worse than
  // one that has it from the start.
  const tagged = live.length > 1;
  const rows = live.flatMap((slice, sliceIndex) =>
    slice.items.map((item, itemIndex) => {
      const ref = spec.ref(slice.serverId, item);
      return {
        row: {
          serverId: slice.serverId,
          item,
          ref,
          key: refKey(ref),
          tag: tagged ? slice.name : null,
        } as UnionRow<T, R>,
        rank: spec.rank ? asList(spec.rank(item, slice.serverId)) : [],
        // Within one server the given order is authoritative; `order` breaks
        // ties between servers.
        tie: [slice.order, sliceIndex, itemIndex] as const,
      };
    }));

  rows.sort((a, b) => byRank(a.rank, b.rank)
    || a.tie[0] - b.tie[0]
    || a.tie[1] - b.tie[1]
    || a.tie[2] - b.tie[2]);

  return { rows: rows.map((r) => r.row), missing };
}

/** Every server contributing to a list, in `order`. Useful to a view that
 * needs a per-server section header or an "unreachable" notice. */
export function contributors<T>(slices: readonly ServerSlice<T>[]): ServerSlice<T>[] {
  return slices.slice().sort((a, b) => a.order - b.order);
}
