import test from 'node:test';
import assert from 'node:assert/strict';
import { mergeMessages, statuslineWindows, stateDotColor, feedBlocks, systemLine, pickLead, addressed, isSelfReport, toolEventParts, splitImages, isDirectUrl, fmtElapsed, unreadSenders, stoppedAgents, toolColor, pickAnchor } from './hub.ts';
import type { HubActivityEvent, HubAgent } from '../core/ws.ts';

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

const ag = (a: Partial<HubAgent>): HubAgent => ({
  window: 1, name: 'a', command: '', agent: 'kiro', managed: true, state: 'idle', detail: '', since: 0, ...a,
});

test('statuslineWindows marks the terminal window with tmux notation', () => {
  const agents = [
    ag({ window: 2, name: 'reviewer', agent: 'claude', state: 'working' }),
    ag({ window: 1, name: 'lead', agent: 'kiro' }),
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
    ev({ ts: 200, kind: 'notif', text: 'input_required' }),
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

test('a finished turn is not a row — the reply already is', () => {
  const feed = [{ ts: 200, from: 'dev', body: 'done' }];
  const completed = [ev({ ts: 210, kind: 'notif', text: 'completed' })];
  assert.deepEqual(feedBlocks(feed, completed, 'tools').map((b) => b.type), ['msg'],
    '"finished a turn" after every answer is noise');
  // The lifecycle events that mean a human is needed DO show.
  for (const kind of ['permission_required', 'input_required', 'failed']) {
    const blocks = feedBlocks([], [ev({ ts: 1, kind: 'notif', text: kind })], 'status');
    assert.deepEqual(blocks.map((b) => b.type), ['note'], kind);
  }
});

test('toolColor buckets tools by what they do, across backend spellings', () => {
  const bucket = (t: string) => toolColor(t);
  assert.equal(bucket('fs_write'), bucket('Edit'), 'both change things');
  assert.equal(bucket('execute_bash'), bucket('Bash'), 'both run things');
  assert.equal(bucket('web_search'), bucket('grep'), 'both look things up');
  assert.equal(bucket('fs_read'), bucket('Read'), 'both read things');
  assert.notEqual(bucket('fs_write'), bucket('fs_read'), 'change and read are not the same');
  assert.equal(toolColor(''), 'var(--text3)', 'no name, no claim');
  assert.equal(toolColor('mystery_tool'), 'var(--text2)', 'unknown stays neutral');
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

test('pickLead: a remembered choice wins while that agent is present', () => {
  const agents = [ag({ window: 1, name: 'dev' }), ag({ window: 2, name: 'qa' })];
  assert.equal(pickLead(agents, [], 'qa'), 'qa');
  assert.equal(pickLead(agents, [], 'gone'), 'dev', 'a departed agent falls back to the rule');
});

test('pickLead: one agent needs no rule, several prefer the one that can hire', () => {
  assert.equal(pickLead([ag({ name: 'solo' })], []), 'solo');
  const agents = [ag({ window: 3, name: 'dev' }), ag({ window: 2, name: 'boss' })];
  assert.equal(pickLead(agents, [{ name: 'boss', can_hire: true }]), 'boss', 'can_hire IS the lead role');
  assert.equal(pickLead(agents, []), 'boss', 'no lead defined → lowest window, stable not arbitrary');
});

test('pickLead ignores direct windows and empty rooms', () => {
  assert.equal(pickLead([ag({ name: 'byhand', managed: false })], []), '', 'direct windows are not participants');
  assert.equal(pickLead([], []), '');
});

test('addressed prefixes the recipient but never rewrites an explicit @', () => {
  assert.equal(addressed('  ship it  ', 'dev'), '@dev ship it', 'no @ ceremony for the lead');
  assert.equal(addressed('@qa look at this', 'dev'), '@qa look at this', 'the user addressed someone by hand');
  assert.equal(addressed('ship it', ''), 'ship it', 'no recipient = the whole room');
  assert.equal(addressed('   ', 'dev'), '', 'nothing to send');
});

test('toolEventParts splits the name off legacy glued-together events', () => {
  // Old server: no tool field, text = "shell tmm send …". Measured live — this
  // shape is why names stayed grey and self-report filtering missed.
  assert.deepEqual(toolEventParts(ev({ kind: 'tool', text: 'shell tmm send "@human hi" 2>&1' })),
    { tool: 'shell', text: 'tmm send "@human hi" 2>&1' });
  assert.deepEqual(toolEventParts(ev({ kind: 'tool', text: 'bash ls -la' })),
    { tool: 'bash', text: 'ls -la' });
  // A modern event with its own tool field passes through untouched.
  assert.deepEqual(toolEventParts(ev({ kind: 'tool', tool: 'fs_read', text: '/src/lib.rs' })),
    { tool: 'fs_read', text: '/src/lib.rs' });
  // No known lead-in: nothing is invented.
  assert.deepEqual(toolEventParts(ev({ kind: 'tool', text: 'Read /src/lib.rs' })),
    { tool: '', text: 'Read /src/lib.rs' });
});

test('an agent reporting through tmm is not also a tool row', () => {
  const feed = [{ ts: 150, from: 'dev', body: 'done looking' }];
  const activity = [
    ev({ ts: 100, kind: 'tool', tool: 'execute_bash', text: '/home/u/.local/bin/tmm send --agent dev "on it"' }),
    ev({ ts: 120, kind: 'tool', tool: 'execute_bash', text: 'tmm status working' }),
    ev({ ts: 130, kind: 'tool', tool: 'execute_bash', text: 'tmm done "shipped"' }),
    ev({ ts: 140, kind: 'tool', tool: 'execute_bash', text: 'tmm log --limit 5' }),
  ];
  assert.deepEqual(feedBlocks(feed, activity, 'tools').map((b) => b.type), ['msg'],
    'the message IS the report — the call that produced it is not a second event');
  // Agents chain the report onto one line, and old events glue the tool name on:
  // both are still nothing but self-report, so both are dropped.
  const chained = [
    ev({ ts: 100, kind: 'tool', tool: 'execute_bash', text: 'tmm send "@human ok" 2>&1; tmm status working "x"' }),
    ev({ ts: 110, kind: 'tool', text: 'shell tmm send "@human 好的，转告给 crew。" 2>&1' }),
  ];
  assert.deepEqual(feedBlocks(feed, chained, 'tools').map((b) => b.type), ['msg'],
    'chained and legacy-glued self-reports are equally invisible');
  // But a chain that also does real work keeps its row.
  const mixed = [ev({ ts: 100, kind: 'tool', tool: 'execute_bash', text: 'tmm send "done" && make deploy' })];
  assert.deepEqual(feedBlocks([], mixed, 'tools').map((b) => b.type), ['steps'],
    'a self-report chained to real work still has something to show');
  // Anything with no other trace in the chat stays visible.
  const kept = [
    ev({ ts: 100, kind: 'tool', tool: 'execute_bash', text: 'tmm task start dev-server' }),
    ev({ ts: 110, kind: 'tool', tool: 'execute_bash', text: 'git commit -m "tmm send fix"' }),
    ev({ ts: 120, kind: 'tool', tool: 'fs_read', text: '/src/lib.rs' }),
  ];
  const steps = feedBlocks([], kept, 'tools');
  assert.equal(steps.length, 1);
  assert.equal(steps[0]?.type === 'steps' && steps[0].events.length, 3, 'only self-reports are dropped');
  assert.equal(isSelfReport(ev({ kind: 'status', text: 'tmm send x' })), false, 'only tool events');
});

test('splitImages pulls image references out of the prose', () => {
  const one = splitImages('look at this\n![](/tmp/shot.png)');
  assert.equal(one.text, 'look at this');
  assert.deepEqual(one.images, ['/tmp/shot.png']);
  // Several, with alt text and a title, in prose and at the end.
  const many = splitImages('before ![alt](https://x/y.png "t") after\n![](~/a.jpg)');
  assert.deepEqual(many.images, ['https://x/y.png', '~/a.jpg']);
  assert.equal(many.text, 'before  after');
  // No images: the body is untouched (markdown and all).
  const none = splitImages('# title\n**bold** and a (paren)');
  assert.deepEqual(none.images, []);
  assert.equal(none.text, '# title\n**bold** and a (paren)');
  assert.deepEqual(splitImages(undefined), { text: '', images: [] });
});

test('isDirectUrl separates what a webview can load from what needs the file service', () => {
  for (const ok of ['http://x/y.png', 'https://x/y.png', 'data:image/png;base64,AA', 'blob:abc']) {
    assert.equal(isDirectUrl(ok), true, ok);
  }
  for (const path of ['/tmp/shot.png', '~/shot.png', './rel.png', 'C:\\x.png']) {
    assert.equal(isDirectUrl(path), false, path);
  }
});

test('fmtElapsed is compact at every magnitude', () => {
  const t = (secsAgo: number) => fmtElapsed(1_000_000, (1_000_000 + secsAgo) * 1000);
  assert.equal(t(0), '0s');
  assert.equal(t(45), '45s');
  assert.equal(t(134), '2m14s');
  assert.equal(t(3600 + 5 * 60), '1h05m');
  assert.equal(t(26 * 3600), '1d02h');
  assert.equal(fmtElapsed(0, Date.now()), '', 'no timestamp, no readout');
});

test('unreadSenders marks who replied after the user last looked', () => {
  const feed = [
    { ts: 100, from: 'human', body: 'go' },
    { ts: 200, from: 'dev', body: 'done' },
    { ts: 300, from: 'qa', body: 'looks fine' },
  ];
  assert.deepEqual([...unreadSenders(feed, 150)], ['dev', 'qa']);
  assert.deepEqual([...unreadSenders(feed, 250)], ['qa'], 'only what is newer than seen');
  assert.deepEqual([...unreadSenders(feed, 300)], [], 'caught up');
  assert.deepEqual([...unreadSenders([{ ts: 400, from: 'human' }], 0)], [],
    'your own message is never unread');
  // A lifecycle line is posted under the agent's name but is not a reply.
  assert.deepEqual([...unreadSenders([{ ts: 400, from: 'dev', body: '[tmm] stopped dev' }], 0)], [],
    'stopping an agent is not the agent answering you');
});

test('stoppedAgents lists declared agents with no live window', () => {
  const slots = [
    { window_name: 'dev', kind: 'agent' },
    { window_name: 'qa', kind: 'agent' },
    { window_name: 'shell', kind: 'shell' },   // a shell is not an agent
  ];
  const live = [ag({ name: 'dev' })];
  assert.deepEqual(stoppedAgents(slots, live), ['qa'], 'declared, not running');
  assert.deepEqual(stoppedAgents(slots, [ag({ name: 'dev' }), ag({ name: 'qa' })]), []);
  assert.deepEqual(stoppedAgents(undefined, live), [], 'a project with no declaration');
  // Case is not the contract: SlotKind serializes lowercase, but be tolerant.
  assert.deepEqual(stoppedAgents([{ window_name: 'x', kind: 'Agent' }], []), ['x']);
});

test('a tool call that ties with a reply is ordered before it', () => {
  // The server stamps a hook event when it CONSUMES the file, so a turn's last
  // tool call and its auto-posted reply can share a millisecond. A reply is what
  // ends a turn, so the work sorts first — the symptom of getting this wrong is
  // tool calls rendered after the answer they produced.
  const feed = [{ ts: 1000, from: 'dev', body: 'done, see above' }];
  const activity = [
    ev({ ts: 1000, kind: 'tool', tool: 'fs_write', text: 'a.rs' }),
    ev({ ts: 900, kind: 'tool', tool: 'fs_read', text: 'a.rs' }),
  ];
  const blocks = feedBlocks(feed, activity, 'tools');
  assert.deepEqual(blocks.map((b) => b.type), ['steps', 'msg'], 'work, then the reply');
  const steps = blocks[0];
  assert.deepEqual(
    steps?.type === 'steps' ? steps.events.map((e) => e.text) : [],
    ['a.rs', 'a.rs'],
    'both calls stayed in one group, oldest first',
  );
  // A message that genuinely precedes a tool call still comes first.
  const after = feedBlocks([{ ts: 500, from: 'human', body: 'go' }], activity, 'tools');
  assert.deepEqual(after.map((b) => b.type), ['msg', 'steps']);
});

test('pickAnchor keeps ONE real bubble until another naturally enters', () => {
  const items = [
    { key: 'a', top: 0, height: 40 },
    { key: 'b', top: 100, height: 40 },
    { key: 'c', top: 300, height: 40 },
    { key: 'd', top: 500, height: 40 },
  ];
  const pick = (
    scrollTop: number,
    direction: 'up' | 'down',
    current = { key: '', edge: '' as 'top' | 'bottom' | '' },
  ) => pickAnchor(items, scrollTop, 100, 600, direction, current);

  // Scrolling down: the real bubble enters at the bottom, travels, then holds
  // the top. In the empty gap it remains b — it does NOT swap invisibly to c.
  assert.deepEqual(pick(0, 'down'), { key: 'a', edge: 'top' });
  assert.deepEqual(pick(50, 'down', { key: 'a', edge: 'top' }), { key: 'b', edge: 'top' });
  assert.deepEqual(pick(200, 'down', { key: 'b', edge: 'top' }), { key: 'b', edge: 'top' });
  assert.deepEqual(pick(220, 'down', { key: 'b', edge: 'top' }), { key: 'c', edge: 'top' });

  // Scrolling up is symmetric: c keeps holding the bottom through the gap,
  // then b becomes active only when its real bubble enters from the top.
  assert.deepEqual(pick(220, 'up', { key: 'c', edge: 'top' }), { key: 'c', edge: 'bottom' });
  assert.deepEqual(pick(170, 'up', { key: 'c', edge: 'bottom' }), { key: 'c', edge: 'bottom' });
  assert.deepEqual(pick(120, 'up', { key: 'c', edge: 'bottom' }), { key: 'b', edge: 'bottom' });

  // A direct page jump has no motion to preserve, so seed from the destination.
  assert.deepEqual(pick(200, 'down'), { key: 'b', edge: 'top' });
  assert.deepEqual(pick(200, 'up'), { key: 'c', edge: 'bottom' });
  assert.deepEqual(pickAnchor(items, 0, 600, 600, 'down'), { key: '', edge: '' });
  assert.deepEqual(pickAnchor([], 200, 100, 600, 'down'), { key: '', edge: '' });
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

test('lifecycle lines fold into one sys row and vanish at chat level', () => {
  const feed = [
    { id: 'a', ts: 100, from: 'human', body: 'hello' },
    { id: 'b', ts: 200, from: 'lead', body: '[tmm] stopped lead' },
    { id: 'c', ts: 210, from: 'lead', body: '[tmm] restarted lead' },
    { id: 'd', ts: 300, from: 'lead', body: 'back to work' },
  ];
  // A stop followed by a restart is one fact: one row, both items.
  const blocks = feedBlocks(feed, [], 'tools');
  assert.deepEqual(blocks.map((b) => b.type), ['msg', 'sys', 'msg']);
  const sys = blocks[1];
  assert.deepEqual(sys?.type === 'sys' && sys.items, ['stopped lead', 'restarted lead']);
  // The chat-only level is the conversation, not the app's record.
  assert.deepEqual(feedBlocks(feed, [], 'chat').map((b) => b.type), ['msg', 'msg']);
  // A real message between two lifecycle lines keeps them apart.
  const split = feedBlocks([
    { id: 'a', ts: 100, from: 'x', body: '[tmm] spawned dev' },
    { id: 'b', ts: 200, from: 'x', body: 'hi' },
    { id: 'c', ts: 300, from: 'x', body: '[tmm] done' },
  ], [], 'status');
  assert.deepEqual(split.map((b) => b.type), ['sys', 'msg', 'sys']);
});

test('an unconfirmed delivery is visible even at the chat-only level', () => {
  const activity = [ev({ ts: 100, kind: 'warn', text: 'unconfirmed: [tmm chat] human: @dev hi' })];
  const blocks = feedBlocks([{ ts: 50, from: 'human', body: '@dev hi' }], activity, 'chat');
  assert.deepEqual(blocks.map((b) => b.type), ['msg', 'note'], 'a failed delivery is not opt-in detail');
});
