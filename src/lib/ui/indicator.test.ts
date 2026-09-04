// The sliding indicator's geometry is pure: an active box → four CSS variables.
import test from 'node:test';
import assert from 'node:assert/strict';
import { indicatorVars, DEFAULT_ACTIVE } from './indicator.ts';

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
