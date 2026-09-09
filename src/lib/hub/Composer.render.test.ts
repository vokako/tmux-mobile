import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { ALL_TARGET } from './hub-composer.ts';

const noop = () => {};
(globalThis as Record<string, unknown>).localStorage ??= { getItem: () => null, setItem: noop, removeItem: noop };
(globalThis as Record<string, unknown>).window ??= {
  addEventListener: noop, removeEventListener: noop, navigator: { language: 'en' },
  matchMedia: () => ({ matches: false, addEventListener: noop, removeEventListener: noop }),
};
(globalThis as Record<string, unknown>).document ??= {
  addEventListener: noop, removeEventListener: noop,
  documentElement: { style: { setProperty: noop, getPropertyValue: () => '' } },
};

test('Composer renders destination, command and attachment states without a new wrapper', { timeout: 60000 }, async () => {
  const { createServer } = await import('vite');
  const vite = await createServer({
    server: { middlewareMode: true, hmr: false }, appType: 'custom', logLevel: 'error',
    cacheDir: 'node_modules/.vite-composer-render-test',
    optimizeDeps: { noDiscovery: true, include: [] },
  });
  try {
    const Composer = (await vite.ssrLoadModule('/src/lib/hub/Composer.svelte')).default;
    const { render } = await vite.ssrLoadModule('svelte/server');
    const agents = [
      { name: 'alice', window: 0, managed: true, agent: 'kiro', state: 'working' },
      { name: 'bob', window: 1, managed: true, agent: 'codex', state: 'idle' },
    ];
    const view = (props: Record<string, unknown> = {}) => JSDOM.fragment(render(Composer, { props: {
      selected: 'fixture', recipient: 'alice', agents, managedAgents: agents, managedNames: ['alice', 'bob'], ...props,
    } }).body as string);
    const empty = view();
    assert.equal(empty.children.length, 1);
    assert.ok(empty.firstElementChild?.classList.contains('composer'));
    assert.ok(empty.querySelector('.send-btn.busy .ss-ring'));
    assert.equal(empty.querySelector<HTMLButtonElement>('.send-btn')!.disabled, false);
    assert.equal(empty.querySelector('.to-name')?.textContent, 'alice');
    assert.equal(view({ recipient: ALL_TARGET }).querySelector('.to-name')?.textContent, 'everyone');
    const note = view({ recipient: '' });
    assert.ok(note.querySelector('.to-chip.note'));
    assert.equal(note.querySelector<HTMLButtonElement>('.send-btn')!.disabled, true);
    assert.equal(view({ managedAgents: [] }).querySelector('.to-wrap'), null);
    assert.ok(view({ composerText: '/clear', sendable: true }).querySelector('.compose-shell.cmd'));
    assert.equal(view({ composerText: '/clear', recipient: '', sendable: true }).querySelector('.compose-shell.cmd'), null);
    assert.equal(view({ composerText: 'hello @bob' }).querySelector('.to-extra')?.textContent, '+@bob');
    const attachments = view({
      failed: true, sendable: true,
      pending: [
        { key: 'err', name: 'failed.pdf', error: 'denied' },
        { key: 'file', kind: 'file', n: 1, name: 'report.txt', path: '/report.txt' },
        { key: 'image', kind: 'image', n: 2, name: 'screen.png', path: '/screen.webp', thumb: 'blob:fixture' },
      ],
    });
    assert.equal(attachments.querySelector('.pend-chip.err .pend-why')?.textContent, 'denied');
    assert.equal(attachments.querySelectorAll('.pend-x').length, 3);
    assert.equal(attachments.querySelector('.pend-thumb img')?.getAttribute('src'), 'blob:fixture');
    assert.equal(attachments.querySelector<HTMLButtonElement>('.send-btn')!.disabled, true);
    assert.equal(view({ attaching: true, sendable: true }).querySelector<HTMLButtonElement>('.send-btn')!.disabled, true);
  } finally { await vite.close(); }
});
