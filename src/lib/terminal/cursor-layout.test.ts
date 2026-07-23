import test from 'node:test';
import assert from 'node:assert/strict';
import {
  cellWidth,
  visualRowsOfLine,
  countVisualRows,
  computeCursorLayout,
} from './cursor-layout.ts';

// ── width model (must mirror xterm's default UnicodeV6 provider) ────────

test('cellWidth: ASCII 1, CJK 2, combining 0, emoji 1 (UnicodeV6!)', () => {
  assert.equal(cellWidth('a'), 1);
  assert.equal(cellWidth('中'), 2);
  assert.equal(cellWidth('ｱ'), 1); // halfwidth katakana FF71
  assert.equal(cellWidth('Ａ'), 2); // fullwidth FF21
  assert.equal(cellWidth('\u0301'), 0); // combining acute
  assert.equal(cellWidth('\u200b'), 0); // zero-width space
  // xterm's default provider is Unicode 6: emoji are width 1 there.
  assert.equal(cellWidth('😀'), 1);
  assert.equal(cellWidth('\u{20000}'), 2); // CJK ext B (plane 2 wide)
});

test('visualRowsOfLine: plain wrapping at cols', () => {
  assert.equal(visualRowsOfLine('', 40), 1);
  assert.equal(visualRowsOfLine('a'.repeat(40), 40), 1); // exactly full = 1 row
  assert.equal(visualRowsOfLine('a'.repeat(41), 40), 2);
  assert.equal(visualRowsOfLine('a'.repeat(120), 40), 3);
});

test('visualRowsOfLine: ANSI escapes take no cells', () => {
  const line = '\x1b[38;5;196m' + 'x'.repeat(40) + '\x1b[0m';
  assert.equal(visualRowsOfLine(line, 40), 1);
  const osc = '\x1b]0;title\x07' + 'x'.repeat(40);
  assert.equal(visualRowsOfLine(osc, 40), 1);
});

test('visualRowsOfLine: wide char that does not fit wraps whole', () => {
  // 39 narrow cells + one wide char: the wide char needs cols 40-41 → wraps.
  assert.equal(visualRowsOfLine('a'.repeat(39) + '中', 40), 2);
  // 38 narrow + wide fits exactly.
  assert.equal(visualRowsOfLine('a'.repeat(38) + '中', 40), 1);
});

test('countVisualRows: sums logical lines', () => {
  assert.equal(countVisualRows('', 40), 1);
  assert.equal(countVisualRows('a\nb', 40), 2);
  assert.equal(countVisualRows('a\n' + 'b'.repeat(41) + '\nc', 40), 4);
});

// ── layout: unwrapped content (must behave exactly like the old math) ───

test('cursor layout: content shorter than the pane pads to the bottom row', () => {
  // 24-row pane, freshly cleared: one prompt line, 23 trimmed trailing rows.
  const layout = computeCursorLayout('prompt', { x: 7, y: 0, w: 80, h: 24, t: 23 }, 24, 80);
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
  const layout = computeCursorLayout(content, { x: 0, y: 23, w: 80, h: 24, t: 0 }, 24, 80);
  assert.equal(layout.row, 24);
  assert.equal(layout.afterPad, '');
});

test('cursor layout: cursor below the last content line forces padding', () => {
  // Pane 24 rows, 10 content lines, cursor parked on pane row 12 (y=12):
  // the pad materializes rows 11..13 and the pane bottom.
  const content = Array.from({ length: 10 }, (_, i) => `l${i}`).join('\n');
  const layout = computeCursorLayout(content, { x: 0, y: 12, w: 80, h: 24, t: 14 }, 24, 80);
  assert.equal(layout.row, 13);
  assert.equal(layout.afterPad.length, 14);
});

// ── layout: wrapped content (the capture -J off-by-N regression) ────────
//
// Real repro: 40×10 pane running vi on a file whose middle line is 100
// chars. `capture-pane -J` joins the wrap, so the pane's 10 visual rows
// arrive as 8 logical lines; tmux reports the cursor at visual row y=4
// (end of the wrapped line). The old logical-line math padded 2 phantom
// blank rows (shearing the viewport up) and parked the cursor on the `~`
// line below the text.

const vi40x10 = [
  ' curtest.txt',
  '  1 short line 1',
  '2   ' + 'a'.repeat(100) + ' '.repeat(16), // 120 cells → 3 visual rows
  '  1 short line 3',
  '~',
  '~',
  ' <te/tmp/curtest.txt text │ 2:100 │ 66%',
  '"/tmp/curtest.txt" 3L, 127B',
].join('\n');

test('cursor layout: joined wrapped line — no phantom pad, cursor on its glyph', () => {
  const layout = computeCursorLayout(vi40x10, { x: 31, y: 4, w: 40, h: 10, t: 0 }, 10, 40);
  // 8 logical lines re-wrap to exactly the pane's 10 visual rows: no pad...
  assert.equal(layout.afterPad, '');
  // ...and the cursor lands on tmux's visual row (y+1 when rows == h).
  assert.equal(layout.row, 5);
});

test('cursor layout: full pane row is measurement-independent (y+1 + rows-h)', () => {
  // Whatever the content measures to, a full pane anchors to the bottom and
  // the cursor row must reduce to y + 1 (+ rows - h when dims disagree).
  const long = Array.from({ length: 30 }, () => 'z'.repeat(95)).join('\n');
  const layout = computeCursorLayout(long, { x: 0, y: 7, w: 40, h: 10, t: 0 }, 10, 40);
  assert.equal(layout.row, 8);
  assert.equal(layout.afterPad, '');
});

test('cursor layout: wrapped line in a short pane still pads correctly', () => {
  // Fresh shell: prompt + one echoed 60-char line in a 40-col, 10-row pane.
  // 3 logical lines but 4 visual rows → pad 10-4=6, not 10-3=7.
  const content = '$ echo xxx\n' + 'x'.repeat(60) + '\n$';
  const layout = computeCursorLayout(content, { x: 2, y: 3, w: 40, h: 10, t: 6 }, 10, 40);
  assert.equal(layout.afterPad, '\n'.repeat(6));
  assert.equal(layout.row, 4);
});

test('cursor layout: CJK wrapped line counts wide cells', () => {
  // 30 CJK chars = 60 cells → 2 visual rows at 40 cols.
  const content = '$ cat cjk\n' + '中'.repeat(30) + '\n$';
  const layout = computeCursorLayout(content, { x: 2, y: 3, w: 40, h: 10, t: 6 }, 10, 40);
  assert.equal(layout.afterPad, '\n'.repeat(6));
  assert.equal(layout.row, 4);
});
