import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { feedBlocks } from './hub.ts';

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

test('Feed renders direct rows, safe rich content, complete capped tools and the original empty slot', { timeout: 60000 }, async () => {
  const { createServer } = await import('vite');
  const vite = await createServer({
    server: { middlewareMode: true, hmr: false }, appType: 'custom', logLevel: 'error',
    cacheDir: 'node_modules/.vite-feed-render-test',
    optimizeDeps: { noDiscovery: true, include: [] },
  });
  try {
    const Feed = (await vite.ssrLoadModule('/src/lib/hub/Feed.svelte')).default;
    const { render } = await vite.ssrLoadModule('svelte/server');
    const { createRawSnippet } = await vite.ssrLoadModule('svelte');
    const agents = [
      { name: 'alice', window: 0, managed: true, agent: 'kiro', state: 'working' },
      { name: 'bob', window: 1, managed: true, agent: 'codex', state: 'idle' },
    ];
    const messages = [
      { id: 'q', ts: 1, from: 'human', body: '@alice ' + 'A long question. '.repeat(400) },
      { id: 'note', ts: 2, from: 'human', body: 'A room-only record.' },
      { id: 'reply', ts: 3, from: 'alice',
        body: '**Result** $x^2$ [source](/source.ts) [bad](javascript:alert(1))\n\n![chart](/chart.png)' },
      { id: 'board', ts: 4, from: 'alice', body: '[tmm] board #7 todo → doing — Feed extraction' },
    ];
    const events = Array.from({ length: 26 }, (_, i) => ({
      id: i + 1, ts: 10 + i, window: i % 2 ? 'bob' : 'alice',
      kind: 'tool' as const, tool: 'Read', text: `source-${i}.ts`,
    }));
    const blocks = feedBlocks(messages, events, 'tools', (name) => name);
    const emptyFeed = createRawSnippet(() => ({ render: () => '<div class="empty">Empty fixture</div>' }));
    const view = (props: Record<string, unknown> = {}) => JSDOM.fragment(render(Feed, { props: {
      selected: 'fixture', roomReady: true, blocks, agents, managedNames: ['alice', 'bob'],
      stepsRows: 5, following: false, newBelow: true, emptyFeed, ...props,
    } }).body as string);
    const tree = view();
    assert.equal(tree.children.length, 1);
    assert.ok(tree.firstElementChild?.classList.contains('feed-wrap'));
    const feed = tree.querySelector('.feed')!;
    for (const row of tree.querySelectorAll('.msg,.steps,.sysline,.day-sep')) assert.equal(row.parentElement, feed);
    assert.equal(tree.querySelectorAll('[data-ask]').length, 2);
    assert.equal(tree.querySelectorAll('.m-unfold').length, 1);
    assert.ok(tree.querySelector('.m-state.note .note-dot'));
    assert.ok(tree.querySelector('.katex'));
    assert.ok(tree.querySelector('.m-body strong'));
    assert.ok(tree.querySelector('a[href="/source.ts"]'));
    assert.equal(tree.querySelector('a[href^="javascript:"]'), null);
    assert.equal(tree.querySelector('.ci-ref')?.textContent, '/chart.png', 'client image loading retains the exact reference');
    assert.ok(tree.querySelector('.sys-jump'));
    assert.equal(tree.querySelectorAll('.s-body.capped').length, 2);
    assert.equal(tree.querySelectorAll('.step').length, 26, 'the cap is a viewport, not data loss');
    assert.equal(tree.querySelectorAll('.s-live.live-dot').length, 1);
    assert.ok(tree.querySelector('.to-tail.news'));
    const empty = view({ blocks: [] }).querySelector('.empty')!;
    assert.ok(empty.parentElement?.classList.contains('feed'), 'the parent snippet introduces no row wrapper');
    assert.ok(view({ blocks: [], roomReady: false }).querySelector('.sk-feed[aria-hidden="true"]'));
  } finally { await vite.close(); }
});
