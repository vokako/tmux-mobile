import test from 'node:test';
import assert from 'node:assert/strict';
import { draftOf, draftDirty, draftValid, draftPatch, rebaseDraft } from './board.ts';

test('an issue draft saves explicitly, cancels cleanly, and patches only what changed (board #11)', () => {
  const issue = { title: 'fix login', body: 'step 2 breaks' };
  const clean = draftOf(issue);
  assert.deepEqual(clean, { title: 'fix login', body: 'step 2 breaks' });
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
  const both = { title: '  new title ', body: 'b' };
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

test('rebase: the agent\u2019s concurrent edit survives a title-only save (#11 review)', () => {
  // The lead's exact scenario: user opens {A, B}, edits ONLY the title;
  // an agent writes a new body B' server-side; a refetch (after a move or
  // note) brings it in.
  const base = { title: 'A', body: 'B' };
  const draft = { title: 'A2', body: 'B' };
  const server = { title: 'A', body: 'B-agent' };
  const r = rebaseDraft(draft, base, server);
  assert.equal(r.draft.body, 'B-agent', 'the untouched body follows the server');
  assert.equal(r.draft.title, 'A2', 'the touched title keeps the user\u2019s edit');
  assert.deepEqual(r.base, server, 'the base catches up to the server');
  // The save then ships ONLY the touched field — the agent's body is safe.
  assert.deepEqual(draftPatch(r.draft, r.base), { title: 'A2' });

  // Both sides touched the same field: the user's copy wins locally and the
  // patch declares it — overwriting is now a choice, not an accident.
  const clash = rebaseDraft({ title: 'A2', body: 'B' }, base, { title: 'A3', body: 'B' });
  assert.equal(clash.draft.title, 'A2');
  assert.deepEqual(draftPatch(clash.draft, clash.base), { title: 'A2' });

  // Nothing touched: draft and base both become the server copy — clean.
  const clean = rebaseDraft(base, base, server);
  assert.deepEqual(clean.draft, server);
  assert.equal(draftPatch(clean.draft, clean.base), null);

  // After a successful save the server carries the saved text: the rebase
  // normalizes the draft to clean without an explicit reset.
  const saved = rebaseDraft({ title: 'A2', body: 'B' }, base, { title: 'A2', body: 'B' });
  assert.equal(draftPatch(saved.draft, saved.base), null);
});
