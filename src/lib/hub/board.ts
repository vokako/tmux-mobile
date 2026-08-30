/** Pure helpers for the board's issue DRAFT (board #11): opening an issue
 * edits a copy, and only an explicit Save persists it. Kept out of the
 * component so save/cancel/patch semantics are testable without a DOM.
 * Since board #15 the draft carries ALL FOUR editable fields — status and
 * assignee joined title/body, because a stray tap on a picker was changing
 * live state with no confirmation ("避免手动手滑随便一点就改变了状态"):
 * now every edit waits for the same ✓. */

export interface IssueDraft {
  title: string;
  body: string;
  status: string;
  assignee: string;
}

const FIELDS = ['title', 'body', 'status', 'assignee'] as const;

type IssueLike = { title?: string; body?: string; status?: string; assignee?: string };

/** The editable copy of an issue's fields. */
export const draftOf = (issue: IssueLike | null | undefined): IssueDraft => ({
  title: issue?.title ?? '',
  body: issue?.body ?? '',
  status: issue?.status ?? '',
  assignee: issue?.assignee ?? '',
});

/** Has the draft diverged from the stored issue? Raw comparison — whitespace
 * the user typed is an edit until they remove it. */
export const draftDirty = (draft: IssueDraft, issue: IssueLike): boolean =>
  FIELDS.some((f) => draft[f] !== (issue[f] ?? ''));

/** A draft the server would accept: a title is the one required field. */
export const draftValid = (draft: IssueDraft): boolean => draft.title.trim().length > 0;

/** ONLY the fields the USER changed, measured against the draft's own BASE —
 * never against the live issue (#11 review): after a concurrent refetch the
 * live body may already be an agent's newer text, and diffing against it
 * would send the user's stale copy of a field they never touched, silently
 * overwriting the agent. `null` when there is nothing to save or the draft
 * is invalid. */
export function draftPatch(
  draft: IssueDraft,
  base: IssueLike,
): Partial<Record<(typeof FIELDS)[number], string>> | null {
  if (!draftValid(draft) || !draftDirty(draft, base)) return null;
  const patch: Partial<Record<(typeof FIELDS)[number], string>> = {};
  for (const f of FIELDS) {
    if (draft[f] !== (base[f] ?? '')) patch[f] = f === 'title' ? draft[f].trim() : draft[f];
  }
  return patch;
}

/** Three-way rebase after a refetch (#11 review): the server moved while the
 * user was editing. Per field — UNTOUCHED (draft == base) follows the server
 * (the editor shows the agent's fresh text), TOUCHED keeps the user's edit
 * (their save then overwrites that one field, knowingly). The new base is
 * always the server's copy, so a later `draftPatch(draft, base)` still flags
 * exactly the touched fields. */
export function rebaseDraft(
  draft: IssueDraft,
  base: IssueDraft,
  server: IssueDraft,
): { draft: IssueDraft; base: IssueDraft } {
  const pick = (field: keyof IssueDraft) => (draft[field] === base[field] ? server[field] : draft[field]);
  const next = {} as IssueDraft;
  for (const f of FIELDS) next[f] = pick(f);
  return { draft: next, base: { ...server } };
}
