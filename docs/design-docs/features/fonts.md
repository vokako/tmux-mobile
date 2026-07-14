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
`src/lib/fonts.svelte.js` `SYSTEM_STACK`):

```
'Noto Sans Symbols 2', 'Symbols Nerd Font Mono',
ui-monospace, 'SF Mono', Menlo, 'Cascadia Mono', Consolas,
'Roboto Mono', 'Droid Sans Mono', 'Noto Sans Mono', monospace
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

The symbol fonts sit FIRST in the stack (they only contain symbols, so
they can't hijack letters — a font earlier in the stack only wins for
codepoints it actually has) which guarantees the markers come from the
bundled files even when a system font has a lower-quality fallback glyph.
They don't drive cell metrics; a one-shot `document.fonts.ready` →
texture-atlas clear + `term.refresh()` in Terminal.svelte repaints any
tofu drawn before they decoded. The old re-measure/refit dance is gone by
construction (the measuring font is present at `open()` time).

xterm weights are back to `normal`/`bold` (real 400/700 faces exist in
every system family).

## Personal preference: the custom font setting

Settings → "Font name" (`fonts.svelte.js`, localStorage `tmux_font`).
The editable control suggests common families that are actually installed on
the current device and also accepts another family typed by the user. Before a
new value is applied, canvas text metrics verify that the browser can resolve
it without falling through to generic fonts. Invalid input is shown inline and
does not replace the active or persisted preference. A valid family name is
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

Settings → Terminal keeps font size and line spacing alongside the custom
font name. Font size remains the application-level `fontSize` state so the
stepper and desktop Cmd/Ctrl `+`, `-`, and `0` shortcuts use the same update
path and always trigger xterm's deferred re-fit.

Line spacing is a per-device preference stored as `tmux_line_height` and
clamped to `0.60`–`1.60`. xterm 6 normally rejects values below `1.00`, so the
Vite config applies a signature-guarded transform that changes only the
line-height lower bound (the tab-width bound remains unchanged).
`terminal-prefs.svelte.js` owns the reactive value;
every `Terminal.svelte` instance reads it when xterm is created and again in
the existing font-metrics effect. This makes the setting apply immediately to
single-pane, split, and embedded Team terminals without passing another prop
through each container.

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
