#!/usr/bin/env bash
# Fetch / generate the bundled web fonts into public/fonts/.
#
# These files are deliberately NOT committed (see .gitignore) — the CJK
# subset alone is 5 MB and would bloat the repo permanently. Run this once
# after cloning, before `npm run build` / `npm run build:android`.
#
#   - Latin / Noto Symbols / Nerd Symbols: downloaded from public CDNs.
#   - Maple Mono CJK subset: GENERATED from a locally-installed
#     `Maple Mono NF CN` (or `Maple Mono CN`) TTF, because the project
#     publishes no Chinese woff2 (only a 152 MB full TTF pack). Requires
#     fonttools with brotli (`pip install fonttools brotli`).
#
# Idempotent: re-running overwrites the outputs.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
OUT="$ROOT/public/fonts"
mkdir -p "$OUT"

dl() { # url, dest
  echo "↓ $(basename "$2")"
  curl -fsSL -o "$2" "$1"
}

echo "== Latin + symbol fonts (CDN) =="
dl "https://cdn.jsdelivr.net/fontsource/fonts/maple-mono@latest/latin-300-normal.woff2" "$OUT/maple-mono-latin-300.woff2"
dl "https://cdn.jsdelivr.net/fontsource/fonts/maple-mono@latest/latin-600-normal.woff2" "$OUT/maple-mono-latin-600.woff2"
dl "https://cdn.jsdelivr.net/npm/@fontsource/noto-sans-symbols-2@latest/files/noto-sans-symbols-2-symbols-400-normal.woff2" "$OUT/noto-symbols2-400.woff2"

echo "== Nerd Symbols (download TTF → woff2) =="
NERD_TTF="$(mktemp -t nerd).ttf"
dl "https://cdn.jsdelivr.net/gh/ryanoasis/nerd-fonts@v3.4.0/patched-fonts/NerdFontsSymbolsOnly/SymbolsNerdFontMono-Regular.ttf" "$NERD_TTF"
fonttools ttLib.woff2 compress -o "$OUT/symbols-nerd-mono.woff2" "$NERD_TTF"
rm -f "$NERD_TTF"

echo "== Maple Mono CJK subset (from local install) =="
# Locate a locally-installed Maple Mono CN TTF (NF preferred, then plain CN).
CN_SRC=""
for name in MapleMono-NF-CN-Regular MapleMono-CN-Regular MapleMono-NF-CN-Medium; do
  for dir in "$HOME/Library/Fonts" "$HOME/.local/share/fonts" "$HOME/.fonts" "/Library/Fonts" "/usr/share/fonts"; do
    cand="$dir/$name.ttf"
    if [ -f "$cand" ]; then CN_SRC="$cand"; break 2; fi
  done
done
if [ -z "$CN_SRC" ]; then
  echo "⚠️  No 'Maple Mono CN' TTF found locally — skipping CJK subset."
  echo "    Install it (https://github.com/subframe7536/maple-font releases,"
  echo "    the *-CN or *-NF-CN pack) then re-run, or Chinese will fall back"
  echo "    to the system font."
  exit 0
fi
echo "  source: $CN_SRC"
python3 - "$CN_SRC" "$OUT/maple-mono-cjk.woff2" <<'PY'
import sys
from fontTools.subset import Subsetter, Options, load_font, save_font
src, out = sys.argv[1], sys.argv[2]
# CJK-only (Latin handled by the latin files): CJK punctuation, fullwidth
# forms, CJK basic block, a few common symbols + CJK ext-A head.
ranges = []
for a, b in [(0x3000,0x303F),(0xFF00,0xFFEF),(0x4E00,0x9FFF),
             (0x2018,0x201F),(0x2026,0x2026),(0x00B7,0x00B7),(0x3400,0x34FF)]:
    ranges += list(range(a, b+1))
opt = Options(); opt.flavor = "woff2"; opt.desubroutinize = True
opt.drop_tables += ['meta']
font = load_font(src, opt)
ss = Subsetter(options=opt); ss.populate(unicodes=ranges); ss.subset(font)
save_font(font, out, opt)
import os
print(f"  → {out}  ({os.path.getsize(out)/1048576:.2f} MB)")
PY

echo "✅ fonts ready in public/fonts/"
