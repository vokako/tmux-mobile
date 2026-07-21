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
