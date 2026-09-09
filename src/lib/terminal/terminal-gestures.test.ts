import test from 'node:test';
import assert from 'node:assert/strict';
import { JSDOM } from 'jsdom';
import { createTerminalGestures } from './terminal-gestures.ts';
import type { GestureEnvironment, GestureHost } from './terminal-gestures.ts';

type Call = [string, ...unknown[]];
// Unit traces supply only consumed event fields; jsdom below tests real Event semantics.
const point = (x = 100, y = 200, id = 7) => ({ clientX: x, clientY: y, identifier: id }) as Touch;

function fixture() {
  const calls: Call[] = [];
  const state = {
    available: true, selected: false, pinned: false, handle: null as 'start' | 'end' | null,
    scrollbar: false, viewport: 40, lineHeight: 20, word: true, top: 0, bottom: 400,
  };
  const record = <T>(name: string, args: unknown[], value: T): T => {
    calls.push([name, ...args]);
    return value;
  };
  const host: GestureHost = {
    available: () => record('available', [], state.available),
    hasSelection: () => record('hasSelection', [], state.selected),
    isPinned: () => record('isPinned', [], state.pinned),
    pinUpdates: () => { calls.push(['pinUpdates']); state.pinned = true; },
    requestRenderRelease: ms => { calls.push(['requestRenderRelease', ms]); },
    hitHandle: (x, y) => record('hitHandle', [x, y], state.handle),
    grabHandle: (side, x, y) => record('grabHandle', [side, x, y], { dx: 4, dy: 6 }),
    dragHeadAt: (x, y) => { calls.push(['dragHeadAt', x, y]); },
    extendHeadAt: (x, y) => { calls.push(['extendHeadAt', x, y]); },
    tryWordSelection(x, y) {
      calls.push(['tryWordSelection', x, y]);
      if (state.word) state.selected = state.pinned = true;
      return state.word;
    },
    clearSelectionOutside(x, y) { calls.push(['clearSelectionOutside', x, y]); state.selected = false; },
    isScrollbarPoint: x => record('isScrollbarPoint', [x], state.scrollbar),
    scrollPosition: () => record('scrollPosition', [], state.viewport),
    dragScrollbar: (startY, startViewport, y) => { calls.push(['dragScrollbar', startY, startViewport, y]); },
    lineHeight: () => record('lineHeight', [], state.lineHeight),
    edgeBounds: () => record('edgeBounds', [], { top: state.top, bottom: state.bottom }),
    scrollLines: lines => { calls.push(['scrollLines', lines]); state.viewport += lines; },
    openFromDoubleTap: () => { calls.push(['openFromDoubleTap']); },
  };
  let now = 1000, nextId = 1;
  const delays = new Map<number, { at: number; callback: () => void }>();
  const frames = new Map<number, FrameRequestCallback>();
  const environment: GestureEnvironment = {
    now: () => record('now', [], now),
    setDelay(callback, ms) {
      calls.push(['setDelay', ms]);
      const id = nextId++;
      delays.set(id, { at: now + ms, callback });
      return id;
    },
    clearDelay(id) { calls.push(['clearDelay', id]); delays.delete(id); },
    requestFrame(callback) {
      calls.push(['requestFrame']);
      const id = nextId++;
      frames.set(id, callback);
      return id;
    },
    cancelFrame(id) { calls.push(['cancelFrame', id]); frames.delete(id); },
    vibrate: ms => { calls.push(['vibrate', ms]); },
  };
  const gestures = createTerminalGestures(host, environment);
  function advance(ms: number) {
    const until = now + ms;
    while (true) {
      const due = [...delays].filter(([, timer]) => timer.at <= until).sort((a, b) => a[1].at - b[1].at)[0];
      if (!due) break;
      now = due[1].at;
      delays.delete(due[0]);
      due[1].callback();
    }
    now = until;
  }
  function frame() {
    now += 16;
    const current = [...frames];
    frames.clear();
    for (const [, callback] of current) callback(now);
  }
  function event(touches = [point()], changedTouches = touches, cancelable = true) {
    return {
      touches, changedTouches, target: null, cancelable, defaultPrevented: false,
      preventDefault(this: { cancelable: boolean; defaultPrevented: boolean }) {
        calls.push(['preventDefault']);
        if (this.cancelable) this.defaultPrevented = true;
      },
    } as unknown as TouchEvent;
  }
  function tap(p = point()) {
    gestures.onTouchStart(event([p]));
    const end = event([], [p]);
    gestures.onTouchEnd(end);
    return end;
  }
  function coast() {
    gestures.onTouchStart(event());
    advance(16);
    gestures.onTouchMove(event([point(100, 180)]));
    gestures.onTouchEnd(event([], [point(100, 180)]));
  }
  return {
    state, host, calls, gestures, delays, frames, advance, frame, event, tap, coast,
    named: (name: string) => calls.filter(c => c[0] === name),
  };
}

test('construction is inert and exposes only the ten approved controller operations', () => {
  const f = fixture();
  assert.deepEqual(f.calls, []);
  assert.equal(f.delays.size + f.frames.size, 0);
  assert.deepEqual(Object.keys(f.gestures).sort(), [
    'onTouchStart', 'onTouchMove', 'onTouchEnd', 'onTouchCancel', 'isIdle', 'isCoasting',
    'stopMomentum', 'resetAfterVisibility', 'cancelHold', 'dispose',
  ].sort());
  assert.equal(f.gestures.isIdle(), true);
  assert.equal(f.gestures.isCoasting(), false);
  assert.deepEqual(f.calls, [], 'state queries do not call Root or the environment');
});

test('hold selects at 500ms, extends the head, and ends without releasing the selection pin', () => {
  const f = fixture();
  f.gestures.onTouchStart(f.event());
  assert.equal(f.gestures.isIdle(), false);
  f.advance(499);
  assert.deepEqual(f.named('tryWordSelection'), []);
  f.advance(1);
  assert.deepEqual(f.calls.slice(-3), [['available'], ['tryWordSelection', 100, 200], ['vibrate', 15]]);
  f.gestures.onTouchMove(f.event([point(120, 220)]));
  assert.deepEqual(f.calls.slice(-2), [['extendHeadAt', 120, 220], ['preventDefault']]);
  f.gestures.onTouchEnd(f.event());
  assert.equal(f.gestures.isIdle(), true);
  assert.equal(f.state.selected && f.state.pinned, true);
  assert.deepEqual(f.named('requestRenderRelease'), []);
});

test('a failed word selection or disappeared terminal cannot create a long-press gesture', () => {
  for (const available of [true, false]) {
    const f = fixture();
    f.gestures.onTouchStart(f.event());
    f.state.word = false;
    f.state.available = available;
    f.advance(500);
    assert.equal(f.named('tryWordSelection').length, Number(available));
    assert.deepEqual(f.named('vibrate'), []);
    assert.equal(f.state.selected, false);
    f.gestures.onTouchEnd(f.event());
    assert.equal(f.gestures.isIdle(), true);
  }
});

test('a second clean tap opens synchronously after preventDefault at the inclusive time/distance boundary', () => {
  const f = fixture();
  assert.equal(f.tap().defaultPrevented, false);
  assert.deepEqual(f.named('openFromDoubleTap'), []);
  f.advance(300);
  assert.equal(f.tap(point(140, 200)).defaultPrevented, true);
  assert.deepEqual(f.calls.slice(-2), [['preventDefault'], ['openFromDoubleTap']]);
  f.advance(1);
  f.tap();
  assert.equal(f.named('openFromDoubleTap').length, 1, 'the consumed pair does not chain');
});

test('a selection acquired after construction spends its dismissal tap, never opening the keyboard', () => {
  const f = fixture();
  f.tap();
  f.state.selected = true;
  f.advance(10);
  f.tap();
  assert.deepEqual(f.named('clearSelectionOutside'), [['clearSelectionOutside', 100, 200]]);
  f.advance(10);
  f.tap();
  assert.deepEqual(f.named('openFromDoubleTap'), []);
  f.advance(10);
  f.tap();
  assert.equal(f.named('openFromDoubleTap').length, 1);
});

test('cancel and scrollbar release break a clean-tap pair', () => {
  for (const cancel of [true, false]) {
    const f = fixture();
    f.tap();
    f.advance(10);
    if (cancel) f.gestures.onTouchCancel();
    else {
      f.state.scrollbar = true;
      f.tap();
      f.state.scrollbar = false;
    }
    f.advance(10);
    f.tap();
    assert.deepEqual(f.named('openFromDoubleTap'), []);
  }
});

test('a handle wins over the scrollbar, grabs before pinning, and compensates both coordinates', () => {
  const f = fixture();
  f.state.selected = f.state.scrollbar = true;
  f.state.handle = 'end';
  f.gestures.onTouchStart(f.event());
  assert.deepEqual(f.calls, [['hasSelection'], ['hitHandle', 100, 200], ['grabHandle', 'end', 100, 200], ['pinUpdates']]);
  f.gestures.onTouchMove(f.event([point(104, 206)]));
  assert.deepEqual(f.calls.slice(-3), [['dragHeadAt', 100, 200], ['edgeBounds'], ['preventDefault']]);
  f.gestures.onTouchEnd(f.event());
  assert.equal(f.state.selected && f.state.pinned, true);
  assert.deepEqual(f.named('requestRenderRelease'), []);
});

test('scrollbar records the live viewport after pinning and delegates release without clearing a selection', () => {
  const f = fixture();
  f.state.scrollbar = f.state.selected = true;
  f.host.pinUpdates = () => { f.calls.push(['pinUpdates']); f.state.viewport = 63; f.state.pinned = true; };
  f.gestures.onTouchStart(f.event());
  assert.deepEqual(f.calls.slice(-2), [['pinUpdates'], ['scrollPosition']]);
  f.gestures.onTouchMove(f.event([point(100, 170)]));
  assert.deepEqual(f.calls.slice(-2), [['dragScrollbar', 200, 63, 170], ['preventDefault']]);
  f.gestures.onTouchEnd(f.event());
  assert.deepEqual(f.named('requestRenderRelease'), [['requestRenderRelease', 500]]);
  assert.equal(f.state.selected && f.state.pinned, true, 'Root owns the eventual release and its selection guard');
});

test('content scroll accumulates fractions and cancels hold before pin/scroll/prevention', () => {
  const f = fixture();
  f.gestures.onTouchStart(f.event());
  f.advance(16);
  assert.equal(f.gestures.onTouchMove(f.event([point(100, 190)])), undefined);
  assert.equal(f.delays.size, 1);
  assert.deepEqual(f.named('scrollLines'), []);
  f.advance(16);
  f.gestures.onTouchMove(f.event([point(100, 179)]));
  assert.deepEqual(f.calls.slice(-4), [['clearDelay', 1], ['pinUpdates'], ['scrollLines', 1], ['preventDefault']]);
  assert.equal(f.delays.size, 0);
  f.advance(16);
  f.gestures.onTouchMove(f.event([point(100, 160)]));
  assert.deepEqual(f.named('scrollLines'), [['scrollLines', 1], ['scrollLines', 1]], 'one-pixel remainder survives');
});

test('release starts a coast from live pin state, finishes with the 200ms Root release request', () => {
  const f = fixture();
  f.coast();
  assert.equal(f.gestures.isCoasting(), true);
  assert.deepEqual(f.named('requestRenderRelease'), []);
  let count = 0;
  while (f.frames.size && count++ < 200) f.frame();
  assert.ok(count < 200);
  assert.equal(f.gestures.isCoasting(), false);
  assert.deepEqual(f.named('requestRenderRelease'), [['requestRenderRelease', 200]]);
  assert.equal(f.state.pinned, true, 'the controller never writes Root release state');
});

test('slow release schedules 500ms; a pin removed before release prevents coast scheduling', () => {
  const f = fixture();
  f.gestures.onTouchStart(f.event());
  f.advance(400);
  f.gestures.onTouchMove(f.event([point(100, 180)]));
  f.gestures.onTouchEnd(f.event());
  assert.equal(f.frames.size, 0);
  assert.deepEqual(f.named('requestRenderRelease'), [['requestRenderRelease', 500]]);
  const g = fixture();
  g.gestures.onTouchStart(g.event());
  g.advance(16);
  g.gestures.onTouchMove(g.event([point(100, 180)]));
  g.state.pinned = false;
  g.gestures.onTouchEnd(g.event());
  assert.equal(g.frames.size, 0);
  assert.deepEqual(g.named('requestRenderRelease'), []);
});

test('input stopMomentum and a new touch both cancel a running coast synchronously', () => {
  for (const input of [true, false]) {
    const f = fixture();
    f.coast();
    f.frame();
    const before = f.named('scrollLines').length;
    if (input) f.gestures.stopMomentum();
    else f.gestures.onTouchStart(f.event());
    assert.equal(f.gestures.isCoasting(), false);
    assert.equal(f.frames.size, 0);
    f.frame();
    assert.equal(f.named('scrollLines').length, before);
  }
});

test('edge scrolling remeasures and remaps the head after the scroll, then cancel preserves selection', () => {
  const f = fixture();
  f.state.selected = true;
  f.state.handle = 'start';
  f.gestures.onTouchStart(f.event());
  f.gestures.onTouchMove(f.event([point(104, 398)]));
  assert.equal(f.frames.size, 1);
  f.state.bottom = 398;
  f.frame();
  assert.deepEqual(f.calls.slice(-5), [
    ['available'], ['edgeBounds'], ['scrollLines', 2], ['dragHeadAt', 100, 392], ['requestFrame'],
  ]);
  f.gestures.onTouchCancel();
  assert.equal(f.frames.size, 0);
  assert.equal(f.state.selected && f.state.pinned, true);
  assert.deepEqual(f.named('requestRenderRelease'), []);
});

test('cancel without a selection delegates the 100ms release without writing the pin itself', () => {
  const f = fixture();
  f.coast();
  f.gestures.onTouchCancel();
  assert.equal(f.delays.size + f.frames.size, 0);
  assert.equal(f.gestures.isIdle(), true);
  assert.deepEqual(f.named('requestRenderRelease'), [['requestRenderRelease', 100]]);
  assert.equal(f.state.pinned, true);
});

test('move tracks one identifier; end deliberately keeps the existing changedTouches[0] behavior', () => {
  const f = fixture();
  f.gestures.onTouchStart(f.event());
  f.gestures.onTouchMove(f.event([point(50, 50, 8)]));
  assert.deepEqual(f.named('scrollLines'), []);
  f.state.selected = true;
  f.gestures.onTouchEnd(f.event([], [point(50, 50, 8), point()]));
  assert.deepEqual(f.named('clearSelectionOutside'), [['clearSelectionOutside', 50, 50]]);
  assert.equal(f.gestures.isIdle(), true, 'do not repair extra-finger handling in a mechanical move');
});

test('visibility reset is not touchcancel: no hold cancellation, pair reset or Root release', () => {
  const f = fixture();
  f.tap();
  f.advance(10);
  f.gestures.resetAfterVisibility();
  f.tap();
  assert.equal(f.named('openFromDoubleTap').length, 1, 'visibility reset preserves the existing pair');
  f.gestures.onTouchStart(f.event());
  f.calls.length = 0;
  f.gestures.resetAfterVisibility();
  assert.deepEqual(f.calls, []);
  assert.equal(f.delays.size, 1);
  f.advance(500);
  assert.deepEqual(f.named('tryWordSelection'), [], 'idle mode makes the already pending hold a no-op');
  assert.deepEqual(f.named('requestRenderRelease'), []);
});

test('two-phase teardown cancels hold, coast and edge work without selection/render/keyboard commands', () => {
  for (const pending of ['hold', 'coast', 'edge'] as const) {
    const f = fixture();
    if (pending === 'coast') f.coast();
    else {
      if (pending === 'edge') { f.state.selected = true; f.state.handle = 'end'; }
      f.gestures.onTouchStart(f.event());
      if (pending === 'edge') f.gestures.onTouchMove(f.event([point(104, 398)]));
    }
    assert.ok(f.delays.size + f.frames.size > 0);
    f.calls.length = 0;
    f.gestures.cancelHold();
    if (pending === 'hold') assert.equal(f.delays.size, 0, 'hold stops in the original pre-blur cleanup slot');
    f.gestures.dispose();
    assert.equal(f.delays.size + f.frames.size, 0);
    assert.ok(f.calls.every(([name]) => name === 'clearDelay' || name === 'cancelFrame'));
    f.calls.length = 0;
    f.advance(1000);
    f.frame();
    f.gestures.dispose();
    assert.deepEqual(f.calls, []);
  }
});

test('controller instances never share gesture state or scheduled work', () => {
  const a = fixture(), b = fixture();
  a.gestures.onTouchStart(a.event());
  b.coast();
  a.gestures.dispose();
  assert.equal(b.gestures.isCoasting(), true);
  b.frame();
  b.frame();
  assert.ok(b.named('scrollLines').length > 1);
  assert.deepEqual(a.named('tryWordSelection'), []);
  assert.equal(a.gestures.isCoasting(), false);
});

test('jsdom toolbar descendants bypass terminal handling; the same listener refs detach cleanly', () => {
  const dom = new JSDOM('<div id="term"><div class="sel-toolbar"><button><span>Copy</span></button></div></div>');
  try {
    const f = fixture(), root = dom.window.document.querySelector('#term')!;
    const refs = [
      ['touchstart', f.gestures.onTouchStart, true],
      ['touchmove', f.gestures.onTouchMove, false],
      ['touchend', f.gestures.onTouchEnd, false],
      ['touchcancel', f.gestures.onTouchCancel, true],
    ] as const;
    for (const [name, handler, passive] of refs) root.addEventListener(name, handler as EventListener, { passive });
    const touch = (name: string) => new dom.window.TouchEvent(name, { bubbles: true, cancelable: true, touches: [point()] });
    f.state.selected = f.state.scrollbar = true;
    f.state.handle = 'end';
    root.querySelector('span')!.dispatchEvent(touch('touchstart'));
    assert.deepEqual(f.calls, [], 'closest checks descendants, before Root selection or scrollbar hit tests');
    assert.equal(f.gestures.isIdle(), true);
    root.dispatchEvent(touch('touchstart'));
    assert.equal(f.gestures.isIdle(), false);
    for (const [name, handler] of refs) root.removeEventListener(name, handler as EventListener);
    f.gestures.dispose();
    f.calls.length = 0;
    for (const [name] of refs) root.dispatchEvent(touch(name));
    assert.deepEqual(f.calls, [], 'the detached old owner receives no more events');
  } finally { dom.window.close(); }
});

test('jsdom touchend cancels the synthetic event only when cancelable, before synchronous opening', () => {
  for (const cancelable of [true, false]) {
    const dom = new JSDOM('<div id="term"></div>');
    try {
      const f = fixture(), root = dom.window.document.querySelector('#term')!;
      let end: TouchEvent;
      let opened = 0;
      f.host.openFromDoubleTap = () => { assert.equal(end.defaultPrevented, cancelable); opened++; };
      root.addEventListener('touchstart', f.gestures.onTouchStart as EventListener, { passive: true });
      root.addEventListener('touchend', f.gestures.onTouchEnd as EventListener, { passive: false });
      for (let i = 0; i < 2; i++) {
        root.dispatchEvent(new dom.window.TouchEvent('touchstart', { touches: [point()] }));
        end = new dom.window.TouchEvent('touchend', { cancelable, changedTouches: [point()] });
        assert.equal(root.dispatchEvent(end), !(i === 1 && cancelable));
        assert.equal(opened, i);
        f.advance(10);
      }
    } finally { dom.window.close(); }
  }
});
