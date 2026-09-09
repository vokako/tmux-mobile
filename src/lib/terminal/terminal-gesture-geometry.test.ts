import assert from 'node:assert/strict';
import test from 'node:test';
import { pointToCell, handleGrabOffset, snapHandleColumn } from './terminal-gesture-geometry.ts';

test('cell mapping floors at cell boundaries relative to the actual client origin', () => {
  const rect = { left: 100, top: 200 }, cell = { w: 8, h: 16 };
  assert.deepEqual(pointToCell(100, 200, rect, cell, 10, 5), { col: 0, row: 0 });
  assert.deepEqual(pointToCell(107.999, 215.999, rect, cell, 10, 5), { col: 0, row: 0 });
  assert.deepEqual(pointToCell(108, 216, rect, cell, 10, 5), { col: 1, row: 1 });
  assert.deepEqual(pointToCell(179.999, 279.999, rect, cell, 10, 5), { col: 9, row: 4 });
});

test('cell mapping clamps outside points to the current grid, not an overshoot cell', () => {
  const rect = { left: 100, top: 200 }, cell = { w: 8, h: 16 };
  assert.deepEqual(pointToCell(-1000, -1000, rect, cell, 10, 5), { col: 0, row: 0 });
  assert.deepEqual(pointToCell(1000, 1000, rect, cell, 10, 5), { col: 9, row: 4 });
  assert.deepEqual(pointToCell(1000, 1000, rect, cell, 1, 1), { col: 0, row: 0 });
});

test('fractional metrics and changed measurements are consumed on each call', () => {
  assert.deepEqual(pointToCell(10.75, 17, { left: 3.25, top: 4.5 }, { w: 7.5, h: 12.5 }, 20, 10),
    { col: 1, row: 1 });
  const rect = { left: 100, top: 200 };
  assert.deepEqual(pointToCell(119, 230, rect, { w: 8, h: 16 }, 10, 5), { col: 2, row: 1 });
  assert.deepEqual(pointToCell(119, 230, rect, { w: 10, h: 10 }, 10, 5), { col: 1, row: 3 });
  assert.deepEqual(pointToCell(119, 230, { left: 110, top: 210 }, { w: 8, h: 16 }, 10, 5),
    { col: 1, row: 1 });
});

test('grab compensation in both axes maps the first movement back to the same buffer cell', () => {
  const rect = { left: 100, top: 200 }, cell = { w: 8, h: 16 };
  const ep = { col: 3, row: 20 }, viewport = 18;
  const offset = handleGrabOffset(139, 257, ep, viewport, rect, cell);
  assert.deepEqual(offset, { dx: 11, dy: 17 });
  const mapped = pointToCell(139 - offset.dx, 257 - offset.dy, rect, cell, 10, 5);
  assert.deepEqual({ col: mapped.col, row: mapped.row + viewport }, ep);
  assert.deepEqual(handleGrabOffset(139, 257, ep, viewport + 1, rect, cell), { dx: 11, dy: 33 });
});

test('grab offsets preserve fractional and negative finger displacement', () => {
  assert.deepEqual(handleGrabOffset(39.5, 20.75, { col: 4, row: 12 }, 11,
    { left: 3.25, top: 4.5 }, { w: 7.5, h: 12.5 }), { dx: 2.5, dy: -2.5 });
});

test('edge snapping includes the threshold but excludes points just inside the text', () => {
  assert.equal(snapHandleColumn(10, 1, 100, 8, 10, 30), 0);
  assert.equal(snapHandleColumn(10.001, 1, 100, 8, 10, 30), 1);
  assert.equal(snapHandleColumn(59.999, 6, 100, 8, 10, 30), 6);
  assert.equal(snapHandleColumn(60, 6, 100, 8, 10, 30), 9);
  assert.equal(snapHandleColumn(-20, 0, 100, 8, 10, 30), 0);
  assert.equal(snapHandleColumn(120, 9, 100, 8, 10, 30), 9);
});

test('edge snapping uses the larger cell-relative zone and the supplied scrollbar width', () => {
  assert.equal(snapHandleColumn(24, 1, 200, 40, 4, 30), 0);
  assert.equal(snapHandleColumn(24.001, 1, 200, 40, 4, 30), 1);
  assert.equal(snapHandleColumn(146, 2, 200, 40, 4, 30), 3);
  assert.equal(snapHandleColumn(61, 6, 100, 8, 10, 12), 6);
});

test('the left edge keeps priority when snap zones overlap in a narrow box', () => {
  assert.equal(snapHandleColumn(10, 2, 50, 8, 5, 30), 0);
  assert.equal(snapHandleColumn(10.001, 2, 50, 8, 5, 30), 4);
});
