import { selStart, selEnd } from './selection-model.ts';
import type { SelPoint, Selection } from './selection-model.ts';

export interface CellMetrics { w: number; h: number }
export interface ClientOrigin { left: number; top: number }

export function pointToCell(
  clientX: number, clientY: number, rect: ClientOrigin, cell: CellMetrics,
  cols: number, rows: number,
): { col: number; row: number } {
  const { w: cellW, h: cellH } = cell;
  return {
    col: Math.min(cols - 1, Math.max(0, Math.floor((clientX - rect.left) / cellW))),
    row: Math.min(rows - 1, Math.max(0, Math.floor((clientY - rect.top) / cellH))),
  };
}

export function handleGrabOffset(
  clientX: number, clientY: number, ep: SelPoint, viewportY: number,
  rect: ClientOrigin, cell: CellMetrics,
): { dx: number; dy: number } {
  const { w: cw, h: ch } = cell;
  const epCenterX = rect.left + (ep.col + 0.5) * cw;
  const epCenterY = rect.top + (ep.row - viewportY + 0.5) * ch;
  return { dx: clientX - epCenterX, dy: clientY - epCenterY };
}

export function snapHandleColumn(
  x: number, col: number, width: number, cellW: number,
  cols: number, scrollbarWidth: number,
): number {
  const EDGE_SNAP_PX = Math.max(10, cellW * 0.6);
  if (x <= EDGE_SNAP_PX) col = 0;
  else if (x >= width - scrollbarWidth - EDGE_SNAP_PX) col = cols - 1;
  return col;
}

export interface SelectionUI {
  startX: number; startY: number; endX: number; endY: number;
  toolbarX: number; toolbarY: number; toolbarBelow: boolean;
  startInView: boolean; endInView: boolean; toolbarVisible: boolean;
  cellH: number; startDotShiftX: number; endDotShiftX: number;
  startAtLeftEdge: boolean; endAtRightEdge: boolean;
}

export function selectionView(
  selection: Selection, cell: CellMetrics,
  viewport: { top: number; rows: number; cols: number; width: number },
): SelectionUI {
  const { w: cellW, h: cellH } = cell;
  const { top, rows, cols, width: innerW } = viewport;
  const a = selStart(selection)!;
  const b = selEnd(selection)!;
  const aRowV = a.row - top;
  const bRowV = b.row - top;
  const startInView = aRowV >= 0 && aRowV < rows;
  const endInView = bRowV >= 0 && bRowV < rows;
  const startX = a.col * cellW;
  const startY = aRowV * cellH;
  const endX = (b.col + 1) * cellW;
  const endY = (bRowV + 1) * cellH;
  const DOT_R = 6;
  const startAtLeftEdge = a.col === 0;
  const endAtRightEdge = b.col >= cols - 1;
  const startDotShiftX = startAtLeftEdge ? DOT_R : 0;
  const endDotShiftX = endAtRightEdge ? -DOT_R : 0;
  const HANDLE_DOT_CLEARANCE = 22;
  let toolbarX, toolbarY, toolbarBelow = false, toolbarVisible = true;
  // Preserve the anchor-only flip. Rendered-height clipping belongs to #143.
  if (startInView) {
    const cx = a.row === b.row
      ? ((a.col + b.col + 1) / 2) * cellW
      : (a.col * cellW + cellW * Math.min(8, cols - a.col) / 2);
    toolbarX = cx;
    toolbarY = aRowV * cellH - HANDLE_DOT_CLEARANCE;
    if (toolbarY < 8) {
      toolbarY = (Math.min(rows - 1, bRowV) + 1) * cellH + HANDLE_DOT_CLEARANCE;
      toolbarBelow = true;
    }
  } else if (endInView) {
    const cx = (b.col + 1) * cellW - cellW;
    toolbarX = cx;
    toolbarY = (bRowV + 1) * cellH + HANDLE_DOT_CLEARANCE;
    toolbarBelow = true;
  } else {
    toolbarVisible = false;
    toolbarX = 0;
    toolbarY = 0;
  }
  toolbarX = Math.max(48, Math.min(innerW - 48, toolbarX));
  return { startX, startY, endX, endY, toolbarX, toolbarY, toolbarBelow, startInView, endInView, toolbarVisible, cellH, startDotShiftX, endDotShiftX, startAtLeftEdge, endAtRightEdge };
}

export function hitSelectionHandle(
  clientX: number, clientY: number, rect: ClientOrigin & { width: number },
  selection: Selection, selUI: SelectionUI,
): 'start' | 'end' | null {
  const px = clientX - rect.left;
  const py = clientY - rect.top;
  const HIT_HALF_W = 28;
  const HIT_DOT_PAD = 22;
  const cellH = selUI.cellH || 16;
  const innerW = rect.width;
  const sameRow =
    selection.anchor.row === selection.head.row &&
    selUI.startInView && selUI.endInView;
  const midX = sameRow ? (selUI.startX + selUI.endX) / 2 : null;

  if (selUI.startInView) {
    let xMin = selUI.startAtLeftEdge ? 0 : selUI.startX - HIT_HALF_W;
    let xMax = selUI.startX + HIT_HALF_W;
    if (midX !== null) xMax = Math.min(xMax, midX);
    const yMin = selUI.startY - HIT_DOT_PAD * 0.5;
    const yMax = selUI.startY + cellH + HIT_DOT_PAD;
    if (px >= xMin && px <= xMax && py >= yMin && py <= yMax) return 'start';
  }
  if (selUI.endInView) {
    let xMin = selUI.endX - HIT_HALF_W;
    let xMax = selUI.endAtRightEdge ? innerW : selUI.endX + HIT_HALF_W;
    if (midX !== null) xMin = Math.max(xMin, midX);
    const yMin = selUI.endY - cellH - HIT_DOT_PAD * 0.5;
    const yMax = selUI.endY + HIT_DOT_PAD;
    if (px >= xMin && px <= xMax && py >= yMin && py <= yMax) return 'end';
  }
  return null;
}
