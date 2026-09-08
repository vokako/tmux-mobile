import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./AgentsPage.svelte', import.meta.url), 'utf8');

test('the desktop config page has THREE levels: categories | rows | editor (board #94)', () => {
  // "在桌面版，可以再多一级，右侧拆分成两级": the chosen category's rows are
  // their own column, so an open editor no longer REPLACES them — the list
  // stays beside what it selected. Compact keeps the two-level drill; the
  // phone's Settings sections keep their sidebar-of-rows shape.
  assert.match(source, /class:with-rows=\{!section\}/u, 'the third column exists only on the desktop page');
  assert.match(source, /<aside class="cat-rows">/u, 'the rows column is its own aside');
  assert.match(source, /\{#if !section\}[\s\S]{0,900}?<aside class="cat-rows">/u, 'and renders only without a section');
  assert.match(source, /\{@render rows\(cat\)\}/u, 'the ONE rows snippet feeds it — no second list dialect');
  // The global instructions have no roster, only the one document.
  assert.match(source, /class:open=\{!!editingGlobal\} onclick=\{startGlobal\}[\s\S]{0,120}?AGENTS\.md/u,
    'the global category\u2019s level is its single AGENTS.md row');
  // Its width is a real divider with a remembered width, like every other.
  assert.match(source, /varName="--agents-rows-w" storeKey="tmux_agents_rows_w"/u, 'the rows column has its own SideHandle');
  assert.match(source, /\.agents-root\.with-rows \{ grid-template-columns: var\(--sidebar-w\) var\(--agents-rows-w, 240px\) minmax\(0, 1fr\); \}/u,
    'three grid columns on the desktop');
  assert.match(source, /localStorage\.getItem\('tmux_agents_rows_w'\)/u, 'the width survives reload');
  // Compact degrades to the pre-#94 shape: the drill, not a squeezed grid.
  const media = /@media \(max-width: 760px\) \{([\s\S]*?)\n  \}/u.exec(source)?.[1] ?? '';
  assert.match(media, /\.cat-rows \{ display: none; \}/u, 'the rows column is desktop-only');
  assert.match(media, /\.agents-root, \.agents-root\.with-rows \{ grid-template-columns: minmax\(0, 1fr\); \}/u,
    'compact is one column regardless');
  // The rows moved OUT of the main column — the old in-mid list stays retired.
  assert.ok(!source.includes('cat-list'), 'no rows in the editor column');
});
