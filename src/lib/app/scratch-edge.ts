/**
 * Which edge the scratch panel docks to (board #324, re-chosen in #326).
 *
 * `bottom` or `right`. It was `bottom | left` until the owner asked for the
 * other side (2026-10-09: "这个 terminal 应该可以显示在下方或右侧"), so a
 * stored `left` MIGRATES to `right` — the same axis, the same stored size
 * (`tmux_scratch_w`), the mirror image. Reading it writes the migration back
 * once, so the old value cannot resurface later.
 *
 * Pure and storage-injected: the migration is the kind of thing that silently
 * stops working (a reader that only recognised 'right' would have thrown
 * every existing user back to the bottom edge), so it is a function with a
 * test rather than a ternary inside App.
 */
export type ScratchEdge = 'bottom' | 'right';
export const SCRATCH_EDGE_KEY = 'tmux_scratch_edge';

type Store = Pick<Storage, 'getItem' | 'setItem'>;

export function readScratchEdge(store: Store): ScratchEdge {
  const stored = store.getItem(SCRATCH_EDGE_KEY);
  // 'left' is the pre-#326 name for the vertical dock; anything unknown (and
  // the unset default) is the bottom edge.
  const edge: ScratchEdge = stored === 'right' || stored === 'left' ? 'right' : 'bottom';
  if (stored !== null && stored !== edge) store.setItem(SCRATCH_EDGE_KEY, edge);
  return edge;
}

export function writeScratchEdge(store: Store, edge: ScratchEdge): void {
  store.setItem(SCRATCH_EDGE_KEY, edge);
}
