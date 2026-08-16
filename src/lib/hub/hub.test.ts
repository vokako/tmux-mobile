import test from 'node:test';
import assert from 'node:assert/strict';
import { mergeMessages, statuslineWindows, stateDotColor, feedBlocks, systemLine } from './hub.ts';
import type { HubActivityEvent } from '../core/ws.ts';

const ev = (e: Partial<HubActivityEvent>): HubActivityEvent => ({
  ts: 0, window: 1, kind: 'tool', text: '', ...e,
});

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

test('feedBlocks respects the feed level and collapses duplicate tool lines', () => {
  const feed = [{ ts: 100, from: 'lead', body: 'hi' }];
  const activity = [
    ev({ ts: 50, kind: 'status', text: 'waiting' }),
    ev({ ts: 150, kind: 'tool', text: 'Edit a.rs' }),
    ev({ ts: 160, kind: 'tool', text: 'Edit a.rs' }),   // pre+post dup
    ev({ ts: 170, kind: 'tool', text: 'Edit b.rs' }),
    ev({ ts: 200, kind: 'notif', text: 'completed' }),
  ];
  assert.equal(feedBlocks(feed, activity, 'chat').length, 1, 'chat = messages only');
  const status = feedBlocks(feed, activity, 'status');
  assert.deepEqual(status.map((i) => i.type), ['note', 'msg', 'note'], 'status + notif, no tools');
  const tools = feedBlocks(feed, activity, 'tools');
  assert.deepEqual(tools.map((i) => i.type), ['note', 'msg', 'steps', 'note'], 'the two tool calls became one group');
  const steps = tools.find((i) => i.type === 'steps');
  assert.deepEqual(steps?.events.map((e) => e.text), ['Edit a.rs', 'Edit b.rs'], 'dup dropped, order kept');
  assert.deepEqual(tools.map((i) => i.ts), [50, 100, 150, 200], 'sorted by ts, group carries its first ts');
});

test('a reply between tool calls splits the group in two', () => {
  const feed = [{ ts: 150, from: 'dev', body: 'found it' }];
  const activity = [
    ev({ ts: 100, kind: 'tool', text: 'Read a.rs' }),
    ev({ ts: 200, kind: 'tool', text: 'Edit a.rs' }),
  ];
  const blocks = feedBlocks(feed, activity, 'tools');
  assert.deepEqual(blocks.map((b) => b.type), ['steps', 'msg', 'steps'], 'a group means "between two replies"');
});

test('concurrent windows never share a tool group', () => {
  const activity = [
    ev({ ts: 100, window: 1, kind: 'tool', text: 'Read a.rs' }),
    ev({ ts: 110, window: 2, kind: 'tool', text: 'Read b.rs' }),
    ev({ ts: 120, window: 1, kind: 'tool', text: 'Edit a.rs' }),
  ];
  const blocks = feedBlocks([], activity, 'tools');
  assert.deepEqual(blocks.map((b) => b.type === 'steps' && b.window), [1, 2, 1], 'per-window runs');
  // Per-window dedup: window 2 repeating window 1's line is not a duplicate.
  const same = feedBlocks([], [
    ev({ ts: 100, window: 1, kind: 'tool', text: 'Read a.rs' }),
    ev({ ts: 110, window: 2, kind: 'tool', text: 'Read a.rs' }),
  ], 'tools');
  assert.equal(same.length, 2, 'two windows doing the same thing are two facts');
});

test('an echoed prompt marks its message delivered instead of repeating it', () => {
  const feed = [
    { ts: 100, from: 'human', body: '@dev ship it' },
    { ts: 400, from: 'human', body: '@dev and update the docs' },
  ];
  const activity = [
    ev({ ts: 200, kind: 'prompt', via: 'app', text: '[tmm chat] human: @dev ship it' }),
  ];
  for (const level of ['chat', 'status', 'tools'] as const) {
    const blocks = feedBlocks(feed, activity, level);
    assert.deepEqual(blocks.map((b) => b.type), ['msg', 'msg'], `${level}: the echo is a receipt, not a row`);
    const [first, second] = blocks;
    assert.equal(first?.type === 'msg' && first.delivered, true, `${level}: first message confirmed`);
    assert.equal(second?.type === 'msg' && second.delivered, false, `${level}: the later one is not`);
  }
});

test('a prompt typed at the agent keyboard becomes its own input row', () => {
  const activity = [ev({ ts: 100, kind: 'prompt', via: 'local', text: 'fix the flaky test' })];
  assert.deepEqual(feedBlocks([], activity, 'status').map((b) => b.type), ['prompt']);
  // An app-origin echo with no matching message must not vanish either.
  const orphan = [ev({ ts: 100, kind: 'prompt', via: 'app', text: '[tmm chat] human: gone' })];
  assert.deepEqual(feedBlocks([], orphan, 'status').map((b) => b.type), ['prompt'], 'never silently dropped');
});

test('systemLine recognizes lifecycle lines and leaves prose alone', () => {
  assert.equal(systemLine('[tmm] spawned dev — fix the bug'), 'spawned dev — fix the bug');
  assert.equal(systemLine('[tmm] done'), 'done');
  // Rooms are persisted: the pre-2026-08 glyph spelling must not regress into
  // a chat bubble when an old room is reopened.
  assert.equal(systemLine('⚡ spawned dev'), 'spawned dev');
  assert.equal(systemLine('✔ done — shipped'), 'done — shipped');
  assert.equal(systemLine('I spawned a subprocess'), null, 'prose stays prose');
  assert.equal(systemLine(undefined), null);
});

test('an unconfirmed delivery is visible even at the chat-only level', () => {
  const activity = [ev({ ts: 100, kind: 'warn', text: 'unconfirmed: [tmm chat] human: @dev hi' })];
  const blocks = feedBlocks([{ ts: 50, from: 'human', body: '@dev hi' }], activity, 'chat');
  assert.deepEqual(blocks.map((b) => b.type), ['msg', 'note'], 'a failed delivery is not opt-in detail');
});
