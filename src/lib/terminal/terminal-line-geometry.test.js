import test from 'node:test';
import assert from 'node:assert/strict';
import { compactLineGeometry } from './terminal-line-geometry.ts';

test('compact lines split clipped height equally above and below', () => {
  const geometry = compactLineGeometry(20, 6, 2, 0.6);
  assert.deepEqual(geometry, { charCssHeight: 10, offset: 2 });
  assert.equal(geometry.charCssHeight - geometry.offset - 6, geometry.offset);
});

test('normal and expanded lines need no clipping correction', () => {
  assert.equal(compactLineGeometry(20, 10, 2, 1), null);
  assert.equal(compactLineGeometry(20, 14, 2, 1.4), null);
});

test('missing renderer dimensions safely disable correction', () => {
  assert.equal(compactLineGeometry(0, 6, 2, 0.6), null);
});
