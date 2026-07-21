export interface CompactLineGeometry {
  charCssHeight: number;
  offset: number;
}

export function compactLineGeometry(
  charDeviceHeight: number,
  cellCssHeight: number,
  devicePixelRatio: number,
  lineHeight: number,
): CompactLineGeometry | null {
  if (lineHeight >= 1 || !charDeviceHeight || !cellCssHeight || !devicePixelRatio) return null;
  const charCssHeight = charDeviceHeight / devicePixelRatio;
  const clippedHeight = Math.max(0, charCssHeight - cellCssHeight);
  return {
    charCssHeight,
    offset: clippedHeight / 2,
  };
}
