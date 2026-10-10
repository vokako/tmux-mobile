import assert from 'node:assert/strict';
import test from 'node:test';
import { createViteConfig, devProxy } from '../vite.config.js';
import { DEV_SERVER_PORT_ENV } from './dev-ports.mjs';

test('Vite proxies WebSocket and downloads to the same internal Rust server', () => {
  const proxy = devProxy({ [DEV_SERVER_PORT_ENV]: '19099' });
  assert.deepEqual(proxy, {
    '/ws': { target: 'ws://127.0.0.1:19099', ws: true },
    '/dl': { target: 'http://127.0.0.1:19099' },
  });
});

test('production build does not parse the development backend port', () => {
  const build = createViteConfig('build', { PORT: 'not-a-port' });
  assert.equal('proxy' in build.server, false);
  assert.throws(() => createViteConfig('serve', { PORT: 'not-a-port' }), /invalid internal dev server port/);
});

test('agent worktrees inside the checkout are neither watched nor scanned (#342)', async () => {
  // Task worktrees live at <repo>/worktree/<agent>-<task>. Without these two
  // entries the dev server serving the checkout watched every worktree (705
  // inotify watches for one fresh worktree; a built one adds a 58k-entry
  // target/) and its dependency scan crawled each worktree's index.html as a
  // second app. Measured on a private Vite with a nested worktree/: 304 → 0
  // watches there, and the scan stopped following its import.
  const { fileURLToPath } = await import('node:url');
  const here = fileURLToPath(new URL('../worktree', import.meta.url));
  const config = createViteConfig('serve', {});
  assert.deepEqual(config.server.watch, { ignored: [`${here}/**`] },
    'the watch ignore is ABSOLUTE and anchored at the config: a worktree’s own Vite ignores only a worktree/ nested inside it');
  assert.deepEqual(config.optimizeDeps.entries, ['**/*.html', '!worktree/**'],
    'Vite’s default entry glob, minus the worktrees — an explicit list REPLACES the default, so it must keep **/*.html');
  assert.deepEqual(config.optimizeDeps.exclude, ['@xterm/xterm'], 'the xterm source-serving rule is untouched');
  // And the build sees the same scan rule (a build never watches).
  assert.deepEqual(createViteConfig('build', {}).optimizeDeps.entries, config.optimizeDeps.entries);
});
