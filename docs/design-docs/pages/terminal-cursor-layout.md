# Terminal Cursor Layout (visual rows vs `capture-pane -J`)

## Context

The server captures pane content with `capture-pane -e -J` and sends it with
tmux's cursor info (`cursor_x`, `cursor_y`, `pane_height`, plus `t` = trailing
blank rows trimmed). The client writes the content into xterm.js and then
positions the cursor with a CUP escape (`ESC[row;colH`).

These two inputs live in **different units**:

- `-J` joins soft-wrapped lines, so the content is **logical** lines. xterm
  re-wraps them at `term.cols` when written (that was the xterm-migration
  decision: same width ⇒ same wrap, see
  `docs/exec-plans/2026-04-02-xterm-migration.md`).
- `cursor_y`, `pane_height`, and `t` are **visual** (screen) rows.

## The bug this doc exists for

`computeCursorLayout` originally counted newlines (`countLines`) — logical
lines — and mixed them directly with `cursor_y`/`pane_height`. Every soft
wrap visible in the pane therefore shifted the computed cursor row one row
down AND added one phantom blank row of bottom padding (which pushed the
pane's top rows into scrollback). Classic symptom: in vi with one long
wrapped line, the cursor renders one row below the text it is on; two wraps,
two rows. Reproduced and verified with a live 40×10 tmux pane.

## Fix: run all layout math in visual-row space

`computeCursorLayout(content, cursor, rows, cols)` measures how many visual
rows the content occupies after xterm re-wraps it at `cols`
(`countVisualRows`), then applies the same bottom-anchoring math as before:

```
paneStart = max(0, visual + t - h)   // pane's first visual row in the buffer
total     = paneStart + h            // buffer must end at the pane's bottom row
pad       = total - visual           // restores trimmed blanks + cursor rows
sb        = max(0, total - rows)     // rows scrolled above the viewport
row       = paneStart + y - sb + 1   // 1-based CUP row
```

For unwrapped content `visual == countLines`, so behavior is provably
identical to the old implementation — no regression risk for the plain case.

### The cancellation invariant (why width-model precision barely matters)

When the pane is full (`visual + t >= h`, i.e. vi, any TUI, any busy shell),
the formula reduces to `pad = t` and `row = y + 1 + (rows - h)` — the
measurement cancels out entirely. Width-model errors can only surface when
the content is shorter than the pane (a fresh prompt), where lines are short
and rarely wrapped. This is what makes a client-side measurement safe.

### Width model: mirror xterm, not tmux

`cellWidth` deliberately mirrors **xterm's default UnicodeV6 provider**
(`@xterm/xterm/src/common/input/UnicodeV6.ts`) — we are predicting the
re-wrap xterm performs, not the wrap tmux performed:

- CJK/fullwidth ranges → 2; combining marks (Mn/Me only — Mc occupies a
  cell in V6) and zero-width format chars → 0; **emoji → 1** (V6 predates
  emoji; no unicode addon is loaded).
- Wrap rule: a glyph that no longer fits starts a new row; a wide glyph at
  the last column wraps whole (matches both xterm and the server's
  `join_unflagged_wraps` handling of tmux's unflagged CJK wraps).
- ANSI sequences (SGR from `-e`, OSC) are skipped.

The server's `is_wide_char` (tmux.rs) is tmux-oriented and counts emoji as
2 — do NOT "unify" the two tables; they answer different questions. The
residual emoji fidelity gap is recorded in `docs/unresolved.md`.

## Verification

- Unit: `src/lib/terminal/cursor-layout.test.ts` — width model, ANSI
  skipping, wide-char boundary wrap, the real 40×10 vi repro (asserts no
  phantom pad and cursor on its glyph), measurement-independence, short-pane
  wrap pad, CJK pad.
- Live: `temp/verify-cursor.mjs` (throwaway harness) re-wraps the joined
  capture with the module's model and diffs text-per-visual-row against
  tmux's non-joined capture, then checks pad/row invariants. Passed for:
  fresh shell + wrapped echo, vi ASCII wrap, vi CJK wrap, and a wide char
  straddling the column boundary (tmux's unflagged-wrap case).

## Related paths reviewed (and why they're fine)

- Trailing `ESC[K` after a line that exactly fills the width: xterm's
  `eraseInLine` calls `_restrictCursor` first, so pending-wrap makes it a
  no-op — the last glyph is not erased.
- `term.resize(cursor.w, cursor.h)` runs before the layout calc in
  `_writeToXtermNow`, so measurement width == actual re-wrap width. During
  resize transitions (term ≠ pane dims) frames are transient and the next
  snapshot self-corrects.
- The cursor-only update path shares `computeCursorLayout`, so it is
  wrap-aware too.
- The 500-line scrollback cap can't skew CUP: CUP is viewport-relative.
- `cursor_x` needs no adjustment: with equal widths the re-wrapped visual
  columns match tmux's (verified per-row in the live harness).
