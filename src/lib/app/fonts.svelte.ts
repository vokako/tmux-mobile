import { localFontSource, normalizeFontFamily } from './font-validation.ts';

// THREE font roles, each user-overridable, each per-device (owner,
// 2026-08-25: "总之就三类，正文内容字体，标题按钮等非文本框内的字体，还有
// 终端字体，这些可以都是系统设置里的字体"):
//
// - mono (--font-mono, 'tmux_font'): the terminal and every data surface.
//   Default = each platform's native mono (SF Mono / Cascadia / Roboto Mono…)
//   + two bundled symbol fonts that only fill glyphs no system font has
//   (agent markers, Nerd PUA icons). See index.html + fonts.md for why we
//   stopped bundling text fonts.
// - ui (--font-ui, 'tmux_font_ui'): content prose — message bodies, input
//   text, rendered documents. Default leads with the bundled Inter Variable.
// - display (--font-display, 'tmux_font_display'): the chrome — titles,
//   section headers, buttons, names. Default leads with the bundled
//   Space Grotesk Variable.
//
// An override puts a font the USER has installed at the front of that
// role's stack. Per-device by design: the same account on a phone without
// the font falls through to the default stack — same layout either way
// (the terminal's alignment comes from xterm's cell grid, and the UI's
// from the box model, not the family).

const COMMON_MONO = [
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

const COMMON_SANS = [
  'Inter',
  'Space Grotesk',
  'SF Pro Text',
  'Helvetica Neue',
  'Segoe UI',
  'Roboto',
  'Noto Sans',
  'IBM Plex Sans',
  'PingFang SC',
  'Microsoft YaHei',
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

// These two literals MUST mirror app.css's --font-ui / --font-display
// declarations: the override rewrites the var inline, and an out-of-sync
// default would silently change the un-customized rendering.
const UI_STACK =
  "'Inter Variable', -apple-system, BlinkMacSystemFont, 'Inter', 'Segoe UI', " +
  "'PingFang SC', 'Microsoft YaHei', 'Noto Sans CJK SC', 'Noto Sans SC', sans-serif";

const DISPLAY_STACK =
  "'Space Grotesk Variable', 'Inter Variable', -apple-system, BlinkMacSystemFont, 'Segoe UI', " +
  "'PingFang SC', 'Microsoft YaHei', 'Noto Sans CJK SC', 'Noto Sans SC', sans-serif";

function quote(name: string): string {
  // Wrap in single quotes for CSS; strip any quotes the user typed.
  const clean = name.trim().replace(/['"]/g, '');
  return clean ? `'${clean}'` : '';
}

async function isAvailable(name: string): Promise<boolean> {
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

export interface FontPref {
  /** The user's custom family name ('' = the role's default stack). */
  readonly custom: string;
  readonly common: string[];
  /** Full CSS font-family stack (custom family first when set). */
  readonly stack: string;
  set(name: string): Promise<boolean>;
  /** Rewrite the role's CSS var inline on <html> so every consumer follows. */
  apply(): void;
}

function makeFontPref(key: string, defaultStack: string, cssVar: string, common: string[]): FontPref {
  let custom = $state(localStorage.getItem(key) || '');
  const pref: FontPref = {
    get custom() {
      return custom;
    },
    get common() {
      return common;
    },
    get stack() {
      const q = quote(custom);
      return q ? `${q}, ${defaultStack}` : defaultStack;
    },
    async set(name: string): Promise<boolean> {
      const next = normalizeFontFamily(name);
      if (!await isAvailable(next)) return false;
      custom = next;
      try {
        if (custom) localStorage.setItem(key, custom);
        else localStorage.removeItem(key);
      } catch {}
      pref.apply();
      return true;
    },
    apply() {
      document.documentElement.style.setProperty(cssVar, pref.stack);
    },
  };
  return pref;
}

/** Terminal + data surfaces. Key predates the split — existing prefs keep working. */
export const fonts = makeFontPref('tmux_font', SYSTEM_STACK, '--font-mono', COMMON_MONO);
/** Content prose. */
export const uiFont = makeFontPref('tmux_font_ui', UI_STACK, '--font-ui', COMMON_SANS);
/** Chrome: titles, buttons, names. */
export const displayFont = makeFontPref('tmux_font_display', DISPLAY_STACK, '--font-display', COMMON_SANS);

// --font-* live on <html> (app.css declares the defaults); the overrides just
// rewrite the inline style so every var() consumer follows.
export function applyMonoVar() {
  fonts.apply();
}

/** Apply every customized role at startup (a no-op writes the default stack,
 * which is identical to the stylesheet's). */
export function applyFontVars() {
  if (fonts.custom) fonts.apply();
  if (uiFont.custom) uiFont.apply();
  if (displayFont.custom) displayFont.apply();
}
