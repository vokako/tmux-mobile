export interface FileViewState<F> {
  view: string;
  currentFile: F | null;
}

export function directoryLoadState<F>(
  { view, currentFile }: FileViewState<F>,
  purpose: string,
): FileViewState<F> {
  if (purpose !== 'navigate') return { view, currentFile };
  return { view: 'list', currentFile: null };
}

/** Leaving the current view: 'go' — nothing to lose, move now; 'ask' — the
 *  editor holds unsaved text, so the move waits behind the discard dialog.
 *  EVERY path out of the editor consults this (the back button, a session
 *  switch, the cwd follow, a drawer jump) — three of them used to bypass it
 *  and silently dropped the edits (review, 2026-09-03). */
export type LeaveDecision = 'go' | 'ask';
export interface EditorGuard { view: string; edited: boolean }
export function leaveDecision({ view, edited }: EditorGuard): LeaveDecision {
  return view === 'edit' && edited ? 'ask' : 'go';
}

/** One step of the follow-the-real-cwd rule. `reported` is what fs_cwd said;
 *  `lastSourceDir` is the last real cwd we acted on. The returned
 *  `lastSourceDir` is committed BEFORE the user answers: a cancelled follow
 *  is skipped for that event, never queued — the same cwd will not ask again
 *  until it changes. */
export interface CwdFollowStep { lastSourceDir: string; move: 'none' | LeaveDecision }
export function cwdFollowStep(reported: string, lastSourceDir: string, guard: EditorGuard): CwdFollowStep {
  if (!reported || reported === lastSourceDir) return { lastSourceDir, move: 'none' };
  return { lastSourceDir: reported, move: leaveDecision(guard) };
}
