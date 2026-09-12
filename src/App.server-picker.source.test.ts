import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./App.svelte', import.meta.url), 'utf8');
const picker = source.slice(source.indexOf('{#if serverMenuOpen}'), source.indexOf('<!-- Forgetting a saved server'));

test('both server doors describe the same non-modal picker, not an action menu (#165)', async () => {
  const prefs = await readFile(new URL('./lib/app/Preferences.svelte', import.meta.url), 'utf8');
  assert.match(picker, /role="dialog" aria-modal="false" aria-label=\{t\('serversTitle'\)\}/u);
  assert.match(picker, /id=\{serverPickerId\}/u);
  assert.doesNotMatch(picker, /role="menu"|role="menuitem"/u);
  assert.match(source, /aria-haspopup="dialog"\s+aria-expanded=\{serverMenuOpen\}\s+aria-controls=\{serverMenuOpen \? serverPickerId : undefined\}/u);
  assert.match(source, /serversControls=\{serverPickerId\}/u);
  assert.match(prefs, /aria-haspopup="dialog"/u);
  assert.match(prefs, /aria-controls=\{serversOpen \? serversControls : undefined\}/u);
});

test('server picker owns border-box placement and a bounded native scroller (#165)', () => {
  assert.match(picker, /bind:this=\{serverMenuEl\}/u);
  assert.match(picker, /bind:offsetWidth=\{serverMenuW\} bind:offsetHeight=\{serverMenuH\}/u);
  assert.doesNotMatch(picker, /bind:clientWidth|bind:clientHeight/u);
  assert.match(source, /if \(serverMenuEl && e\.target instanceof Node && serverMenuEl\.contains\(e\.target\)\) return;/u);
  const effect = source.slice(source.indexOf('if (!serverMenuOpen'), source.indexOf('/** Switch:'));
  assert.match(effect, /addEventListener\('scroll', onScroll, true\)/u);
  assert.match(effect, /removeEventListener\('scroll', onScroll, true\)/u);
  assert.match(effect, /if \(!\(scroller instanceof Node\) \|\| !origin \|\| !scroller\.contains\(origin\)\) return;/u,
    'background terminal output is not movement of the picker opener');
  const panel = /\.server-menu \{([^}]+)\}/u.exec(source)?.[1] ?? '';
  assert.match(panel, /width: max-content/u);
  assert.match(panel, /max-height: calc\(100vh \/ var\(--ui-zoom, 1\) - 16px\)/u);
  assert.match(panel, /overflow-y: auto/u);
});

test('server picker keys yield to modals and composition, with a local rename cancellation (#165)', () => {
  assert.match(source, /serverRenameComposing/u);
  assert.match(picker, /oncompositionstart=\{serverRenameCompositionStart\}/u);
  assert.match(picker, /oncompositionend=\{serverRenameCompositionEnd\}/u);
  assert.match(source, /if \(serverRenameComposing\) \{\s*serverRenamePending = \{ id: serverRenaming, menu: serverMenuEl, close: true \};\s*return;/u,
    'outside dismissal must wait for the native final composition value');
  const finish = source.slice(source.indexOf('async function serverRenameCompositionEnd'), source.indexOf('/** svelte action: focus'));
  assert.ok(finish.indexOf('await tick()') >= 0 && finish.indexOf('await tick()') < finish.indexOf('serverRenameComposing = null'),
    'blur cannot bypass composition ending while the final input is still pending');
  assert.match(finish, /serverRenameDraft = composition\.input\.value/u);
  assert.match(picker, /serverRenameKey/u);
  assert.match(source, /function serverRenameKey\(e\)[\s\S]*?e\.isComposing \|\| e\.keyCode === 229 \|\| serverRenameComposing/u);
  assert.match(source, /function cancelServerRename/u);
  const effect = source.slice(source.indexOf('if (!serverMenuOpen'), source.indexOf('/** Switch:'));
  assert.match(effect, /activeModal\(document\)/u);
  assert.match(effect, /menu\.contains\(document\.activeElement\)/u);
  assert.match(effect, /menu\.focus\(\{ preventScroll: true \}\)/u);
  assert.match(effect, /const onDown = \(e\) => \{\s*if \(modalOwnsInteraction\(\)\) return;/u);
  assert.match(effect, /const onScroll = \(e\) => \{\s*if \(modalOwnsInteraction\(\)\) return;/u,
    'clicking or scrolling a confirmation cannot dismiss its underlying picker');
});

test('app navigation shortcuts yield to focused menu and picker controls (#165)', async () => {
  const shortcuts = await readFile(new URL('./lib/app/shortcuts.svelte.ts', import.meta.url), 'utf8');
  const guard = shortcuts.slice(shortcuts.indexOf('export function isShortcutInputTarget'), shortcuts.indexOf('export const shortcuts'));
  assert.match(guard, /\[role="menu"\]/u);
  assert.match(guard, /\[role="dialog"\]/u);
});
