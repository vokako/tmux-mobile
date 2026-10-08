import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import {
  SHORTCUTS, SHORTCUT_GROUPS, actionForShortcut, cycleItem, defaultBindings, migrateBindings,
  reservedReason, resolveDefault, shortcutFromEvent, shortcutLabel, type ShortcutHost,
} from './shortcuts.ts';

function event(code: string, modifiers: Partial<KeyboardEvent> = {}): KeyboardEvent {
  return { code, metaKey: false, ctrlKey: false, altKey: false, shiftKey: false, ...modifiers } as KeyboardEvent;
}

test('normalizes Command and Option shortcuts by physical key', () => {
  assert.equal(shortcutFromEvent(event('KeyU', { metaKey: true })), 'Meta+KeyU');
  assert.equal(shortcutFromEvent(event('KeyI', { altKey: true })), 'Alt+KeyI');
  assert.equal(shortcutFromEvent(event('KeyK', { shiftKey: true })), '');
});

test('labels: compact glyphs on macOS, words elsewhere, punctuation by name', () => {
  assert.equal(shortcutLabel('Meta+Shift+KeyT', true), '⌘⇧T');
  assert.equal(shortcutLabel('Meta+Alt+BracketRight', true), '⌘⌥]');
  assert.equal(shortcutLabel('Ctrl+Alt+Digit3', false), 'Ctrl+Alt+3');
  assert.equal(shortcutLabel('', true), '—');
});

test('the registry: unique ids, every group used, every default resolvable and unique per platform (board 316)', () => {
  const ids = SHORTCUTS.map((s) => s.id);
  assert.equal(new Set(ids).size, ids.length);
  for (const g of SHORTCUT_GROUPS) assert.ok(SHORTCUTS.some((s) => s.group === g), `${g} has actions`);
  // The six pre-316 ids keep their storage keys.
  for (const id of ['previousPage', 'nextPage', 'previousWindow', 'nextWindow', 'openTerminal', 'openFiles']) assert.ok(ids.includes(id), id);
  for (const mac of [true, false]) {
    const d = defaultBindings(mac);
    const values = Object.values(d).filter(Boolean);
    assert.equal(new Set(values).size, values.length, `no two defaults share a combo (mac=${mac})`);
    for (const [id, v] of Object.entries(d)) assert.equal(reservedReason(v, mac), '', `${id} default ${v} is not reserved (mac=${mac})`);
  }
  assert.equal(resolveDefault('Mod+Alt+Digit1', true), 'Meta+Alt+Digit1');
  assert.equal(resolveDefault('Mod+Alt+Digit1', false), 'Ctrl+Alt+Digit1');
});

test('reserved combos are refused with a reason; ordinary ones are not', () => {
  assert.equal(reservedReason('Meta+KeyT', true), 'shortcutReservedBrowser', 'new tab');
  assert.equal(reservedReason('Meta+Shift+KeyT', true), 'shortcutReservedBrowser', 'reopen tab');
  assert.equal(reservedReason('Ctrl+KeyW', false), 'shortcutReservedBrowser', 'close tab');
  assert.equal(reservedReason('Meta+Digit2', true), 'shortcutReservedBrowser', 'tab 2');
  assert.equal(reservedReason('Alt+Digit2', false), 'shortcutReservedBrowser', 'tab 2 on Linux');
  assert.equal(reservedReason('Meta+Alt+KeyI', true), 'shortcutReservedBrowser', 'devtools');
  assert.equal(reservedReason('Ctrl+Shift+KeyJ', false), 'shortcutReservedBrowser', 'devtools');
  assert.equal(reservedReason('Meta+Shift+Digit4', true), 'shortcutReservedSystem', 'macOS screenshot');
  assert.equal(reservedReason('Meta+KeyH', true), 'shortcutReservedSystem', 'hide');
  assert.equal(reservedReason('Ctrl+Alt+KeyT', false), 'shortcutReservedSystem', 'Linux terminal');
  assert.equal(reservedReason('Meta+KeyU', true), '');
  assert.equal(reservedReason('Meta+Alt+Digit2', true), '');
  assert.equal(reservedReason('Alt+KeyI', false), '');
});

test('migration: stored choices stay, legacy and reserved defaults follow the new default, conflicts resolve to the choice', () => {
  const d = defaultBindings(true);
  // The old full-object store: every id at its old default, openTerminal on ⌘T (reserved).
  const legacy = { previousPage: 'Meta+KeyU', nextPage: 'Meta+KeyI', previousWindow: 'Alt+KeyU', nextWindow: 'Alt+KeyI', openTerminal: 'Meta+KeyT', openFiles: 'Meta+KeyF' };
  const m = migrateBindings(legacy, true);
  assert.equal(m.openTerminal, d.openTerminal, '⌘T never reached the page: re-defaulted');
  assert.equal(m.openFiles, d.openFiles, 'an untouched legacy default follows the new one');
  assert.equal(m.goHub, d.goHub, 'new ids get their defaults');
  // A user choice survives, and a new default that lands on it gives way.
  const chosen = migrateBindings({ previousPage: 'Meta+Alt+Digit1', bogus: 'Meta+KeyZ' }, true);
  assert.equal(chosen.previousPage, 'Meta+Alt+Digit1');
  assert.equal(chosen.goHub, '', 'goHub’s default met a stored choice: unbound, never a duplicate');
  assert.ok(!('bogus' in chosen));
  assert.equal(migrateBindings({ nextPage: '' }, true).nextPage, '', 'a cleared binding stays cleared');
  assert.equal(actionForShortcut(d, d.nextServer!), 'nextServer');
});

function host(over: Partial<ShortcutHost> = {}): ShortcutHost & { calls: string[] } {
  const calls: string[] = [];
  const h = {
    connected: true, hub: true, page: 'hub', terminalTarget: true, servers: 2, splitEligible: true, sidebarPage: true,
    goTo: (p: string) => calls.push(`goTo ${p}`), cyclePage: (d: number) => calls.push(`page ${d}`),
    cycleServer: (d: number) => calls.push(`server ${d}`), openServers: () => calls.push('servers'),
    toggleSidebar: () => calls.push('sidebar'), toggleSplit: () => calls.push('split'),
    focusComposer: () => calls.push('composer'), focusTerminal: () => calls.push('terminal'),
    cycleWindow: (d: number) => calls.push(`window ${d}`), calls, ...over,
  };
  return h;
}

test('every action runs one host primitive, and says why when it cannot', () => {
  for (const def of SHORTCUTS) {
    const h = host({ page: def.group === 'terminal' ? 'terminal' : 'hub' });
    assert.equal(def.need(h), '', `${def.id} is available in a full context`);
    def.run(h);
    assert.equal(h.calls.length, 1, `${def.id} calls exactly one primitive`);
  }
  const need = (id: string, over: Partial<ShortcutHost>) => SHORTCUTS.find((s) => s.id === id)!.need(host(over));
  assert.equal(need('goHub', { hub: false }), 'shortcutNeedHub');
  assert.equal(need('nextServer', { servers: 1 }), 'shortcutNeedServers');
  assert.equal(need('toggleSplit', { splitEligible: false }), 'shortcutNeedWide');
  assert.equal(need('nextWindow', { page: 'files' }), 'shortcutNeedTerminalPage');
  assert.equal(need('openFiles', { connected: false }), 'shortcutNeedConnection');
  assert.equal(need('goSettings', { connected: false }), '', 'Settings works disconnected');
});

test('App’s handler reads only the registry: one lookup, one need check, one run (board 316)', async () => {
  const app = await readFile(new URL('../../App.svelte', import.meta.url), 'utf8');
  const handler = app.match(/const onShortcut = \(event\) => \{[\s\S]*?\n    \};/u)?.[0] ?? '';
  assert.match(handler, /const def = shortcutDef\(shortcuts\.action\(shortcutFromEvent\(event\)\)\);/u);
  assert.match(handler, /if \(!def \|\| def\.need\(shortcutHost\)\) return;/u);
  assert.match(handler, /def\.run\(shortcutHost\);/u);
  assert.doesNotMatch(handler, /action ===|'previousPage'|'openFiles'/u, 'no per-action branch');
  assert.match(app, /const shortcutsOn = \$derived\(!layout\.isTouchDevice\);/u, 'the gate is the desktop form factor');
  assert.match(app, /showShortcuts=\{shortcutsOn\}/u);
  assert.doesNotMatch(app, /showShortcuts=\{isTauriDesktop\}/u);
  // Every rail page shows its binding on hover.
  const rail = /const RAIL_SHORTCUT = \{([^}]+)\}/u.exec(app)?.[1] ?? '';
  for (const slot of ['hub', 'board', 'terminal', 'files', 'agents', 'prefs']) assert.match(rail, new RegExp(`\\b${slot}: '`, 'u'), slot);
});

test('cycles page and window lists in both directions', () => {
  const items = ['sessions', 'terminal', 'team', 'files'];
  assert.equal(cycleItem(items, 'sessions', -1), 'files');
  assert.equal(cycleItem(items, 'files', 1), 'sessions');
  assert.equal(cycleItem(items, 'terminal', 1), 'team');
});
