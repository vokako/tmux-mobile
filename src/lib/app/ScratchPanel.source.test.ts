// Board #324 source contract: the scratch panel reuses the one Terminal and
// the one resize handle, slides on the shared tempo, owns no Escape inside
// the terminal, and App mounts it inside the server key, desktop only.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./ScratchPanel.svelte', import.meta.url), 'utf8');
const app = await readFile(new URL('../../App.svelte', import.meta.url), 'utf8');

test('the shared Terminal and SideHandle, one slide tempo, no backdrop', () => {
  assert.match(source, /import Terminal from '\.\.\/terminal\/Terminal\.svelte';/u);
  assert.match(source, /<Terminal \{target\} \{session\} \{fontSize\} embedded chromeless active=\{open\} visible=\{open\} onPaneExit=\{ended\} \/>/u);
  assert.match(source, /<SideHandle varName="--scratch-h"[^>]*edge="top"[^>]*always \/>/u);
  assert.match(source, /<SideHandle varName="--scratch-w"[^>]*edge="right"[^>]*always \/>/u);
  const css = (/<style>[\s\S]*<\/style>/u.exec(source)?.[0] ?? '').replace(/\/\*[\s\S]*?\*\//gu, '');
  assert.match(css, /transition: transform var\(--t-move\) ease/u);
  assert.doesNotMatch(css, /\d+ms|scrim|backdrop/u, 'no private tempo, no backdrop');
});

test('Escape closes from the head only; the terminal keeps it', () => {
  assert.match(source, /<header class="scratch-head" onkeydown=\{onHeadKey\}/u);
  assert.doesNotMatch(source, /window\.addEventListener\('keydown'|<section[^>]*onkeydown/u, 'no Escape catcher over the terminal');
});

test('App: desktop only, inside the server key, closed when a switch starts; the toggle is a registry shortcut', async () => {
  const keyed = app.slice(app.indexOf('{#key serverEpoch}'), app.indexOf('{/key}'));
  assert.match(keyed, /\{#if !layout\.isTouchDevice && connected\}\s*<ScratchPanel open=\{scratchOpen\} live=\{!switching\}/u);
  assert.match(app, /\$effect\(\(\) => \{ if \(switching\) scratchOpen = false; \}\);/u);
  assert.match(app, /function resetServerMemory\(\) \{\s*scratchOpen = false;/u);
  assert.match(app, /toggleScratch\(\) \{ scratchOpen = !scratchOpen; \},/u);
  const shortcuts = await readFile(new URL('./shortcuts.ts', import.meta.url), 'utf8');
  assert.match(shortcuts, /\{ id: 'toggleScratch', label: 'shortcutToggleScratch', group: 'panels', default: 'Mod2\+Backquote'/u);
});

test('the scratch RPCs are desktop-gated like projects (#324 review): a phone build never names the module', async () => {
  const rpc = await readFile(new URL('../../../src-tauri/src/server/rpc.rs', import.meta.url), 'utf8');
  for (const m of ['scratch_session', 'scratch_kill']) {
    assert.match(rpc, new RegExp(`#\\[cfg\\(not\\(any\\(target_os = "android", target_os = "ios"\\)\\)\\)\\]\\n\\s*"${m}" =>`, 'u'), `${m} carries the projects gate`);
  }
});

test('the panel size is clamped by the layout itself, on restore and as the window shrinks (#324 review)', () => {
  // Measured in Chromium: h=1200 / w=1400 restored into 900x600 and then
  // 700x420 keeps the head and the handle on screen for both edges
  // (temp/check/c324clamp.mjs); ResizeObserver refits the terminal, no timer.
  assert.match(source, /height: min\(var\(--scratch-h, 320px\), calc\(100vh \/ var\(--ui-zoom, 1\) - 80px\)\);/u);
  assert.match(source, /width: min\(var\(--scratch-w, 560px\), calc\(100vw \/ var\(--ui-zoom, 1\) - var\(--shell-left, 0px\) - 80px\)\);/u);
  assert.match(source, /<Terminal \{target\} \{session\}/u, 'the server\'s returned session name, never spelled here');
  assert.doesNotMatch(source, /setTimeout|setInterval/u, 'no timed resize');
});

test('the left-docked panel starts under the Android status bar (#332)', () => {
  const left = source.match(/\.scratch\.left \{([^}]*)\}/u)?.[1] ?? '';
  assert.match(left, /right: auto; top: var\(--sat, 0px\); height: auto;/u, 'fixed, so it carries --sat itself');
});
