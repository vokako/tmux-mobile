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
  CUE_COOLDOWN_MS, CUE_SRC, SEEN_CAP,
  type FeedMsg, type NotifyState,
} from './notifications.ts';

const away = { first: false, away: true };
const fresh = (): NotifyState => ({ seen: new Set(), lastCueAt: 0 });
/** A notifyNews harness with counting channels and the mute injectable. */
function harness(enabled = true) {
  const fired: { cue: number; sys: string[] } = { cue: 0, sys: [] };
  const effects = {
    cue: () => { fired.cue++; return true; },
    sys: (t: { title: string; body: string }) => { fired.sys.push(t.title); return true; },
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
});

test('notifiable: ambient [tmm status] progress is not news, [tmm done] is', () => {
  const out = notifiable([
    { from: 'builder-2', body: '[tmm status working] refactoring the parser' },
    { from: 'builder-2', body: '[tmm done] parser refactored, 12 tests green' },
  ], away);
  assert.equal(out.length, 1);
  assert.match(out[0]?.body ?? '', /\[tmm done\]/u);
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

// ─── Text ───────────────────────────────────────────────────────────────────

test('notifyText: names only, deduped, last body excerpted', () => {
  const { title, body } = notifyText(
    [{ from: 'a', body: 'first' }, { from: 'b', body: 'second' }, { from: 'a', body: 'third' }],
    'proj',
  );
  assert.equal(title, 'a, b · proj');
  assert.equal(body, 'third');
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

test('Hub wires notifyNews into the feed merge with the away verdict', () => {
  const calls = hub.match(/notifyNews\(/gu) ?? [];
  assert.equal(calls.length, 1);
  assert.match(hub, /notifyNews\(messages, \{ first, away: document\.hidden \|\| !document\.hasFocus\(\) \|\| !visible/u);
});

test('the permission request and the audio unlock ride the BELL, never the send path', () => {
  const sendBody = hub.slice(hub.indexOf('async function send()'), hub.indexOf('async function send()') + 4000);
  assert.ok(!sendBody.includes('ensurePermission'), 'send() asks for nothing');
  // The bell's click carries all three: persist, permission, preview.
  assert.match(hub, /setNotifyEnabled\(notifyOn\);\s*\n\s*if \(notifyOn\) \{ ensurePermission\(\); previewCue\(\); \}/u);
  const asks = hub.match(/ensurePermission\(\)/gu) ?? [];
  assert.equal(asks.length, 1);
});

test('the placeholder cue asset exists where CUE_SRC points', () => {
  const wav = readFileSync(join(here, '..', '..', '..', 'public', CUE_SRC));
  assert.equal(wav.subarray(0, 4).toString(), 'RIFF');
  assert.ok(wav.length > 1000 && wav.length < 200_000, 'a short cue, not a soundtrack');
  assert.ok(SEEN_CAP >= 100, 'the seen memory holds a real conversation');
});
