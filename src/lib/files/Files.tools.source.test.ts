import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Files.svelte', import.meta.url), 'utf8');
const style = source.slice(source.indexOf('<style>'));

test('Files uses named shared tools rather than a private 24px dialect (#157)', () => {
  assert.match(source, /import CommandButton from '\.\.\/ui\/CommandButton\.svelte'/u);
  assert.doesNotMatch(source, /class="(?:tool-btn|act-btn|back-btn|bm-del)"/u);
  assert.doesNotMatch(style, /\.tool-btn|\.act-btn|\.back-btn|\.bm-del/u);
  assert.match(style, /\.toolbar \{[^}]*flex-wrap: wrap/u);
});

test('the empty preview yields its space and file names do not lose to size metadata (#157)', () => {
  assert.match(source, /class:preview-open=\{view !== 'list'\}/u);
  assert.match(source, /varName="--files-list-w" storeKey="tmux_files_list_w"/u);
  assert.doesNotMatch(style, /\.files-left \{[^}]*width: var\(--sidebar-w\)/u);
  assert.match(style, /\.file-name \{[^}]*white-space: pre-wrap;[^}]*overflow-wrap: anywhere/u);
  assert.doesNotMatch(style, /\.file-name \{[^}]*overflow-x: auto/u);
  assert.match(source, /class="file-label">[\s\S]*?class="file-name"[\s\S]*?class="file-size"/u);
});

test('long code cannot expand the Files flex item beyond its allocated pane (#157)', () => {
  assert.match(style, /\.files \{[^}]*min-width: 0/u);
  assert.match(style, /\.files-split \{[^}]*min-width: 0/u);
});
