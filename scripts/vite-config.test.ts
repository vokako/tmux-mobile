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
