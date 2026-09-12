import test from 'node:test';
import assert from 'node:assert/strict';
import { createKeyQueue, pasteOrFallback } from './terminal-input.ts';

type Deferred = { resolve: () => void; reject: (e: unknown) => void };
function harness() {
  const sends: [string, string, boolean][] = [];
  const pending: Deferred[] = [];
  const events: string[] = [];
  const queue = createKeyQueue({
    send: (target, keys, literal) => { sends.push([target, keys, literal]); return new Promise<void>((resolve, reject) => pending.push({ resolve, reject })); },
    onSuccess: () => events.push('ok'), onFailure: () => events.push('fail'), dbg: (m) => events.push(m),
  });
  return { queue, sends, pending, events };
}
const settle = () => new Promise(r => setTimeout(r, 0));

test('a queued key goes to the pane it was typed into, never the pane shown when the link frees up (board #190)', async () => {
  const { queue, sends, pending } = harness();
  queue.enqueue('A', 'a', true);            // in flight to A
  queue.enqueue('A', 'b', true);            // merges behind it: "b" for A
  queue.reset();                            // the pane switch: queued keys belong to the previous pane
  queue.enqueue('B', 'c', true);            // typed into the new pane while A's send is still pending
  assert.deepEqual(sends, [['A', 'a', true]]);
  pending[0]!.resolve(); await settle();
  assert.deepEqual(sends, [['A', 'a', true], ['B', 'c', true]], 'the merged "b" died with the switch; "c" reaches B, not A');
  pending[1]!.resolve(); await settle();
  assert.equal(queue.sending, false);
});

test('one send in flight; literals merge only for the same pane; special keys keep their order (board #190)', async () => {
  const { queue, sends, pending } = harness();
  queue.enqueue('A', 'x', true);
  queue.enqueue('A', 'y', true);
  queue.enqueue('A', 'Enter', false);
  queue.enqueue('A', 'z', true);
  queue.enqueue('B', 'w', true);            // a different pane never merges into A's literal
  assert.equal(sends.length, 1, 'only one RPC in flight');
  for (let i = 0; i < 4; i++) { pending[i]!.resolve(); await settle(); }
  assert.deepEqual(sends.map(s => [s[0], s[1], s[2]]), [['A', 'x', true], ['A', 'y', true], ['A', 'Enter', false], ['A', 'z', true], ['B', 'w', true]]);
});

test('a failed send drops everything queued behind it; the cap drops the newest (board #190)', async () => {
  const { queue, sends, pending, events } = harness();
  queue.enqueue('A', 'a', true);
  queue.enqueue('A', 'Up', false);
  queue.enqueue('A', 'Up', false);
  pending[0]!.reject(new Error('link down')); await settle();
  assert.deepEqual(sends, [['A', 'a', true]], 'nothing replayed after the failure');
  assert.ok(events.includes('fail'));
  const small = createKeyQueue({ send: () => new Promise(() => {}), onSuccess() {}, onFailure() {}, dbg: (m) => events.push(m), max: 2 });
  small.enqueue('A', 'Up', false); small.enqueue('A', 'Up', false); small.enqueue('A', 'Up', false); small.enqueue('A', 'Up', false);
  assert.equal(small.length, 2, 'one in flight, two queued (the cap counts what waits), the fourth dropped');
  assert.ok(events.some(e => /queue full/u.test(e)));
});

test('the paste fallback types into the pane that was pasted into, even after a switch (board #190)', async () => {
  // A pre-paste_text server answers -32601 after a round trip; by then the
  // user may be looking at another pane. The fallback keeps the ORIGINAL pane.
  const enqueued: [string, string, boolean][] = [];
  const events: string[] = [];
  let reject!: (e: unknown) => void;
  const done = pasteOrFallback('A', 'ls\r', {
    paste: () => new Promise((_, rj) => { reject = rj; }),
    enqueue: (target, keys, literal) => enqueued.push([target, keys, literal]),
    onSuccess: () => events.push('ok'), onFailure: (k) => events.push('fail:' + k),
  });
  // (the caller's live target has moved to B meanwhile — irrelevant to the captured pane)
  reject(Object.assign(new Error('method not found'), { code: -32601 }));
  await done;
  assert.deepEqual(enqueued, [['A', 'ls\r', true]]);
  assert.deepEqual(events, []);
  // Any other failure is a paste failure, not a retype.
  const enqueued2: unknown[] = []; const events2: string[] = [];
  await pasteOrFallback('A', 'x', { paste: () => Promise.reject(new Error('boom')), enqueue: (...a) => enqueued2.push(a), onSuccess: () => events2.push('ok'), onFailure: (k) => events2.push('fail:' + k) });
  assert.deepEqual(enqueued2, []); assert.deepEqual(events2, ['fail:paste']);
  const events3: string[] = [];
  await pasteOrFallback('A', 'x', { paste: () => Promise.resolve(), enqueue: () => { throw new Error('no'); }, onSuccess: () => events3.push('ok'), onFailure: (k) => events3.push('fail:' + k) });
  assert.deepEqual(events3, ['ok']);
});
