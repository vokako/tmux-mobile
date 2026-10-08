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
  'LXGW WenKai Mono',
  'Sarasa Mono SC',
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

// The suggestion pool, not the offer: `FontPref.available()` filters this to
// the families the DEVICE resolves, so the picker never offers a font that
// would fail validation (owner, 2026-09-21, board #233: "很多字体都用不了，
// 用不了前端应该提前过滤一下"). Adding a name here costs nothing on devices
// without it — which is why the pool may carry tasteful Han faces (LXGW 霞鹜,
// Sarasa 更纱, MiSans, HarmonyOS Sans) that only some machines have.
const COMMON_SANS = [
  'Inter',
  'Space Grotesk',
  'LXGW WenKai',
  'LXGW WenKai GB',
  'LXGW Neo XiHei',
  'MiSans',
  'HarmonyOS Sans SC',
  'Sarasa UI SC',
  'SF Pro Text',
  'Helvetica Neue',
  'Segoe UI',
  'Roboto',
  'Noto Sans',
  'IBM Plex Sans',
  'PingFang SC',
  'Microsoft YaHei',
  'Noto Sans SC',
  'Source Han Sans SC',
];

// Symbol fillers + per-platform fallbacks. The generic `monospace` keyword
// follows every mono family so an unknown/typo'd custom family degrades
// safely; only the Han-only SC tail comes after it (see below).
// The bundled symbol fonts must come AFTER the text families: the CSS line
// box (strut) — and xterm's fontBoundingBox cell measurement — derive from
// the FIRST available font in the stack, and 'Noto Sans Symbols 2' carries a
// huge 1.7em vertical box that inflates every terminal row and makes the
// block cursor protrude far below the text. Symbol codepoints missing from
// the text families still fall through to the bundled files (per-codepoint
// font matching), so only glyphs a text font actually has change source.
// The SC families close the stack, AFTER the generic `monospace` (board #97):
// no mono face carries Han, and a Han glyph that fell off the end of the
// list was chosen by the OS's language cascade — Japanese variants on a Mac
// that prefers Japanese. Named SC families make 骨/直/门 deterministic in
// the terminal and on every data surface. They come after the generic so
// latin can never land on them: on a platform with none of the named monos
// the generic still resolves latin to A monospace, and only the glyphs it
// lacks (Han) travel on. The line box is unaffected (the first available
// font is still the mono).
const SYSTEM_STACK =
  "ui-monospace, 'SF Mono', Menlo, 'Cascadia Mono', Consolas, " +
  "'Roboto Mono', 'Droid Sans Mono', 'Noto Sans Mono', " +
  "'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', monospace, " +
  "'PingFang SC', 'Hiragino Sans GB', 'Microsoft YaHei', 'Noto Sans CJK SC', 'Noto Sans SC', " +
  "'Source Han Sans SC', 'Source Han Sans CN', 'WenQuanYi Micro Hei'";

// These two literals MUST mirror app.css's --font-ui / --font-display
// declarations: the override rewrites the var inline, and an out-of-sync
// default would silently change the un-customized rendering.
const UI_STACK =
  "'Inter Variable', 'Inter', 'PingFang SC', 'Hiragino Sans GB', 'Microsoft YaHei', " +
  "'Noto Sans CJK SC', 'Noto Sans SC', 'Source Han Sans SC', 'Source Han Sans CN', 'WenQuanYi Micro Hei', " +
  "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif";

const DISPLAY_STACK =
  "'Space Grotesk Variable', 'Inter Variable', 'PingFang SC', 'Hiragino Sans GB', 'Microsoft YaHei', " +
  "'Noto Sans CJK SC', 'Noto Sans SC', 'Source Han Sans SC', 'Source Han Sans CN', 'WenQuanYi Micro Hei', " +
  "-apple-system, BlinkMacSystemFont, 'Segoe UI', sans-serif";

// The faces main.ts bundles (board 312, owner 2026-10-08: the project titles'
// face could not be picked back once replaced). They are always present, but
// a `local()` probe cannot see a webfont, so the pickers offer them and the
// validator accepts them without one. Offered in both prose roles.
const BUNDLED_SANS = ['Inter Variable', 'Space Grotesk Variable'];

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

/** Dispatched on `document` after a font role's CSS var is rewritten. */
export const FONT_CHANGE_EVENT = 'tmux:font';

export interface FontPref {
  /** The user's custom family name ('' = the role's default stack). */
  readonly custom: string;
  readonly common: string[];
  /** Bundled webfonts: always offered, valid without a probe. */
  readonly bundled: string[];
  /** The bundled face the default stack leads with — what Settings shows
   * when nothing is customized ('' = no nameable lead, see `fonts`). */
  readonly defaultFace: string;
  /** Full CSS font-family stack (custom family first when set). */
  readonly stack: string;
  set(name: string): Promise<boolean>;
  /** Rewrite the role's CSS var inline on <html> so every consumer follows. */
  apply(): void;
}

/** The suggestion pool filtered to families THIS device resolves — what the
 * pickers offer, so nothing offered can fail validation (board #233:
 * "很多字体都用不了，用不了前端应该提前过滤一下"). Uncached: the caller
 * (Settings) mounts rarely, and a probe is one registry lookup. */
export async function availableFamilies(pref: FontPref): Promise<string[]> {
  const hits = await Promise.all(pref.common.map(async (name) => (await isAvailable(name)) ? name : ''));
  const names = [...pref.bundled, ...hits.filter(Boolean)];
  // The custom family joined through `set`, which validated it — offer it
  // even when it is not in the pool.
  return pref.custom && !names.includes(pref.custom) ? [pref.custom, ...names] : names;
}

function makeFontPref(key: string, defaultStack: string, cssVar: string, common: string[], defaultFace = '', bundled: string[] = []): FontPref {
  let custom = $state(localStorage.getItem(key) || '');
  const pref: FontPref = {
    get custom() {
      return custom;
    },
    get common() {
      return common;
    },
    get bundled() {
      return bundled;
    },
    get defaultFace() {
      return defaultFace;
    },
    get stack() {
      const q = quote(custom);
      return q ? `${q}, ${defaultStack}` : defaultStack;
    },
    async set(name: string): Promise<boolean> {
      const next = normalizeFontFamily(name);
      if (!bundled.includes(next) && !await isAvailable(next)) return false;
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
      // Announce the swap (board #189): a system family arrives through no
      // browser event — `document.fonts` stays silent, nothing resizes — yet
      // every line re-wraps. Layout readers (the Feed's reading transaction)
      // listen for this the way they listen for `fonts.loadingdone`.
      document.dispatchEvent(new CustomEvent(FONT_CHANGE_EVENT, { detail: { role: cssVar } }));
    },
  };
  return pref;
}

/** Terminal + data surfaces. Key predates the split — existing prefs keep working. */
export const fonts = makeFontPref('tmux_font', SYSTEM_STACK, '--font-mono', COMMON_MONO);
// The terminal has no defaultFace: its stack leads with the `ui-monospace`
// keyword, which no probe can name (on a Mac it is SF Mono, which `local()`
// does not resolve, so a probe would answer Menlo while SF Mono draws).
/** Content prose. */
export const uiFont = makeFontPref('tmux_font_ui', UI_STACK, '--font-ui', COMMON_SANS, 'Inter Variable', BUNDLED_SANS);
/** Chrome: titles, buttons, names. */
export const displayFont = makeFontPref('tmux_font_display', DISPLAY_STACK, '--font-display', COMMON_SANS, 'Space Grotesk Variable', BUNDLED_SANS);

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
