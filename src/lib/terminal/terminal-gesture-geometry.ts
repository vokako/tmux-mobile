import type { SelPoint } from './selection-model.ts';

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
