// A server-persisted, whole-array list with clobber/race guards.
//
// Extracted from Files.svelte, where bookmarks and recent-files carried two
// hand-written copies of the same discipline. Both lists are last-writer-wins
// on the server, and reads/writes are concurrent RPCs (a response can be
// answered from state that predates a later write). Two rules keep a client
// from wiping data:
//
//   1. Never persist before the first successful load — a write of the
//      in-memory default [] would erase the server list.
//   2. A fetch response must never overwrite local mutations made while it
//      was in flight — each local mutation bumps a generation counter, and
//      fetch continuations only assign if their generation is still current.
//
// The lazy first load is single-flighted so two rapid mutations before the
// first load don't each read-modify-write from a stale base.
//
// Framework-free: the host mirrors `onChange(items)` into its own $state.
// The deeper fix (server-side merge semantics) is tracked in
// docs/unresolved.md ("Prefs/bookmarks: cross-client last-writer-wins").

export interface PersistedListDeps<T> {
  /** Read the server copy. Errors propagate to load() callers. */
  fetch: () => Promise<T[]>;
  /** Write the whole list. Fire-and-forget; errors are swallowed. */
  persist: (items: T[]) => Promise<unknown>;
  /** Host mirror (e.g. assign into a $state array). */
  onChange: (items: T[]) => void;
}

export function createPersistedList<T>({ fetch, persist, onChange }: PersistedListDeps<T>) {
  let items: T[] = [];
  let loaded = false;
  let gen = 0;
  let loadPromise: Promise<void> | null = null;

  function load(): Promise<void> {
    loadPromise ??= (async () => {
      const genAtStart = gen;
      const fetched = await fetch(); // throws propagate to callers
      if (genAtStart === gen) {
        items = fetched;
        onChange(items);
      }
      loaded = true;
    })().finally(() => { loadPromise = null; });
    return loadPromise;
  }

  /**
   * Apply a local mutation and persist it. Rule 1: if the first load hasn't
   * landed, fetch-then-merge; if that fetch fails, skip persisting entirely
   * rather than wipe the server list. Returns false when skipped.
   */
  async function mutate(updater: (current: T[]) => T[]): Promise<boolean> {
    if (!loaded) {
      try { await load(); } catch { return false; }
    }
    gen++; // rule 2: invalidate any fetch still in flight
    items = updater(items);
    onChange(items);
    persist(items).catch(() => {});
    return true;
  }

  return {
    load,
    mutate,
    get items() { return items; },
    get loaded() { return loaded; },
  };
}
