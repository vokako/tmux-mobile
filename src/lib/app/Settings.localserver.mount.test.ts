import assert from 'node:assert/strict';
import test from 'node:test';
import { compileMount } from '../test/mount.ts';

// Board #323 review P1-b: the connect card follows the local server's mode
// through the server_mode_changed event — a `starting` that turns `failed`
// shows its reason without reopening the page. No polling: the invoke is
// asked once.
const compiled = compileMount(new URL('./Settings.svelte', import.meta.url), [
  new URL('../core/clipboard.ts', import.meta.url),
]);

test('a start that fails after the page opened shows its reason at once (#323)', async (context) => {
  const calls: string[] = [];
  let emit!: (payload: unknown) => void;
  const app = await (await compiled).mount(context, {
    props: { onConnected() {} },
    modules: [{ copyText: () => true }],
    setup(window) {
      window.localStorage.setItem('tmux_locale', 'en');
      (window as unknown as { __TAURI__: unknown }).__TAURI__ = {
        core: { invoke: (cmd: string) => { calls.push(cmd); return cmd === 'server_mode' ? Promise.resolve({ mode: 'starting', url: 'ws://127.0.0.1:9899' }) : Promise.reject(new Error(cmd)); } },
        event: { listen: (name: string, fn: (e: { payload: unknown }) => void) => { if (name === 'server_mode_changed') emit = (p) => fn({ payload: p }); return Promise.resolve(() => {}); } },
      };
    },
  });
  try {
    for (let i = 0; i < 8; i++) await app.flush();
    const note = () => app.document.querySelector('.config-note[role="status"]')?.textContent ?? '';
    assert.equal(note(), '', 'starting is no trouble');
    assert.ok(emit, 'subscribed to server_mode_changed');
    emit({ mode: 'failed', url: 'ws://127.0.0.1:9899', reason: 'Address already in use' });
    await app.flush();
    assert.equal(note(), 'This computer: Server failed · Address already in use');
    emit({ mode: 'embedded', url: 'ws://127.0.0.1:9899' });
    await app.flush();
    assert.equal(note(), '', 'a later good state clears it');
    assert.equal(calls.filter((c) => c === 'server_mode').length, 1, 'one read; the rest is events');
  } finally { await app.close(); }
});
