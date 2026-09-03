import test from 'node:test';
import assert from 'node:assert/strict';
import { CTRL_ONE_SHOT_MS, createDoubleTapDetector, createOneShotCtrl, encodeTerminalShortcut } from './terminal-keyboard.ts';

function key(key: string, modifiers: Partial<KeyboardEvent> = {}): KeyboardEvent {
  return { key, code: '', metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...modifiers } as KeyboardEvent;
}

test('encodes Ctrl letters and punctuation as C0 bytes', () => {
  assert.equal(encodeTerminalShortcut(key('c', { ctrlKey: true })), '\x03');
  assert.equal(encodeTerminalShortcut(key('f', { ctrlKey: true })), '\x06');
  assert.equal(encodeTerminalShortcut(key('x', { ctrlKey: true })), '\x18');
  assert.equal(encodeTerminalShortcut(key('[', { ctrlKey: true })), '\x1b');
  assert.equal(encodeTerminalShortcut(key('\\', { ctrlKey: true })), '\x1c');
});

test('encodes Option text as an Escape-prefixed Meta key', () => {
  assert.equal(encodeTerminalShortcut(key('u', { altKey: true })), '\x1bu');
  assert.equal(encodeTerminalShortcut(key('Dead', { code: 'KeyU', altKey: true })), '\x1bu');
  assert.equal(encodeTerminalShortcut(key('Enter', { altKey: true })), '\x1b\r');
});

test('encodes combined modifiers for navigation and function keys', () => {
  assert.equal(encodeTerminalShortcut(key('ArrowLeft', { altKey: true })), '\x1b[1;3D');
  assert.equal(encodeTerminalShortcut(key('ArrowUp', { ctrlKey: true, altKey: true })), '\x1b[1;7A');
  assert.equal(encodeTerminalShortcut(key('F5', { ctrlKey: true })), '\x1b[15;5~');
});

test('leaves Command and unmodified keys to the app or xterm', () => {
  assert.equal(encodeTerminalShortcut(key('t', { metaKey: true })), '');
  assert.equal(encodeTerminalShortcut(key('a')), '');
});

// --- double-tap -------------------------------------------------------------

test('two clean taps close together in time and space make a double-tap, then the pair is consumed', () => {
  const d = createDoubleTapDetector(300, 40);
  assert.equal(d.tap({ x: 100, y: 100, t: 1000 }), false);
  assert.equal(d.tap({ x: 110, y: 95, t: 1200 }), true);
  // The pair is spent: a third tap starts a NEW pair, it does not chain.
  assert.equal(d.tap({ x: 110, y: 95, t: 1300 }), false);
});

test('a slow second tap or a far one is a fresh first tap', () => {
  const d = createDoubleTapDetector(300, 40);
  d.tap({ x: 100, y: 100, t: 1000 });
  assert.equal(d.tap({ x: 100, y: 100, t: 1301 }), false, 'too slow');
  // That slow tap is itself the new anchor.
  assert.equal(d.tap({ x: 100, y: 100, t: 1400 }), true);
  d.tap({ x: 100, y: 100, t: 5000 });
  assert.equal(d.tap({ x: 150, y: 100, t: 5100 }), false, 'too far');
});

test('a non-tap gesture between two taps breaks the pair', () => {
  const d = createDoubleTapDetector(300, 40);
  d.tap({ x: 100, y: 100, t: 1000 });
  d.reset(); // e.g. the finger scrolled, or the tap cancelled a selection
  assert.equal(d.tap({ x: 100, y: 100, t: 1100 }), false);
});

// --- Ctrl one-shot ------------------------------------------------------------

/** Manual clock: `fire()` runs the pending expiry, as the wall clock would. */
function fakeTimer() {
  let pending: (() => void) | null = null;
  let armedFor = -1;
  let cleared = 0;
  return {
    setTimer: (fn: () => void, ms: number) => { pending = fn; armedFor = ms; return 1; },
    clearTimer: () => { pending = null; cleared++; },
    fire() { const fn = pending; pending = null; fn?.(); },
    get pending() { return pending != null; },
    get armedFor() { return armedFor; },
    get cleared() { return cleared; },
  };
}

test('the armed Ctrl turns the next letter into its C0 byte and releases', () => {
  const changes: boolean[] = [];
  const ctrl = createOneShotCtrl({ onChange: (a) => changes.push(a), ...fakeTimer() });
  ctrl.toggle();
  assert.equal(ctrl.armed, true);
  assert.equal(ctrl.apply('c'), '\x03');
  assert.equal(ctrl.armed, false, 'one shot');
  assert.equal(ctrl.apply('c'), 'c', 'released: the next letter is plain');
  assert.deepEqual(changes, [true, false]);
});

test('non-letters pass through and leave the arm alone; tapping Ctrl again cancels', () => {
  const ctrl = createOneShotCtrl(fakeTimer());
  ctrl.toggle();
  assert.equal(ctrl.apply('1'), '1');
  assert.equal(ctrl.apply('，'), '，');
  assert.equal(ctrl.apply('ab'), 'ab', 'a two-character IME commit is not a letter');
  assert.equal(ctrl.armed, true);
  ctrl.toggle();
  assert.equal(ctrl.armed, false);
  assert.equal(ctrl.apply('a'), 'a');
});

test('the arm expires on its own after CTRL_ONE_SHOT_MS', () => {
  const timer = fakeTimer();
  const changes: boolean[] = [];
  const ctrl = createOneShotCtrl({ onChange: (a) => changes.push(a), ...timer });
  ctrl.toggle();
  assert.equal(timer.pending, true, 'arming starts the clock');
  assert.equal(timer.armedFor, CTRL_ONE_SHOT_MS);
  timer.fire();
  assert.equal(ctrl.armed, false);
  assert.deepEqual(changes, [true, false], 'the template mirror sees the expiry');
  assert.equal(ctrl.apply('c'), 'c', 'a letter typed minutes later is plain');
});

test('consuming, cancelling or disarming stops the clock; re-arming restarts it', () => {
  const timer = fakeTimer();
  const ctrl = createOneShotCtrl(timer);
  ctrl.toggle();
  ctrl.apply('x');
  assert.equal(timer.pending, false, 'consumed: no stale expiry left to fire');
  ctrl.toggle();
  ctrl.disarm(); // blur or pane switch
  assert.equal(timer.pending, false);
  assert.equal(ctrl.armed, false);
  ctrl.disarm(); // idempotent
  ctrl.toggle();
  assert.equal(timer.pending, true, 'a fresh arm gets a fresh clock');
  assert.ok(timer.cleared >= 2, 'every release clears before it re-sets — one clock at most');
});
