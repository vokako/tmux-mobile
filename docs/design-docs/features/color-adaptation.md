# Terminal Color Adaptation

## Context
AI CLIs (Kiro, Claude Code, etc.) ship ANSI truecolor / 256-color codes tuned for a *typical* dark terminal around bg ≈ `#1e1e1e`. tmux-mobile's themes are harsher at both ends:

- Dark: `#0a0a0f` (near-black)
- Light: `#f5f5f7` (near-white)

Unchanged, a Kiro "Tasks" panel with BG `rgb(238,238,238)` reads as a **blinding near-white block** on our dark background, and in light mode the previous behavior (flat `255 - c` RGB inversion) produced **pure-black blocks and wrong hues**.

## Decision
Adapt colors at write-time, in JS, before handing to xterm.js. Instead of a blanket inversion, **preserve hue + saturation** and only move a color's luminance when:

- It's a **FG** that lacks contrast against the terminal bg (WCAG contrast < 3.5) — push it into the readable band for this theme.
- It's a **BG block** that either clashes with the terminal bg (too bright in dark mode, too dark in light mode) or blends into it (too close to the terminal bg in light mode) — push it to a mid-luminance band that reads as a "block" without dominating.

Decisions use **WCAG relative luminance** (perceptually correct); construction uses HSL L (cheap to edit hue/saturation separately). Thresholds are tuned against a real Kiro capture (see `temp/color_test.js`).

## How It Works
1. Regex-replace every `\x1b[38;2;r;g;bm` / `\x1b[48;2;r;g;bm` truecolor sequence.
2. Regex-replace every `\x1b[38;5;nm` / `\x1b[48;5;nm` 256-color indexed sequence for indices 16-255 (0-15 are left to xterm.js's theme).
3. For each RGB, compute WCAG L. Apply the FG or BG decision above.
4. When a change is needed, convert to HSL (preserving h, s), set a target L, convert back to RGB.
5. Output rewrites indexed colors as truecolor for consistency.

Cache by (text, theme) so a stable pane doesn't re-transform on every frame.

### Key constants
| Name | Value | Role |
|------|-------|------|
| `MIN_FG_CONTRAST` | 3.5 | FG ≥ this vs terminal bg → leave alone |
| `BG_CLASH_RATIO_DARK` | 4.5 | Dark BG > 4.5× brighter than term bg → recolor |
| `BG_CLASH_RATIO_LIGHT` | 1.8 | Light BG > 1.8× darker than term bg → recolor |
| `BG_BLEND_RATIO_LIGHT` | 1.15 | Light BG within 1.15× of term bg → recolor (invisible) |
| `HSL_L_{BG,FG}_{DARK,LIGHT}` | 0.28-0.75 | Target HSL L by role × theme |

## Alternatives Considered
- **Flat RGB inversion** (old behavior). Rejected: destroys hue, produces near-black UI blocks in light mode, and leaves Kiro's bright UI blocks blinding in dark mode.
- **xterm.js `minimumContrastRatio`**. Rejected for now: xterm adjusts FG *for every cell at render time* which is robust but opaque — hard to tune, and it can't help with glaring BG blocks which are the loudest complaint.
- **Do-nothing**. Rejected: that's what produced the user-reported bug.
- **SGR state machine that re-balances FG+BG as a pair**. Deferred. Solves the remaining limitation (see below) but is a much bigger change; tracked in `docs/unresolved.md`.

## Trade-offs
- **Independent FG/BG adjustment** means hand-picked pairs can lose contrast in edge cases (e.g. purple bg + yellow fg in light mode). Typical AI CLI output uses FG-only colors on the default terminal bg, so this is rare.
- Target luminance values are global constants; colors from different CLIs map to the same targets. Saturated colors (pure green, pure red) land at slightly different WCAG L than greys because HSL L → WCAG L is non-linear; the effect is that saturated colors end up *slightly* brighter than their grey peers, which actually helps semantic colors stand out.
- Always running adaptation (vs the old "light-only" path) adds a regex pass per terminal frame. Benchmarks on a ~10 KB pane were sub-millisecond and there is a content-based cache for repeat frames, so the cost is effectively zero for static panes.

## Lessons Learned
- The original symptom was reported as "dark mode renders as white background, light mode as black background" — it was literally the symptom of a blind RGB inversion; the fix required separating the *role* (FG vs BG) from the *color space* (perceptual vs nominal).
- HSL L and WCAG L are **not** the same. Early drafts used HSL L for decisions; saturated dark colors (e.g. `rgb(200,0,0)`) slip past thresholds unexpectedly. Always decide in WCAG L; only use HSL to *construct* the target color.
- Keep a runnable offline harness (`temp/color_test.js`) with representative inputs (Kiro Tasks block, error boxes, rainbow hues, pure black/white, 256-color spot checks, FG+BG pair samples). Iterating against the harness caught two rounds of wrong thresholds before any on-device test.
