import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./ContextMenu.svelte', import.meta.url), 'utf8');

test('context menu touch targets follow input capability, not viewport width (#164)', () => {
  // More inherits existing tool actions; moving them to a menu cannot reduce
  // the44px coarse hit floor, including on a wide touch screen.
  assert.match(source, /@media \(any-pointer: coarse\) \{\s*\.ctx button \{ min-height: 44px/u);
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
