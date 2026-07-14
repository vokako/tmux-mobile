const LINE_HEIGHT_KEY = 'tmux_line_height';
const storedLineHeight = Number.parseFloat(localStorage.getItem(LINE_HEIGHT_KEY) || '1');

const state = $state({
  lineHeight: Number.isFinite(storedLineHeight) ? Math.max(0.8, Math.min(1.6, storedLineHeight)) : 1,
});

export const terminalPrefs = {
  get lineHeight() { return state.lineHeight; },
  setLineHeight(value) {
    state.lineHeight = Math.max(0.8, Math.min(1.6, value));
    localStorage.setItem(LINE_HEIGHT_KEY, String(state.lineHeight));
  },
};
