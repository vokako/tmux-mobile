import test from 'node:test';
import assert from 'node:assert/strict';
import { draftOf, draftDirty, draftValid, draftPatch, rebaseDraft } from './board.ts';

// Since board #15 a draft carries all four editable fields; these helpers
// build the full shape from the short form the assertions speak.
const d = (over: Partial<{ title: string; body: string; status: string; assignee: string }>) => ({
  title: '', body: '', status: '', assignee: '', ...over,
});

test('an issue draft saves explicitly, cancels cleanly, and patches only what changed (board #11)', () => {
  const issue = { title: 'fix login', body: 'step 2 breaks' };
  const clean = draftOf(issue);
  assert.deepEqual(clean, d({ title: 'fix login', body: 'step 2 breaks' }));
  assert.ok(!draftDirty(clean, issue), 'a fresh draft is not dirty');
  assert.equal(draftPatch(clean, issue), null, 'nothing changed, nothing to save');

  // Editing one field patches ONE field — the untouched body must not ride
  // along and overwrite a concurrent edit (the server COALESCEs).
  const retitled = { ...clean, title: 'fix login flow' };
  assert.ok(draftDirty(retitled, issue));
  assert.deepEqual(draftPatch(retitled, issue), { title: 'fix login flow' });

  const rebodied = { ...clean, body: 'root cause: session cookie' };
  assert.deepEqual(draftPatch(rebodied, issue), { body: 'root cause: session cookie' });

  // Both changed → both sent; the title is trimmed on the way out.
  const both = d({ title: '  new title ', body: 'b' });
  assert.deepEqual(draftPatch(both, issue), { title: 'new title', body: 'b' });

  // A blank title is invalid: the save stays disabled, the draft survives.
  const blank = { ...clean, title: '   ' };
  assert.ok(!draftValid(blank));
  assert.equal(draftPatch(blank, issue), null);

  // Cancel = draftOf(sel) again: identity with the stored issue.
  assert.ok(!draftDirty(draftOf(issue), issue));

  // Whitespace typed IS an edit until removed (dirty), but body-only spaces
  // still patch verbatim — the body is prose, not an id.
  const spaced = { ...clean, body: 'step 2 breaks ' };
  assert.ok(draftDirty(spaced, issue));
  assert.deepEqual(draftPatch(spaced, issue), { body: 'step 2 breaks ' });
});

test('status and assignee are DRAFT fields too — a stray tap changes nothing until ✓ (board #15)', () => {
  const issue = { title: 'T', body: 'B', status: 'todo', assignee: '' };
  const clean = draftOf(issue);
  assert.deepEqual(clean, d({ title: 'T', body: 'B', status: 'todo' }));
  assert.ok(!draftDirty(clean, issue));

  // Sliding the status marks the draft dirty and patches ONE field.
  const moved = { ...clean, status: 'doing' };
  assert.ok(draftDirty(moved, issue), 'a status change is an edit awaiting the ✓');
  assert.deepEqual(draftPatch(moved, issue), { status: 'doing' });

  // Picking an assignee likewise — the dispatch happens at SAVE time.
  const assigned = { ...clean, assignee: 'builder' };
  assert.deepEqual(draftPatch(assigned, issue), { assignee: 'builder' });

  // Cancel restores both: identity again.
  assert.ok(!draftDirty(draftOf(issue), issue));

  // The rebase covers the new fields: an agent's concurrent take (status +
  // assignee moved server-side) flows into an untouched draft…
  const server = d({ title: 'T', body: 'B', status: 'doing', assignee: 'builder-2' });
  const flowed = rebaseDraft(clean, clean, server);
  assert.deepEqual(flowed.draft, server);
  assert.equal(draftPatch(flowed.draft, flowed.base), null);

  // …while the user's own pending status keeps priority on a clash, and the
  // patch declares exactly that field.
  const clash = rebaseDraft({ ...clean, status: 'review' }, clean, server);
  assert.equal(clash.draft.status, 'review', 'the touched status keeps the user\u2019s pick');
  assert.equal(clash.draft.assignee, 'builder-2', 'the untouched assignee follows the server');
  assert.deepEqual(draftPatch(clash.draft, clash.base), { status: 'review' });
});

test('rebase: the agent\u2019s concurrent edit survives a title-only save (#11 review)', () => {
  // The lead's exact scenario: user opens {A, B}, edits ONLY the title;
  // an agent writes a new body B' server-side; a refetch (after a move or
  // note) brings it in.
  const base = d({ title: 'A', body: 'B' });
  const draft = d({ title: 'A2', body: 'B' });
  const server = d({ title: 'A', body: 'B-agent' });
  const r = rebaseDraft(draft, base, server);
  assert.equal(r.draft.body, 'B-agent', 'the untouched body follows the server');
  assert.equal(r.draft.title, 'A2', 'the touched title keeps the user\u2019s edit');
  assert.deepEqual(r.base, server, 'the base catches up to the server');
  // The save then ships ONLY the touched field — the agent's body is safe.
  assert.deepEqual(draftPatch(r.draft, r.base), { title: 'A2' });

  // Both sides touched the same field: the user's copy wins locally and the
  // patch declares it — overwriting is now a choice, not an accident.
  const clash = rebaseDraft(d({ title: 'A2', body: 'B' }), base, d({ title: 'A3', body: 'B' }));
  assert.equal(clash.draft.title, 'A2');
  assert.deepEqual(draftPatch(clash.draft, clash.base), { title: 'A2' });

  // Nothing touched: draft and base both become the server copy — clean.
  const clean = rebaseDraft(base, base, server);
  assert.deepEqual(clean.draft, server);
  assert.equal(draftPatch(clean.draft, clean.base), null);

  // After a successful save the server carries the saved text: the rebase
  // normalizes the draft to clean without an explicit reset.
  const saved = rebaseDraft(d({ title: 'A2', body: 'B' }), base, d({ title: 'A2', body: 'B' }));
  assert.equal(draftPatch(saved.draft, saved.base), null);
});
