import assert from 'node:assert/strict';
import test from 'node:test';
import { COMPLETION_FEEDBACK_MS, createFeedbackLifetime, scheduleCompletion, type FeedbackValue } from './feedback-lifetime.ts';

function fixture() {
  const values: (FeedbackValue | null)[] = [];
  const tasks: { run: () => void; delay: number; cancelled: boolean }[] = [];
  const clock = {
    set(run: () => void, delay: number) {
      const task = { run, delay, cancelled: false }; tasks.push(task); return task;
    },
    clear(task: unknown) { (task as typeof tasks[number]).cancelled = true; },
  };
  return { values, tasks, clock, life: createFeedbackLifetime(value => values.push(value), clock) };
}

test('only an ordinary success expires, through the one 1500ms scheduler (#167)', () => {
  const { life, values, tasks } = fixture();
  const token = life.begin();
  for (const kind of ['progress', 'error', 'result'] as const) {
    assert.equal(life.update(token, { kind, message: kind }), true);
    assert.equal(tasks.length, 0, `${kind} remains until an explicit transition`);
  }
  life.update(token, { kind: 'success', message: 'Copied' });
  const before = values.length;
  assert.equal(tasks[0]!.delay, COMPLETION_FEEDBACK_MS);
  assert.equal(COMPLETION_FEEDBACK_MS, 1500);
  tasks[0]!.run();
  assert.equal(values.at(-1), null);
  assert.equal(values.length, before + 1);
  tasks[0]!.run();
  assert.equal(values.length, before + 1, 'an expiry cannot fire twice');
});

test('A-B-A completions and already-queued expiry callbacks belong to their attempt (#167)', () => {
  const { life, values, tasks } = fixture();
  const a = life.begin();
  life.update(a, { kind: 'success', message: 'Copied' });
  const firstExpiry = tasks[0]!;
  const b = life.begin();
  assert.equal(life.update(a, { kind: 'error', message: 'Old failure' }), false);
  life.update(b, { kind: 'success', message: 'Copied' });
  firstExpiry.run();
  assert.equal(values.at(-1)?.message, 'Copied');
  const again = life.begin();
  assert.equal(life.update(b, { kind: 'success', message: 'Copied' }), false);
  life.update(again, { kind: 'success', message: 'Copied' });
  tasks[1]!.run();
  assert.equal(values.at(-1)?.kind, 'success', 'equal text does not mean equal intent');
  assert.ok(firstExpiry.cancelled);
});

test('a new state in the same operation invalidates the old completion timer (#167)', () => {
  const { life, values, tasks } = fixture();
  const token = life.begin();
  life.update(token, { kind: 'success', message: 'Copied' });
  life.update(token, { kind: 'error', message: 'Open failed' });
  tasks[0]!.run();
  assert.equal(values.at(-1)?.message, 'Open failed');
  assert.ok(tasks[0]!.cancelled);
});

test('clear and dispose invalidate pending results and release scheduled callbacks (#167)', () => {
  const { life, values, tasks } = fixture();
  const token = life.begin();
  life.update(token, { kind: 'success', message: 'Copied' });
  life.clear();
  assert.equal(life.current(token), false);
  assert.equal(life.update(token, { kind: 'error', message: 'Late' }), false);
  const next = life.begin();
  life.update(next, { kind: 'success', message: 'Copied again' });
  life.dispose();
  const length = values.length;
  life.begin();
  life.update(next, { kind: 'success', message: 'After unmount' });
  for (const task of tasks) task.run();
  assert.equal(values.length, length);
  assert.ok(tasks.every(task => task.cancelled));
});

test('independent surfaces have no shared queue or generation (#167)', () => {
  const first = fixture(), second = fixture();
  first.life.update(first.life.begin(), { kind: 'error', message: 'Connection' });
  second.life.update(second.life.begin(), { kind: 'success', message: 'Copied' });
  second.tasks[0]!.run();
  assert.equal(first.values.at(-1)?.kind, 'error');
  assert.equal(second.values.at(-1), null);
});

test('message action owners reuse the completion scheduler without a second TTL (#167)', () => {
  const { clock, tasks } = fixture();
  let expired = 0;
  const cancel = scheduleCompletion(() => expired++, clock);
  assert.equal(tasks[0]!.delay, 1500);
  cancel();
  assert.ok(tasks[0]!.cancelled);
  assert.equal(expired, 0);
});
