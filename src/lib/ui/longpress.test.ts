// The slop rule: what makes a long-pressable list still scrollable.
import test from 'node:test';
import assert from 'node:assert/strict';
import { isScroll } from './longpress.ts';

test('a hold that travels is a scroll, not a press', () => {
  const from = { x: 100, y: 200 };
  // Resting fingers wobble by a pixel or three; that is still a press.
  assert.equal(isScroll(from, { x: 100, y: 200 }), false);
  assert.equal(isScroll(from, { x: 103, y: 197 }), false);
  assert.equal(isScroll(from, { x: 110, y: 210 }), false, '10px is the limit, not past it');
  // Past the slop in either axis it is a scroll — a list must stay flickable.
  assert.equal(isScroll(from, { x: 100, y: 211 }), true);
  assert.equal(isScroll(from, { x: 89, y: 200 }), true);
  // Direction does not matter.
  assert.equal(isScroll(from, { x: 100, y: 189 }), true);
  // The slop is a parameter, so a surface with different needs can say so.
  assert.equal(isScroll(from, { x: 100, y: 205 }, 2), true);
});
