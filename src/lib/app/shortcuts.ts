// Desktop keyboard shortcuts (board 316): ONE registry. Every action is
// declared here once — its id (the storage key), label, group, default and
// what it needs — and both App's handler and the Settings › Shortcuts tab are
// derived from it. App supplies a ShortcutHost (the primitives the actions
// call); there is no per-action branch anywhere else.
//
// The gate is the desktop FORM FACTOR (a pointer and a keyboard, i.e. not the
// touch layout), not the Tauri shell: the owner runs the app in a desktop
// browser, where the old isTauriDesktop gate hid the tab and the handler, and
// "the shortcuts don't work" (owner, 2026-10-08).

export const SHORTCUT_STORAGE_KEY = 'tmux_shortcuts';

export type ShortcutGroup = 'pages' | 'servers' | 'panels' | 'terminal';
export const SHORTCUT_GROUPS: readonly ShortcutGroup[] = ['pages', 'servers', 'panels', 'terminal'];

/** What the actions can do and know. App implements it. */
export interface ShortcutHost {
  connected: boolean;
  hub: boolean;               // the server hosts the Hub (hub / board / agents pages)
  page: string;
  terminalTarget: boolean;    // a pane is open
  servers: number;            // saved servers
  splitEligible: boolean;     // a wide desktop window
  sidebarPage: boolean;       // the current page has the shell sidebar
  goTo(page: string): void;
  cyclePage(direction: number): void;
  cycleServer(direction: number): void;
  openServers(): void;
  toggleSidebar(): void;
  toggleSplit(): void;
  focusComposer(): void;
  focusTerminal(): void;
  cycleWindow(direction: number): void;
}

/** '' = available; otherwise the i18n key of the reason it is not. A reason
 * means "not here, not now": the key is left to the browser. */
type Need = (h: ShortcutHost) => string;
const connected: Need = (h) => (h.connected ? '' : 'shortcutNeedConnection');
const hub: Need = (h) => connected(h) || (h.hub ? '' : 'shortcutNeedHub');

export interface ShortcutDef {
  id: string;
  label: string;
  group: ShortcutGroup;
  /** Platform-neutral default: `Mod` is ⌘ on macOS and Ctrl elsewhere. */
  default: string;
  need: Need;
  run: (h: ShortcutHost) => void;
}

// Defaults are chosen OUTSIDE everything a browser or the OS keeps for itself
// (RESERVED below): Mod+Alt+digit for pages, Mod+Alt+bracket/key for the
// rest. The six ids from before board 316 keep their storage keys.
export const SHORTCUTS: readonly ShortcutDef[] = Object.freeze([
  { id: 'previousPage', label: 'shortcutPreviousPage', group: 'pages', default: 'Mod+KeyU', need: connected, run: (h) => h.cyclePage(-1) },
  { id: 'nextPage', label: 'shortcutNextPage', group: 'pages', default: 'Mod+KeyI', need: connected, run: (h) => h.cyclePage(1) },
  { id: 'goHub', label: 'shortcutGoHub', group: 'pages', default: 'Mod+Alt+Digit1', need: hub, run: (h) => h.goTo('hub') },
  { id: 'goBoard', label: 'shortcutGoBoard', group: 'pages', default: 'Mod+Alt+Digit2', need: hub, run: (h) => h.goTo('board') },
  { id: 'openTerminal', label: 'shortcutOpenTerminal', group: 'pages', default: 'Mod+Alt+Digit3', need: connected, run: (h) => h.goTo('terminal') },
  { id: 'openFiles', label: 'shortcutOpenFiles', group: 'pages', default: 'Mod+Alt+Digit4', need: connected, run: (h) => h.goTo('files') },
  { id: 'goAgents', label: 'shortcutGoAgents', group: 'pages', default: 'Mod+Alt+Digit5', need: hub, run: (h) => h.goTo('agents') },
  { id: 'goSettings', label: 'shortcutGoSettings', group: 'pages', default: 'Mod+Alt+Digit6', need: () => '', run: (h) => h.goTo('prefs') },
  { id: 'previousServer', label: 'shortcutPreviousServer', group: 'servers', default: 'Mod+Alt+BracketLeft',
    need: (h) => (h.servers > 1 ? '' : 'shortcutNeedServers'), run: (h) => h.cycleServer(-1) },
  { id: 'nextServer', label: 'shortcutNextServer', group: 'servers', default: 'Mod+Alt+BracketRight',
    need: (h) => (h.servers > 1 ? '' : 'shortcutNeedServers'), run: (h) => h.cycleServer(1) },
  { id: 'openServers', label: 'shortcutOpenServers', group: 'servers', default: 'Mod+Alt+KeyS', need: connected, run: (h) => h.openServers() },
  { id: 'toggleSidebar', label: 'shortcutToggleSidebar', group: 'panels', default: 'Mod+Backslash',
    need: (h) => connected(h) || (h.sidebarPage ? '' : 'shortcutNeedSidebarPage'), run: (h) => h.toggleSidebar() },
  { id: 'toggleSplit', label: 'shortcutToggleSplit', group: 'panels', default: 'Mod+Alt+Backslash',
    need: (h) => connected(h) || (h.splitEligible ? '' : 'shortcutNeedWide'), run: (h) => h.toggleSplit() },
  { id: 'focusComposer', label: 'shortcutFocusComposer', group: 'panels', default: 'Mod+Alt+KeyM', need: hub, run: (h) => h.focusComposer() },
  { id: 'focusTerminal', label: 'shortcutFocusTerminal', group: 'panels', default: 'Mod+Alt+KeyK',
    need: (h) => connected(h) || (h.terminalTarget ? '' : 'shortcutNeedPane'), run: (h) => h.focusTerminal() },
  { id: 'previousWindow', label: 'shortcutPreviousWindow', group: 'terminal', default: 'Alt+KeyU',
    need: (h) => connected(h) || (h.page === 'terminal' ? '' : 'shortcutNeedTerminalPage'), run: (h) => h.cycleWindow(-1) },
  { id: 'nextWindow', label: 'shortcutNextWindow', group: 'terminal', default: 'Alt+KeyI',
    need: (h) => connected(h) || (h.page === 'terminal' ? '' : 'shortcutNeedTerminalPage'), run: (h) => h.cycleWindow(1) },
] satisfies ShortcutDef[]);

export type ShortcutAction = string;
export const shortcutDef = (id: string) => SHORTCUTS.find((s) => s.id === id);

export const isMac = (platform = typeof navigator === 'undefined' ? '' : navigator.platform) => /Mac|iPhone|iPad/u.test(platform);

/** A default with `Mod` resolved for this platform. */
export function resolveDefault(value: string, mac = isMac()): string {
  return value.replace(/^Mod\+/u, mac ? 'Meta+' : 'Ctrl+');
}
export function defaultBindings(mac = isMac()): Record<string, string> {
  return Object.fromEntries(SHORTCUTS.map((s) => [s.id, resolveDefault(s.default, mac)]));
}

/** The defaults the six pre-316 ids shipped with: a stored copy of one of
 * these is a default the user never chose, so it follows the new default. */
const LEGACY_DEFAULTS: Record<string, string> = {
  previousPage: 'Meta+KeyU', nextPage: 'Meta+KeyI', previousWindow: 'Alt+KeyU',
  nextWindow: 'Alt+KeyI', openTerminal: 'Meta+KeyT', openFiles: 'Meta+KeyF',
};

/** Stored bindings → the current registry. Unknown ids are dropped; a value
 * the browser would never deliver (a reserved combo) falls back to the
 * default; a legacy default is re-defaulted; any other stored choice stays. */
export function migrateBindings(stored: unknown, mac = isMac()): Record<string, string> {
  const defaults = defaultBindings(mac);
  const raw = stored && typeof stored === 'object' ? stored as Record<string, unknown> : {};
  const out: Record<string, string> = {};
  for (const s of SHORTCUTS) {
    const v = raw[s.id];
    if (typeof v !== 'string') { out[s.id] = defaults[s.id]!; continue; }
    if (v && (reservedReason(v, mac) || LEGACY_DEFAULTS[s.id] === v)) { out[s.id] = defaults[s.id]!; continue; }
    out[s.id] = v;
  }
  // Two ids on one combo (a new default met an old stored choice): the
  // user's stored choice keeps it, the default gives way.
  const owner = new Map<string, string>();
  for (const s of SHORTCUTS) {
    const v = out[s.id]!;
    if (!v) continue;
    const other = owner.get(v);
    if (!other) { owner.set(v, s.id); continue; }
    const chosen = raw[s.id] === v;
    if (chosen) { out[other] = ''; owner.set(v, s.id); } else out[s.id] = '';
  }
  return out;
}

/**
 * Combos the page never receives, or must not take: the browsers keep them
 * (Chrome, Safari and Firefox do not let a page prevent new tab / window /
 * close / quit, tab switching, or the developer tools), or the OS does
 * (macOS screenshots, hide, minimize, Spotlight, app switching). Conservative
 * on purpose: a refused combo costs a second try; an accepted one that never
 * fires is the "shortcuts don't work" report again. Returns the i18n key of
 * the reason, '' when the combo is usable.
 */
export function reservedReason(value: string, mac = isMac()): string {
  const parts = value.split('+');
  const code = parts.at(-1) ?? '';
  const mod = new Set(parts.slice(0, -1));
  const primary = mod.has(mac ? 'Meta' : 'Ctrl');
  const plain = !mod.has('Alt');
  if (mac && mod.has('Meta')) {
    if (mod.has('Shift') && ['Digit3', 'Digit4', 'Digit5'].includes(code)) return 'shortcutReservedSystem';
    if (plain && ['KeyH', 'KeyM', 'Space', 'Backquote'].includes(code)) return 'shortcutReservedSystem';
  }
  if (primary && plain && ['KeyN', 'KeyT', 'KeyW', 'KeyQ', 'Tab'].includes(code)) return 'shortcutReservedBrowser';
  if (mod.has('Ctrl') && code === 'Tab') return 'shortcutReservedBrowser';
  if (primary && plain && /^Digit[1-9]$/u.test(code)) return 'shortcutReservedBrowser'; // tab N
  if (!mac && mod.has('Alt') && !mod.has('Ctrl') && !mod.has('Meta') && /^Digit[1-9]$/u.test(code)) return 'shortcutReservedBrowser'; // Linux tab N
  // Developer tools and page source: ⌘⌥I/J/C/U, Ctrl+Shift+I/J/C.
  if (mac && mod.has('Meta') && mod.has('Alt') && ['KeyI', 'KeyJ', 'KeyC', 'KeyU'].includes(code)) return 'shortcutReservedBrowser';
  if (!mac && mod.has('Ctrl') && mod.has('Shift') && ['KeyI', 'KeyJ', 'KeyC'].includes(code)) return 'shortcutReservedBrowser';
  if (!mac && mod.has('Ctrl') && mod.has('Alt') && ['KeyT', 'Delete'].includes(code)) return 'shortcutReservedSystem';
  return '';
}

const MODIFIER_CODES = new Set([
  'AltLeft', 'AltRight', 'ControlLeft', 'ControlRight',
  'MetaLeft', 'MetaRight', 'ShiftLeft', 'ShiftRight',
]);

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

const KEY_LABELS: Record<string, string> = {
  BracketLeft: '[', BracketRight: ']', Backslash: '\\', Backquote: '`', Comma: ',', Period: '.',
  Slash: '/', Semicolon: ';', Quote: "'", Minus: '-', Equal: '=', Space: 'Space',
};

export function shortcutLabel(value: string | null | undefined, mac = isMac()): string {
  if (!value) return '—';
  const parts = value.split('+');
  const key = parts.pop()!;
  const name = KEY_LABELS[key] ?? key.replace(/^Key([A-Z])$/u, '$1').replace(/^Digit([0-9])$/u, '$1');
  const mods = parts.map((m) => (mac
    ? ({ Meta: '⌘', Ctrl: '⌃', Alt: '⌥', Shift: '⇧' } as Record<string, string>)[m] ?? m
    : `${({ Meta: 'Win', Ctrl: 'Ctrl', Alt: 'Alt', Shift: 'Shift' } as Record<string, string>)[m] ?? m}+`));
  return mods.join('') + name;
}

export function actionForShortcut(bindings: Record<string, string>, value: string): string {
  if (!value) return '';
  return SHORTCUTS.find((s) => bindings[s.id] === value)?.id || '';
}

export function cycleItem<T>(items: T[], current: T, direction: number): T | null {
  if (!items.length) return null;
  const index = items.indexOf(current);
  const start = index >= 0 ? index : 0;
  return items[(start + direction + items.length) % items.length]!; // modulo keeps the index in range
}
