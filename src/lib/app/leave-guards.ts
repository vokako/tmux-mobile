// Leave guards for a server switch (board 315).
//
// An in-place switch DESTROYS every server-bound component, so an unsaved
// editor or a dirty agent draft would be thrown away without a word. Each
// such component already owns its exit guard (Files' discard dialog, the
// Agents editor's leave confirmation); it registers it here, and the switch
// asks them all BEFORE anything changes. There is no second confirmation
// flow: the question is the component's own, in its own dialog.
//
// A guard on a page the user is not looking at is asked on THAT page: the
// switch reveals it first (`reveal(page)`), so the dialog is seen next to
// what it would discard, and a cancel leaves the user looking at it.
//
// Framework-free and module-level: the instances live in different subtrees
// (the Files page, the Hub drawer's Files, the Agents page on the rail or
// inside Settings), and threading a callback through each host would be
// plumbing that a new host forgets.

export interface LeaveGuard {
  /** The page whose layer holds this instance ('' = always visible). */
  page: string;
  /** Something would be lost, or a mutation is still in flight. */
  dirty(): boolean;
  /** Ask in the component's own dialog. Resolves true to leave (a pending
   * save/delete has finished first), false to stay. */
  ask(): Promise<boolean>;
}

const guards = new Set<LeaveGuard>();

/** Register from a component's $effect; the returned cleanup unregisters. */
export function registerLeaveGuard(guard: LeaveGuard): () => void {
  guards.add(guard);
  return () => { guards.delete(guard); };
}

/** Ask every dirty guard in turn, one dialog at a time. A guard that left
 * (its component unmounted) while an earlier one was asking is skipped. */
export async function confirmLeave(reveal: (page: string) => Promise<void>): Promise<boolean> {
  for (const guard of [...guards]) {
    if (!guards.has(guard) || !guard.dirty()) continue;
    if (guard.page) await reveal(guard.page);
    if (!guards.has(guard)) continue;
    if (!(await guard.ask())) return false;
  }
  return true;
}

/** Test seam: the registry is module state. */
export function clearLeaveGuardsForTests(): void { guards.clear(); }
