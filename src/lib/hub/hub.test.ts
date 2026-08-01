import test from 'node:test';
import assert from 'node:assert/strict';
import { mergeMessages, statuslineWindows, stateDotColor } from './hub.ts';

test('mergeMessages dedupes by id and by content triple, sorts by ts', () => {
  const a = [{ id: '1', ts: 100, from: 'x', body: 'hi' }];
  const merged = mergeMessages(a, [
    { id: '1', ts: 100, from: 'x', body: 'hi' },          // dup by id
    { ts: 50, from: 'y', body: 'earlier' },                // no id — content key
    { ts: 50, from: 'y', body: 'earlier' },                // dup by content
    { id: '2', ts: 200, from: 'x', body: 'later' },
  ]);
  assert.equal(merged.length, 3);
  assert.deepEqual(merged.map((m) => m.ts), [50, 100, 200], 'oldest first');
});

test('statuslineWindows marks the terminal window with tmux notation', () => {
  const agents = [
    { window: 2, name: 'reviewer', command: '', agent: 'claude', state: 'working', detail: '', since: 0 },
    { window: 1, name: 'lead', command: '', agent: 'kiro', state: 'idle', detail: '', since: 0 },
  ];
  const wins = statuslineWindows(agents, 'blog:2.1');
  assert.deepEqual(wins.map((w) => w.label), ['1:lead', '2:reviewer*'], 'index order, * on current');
  assert.equal(wins[1]?.current, true);
});

test('every derived state has a dot color and unknown falls back', () => {
  for (const s of ['working', 'waiting', 'blocked', 'stuck', 'failed', 'shell', 'idle']) {
    assert.ok(stateDotColor(s).startsWith('var(--'), s);
  }
});
