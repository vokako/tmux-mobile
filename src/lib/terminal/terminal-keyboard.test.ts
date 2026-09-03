import test from 'node:test';
import assert from 'node:assert/strict';
import { createDoubleTapDetector, encodeTerminalShortcut } from './terminal-keyboard.ts';

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
