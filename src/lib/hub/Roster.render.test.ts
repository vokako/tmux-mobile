import assert from 'node:assert/strict';
import { mkdtemp, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { JSDOM } from 'jsdom';

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

test('Roster renders the controlled destination strip (#168)', { timeout: 60000 }, async (ctx) => {
  const { createServer } = await import('vite');
  const { svelte } = await import('@sveltejs/vite-plugin-svelte');
  const cacheDir = await mkdtemp(join(tmpdir(), 'roster-render-'));
  const vite = await createServer({
    configFile: false, plugins: [svelte()], cacheDir,
    server: { middlewareMode: true, hmr: false }, appType: 'custom', logLevel: 'error',
    optimizeDeps: { noDiscovery: true, include: [] },
  });
  try {
    const Roster = (await vite.ssrLoadModule('/src/lib/hub/Roster.svelte')).default;
    const { render } = await vite.ssrLoadModule('svelte/server');
    const agents = [
      { name: 'runner', window: 1, agent: 'kiro', state: 'working', team: 'dev', since: 0 },
      { name: 'solo', window: 3, agent: 'claude', state: 'idle', since: 0 },
      { name: 'waiting', window: 2, agent: 'codex', state: 'waiting', team: 'dev/review', since: 0 },
    ];
    const view = (props: Record<string, unknown> = {}) => JSDOM.fragment(render(Roster, { props: {
      selected: 'fixture', roomReady: true, managedAgents: agents, stopped: ['paused'],
      managedNames: agents.map((agent) => agent.name), busyNames: ['runner', 'waiting'],
      selectedRow: { project: { path: '/fixture' }, live: false, slots: [{ window_name: 'paused', command: 'codex' }] },
      recipient: 'runner', unread: new Set(['runner']), stateLabel: (state: string) => state,
      ...props,
    } }).body as string);
    const card = (root: DocumentFragment, name: string) => root.querySelector(`.acard[data-agent="${name}"]`)!;
    const select = (root: DocumentFragment, name: string) => card(root, name).querySelector('button.agent-select')!;
    const stop = (root: DocumentFragment, name: string) => card(root, name).querySelector('.agent-stop > button');

    await ctx.test('native sibling controls preserve team order, identity and status', () => {
      const root = view();
      assert.equal(root.children.length, 1);
      assert.equal(root.querySelectorAll('.roster').length, 1);
      assert.equal(root.querySelectorAll('.tgroup').length, 2);
      assert.equal(root.querySelectorAll('.tgroup[role="group"][aria-label]').length, 2);
      assert.deepEqual([...root.querySelectorAll('.acard[data-agent]')].map((node) => node.getAttribute('data-agent')),
        ['all', 'runner', 'waiting', 'solo', 'paused']);
      assert.deepEqual([...root.querySelectorAll('.tg-label')].map((node) => node.textContent?.trim()), ['dev', 'review']);
      assert.equal(select(root, 'runner').getAttribute('aria-pressed'), 'true');
      assert.equal(select(root, 'all').getAttribute('aria-pressed'), 'false');
      assert.equal(select(root, 'waiting').getAttribute('aria-pressed'), 'false');
      assert.match(select(root, 'all').getAttribute('aria-label')!, /everyone/u);
      assert.match(select(root, 'runner').getAttribute('aria-label')!, /unread/iu);
      assert.ok(card(root, 'runner').querySelector('.st.live-dot'));
      assert.ok(card(root, 'runner').querySelector('.unread'));
      assert.ok(card(root, 'waiting').querySelector('.agent-state.needs'));
      assert.equal(card(root, 'waiting').querySelector('.st.live-dot'), null);
      assert.equal(stop(root, 'runner')?.parentElement?.parentElement, card(root, 'runner'));
      assert.equal(stop(root, 'runner')?.getAttribute('aria-label'), 'Interrupt runner');
      assert.equal(stop(root, 'all')?.getAttribute('aria-label'), 'Interrupt everyone');
      assert.equal(select(root, 'runner').querySelector('button'), null);
      assert.equal(root.querySelector('.a-menu, [role="menu"], [role="button"]'), null);
    });

    await ctx.test('Stop visibility follows busyNames, never a local state guess', () => {
      const root = view({ busyNames: ['solo'] });
      assert.equal(stop(root, 'runner'), null, 'working without parent membership has no Stop');
      assert.equal(stop(root, 'waiting'), null);
      assert.ok(stop(root, 'solo'), 'parent membership is authoritative even with a stale status label');
      assert.ok(stop(root, 'all'));
      assert.equal(view({ busyNames: [] }).querySelector('.agent-stop'), null);
    });

    await ctx.test('pending blocks its member and all, without dimming selection or peers', () => {
      const root = view({ interrupting: ['runner'] });
      for (const name of ['runner', 'all']) {
        assert.ok(stop(root, name)?.hasAttribute('disabled'));
        assert.equal(stop(root, name)?.getAttribute('aria-busy'), 'true');
      }
      assert.equal(stop(root, 'waiting')?.hasAttribute('disabled'), false);
      assert.equal(select(root, 'runner').hasAttribute('disabled'), false);
      const allPending = view({ interrupting: ['runner', 'waiting'] });
      for (const name of ['all', 'runner', 'waiting']) assert.ok(stop(allPending, name)?.hasAttribute('disabled'));
      const unrelated = view({ interrupting: ['removed'] });
      assert.equal(stop(unrelated, 'all')?.hasAttribute('disabled'), false);
    });

    await ctx.test('body extras mark only reached cards with @, without selecting them', () => {
      const marked = (root: DocumentFragment) => [...root.querySelectorAll('.agent-mention')].map((node) => {
        assert.equal(node.textContent, '@');
        return node.closest('.acard')?.getAttribute('data-agent');
      });
      const extra = view({ composerText: '@waiting, @runner @missing' });
      assert.deepEqual(marked(extra), ['waiting']);
      assert.equal(select(extra, 'runner').getAttribute('aria-pressed'), 'true');
      assert.equal(select(extra, 'waiting').getAttribute('aria-pressed'), 'false');
      assert.deepEqual(marked(view({ composerText: '@all' })), ['all', 'runner', 'waiting', 'solo']);
      assert.deepEqual(marked(view({ recipient: 'all', composerText: '@waiting' })), []);
      assert.equal(select(view({ recipient: 'all' }), 'all').getAttribute('aria-pressed'), 'true');
      const none = view({ recipient: '', composerText: '@waiting' });
      assert.equal(none.querySelector('[aria-pressed="true"]'), null);
      assert.deepEqual(marked(none), ['waiting']);
      assert.equal(view({ recipient: '' }).querySelector('.agent-mention'), null);
    });

    await ctx.test('stopped context targets retain backend identity but cannot receive or interrupt', () => {
      const root = view({ acting: true });
      assert.equal(card(root, 'paused').querySelector('img')?.getAttribute('src'), '/assets/codex.svg');
      assert.ok(select(root, 'paused').hasAttribute('disabled'));
      assert.equal(select(root, 'paused').getAttribute('aria-pressed'), null);
      assert.equal(stop(root, 'paused'), null);
      assert.equal(root.querySelector('.a-start'), null);
    });

    await ctx.test('empty/closed rooms retain add; loading and long names remain honest', () => {
      assert.ok(view({ managedAgents: [], stopped: [], managedNames: [], busyNames: [] }).querySelector('.acard.add'));
      assert.equal(view({ selected: '' }).querySelector('.roster'), null);
      assert.ok(view({ roomReady: false }).querySelector('.sk-cards[aria-hidden="true"]'));
      assert.equal(view({ roomReady: false }).querySelector('[data-agent="all"]'), null, 'no empty-all verdict before first answer');
      const name = 'a-very-long-agent-name-with-identity-intact';
      const root = view({ managedAgents: [{ ...agents[0], name }], managedNames: [name], busyNames: [name] });
      assert.equal(card(root, name).querySelector('.a-name')?.textContent, name);
    });
  } finally {
    await vite.close();
    await rm(cacheDir, { recursive: true, force: true });
  }
});
