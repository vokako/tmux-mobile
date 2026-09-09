// Mobile text-selection model: the source of truth is { anchor, head }
// (both inclusive buffer coordinates), NOT a pre-sorted (start, end) pair —
// selStart/selEnd derive the ordering, so a handle drag that crosses the
// other endpoint just flips which one is "leading" without any swap
// bookkeeping. See docs/design-docs/pages/terminal-gestures.md.

export interface SelPoint { row: number; col: number }
export interface Selection { anchor: SelPoint; head: SelPoint }

export function selStart(s: Selection | null | undefined): SelPoint | null {
  if (!s) return null;
  const { anchor, head } = s;
  if (head.row < anchor.row || (head.row === anchor.row && head.col < anchor.col)) return head;
  return anchor;
}

export function selEnd(s: Selection | null | undefined): SelPoint | null {
  if (!s) return null;
  const { anchor, head } = s;
  if (head.row < anchor.row || (head.row === anchor.row && head.col < anchor.col)) return anchor;
  return head;
}

export function selContains(selection: Selection | null | undefined, row: number, col: number): boolean {
  if (!selection) return false;
  const a = selStart(selection)!, b = selEnd(selection)!;
  if (row < a.row || row > b.row) return false;
  if (a.row === b.row) return col >= a.col && col <= b.col;
  if (row === a.row) return col >= a.col;
  if (row === b.row) return col <= b.col;
  return true;
}

export function selLength(selection: Selection, cols: number): number {
  const a = selStart(selection)!, b = selEnd(selection)!;
  const len = (b.row - a.row) * cols + (b.col - a.col + 1);
  return Math.max(1, len);
}

export function selForDrag(selection: Selection, which: 'start' | 'end'): Selection {
  const a = selStart(selection)!, b = selEnd(selection)!;
  return which === 'start'
    ? { anchor: { ...b }, head: { ...a } }
    : { anchor: { ...a }, head: { ...b } };
}

/** Translate the boundary format, retaining the existing next-row col-0 clamp. */
export function selFromExclusive(pos: {
  start: { x: number; y: number };
  end: { x: number; y: number };
}): Selection {
  const sRow = pos.start.y, sCol = pos.start.x;
  const eRow = pos.end.y;
  const eCol = Math.max(0, pos.end.x - 1);
  return { anchor: { row: sRow, col: sCol }, head: { row: eRow, col: eCol } };
}

/** Scan the already-translated string by its original UTF-16 indices. */
export function wordBounds(text: string | null | undefined, col: number): { start: number; end: number } {
  if (text == null) return { start: col, end: col + 1 };
  if (col >= text.length || /\s/.test(text[col]!)) return { start: col, end: col + 1 };
  let start = col, end = col;
  while (start > 0 && !/\s/.test(text[start - 1]!)) start--;
  while (end < text.length - 1 && !/\s/.test(text[end + 1]!)) end++;
  return { start, end: end + 1 };
}
