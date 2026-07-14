// Terminal monospace font: system stack by default, user-overridable.
//
// Default = each platform's native mono (SF Mono / Cascadia / Roboto Mono …)
// + two bundled symbol fonts that only fill glyphs no system font has
// (agent markers, Nerd PUA icons). See index.html + design doc for why we
// stopped bundling text fonts.
//
// The override ('tmux_font' in localStorage) lets a user put a font THEY
// have installed (e.g. 'Maple Mono NF CN') at the front of the stack. It's
// per-device by design: the same account on a phone without that font just
// falls through to the system stack — same layout either way, because
// xterm's cell grid fixes alignment regardless of family.

const KEY = 'tmux_font';

const COMMON_FAMILIES = [
  'Maple Mono NF CN',
  'Maple Mono',
  'SF Mono',
  'Menlo',
  'Monaco',
  'Cascadia Mono',
  'JetBrains Mono',
  'Fira Code',
  'Hack',
  'IBM Plex Mono',
  'Roboto Mono',
  'Noto Sans Mono',
  'Source Code Pro',
  'Ubuntu Mono',
  'Consolas',
];

// Symbol fillers + per-platform fallbacks. The generic `monospace` keyword
// stays last so an unknown/typo'd custom family degrades safely.
const SYSTEM_STACK =
  "'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', " +
  "ui-monospace, 'SF Mono', Menlo, 'Cascadia Mono', Consolas, " +
  "'Roboto Mono', 'Droid Sans Mono', 'Noto Sans Mono', monospace";

let custom = $state(localStorage.getItem(KEY) || '');

function quote(name) {
  // Wrap in single quotes for CSS; strip any quotes the user typed.
  const clean = name.trim().replace(/['"]/g, '');
  return clean ? `'${clean}'` : '';
}

function isAvailable(name) {
  const family = (name || '').trim().replace(/['"]/g, '');
  if (!family) return true;

  const canvas = document.createElement('canvas');
  const context = canvas.getContext('2d');
  if (!context) return false;

  const sample = 'mmmmmmmmmmlliWW00@#中文';
  for (const fallback of ['monospace', 'serif', 'sans-serif']) {
    context.font = `72px ${fallback}`;
    const fallbackWidth = context.measureText(sample).width;
    context.font = `72px ${quote(family)}, ${fallback}`;
    if (context.measureText(sample).width !== fallbackWidth) return true;
  }
  return false;
}

export const fonts = {
  /** The user's custom family name ('' = system default). */
  get custom() {
    return custom;
  },
  get common() {
    return COMMON_FAMILIES.filter(isAvailable);
  },
  set(name) {
    const next = (name || '').trim().replace(/['"]/g, '');
    if (!isAvailable(next)) return false;
    custom = next;
    try {
      if (custom) localStorage.setItem(KEY, custom);
      else localStorage.removeItem(KEY);
    } catch {}
    applyMonoVar();
    return true;
  },
  /** Full CSS font-family stack (custom family first when set). */
  get stack() {
    const q = quote(custom);
    return q ? `${q}, ${SYSTEM_STACK}` : SYSTEM_STACK;
  },
};

// --font-mono lives on <html> (App.svelte declares the default); the
// override just rewrites the inline style so every var() consumer follows.
export function applyMonoVar() {
  document.documentElement.style.setProperty('--font-mono', fonts.stack);
}
