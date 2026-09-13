import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Files.svelte', import.meta.url), 'utf8');
const style = source.slice(source.indexOf('<style>'));

test('Files uses named shared tools rather than a private 24px dialect (#157)', () => {
  assert.match(source, /import CommandButton from '\.\.\/ui\/CommandButton\.svelte'/u);
  assert.doesNotMatch(source, /class="(?:tool-btn|act-btn|back-btn|bm-del)"/u);
  assert.doesNotMatch(style, /\.tool-btn|\.act-btn|\.back-btn|\.bm-del/u);
  // Owner2026-09-11: measured trailing overflow replaces the two-row toolbar.
  assert.match(style, /\.toolbar \{[^}]*flex-wrap: nowrap/u);
  assert.match(source, /visibleToolCount\(toolbarActions\.length/u);
  assert.match(source, /use:measureToolbar/u);
});

test('the empty preview yields its space and file names do not lose to size metadata (#157)', () => {
  assert.match(source, /class:preview-open=\{view !== 'list'\}/u);
  assert.match(source, /varName="--files-list-w" storeKey="tmux_files_list_w"/u);
  assert.doesNotMatch(style, /\.files-left \{[^}]*width: var\(--sidebar-w\)/u);
  assert.match(style, /\.file-name \{[^}]*white-space: pre-wrap;[^}]*overflow-wrap: anywhere/u);
  assert.doesNotMatch(style, /\.file-name \{[^}]*overflow-x: auto/u);
  // The owner's03:00 revision deliberately retires #157's size-below rule.
  assert.doesNotMatch(source, /class="file-label"/u);
  assert.match(source, /class="file-name"[\s\S]*?class="file-size"/u);
  assert.match(style, /\.file-size \{[^}]*flex: none/u);
  assert.match(style, /\.file-main \{[^}]*min-height: var\(--files-row-height\)/u);
});

test('Files menus reuse the shared menu and long-press, without taking preview selection (#164)', () => {
  assert.match(source, /import ContextMenu from '\.\.\/ui\/ContextMenu\.svelte'/u);
  assert.match(source, /import \{ longpress \} from '\.\.\/ui\/longpress\.ts'/u);
  assert.match(source, /<ContextMenu at=\{fileMenu\?\.at\}/u);
  assert.match(source, /entryToolActions\(entry,/u);
  assert.match(source, /if \(fileMenu\) \{ closeFileMenu\(\); return true; \}/u);
  assert.match(source, /systemOwnsContextMenu\(event\)/u);
  // Files.browser.test-fixture.ts executes the dirty-draft regression in Chromium
  // with real highlighting; compileMount's heavy-import guards stay strict.
  assert.match(source, /open: \(entry\) => \{ const target = \{ \.\.\.entry \}; leaveEditor\(\(\) => openEntry\(target\)\); \}/u);
});

test('Files confirmations name the scope and keep errors/Discard semantics in the shared dialog (#167)', () => {
  // Local deletion removes a copy, not the server original; Discard abandons a
  // draft, not a file. These are caller-owned semantics, not dialog defaults.
  assert.match(source, /local: \{ title: 'confirmDeleteLocalFileTitle', note: 'confirmDeleteLocalFileNote'/u);
  assert.match(source, /confirmIcon=\{pendingAct\?\.kind === 'leave' \? 'check' : 'trash'\}/u);
  assert.match(source, /danger=\{pendingAct\?\.kind !== 'leave'\}/u);
  assert.match(source, /cancelLabel=\{pendingAct\?\.kind === 'leave' \? t\('configKeepEditing'\) : t\('cancel'\)\}/u);
  assert.match(source, /error=\{pendingAct\?\.error \|\| ''\}/u);
  assert.match(source, /oncancel=\{cancelPendingAct\}/u);
});

test('long code cannot expand the Files flex item beyond its allocated pane (#157)', () => {
  assert.match(style, /\.files \{[^}]*min-width: 0/u);
  assert.match(style, /\.files-split \{[^}]*min-width: 0/u);
});

test('the session-directory tool wears the house (board #194)', () => {
  // Owner 2026-09-13: "第一个按钮有歧义，应该变成小房子" — the terminal glyph reads as
  // "open a terminal" everywhere else in the app.
  assert.match(source, /\{ key: 'cwd', label: t\('filesSessionDir'\), icon: 'home', run: goSessionDir \}/u);
});
