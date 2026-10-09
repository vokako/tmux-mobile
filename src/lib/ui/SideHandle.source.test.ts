// Board #324: the ONE resize handle gained a top edge (the scratch terminal
// from the bottom) without a second implementation, and its old left/right
// consumers keep their exact behaviour.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./SideHandle.svelte', import.meta.url), 'utf8');

test('a top edge works on the Y axis, with Up/Down and a horizontal separator', () => {
  assert.match(source, /const vertical = \$derived\(edge === 'top'\);/u);
  assert.match(source, /const sign = \$derived\(edge === 'left' \|\| edge === 'top' \? -1 : 1\);/u, 'dragging up grows a bottom panel');
  assert.match(source, /const at = \(ev\) => \(vertical \? ev\.clientY : ev\.clientX\);/u);
  assert.match(source, /vertical \? \['ArrowUp', 'ArrowDown'\] : \['ArrowLeft', 'ArrowRight'\]/u);
  assert.match(source, /aria-orientation=\{vertical \? 'horizontal' : 'vertical'\}/u);
  assert.match(source, /\.side-handle\.on-top \{[^}]*cursor: row-resize;/u);
});

test('a stored size is clamped to what the viewport can hold, and the old consumers keep their gate', () => {
  assert.match(source, /const cap = \(\) => Math\.max\(MIN, Math\.min\(max, room\(\)\)\);/u);
  assert.match(source, /Math\.min\(cap\(\), Math\.max\(MIN, Math\.round\(w\)\)\)/u);
  assert.match(source, /\.side-handle:not\(\.always\) \{ display: none; \}/u, 'only an opted-in panel keeps its handle below 760px');
});
