import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./App.svelte', import.meta.url), 'utf8');
const picker = source.slice(source.indexOf('{#if serverMenuOpen}'), source.indexOf('<!-- Forgetting a saved server'));

test('server switching is immediate and rename has its own sibling command (#165)', () => {
  assert.doesNotMatch(source, /smClickTimer|smRowClick|smRowDbl/u,
    'do not hold a click to infer a double click; remove the old mechanism whole');
  assert.doesNotMatch(picker, /ondblclick/u);
  assert.match(picker, /onclick=\{\(\) => \{ serverMenuOpen = false; doServerSwitch\(s\.id\); \}\}/u);
  assert.match(picker, /<CommandButton variant="icon" icon="edit"/u);
  assert.match(picker, /disabled=\{serverRenaming === s\.id\} onclick=\{\(\) => serverRenameStart\(s\)\}/u,
    'the pencil captures its own row, and cannot restart an in-progress edit');
  assert.match(source, /reconnectMachine\.cancel\(\);\s*disconnect\(\);\s*if \(applySwitch\(localStorage, id\)\) location\.reload\(\);/u,
    'switch transport and storage ordering remains unchanged');
});
