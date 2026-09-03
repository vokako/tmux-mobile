import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { gapWalkStep, TAIL_GAP, bottomGap, tailAfterScroll, uploadImagePath, uploadFilePath, imageId, pastedFiles, isSessionStart, STEPS_ROWS, clampStepsRows, markLeadingMention, mergeMessages, stateDotColor, stateIsLive, feedBlocks, systemLine, sysParts, sysVerbColor, pickLead, addressed, isSelfReport, toolEventParts, splitImages, isDirectUrl, fmtElapsed, agoShort, unreadSenders, stoppedAgents, toolColor, pickAnchor, elideTail, ELIDE, slashCommand, commandPalette, KIRO_COMMANDS, OFFERED_COMMANDS, ctxColor, statusNote, noteStateColor, fuzzyRank, sameDay, draftUpdate, DRAFT_MAX, readlineEdit, squashWs, mentionsAgent, filterBlocks, foldLines, PHONE_FOLD_LINES, mergeStates, mergeEvents , boardLine, boardStatusColor, promptParts, touchContextMenu, perLineOf, modelLabel, echoContains, echoTruncated, PROMPT_ECHO_MAX } from './hub.ts';
import type { HubActivityEvent, HubAgent } from '../core/ws.ts';

const ev = (e: Partial<HubActivityEvent>): HubActivityEvent => ({
  ts: 0, window: 1, kind: 'tool', text: '', ...e,
});

test('gapWalkStep walks before_seq pages back to the poll cursor and no further', () => {
  const m = (seq: number) => ({ id: `m${seq}`, seq, ts: seq * 10 });
  const page = (from: number, to: number, has_more = true) => ({
    messages: Array.from({ length: to - from + 1 }, (_, i) => m(from + i)), has_more, oldest_seq: from,
  });
  // The poll started at ts 1000 (seq 100); the newest page was 151..250.
  // A page entirely newer than the cursor is all new, and the walk continues.
  let step = gapWalkStep(page(151, 250), 1000);
  assert.equal(step.newer.length, 100);
  assert.equal(step.next, 151);
  // A page that reaches the cursor keeps only the rows newer than it and stops.
  step = gapWalkStep(page(51, 150), 1000);
  assert.deepEqual(step.newer.map((x) => x.seq), Array.from({ length: 50 }, (_, i) => 101 + i));
  assert.equal(step.next, null);
  // The room's beginning stops the walk even when the page was all new.
  step = gapWalkStep(page(1, 20, false), 0);
  assert.equal(step.newer.length, 20);
  assert.equal(step.next, null);
  // Nothing on the page, no cursor, or no page at all: done, nothing new.
  assert.deepEqual(gapWalkStep({ messages: [], has_more: true, oldest_seq: 5 }, 0), { newer: [], next: null });
  assert.deepEqual(gapWalkStep({ messages: [m(3)], has_more: true }, 0), { newer: [m(3)], next: null });
  assert.deepEqual(gapWalkStep(null, 0), { newer: [], next: null });
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


test('every derived state has a dot color and unknown falls back', () => {
  for (const s of ['working', 'waiting', 'blocked', 'stuck', 'failed', 'shell', 'idle']) {
    assert.ok(stateDotColor(s).startsWith('var(--'), s);
  }
});

test('the RUNNING cue is worn by exactly the in-motion states', () => {
  // Colour alone could not carry running-vs-idle at 5–7px (owner, 2026-08-29),
  // so an in-motion dot also wears app.css `.live-dot` — halo + breathe. Which
  // states that is has ONE definition, or the sidebar chip, the roster card and
  // the recipient picker drift apart.
  assert.equal(stateIsLive('running'), true);
  assert.equal(stateIsLive('working'), true, "'working' is the pre-2026-08 name for running");
  // Everything else is NOT in motion — a resting, waiting or failed dot that
  // breathed would say a turn is open when none is.
  for (const s of ['idle', 'waiting', 'blocked', 'stuck', 'failed', 'shell', 'done', '']) {
    assert.equal(stateIsLive(s), false, s);
  }
  // The cue and the colour must agree on what "in motion" means: exactly the
  // states painted with the accent get it.
  for (const s of ['running', 'working', 'idle', 'waiting', 'failed', 'shell']) {
    assert.equal(stateIsLive(s), stateDotColor(s) === 'var(--accent)', s);
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
  // The bare `waiting` claim carries no note, so it is not a row — the derived
  // state already says that better than a word the agent typed.
  assert.deepEqual(status.map((i) => i.type), ['msg', 'note'], 'notif, no tools, no contentless claim');
  const tools = feedBlocks(feed, activity, 'tools');
  assert.deepEqual(tools.map((i) => i.type), ['msg', 'steps', 'note'], 'the two tool calls became one group');
  const steps = tools.find((i) => i.type === 'steps');
  assert.deepEqual(steps?.events.map((e) => e.text), ['Edit a.rs', 'Edit b.rs'], 'dup dropped, order kept');
  assert.deepEqual(tools.map((i) => i.ts), [100, 150, 200], 'sorted by ts, group carries its first ts');
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
  // With no window map a reply cannot be attributed, so it ends every run.
  assert.deepEqual(
    feedBlocks(feed, activity, 'tools').map((b) => b.type),
    ['steps', 'msg', 'steps'],
    'a group means "between two replies"',
  );
  // With one, it ends the lane it came from — the same split, now for a reason.
  assert.deepEqual(
    feedBlocks(feed, activity, 'tools', (from) => (from === 'dev' ? 1 : undefined)).map((b) => b.type),
    ['steps', 'msg', 'steps'],
  );
});

test('two agents working at once keep one lane each', () => {
  // The churn this replaces: folding only CONSECUTIVE events turned an
  // interleaved run into one group per call (owner report, 2026-08-19).
  const activity = [
    ev({ ts: 100, window: 1, kind: 'tool', text: 'Read a.rs' }),
    ev({ ts: 110, window: 2, kind: 'tool', text: 'Read b.rs' }),
    ev({ ts: 120, window: 1, kind: 'tool', text: 'Edit a.rs' }),
    ev({ ts: 130, window: 2, kind: 'tool', text: 'Edit b.rs' }),
    ev({ ts: 140, window: 1, kind: 'tool', text: 'Bash cargo test' }),
  ];
  const blocks = feedBlocks([], activity, 'tools');
  assert.deepEqual(blocks.map((b) => b.type === 'steps' && b.window), [1, 2], 'two lanes, not five rows');
  const w1 = blocks.find((b) => b.type === 'steps' && b.window === 1);
  const w2 = blocks.find((b) => b.type === 'steps' && b.window === 2);
  assert.deepEqual(
    w1?.type === 'steps' ? w1.events.map((e) => e.text) : [],
    ['Read a.rs', 'Edit a.rs', 'Bash cargo test'],
    'every call of that agent, in order',
  );
  assert.equal(w2?.type === 'steps' && w2.events.length, 2);
  // A lane is closed only by ITS OWN rows: another agent's reply is a different
  // conversation, not a boundary.
  const withReply = feedBlocks(
    [{ ts: 115, from: 'other', body: 'done over here' }],
    activity,
    'tools',
    (from) => (from === 'other' ? 2 : undefined),
  );
  const lanes = withReply.filter((b) => b.type === 'steps');
  assert.equal(lanes.length, 3, 'window 2 was split by its own reply, window 1 was not');
  assert.equal(lanes[0]?.type === 'steps' && lanes[0].events.length, 3, 'window 1 stayed whole');
});

test('concurrent windows never share a tool group', () => {
  const activity = [
    ev({ ts: 100, window: 1, kind: 'tool', text: 'Read a.rs' }),
    ev({ ts: 110, window: 2, kind: 'tool', text: 'Read b.rs' }),
    ev({ ts: 120, window: 1, kind: 'tool', text: 'Edit a.rs' }),
  ];
  const blocks = feedBlocks([], activity, 'tools');
  assert.deepEqual(blocks.map((b) => b.type === 'steps' && b.window), [1, 2], 'one lane per window');
  // Per-window dedup: window 2 repeating window 1's line is not a duplicate.
  const same = feedBlocks([], [
    ev({ ts: 100, window: 1, kind: 'tool', text: 'Read a.rs' }),
    ev({ ts: 110, window: 2, kind: 'tool', text: 'Read a.rs' }),
  ], 'tools');
  assert.equal(same.length, 2, 'two windows doing the same thing are two facts');
});

test('a status note is a spoken line at every level, and never breaks a lane', () => {
  // What the owner could not see: hooks report that a turn is open, never what
  // it is about (2026-08-19). The note is the only account of the work.
  const activity = [
    ev({ ts: 100, window: 1, kind: 'tool', text: 'Read a.rs' }),
    { ts: 110, window: 1, kind: 'status', text: '重写状态机', state: 'working' } as HubActivityEvent,
    ev({ ts: 120, window: 1, kind: 'tool', text: 'Edit a.rs' }),
  ];
  for (const level of ['chat', 'status', 'tools'] as const) {
    const kinds = feedBlocks([], activity, level).map((b) => b.type);
    assert.ok(kinds.includes('progress'), `visible at the ${level} level: ${kinds}`);
  }
  const blocks = feedBlocks([], activity, 'tools');
  // One lane, not two: the note was written in the middle of the run.
  const lanes = blocks.filter((b) => b.type === 'steps');
  assert.equal(lanes.length, 1, 'a progress note is not a boundary');
  assert.equal(lanes[0]?.type === 'steps' && lanes[0].events.length, 2);
  const prog = blocks.find((b) => b.type === 'progress');
  assert.equal(prog?.type === 'progress' && prog.text, '重写状态机');
  assert.equal(prog?.type === 'progress' && prog.state, 'working');

  // A claim with no note says nothing the derived state does not already say —
  // the server sends an empty text for it, and an older one echoed the state
  // word into the text.
  for (const bareEv of [
    { ts: 100, window: 1, kind: 'status', text: '', state: 'working' } as HubActivityEvent,
    { ts: 100, window: 1, kind: 'status', text: 'working', state: 'working' } as HubActivityEvent,
    ev({ ts: 100, kind: 'status', text: 'waiting' }),
  ]) {
    assert.deepEqual(feedBlocks([], [bareEv], 'tools'), [], `no content, no row: ${JSON.stringify(bareEv)}`);
  }

  // Rooms and rings outlive a build: the old glued shape still parses.
  const legacy = feedBlocks([], [ev({ ts: 100, kind: 'status', text: 'blocked — no creds' })], 'chat');
  assert.equal(legacy.length, 1);
  assert.equal(legacy[0]?.type === 'progress' && legacy[0].state, 'blocked');
  assert.equal(legacy[0]?.type === 'progress' && legacy[0].text, 'no creds');
});

test('elideTail truncates a long user message at the rear, marker inline', () => {
  const lines = Array.from({ length: 20 }, (_, i) => `line ${i + 1}`).join('\n');
  const out = elideTail(lines, 6);
  const rows = out.split('\n');
  // Rear truncation, and the marker is GLUED to the last kept line — never a
  // line of its own (owner, 2026-08-27: "直接后截断的形式 最后文末不用换行三个
  // 点 中间不要了").
  assert.equal(rows.length, 6, 'the marker costs no extra line');
  assert.equal(rows[0], 'line 1');
  assert.equal(rows.at(-1), `line 6${ELIDE}`);
  assert.ok(!out.includes(`\n${ELIDE}`), 'no newline before the marker');

  // Already short enough: returned unchanged, by identity, so the caller can
  // skip re-rendering.
  const short = 'one\ntwo';
  assert.equal(elideTail(short, 6), short);
  assert.equal(elideTail('', 6), '');

  // A single paragraph cannot be cut by lines; it is cut by characters, on a
  // word boundary, marker still inline.
  const para = `${'alpha '.repeat(60)}END`;
  const cut = elideTail(para, 3, 20);
  assert.ok(cut.endsWith(ELIDE), cut);
  assert.ok(cut.startsWith('alpha'), cut);
  assert.ok(cut.length < para.length / 2, `much shorter: ${cut.length} vs ${para.length}`);
  assert.ok(cut.slice(0, -ELIDE.length).endsWith('alpha'), 'no word sliced in half');

  // A cut that drops a closing fence must not swallow the rest in a code
  // block.
  const fenced = ['```js', ...Array.from({ length: 12 }, (_, i) => `const x${i} = 1;`), '```', 'after'].join('\n');
  const fcut = elideTail(fenced, 5);
  assert.equal((fcut.match(/^```/gmu) ?? []).length % 2, 0, `fences balanced:\n${fcut}`);
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

test('a multi-line message still confirms when its echo drifts in whitespace', () => {
  // The delivered line rides tmux send-keys and the TUI's composer before the
  // hook echoes it back — a newline in the body does not survive that round
  // trip byte-for-byte (owner, 2026-08-22: "发送内容有换行 好像就不会被confirm"),
  // and tmux in extended-keys mode DROPPED the raw \n outright, gluing the
  // lines together in the echo (owner, 2026-08-24: "多行内容…没办法正确已读").
  // Matching is whitespace-BLIND on both sides (squashWs — the server's
  // strip_ws twin, same cases): any rendering of a break forgives — space,
  // wrap, or nothing at all — while character changes do not.
  assert.equal(squashWs('a\nb\r\n  c\td '), 'abcd');
  assert.equal(squashWs('AgenticAI\nAgentic AI基础设施'), 'AgenticAIAgenticAI基础设施');
  const feed = [
    { ts: 100, from: 'human', body: '@dev line one\nline two\n  line three' },
    { ts: 150, from: 'human', body: '@dev alpha beta' },
  ];
  const activity = [
    ev({ ts: 200, kind: 'prompt', via: 'app', text: '[tmm chat] human: @dev line one line two  line three' }),
    ev({ ts: 250, kind: 'prompt', via: 'app', text: '[tmm chat] human: @dev alpha gamma' }),
  ];
  const blocks = feedBlocks(feed, activity, 'chat');
  const [a, b] = blocks.filter((x) => x.type === 'msg');
  assert.equal(a?.type === 'msg' && a.delivered, true, 'newline → space still confirms');
  assert.equal(b?.type === 'msg' && b.delivered, false, 'different words still refuse');
});

test('one prompt carrying several queued lines confirms all of them', () => {
  // A busy agent queues what we type and can submit more than one line when the
  // turn ends. Marking only the newest left the earlier message hollow for ever —
  // and, before the server kept a QUEUE of typed lines, made it come back as an
  // "input" row instead of a receipt (owner, 2026-08-20).
  const feed = [
    { ts: 100, from: 'human', body: '@dev first thing' },
    { ts: 200, from: 'human', body: '@dev second thing' },
    { ts: 300, from: 'human', body: '@dev not in that prompt' },
  ];
  const activity = [
    ev({ ts: 900, kind: 'prompt', via: 'app', text: '[tmm chat] human: @dev first thing\n[tmm chat] human: @dev second thing' }),
  ];
  const blocks = feedBlocks(feed, activity, 'tools');
  assert.deepEqual(blocks.map((b) => b.type), ['msg', 'msg', 'msg'], 'still a receipt, not a row');
  const [a, b, c] = blocks;
  assert.equal(a?.type === 'msg' && a.delivered, true);
  assert.equal(b?.type === 'msg' && b.delivered, true);
  assert.equal(c?.type === 'msg' && c.delivered, false, 'it was not in that prompt');
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

test('addressed always keeps the chip; body mentions ride along', () => {
  assert.equal(addressed('  ship it  ', 'dev'), '@dev ship it', 'no @ ceremony for the lead');
  assert.equal(addressed('ship it', ''), 'ship it', 'no recipient = the whole room');
  assert.equal(addressed('   ', 'dev'), '', 'nothing to send');
  // The chip survives an @ in the body — the server delivers to BOTH.
  assert.equal(addressed('ping @qa about the diff', 'dev'), '@dev ping @qa about the diff', 'mid-body mention never defeats the chip');
  assert.equal(addressed('mail root@host about it', 'dev'), '@dev mail root@host about it', 'an email is not an address');
  assert.equal(addressed('@qa look at this', 'dev'), '@dev @qa look at this', 'a hand-typed lead rides along too');
  // ...but the SAME recipient at the head is not doubled.
  assert.equal(addressed('@dev go', 'dev'), '@dev go', 'no @dev @dev');
  assert.equal(addressed('@dev, go', 'dev'), '@dev, go', 'punctuated head counts');
  assert.equal(addressed('@developer go', 'dev'), '@dev @developer go', 'prefix match is word-bounded');
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
  // BARE refs — how agents actually answer (owner, 2026-08-26: "返回图片
  // 路径或者 url，然后你正确读图渲染出来"). Alone on a line: folds away.
  const line = splitImages('看这里\n/tmp/shot.png\n完成');
  assert.equal(line.text, '看这里\n\n完成');
  assert.deepEqual(line.images, ['/tmp/shot.png']);
  // Inside prose: the sentence stays readable AND the image renders.
  const prose = splitImages('截图在 /tmp/shot.png 里，看 https://x.io/i.jpg?w=2 这张');
  assert.equal(prose.text, '截图在 /tmp/shot.png 里，看 https://x.io/i.jpg?w=2 这张');
  assert.deepEqual(prose.images, ['/tmp/shot.png', 'https://x.io/i.jpg?w=2']);
  // The same ref via markdown AND bare renders once.
  assert.deepEqual(splitImages('![](/x/a.webp)\n/x/a.webp').images, ['/x/a.webp']);
  // Not images: plain paths, and a path fragment after a space never yields
  // a bogus tail segment.
  assert.deepEqual(splitImages('run /usr/bin/env and /etc/passwd').images, []);
  assert.deepEqual(splitImages('at /tmp/a b/shot.png ok').images, []);
  // ~ paths and = boundaries count.
  assert.deepEqual(splitImages('看 ~/pics/cat.jpeg 这张').images, ['~/pics/cat.jpeg']);
  assert.deepEqual(splitImages('out=/o/f.webp done').images, ['/o/f.webp']);
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

test('agoShort speaks sidebar precision: one unit, coarser as it ages', () => {
  // A row's last-reply time is a summary, not a running timer — one unit is
  // legible in a 40px-wide slot where fmtElapsed's "2h14m" is not (owner,
  // 2026-08-24: the project list should show "上次回复的时间").
  const t = (secsAgo: number) => agoShort(1_000_000_000, (1_000_000 + secsAgo) * 1000);
  assert.equal(t(0), 'now');
  assert.equal(t(45), 'now', 'under a minute is not worth counting');
  assert.equal(t(134), '2m');
  assert.equal(t(3600 + 5 * 60), '1h');
  assert.equal(t(26 * 3600), '1d');
  assert.equal(t(9 * 86400), '9d');
  assert.equal(agoShort(0, Date.now()), '', 'a room that never spoke shows nothing');
});

test('the tool-lane cap is a setting with a floor, a ceiling and a default', () => {
  // "工具调用最大显示的行数应该也变成一个可配置的参数。现在默认把这个参数配置
  // 成 5 行" (owner, 2026-08-24). The clamp guards BOTH the setter and what an
  // old localStorage entry feeds back on load.
  assert.equal(STEPS_ROWS, 5, 'the default the owner asked for');
  assert.equal(clampStepsRows(5), 5);
  assert.equal(clampStepsRows('12'), 12, 'localStorage hands back a string');
  assert.equal(clampStepsRows(1), 3, 'one or two rows cannot show a run');
  assert.equal(clampStepsRows(999), 30, 'past ~30 the cap caps nothing');
  assert.equal(clampStepsRows('junk'), STEPS_ROWS, 'garbage falls to the default');
  assert.equal(clampStepsRows(7.6), 8, 'fractions land on whole rows');
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

test('a lifecycle line splits into a highlighted verb and its subject', () => {
  // The reason this exists: folded rows were joined with a `·` and read as one
  // run-on grey string ("removed k spawned", owner 2026-08-24). One grammar for
  // every narrated line — who, what happened, detail ("都用统一的 ui 来展示…包括
  // agent 的名字，状态，或者发送的指令", owner 2026-08-24).
  assert.deepEqual(sysParts('spawned k'), { verb: 'spawned', who: 'k', text: '', cmd: false });
  assert.deepEqual(sysParts('removed k'), { verb: 'removed', who: 'k', text: '', cmd: false });
  // The spawn brief follows the em-dash; the dash itself is layout, not content.
  assert.deepEqual(sysParts('spawned dev — fix the bug'), { verb: 'spawned', who: 'dev', text: 'fix the bug', cmd: false });
  // A verb with nothing after it (`[tmm] done`) names nobody.
  assert.deepEqual(sysParts('done'), { verb: 'done', who: '', text: '', cmd: false });
  // A /command record (`[tmm] {text} → {targets}`) splits into the same three
  // atoms: the command is the action, the targets are the who, the arguments are
  // the detail — badge + args read back as exactly the line that was typed.
  assert.deepEqual(sysParts('/model claude-4.5 → dev, reviewer'), { verb: '/model', who: 'dev, reviewer', text: 'claude-4.5', cmd: true });
  assert.deepEqual(sysParts('/clear → dev'), { verb: '/clear', who: 'dev', text: '', cmd: true });
  // The typed text may contain an arrow; the split is the LAST one — the one
  // hub_command appended.
  assert.deepEqual(sysParts('/compact a → b → dev'), { verb: '/compact', who: 'dev', text: 'a → b', cmd: true });
  // An unknown shape is left WHOLE — a guessed verb or name would truncate it.
  assert.deepEqual(sysParts('agent k left the building'), { verb: '', who: '', text: 'agent k left the building', cmd: false });
  assert.deepEqual(sysParts(undefined), { verb: '', who: '', text: '', cmd: false });
});

test('a lifecycle verb speaks the one status colour language', () => {
  // Same vocabulary as noteStateColor/stateDotColor: accent = in motion,
  // green = ended well, grey = at rest, red = destructive.
  assert.equal(sysVerbColor('spawned'), 'var(--accent)');
  assert.equal(sysVerbColor('restarted'), 'var(--accent)');
  assert.equal(sysVerbColor('done'), 'var(--status-ok)');
  assert.equal(sysVerbColor('stopped'), 'var(--text3)');
  assert.equal(sysVerbColor('removed'), 'var(--status-danger)');
  // Never a literal colour: both themes read the same tokens.
  for (const v of ['spawned', 'stopped', 'removed', 'done', 'interrupted', 'wat', '']) {
    assert.match(sysVerbColor(v), /^var\(--|^color-mix\(/u, `${v} must resolve through a token`);
  }
});

test('an unconfirmed delivery is visible even at the chat-only level', () => {
  const activity = [ev({ ts: 100, kind: 'warn', text: 'unconfirmed: [tmm chat] human: @dev hi' })];
  const blocks = feedBlocks([{ ts: 50, from: 'human', body: '@dev hi' }], activity, 'chat');
  assert.deepEqual(blocks.map((b) => b.type), ['msg', 'note'], 'a failed delivery is not opt-in detail');
});

test('markLeadingMention wraps only a leading address', () => {
  assert.equal(
    markLeadingMention('<p>@lead 查一下状态</p>'),
    '<p><span class="m-to">@lead</span> 查一下状态</p>',
  );
  assert.equal(
    markLeadingMention('<p>@all everyone</p>'),
    '<p><span class="m-to">@all</span> everyone</p>',
  );
  // a bare address with nothing after it
  assert.equal(markLeadingMention('<p>@human</p>'), '<p><span class="m-to">@human</span></p>');
});

test('markLeadingMention leaves non-address mentions alone', () => {
  // mid-text mention is content, not an address
  assert.equal(markLeadingMention('<p>ping @lead later</p>'), '<p>ping @lead later</p>');
  // a message that does not start with a paragraph (list, code) is untouched
  assert.equal(markLeadingMention('<ul><li>@lead x</li></ul>'), '<ul><li>@lead x</li></ul>');
  // an email is not a mention
  assert.equal(markLeadingMention('<p>a@b.com hi</p>'), '<p>a@b.com hi</p>');
});

test('the spawn kick never appears in the transcript', () => {
  assert.equal(isSessionStart('[2026-08-18 16:29] (session start)'), true);
  // rooms are persisted: the old instruction kick must stay filtered too
  assert.equal(isSessionStart('[2026-08-18 16:29] Start now: read your instructions and task brief.'), true);
  // a real prompt that merely mentions it is NOT the marker
  assert.equal(isSessionStart('[2026-08-18 16:30] what does (session start) mean?'), false);
  assert.equal(isSessionStart('run the tests'), false);
  assert.equal(isSessionStart(null), false);
});

test('slashCommand tells a CLI command from a message (and from a path)', () => {
  // The whole point: these are for the TUI, so they cannot carry the
  // `[tmm chat …] human:` prefix the normal delivery adds.
  assert.deepEqual(slashCommand('/model'), { to: '', command: '/model' });
  assert.deepEqual(slashCommand('  /model claude-opus-5  '), { to: '', command: '/model claude-opus-5' });
  assert.deepEqual(slashCommand('/clear'), { to: '', command: '/clear' });
  // An explicit address picks the target and is stripped from what is typed.
  assert.deepEqual(slashCommand('@builder-2 /compact'), { to: 'builder-2', command: '/compact' });
  assert.deepEqual(slashCommand('@all /clear'), { to: 'all', command: '/clear' });
  // A PATH is a message. This is the discriminator that keeps `/tmp/foo` out.
  assert.equal(slashCommand('/tmp/foo'), null);
  assert.equal(slashCommand('/usr/bin/env node'), null);
  assert.equal(slashCommand('look at /etc/hosts'), null);
  // Ordinary prose, and the shapes that only look like commands.
  for (const msg of ['hello', '', '   ', '/', '//', '/1234', 'a /model', '@dev hello']) {
    assert.equal(slashCommand(msg), null, JSON.stringify(msg));
  }
});

test('commandPalette completes a command, then its argument', () => {
  const models = ['auto', 'claude-opus-5', 'claude-sonnet-4.5'];

  // Stage one: a bare slash offers everything, and a prefix narrows it.
  const all = commandPalette('/', models);
  assert.equal(all?.stage, 'command');
  assert.equal(all?.items.length, OFFERED_COMMANDS.length, 'every OFFERED command is offered');
  assert.equal(all?.from, 0);
  assert.ok(all?.more, 'accepting a command may open its argument list');
  assert.deepEqual(
    commandPalette('/co', models)?.items.map((i) => i.value),
    ['/compact'],
    'prefix match, in table order — /context and /code are interactive views',
  );
  // Every offered item carries the CLI's own description, not an invented one.
  assert.ok(all?.items.every((i) => i.hint.length > 3));

  // Stage two: the argument. `/model` takes a model id, which is fetched.
  const arg = commandPalette('/model ', models);
  assert.equal(arg?.stage, 'arg');
  assert.deepEqual(arg?.items.map((i) => i.value), [...models, 'set-current-as-default']);
  assert.equal(arg?.from, '/model '.length, 'replace only the (empty) argument');
  assert.equal(arg?.more, false, 'nothing follows a single argument');
  assert.deepEqual(commandPalette('/model cla', models)?.items.map((i) => i.value),
    ['claude-opus-5', 'claude-sonnet-4.5']);
  // Fixed argument lists come from the table.
  assert.deepEqual(commandPalette('/effort ', models)?.items.map((i) => i.value),
    ['low', 'medium', 'high', 'xhigh', 'max', 'set-current-as-default']);

  // A filled argument ends the palette: what follows is a path or free text.
  assert.equal(commandPalette('/model claude-opus-5 ', models), null);
  assert.equal(commandPalette('/context add ', models), null);
  // A command with no arguments offers nothing once it is typed.
  assert.equal(commandPalette('/compact ', models), null);
  // An address is allowed before it, and shifts the replace offset.
  assert.equal(commandPalette('@builder-2 /comp', models)?.from, '@builder-2 '.length);
  // Not a command line at all.
  for (const s of ['hello', '/tmp/x', '', ' ', '/nope ']) {
    assert.equal(commandPalette(s, models), null, JSON.stringify(s));
  }
});

test('the palette matches fuzzily, but a tighter match always outranks a looser one', () => {
  // "我打字提示自动补全的时候，不一定从第一个字符开始匹配，可以模糊匹配"
  // (owner, 2026-08-24). Three tiers — prefix, substring, subsequence — because
  // Enter accepts the TOP item: /co must keep meaning /compact even while /mdl
  // finds /model.
  assert.equal(fuzzyRank('mo', 'model'), 0, 'prefix');
  assert.equal(fuzzyRank('son', 'claude-sonnet-4.5'), 1, 'substring');
  assert.equal(fuzzyRank('mdl', 'model'), 2, 'subsequence');
  assert.equal(fuzzyRank('xyz', 'model'), -1, 'no match');
  assert.equal(fuzzyRank('', 'anything'), 0, 'empty query is a prefix of everything');
  assert.equal(fuzzyRank('SON', 'claude-sonnet-4.5'), 1, 'case-insensitive');

  // Stage one: a subsequence finds the command mid-word.
  assert.deepEqual(commandPalette('/mdl', [])?.items.map((i) => i.value), ['/model']);
  // Stage two: the discriminating part of a model id never comes first.
  const models = ['auto', 'claude-opus-5', 'claude-sonnet-4.5'];
  assert.deepEqual(commandPalette('/model son', models)?.items.map((i) => i.value),
    ['claude-sonnet-4.5']);
  // A prefix match stays on top of a substring match, list order within a tier
  // stays the table's own.
  assert.deepEqual(commandPalette('/model op', ['claude-opus-5', 'opus-fast'])?.items.map((i) => i.value),
    ['opus-fast', 'claude-opus-5'], 'tighter first, then table order');
  // Nothing matches: the palette closes rather than offering everything.
  assert.equal(commandPalette('/model zzz', models), null);
});

test('an interactive view is not offered — it would park the agent in a panel', () => {
  // kiro marks these `inputType: "panel"` (or they open $EDITOR / a recorder):
  // sending one from the chat leaves the agent inside something nobody here can
  // see or dismiss, so they are filtered out until there is a way to show them
  // (owner, 2026-08-19: "比如/tools 就先去掉吧").
  assert.ok(KIRO_COMMANDS.some((c) => c.name === 'tools' && c.view), 'kept in the table, with the reason');
  assert.ok(OFFERED_COMMANDS.every((c) => !c.view));
  for (const typed of ['/tools', '/too', '/help', '/mcp', '/context', '/context ', '/rewind', '/voice']) {
    assert.equal(commandPalette(typed, ['auto']), null, `${typed} must not be offered`);
  }
  // The actors are still there, and `/model` still completes its values.
  const offered = commandPalette('/', [])?.items.map((i) => i.value) ?? [];
  assert.deepEqual(offered, ['/model', '/compact', '/clear', '/effort', '/agent', '/chat', '/spec', '/plan', '/paste', '/quit']);
  assert.ok(commandPalette('/model ', ['auto'])?.items.some((i) => i.value === 'auto'));
  // `/agent` offers only `swap`: create/edit open $EDITOR in the pane.
  assert.deepEqual(commandPalette('/agent ', [])?.items.map((i) => i.value), ['swap']);
});

test('the palette speaks the addressee backend dialect', () => {
  // grok (1.0.5, table transcribed from its own docs): /model is inline and
  // dynamic, /effort has grok's four levels (no kiro-only max), /resume is a
  // picker and never offered.
  const grokAll = commandPalette('/', [], 'grok')?.items.map((i) => i.value) ?? [];
  assert.ok(grokAll.includes('/model') && grokAll.includes('/btw'), `${grokAll}`);
  assert.ok(!grokAll.includes('/resume'), 'grok /resume is a picker');
  assert.deepEqual(commandPalette('/effort ', [], 'grok')?.items.map((i) => i.value),
    ['low', 'medium', 'high', 'xhigh']);
  assert.ok(commandPalette('/model ', ['grok-4.6'], 'grok')?.items.some((i) => i.value === 'grok-4.6'),
    'grok models are enumerable and complete inline');

  // codex (0.148.0, popup transcribed live): /model is a PICKER there, so it
  // is a view; /compact and /diff act in the pane.
  const codexAll = commandPalette('/', [], 'codex')?.items.map((i) => i.value) ?? [];
  assert.ok(codexAll.includes('/compact') && codexAll.includes('/diff'), `${codexAll}`);
  assert.ok(!codexAll.includes('/model'), 'codex /model parks the TUI at a picker');
  assert.ok(!codexAll.includes('/delete'), 'destructive, never offered');

  // kiro stays the default dialect ('' = backend unknown, historically kiro).
  assert.deepEqual(commandPalette('/', [], '')?.items.length, OFFERED_COMMANDS.length);
  assert.deepEqual(commandPalette('/', [], 'kiro')?.items.length, OFFERED_COMMANDS.length);

  // claude has no transcribed table (CLI not installed — a made-up command
  // looks authoritative and then does nothing), and a mixed @all roster has
  // no single dialect: no palette beats a wrong one.
  assert.equal(commandPalette('/', [], 'claude'), null);
  assert.equal(commandPalette('/', [], 'mixed'), null);
});

test('ctxColor ramps through the theme tokens, never a raw colour', () => {
  // Every value must be expressed in the app's status tokens: a raw hex here
  // would be right in one theme and wrong in the other.
  for (const pct of [0, 1, 20, 21, 42, 60, 61, 85, 86, 100, -5, 999, NaN]) {
    const c = ctxColor(pct);
    assert.ok(c.includes('var(--status-'), `${pct} → ${c}`);
    assert.ok(!/#[0-9a-f]{3}/i.test(c), `${pct} → ${c} must not carry a literal colour`);
  }
  // kiro's own anchors: green up to 20%, amber by 60%.
  assert.equal(ctxColor(0), 'var(--status-ok)');
  assert.equal(ctxColor(20), 'var(--status-ok)');
  assert.equal(ctxColor(60), 'color-mix(in srgb, var(--status-warn) 100%, var(--status-ok))');
  // Past the warning threshold it keeps going: hot, then danger.
  assert.ok(ctxColor(70).includes('--status-hot'));
  assert.ok(ctxColor(95).includes('--status-danger'));
  assert.equal(ctxColor(100), 'color-mix(in srgb, var(--status-danger) 100%, var(--status-hot))');
  // Out-of-range and garbage clamp instead of producing a broken expression.
  assert.equal(ctxColor(-5), 'var(--status-ok)');
  assert.equal(ctxColor(NaN), 'var(--status-ok)');
  assert.equal(ctxColor(999), ctxColor(100));
});

test('statusNote reads an agent status message, and only that', () => {
  assert.deepEqual(statusNote('[tmm status working] compiling the server'),
    { state: 'working', text: 'compiling the server' });
  assert.deepEqual(statusNote('[tmm status blocked] waiting for the API spec'),
    { state: 'blocked', text: 'waiting for the API spec' });
  // A multi-line note keeps its body.
  assert.equal(statusNote('[tmm status working] one\ntwo')?.text, 'one\ntwo');
  // A `tmm done` summary is the same kind of thing: the agent reporting on its
  // own work, and it must not be app narration — `[tmm] ` folds into a grey sys
  // row and the chat-only level drops it, so the text vanished where a reader
  // looks (owner, 2026-08-19: "返回的状态信息要用消息的形式展示在对话里").
  assert.deepEqual(statusNote('[tmm done] shipped the palette'),
    { state: 'done', text: 'shipped the palette' });
  // Not a note: an ordinary message, a lifecycle line, an empty one.
  for (const b of ['hello', '[tmm] stopped dev', '[tmm] done', '[tmm status working]',
                   '[tmm status working]   ', '[tmm done]', '[tmm done]  ',
                   '[tmm status] no state', 'prefix [tmm status working] x', '', null, undefined]) {
    assert.equal(statusNote(b), null, JSON.stringify(b));
  }
  // The two markers must not overlap: `[tmm] ` folds into a grey sys row, which
  // is exactly the treatment a status note was moved out of.
  assert.equal(systemLine('[tmm status working] compiling'), null);
  assert.equal(systemLine('[tmm done] shipped it'), null);
  // A summary-less done stays a lifecycle line: there is nothing to read.
  assert.equal(systemLine('[tmm] done'), 'done');
});

test('noteStateColor speaks the ONE progressive colour language', () => {
  // Owner, 2026-08-20: colours must match intuition, as a progression —
  // accent means IN MOTION (a spinner is never green), green means it ENDED
  // WELL, amber means paused on a person, red is the only distress signal.
  assert.equal(noteStateColor('working'), 'var(--accent)');
  assert.equal(noteStateColor('running'), 'var(--accent)');
  assert.equal(noteStateColor('done'), 'var(--status-ok)');
  assert.equal(noteStateColor('waiting'), 'var(--status-warn)');
  assert.equal(noteStateColor('blocked'), 'var(--status-warn)');
  assert.equal(noteStateColor('failed'), 'var(--status-danger)');
  // A word we do not know renders quiet, never wrong.
  assert.equal(noteStateColor('pondering'), 'var(--text3)');
  // The roster dots MUST agree with the badges — two dialects of the same
  // vocabulary is how colours stop meaning anything.
  for (const s of ['working', 'running', 'waiting', 'blocked', 'failed']) {
    assert.equal(stateDotColor(s), noteStateColor(s), s);
  }
});

test('sameDay is a LOCAL calendar-day rule', () => {
  const at = (y: number, mo: number, d: number, h = 12) => new Date(y, mo - 1, d, h).getTime();
  assert.ok(sameDay(at(2026, 8, 20, 0), at(2026, 8, 20, 23)));
  assert.ok(!sameDay(at(2026, 8, 20, 23), at(2026, 8, 21, 0)));
  // Adjacent months and years are different days, not modular arithmetic.
  assert.ok(!sameDay(at(2026, 7, 20), at(2026, 8, 20)));
  assert.ok(!sameDay(at(2025, 8, 20), at(2026, 8, 20)));
  // One millisecond across local midnight flips the separator.
  const midnight = new Date(2026, 7, 21, 0, 0, 0, 0).getTime();
  assert.ok(!sameDay(midnight - 1, midnight));
});

test('a draft belongs to its project, and an empty one leaves no trace', () => {
  // Typing in one project cannot touch another's draft.
  const one = draftUpdate({}, 'proj-a', 'half a sen');
  assert.deepEqual(one, { 'proj-a': 'half a sen' });
  const two = draftUpdate(one, 'proj-b', '@dev hi');
  assert.deepEqual(two, { 'proj-a': 'half a sen', 'proj-b': '@dev hi' });

  // Clearing the box REMOVES the key: otherwise every project ever visited
  // leaves a row behind for good.
  assert.deepEqual(draftUpdate(two, 'proj-b', ''), { 'proj-a': 'half a sen' });
  assert.deepEqual(draftUpdate({ x: '' }, 'x', ''), {});

  // No change means the SAME object, which is how the caller skips a write on a
  // keystroke that changed nothing (arrow keys, a re-render).
  assert.equal(draftUpdate(one, 'proj-a', 'half a sen'), one);
  assert.equal(draftUpdate(one, '', 'anything'), one, 'no project, no draft');

  // A pasted file is capped rather than allowed to fill localStorage.
  const huge = draftUpdate({}, 'p', 'x'.repeat(DRAFT_MAX + 500));
  assert.equal(huge['p']?.length, DRAFT_MAX);
  // The input is never mutated — the map is state somebody else owns.
  const before = { p: 'a' };
  draftUpdate(before, 'p', 'b');
  assert.deepEqual(before, { p: 'a' });
});

// ── readlineEdit: the composer's readline set ──
const rl = (key: string, text: string, start: number, end = start, kill = '', killing = false) =>
  readlineEdit({ key, text, start, end, kill, killing });

test('readline A/E move within the LINE of a multi-line draft', () => {
  const text = 'first line\nsecond line';
  assert.equal(rl('a', text, 17)?.caret, 11);          // start of "second"
  assert.equal(rl('e', text, 3)?.caret, 10);           // end of "first line"
  assert.equal(rl('a', text, 0)?.caret, 0);
  assert.equal(rl('e', text, text.length)?.caret, text.length);
});

test('readline U kills to line start, K to line end, and both feed the buffer', () => {
  const u = rl('u', 'hello world', 5)!;
  assert.deepEqual([u.text, u.caret, u.kill, u.killing], [' world', 0, 'hello', true]);
  const k = rl('k', 'hello world', 5)!;
  assert.deepEqual([k.text, k.caret, k.kill], ['hello', 5, ' world']);
  // U at line start and K at text end are handled no-ops, not crashes.
  assert.equal(rl('u', 'ab\ncd', 3)?.text, 'ab\ncd');
  assert.equal(rl('k', 'ab', 2)?.text, 'ab');
});

test('readline K at end of line eats the newline (join), like readline', () => {
  const k = rl('k', 'ab\ncd', 2)!;
  assert.equal(k.text, 'abcd');
  assert.equal(k.kill, '\n');
});

test('readline W kills the previous word, whitespace included', () => {
  const w = rl('w', 'one two  ', 9)!;
  assert.deepEqual([w.text, w.caret, w.kill], ['one ', 4, 'two  ']);
});

test('consecutive kills accumulate — backward prepends, forward appends', () => {
  // Ctrl-W Ctrl-W then Ctrl-Y restores the words in original order.
  const w1 = rl('w', 'one two three', 13)!;
  const w2 = readlineEdit({ key: 'w', text: w1.text, start: w1.caret, end: w1.caret, kill: w1.kill, killing: w1.killing })!;
  assert.equal(w2.kill, 'two three');
  const y = readlineEdit({ key: 'y', text: w2.text, start: w2.caret, end: w2.caret, kill: w2.kill, killing: false })!;
  assert.equal(y.text, 'one two three');
  // A broken chain replaces instead of accumulating.
  const k = rl('k', 'abc', 0, 0, 'old', false)!;
  assert.equal(k.kill, 'abc');
});

test('readline Y yanks at the caret and replaces a selection; empty buffer is a handled no-op', () => {
  const y = rl('y', 'ab', 1, 1, 'XY')!;
  assert.deepEqual([y.text, y.caret], ['aXYb', 3]);
  const sel = rl('y', 'abcd', 1, 3, 'Z')!;
  assert.equal(sel.text, 'aZd');
  assert.notEqual(rl('y', 'ab', 1), null); // handled — Ctrl-Y must not fall through to redo
});

test('readline D and H delete without touching the kill buffer', () => {
  const d = rl('d', 'abc', 1, 1, 'keep')!;
  assert.deepEqual([d.text, d.caret, d.kill], ['ac', 1, 'keep']);
  const h = rl('h', 'abc', 2)!;
  assert.deepEqual([h.text, h.caret], ['ac', 1]);
  assert.deepEqual(rl('d', 'abcd', 1, 3)?.text, 'ad'); // selection: delete it
});

test('readline T transposes, F/B move, unknown keys fall through', () => {
  const t1 = rl('t', 'abc', 1)!;               // drag a over b
  assert.deepEqual([t1.text, t1.caret], ['bac', 2]);
  const t2 = rl('t', 'abc', 3)!;               // at end: last two
  assert.deepEqual([t2.text, t2.caret], ['acb', 3]);
  assert.equal(rl('t', 'a\nb', 2)?.text, 'a\nb'); // never across the newline
  assert.equal(rl('f', 'ab', 0)?.caret, 1);
  assert.equal(rl('b', 'ab', 1)?.caret, 0);
  assert.equal(rl('b', 'abc', 1, 3)?.caret, 1); // selection collapses, no move
  assert.equal(rl('c', 'ab', 0), null);         // copy is the browser's
  assert.equal(rl('v', 'ab', 0), null);
  assert.equal(rl('z', 'ab', 0), null);
});

test('composer uploads land under the project .tmm with a random id', () => {
  assert.equal(uploadImagePath('/w/s/', 'abc', 'webp'), '/w/s/.tmm/uploads/abc.webp');
  assert.equal(uploadImagePath('/w/s', 'abc', 'jpg'), '/w/s/.tmm/uploads/abc.jpg');
  assert.match(imageId(), /^[a-z0-9]+-[a-z0-9]{8}$/i);
  // Non-images keep their OWN name, uniqued and de-spaced (owner, 2026-08-26:
  // "非图片文件直接原封不动地存放" — the content; the name still has to be
  // path-safe and unambiguous in a pane).
  assert.equal(uploadFilePath('/w/s', 'id1', 'report.pdf'), '/w/s/.tmm/uploads/id1-report.pdf');
  assert.equal(uploadFilePath('/w/s', 'id1', 'my notes v2.pdf'), '/w/s/.tmm/uploads/id1-my_notes_v2.pdf');
  assert.equal(uploadFilePath('/w/s', 'id1', 'a/b\\c:d?.txt'), '/w/s/.tmm/uploads/id1-cd.txt', 'separators and reserved chars stripped');
  assert.equal(uploadFilePath('/w/s', 'id1', '   '), '/w/s/.tmm/uploads/id1-file', 'nothing left → generic');
  assert.notEqual(imageId(), imageId(), 'random half differs');
});

test('pastedFiles pulls the files out of a paste, or [] for plain text (board #25)', () => {
  const file = (name: string, type: string) => ({ name, type }) as unknown as File;
  const item = (f: File | null, kind = 'file') => ({ kind, getAsFile: () => f });
  const shot = file('image.png', 'image/png');
  const pdf = file('report.pdf', 'application/pdf');
  // A screenshot paste: one file item, usually a text/plain item beside it.
  assert.deepEqual(pastedFiles({ items: [item(null, 'string'), item(shot)] }), [shot],
    'string items ignored, file items staged');
  // Some engines expose pasted files only on `files`.
  assert.deepEqual(pastedFiles({ items: [], files: [pdf] }), [pdf], 'files is the fallback');
  // Items and files alias the SAME files — never read both (would duplicate).
  assert.deepEqual(pastedFiles({ items: [item(shot)], files: [shot, pdf] }), [shot],
    'items win outright when they yield anything');
  // A file item whose getAsFile() is null is a miss, not a crash.
  assert.deepEqual(pastedFiles({ items: [item(null)], files: [pdf] }), [pdf]);
  // Plain text paste: nothing to stage, the textarea's default insertion runs.
  assert.deepEqual(pastedFiles({ items: [item(null, 'string')] }), []);
  assert.deepEqual(pastedFiles({}), []);
  assert.deepEqual(pastedFiles(null), [], 'clipboardData can be null');
});

test('mentionsAgent parses addresses the way deliver_mentions does', () => {
  assert.ok(mentionsAgent('@builder fix it', 'builder'));
  assert.ok(mentionsAgent('please @builder: now', 'builder'), 'trailing punctuation trimmed');
  assert.ok(mentionsAgent('@all standup', 'builder'), '@all reaches every agent');
  assert.ok(!mentionsAgent('@builder-2 yours', 'builder'), 'a longer name is a DIFFERENT agent');
  assert.ok(!mentionsAgent('mail me at x@builder.com... wait no', 'builder-2'));
  assert.ok(!mentionsAgent('no address here', 'builder'));
});

test('filterBlocks keeps one agent\u2019s world and nobody else\u2019s (board #3)', () => {
  const msg = (from: string, body: string, ts = 1) => ({ type: 'msg' as const, ts, msg: { from, body }, delivered: false });
  const blocks = [
    msg('builder', 'my reply'),                                  // from the agent
    msg('human', '@builder do this'),                            // addressed to it
    msg('lead', '@builder-2 yours'),                             // addressed to a LONGER name
    msg('human', '@all everyone'),                               // broadcast reaches it
    msg('builder-2', 'concurrent reply'),                        // someone else talking
    { type: 'steps' as const, ts: 2, window: 2, key: 's2', events: [] },
    { type: 'steps' as const, ts: 3, window: 4, key: 's4', events: [] },
    { type: 'prompt' as const, ts: 4, window: 2, text: 'typed locally' },
    { type: 'progress' as const, ts: 5, window: 4, state: 'working', text: 'other lane' },
    { type: 'note' as const, ts: 6, window: 2, event: {} as never },
    { type: 'sys' as const, ts: 7, key: 'sys1', items: ['[tmm] spawned builder — brief', '[tmm] spawned builder-2 — other', '[tmm] interrupted lead'] },
    { type: 'sys' as const, ts: 8, key: 'sys2', items: ['[tmm] board #1 todo → doing — title'] },
  ];
  const out = filterBlocks(blocks as never, 'builder', 2);
  const kinds = out.map((b) => (b.type === 'msg' ? `msg:${b.msg.from}` : b.type === 'sys' ? `sys:${b.items.length}` : `${b.type}:${'window' in b ? b.window : ''}`));
  assert.deepEqual(kinds, ['msg:builder', 'msg:human', 'msg:human', 'steps:2', 'prompt:2', 'note:2', 'sys:1'],
    'from-agent + addressed + own-window telemetry + the one sys line naming it');
  const sys = out.find((b) => b.type === 'sys');
  assert.deepEqual(sys && 'items' in sys ? sys.items : [], ['[tmm] spawned builder — brief'],
    'builder-2\u2019s spawn line and an unrelated board line never surface for builder');
  // No window (a stopped agent, an unmapped sender): telemetry keeps nothing,
  // messages still filter by name.
  const noWin = filterBlocks(blocks as never, 'builder', undefined);
  assert.ok(noWin.every((b) => b.type === 'msg' || b.type === 'sys'), 'no window \u2192 no telemetry claimed');
});

test('foldLines: a phone fold is small and IMMOVABLE, a desktop fold keeps its fifth (board #4)', () => {
  // Compact: constant 4 — one above the 3-line floor (the floor is where a
  // fold "no longer says anything"), small enough that several messages share
  // a phone screen, and with NO basis term the on-screen keyboard (which
  // resizes the viewport) and the composer cannot re-cut a message mid-read.
  assert.equal(PHONE_FOLD_LINES, 4);
  for (const basis of [400, 640, 700, 844, 900]) {
    assert.equal(foldLines(true, basis, 20), PHONE_FOLD_LINES, `compact @${basis}px stays flat`);
  }
  assert.equal(foldLines(true, 700, 24), PHONE_FOLD_LINES, 'line-height does not move it either');

  // Desktop keeps the screen-derived budget — no regression: a fifth of the
  // column minus bubble padding (26px), in whole line boxes.
  assert.equal(foldLines(false, 1000, 20), 8, 'a 1000px column reads 8 lines');
  assert.equal(foldLines(false, 700, 20), 5, 'a 700px column reads 5');
  assert.equal(foldLines(false, 1000, 24), 7, 'a taller line box fits fewer');
  // The 96px floor and the 3-line floor still hold in a tiny window.
  assert.equal(foldLines(false, 300, 20), 3);
  assert.equal(foldLines(false, 0, 20), 3);
  // An unmeasured line-height falls back to 20 instead of dividing by NaN.
  assert.equal(foldLines(false, 1000, NaN), 8);
});

test('mergeStates: one truth per dot — the roster overlays its own project only (board #8)', () => {
  const snapshot = { 'proj:2': 'idle', 'proj:3': 'running', 'other:1': 'waiting' };
  const roster = [
    { window: 2, state: 'running', managed: true },   // fresher than the snapshot
    { window: 5, state: 'waiting', managed: true },   // new window the snapshot missed
    { window: 7, state: 'running', managed: false },  // a shell asserts nothing
    { window: 8, state: '', managed: true },          // no reading, no key
  ];
  const out = mergeStates(snapshot, 'proj', roster);
  assert.equal(out['proj:2'], 'running', 'the roster wins for its project');
  assert.equal(out['proj:5'], 'waiting', 'a window the snapshot lacked appears');
  assert.equal(out['other:1'], 'waiting', 'other projects keep the snapshot');
  assert.ok(!('proj:7' in out) && !('proj:8' in out), 'unmanaged / stateless write nothing');
  assert.equal(out['proj:3'], 'running', 'a stopped/unlisted window\u2019s key passes through — absence is not invented');
  assert.notEqual(out, snapshot, 'pure: a new map, the input untouched');
  assert.equal(snapshot['proj:2'], 'idle');

  // Polling order: a STALE rooms response landing after a fresh roster must
  // not roll the selected project back — reload overlays the roster on top.
  const staleRooms = { 'proj:2': 'idle', 'other:1': 'idle' };
  const afterReload = mergeStates(staleRooms, 'proj', roster);
  assert.equal(afterReload['proj:2'], 'running', 'stale snapshot cannot overwrite the newer roster');
  assert.equal(afterReload['other:1'], 'idle', 'while other projects take the fresh snapshot');

  // Legacy vocabulary passes through untranslated — stateDotColor/stateIsLive
  // already read \'working\'; normalizing here would fork the one status language.
  assert.equal(mergeStates({}, 'p', [{ window: 1, state: 'working', managed: true }])['p:1'], 'working');
});

test('mergeEvents: a prepended page and a poll meet without doubles (board #9)', () => {
  const e = (id: number, ts: number, text = 't') => ({ id, ts, window: 1, kind: 'tool', text });
  const current = [e(10, 1000), e(11, 1000), e(12, 1200)];
  // An older page arrives (walked backwards): lands IN FRONT, log order.
  const older = mergeEvents(current, [e(8, 900), e(9, 950)]);
  assert.deepEqual(older.map((x) => x.id), [8, 9, 10, 11, 12]);
  // Same-millisecond siblings order by id — ts alone cannot address them.
  const sib = mergeEvents(older, [e(9.5 as never, 1000)]);
  assert.deepEqual(sib.map((x) => x.id), [8, 9, 9.5, 10, 11, 12]);
  // A page overlapping what is already loaded dedupes by id…
  const overlap = mergeEvents(older, [e(9, 950), e(13, 1300)]);
  assert.deepEqual(overlap.map((x) => x.id), [8, 9, 10, 11, 12, 13]);
  // …and nothing new returns the SAME array, so the poll can skip a render.
  assert.equal(mergeEvents(older, [e(10, 1000)]), older);
  assert.equal(mergeEvents(older, []), older);
  // Pre-paging rows (no id) fall back to a content key: no doubles either.
  const legacy = [{ ts: 500, window: 2, kind: 'status', text: 'w' }];
  assert.equal(mergeEvents(legacy as never, legacy as never).length, 1);
});

test('mergeMessages orders same-millisecond messages by seq across pages (#9 review)', () => {
  const m = (id: string, ts: number, seq: number) => ({ id, ts, seq, from: 'a', body: id });
  // The NEWER half of a burst was already loaded; the older page arrives late.
  const live = [m('c', 1000, 12), m('d', 1100, 13)];
  const older = mergeMessages(live, [m('a', 1000, 10), m('b', 1000, 11)]);
  assert.deepEqual(older.map((x) => x.id), ['a', 'b', 'c', 'd'],
    'same-ts rows sort by the bus log position, not by arrival');
  // Rows without seq (older servers) keep plain ts order.
  const legacy = mergeMessages([{ id: 'x', ts: 5, from: 'h', body: 'x' }], [{ id: 'y', ts: 3, from: 'h', body: 'y' }]);
  assert.deepEqual(legacy.map((x) => x.id), ['y', 'x']);
});

test('boardLine parses a board move: id, both statuses, title', () => {
    assert.deepEqual(boardLine('board #12 todo → doing — 回顾开发历史优化'),
      { id: '12', from: 'todo', to: 'doing', title: '回顾开发历史优化' });
});
test('boardLine: a reopen keeps its origin — done → todo is the message', () => {
    const b = boardLine('board #11 done → todo — board页面优化');
    assert.equal(b?.from, 'done');
    assert.equal(b?.to, 'todo');
});
test('boardLine: a title-less line still parses; unknown shapes return null', () => {
    assert.deepEqual(boardLine('board #3 doing → review'), { id: '3', from: 'doing', to: 'review', title: '' });
    assert.equal(boardLine('spawned dev — brief'), null);
    assert.equal(boardLine('board #x todo → doing'), null);
    assert.equal(boardLine(''), null);
    assert.equal(boardLine(null), null);
});

test('boardStatusColor speaks the one progressive status language', () => {
    assert.equal(boardStatusColor('doing'), 'var(--accent)');     // started moving
    assert.equal(boardStatusColor('review'), 'var(--status-warn)'); // waits for a person
    // done keeps the language's green HERE — the feed's "→ done" badge must
    // agree with the [tmm done] state badge beside it. The Board sidebar's
    // count chips are the one sanctioned departure (Board.svelte countColor:
    // four categorical colours, owner 2026-09-01).
    assert.equal(boardStatusColor('done'), 'var(--status-ok)');   // ended well
    assert.equal(boardStatusColor('todo'), 'var(--text3)');       // at rest
    assert.equal(boardStatusColor('shipped'), 'var(--text2)');    // unknown: reading ink
});

test('promptParts strips the machine stamp and structures board deliveries', () => {
  // A board change notice: stamp off, sender out, chip + text.
  assert.deepEqual(
    promptParts('[tmm chat 2026-08-30 11:33] human: [board #15] board任务交互优化: status doing → todo'),
    { from: 'human', board: { id: '15', review: false }, text: 'board任务交互优化: status doing → todo' });
  // A review handoff keeps its review flag for the badge.
  const r = promptParts('[tmm chat 2026-08-30 12:00] builder: [board #17 review] file返回逻辑 — done. `tmm board move 17 done` to accept, or note what to fix + move doing.');
  assert.equal(r.from, 'builder');
  assert.deepEqual(r.board, { id: '17', review: true });
  // A stamped plain message: stamp off, no chip.
  assert.deepEqual(promptParts('[tmm chat 2026-08-30 10:00] lead: please rebase'),
    { from: 'lead', board: null, text: 'please rebase' });
  // An unstamped local prompt passes through whole.
  assert.deepEqual(promptParts('fix the tests'), { from: '', board: null, text: 'fix the tests' });
  assert.deepEqual(promptParts(''), { from: '', board: null, text: '' });
});

test('tail intent is a bottom GAP, and hidden scroll noise cannot flip it (board #38)', () => {
  // The measure: distance from the bottom edge — never absolute scrollTop.
  assert.equal(bottomGap({ scrollHeight: 1000, scrollTop: 700, clientHeight: 300 }), 0, 'parked at the tail');
  assert.equal(bottomGap({ scrollHeight: 1000, scrollTop: 500, clientHeight: 300 }), 200, 'reading history');
  assert.equal(bottomGap(null), 0, 'no element yet reads as the tail');

  // Visible, the gap decides — the threshold boundary is exact.
  assert.equal(tailAfterScroll(true, false, TAIL_GAP - 1), true, 'inside the margin regains the tail');
  assert.equal(tailAfterScroll(true, true, TAIL_GAP), false, 'at the threshold the tail is lost');

  // Hidden, the event is layout noise: it can neither TAKE the intent from a
  // reader who left at the tail, nor GRANT it to one who left in history.
  assert.equal(tailAfterScroll(false, true, 9999), true, 'hidden noise cannot pollute following');
  assert.equal(tailAfterScroll(false, false, 0), false, 'hidden settling cannot fake a return to the tail');
});

test('a touch long-press leaves the context menu to the SYSTEM (board #48)', () => {
  // The owner's report: long-pressing a chat message to select text popped
  // OUR menu instead of the phone's native selection ("不应该出现选项卡 应该走
  // 手机系统本身默认选中文字的逻辑"). On Android a long-press over text FIRES
  // contextmenu, and an unconditional preventDefault kills the selection.
  // The gate: only a touch-sourced event belongs to the system.
  assert.equal(touchContextMenu('touch'), true, 'finger long-press → native selection');
  assert.equal(touchContextMenu('pen'), true, 'a stylus hold is the same gesture');
  // A mouse has a right button — that IS the menu request, never a selection.
  assert.equal(touchContextMenu('mouse'), false);
  // Chromium's keyboard menu key reports an empty pointerType; a browser that
  // predates PointerEvent contextmenu reports undefined. Neither is a finger,
  // and neither can be mid-text-selection — the menu stays available.
  assert.equal(touchContextMenu(''), false);
  assert.equal(touchContextMenu(undefined), false);
});

test('the fold budget is CHARACTERS, not source lines (board #53)', () => {
  // Owner: "chat 用户消息的折叠 应该是按照消息字符长度来，不只是行数，不然
  // 一大段就占满了，没起到折叠作用" (2026-09-01). The old line branch kept
  // `budget` SOURCE lines whole — one giant paragraph among them wrapped
  // into pages and the fold held nothing back.
  const wall = 'word '.repeat(600).trim(); // ~3000 chars on ONE source line
  const mixed = ['intro line', wall, 'after 1', 'after 2', 'after 3', 'after 4', 'after 5'].join('\n');
  const out = elideTail(mixed, 5, 80);
  // 5 visual lines ≈ 5×80 chars is the whole budget; the wall is cut INSIDE
  // it, not carried whole (old output kept all ~3000 chars).
  assert.ok(out.length <= 5 * 80 + ELIDE.length + 'intro line\n'.length,
    `bounded by the visual budget: ${out.length} chars`);
  assert.ok(out.endsWith(ELIDE), 'marker glued inline at the cut');
  assert.ok(!out.includes('after 1'), 'nothing after the wall survives');
  assert.ok(!out.includes(`\n${ELIDE}`), 'the marker never takes its own line');

  // Whole short lines still spend ONE visual line each — the 2026-08-27 rear
  // truncation is unchanged when no line wraps.
  const lines = Array.from({ length: 20 }, (_, i) => `l${i + 1}`).join('\n');
  const cut = elideTail(lines, 4);
  assert.equal(cut.split('\n').length, 4, 'four visual lines kept');
  assert.equal(cut.split('\n').at(-1), `l4${ELIDE}`, 'marker on the LAST kept line');

  // A wrapped line is priced at its real height: two 200-char lines exhaust a
  // 5-line budget before a third short line fits.
  const two = [`${'a'.repeat(200)}`, `${'b'.repeat(200)}`, 'tail'].join('\n');
  const tcut = elideTail(two, 5, 80);
  assert.ok(!tcut.includes('tail'), 'the third line is beyond the wrapped budget');
  assert.ok(tcut.endsWith(ELIDE), tcut);

  // CJK renders two units wide, so a Chinese paragraph reaches the SAME
  // visual height at half the characters — the estimate must price that in,
  // or the owner's own messages fold at twice the promised height.
  const zh = '中文字符宽度是两倍。'.repeat(100); // 1000 chars ≈ 2000 units
  const zcut = elideTail(zh, 5, 80);
  assert.ok(zcut.length - ELIDE.length <= Math.ceil((5 * 80) / 2) + 1,
    `CJK cut at half the latin count: ${zcut.length}`);
  assert.ok(zcut.endsWith(ELIDE), zcut);

  // Identity when everything fits — the caller skips re-rendering on it.
  const fits = 'short\nlines\nonly';
  assert.equal(elideTail(fits, 5), fits);
});

test('perLine is MEASURED from the real line, never assumed (board #53 review)', () => {
  // Lead blocker (2026-09-01, real Chromium): at 1280px the content line is
  // ~723px ≈ 82 latin chars, so the default 80 was accidentally right; at
  // 420px it is ~338px ≈ 38 chars, and foldBody still said 80 — wrap
  // underestimated ~2.1×, a "folded" bubble rendered ~8 lines on a 4-line
  // budget. The mapping from a measured line to a perLine: width ÷ average
  // glyph, floored, clamped against degenerate measurements.
  assert.equal(perLineOf(723, 8.8), 82, 'the desktop line the default was tuned on');
  assert.equal(perLineOf(338, 8.8), 38, 'the 420px drawer line that broke it');
  // Unmeasured inputs answer the historic default — same fallback posture as
  // chipCols' pre-measure 2 columns.
  assert.equal(perLineOf(0, 8.8), 80, 'no width yet');
  assert.equal(perLineOf(723, 0), 80, 'no glyph yet');
  assert.equal(perLineOf(NaN, 8.8), 80);
  assert.equal(perLineOf(723, NaN), 80);
  // Degenerate measurements clamp instead of folding to nothing or never:
  // a sliver of a column still shows words, a wall of glass still folds.
  assert.equal(perLineOf(40, 9), 16, 'floor: never fewer than 16 units');
  assert.equal(perLineOf(9000, 3), 240, 'cap: never more than 240 units');
});

test('modelLabel drops vendor and region prefixes, keeps the model id', () => {
  assert.equal(modelLabel('openai.gpt-5.6-sol'), 'gpt-5.6-sol');
  assert.equal(modelLabel('xai.grok-4.6'), 'grok-4.6');
  assert.equal(modelLabel('global.anthropic.claude-fable-5-1'), 'claude-fable-5-1');
  assert.equal(modelLabel('us.xai.grok-4.6'), 'grok-4.6');
  assert.equal(modelLabel('gpt-5.6-sol'), 'gpt-5.6-sol', 'no prefix, unchanged');
  assert.equal(modelLabel('Fable 5.1'), 'Fable 5.1', 'a display name is not dotted-prefixed');
  assert.equal(modelLabel('gpt-5.6'), 'gpt-5.6', 'a version dot is not a prefix');
  assert.equal(modelLabel('openai.'), 'openai.', 'a bare prefix is left alone rather than emptied');
});

test('a long message confirms against its TRUNCATED echo (board #78)', () => {
  // The server acks on the full prompt but stores the echo cut at
  // PROMPT_ECHO_MAX chars + "…"; the feed must not demand the whole body.
  const body = 'Review Info\n' + 'line of a long paste that keeps going and going\n'.repeat(40);
  assert.ok(body.length > PROMPT_ECHO_MAX);
  const typed = `[tmm chat 2026-09-03 07:04] human: @aws-expert ${body}`;
  const stored = [...typed].slice(0, PROMPT_ECHO_MAX).join('') + '…';
  assert.ok(echoTruncated(stored));
  const feed = [{ id: 'm1', ts: 10, from: 'human', body }];
  const blocks = feedBlocks(feed, [ev({ ts: 11, kind: 'prompt', via: 'app', text: stored })], 'chat');
  assert.equal(blocks.length, 1);
  assert.equal((blocks[0] as any).delivered, true, 'the ring closes');
  // A different long message that merely shares the opening is NOT acked.
  const other = body.replace('going and going', 'going and stopping');
  assert.equal(echoContains(squashWs(stored), squashWs(other), true), false);
  // A short echo that happens to END with "…" is not a truncation marker.
  assert.equal(echoTruncated('human: 稍等…'), false);
  assert.equal(echoContains(squashWs('human: 稍等…'), squashWs('稍等…完整版'), false), false);
  // The client's cut point mirrors the server's constant — pin them together.
  const telemetry = readFileSync(new URL('../../../src-tauri/src/projects/telemetry.rs', import.meta.url), 'utf8');
  assert.match(telemetry, new RegExp(`const MAX_PROMPT_CHARS: usize = ${PROMPT_ECHO_MAX};`, 'u'));
});
