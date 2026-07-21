import test from 'node:test';
import assert from 'node:assert/strict';
import { selStart, selEnd } from './selection-model.ts';
import { countLines, computeCursorLayout } from './cursor-layout.ts';

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

test('countLines counts newline-separated lines without splitting', () => {
  assert.equal(countLines(''), 1);
  assert.equal(countLines('a'), 1);
  assert.equal(countLines('a\nb'), 2);
  assert.equal(countLines('a\nb\n'), 3);
});

test('cursor layout: content shorter than the pane pads to the bottom row', () => {
  // 24-row pane, freshly cleared: one prompt line, 23 trimmed trailing rows.
  const layout = computeCursorLayout('prompt', { x: 7, y: 0, w: 80, h: 24, t: 23 }, 24);
  // Cursor stays on the first line...
  assert.equal(layout.row, 1);
  // ...and the buffer is padded so it ends at the pane's bottom row.
  assert.equal(layout.afterPad, '\n'.repeat(23));
});

test('cursor layout: long scrollback keeps the cursor inside the viewport', () => {
  // 100 content lines, 24-row pane, no trimmed rows, cursor on the pane's
  // last row (y = h-1). The write scrolls; the returned row is viewport-
  // relative (1-based) and must be the bottom row.
  const content = Array.from({ length: 100 }, (_, i) => `line${i}`).join('\n');
  const layout = computeCursorLayout(content, { x: 0, y: 23, w: 80, h: 24, t: 0 }, 24);
  assert.equal(layout.row, 24);
  assert.equal(layout.afterPad, '');
});

test('cursor layout: cursor below the last content line forces padding', () => {
  // Pane 24 rows, 10 content lines, cursor parked on pane row 12 (y=12):
  // paneStart = 0, cursorLine = 12 → needs lines 11..13 created.
  const content = Array.from({ length: 10 }, (_, i) => `l${i}`).join('\n');
  const layout = computeCursorLayout(content, { x: 0, y: 12, w: 80, h: 24, t: 14 }, 24);
  assert.equal(layout.row, 13);
  // afterPad restores both the missing cursor rows and the pane bottom.
  assert.equal(layout.afterPad.length, 14);
});
