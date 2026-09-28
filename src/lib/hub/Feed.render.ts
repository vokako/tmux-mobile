import assert from 'node:assert/strict';
import test from 'node:test';
import { feedBlocks, ELIDE } from './hub.ts';
import type { HubActivityEvent } from '../core/ws.ts';
import { renderHarness, RENDER_TIMEOUT_MS } from '../test/ssr.ts';

// One warm server for the whole render tier (board #178): the environment
// and the svelte runtime are built at import, outside this suite's timer.
const h = await renderHarness();

test('Feed renders direct rows, safe rich content, complete capped tools and the original empty slot', { timeout: RENDER_TIMEOUT_MS }, async () => {
  const Feed = (await h.load('/src/lib/hub/Feed.svelte')).default;
  const { render } = h;
  const { createRawSnippet } = await h.load('svelte');
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
  const view = (props: Record<string, unknown> = {}) => h.fragment(render(Feed, { props: {
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
});


test('a sent /command renders as the sender bubble, whole, name in inline code (#264)', { timeout: RENDER_TIMEOUT_MS }, async () => {
  const Feed = (await h.load('/src/lib/hub/Feed.svelte')).default;
  const { render } = h;
  const { createRawSnippet } = await h.load('svelte');
  // The owner's case (2026-09-28): a long /goal that the capsule cut off.
  const args = '你看一下 ../260928-eks-gpu-training 这里有可用的 gpu eks 集群，你准备好数据后，可以在 eks 上进行数据处理，以及先跑通一个 初始的实验，注意用验证集验证效果 *not bold* <b>x</b>';
  const messages = [
    { id: 'g', ts: 1, from: 'human', body: `[tmm] /goal ${args} → kiro` },
    { id: 'm', ts: 2, from: 'human', body: '[tmm] /compact → lead, dev' },
    { id: 's', ts: 3, from: 'human', body: '[tmm] stopped dev' },
  ];
  const blocks = feedBlocks(messages, [], 'chat', (n) => n);
  const tree = h.fragment(render(Feed, { props: {
    selected: 'fixture', roomReady: true, blocks, agents: [], managedNames: ['kiro', 'lead', 'dev'],
    stepsRows: 5, following: false, newBelow: false,
    emptyFeed: createRawSnippet(() => ({ render: () => '<div></div>' })),
  } }).body as string);
  const bubbles = tree.querySelectorAll('.msg.me');
  assert.equal(bubbles.length, 2, 'both commands are the sender bubble, even at the chat level');
  assert.equal(tree.querySelector('.sysline'), null, 'the lifecycle line is hidden at chat level; no command capsule');
  const goal = bubbles[0]!.querySelector('.m-body p')!;
  assert.equal(goal.querySelector('.m-to')?.textContent, '@kiro');
  assert.equal(goal.querySelector('code')?.textContent, '/goal');
  assert.ok(goal.textContent!.includes(args), 'the arguments render whole, as text');
  assert.equal(goal.querySelector('b,em'), null, 'arguments are data, not markdown or HTML');
  assert.equal(bubbles[0]!.querySelector('.m-unfold'), null, 'a command never folds');
  const fan = bubbles[1]!.querySelector('.m-body p')!;
  assert.deepEqual([...fan.querySelectorAll('.m-to')].map((n) => n.textContent), ['@lead', '@dev'], 'fan-out lists every target like a message does');
  assert.equal(fan.querySelector('code')?.textContent, '/compact');
});
