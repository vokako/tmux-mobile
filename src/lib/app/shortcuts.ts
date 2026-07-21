export const SHORTCUT_STORAGE_KEY = 'tmux_shortcuts';

export const SHORTCUT_DEFAULTS = Object.freeze({
  previousPage: 'Meta+KeyU',
  nextPage: 'Meta+KeyI',
  previousWindow: 'Alt+KeyU',
  nextWindow: 'Alt+KeyI',
  openTerminal: 'Meta+KeyT',
  openFiles: 'Meta+KeyF',
});

const MODIFIER_CODES = new Set([
  'AltLeft', 'AltRight', 'ControlLeft', 'ControlRight',
  'MetaLeft', 'MetaRight', 'ShiftLeft', 'ShiftRight',
]);

export type ShortcutAction = keyof typeof SHORTCUT_DEFAULTS;

export function shortcutFromEvent(event: KeyboardEvent): string {
  if (!event.code || MODIFIER_CODES.has(event.code)) return '';
  if (!event.metaKey && !event.ctrlKey && !event.altKey) return '';
  const parts = [];
  if (event.metaKey) parts.push('Meta');
  if (event.ctrlKey) parts.push('Ctrl');
  if (event.altKey) parts.push('Alt');
  if (event.shiftKey) parts.push('Shift');
  parts.push(event.code);
  return parts.join('+');
}

export function shortcutLabel(value: string | null | undefined): string {
  if (!value) return '—';
  return value
    .replace('Meta', '⌘')
    .replace('Ctrl', '⌃')
    .replace('Alt', '⌥')
    .replace('Shift', '⇧')
    .replace(/Key([A-Z])/g, '$1')
    .replace(/Digit([0-9])/g, '$1')
    .replaceAll('+', '');
}

export function actionForShortcut(bindings: Record<string, string>, value: string): string {
  if (!value) return '';
  return (Object.keys(SHORTCUT_DEFAULTS) as ShortcutAction[]).find(action => bindings[action] === value) || '';
}

export function cycleItem<T>(items: T[], current: T, direction: number): T | null {
  if (!items.length) return null;
  const index = items.indexOf(current);
  const start = index >= 0 ? index : 0;
  return items[(start + direction + items.length) % items.length]!; // modulo keeps the index in range
}
