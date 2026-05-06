import '@xterm/xterm/css/xterm.css';
import { mount } from 'svelte';
import App from './App.svelte';

// ─── Web fonts loaded at runtime ─────────────────────────────────────────
// We avoid bundling fonts into the APK (they'd add MBs to every install)
// and instead pull them from a CDN with `display: swap` so the UI never
// blocks on font load. macOS users with the local Maple Mono NF CN
// installed match it first via the fontFamily stack and never download
// any of these.

// Maple Mono — Latin subset is enough for our UI text. CJK glyphs fall
// through to the system fallback (which is fine; we don't render Chinese
// in the terminal proper).
const fontWeights = [300, 600];
fontWeights.forEach(w => {
  const face = new FontFace(
    'Maple Mono',
    `url(https://cdn.jsdelivr.net/fontsource/fonts/maple-mono@latest/latin-${w}-normal.woff2) format('woff2')`,
    { weight: String(w), style: 'normal', display: 'swap' }
  );
  face.load().then(f => document.fonts.add(f)).catch(() => {});
});

// Two complementary fallback fonts cover the full TUI symbol range:
//
// 1. Noto Sans Symbols 2 — standard Unicode blocks: Miscellaneous
//    Technical (U+2300–U+23FF), Dingbats (U+2700–U+27BF), Geometric
//    Shapes Extended (U+1F780–U+1F7FF). Used by Claude Code / Kiro /
//    gemini for ⏵ ⏺ ✳ ✷ ✸ ⭘ ❯ … (382 KB woff2)
//
// 2. Symbols Nerd Font Mono — Private Use Area icons: Powerline
//    separators, Octicons, Devicons, Material, Pomicons, Weather, etc.
//    Used by starship / oh-my-zsh themes /  󰀄  ⮕ … (1.5 MB ttf)
//
// They cover disjoint codepoint ranges, so loading both and listing
// them after Maple Mono in fontFamily lets the browser route each
// glyph to whichever font has it. `display: swap` keeps both async.
//
// macOS users with `Maple Mono NF CN` installed locally already cover
// both ranges from that one font, so the fontFamily stack matches it
// first and these CDN fonts are never downloaded.
{
  const face = new FontFace(
    'Noto Sans Symbols 2',
    "url(https://cdn.jsdelivr.net/npm/@fontsource/noto-sans-symbols-2@latest/files/noto-sans-symbols-2-symbols-400-normal.woff2) format('woff2')",
    { display: 'swap' }
  );
  face.load()
    .then(f => document.fonts.add(f))
    .catch(e => console.warn('Noto Symbols font load failed:', e));
}
{
  const face = new FontFace(
    'Symbols Nerd Font Mono',
    "url(https://cdn.jsdelivr.net/gh/ryanoasis/nerd-fonts@v3.4.0/patched-fonts/NerdFontsSymbolsOnly/SymbolsNerdFontMono-Regular.ttf) format('truetype')",
    { display: 'swap' }
  );
  face.load()
    .then(f => document.fonts.add(f))
    .catch(e => console.warn('Nerd Symbols font load failed:', e));
}

const app = mount(App, { target: document.getElementById('app') });

export default app;
