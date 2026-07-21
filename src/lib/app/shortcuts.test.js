import test from 'node:test';
import assert from 'node:assert/strict';
import { SHORTCUT_DEFAULTS, actionForShortcut, cycleItem, shortcutFromEvent, shortcutLabel } from './shortcuts.ts';

function event(code, modifiers = {}) {
  return { code, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...modifiers };
}

test('normalizes Command and Option shortcuts by physical key', () => {
  assert.equal(shortcutFromEvent(event('KeyU', { metaKey: true })), 'Meta+KeyU');
  assert.equal(shortcutFromEvent(event('KeyI', { altKey: true })), 'Alt+KeyI');
  assert.equal(shortcutFromEvent(event('KeyK', { shiftKey: true })), '');
});

test('renders compact macOS shortcut labels', () => {
  assert.equal(shortcutLabel('Meta+Shift+KeyT'), '⌘⇧T');
  assert.equal(shortcutLabel('Alt+KeyU'), '⌥U');
  assert.equal(shortcutLabel(''), '—');
});

test('finds duplicate bindings before persistence', () => {
  assert.equal(actionForShortcut(SHORTCUT_DEFAULTS, 'Meta+KeyU'), 'previousPage');
  assert.equal(actionForShortcut(SHORTCUT_DEFAULTS, ''), '');
  assert.equal(actionForShortcut(SHORTCUT_DEFAULTS, 'Meta+KeyK'), '');
});

test('cycles page and window lists in both directions', () => {
  const items = ['sessions', 'terminal', 'team', 'files'];
  assert.equal(cycleItem(items, 'sessions', -1), 'files');
  assert.equal(cycleItem(items, 'files', 1), 'sessions');
  assert.equal(cycleItem(items, 'terminal', 1), 'team');
});
