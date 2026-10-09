import assert from 'node:assert/strict';
import test from 'node:test';
import type { TestContext } from 'node:test';
import { compileMount } from '../test/mount.ts';

// Board #323 (reviewer P1): the desktop app fills this machine's gateway
// into Settings — and its token/socket NEVER end up next to another
// server's address. The real Settings, a desktop platform, and a
// get_local_config whose answer the test releases when it wants. The shell
// is the real platform detection (a `__TAURI__` global, desktop UA).
const compiled = compileMount(new URL('./Settings.svelte', import.meta.url), [
  new URL('../core/clipboard.ts', import.meta.url),
]);

const LOCAL = { url: 'ws://[::1]:19900', token: 'LOCAL-TOKEN', tmux_socket: '/local/sock', host: '::', port: 19900 };
const ADDR = 'input[placeholder="ws://host:port"]';
const TOKEN = 'input[placeholder="auth token"]';
const SOCKET = 'input[placeholder="/tmp/tmux-1000/default"]';

async function mount(context: TestContext, saved: Record<string, string>, userAgent?: string) {
  const calls: string[] = [];
  let release!: (cfg: typeof LOCAL) => void;
  const answer = new Promise<typeof LOCAL>(yes => { release = yes; });
  const app = await (await compiled).mount(context, {
    props: { onConnected() {} },
    modules: [
      { copyText: () => true },
    ],
    setup(window) {
      window.localStorage.setItem('tmux_locale', 'en');
      for (const [k, v] of Object.entries(saved)) window.localStorage.setItem(k, v);
      if (userAgent) Object.defineProperty(window.navigator, 'userAgent', { value: userAgent });
      (window as unknown as { __TAURI__: unknown }).__TAURI__ = {
        core: { invoke: (cmd: string) => { calls.push(cmd); return cmd === 'get_local_config' ? answer : Promise.reject(new Error(cmd)); } },
      };
    },
  });
  const field = (sel: string) => app.document.querySelector<HTMLInputElement>(sel)!;
  return {
    ...app,
    calls,
    value: (sel: string) => field(sel).value,
    async edit(sel: string, value: string) {
      field(sel).value = value;
      field(sel).dispatchEvent(new app.window.Event('input', { bubbles: true }));
      await app.flush();
    },
    async answer() {
      release(LOCAL);
      for (let i = 0; i < 6; i++) await app.flush();
    },
  };
}

test('a fresh desktop app is filled with this machine\'s gateway', async context => {
  const app = await mount(context, {});
  await app.answer();
  assert.equal(app.value(ADDR), LOCAL.url);
  assert.equal(app.value(TOKEN), LOCAL.token);
  assert.equal(app.value(SOCKET), LOCAL.tmux_socket);
});

test('a saved remote address with no token gets no local token or socket', async context => {
  const app = await mount(context, { tmux_address: 'ws://remote.example:9899' });
  await app.answer();
  assert.equal(app.value(ADDR), 'ws://remote.example:9899');
  assert.equal(app.value(TOKEN), '');
  assert.equal(app.value(SOCKET), '');
});

test('a remote address typed while the config loads gets no local credentials', async context => {
  const app = await mount(context, {});
  await app.edit(ADDR, 'ws://remote.example:9899');
  await app.answer();
  assert.equal(app.value(ADDR), 'ws://remote.example:9899');
  assert.equal(app.value(TOKEN), '');
  assert.equal(app.value(SOCKET), '');
});

test('auto-filled local credentials do not follow the address to a remote; typed ones stay', async context => {
  const app = await mount(context, {});
  await app.answer();
  assert.equal(app.value(TOKEN), LOCAL.token);
  await app.edit(ADDR, 'ws://remote.example:9899');
  assert.equal(app.value(TOKEN), '', 'the local token is gone');
  assert.equal(app.value(SOCKET), '', 'the local socket is gone');
  await app.edit(TOKEN, 'REMOTE-TOKEN');
  await app.edit(ADDR, 'ws://other.example:9899');
  assert.equal(app.value(TOKEN), 'REMOTE-TOKEN', 'what the person typed is never cleared');
});

test('the Android app never fills a phone with its own loopback config', async context => {
  const app = await mount(context, {}, 'Mozilla/5.0 (Linux; Android 14) AppleWebKit');
  await app.answer();
  assert.deepEqual(app.calls, [], 'get_local_config is not even asked');
  assert.equal(app.value(TOKEN), '');
});
