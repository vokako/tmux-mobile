import assert from 'node:assert/strict';
import test from 'node:test';
import { heldAnchor, readingDirection, refoldEligible, sameReadingSize } from './hub-reading.ts';
import type { HeldAnchor, ReadingDirection } from './hub-reading.ts';
import { pickAnchor } from './hub.ts';

test('reading size compares both dimensions and never treats unknown boxes as stable', () => {
  const size = { width: 370.5, height: 600 };
  assert.equal(sameReadingSize(null, null), false);
  assert.equal(sameReadingSize(size, null), false);
  assert.equal(sameReadingSize(null, size), false);
  assert.equal(sameReadingSize(size, { ...size }), true);
  assert.equal(sameReadingSize(size, { ...size, width: 370.25 }), false);
  assert.equal(sameReadingSize(size, { ...size, height: 599.75 }), false);
});

test('direction keeps zero travel unchanged and clears reversals on forward motion', () => {
  for (const direction of ['up', 'down'] as const) {
    assert.deepEqual(readingDirection(0, direction, 9), { direction, travel: 9 });
    assert.deepEqual(readingDirection(direction === 'down' ? 2 : -2, direction, 9),
      { direction, travel: 0 });
  }
});

test('direction accumulates jitter and flips exactly at 16px, symmetrically', () => {
  for (const initial of ['up', 'down'] as const) {
    const sign = initial === 'down' ? -1 : 1;
    let state = { direction: initial as ReadingDirection, travel: 0 };
    for (const magnitude of [1, 3, 2, 9.5]) {
      state = readingDirection(sign * magnitude, state.direction, state.travel);
    }
    assert.deepEqual(state, { direction: initial, travel: 15.5 });
    state = readingDirection(sign * 0.5, state.direction, state.travel);
    assert.deepEqual(state, { direction: initial === 'down' ? 'up' : 'down', travel: 0 });
    assert.deepEqual(readingDirection(sign * 100, initial, 0), state,
      'overshoot commits the direction without carrying excess travel');
  }
});

test('forward movement starts a fresh reversal budget', () => {
  assert.deepEqual(readingDirection(1, 'down', 15), { direction: 'down', travel: 0 });
  assert.deepEqual(readingDirection(-2, 'down', 0), { direction: 'down', travel: 2 });
});

const idle: HeldAnchor = { key: '', edge: '', held: false };
const picked = (edge: HeldAnchor['edge'], key = 'a') => ({ key, edge });

test('first contact uses the inclusive 1px threshold on either edge', () => {
  assert.deepEqual(heldAnchor(picked('top'), { top: 101, height: 40 }, idle, 100, 300),
    { key: 'a', edge: 'top', held: true });
  assert.equal(heldAnchor(picked('top'), { top: 101.01, height: 40 }, idle, 100, 300).held, false);
  assert.deepEqual(heldAnchor(picked('bottom'), { top: 259, height: 40 }, idle, 100, 300),
    { key: 'a', edge: 'bottom', held: true });
  assert.equal(heldAnchor(picked('bottom'), { top: 258.99, height: 40 }, idle, 100, 300).held, false);
});

test('a held top edge survives a re-edge through 8px, then releases', () => {
  const current: HeldAnchor = { key: 'a', edge: 'top', held: true };
  assert.deepEqual(heldAnchor(picked('bottom'), { top: 108, height: 40 }, current, 100, 300),
    current);
  assert.deepEqual(heldAnchor(picked('bottom'), { top: 108.01, height: 40 }, current, 100, 300),
    { key: 'a', edge: 'bottom', held: false });
});

test('a held bottom edge survives a re-edge through 8px, then releases', () => {
  const current: HeldAnchor = { key: 'a', edge: 'bottom', held: true };
  assert.deepEqual(heldAnchor(picked('top'), { top: 252, height: 40 }, current, 100, 300),
    current);
  assert.deepEqual(heldAnchor(picked('top'), { top: 251.99, height: 40 }, current, 100, 300),
    { key: 'a', edge: 'top', held: false });
});

test('retention is not shared across keys or unheld bubbles', () => {
  const current: HeldAnchor = { key: 'a', edge: 'top', held: true };
  assert.deepEqual(heldAnchor(picked('bottom', 'b'), { top: 105, height: 40 }, current, 100, 300),
    { key: 'b', edge: 'bottom', held: false });
  assert.deepEqual(heldAnchor(picked('bottom'), { top: 105, height: 40 },
    { ...current, held: false }, 100, 300),
  { key: 'a', edge: 'bottom', held: false });
});

test('missing geometry clears held without inventing a selection or edge', () => {
  const current: HeldAnchor = { key: 'a', edge: 'top', held: true };
  assert.deepEqual(heldAnchor(picked('bottom'), undefined, current, 100, 300),
    { key: 'a', edge: 'bottom', held: false });
  assert.deepEqual(heldAnchor(picked('', ''), undefined, current, 100, 300), idle);
  assert.deepEqual(heldAnchor(picked(''), { top: 100, height: 40 }, idle, 100, 300),
    { key: 'a', edge: '', held: false });
});

test('pickAnchor jump reset selects from the destination, not the previous gap key', () => {
  const items = [{ key: 'a', top: 0, height: 40 }, { key: 'b', top: 300, height: 40 }];
  const current: HeldAnchor = { key: 'a', edge: 'top', held: true };
  const continuing = pickAnchor(items, 100, 100, 500, 'up', current);
  const jumped = pickAnchor(items, 100, 100, 500, 'up', undefined);
  assert.deepEqual(heldAnchor(continuing, items.find((it) => it.key === continuing.key), current, 100, 200),
    current);
  assert.deepEqual(heldAnchor(jumped, items.find((it) => it.key === jumped.key), current, 100, 200),
    { key: 'b', edge: 'bottom', held: true });
});

test('jump reset does not independently erase the current same-key held state', () => {
  // Hub resets pickAnchor's seed, not its separately tracked held state.
  const items = [{ key: 'a', top: 105, height: 40 }];
  const current: HeldAnchor = { key: 'a', edge: 'top', held: true };
  const jumped = pickAnchor(items, 100, 200, 500, 'up', undefined);
  assert.deepEqual(jumped, { key: 'a', edge: 'bottom' });
  assert.deepEqual(heldAnchor(jumped, items[0], current, 100, 300), current);
});

test('refold requires the whole box to cross the strict 120px margin', () => {
  assert.equal(refoldEligible({ top: 40, height: 40 }, 200, 400), false);
  assert.equal(refoldEligible({ top: 39.99, height: 40 }, 200, 400), true);
  assert.equal(refoldEligible({ top: 520, height: 40 }, 200, 400), false);
  assert.equal(refoldEligible({ top: 520.01, height: 40 }, 200, 400), true);
  assert.equal(refoldEligible({ top: 0, height: 600 }, 200, 400), false,
    'a tall expanded bubble intersecting the viewport remains expanded');
  assert.equal(refoldEligible({ top: 200, height: 40 }, 200, 400), false);
});
