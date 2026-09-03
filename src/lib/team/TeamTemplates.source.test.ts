// Source-contract test for TeamTemplates.svelte (see docs/conventions/testing.md).
//
// The phone's template picker is the app's ONE dropdown (`ui/Select`). It was a
// hand-rolled backdrop panel until 2026-09-03 — the kind of second dropdown
// species design-language.md §5 forbids — and nothing failed while it drifted.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./TeamTemplates.svelte', import.meta.url), 'utf8');

test('the compact template picker is ui/Select, not a backdrop panel', () => {
  assert.match(source, /import Select from '\.\.\/ui\/Select\.svelte'/u);
  assert.match(source, /<div class="tpl-picker-sel">\s*<Select value=\{String\(selIdx\)\}/u, 'the picker IS the shared Select, driven by the selected index');
  assert.doesNotMatch(source, /tpl-picker-backdrop|tpl-picker-menu|tpl-picker-item|pickerOpen/u, 'no hand-rolled panel left');
  // New template is an action beside the field, never a row inside a pick-one list.
  assert.match(source, /<button class="tpl-add tpl-picker-add" onclick=\{addTemplate\}/u);
});
