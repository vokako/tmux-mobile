export const UI_ZOOM_DEFAULT = 1;
export const UI_ZOOM_MIN = 0.6;
export const UI_ZOOM_MAX = 1.8;
export const UI_ZOOM_STEP = 0.1;

export function normalizeUiZoom(value: unknown): number {
  // Number(null) === 0 and Number('') === 0 — a fresh client with no stored
  // zoom must get the DEFAULT, not 0 clamped to UI_ZOOM_MIN (which shipped a
  // 60% boot scale to every first-run web client).
  if (value == null || value === '') return UI_ZOOM_DEFAULT;
  const parsed = Number(value);
  if (!Number.isFinite(parsed) || parsed <= 0) return UI_ZOOM_DEFAULT;
  return Math.round(Math.max(UI_ZOOM_MIN, Math.min(UI_ZOOM_MAX, parsed)) * 10) / 10;
}

export function stepUiZoom(value: unknown, direction: 1 | -1): number {
  return normalizeUiZoom(normalizeUiZoom(value) + direction * UI_ZOOM_STEP);
}
