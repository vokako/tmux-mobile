// Cursor placement math for full-rewrite and cursor-only xterm writes.
// Extracted from Terminal.svelte so both paths share one implementation
// and the invariants below are unit-tested.
import type { Cursor } from '../core/ws.ts';

/** Count lines without allocating a split array. */
export function countLines(s: string): number {
  let n = 1;
  for (let i = 0; i < s.length; i++) if (s[i] === '\n') n++;
  return n;
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
export function computeCursorLayout(content: string, cursor: Cursor, rows: number): CursorLayout {
  const N = countLines(content);
  const trailing = cursor.t || 0;
  const paneStart = Math.max(0, N + trailing - cursor.h);
  const cursorLine = paneStart + cursor.y;
  const needAfter = Math.max(0, cursorLine + 1 - N);
  const contentLines = N + needAfter;
  const bottomPad = Math.max(0, paneStart + cursor.h - contentLines);
  const totalWritten = contentLines + bottomPad;
  const sb = Math.max(0, totalWritten - rows);
  return {
    row: cursorLine - sb + 1,
    afterPad: needAfter + bottomPad > 0 ? '\n'.repeat(needAfter + bottomPad) : '',
  };
}
