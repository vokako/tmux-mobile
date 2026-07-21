import test from 'node:test';
import assert from 'node:assert/strict';
import { selStart, selEnd } from './selection-model.ts';

test('selStart/selEnd order anchor and head without mutating them', () => {
  const forward = { anchor: { row: 1, col: 2 }, head: { row: 3, col: 0 } };
  assert.equal(selStart(forward), forward.anchor);
  assert.equal(selEnd(forward), forward.head);

  // Head dragged above the anchor: endpoints flip, no swap bookkeeping.
  const backward = { anchor: { row: 3, col: 0 }, head: { row: 1, col: 2 } };
  assert.equal(selStart(backward), backward.head);
  assert.equal(selEnd(backward), backward.anchor);

  // Same row: column decides.
  const sameRow = { anchor: { row: 2, col: 9 }, head: { row: 2, col: 4 } };
  assert.equal(selStart(sameRow), sameRow.head);
  assert.equal(selEnd(sameRow), sameRow.anchor);

  assert.equal(selStart(null), null);
  assert.equal(selEnd(null), null);
});
