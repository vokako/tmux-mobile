import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';
import { feedBlocks } from './hub.ts';

// Board #295 (validator P1): a run riding in its reply's bubble is part of
// the card, but reading it is not a tap on the message. A click on its head,
// a row, or "show all" must never open (or close) the message's action row.
const compiled = compileMount(new URL('./Feed.svelte', import.meta.url), []);

test('clicks inside a reply\u2019s run never reach the bubble; the bubble itself still opens its row (#295)', async (context) => {
  const tool = (ts: number, text: string) => ({ id: ts, ts, window: 'dev', kind: 'tool' as const, tool: 'Read', text });
  const blocks = feedBlocks([{ id: 'r', ts: 100, from: 'dev', body: 'fixed it' }],
    Array.from({ length: 8 }, (_, i) => tool(10 + i, `f${i}.rs`)), 'tools', (n) => n);
  const app = await (await compiled).mount(context, {
    props: { selected: 'fixture', roomReady: true, visible: true, blocks, managedNames: ['dev'], stepsRows: 3,
      agents: [{ name: 'dev', window: 0, managed: true, agent: 'kiro', state: 'idle' }] },
    setup(window) { window.Element.prototype.getAnimations = () => []; window.HTMLCanvasElement.prototype.getContext = () => null; },
    modules: [],
  });
  try {
    const q = (s: string) => app.document.querySelector<HTMLElement>(s);
    const acts = () => app.document.querySelectorAll('.m-acts').length;
    for (let i = 0; i < 4; i++) await app.flush();
    q('.bubble .steps .s-head')!.click(); await app.flush();
    assert.ok(q('.bubble .steps.open'), 'the head unfolds the run');
    assert.equal(acts(), 0, 'the head is not a tap on the message');
    q('.bubble .steps .step')!.click(); await app.flush();
    assert.equal(acts(), 0, 'a row is not a tap on the message');
    q('.bubble .steps .s-all')!.click(); await app.flush();
    assert.equal(acts(), 0, '"show all" is not a tap on the message');
    q('.bubble .m-body')!.click(); await app.flush();
    assert.equal(acts(), 1, 'the words still open the action row');
  } finally { await app.close(); }
});

// Board #304 (owner 2026-10-03: "双击钉住的用户消息 可以跳到那个消息的位置"):
// a HELD user bubble, double-clicked (mouse) or double-tapped (touch, the
// terminal's createDoubleTapDetector), jumps the feed to the message's natural
// position. It neither expands nor unpins it; a single tap still opens the row.
test('double-click / double-tap on the held user bubble jumps to its natural position (#304)', async (context) => {
  const blocks = feedBlocks([
    { id: 'a', ts: 100, from: 'dev', body: 'earlier reply' },
    { id: 'q', ts: 200, from: 'human', body: 'the question' },
    { id: 'b', ts: 300, from: 'dev', body: 'later reply' },
  ], [], 'chat', (n) => n);
  const app = await (await compiled).mount(context, {
    props: { selected: 'fixture', roomReady: true, visible: true, blocks, managedNames: ['dev'],
      agents: [{ name: 'dev', window: 0, managed: true, agent: 'kiro', state: 'idle' }] },
    setup(window) { window.Element.prototype.getAnimations = () => []; window.HTMLCanvasElement.prototype.getContext = () => null; },
    modules: [],
  });
  try {
    const q = (s: string) => app.document.querySelector<HTMLElement>(s);
    const frames = async () => { for (let i = 0; i < 4; i++) { await app.advance(20); await app.flush(); } };
    await frames();
    const feed = q('.feed')!;
    Object.defineProperty(feed, 'scrollHeight', { get: () => 3000, configurable: true });
    Object.defineProperty(feed, 'clientHeight', { get: () => 500, configurable: true });
    const ask = q('[data-ask]')!;
    Object.defineProperty(ask, 'offsetTop', { get: () => 2000, configurable: true });
    Object.defineProperty(ask, 'offsetHeight', { get: () => 60, configurable: true });
    // Read upward from below: the question is caught at the bottom edge.
    const scrollTo = async (top: number) => { feed.scrollTop = top; feed.dispatchEvent(new app.window.Event('scroll')); await frames(); };
    await scrollTo(1200); await scrollTo(800); await scrollTo(400);
    assert.ok(q('.msg.held.ask-bottom'), 'the question is held at the bottom edge');
    const bubble = q('.msg.held .bubble')!;
    bubble.click(); await app.flush();
    assert.equal(app.document.querySelectorAll('.m-acts').length, 1, 'a single tap on a held bubble still opens its row');
    bubble.click(); await app.flush();
    bubble.dispatchEvent(new app.window.MouseEvent('dblclick', { bubbles: true }));
    await frames();
    assert.equal(feed.scrollTop, 1992, 'double-click lands on the natural position (8px above)');
    assert.ok(!q('.m-unfold[aria-expanded="true"]'), 'nothing was expanded');
    assert.equal(app.document.querySelectorAll('.m-acts').length, 0, 'no action row is left open');
    // Touch: two quick taps through the shared detector.
    await scrollTo(800); await scrollTo(400);
    const held = q('.msg.held .bubble')!;
    const tap = (t: number) => {
      const e = new app.window.MouseEvent('pointerup', { bubbles: true, clientX: 50, clientY: 50 });
      Object.defineProperty(e, 'pointerType', { value: 'touch' });
      Object.defineProperty(e, 'timeStamp', { value: t });
      held.dispatchEvent(e);
      held.click();
    };
    tap(1000); await app.flush();
    tap(1150); await frames();
    assert.equal(feed.scrollTop, 1992, 'a double-tap lands on the same position');
    assert.equal(app.document.querySelectorAll('.m-acts').length, 0, 'the pair\u2019s second click does not reopen the row');
  } finally { await app.close(); }
});
