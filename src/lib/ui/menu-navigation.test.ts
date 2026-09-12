import test from 'node:test';
import assert from 'node:assert/strict';
import { nextMenuIndex } from './menu-navigation.ts';

test('menu cursors enter at either edge, wrap, skip disabled items and tolerate a replaced list (#165)', () => {
  assert.equal(nextMenuIndex(-1, 1, 3), 0);
  assert.equal(nextMenuIndex(-1, -1, 3), 2);
  assert.equal(nextMenuIndex(2, 1, 3), 0);
  assert.equal(nextMenuIndex(0, -1, 3), 2);
  assert.equal(nextMenuIndex(-1, -1, 4, i => i < 2), 1);
  assert.equal(nextMenuIndex(0, 1, 3, i => i === 2), 2);
  assert.equal(nextMenuIndex(9, -1, 3), 2);
  assert.equal(nextMenuIndex(0, 1, 0), -1);
  assert.equal(nextMenuIndex(0, 1, 3, () => false), -1);
  assert.notEqual((-1 - 1 + 3) % 3, 2, 'negative control: the previous modulo skipped the last item');
});
