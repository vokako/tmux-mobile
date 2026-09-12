import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import { feedBlocks, ELIDE } from './hub.ts';
import type { HubActivityEvent } from '../core/ws.ts';
import { ssrServer } from '../test/ssr.ts';

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
  const vite = await ssrServer({ cacheDir: 'node_modules/.vite-feed-render-test' });
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
    const events: HubActivityEvent[] = Array.from({ length: 26 }, (_, i) => ({
      id: i + 1, ts: 10 + i, window: i % 2 ? 'bob' : 'alice',
      kind: 'tool' as const, tool: 'Read', text: `source-${i}.ts`,
    }));
    // The input half (board #172): the 601-char notice that stopped
    // mid-sentence in the owner's screenshot, and a short delivery that fits.
    const longNotice = '[tmm chat 2026-09-11 08:00] claude: [board #168 reply] Composer agent strip — ' + 'the strip replaces the chip. '.repeat(20).trimEnd() + '. Reply on the issue with `tmm board note 168 "..."`.';
    events.push(
      { id: 90, ts: 40, window: 'bob', kind: 'prompt', text: longNotice },
      { id: 91, ts: 41, window: 'alice', kind: 'prompt', text: '[tmm chat 2026-09-11 08:01] human: [board #7] Feed extraction: status todo → doing' },
    );
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
    assert.equal(tree.querySelectorAll('.m-unfold').length, 2, 'the long ask and the long prompt fold; nothing else does');
    // A prompt row folds its TEXT through elideTail — a visible `……` and the
    // same unfold control as a bubble — instead of clipping silently.
    const prompts = tree.querySelectorAll('.prompt');
    assert.equal(prompts.length, 2);
    const longRow = prompts[0]!;
    const longBody = longRow.querySelector('.p-body')!.textContent ?? '';
    assert.ok(longBody.endsWith(ELIDE), `the folded prompt ends with the marker: ${longBody.slice(-20)}`);
    assert.ok(longBody.length < longNotice.length, 'folded means shorter');
    assert.ok(longRow.querySelector('.m-unfold'), 'the way to the whole prompt');
    // The reply notice wears the board dialect: chip + badge, marker gone.
    assert.equal(longRow.querySelector('.p-chip')?.textContent, '#168');
    assert.ok(longRow.querySelector('.p-badge'), 'a reply notice carries the badge');
    assert.ok(!longBody.includes('[board #168 reply]'), 'the marker never renders as text');
    assert.equal(prompts[1]!.querySelector('.p-badge'), null, 'a plain board delivery has no badge');
    const shortBody = prompts[1]!.querySelector('.p-body')!.textContent ?? '';
    assert.ok(shortBody.includes('Feed extraction: status todo → doing'), 'a short prompt renders whole');
    assert.equal(prompts[1]!.querySelector('.m-unfold'), null, 'nothing to unfold when it fits');
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
