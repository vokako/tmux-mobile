import test from 'node:test';
import assert from 'node:assert/strict';
import { normalizeUiZoom, stepUiZoom, UI_ZOOM_DEFAULT } from './ui-zoom.ts';

test('normalizes persisted UI zoom to supported tenths', () => {
  assert.equal(normalizeUiZoom('1.24'), 1.2);
  assert.equal(normalizeUiZoom('bad'), UI_ZOOM_DEFAULT);
  assert.equal(normalizeUiZoom(0.1), 0.6);
  assert.equal(normalizeUiZoom(3), 1.8);
});

test('steps UI zoom without floating point drift', () => {
  assert.equal(stepUiZoom(1, 1), 1.1);
  assert.equal(stepUiZoom(1.1, -1), 1);
  assert.equal(stepUiZoom(1.8, 1), 1.8);
});
