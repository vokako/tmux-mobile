import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./Sidebar.svelte', import.meta.url), 'utf8');

test('sidebar rows share Terminal time and route all menu doors with the clicked row', () => {
  // Moved from Hub.source.test.ts with its owner (#121); only the callback
  // boundary changed. The host still builds the shared project action list.
  assert.match(source, /import \{ projectAgeLabel, type ProjectRow \} from '\.\.\/projects\/projects\.ts';/u);
  const start = source.indexOf('{#each rows as row (row.project.id)}');
  const row = source.slice(start, source.indexOf('{#if trash.length}', start));
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

test('the sidebar owns no collapse control — the shell\'s single toggle stands in its head row (boards #174, #197, #217)', () => {
  // #174 handed the head a snippet; #197 replaced the two-slot snippet with
  // one node that rode the partition; #202 moved it to the rail; #217 seats
  // the shell's fixed node at the head row's left — the head only makes room
  // (`.side-toggle-row`, app.css), it renders nothing.
  assert.doesNotMatch(source, /collapse\?: Snippet|@render collapse/u);
  assert.match(source, /<div class="side-h side-head side-toggle-row"><span>\{t\('hubProjects'\)\}<\/span>\n[\s\S]{0,400}?<button class="icon-btn head-add" aria-label=\{t\('projectNew'\)\} title=\{t\('projectNew'\)\} onclick=\{oncreate\}>/u,
    'New project sits at the head, reachable without scrolling (owner, 2026-09-23)');
  assert.doesNotMatch(source, /<button class="side-row add" onclick=\{oncreate\}/u, 'one create entry, not a second at the foot');
  // The open row's wash is the travelling marker (motion principle 14).
  assert.match(source, /use:slideIndicator=\{\{ key: rowKey, active: '\.proj-row\.open', hidden: !rowLit \}\}>\n[\s\S]{0,300}?<span class="slide-pill soft" aria-hidden="true"><\/span>/u);
  assert.doesNotMatch(source, /\.side-row\.open|\.proj-row\s*\{/u, 'the row\'s stacking and its absent in-place wash live with the shared atoms in app.css');
  assert.doesNotMatch(source, /CommandButton/u, 'no second button species in the sidebar');
});
