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

/** ONLY the changed fields, aligned with the server's COALESCE patch — a
 * field the user never touched must not ride along (an agent's concurrent
 * note/move bumps updated_at, not these, but sending an unchanged body would
 * still overwrite a concurrent body edit). `null` when there is nothing to
 * save or the draft is invalid. */
export function draftPatch(
  draft: IssueDraft,
  issue: { title?: string; body?: string },
): { title?: string; body?: string } | null {
  if (!draftValid(draft) || !draftDirty(draft, issue)) return null;
  const patch: { title?: string; body?: string } = {};
  if (draft.title !== (issue.title ?? '')) patch.title = draft.title.trim();
  if (draft.body !== (issue.body ?? '')) patch.body = draft.body;
  return patch;
}
