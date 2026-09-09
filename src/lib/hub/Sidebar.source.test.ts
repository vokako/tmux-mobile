import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./Sidebar.svelte', import.meta.url), 'utf8');

test('sidebar rows share Terminal time and route all menu doors with the clicked row', () => {
  // Moved from Hub.source.test.ts with its owner (#121); only the callback
  // boundary changed. The host still builds the shared project action list.
  assert.match(source, /import \{ projectAgeLabel, type ProjectRow \} from '\.\.\/projects\/projects\.ts';/u);
  const start = source.indexOf('{#each rows as row (row.project.id)}');
  const row = source.slice(start, source.indexOf('<button class="side-row add"', start));
  assert.match(row, /<div class="side-row proj-row"/u);
  assert.match(row, /<button class="proj-pick" onclick=\{\(\) => onselect\(row\.project\.session\)\}/u);
  assert.match(row, /projectAgeLabel\(row, talkMap, tick\)/u);
  assert.match(row, /<button class="icon-btn row-menu" aria-label=\{t\('hubProjectMenu'\)\}/u);
  assert.match(row, /<Icon name="dots" size=\{13\} \/>/u);
  assert.match(row, /e\.preventDefault\(\); onmenu\(row, \{ x: e\.clientX \?\? 0, y: e\.clientY \?\? 0 \}\)/u);
  assert.match(row, /onlongpress: \(pt\) => onmenu\(row, pt\)/u);
  assert.match(row, /onmenu\(row, \{ anchor: anchorOf\(e\.currentTarget\), trigger: e\.currentTarget \}\)/u);
  assert.equal([...row.matchAll(/onmenu\(/g)].length, 3, 'three doors, one host action source');
  const menuStyle = /\.row-menu \{([^}]*)\}/u.exec(source)?.[1] ?? '';
  assert.doesNotMatch(menuStyle, /border/u, 'private CSS places the shared borderless icon button');
});

test('hover reads the full live count and the same clock at open time', () => {
  assert.match(source, /use:hoverInfo=\{\(\) => rowInfo\(row\)\}/u);
  assert.match(source, /const n = rowAgentCounts\(row, panes\)/u);
  assert.match(source, /const age = projectAgeLabel\(row, talkMap, tick\)/u);
  assert.match(source, /row\.project\.session === selected && unreadCount/u);
  assert.doesNotMatch(source, /getBoundingClientRect|setInterval/u,
    'the sidebar adds neither its own geometry calculation nor a clock');
});

test('the sidebar keeps the shared sheet and delegates consequential actions', () => {
  assert.match(source, /\{#if compact && open\}\s*<div class="side-scrim" onclick=\{onclose\}/u);
  assert.match(source, /<aside class="sidebar" class:side-sheet=\{compact\} class:sheet=\{compact\} class:open=\{compact && open\}>/u);
  assert.match(source, /let trashOpen = \$state\(false\)/u);
  assert.match(source, /onclick=\{\(\) => onrestore\(r\)\}/u);
  assert.match(source, /onclick=\{\(\) => onpurge\(r\)\}/u);
  assert.doesNotMatch(source, /projectDelete|projectArchive|projectDown|createHubBackRegistry|onGoBack/u,
    'RPCs, confirmations and the Back floor remain in Hub');
  const css = source.slice(source.indexOf('<style>'));
  assert.match(css, /\.sidebar\.sheet \.side-row \{ min-height: 44px; \}/u);
  assert.doesNotMatch(css, /^\s*\.(?:st|note-dot|spacer|empty)(?:[.#:{\s])/mu,
    'unrelated private Hub atoms are not copied into this component');
});
