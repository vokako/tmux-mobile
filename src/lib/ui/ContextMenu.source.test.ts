import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./ContextMenu.svelte', import.meta.url), 'utf8');

test('context menu touch targets follow input capability, not viewport width (#164)', async () => {
  // More inherits existing tool actions; moving them to a menu cannot reduce
  // the44px coarse hit floor, including on a wide touch screen.
  // #165 moves the retained coarse floor into the shared menu row owner.
  const css = await readFile(new URL('../../app.css', import.meta.url), 'utf8');
  assert.match(source, /class="menu-item"/u);
  assert.match(css, /@media \(any-pointer: coarse\) \{\s*:root \{ --menu-row-height: 44px/u);
  assert.match(css, /\.menu-item \{[^}]*min-height: var\(--menu-row-height\)/u);
  assert.match(source, /bind:offsetWidth=\{w\} bind:offsetHeight=\{h\}/u);
  assert.match(source, /max-height: calc\(100vh \/ var\(--ui-zoom, 1\) - 16px\); overflow-y: auto/u,
    'a full directory menu must stay within the same8px inset on a short viewport');
});

test('keeping a click trigger clear opts into the shared geometry cap, not another placement path (#173)', () => {
  assert.match(source, /at\?\.keepTriggerClear && at\.anchor\s*\? menuHeightLimit\(at\.anchor, viewBox\(\)\)/u);
  assert.match(source, /style:max-height=\{heightLimit === undefined \? undefined : `\$\{heightLimit\}px`\}/u);
  assert.match(source, /menuPlacement\(at\.anchor \?\? pointAnchor\(at\.x, at\.y\), \{ w, h \}/u,
    'placement consumes the measured capped border box');
});
