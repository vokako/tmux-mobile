// The app-wide path-link net (board #99): a path href nobody routed must
// never navigate — see path-link-net.ts for the reasoning.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { shouldSwallowPathClick } from './path-link-net.ts';

test('the net swallows unrouted path links and nothing else', () => {
  // Unrouted paths — swallowed (absolute, relative, line-suffixed).
  assert.equal(shouldSwallowPathClick(false, '/local/home/cfu/x.md'), true);
  assert.equal(shouldSwallowPathClick(false, 'temp/notes.md'), true);
  assert.equal(shouldSwallowPathClick(false, '/a/spawn.rs:587'), true);
  // A surface already routed it — the net stays out of the way.
  assert.equal(shouldSwallowPathClick(true, '/local/home/cfu/x.md'), false);
  // The browser's business keeps its native behaviour.
  assert.equal(shouldSwallowPathClick(false, 'https://example.com'), false);
  assert.equal(shouldSwallowPathClick(false, 'mailto:a@b.c'), false);
  assert.equal(shouldSwallowPathClick(false, '#heading'), false);
  assert.equal(shouldSwallowPathClick(false, null), false);
});

test('App installs the net, and it covers the middle click (source contract)', async () => {
  const app = await readFile(new URL('../../App.svelte', import.meta.url), 'utf8');
  assert.match(app, /\$effect\(\(\) => installPathLinkNet\(window\)\);/u, 'one install, App-level');
  const net = await readFile(new URL('./path-link-net.ts', import.meta.url), 'utf8');
  assert.match(net, /addEventListener\('auxclick', onClick\)/u, 'a middle-click 404 window is also swallowed');
  assert.ok(!net.includes("capture: true"), 'bubble phase — routed clicks pass by defaultPrevented');
});
