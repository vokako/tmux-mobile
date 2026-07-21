import test from 'node:test';
import assert from 'node:assert/strict';
import { localFontSource, normalizeFontFamily } from './font-validation.ts';

test('keeps multi-word local font family names intact', () => {
  assert.equal(normalizeFontFamily("  'Maple Mono NF CN'  "), 'Maple Mono NF CN');
  assert.equal(localFontSource('Maple Mono NF CN'), 'local("Maple Mono NF CN")');
});
