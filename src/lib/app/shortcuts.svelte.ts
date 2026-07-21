import { SHORTCUT_DEFAULTS, SHORTCUT_STORAGE_KEY, actionForShortcut, type ShortcutAction } from './shortcuts.ts';

type Bindings = Record<ShortcutAction, string>;

function loadShortcuts(): Bindings {
  try {
    const stored = JSON.parse(localStorage.getItem(SHORTCUT_STORAGE_KEY) || '{}');
    return Object.fromEntries((Object.keys(SHORTCUT_DEFAULTS) as ShortcutAction[]).map(action => [
      action,
      typeof stored[action] === 'string' ? stored[action] : SHORTCUT_DEFAULTS[action],
    ])) as Bindings;
  } catch {
    return { ...SHORTCUT_DEFAULTS };
  }
}

const state = $state(loadShortcuts());

function persist() {
  localStorage.setItem(SHORTCUT_STORAGE_KEY, JSON.stringify(state));
}

export function isShortcutInputTarget(target: EventTarget | null): boolean {
  if (!(target instanceof Element)) return false;
  if (target.closest('[data-shortcut-recorder]')) return true;
  if (target.closest('.xterm')) return false;
  return !!target.closest('input, textarea, select, [contenteditable="true"]');
}

export const shortcuts = {
  get(action: ShortcutAction) { return state[action] || ''; },
  action(value: string) { return actionForShortcut(state, value); },
  set(action: ShortcutAction, value: string) {
    if (!(action in SHORTCUT_DEFAULTS)) return false;
    const conflict = actionForShortcut(state, value);
    if (value && conflict && conflict !== action) return false;
    state[action] = value;
    persist();
    return true;
  },
  reset() {
    for (const [action, value] of Object.entries(SHORTCUT_DEFAULTS) as [ShortcutAction, string][]) state[action] = value;
    persist();
  },
};
