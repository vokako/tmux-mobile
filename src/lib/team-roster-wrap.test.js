import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./Team.svelte', import.meta.url), 'utf8');

test('Team roster chips wrap before using bounded vertical overflow', () => {
  const rosterRule = source.match(/\.team-header-scroll\s*\{([^}]+)\}/u)?.[1] || '';

  assert.match(rosterRule, /flex-wrap:\s*wrap/u);
  assert.match(rosterRule, /max-height:\s*80px/u);
  assert.match(rosterRule, /overflow-x:\s*hidden/u);
  assert.match(rosterRule, /overflow-y:\s*auto/u);
  assert.doesNotMatch(rosterRule, /overflow-x:\s*auto/u);
});
