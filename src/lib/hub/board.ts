/** Pure helpers for the board's issue DRAFT (board #11): opening an issue
 * edits a copy, and only an explicit Save persists it. Kept out of the
 * component so save/cancel/patch semantics are testable without a DOM. */

export interface IssueDraft {
  title: string;
  body: string;
}

/** The editable copy of an issue's text fields. */
export const draftOf = (issue: { title?: string; body?: string } | null | undefined): IssueDraft => ({
  title: issue?.title ?? '',
  body: issue?.body ?? '',
});

/** Has the draft diverged from the stored issue? Raw comparison — whitespace
 * the user typed is an edit until they remove it. */
export const draftDirty = (draft: IssueDraft, issue: { title?: string; body?: string }): boolean =>
  draft.title !== (issue.title ?? '') || draft.body !== (issue.body ?? '');

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
  base: { title?: string; body?: string },
): { title?: string; body?: string } | null {
  if (!draftValid(draft) || !draftDirty(draft, base)) return null;
  const patch: { title?: string; body?: string } = {};
  if (draft.title !== (base.title ?? '')) patch.title = draft.title.trim();
  if (draft.body !== (base.body ?? '')) patch.body = draft.body;
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
  return {
    draft: { title: pick('title'), body: pick('body') },
    base: { ...server },
  };
}
