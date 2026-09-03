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

test('the composer speaks the Hub rule: Enter sends, Shift+Enter is the newline (2026-09-03)', () => {
  const fn = source.match(/function onKeydown\(e\) \{[\s\S]*?\n  \}/u)?.[0] ?? '';
  assert.ok(fn, 'onKeydown exists');
  // IME first: a composing Enter confirms a candidate and must never send.
  assert.match(fn, /if \(e\.isComposing \|\| e\.keyCode === 229\) return;/u);
  // Cmd/Ctrl+Enter keeps sending everywhere.
  assert.match(fn, /if \(e\.metaKey \|\| e\.ctrlKey\) \{ e\.preventDefault\(\); send\(\); return; \}/u);
  // Shift (and a soft keyboard) is the newline; a bare Enter sends.
  assert.match(fn, /if \(e\.shiftKey \|\| layout\.isTouchDevice\) return;/u);
  assert.match(fn, /e\.preventDefault\(\);\s*send\(\);\s*\}\s*$/u, 'a bare Enter ends in send()');
  // The inverted rule must not come back.
  assert.doesNotMatch(fn, /e\.key === 'Enter' && \(e\.metaKey \|\| e\.ctrlKey\)/u);
});
