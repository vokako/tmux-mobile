// Board #57 — new-message notifications: the GATE is the design, so the gate
// is what these tests pin, in both layers. Layer 1: seen keys — hub_log's
// since_ts poll is inclusive and cache-restored rooms re-pull, so a replayed
// batch must alert at most once, and first pages / watched batches / muted
// periods must never BACKFILL later. Layer 2: the news gate — own words, app
// narration and ambient progress are not news; nothing fires while looking.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';
import {
  notifiable, notifyText, excerpt, cueDue, msgKey, sift, playCue, notifyNews,
  isAway, roomProjectName, systemNotify, taskFinished, DEFAULT_LEVEL, NOTIFY_LEVELS,
  CUE_COOLDOWN_MS, CUE_SRC, SEEN_CAP,
  type FeedMsg, type NotifyState, type NotifyEnv,
} from './notifications.ts';

const away = { first: false, away: true };
const fresh = (): NotifyState => ({ seen: new Set(), lastCueAt: 0 });
/** A notifyNews harness with counting channels and the mute injectable. */
function harness(enabled = true) {
  const fired: { cue: number; sys: string[] } = { cue: 0, sys: [] };
  const effects = {
    cue: () => { fired.cue++; return true; },
    sys: (t: { title: string; body: string; tag?: string }) => { fired.sys.push(t.title); return true; },
    enabled: () => enabled,
  };
  return { fired, effects };
}

// ─── Layer 2: the news gate ─────────────────────────────────────────────────

test('notifiable: an agent message while away is news', () => {
  assert.equal(notifiable([{ from: 'builder-2', body: 'done with the parser', ts: 5 }], away).length, 1);
});

test('notifiable: the first page is history, never news', () => {
  assert.equal(notifiable([{ from: 'builder-2', body: 'hello', ts: 5 }], { first: true, away: true }).length, 0);
});

test('notifiable: a reader looking at the room is never interrupted', () => {
  assert.equal(notifiable([{ from: 'builder-2', body: 'hello', ts: 5 }], { first: false, away: false }).length, 0);
});

test('notifiable: your own words and nameless rows are not news', () => {
  assert.equal(notifiable([{ from: 'human', body: 'hi' }, { body: 'orphan' }, { from: '', body: 'x' }], away).length, 0);
});

test('notifiable: app narration ([tmm] sys lines) is not news', () => {
  assert.equal(notifiable([{ from: 'builder-2', body: '[tmm] spawned dev — brief' }], away).length, 0);
  assert.equal(notifiable([{ from: 'builder-2', body: '[tmm] board #7 todo → doing — Fix parser' }], away).length, 0, 'starting a task is not finishing it');
});

test('notifiable: an agent moving an issue to review/done IS news — "谁完成了什么任务" (board #72)', () => {
  const out = notifiable([
    { from: 'builder-2', body: '[tmm] board #7 doing → review — Fix parser' },
    { from: 'lead', body: '[tmm] board #7 review → done — Fix parser' },
    { from: 'human', body: '[tmm] board #8 doing → done — Mine' },
  ], away);
  assert.equal(out.length, 2, 'the human moving their own issue is not news');
  assert.deepEqual(taskFinished('[tmm] board #7 doing → review — Fix parser'), { id: '7', to: 'review', title: 'Fix parser' });
  assert.equal(taskFinished('[tmm] board #7 todo → doing — Fix parser'), null);
  assert.equal(taskFinished('board #7 doing → review'), null, 'only a lifecycle line, never a message that quotes one');
  assert.equal(excerpt('[tmm] board #7 doing → review — Fix parser'), '#7 → review · Fix parser');
});

test('notifiable: ambient [tmm status] progress is not news, [tmm done] is', () => {
  const out = notifiable([
    { from: 'builder-2', body: '[tmm status working] refactoring the parser' },
    { from: 'builder-2', body: '[tmm done] parser refactored, 12 tests green' },
  ], away);
  assert.equal(out.length, 1);
  assert.match(out[0]?.body ?? '', /\[tmm done\]/u);
});

test('notifiable: the LEVEL is three nested rungs — done ⊂ replies ⊂ all (board #72)', () => {
  const batch: FeedMsg[] = [
    { from: 'b', body: 'a plain reply' },
    { from: 'b', body: '[tmm done] shipped it' },
    { from: 'b', body: '[tmm] board #3 doing → review — Task' },
    { from: 'b', body: '[tmm status working] still at it' },
    { from: 'b', body: '[tmm] spawned dev — brief' },
  ];
  const bodies = (level: 'done' | 'replies' | 'all') => notifiable(batch, { ...away, level }).map((m) => m.body);
  assert.deepEqual(bodies('done'), ['[tmm done] shipped it', '[tmm] board #3 doing → review — Task']);
  assert.deepEqual(bodies('replies'), ['a plain reply', '[tmm done] shipped it', '[tmm] board #3 doing → review — Task']);
  assert.deepEqual(bodies('all'), ['a plain reply', '[tmm done] shipped it', '[tmm] board #3 doing → review — Task', '[tmm status working] still at it']);
  assert.equal(DEFAULT_LEVEL, 'replies');
  assert.deepEqual(notifiable(batch, away), notifiable(batch, { ...away, level: DEFAULT_LEVEL }), 'no level = the default');
  assert.deepEqual([...NOTIFY_LEVELS], ['done', 'replies', 'all'], 'the Settings row offers them in rising order');
});

// ─── Layer 1: seen keys — replays and backfill ──────────────────────────────

test('msgKey: id wins, else the from/ts/body triple', () => {
  assert.equal(msgKey({ id: 7, from: 'a', ts: 1, body: 'x' }), '#7');
  assert.equal(msgKey({ from: 'a', ts: 1, body: 'x' }), 'a\u00001\u0000x');
  assert.notEqual(msgKey({ from: 'a', ts: 1, body: 'x' }), msgKey({ from: 'a', ts: 1, body: 'y' }));
});

test('sift: splits never-seen, remembers everything, stays bounded', () => {
  const seen = new Set<string>();
  const batch: FeedMsg[] = [{ from: 'a', ts: 1, body: 'x' }, { from: 'b', ts: 2, body: 'y' }];
  assert.equal(sift(batch, seen).length, 2);
  assert.equal(sift(batch, seen).length, 0); // the replay
  const cap = 10;
  for (let i = 0; i < 30; i++) sift([{ from: 'c', ts: i, body: 'z' }], seen, cap);
  assert.ok(seen.size <= cap);
});

test('notifyNews: the same batch replayed alerts exactly once', () => {
  const st = fresh();
  const { fired, effects } = harness();
  const batch: FeedMsg[] = [{ from: 'builder-2', ts: 100, body: 'reply' }];
  assert.equal(notifyNews(batch, { ...away, project: 'p' }, st, effects), true);
  assert.equal(notifyNews(batch, { ...away, project: 'p' }, st, effects), false); // inclusive since_ts re-pull
  assert.equal(fired.cue, 1);
  assert.equal(fired.sys.length, 1);
});

test('notifyNews: a first page (initial load / project switch / cache restore) never backfills', () => {
  const st = fresh();
  const { fired, effects } = harness();
  const history: FeedMsg[] = [{ from: 'builder-2', ts: 1, body: 'old reply' }];
  assert.equal(notifyNews(history, { first: true, away: true, project: 'p' }, st, effects), false);
  // the same rows come back on the next incremental poll — still not news
  assert.equal(notifyNews(history, { ...away, project: 'p' }, st, effects), false);
  assert.equal(fired.cue, 0);
});

test('notifyNews: messages watched while looking never re-alert after tabbing away', () => {
  const st = fresh();
  const { fired, effects } = harness();
  const batch: FeedMsg[] = [{ from: 'builder-2', ts: 5, body: 'seen live' }];
  assert.equal(notifyNews(batch, { first: false, away: false, project: 'p' }, st, effects), false);
  assert.equal(notifyNews(batch, { ...away, project: 'p' }, st, effects), false); // same-ts re-pull, now away
  assert.equal(fired.cue, 0);
});

test('notifyNews: a muted period is remembered, not deferred', () => {
  const st = fresh();
  const muted = harness(false);
  const batch: FeedMsg[] = [{ from: 'builder-2', ts: 9, body: 'while muted' }];
  assert.equal(notifyNews(batch, { ...away, project: 'p' }, st, muted.effects), false);
  const unmuted = harness(true);
  assert.equal(notifyNews(batch, { ...away, project: 'p' }, st, unmuted.effects), false); // no backfill on unmute
  assert.equal(unmuted.fired.cue, 0);
});

// ─── The cue: cooldown + failure semantics ──────────────────────────────────

test('cueDue: one cue per cooldown window', () => {
  assert.equal(cueDue(0, CUE_COOLDOWN_MS), true);
  assert.equal(cueDue(1000, 1000 + CUE_COOLDOWN_MS - 1), false);
  assert.equal(cueDue(1000, 1000 + CUE_COOLDOWN_MS), true);
});

test('playCue: a resolved play consumes the window; a second same-tick call does not double-fire', async () => {
  const st = fresh();
  let plays = 0;
  const play = () => { plays++; return Promise.resolve(); };
  assert.equal(playCue(1000, st, play), true);
  assert.equal(playCue(1000, st, play), false); // claimed up front
  await Promise.resolve();
  assert.equal(st.lastCueAt, 1000);
  assert.equal(plays, 1);
});

test('playCue: a REJECTED play rolls the window back — autoplay block must not swallow the first real cue', async () => {
  const st = fresh();
  const play = () => Promise.reject(new Error('autoplay blocked'));
  assert.equal(playCue(1000, st, play), true);
  await Promise.resolve(); await Promise.resolve(); // let the rejection land
  assert.equal(st.lastCueAt, 0, 'the failed attempt consumed nothing');
  let ok = 0;
  assert.equal(playCue(1500, st, () => { ok++; return Promise.resolve(); }), true, 'the next cue plays immediately');
  assert.equal(ok, 1);
});

test('playCue: a DEFERRED old rejection cannot reopen the window a newer cue claimed', async () => {
  const st = fresh();
  let rejectA: (e: Error) => void = () => {};
  const playA = () => new Promise<void>((_res, rej) => { rejectA = rej; });
  assert.equal(playCue(1000, st, playA), true);           // A claims, hangs
  const tB = 1000 + CUE_COOLDOWN_MS;
  assert.equal(playCue(tB, st, () => Promise.resolve()), true); // B claims later, succeeds
  rejectA(new Error('late autoplay block'));              // A's rejection lands AFTER B
  await Promise.resolve(); await Promise.resolve();
  assert.equal(st.lastCueAt, tB, "A's late catch must not roll back B's claim");
  assert.equal(playCue(tB + 1, st, () => Promise.resolve()), false, 'the cooldown B claimed still holds');
});

// ─── Text ───────────────────────────────────────────────────────────────────

test('notifyText: names only, deduped, last body excerpted', () => {
  const { title, body } = notifyText(
    [{ from: 'a', body: 'first' }, { from: 'b', body: 'second' }, { from: 'a', body: 'third' }],
    'proj',
  );
  assert.equal(title, 'a, b · proj');
  assert.equal(body, 'third');
  assert.equal(notifyText([{ from: 'a', body: 'x' }], 'proj').tag, 'tmm:proj', 'one tray card per project, replaced not stacked');
});

// ─── Where the check runs (board #72) ───────────────────────────────────────

test('isAway: hidden, unfocused, or another page on screen', () => {
  const doc = (hidden: boolean, focus: boolean) => ({ hidden, hasFocus: () => focus });
  assert.equal(isAway(true, doc(false, true)), false, 'looking at the Hub');
  assert.equal(isAway(false, doc(false, true)), true, 'Terminal/Files/Board on screen');
  assert.equal(isAway(true, doc(true, true)), true, 'tab hidden');
  assert.equal(isAway(true, doc(false, false)), true, 'window unfocused');
});

test('roomProjectName: the recorded room wins, then proj:<session>, then the room itself', () => {
  const rows = [
    { project: { name: 'Renamed', session: 'new-name', room: 'proj:old-name' } },
    { project: { name: 'Plain', session: 'plain', room: null } },
  ];
  assert.equal(roomProjectName(rows, 'proj:old-name'), 'Renamed', 'a renamed project keeps its recorded room');
  assert.equal(roomProjectName(rows, 'proj:plain'), 'Plain');
  assert.equal(roomProjectName(rows, 'proj:unknown'), 'unknown', 'never an empty title');
});

// ─── The system channel on a phone (board #72) ──────────────────────────────

function swEnv(permission: string | null, reg: boolean, constructThrows = false) {
  const log: string[] = [];
  const env: NotifyEnv = {
    permission,
    construct: (title) => { if (constructThrows) throw new TypeError('Illegal constructor'); log.push(`new:${title}`); },
    registration: async () => (reg ? { showNotification: async (title) => { log.push(`sw:${title}`); } } : undefined),
  };
  return { env, log };
}
const settle = () => new Promise((r) => setTimeout(r, 0));

test('systemNotify: with a service worker the WORKER shows it — Android Chrome refuses the page constructor', async () => {
  const { env, log } = swEnv('granted', true, true);
  assert.equal(systemNotify({ title: 't', body: 'b', tag: 'tmm:p' }, env), true);
  await settle();
  assert.deepEqual(log, ['sw:t'], 'the constructor was never tried');
});

test('systemNotify: without a registration the page constructor is the fallback', async () => {
  const { env, log } = swEnv('granted', false);
  assert.equal(systemNotify({ title: 't', body: 'b' }, env), true);
  await settle();
  assert.deepEqual(log, ['new:t']);
});

test('systemNotify: not granted (or no API) attempts nothing — prompting is the Settings toggle\'s job', async () => {
  for (const p of ['default', 'denied', null]) {
    const { env, log } = swEnv(p, true);
    assert.equal(systemNotify({ title: 't', body: 'b' }, env), false);
    await settle();
    assert.deepEqual(log, []);
  }
});

test('excerpt: status marker gives way to its text, images to prose, 120-char cap', () => {
  assert.equal(excerpt('[tmm done] shipped the thing'), 'shipped the thing');
  assert.equal(excerpt('look ![](/tmp/shot.png) here'), 'look here');
  const cut = excerpt('x'.repeat(300));
  assert.equal(cut.length, 120);
  assert.ok(cut.endsWith('…'));
});

// ─── Source pins: the wiring that regresses silently ────────────────────────
const here = dirname(fileURLToPath(import.meta.url));
const hub = readFileSync(join(here, 'Hub.svelte'), 'utf8');

test('Hub notifies from the PUSH first and the poll second, both through isAway (board #72)', () => {
  // The poll stops with the page (`if (!visible) return` guards its effect),
  // so a poll-only notifier could never reach a reader on another page. The
  // push handler is the primary site; the poll is the fallback; sift dedups.
  const calls = hub.match(/notifyNews\(/gu) ?? [];
  assert.equal(calls.length, 2);
  assert.match(hub, /notifyNews\(messages, \{ first, away: isAway\(visible\), project: s \}\)/u, 'the poll site');
  const push = hub.slice(hub.indexOf('const onPush = (m) => {'), hub.indexOf('if (!selected || m?.room !== room(selected)) return;'));
  assert.match(push, /away: !selected \|\| m\.room !== room\(selected\) \|\| isAway\(visible\)/u,
    'another room is away by definition; the selected one asks the document');
  assert.match(push, /project: roomProjectName\(rows, m\.room\)/u, 'the title names the project, not the room');
  assert.match(push, /first: false/u, 'a push is never history');
  assert.ok(!/document\.hidden \|\| !document\.hasFocus\(\)/u.test(hub), 'ONE away verdict — the helper, never an inline copy');
});

const prefs = readFileSync(join(here, '..', 'app', 'Preferences.svelte'), 'utf8');

test('the permission request and the audio unlock ride the SETTINGS toggle, never the header or the send path (board #72)', () => {
  const sendBody = hub.slice(hub.indexOf('async function send()'), hub.indexOf('async function send()') + 4000);
  assert.ok(!sendBody.includes('ensurePermission'), 'send() asks for nothing');
  assert.ok(!hub.includes('ensurePermission') && !hub.includes('setNotifyEnabled') && !hub.includes("'bell'"),
    'the Hub header carries no notification switch — a header keeps no spare switches, and on a phone it cost the row a button');
  // The toggle's click carries all three: persist, preview (the unlock), permission.
  assert.match(prefs, /setNotifyEnabled\(on\);\s*\n\s*if \(!on\) return;\s*\n\s*previewCue\(\);\s*\n\s*await ensurePermission\(\);/u);
  const asks = prefs.match(/await ensurePermission\(\)/gu) ?? [];
  assert.equal(asks.length, 2, 'the toggle and the test row, nothing else');
  assert.match(prefs, /\{t\('hubNotify'\)\}/u, 'a labelled setting-row');
  // Its OWN category (owner, 2026-09-02: "应该在一个单独的 notification 二级页面"),
  // right after Appearance, never a row under it.
  assert.match(prefs, /\{ id: 'appearance', label: \(\) => t\('settingsAppearance'\) \},\s*\n\s*\{ id: 'notifications', label: \(\) => t\('settingsNotifications'\) \},/u);
  const appearance = prefs.slice(prefs.indexOf("{#if tab === 'appearance'}"), prefs.indexOf("{:else if tab === 'notifications'}"));
  assert.ok(appearance.length > 0 && !appearance.includes('hubNotify'), 'no notification row under Appearance');
  assert.match(prefs, /storedTab === 'notifications'/u, 'the category is restorable');
  assert.match(prefs, /systemNotify\(\{ title: t\('hubNotifyTestTitle'\)/u, 'a test row — the real alert only fires while you are NOT looking');
  assert.match(prefs, /options=\{NOTIFY_LEVELS\.map\(\(l\) => \(\{ value: l, label: t\('hubNotifyLevel_' \+ l\) \}\)\)\}/u,
    'the level row (a ui/Segmented) is driven by the ONE list, in its order');
  assert.match(prefs, /setNotifyLevel\(l\)/u);
  // Asking whenever not yet granted is what reaches Android's runtime prompt:
  // the Tauri shim reports `denied` before the first ask.
  const notif = readFileSync(join(here, 'notifications.ts'), 'utf8');
  assert.match(notif, /Notification\.permission !== 'granted'\) \{\s*\n\s*await Notification\.requestPermission\(\)/u);
  assert.match(prefs, /notifyPerm === 'unsupported' \? t\('hubNotifySoundOnly'\)/u, 'the caption tells a webview user the sound is the whole channel');
});

test('the placeholder cue asset exists where CUE_SRC points', () => {
  const wav = readFileSync(join(here, '..', '..', '..', 'public', CUE_SRC));
  assert.equal(wav.subarray(0, 4).toString(), 'RIFF');
  assert.ok(wav.length > 1000 && wav.length < 200_000, 'a short cue, not a soundtrack');
  assert.ok(SEEN_CAP >= 100, 'the seen memory holds a real conversation');
});
