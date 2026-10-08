// Where "type in the terminal" lands (board 316 review P1). In split screen
// the identity of the pane being worked on is the ACTIVE cell (App's
// activeCellId, SplitView's data-cell-id + .active); keyboard input is routed
// by DOM focus, so focusing any other terminal would send keys to a pane the
// highlight does not show. An active cell with no pane yields nothing: its
// input must not fall through to another cell.

const INPUT = '.xterm-helper-textarea';

export function terminalFocusTarget(page: ParentNode | null | undefined, activeCellId: number | null): HTMLElement | null {
  if (!page) return null;
  if (activeCellId != null) {
    const cell = page.querySelector(`.cell[data-cell-id="${CSS.escape(String(activeCellId))}"]`);
    return cell?.querySelector<HTMLElement>(INPUT) ?? null;
  }
  // Single pane: the one terminal of the page.
  return page.querySelector<HTMLElement>(`.term-main ${INPUT}`);
}
