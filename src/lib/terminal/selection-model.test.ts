import test from 'node:test';
import assert from 'node:assert/strict';
import { selStart, selEnd, selContains, selLength, selForDrag, selFromExclusive, wordBounds } from './selection-model.ts';
import type { Selection } from './selection-model.ts';

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

test('contains handles absent and equal-endpoint selections inclusively', () => {
  assert.equal(selContains(null, 1, 2), false);
  assert.equal(selContains(undefined, 1, 2), false);
  const selection = { anchor: { row: 1, col: 2 }, head: { row: 1, col: 2 } };
  assert.equal(selContains(selection, 1, 2), true);
  for (const [row, col] of [[0, 2], [2, 2], [1, 1], [1, 3]]) {
    assert.equal(selContains(selection, row!, col!), false);
  }
});

test('contains keeps the same row and column boundaries in either direction', () => {
  const a = { row: 1, col: 3 }, b = { row: 3, col: 2 };
  for (const selection of [{ anchor: a, head: b }, { anchor: b, head: a }]) {
    for (const [row, col, expected] of [
      [0, 5, false], [4, 0, false], [1, 2, false], [1, 3, true],
      [2, 0, true], [2, 999, true], [3, 2, true], [3, 3, false],
    ] as const) assert.equal(selContains(selection, row, col), expected);
  }
  const sameRow = { anchor: { row: 5, col: 8 }, head: { row: 5, col: 2 } };
  for (const col of [2, 5, 8]) assert.ok(selContains(sameRow, 5, col));
  for (const col of [1, 9]) assert.equal(selContains(sameRow, 5, col), false);
});

test('length is inclusive, row-aware and independent of endpoint direction', () => {
  const a = { row: 1, col: 3 }, b = { row: 3, col: 2 };
  assert.equal(selLength({ anchor: a, head: b }, 10), 20);
  assert.equal(selLength({ anchor: b, head: a }, 10), 20);
  assert.equal(selLength({ anchor: a, head: a }, 10), 1);
  assert.equal(selLength({ anchor: a, head: { row: 1, col: 9 } }, 10), 7);
});

test('length retains its one-cell floor for coordinates spanning a narrower resized grid', () => {
  assert.equal(selLength({ anchor: { row: 0, col: 9 }, head: { row: 1, col: 1 } }, 4), 1);
});

test('grab makes the selected geometric endpoint the head and copies both points', () => {
  const a = { row: 1, col: 3 }, b = { row: 3, col: 2 };
  for (const selection of [{ anchor: a, head: b }, { anchor: b, head: a }]) {
    for (const which of ['start', 'end'] as const) {
      const before = structuredClone(selection);
      const grabbed = selForDrag(selection, which);
      assert.deepEqual(grabbed, which === 'start' ? { anchor: b, head: a } : { anchor: a, head: b });
      assert.notEqual(grabbed.anchor, a);
      assert.notEqual(grabbed.anchor, b);
      assert.notEqual(grabbed.head, a);
      assert.notEqual(grabbed.head, b);
      assert.deepEqual(selection, before, 'grab never mutates the original model');
    }
  }
});

test('the stationary endpoint remains fixed through repeated crossings', () => {
  const initial = { anchor: { row: 3, col: 10 }, head: { row: 3, col: 14 } };
  for (const which of ['start', 'end'] as const) {
    let drag: Selection = selForDrag(initial, which);
    const fixed = drag.anchor;
    assert.deepEqual(fixed, which === 'start' ? initial.head : initial.anchor);
    for (const head of [{ row: 4, col: 20 }, { row: 2, col: 0 }, { ...fixed }, { row: 5, col: 1 }]) {
      // Terminal's unchanged moveHead writes only head, never a sorted endpoint.
      drag = { anchor: drag.anchor, head };
      assert.equal(drag.anchor, fixed);
      assert.ok(selStart(drag) === fixed || selEnd(drag) === fixed);
    }
  }
});

test('grabbing an equal-endpoint selection still produces independent points', () => {
  const point = { row: 4, col: 8 };
  const result = selForDrag({ anchor: point, head: point }, 'start');
  assert.deepEqual(result, { anchor: point, head: point });
  assert.notEqual(result.anchor, result.head);
});

test('exclusive input converts once without mutating or reordering the source', () => {
  const pos = { start: { x: 3, y: 1 }, end: { x: 9, y: 4 } };
  const before = structuredClone(pos);
  assert.deepEqual(selFromExclusive(pos), { anchor: { row: 1, col: 3 }, head: { row: 4, col: 8 } });
  assert.deepEqual(pos, before);
  assert.deepEqual(selFromExclusive({ start: { x: 8, y: 3 }, end: { x: 2, y: 1 } }),
    { anchor: { row: 3, col: 8 }, head: { row: 1, col: 1 } });
});

test('exclusive end.x=0 keeps the old next-row col-0 clamp', () => {
  assert.deepEqual(selFromExclusive({ start: { x: 3, y: 1 }, end: { x: 0, y: 2 } }),
    { anchor: { row: 1, col: 3 }, head: { row: 2, col: 0 } });
});

test('word scan handles missing lines, whitespace and positions beyond the text', () => {
  for (const text of [undefined, null, '', ' ', '\t', '\u00a0']) {
    assert.deepEqual(wordBounds(text, 0), { start: 0, end: 1 });
  }
  assert.deepEqual(wordBounds(undefined, 9), { start: 9, end: 10 });
  assert.deepEqual(wordBounds('abc', 3), { start: 3, end: 4 });
  assert.deepEqual(wordBounds('abc', 8), { start: 8, end: 9 });
  assert.deepEqual(wordBounds('abc\tdef', 3), { start: 3, end: 4 });
});

test('word scan splits on whitespace, not punctuation or path separators', () => {
  for (const col of [2, 6, 11]) assert.deepEqual(wordBounds('  foo/bar.js next', col), { start: 2, end: 12 });
  assert.deepEqual(wordBounds('  foo/bar.js next', 14), { start: 13, end: 17 });
});

test('word scan preserves UTF-16 string indexing, not terminal-cell width', () => {
  assert.deepEqual(wordBounds('a\u4e2d\u6587 b', 2), { start: 0, end: 3 });
  assert.deepEqual(wordBounds('a\ud83d\ude42 b', 2), { start: 0, end: 3 });
  assert.deepEqual(wordBounds('e\u0301 x', 1), { start: 0, end: 2 });
  assert.deepEqual(wordBounds('a\u00a0b', 2), { start: 2, end: 3 });
});
