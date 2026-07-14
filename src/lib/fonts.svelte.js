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

export const fonts = {
  /** The user's custom family name ('' = system default). */
  get custom() {
    return custom;
  },
  set(name) {
    custom = (name || '').trim();
    try {
      if (custom) localStorage.setItem(KEY, custom);
      else localStorage.removeItem(KEY);
    } catch {}
    applyMonoVar();
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
