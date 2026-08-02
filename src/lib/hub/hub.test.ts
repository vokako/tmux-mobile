import test from 'node:test';
import assert from 'node:assert/strict';
import { mergeMessages, statuslineWindows, stateDotColor, timelineItems } from './hub.ts';

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

test('timelineItems respects the feed level and collapses duplicate tool lines', () => {
  const feed = [{ ts: 100, from: 'lead', body: 'hi' }];
  const activity = [
    { ts: 50, window: 1, kind: 'status', text: 'waiting' },
    { ts: 150, window: 1, kind: 'tool', text: 'Edit a.rs' },
    { ts: 160, window: 1, kind: 'tool', text: 'Edit a.rs' },   // pre+post dup
    { ts: 170, window: 1, kind: 'tool', text: 'Edit b.rs' },
    { ts: 200, window: 1, kind: 'notif', text: 'completed' },
  ] as const satisfies readonly { ts: number; window: number; kind: 'tool' | 'status' | 'notif'; text: string }[];
  assert.equal(timelineItems(feed, activity, 'chat').length, 1, 'chat = messages only');
  const status = timelineItems(feed, activity, 'status');
  assert.deepEqual(status.map((i) => i.type), ['activity', 'msg', 'activity'], 'status + notif, no tools');
  const tools = timelineItems(feed, activity, 'tools');
  assert.equal(tools.length, 5, 'dup tool line collapsed');
  assert.deepEqual(tools.map((i) => i.ts), [50, 100, 150, 170, 200], 'sorted by ts');
});
