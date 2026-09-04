// The sliding indicator's geometry is pure: an active box → four CSS variables.
import test from 'node:test';
import assert from 'node:assert/strict';
import { indicatorVars, boxFromOffsets, DEFAULT_ACTIVE } from './indicator.ts';

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

test('boxFromOffsets walks offsetParent from both ends, so a nested item and a transformed one measure the same', () => {
  const root = { offsetLeft: 0, offsetTop: 0, offsetWidth: 400, offsetHeight: 800, offsetParent: null };
  const nav = { offsetLeft: 0, offsetTop: 100, offsetWidth: 46, offsetHeight: 600, offsetParent: root };
  // The rail button sits inside a .rail-slot wrapper (its offsetParent when the slot is positioned).
  const slot = { offsetLeft: 6, offsetTop: 52, offsetWidth: 34, offsetHeight: 32, offsetParent: nav };
  const btn = { offsetLeft: 0, offsetTop: 0, offsetWidth: 34, offsetHeight: 32, offsetParent: slot };
  assert.deepEqual(boxFromOffsets(btn, nav), { offsetLeft: 6, offsetTop: 52, offsetWidth: 34, offsetHeight: 32 });
  // A direct child of a bordered container (the tab bar's 1px top border): the
  // absolute pill's origin is the padding box, so the border comes off.
  const bar = { offsetLeft: 0, offsetTop: 700, offsetWidth: 400, offsetHeight: 56, offsetParent: root };
  const tab = { offsetLeft: 80, offsetTop: 1, offsetWidth: 80, offsetHeight: 55, offsetParent: bar };
  assert.deepEqual(boxFromOffsets(tab, bar, { left: 0, top: 1 }), { offsetLeft: 80, offsetTop: 0, offsetWidth: 80, offsetHeight: 55 });
  // Offsets are layout: a press scale or a mid-flip transform on the item does
  // not enter the answer (a client rect would have shrunk or lagged).
});
