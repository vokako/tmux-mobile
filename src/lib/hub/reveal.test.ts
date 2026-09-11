// The reveal technique (board #174): a partition's CONTENT is laid out at its
// final width and pinned there; only the grid TRACK moves. These helpers are
// the whole imperative part — measured once, one `.moving` gate, cleanup after
// `moveMs()` — so the rest of the technique can be CSS the source test pins.
import test from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';

const dom = new JSDOM('<!doctype html><div class="cols"><div class="track"></div></div>');
const win = dom.window;
(globalThis as Record<string, unknown>).window ??= win;
(globalThis as Record<string, unknown>).document ??= win.document;
(globalThis as Record<string, unknown>).HTMLElement ??= win.HTMLElement;
let reduced = false;
(globalThis as Record<string, unknown>).matchMedia = () => ({ matches: reduced });

const { pinTrack, moveTrack } = await import('./reveal.ts');
const { T_MOVE_MS } = await import('../ui/motion.ts');

const cols = win.document.querySelector<HTMLElement>('.cols')!;
const track = win.document.querySelector<HTMLElement>('.track')!;
// jsdom lays nothing out: the measured width is what the test says it is.
Object.defineProperty(track, 'offsetWidth', { configurable: true, get: () => 312 });
let reflows = 0;
Object.defineProperty(cols, 'offsetWidth', { configurable: true, get: () => { reflows++; return 1440; } });
const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));

test('pinTrack freezes the content at its measured width, anchored to one edge, and releases whole', () => {
  const unpin = pinTrack(track, 'end');
  assert.equal(track.style.width, '312px', 'the width is the measured one, not a token');
  assert.ok(track.classList.contains('pinned') && track.classList.contains('pin-end'));
  unpin();
  assert.equal(track.style.width, '');
  assert.ok(!track.classList.contains('pinned') && !track.classList.contains('pin-end'));
  pinTrack(track, 'start')();
  assert.equal(track.className, 'track', 'nothing of the pin survives');
});

const ended = (prop: string) => cols.dispatchEvent(Object.assign(new win.Event('transitionend', { bubbles: true }), { propertyName: prop }));

test('moveTrack starts from the inline `from`, moves under ONE .moving gate, and releases on its own transitionend', async () => {
  reflows = 0;
  const p = moveTrack(cols, '--drawer-open', 0);
  // Synchronously: the start value was laid out (a forced reflow), the gate
  // is on, and the inline start is gone so the class value is the target.
  assert.ok(reflows >= 1, 'the start value is laid out before the transition may begin');
  assert.ok(cols.classList.contains('moving'));
  assert.equal(cols.style.getPropertyValue('--drawer-open'), '', 'the inline start is removed — the transition runs to the class value');
  await sleep(T_MOVE_MS / 2);
  assert.ok(cols.classList.contains('moving'), 'still moving at half time');
  ended('--side-open');
  await sleep(0);
  assert.ok(cols.classList.contains('moving'), 'another property ending is not this move');
  ended('--drawer-open');
  await p;
  assert.ok(!cols.classList.contains('moving'), 'the gate drops when THIS track has arrived');
});

test('without a transitionend (an engine that cut instead of moving) the gate still drops after moveMs() + 100', async () => {
  const before = Date.now();
  await moveTrack(cols, '--drawer-open', 0);
  const took = Date.now() - before;
  assert.ok(took >= T_MOVE_MS && took < T_MOVE_MS + 200, `safety net at ~${T_MOVE_MS + 100}ms, took ${took}`);
  assert.ok(!cols.classList.contains('moving'));
});

test('two overlapping moves keep the gate until the LAST one ends', async () => {
  const a = moveTrack(cols, '--side-open', 1);
  await sleep(T_MOVE_MS / 2);
  const b = moveTrack(cols, '--drawer-open', 0);
  ended('--side-open');
  await a;
  assert.ok(cols.classList.contains('moving'), 'the first move ending must not snap the second');
  ended('--drawer-open');
  await b;
  assert.ok(!cols.classList.contains('moving'));
});

test('under reduced motion the move is a cut: no waiting, no gate left behind', async () => {
  reduced = true;
  try {
    const before = Date.now();
    await moveTrack(cols, '--side-open', 1);
    assert.ok(Date.now() - before < T_MOVE_MS / 2, 'moveMs() is 0');
    assert.ok(!cols.classList.contains('moving'));
  } finally { reduced = false; }
});
