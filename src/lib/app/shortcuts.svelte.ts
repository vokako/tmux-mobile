import { SHORTCUTS, SHORTCUT_STORAGE_KEY, actionForShortcut, defaultBindings, migrateBindings, reservedReason, shortcutDef } from './shortcuts.ts';

function loadShortcuts(): Record<string, string> {
  try { return migrateBindings(JSON.parse(localStorage.getItem(SHORTCUT_STORAGE_KEY) || '{}')); }
  catch { return defaultBindings(); }
}

const state = $state(loadShortcuts());

function persist() {
  localStorage.setItem(SHORTCUT_STORAGE_KEY, JSON.stringify(state));
}

export function isShortcutInputTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  if (target.closest('[data-shortcut-recorder]')) return true;
  if (target.closest('.xterm')) return false;
  return !!target.closest('input, textarea, select, [role="combobox"], [role="listbox"], [role="menu"], [role="dialog"], [contenteditable="true"]');
}

export const shortcuts = {
  get(action: string) { return state[action] || ''; },
  action(value: string) { return actionForShortcut(state, value); },
  /** '' on success; otherwise the i18n key of why the binding was refused. */
  set(action: string, value: string): string {
    if (!shortcutDef(action)) return 'shortcutUnknown';
    if (value) {
      const reserved = reservedReason(value);
      if (reserved) return reserved;
      const conflict = actionForShortcut(state, value);
      if (conflict && conflict !== action) return 'shortcutConflict';
    }
    state[action] = value;
    persist();
    return '';
  },
  /** One action back to its default (if the default is free). */
  resetOne(action: string): string {
    const def = defaultBindings()[action];
    if (def == null) return 'shortcutUnknown';
    const holder = actionForShortcut(state, def);
    if (holder && holder !== action) return 'shortcutConflict';
    state[action] = def;
    persist();
    return '';
  },
  isDefault(action: string) { return state[action] === defaultBindings()[action]; },
  reset() {
    const d = defaultBindings();
    for (const s of SHORTCUTS) state[s.id] = d[s.id]!;
    persist();
  },
};
