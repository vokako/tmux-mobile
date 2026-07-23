// Cursor placement math for full-rewrite and cursor-only xterm writes.
// Extracted from Terminal.svelte so both paths share one implementation
// and the invariants below are unit-tested.
import type { Cursor } from '../core/ws.ts';

// ── Visual-row measurement ──────────────────────────────────────────────
//
// The server captures with `capture-pane -J`, which joins soft-wrapped
// lines into single LOGICAL lines; xterm re-wraps them at term.cols when
// written. But tmux's cursor_y / pane_height are VISUAL rows. All layout
// math below therefore runs in visual-row space: each logical line is
// measured with the same width model xterm's default UnicodeV6 provider
// uses (no unicode addon is loaded), so the predicted wrap count matches
// what xterm actually renders. Counting logical lines instead (the old
// implementation) put the cursor one row too low per wrap visible in the
// pane and padded the same number of spurious blank rows at the bottom.
//
// NOTE this deliberately mirrors XTERM's width table, not the server's
// `is_wide_char` (tmux-oriented, counts emoji as 2) — we are predicting
// the re-wrap xterm performs, not the wrap tmux performed.

/** Zero-width besides Unicode marks (mirrors UnicodeV6 BMP_COMBINING extras). */
function isZeroWidth(cp: number): boolean {
  return (
    (cp >= 0x1160 && cp <= 0x11ff) || // Hangul jungseong/jongseong
    (cp >= 0x200b && cp <= 0x200f) ||
    (cp >= 0x202a && cp <= 0x202e) ||
    (cp >= 0x2060 && cp <= 0x2063) ||
    (cp >= 0x206a && cp <= 0x206f) ||
    (cp >= 0x0600 && cp <= 0x0603) ||
    cp === 0x070f || cp === 0xfeff ||
    (cp >= 0xfff9 && cp <= 0xfffb)
  );
}

/** Wide (2-cell) per xterm's UnicodeV6 table. */
function isWide(cp: number): boolean {
  return (
    (cp >= 0x1100 && cp <= 0x115f) ||
    cp === 0x2329 || cp === 0x232a ||
    (cp >= 0x2e80 && cp <= 0xa4cf && cp !== 0x303f) ||
    (cp >= 0xac00 && cp <= 0xd7a3) ||
    (cp >= 0xf900 && cp <= 0xfaff) ||
    (cp >= 0xfe10 && cp <= 0xfe19) ||
    (cp >= 0xfe30 && cp <= 0xfe6f) ||
    (cp >= 0xff00 && cp <= 0xff60) ||
    (cp >= 0xffe0 && cp <= 0xffe6) ||
    (cp >= 0x20000 && cp <= 0x2fffd) ||
    (cp >= 0x30000 && cp <= 0x3fffd)
  );
}

// Nonspacing/enclosing marks only — spacing combining marks (Mc) occupy a
// cell in xterm's V6 table.
const MARK_RE = /[\p{Mn}\p{Me}]/u;

/** Cell width of one code point under xterm's default UnicodeV6 provider. */
export function cellWidth(ch: string): number {
  const cp = ch.codePointAt(0)!;
  if (cp < 32 || (cp >= 0x7f && cp < 0xa0)) return 0; // control chars
  if (cp < 0x7f) return 1; // printable ASCII fast path
  if (isWide(cp)) return 2;
  if (isZeroWidth(cp) || MARK_RE.test(ch)) return 0;
  return 1;
}

/**
 * Visual rows one logical line occupies when xterm re-wraps it at `cols`,
 * skipping ANSI escape sequences (SGR from capture -e, OSC just in case).
 * Wrap rule mirrors xterm: a glyph that no longer fits starts a new row
 * (a wide glyph at the last column wraps whole, leaving the cell empty).
 */
export function visualRowsOfLine(line: string, cols: number): number {
  let rows = 1;
  let col = 0;
  let i = 0;
  const len = line.length;
  while (i < len) {
    const code = line.charCodeAt(i);
    if (code === 0x1b) {
      // Skip ESC sequence: CSI `ESC [ ... final @-~`, OSC `ESC ] ... BEL|ST`,
      // else two-byte `ESC x`.
      const next = line[i + 1];
      if (next === '[') {
        i += 2;
        while (i < len && !(line.charCodeAt(i) >= 0x40 && line.charCodeAt(i) <= 0x7e)) i++;
        i++; // final byte
      } else if (next === ']') {
        i += 2;
        while (i < len && line.charCodeAt(i) !== 0x07 && !(line.charCodeAt(i) === 0x1b && line[i + 1] === '\\')) i++;
        i += line.charCodeAt(i) === 0x07 ? 1 : 2;
      } else {
        i += 2;
      }
      continue;
    }
    // Printable ASCII fast path (the overwhelming majority of cells).
    if (code >= 0x20 && code < 0x7f) {
      if (col + 1 > cols) { rows++; col = 0; }
      col++;
      i++;
      continue;
    }
    const ch = String.fromCodePoint(line.codePointAt(i)!);
    const w = cellWidth(ch);
    if (w > 0) {
      if (col + w > cols) { rows++; col = 0; }
      col += w;
    }
    i += ch.length;
  }
  return rows;
}

/** Total visual rows `content` occupies at width `cols` (each '\n' starts a new logical line). */
export function countVisualRows(content: string, cols: number): number {
  let rows = 0;
  let start = 0;
  for (;;) {
    const nl = content.indexOf('\n', start);
    const line = nl === -1 ? content.slice(start) : content.slice(start, nl);
    rows += visualRowsOfLine(line, cols);
    if (nl === -1) return rows;
    start = nl + 1;
  }
}

export interface CursorLayout {
  /** xterm row (1-based) where the cursor must land after the write. */
  row: number;
  /** Newlines to append so the buffer ends at the pane's bottom row. */
  afterPad: string;
}

// The written buffer must end at the pane's BOTTOM row: capture trims the
// pane's trailing blank rows (cursor.t), and without restoring them
// xterm's bottom-anchored viewport would show the tail of the CONTENT
// (scrollback history included) instead of the visible pane — a freshly
// cleared pane rendered with its prompt/cursor at the page bottom.
// Restoring them makes the viewport mirror the pane exactly: prompt at
// top, blanks below, history up in the scrollback.
//
// All quantities are VISUAL rows (see measurement note above). A useful
// invariant: when the pane is full (visual content + trimmed rows ≥ pane
// height) this reduces to afterPad = t and row = y + 1 + (rows - h) — the
// cursor row no longer depends on the measurement at all, so width-model
// imprecision can only surface on a mostly-empty pane, where lines are
// short and unwrapped anyway.
export function computeCursorLayout(
  content: string,
  cursor: Cursor,
  rows: number,
  cols: number
): CursorLayout {
  const trailing = cursor.t || 0;
  const visual = countVisualRows(content, cols);
  // First visual row of the pane within the written buffer (0-based).
  const paneStart = Math.max(0, visual + trailing - cursor.h);
  // Buffer must extend to the pane's bottom row. paneStart + h ≥ visual
  // always (equality analysis in the invariant note above), so pad ≥ 0;
  // the pad also materializes any rows between content end and the cursor.
  const total = paneStart + cursor.h;
  const pad = total - visual;
  // Rows that scroll above the viewport during the write.
  const sb = Math.max(0, total - rows);
  return {
    row: paneStart + cursor.y - sb + 1,
    afterPad: pad > 0 ? '\n'.repeat(pad) : '',
  };
}
