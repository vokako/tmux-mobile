import test from 'node:test';
import assert from 'node:assert/strict';
import { draftOf, draftDirty, draftValid, draftPatch } from './board.ts';

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
