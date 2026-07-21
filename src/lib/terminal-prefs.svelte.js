const LINE_HEIGHT_KEY = 'tmux_line_height';
export const LINE_HEIGHT_MIN = 0.4;
export const LINE_HEIGHT_MAX = 1.6;
const storedLineHeight = Number.parseFloat(localStorage.getItem(LINE_HEIGHT_KEY) || '1');

const state = $state({
  lineHeight: Number.isFinite(storedLineHeight) ? Math.max(LINE_HEIGHT_MIN, Math.min(LINE_HEIGHT_MAX, storedLineHeight)) : 1,
});

export const terminalPrefs = {
  get lineHeight() { return state.lineHeight; },
  setLineHeight(value) {
    state.lineHeight = Math.max(LINE_HEIGHT_MIN, Math.min(LINE_HEIGHT_MAX, value));
    localStorage.setItem(LINE_HEIGHT_KEY, String(state.lineHeight));
  },
};
