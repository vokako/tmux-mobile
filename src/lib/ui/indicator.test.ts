// The sliding indicator's geometry is pure: an active box → four CSS variables.
import test from 'node:test';
import assert from 'node:assert/strict';
import { indicatorVars, boxFromRects, DEFAULT_ACTIVE } from './indicator.ts';

test('indicatorVars maps the active child’s offset box to the atom’s variables', () => {
  assert.deepEqual(indicatorVars({ offsetLeft: 96, offsetTop: 4, offsetWidth: 72, offsetHeight: 28 }), {
    '--ind-x': '96px', '--ind-y': '4px', '--ind-w': '72px', '--ind-h': '28px',
  });
});

test('no active child collapses the indicator instead of leaving it stranded', () => {
  assert.deepEqual(indicatorVars(null), { '--ind-x': '0px', '--ind-y': '0px', '--ind-w': '0px', '--ind-h': '0px' });
});

test('the default active selector covers the app’s selected-state spellings', () => {
  for (const s of ['[aria-current]', '.active', '.on', '.sel', '[aria-selected="true"]']) assert.ok(DEFAULT_ACTIVE.includes(s), s);
});

test('boxFromRects puts the active rect in the container’s padding box, in its own CSS pixels', () => {
  // A nested button (the rail's .rail-slot wrapper) measured by rects, not offsets.
  const container = { left: 0, top: 100, width: 46, height: 600 };
  const item = { left: 6, top: 152, width: 34, height: 32 };
  assert.deepEqual(boxFromRects(container, item), { offsetLeft: 6, offsetTop: 52, offsetWidth: 34, offsetHeight: 32 });
  // Under the root's CSS zoom the client rects are visual pixels: divide them back.
  assert.deepEqual(boxFromRects({ left: 0, top: 0, width: 200, height: 40 }, { left: 60, top: 0, width: 80, height: 40 }, 2),
    { offsetLeft: 30, offsetTop: 0, offsetWidth: 40, offsetHeight: 20 });
  // A bordered container (the tab bar's 1px top border) is subtracted, since the
  // absolute indicator's origin is the padding box.
  assert.deepEqual(boxFromRects({ left: 10, top: 10, width: 100, height: 50 }, { left: 20, top: 11, width: 30, height: 49 }, 1, { left: 0, top: 1 }),
    { offsetLeft: 10, offsetTop: 0, offsetWidth: 30, offsetHeight: 49 });
  // A zero zoom (unparsable --ui-zoom) is treated as 1, never a division by zero.
  assert.equal(boxFromRects({ left: 0, top: 0, width: 1, height: 1 }, { left: 5, top: 0, width: 1, height: 1 }, 0).offsetLeft, 5);
});
