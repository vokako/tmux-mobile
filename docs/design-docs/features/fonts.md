# Fonts — system stack + bundled symbols + per-device custom override

## Context

The terminal (xterm.js) and all mono UI (`--font-mono`) need a monospace
font on every platform: macOS desktop, Android WebView, plus any browser
via the web UI. Three constraints pull against each other:

1. **Small payload** — the app ships in an APK and over the network; the
   user asked for "as little font bundling as possible".
2. **Correct rendering everywhere** — agent status markers (⏺ ✳ ✻),
   braille spinners, box drawing, CJK, Nerd Font prompt icons.
3. **Personal preference** — the operator likes Maple Mono on their own
   devices.

## History: why we un-bundled the text fonts (2026-07)

An earlier iteration bundled Maple Mono latin (2 cuts) + a 5 MB CJK subset
+ 2 symbol fonts (6.6 MB total), stack-fronted by `'Maple Mono NF CN'`.
Three defects, found by inspecting the actual woff2 payloads:

- **The latin file was the Light (300) cut** (`usWeightClass` 300 inside;
  the "600" file was actually Medium/500). Normal-weight text matched the
  300 face → thin rendering on every device *except* machines with the
  full font installed system-wide — i.e. it looked right on the dev
  machine and wrong for users. xterm was even configured with
  `fontWeight: 300 / fontWeightBold: 600` to match the bundle.
- **Dev machine ≠ user device**: the stack-front family only resolved on
  the developer's Mac. What users actually got was a four-font patchwork
  (thin latin → Noto Symbols → Nerd → CJK), each with different metrics.
- **Async webfont swap broke xterm metrics**: xterm measures cell width at
  `open()`; when the woff2 swapped in later the cached width was stale
  ("characters stuck together" on some MIUI WebViews). A
  `document.fonts.ready` re-measure + atlas-clear patch papered over it.

## Decision

**Text fonts come from the system; only symbols are bundled.**

xterm renders a fixed cell grid — column alignment comes from the grid,
not the font, and CJK is force-rendered at 2 cells. So platform-native
mono + CJK (SF Mono / Cascadia / Roboto Mono; PingFang / YaHei / Noto CJK
/ MiSans) is metric-safe and higher quality than any downloadable subset.

The default stack (App.svelte `--font-mono`, mirrored in
`src/lib/app/fonts.svelte.js` `SYSTEM_STACK`):

```
ui-monospace, 'SF Mono', Menlo, 'Cascadia Mono', Consolas,
'Roboto Mono', 'Droid Sans Mono', 'Noto Sans Mono',
'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', monospace
```

Bundled (public/fonts/, @font-face in index.html):

- `noto-symbols2-subset.woff2` (~70 KB) — subset of Noto Sans Symbols 2 to
  the terminal-relevant blocks `U+2190-21FF, 2300-23FF, 25A0-25FF,
  2600-27BF, 2800-28FF, 2B00-2BFF` (arrows, technical, geometric shapes,
  dingbats incl. the agent markers ⏺✳✻, braille spinners, misc symbols).
  Regenerate with:
  `python -m fontTools.subset noto-symbols2-400.woff2 --unicodes="U+2190-21FF,U+2300-23FF,U+25A0-25FF,U+2600-27BF,U+2800-28FF,U+2B00-2BFF" --flavor=woff2 --output-file=noto-symbols2-subset.woff2`
- `symbols-nerd-mono.woff2` (1.1 MB) — Nerd Font PUA icons (starship
  prompts etc.). No system font anywhere ships these. Could be subset to
  the common PUA ranges (~300 KB) if payload matters more than long-tail
  icon coverage; not done yet.

**Trap: the first available font in the stack defines the line box.** The
symbol fonts originally sat FIRST in the stack, on the theory that a font
earlier in the stack only wins for codepoints it actually has (true for glyph
selection). But the CSS strut — and xterm 6's cell measurement via canvas
`fontBoundingBoxAscent/Descent` — come from the FIRST available font
regardless of which font renders the text. `Noto Sans Symbols 2` carries a
1.7 em vertical box (ascent 1.07, descent 0.63 per em vs Menlo's 1.17 em
total), so once its woff2 decoded, every terminal cell inflated ~45%, text
sat high in the cell, and the full-cell block cursor protruded far below the
glyphs ("cursor not vertically centered, sticks out at the bottom"; line
spacing looked loose even at 1.00). The symbol fonts therefore sit AFTER the
text families (still before generic `monospace`): codepoints missing from the
text font — the agent markers ⏺, braille spinners, Nerd PUA icons — still
resolve to the bundled files via per-codepoint fallback; the only change is
that symbols a text font DOES contain (✳ ✻ ● ▲ on macOS Menlo) now render
from that font, metric-matched to the text. `@font-face` metric overrides
(`ascent-override`) were rejected as the fix: WKWebView (Tauri macOS) doesn't
support them. When a custom font is set it is prepended, so the strut follows
the user's text font either way.

The symbol fonts don't drive cell metrics; a one-shot `document.fonts.ready` →
texture-atlas clear + `term.refresh()` in Terminal.svelte repaints any
tofu drawn before they decoded. The old re-measure/refit dance is gone by
construction (the measuring font is present at `open()` time).

xterm weights are back to `normal`/`bold` (real 400/700 faces exist in
every system family).

## Personal preference: the custom font setting

Settings → "Font name" (`fonts.svelte.js`, localStorage `tmux_font`).
The editable control suggests common families and also accepts another family
typed by the user. Before a new value is applied, a temporary `FontFace` with
`local("<family>")` asks the platform font registry to resolve it. Canvas width
comparison remains only as a compatibility fallback: it falsely rejects an
installed monospace family such as Maple Mono NF CN when its advances equal the
generic fallback. Invalid input is shown inline and does not replace the active
or persisted preference. A valid family name is
prepended (quoted) to the stack and applied two ways:

- rewrites the `--font-mono` CSS var inline on `<html>` (all mono UI), and
- flows into `term.options.fontFamily` via the fontSize/`fonts.stack`
  $effect in Terminal.svelte, which re-measures and refits.

Semantics are deliberately **per-device**: the name resolves against fonts
installed on that device (e.g. the operator's Macs/pads with Maple Mono NF
CN installed). On a device without the font, CSS font matching falls
through to the system stack — a typo or missing font degrades safely, no
tofu, no layout break (the cell grid absorbs the metric change). This is
the "I want Maple on MY devices" requirement without shipping Maple to
everyone: install the font on the device (macOS: double-click; Android
can't install system fonts — an Android device that must have Maple would
need it re-bundled, which we explicitly traded away).

## Terminal size and line spacing

Settings → Appearance keeps terminal font size and line spacing alongside the
custom font name. Terminal font size remains the application-level `fontSize`
state and the stepper triggers xterm's deferred re-fit. Desktop Cmd/Ctrl `+`,
`-`, and `0` do not change this value; they control the independent native
WebView interface scale instead.

Line spacing is a per-device preference stored as `tmux_line_height` and
clamped to `0.40`–`1.60` (`LINE_HEIGHT_MIN`/`MAX` in
`terminal-prefs.svelte.js`). xterm 6 normally rejects values below `1.00`, so
the Vite config applies a signature-guarded transform that changes only the
line-height lower bound (the tab-width bound remains unchanged); the patched
bound must stay in sync with `LINE_HEIGHT_MIN`, or sub-minimum values throw in
xterm's option setter and the slider silently does nothing below `1.00`.

**Trap: Vite dep pre-bundling bypasses plugin transforms.** In dev, optimized
deps are served from `node_modules/.vite/deps/` as esbuild output that never
went through the plugin, so the patch only took effect in `npm run build` while
`npm run dev` / `npm run tauri:dev` kept the stock `lineHeight >= 1` check.
`@xterm/xterm` is therefore excluded via `optimizeDeps.exclude` (safe: the
addons only type-import the core). Excluded deps get a `?v=<hash>` query on
their module id in dev, so the transform strips the query before matching.

`terminal-prefs.svelte.js` owns the reactive value;
every `Terminal.svelte` instance reads it when xterm is created and again in
the existing font-metrics effect. This makes the setting apply immediately to
single-pane, split, and embedded Team terminals without passing another prop
through each container.

xterm's DOM renderer normally assumes `lineHeight >= 1`: it shortens each row
for compact values but leaves the glyph top-aligned at the original font
height, so clipping is visually one-sided. The Vite transform adds a dedicated
`xterm-glyph` child inside each xterm cell (signature-guarded for xterm 6), and
`Terminal.svelte` centers that original-height glyph box over the compact cell.
The row's existing overflow clipping then removes exactly half of the excess
above and below. Cell backgrounds, block/bar/underline cursors, selection
overlays, decorations, and touch coordinates stay on the unmoved outer cell;
foreground color, weight, italics, letter spacing, and text decorations inherit
into the glyph child. The visible IME composition text uses the same inner
glyph layer, leaving its containing box and IME candidate-window anchor
unchanged. `npm test` runs the full frontend suite (including the
line-geometry test that verifies the split), while `npm run
build` verifies both xterm renderer signatures before shipping.

## Alternatives considered

- **Keep bundling, fix the weight bug** (bundle Regular/Bold cuts):
  fixes thin text but keeps 6.6 MB, the four-font patchwork, and the
  async-swap re-measure machinery. Rejected — the user's explicit goal
  was to stop bundling.
- **Serve the custom font from the tmux server** (`fs_download` the TTF at
  connect, register via `FontFace`): would give Maple on Android without
  bundling it in the APK. Deferred — adds a moving part (font file path
  config, 12 MB TTF over weak links, FontFace lifecycle across
  reconnects) for a single-device gap; revisit if the operator actually
  wants Maple on Android.
- **`local()` sources in @font-face**: `src: local('Maple Mono NF CN')`
  before the system stack would auto-use a locally-installed Maple with no
  setting. Rejected: invisible magic (font changes depending on what's
  installed, with no UI to see/override why), and fingerprinting-protection
  in some browsers ignores `local()` entirely.

## Consequences

- Payload: 6.6 MB → ~1.2 MB of fonts.
- Mono text looks *native per platform*, not identical across platforms.
  That's accepted: "correct and readable everywhere" won over "identical
  everywhere".
- The thin-text bug and the stuck-together-glyphs bug class are both gone
  by construction.
- `--font-mono` default in App.svelte and `SYSTEM_STACK` in
  fonts.svelte.js must stay in sync (single-source refactor not worth the
  indirection for one literal; both sites carry a pointer comment).
