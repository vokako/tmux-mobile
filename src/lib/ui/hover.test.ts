import test from 'node:test';
import assert from 'node:assert/strict';
import { dwellFor, HOVER_DWELL_MS, HOVER_HOP_MS } from './hover-dwell.ts';

test('a first hover dwells, a hop between neighbours does not', () => {
  assert.equal(dwellFor(false), HOVER_DWELL_MS);
  assert.equal(dwellFor(true), HOVER_HOP_MS);
  assert.ok(HOVER_HOP_MS < 100 && HOVER_DWELL_MS >= 300, 'a dwell is a decision, a hop is a scan');
});
