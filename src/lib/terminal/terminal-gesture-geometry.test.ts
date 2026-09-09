import assert from 'node:assert/strict';
import test from 'node:test';
import { pointToCell, handleGrabOffset, snapHandleColumn, selectionView, hitSelectionHandle } from './terminal-gesture-geometry.ts';

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

test('selection projection orders endpoints and keeps exact cell-corner anchors', () => {
  const a = { row: 22, col: 2 }, b = { row: 22, col: 5 };
  const cell = { w: 8, h: 16 }, viewport = { top: 20, rows: 5, cols: 10, width: 200 };
  const view = selectionView({ anchor: a, head: b }, cell, viewport);
  assert.deepEqual(view, {
    startX: 16, startY: 32, endX: 48, endY: 48,
    toolbarX: 48, toolbarY: 10, toolbarBelow: false,
    startInView: true, endInView: true, toolbarVisible: true,
    cellH: 16, startDotShiftX: 0, endDotShiftX: 0,
    startAtLeftEdge: false, endAtRightEdge: false,
  });
  assert.deepEqual(selectionView({ anchor: b, head: a }, cell, viewport), view);
});

test('each offscreen endpoint hides independently and both hidden endpoints hide the toolbar', () => {
  const cell = { w: 8, h: 16 }, viewport = { top: 20, rows: 5, cols: 10, width: 200 };
  const view = (start: number, end: number) => selectionView(
    { anchor: { row: start, col: 2 }, head: { row: end, col: 5 } }, cell, viewport);
  const leading = view(19, 22);
  assert.equal(leading.startInView, false);
  assert.equal(leading.endInView, true);
  assert.equal(leading.toolbarBelow, true);
  assert.equal(leading.toolbarY, 70);
  const trailing = view(22, 25);
  assert.equal(trailing.startInView, true);
  assert.equal(trailing.endInView, false);
  assert.equal(trailing.toolbarVisible, true);
  const outside = view(19, 25);
  assert.equal(outside.toolbarVisible, false);
  assert.equal(outside.toolbarX, 48, 'the old clamp still runs for a hidden toolbar');
  assert.equal(outside.toolbarY, 0);
  assert.equal(view(20, 24).startInView, true);
  assert.equal(view(20, 24).endInView, true);
});

test('an unmeasured toolbar has only a provisional anchor until its border-box size arrives', () => {
  const cell = { w: 10, h: 10 }, viewport = { top: 0, rows: 10, cols: 20, width: 200 };
  const atThreshold = selectionView({ anchor: { row: 3, col: 4 }, head: { row: 3, col: 6 } }, cell, viewport);
  assert.equal(atThreshold.toolbarY, 8);
  assert.equal(atThreshold.toolbarBelow, false);
  const below = selectionView({ anchor: { row: 2, col: 4 }, head: { row: 15, col: 6 } }, cell, viewport);
  assert.equal(below.toolbarBelow, true);
  assert.equal(below.toolbarY, 122);
});

test('row-3 Copy flips using its rendered height, preserving the handle clearance (#143)', () => {
  const view = selectionView({ anchor: { row: 3, col: 10 }, head: { row: 3, col: 14 } },
    { w: 8, h: 16 }, { top: 0, rows: 47, cols: 46, width: 390, height: 762 }, 42);
  assert.equal(view.toolbarY, 86);
  assert.equal(view.toolbarBelow, true);
  assert.equal(view.toolbarY - view.endY, 22);
});

test('the rendered top, not the anchor, uses the strict eight-pixel flip boundary (#143)', () => {
  const range = { anchor: { row: 5, col: 2 }, head: { row: 5, col: 5 } };
  const viewport = { top: 0, rows: 10, cols: 20, width: 200, height: 200 };
  const at = selectionView(range, { w: 8, h: 16 }, viewport, 50);
  assert.equal(at.toolbarBelow, false);
  assert.equal(at.toolbarY - 50, 8);
  const over = selectionView(range, { w: 8, h: 16 }, viewport, 50.25);
  assert.equal(over.toolbarBelow, true, 'font/locale size changes are measured, not fixed at 42px');
});

test('a long selection flips below its leading handle when there is no room after its end (#143)', () => {
  const viewport = { top: 20, rows: 47, cols: 46, width: 390, height: 762 };
  const view = selectionView({ anchor: { row: 23, col: 10 }, head: { row: 66, col: 14 } },
    { w: 8, h: 16 }, viewport, 42);
  assert.equal(view.toolbarBelow, true);
  assert.equal(view.toolbarY, 86);
  assert.ok(view.toolbarY + 42 < view.endY - 16 - 22, 'both handles remain clear');
});

test('with only the trailing handle visible, an overflowing below placement flips above it (#143)', () => {
  const view = selectionView({ anchor: { row: 19, col: 10 }, head: { row: 66, col: 14 } },
    { w: 8, h: 16 }, { top: 20, rows: 47, cols: 46, width: 390, height: 762 }, 42);
  assert.equal(view.toolbarBelow, false);
  assert.equal(view.toolbarY, 714);
  assert.equal(view.toolbarY + 22, view.endY - 16);
});

test('when neither clearance fits, the toolbar remains within a short viewport (#143)', () => {
  for (const height of [64, 48]) {
    const view = selectionView({ anchor: { row: 0, col: 2 }, head: { row: 3, col: 5 } },
      { w: 8, h: 16 }, { top: 0, rows: 4, cols: 20, width: 200, height }, 42);
    const top = view.toolbarY - (view.toolbarBelow ? 0 : 42);
    const edge = height === 64 ? 8 : 3;
    assert.ok(top >= edge);
    assert.ok(top + 42 <= height - edge);
  }
});

test('edge dots shift inward while stems and horizontal toolbar clamping stay unchanged', () => {
  const selection = { anchor: { row: 2, col: 0 }, head: { row: 2, col: 9 } };
  const view = selectionView(selection, { w: 10, h: 20 }, { top: 0, rows: 10, cols: 10, width: 100 });
  assert.equal(view.startX, 0);
  assert.equal(view.endX, 100);
  assert.equal(view.startDotShiftX, 6);
  assert.equal(view.endDotShiftX, -6);
  assert.equal(view.toolbarX, 50);
  const narrow = selectionView(selection, { w: 10, h: 20 }, { top: 0, rows: 10, cols: 10, width: 80 });
  assert.equal(narrow.toolbarX, 48);
});

test('projection consumes changed viewport and fractional metrics without caching', () => {
  const selection = { anchor: { row: 11, col: 1 }, head: { row: 13, col: 6 } };
  const cell = { w: 7.5, h: 12.5 }, viewport = { top: 10, rows: 5, cols: 8, width: 200 };
  const view = selectionView(selection, cell, viewport);
  assert.equal(view.startX, 7.5);
  assert.equal(view.startY, 12.5);
  assert.equal(view.endX, 52.5);
  assert.equal(view.endY, 50);
  assert.equal(view.toolbarY, 72);
  const moved = selectionView(selection, cell, { ...viewport, top: 11 });
  assert.equal(moved.startY, 0);
  assert.equal(moved.endY, 37.5);
  assert.equal(moved.toolbarY, 59.5);
});

test('capsule hit bounds include their edges and use client-relative coordinates', () => {
  const selection = { anchor: { row: 2, col: 5 }, head: { row: 4, col: 10 } };
  const rect = { left: 100, top: 200, width: 200 };
  const ui = selectionView(selection, { w: 8, h: 16 }, { top: 0, rows: 10, cols: 20, width: 200 });
  const hit = (x: number, y: number) => hitSelectionHandle(x + rect.left, y + rect.top, rect, selection, ui);
  assert.equal(hit(12, 21), 'start');
  assert.equal(hit(11.999, 21), null);
  assert.equal(hit(40, 70), 'start');
  assert.equal(hit(40, 70.001), null);
  assert.equal(hit(116, 102), 'end');
  assert.equal(hit(116.001, 102), null);
  assert.equal(hit(64, 60), 'start', 'different-row overlap keeps the existing first-hit priority');
});

test('same-row capsules split at their midpoint with a leading-handle tie', () => {
  const a = { row: 2, col: 4 }, b = { row: 2, col: 5 };
  for (const selection of [{ anchor: a, head: b }, { anchor: b, head: a }]) {
    const rect = { left: 0, top: 0, width: 200 };
    const ui = selectionView(selection, { w: 10, h: 20 }, { top: 0, rows: 10, cols: 20, width: 200 });
    assert.equal(hitSelectionHandle(49.999, 60, rect, selection, ui), 'start');
    assert.equal(hitSelectionHandle(50, 60, rect, selection, ui), 'start');
    assert.equal(hitSelectionHandle(50.001, 60, rect, selection, ui), 'end');
  }
});

test('an edge handle wins inside the scrollbar touch zone', () => {
  const selection = { anchor: { row: 2, col: 7 }, head: { row: 3, col: 9 } };
  const rect = { left: 0, top: 0, width: 100 };
  const ui = selectionView(selection, { w: 10, h: 20 }, { top: 0, rows: 10, cols: 10, width: 100 });
  assert.ok(rect.width - 99 < 30, 'this point would be a scrollbar drag without the handle priority');
  assert.equal(hitSelectionHandle(99, 86, rect, selection, ui), 'end');
  assert.equal(hitSelectionHandle(100, 86, rect, selection, ui), 'end');
  assert.equal(hitSelectionHandle(100.001, 86, rect, selection, ui), null);
});

test('the leading edge extends to zero, and offscreen handles cannot receive a hit', () => {
  const selection = { anchor: { row: 1, col: 0 }, head: { row: 3, col: 2 } };
  const rect = { left: 100, top: 200, width: 100 };
  const cell = { w: 10, h: 20 }, viewport = { top: 0, rows: 10, cols: 10, width: 100 };
  const ui = selectionView(selection, cell, viewport);
  assert.equal(hitSelectionHandle(100, 245, rect, selection, ui), 'start');
  assert.equal(hitSelectionHandle(99.999, 245, rect, selection, ui), null);
  const hidden = selectionView(selection, cell, { ...viewport, top: 10 });
  assert.equal(hitSelectionHandle(100, 245, rect, selection, hidden), null);
});
