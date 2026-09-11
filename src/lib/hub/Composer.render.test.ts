import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { ALL_TARGET } from './hub-composer.ts';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';

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

test('Composer renders a destination-labelled textarea and send-only actions (#168)', { timeout: 60000 }, async () => {
  const { createServer } = await import('vite');
  const { svelte } = await import('@sveltejs/vite-plugin-svelte');
  const cacheDir = await mkdtemp(join(tmpdir(), 'tmm-composer-render-'));
  const vite = await createServer({
    configFile: false, plugins: [svelte({ configFile: false })],
    server: { middlewareMode: true, hmr: false }, appType: 'custom', logLevel: 'error',
    cacheDir,
    optimizeDeps: { noDiscovery: true, include: [] },
  });
  try {
    const Composer = (await vite.ssrLoadModule('/src/lib/hub/Composer.svelte')).default;
    const { render } = await vite.ssrLoadModule('svelte/server');
    const { t } = await vite.ssrLoadModule('/src/lib/core/i18n.svelte.ts');
    const agents = [
      { name: 'alice', window: 0, managed: true, agent: 'kiro', state: 'working' },
      { name: 'bob', window: 1, managed: true, agent: 'codex', state: 'idle' },
    ];
    const view = (props: Record<string, unknown> = {}) => JSDOM.fragment(render(Composer, { props: {
      selected: 'fixture', recipient: 'alice', agents, ...props,
    } }).body as string);
    const send = (fragment: DocumentFragment) =>
      fragment.querySelector<HTMLButtonElement>('.composer-actions button:last-child')!;
    const textarea = (fragment: DocumentFragment) => fragment.querySelector('textarea')!;
    const empty = view();
    assert.equal(empty.children.length, 1);
    assert.ok(empty.firstElementChild?.classList.contains('composer'));
    assert.equal(empty.querySelectorAll('.composer-actions .command-button').length, 2);
    assert.equal(send(empty).disabled, true);
    assert.equal(send(view({ interruptible: true })).disabled, true, 'an interruptible recipient never turns empty Send into Stop');
    assert.equal(send(empty).getAttribute('aria-label'), t('hubSend'));
    assert.equal(textarea(empty).getAttribute('placeholder'), t('hubComposerDm').replace('{name}', 'alice'));
    assert.equal(textarea(empty).getAttribute('aria-label'), textarea(empty).getAttribute('placeholder'));
    assert.equal(textarea(view({ recipient: ALL_TARGET })).getAttribute('placeholder'), t('hubComposerAll'));
    const note = view({ recipient: '' });
    assert.equal(textarea(note).getAttribute('placeholder'), t('hubComposerRoom'));
    assert.equal(send(note).disabled, true);
    assert.equal(send(view({ recipient: '', composerText: 'note', sendable: true })).disabled, false);
    assert.equal(empty.querySelector('.to-wrap,.to-chip,.to-menu,.int-pill,.stop-spin,.ss-ring,.c-mirror'), null);
    assert.equal(textarea(empty).style.textIndent, '');
    assert.equal(textarea(empty).parentElement, empty.querySelector('.composer-actions')?.parentElement);
    assert.ok(view({ composerText: '/clear', sendable: true }).querySelector('.compose-shell.cmd'));
    assert.equal(view({ composerText: '/clear', recipient: '', sendable: true }).querySelector('.compose-shell.cmd'), null);
    assert.equal(textarea(view({ composerText: '/clear', sendable: true })).getAttribute('placeholder'), textarea(empty).getAttribute('placeholder'),
      'command detection does not rewrite the existing destination placeholder');
    assert.equal(view({ composerText: 'hello @bob' }).querySelector('.to-extra'), null, 'body-mention display belongs to Roster');
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
    assert.equal(send(attachments).disabled, true);
    assert.equal(send(view({ attaching: true, sendable: true })).disabled, true);
    assert.equal(send(view({ selected: '', sendable: true })).disabled, true);
    assert.equal(send(view({ composerText: 'hello', sendable: true })).disabled, false);
  } finally {
    await vite.close();
    await rm(cacheDir, { recursive: true, force: true });
  }
});
