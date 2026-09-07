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

test('a right-click menu opens from the pointer’s TOP-LEFT; a trigger rect keeps its right-aligned dialect', async () => {
  // Owner, 2026-09-07: "选项卡展示的都是点击点位是选项卡的右上点，这个不符合
  // 我的习惯，应该都为左上角点" — a POINT anchor (right-click / long-press)
  // now defaults to `left`, the OS convention, so the menu grows down-right
  // from the click. A menu on a trigger RECT keeps the right-aligned
  // dot-menu dialect, and any caller may still say `align` explicitly. ONE
  // resolved value feeds both the placement and the grow origin, so the
  // animation starts at the same corner the menu is placed by.
  const source = await readFile(new URL('ui/ContextMenu.svelte', SRC), 'utf8');
  assert.match(source, /const align = \$derived\(at \? \(at\.align \?\? \(at\.anchor \? 'right' : 'left'\)\) : 'right'\);/u,
    'the default is decided by ANCHOR KIND: rect → right, point → left');
  assert.match(source, /menuPlacement\(at\.anchor \?\? pointAnchor\(at\.x, at\.y\), \{ w, h \}, viewBox\(\), 6, 8, align\)/u,
    'placement reads the one resolved alignment');
  assert.match(source, /popOrigin\(at\.anchor \?\? pointAnchor\(at\.x, at\.y\), pos, align\)/u,
    'and the grow origin reads the SAME value — never a second ?? chain');
  // The sidebar dots button fakes its dropdown with a point at the trigger's
  // bottom-right corner; it opts back into the rect dialect explicitly.
  const projects = await readFile(new URL('projects/Projects.svelte', SRC), 'utf8');
  assert.match(projects, /openCtx\(\{ x: r\.right, y: r\.bottom, align: 'right' \}, row\)/u,
    'the dots dropdown stays flush with its trigger');
});
