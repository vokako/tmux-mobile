import { localFontSource, normalizeFontFamily } from './font-validation.js';

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
// The bundled symbol fonts must come AFTER the text families: the CSS line
// box (strut) — and xterm's fontBoundingBox cell measurement — derive from
// the FIRST available font in the stack, and 'Noto Sans Symbols 2' carries a
// huge 1.7em vertical box that inflates every terminal row and makes the
// block cursor protrude far below the text. Symbol codepoints missing from
// the text families still fall through to the bundled files (per-codepoint
// font matching), so only glyphs a text font actually has change source.
const SYSTEM_STACK =
  "ui-monospace, 'SF Mono', Menlo, 'Cascadia Mono', Consolas, " +
  "'Roboto Mono', 'Droid Sans Mono', 'Noto Sans Mono', " +
  "'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', monospace";

let custom = $state(localStorage.getItem(KEY) || '');

function quote(name) {
  // Wrap in single quotes for CSS; strip any quotes the user typed.
  const clean = name.trim().replace(/['"]/g, '');
  return clean ? `'${clean}'` : '';
}

async function isAvailable(name) {
  const family = normalizeFontFamily(name);
  if (!family) return true;

  // Width comparison gives false negatives for monospace families: an installed
  // font can have the exact same advances as the fallback. Ask the browser to
  // load the local face by name instead; this checks the font registry itself.
  if (typeof FontFace === 'function') {
    try {
      await new FontFace('__tmux_font_probe__', localFontSource(family)).load();
      return true;
    } catch {}
  }

  // Compatibility fallback for WebViews without FontFace. Multiple proportional
  // fallbacks make an equal-width coincidence less likely, but this is no longer
  // the primary validation path on macOS or modern Android.
  const canvas = document.createElement('canvas');
  const context = canvas.getContext('2d');
  if (!context) return false;
  const sample = 'mmmmmmmmmmlliWW00@#中文';
  return ['serif', 'sans-serif'].some(fallback => {
    context.font = `72px ${fallback}`;
    const fallbackWidth = context.measureText(sample).width;
    context.font = `72px ${quote(family)}, ${fallback}`;
    return context.measureText(sample).width !== fallbackWidth;
  });
}

export const fonts = {
  /** The user's custom family name ('' = system default). */
  get custom() {
    return custom;
  },
  get common() {
    return COMMON_FAMILIES;
  },
  async set(name) {
    const next = normalizeFontFamily(name);
    if (!await isAvailable(next)) return false;
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
