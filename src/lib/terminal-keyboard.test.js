import test from 'node:test';
import assert from 'node:assert/strict';
import { encodeTerminalShortcut } from './terminal-keyboard.js';

function key(key, modifiers = {}) {
  return { key, code: '', metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...modifiers };
}

test('encodes Ctrl letters and punctuation as C0 bytes', () => {
  assert.equal(encodeTerminalShortcut(key('c', { ctrlKey: true })), '\x03');
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
