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

test('team members have exactly three sources, and only bare agents own prompt, Skills and MCP', () => {
  assert.match(source,
    /\{ value: '', label: t\('teamsBare'\) \}[\s\S]{0,400}?\.\.\.defs\.map\([\s\S]{0,220}?teamsCustomAgent[\s\S]{0,400}?\.\.\.subTeams\.map/u,
    'the source picker offers bare backend, custom registry agent, then sub-team');
  assert.match(source,
    /\{#if !m\.base && !m\.team && m\.agent\}[\s\S]*?bind:value=\{m\.agent\.system\}[\s\S]*?m\.agent\.skillSel[\s\S]*?m\.agent\.mcpSel[\s\S]*?\{\/if\}/u,
    'the bare branch owns prompt, Skills and MCP controls');
  assert.match(source,
    /skills: JSON\.stringify\(m\.agent\.skillSel \?\? \[\]\)[\s\S]{0,180}?mcp: JSON\.stringify\(\[\.\.\.\(m\.agent\.mcpSel \?\? \[\]\), \.\.\.\(m\.agent\.mcpExtra \?\? \[\]\)\]\)/u,
    'bare selections serialize into the inline RegAgent');
  assert.match(source,
    /if \(v\.startsWith\('team:'\)\)[\s\S]{0,160}?m\.agent = null[\s\S]{0,160}?m\.agent = v \? null : \(m\.agent \?\? bareEditor\(\)\)/u,
    'switching to inherited sources drops bare-only configuration');
});

test('the team editor keeps a compact member summary and responsive expanded details', () => {
  assert.match(source, /class="member" class:open=\{m\.expanded\}/u, 'each member has one stable card');
  assert.match(source, /aria-expanded=\{m\.expanded\}[\s\S]{0,520}?class:on=\{m\.expanded\}/u,
    'one icon control names and turns with the disclosure state');
  assert.match(source, /\{#if m\.expanded\}[\s\S]{0,120}?<div class="member-body appear">/u,
    'details mount only while expanded');
  assert.match(source, /class="member-section"[\s\S]{0,220}?teamsOverrides/u,
    'custom-agent overrides have a clear section');
  assert.match(source, /class="member-section"[\s\S]{0,220}?teamsBareConfig/u,
    'bare-agent configuration has a clear section');
  assert.match(source, /class="chip-btn team-add"/u, 'Add member is a command, not a membership chip');
  assert.match(source, /\.member-head \{[\s\S]{0,180}?grid-template-columns: minmax\(120px, 1fr\) minmax\(190px, 1\.35fr\) auto/u,
    'desktop summaries reserve stable tracks for name, source and actions');
  assert.match(source, /@media \(max-width: 760px\) \{[\s\S]*?\.member-source \{ grid-column: 1 \/ -1; grid-row: 2; \}[\s\S]*?\.row2, \.row3, \.member-assets \{ grid-template-columns: minmax\(0, 1fr\); \}/u,
    'compact moves source below the name and returns every detail grid to one column');
});
