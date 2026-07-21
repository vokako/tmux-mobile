export const UI_ZOOM_DEFAULT = 1;
export const UI_ZOOM_MIN = 0.6;
export const UI_ZOOM_MAX = 1.8;
export const UI_ZOOM_STEP = 0.1;

export function normalizeUiZoom(value) {
  const parsed = Number(value);
  if (!Number.isFinite(parsed)) return UI_ZOOM_DEFAULT;
  return Math.round(Math.max(UI_ZOOM_MIN, Math.min(UI_ZOOM_MAX, parsed)) * 10) / 10;
}

export function stepUiZoom(value, direction) {
  return normalizeUiZoom(normalizeUiZoom(value) + direction * UI_ZOOM_STEP);
}
