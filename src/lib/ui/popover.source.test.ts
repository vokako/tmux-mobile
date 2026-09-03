// Source-contract test for the ONE popover mechanism (design-language.md §5).
//
// Select, ContextMenu and PanePicker dismiss on "any ancestor scroll" with a
// CAPTURE listener on window — which also receives the menu's OWN scroll. A
// long list therefore closed the moment it was scrolled (review, 2026-09-03),
// and nothing failed while it did. Each layer must exclude itself.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const SRC = new URL('../', import.meta.url); // src/lib/

test('a popover’s own scroll never dismisses it; only scrolls outside do', async () => {
  const cases = [
    ['ui/Select.svelte', /if \(menuEl && e\.target instanceof Node && menuEl\.contains\(e\.target\)\) return;/u],
    ['ui/ContextMenu.svelte', /if \(el && e\.target instanceof Node && el\.contains\(e\.target\)\) return;/u],
    ['sessions/PanePicker.svelte', /if \(!\(scroller instanceof Node\) \|\| !opener \|\| !scroller\.contains\(opener\)\) return;/u],
  ] as const;
  for (const [file, guard] of cases) {
    const source = await readFile(new URL(file, SRC), 'utf8');
    assert.match(source, guard, `${file}: the scroll listener must spare the layer itself`);
    assert.match(source, /addEventListener\('scroll', onScroll, true\)/u, `${file}: still a capture listener — any OUTSIDE ancestor scroll dismisses`);
    assert.doesNotMatch(source, /addEventListener\('scroll', (?:hide|oncancel|onClose), true\)/u, `${file}: no bare dismiss on scroll`);
  }
});
