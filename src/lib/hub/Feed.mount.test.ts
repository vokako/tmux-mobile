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
