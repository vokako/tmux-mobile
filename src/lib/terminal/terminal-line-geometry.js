export function compactLineGeometry(charDeviceHeight, cellCssHeight, devicePixelRatio, lineHeight) {
  if (lineHeight >= 1 || !charDeviceHeight || !cellCssHeight || !devicePixelRatio) return null;
  const charCssHeight = charDeviceHeight / devicePixelRatio;
  const clippedHeight = Math.max(0, charCssHeight - cellCssHeight);
  return {
    charCssHeight,
    offset: clippedHeight / 2,
  };
}
