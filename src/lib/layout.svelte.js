// Shared layout-mode setting. Touch detection alone mis-classifies iPad-style
// tablets (touch + wide screen) as "mobile", hiding the desktop split UI. This
// lets the user force Desktop or Mobile, or leave it on Auto (detection).
//
// Scope: this only affects LAYOUT decisions (desktop split panes). Input/gesture
// handling stays touch-based regardless, so a tablet in "desktop" mode is still
// fully touch-usable.

const KEY = 'tmux_layout_mode';
const isValid = (v) => v === 'desktop' || v === 'mobile' || v === 'auto';

let initial = 'auto';
try {
  const s = localStorage.getItem(KEY);
  if (isValid(s)) initial = s;
} catch {}

let mode = $state(initial); // 'auto' | 'desktop' | 'mobile'

const rawTouch =
  typeof window !== 'undefined' &&
  ('ontouchstart' in window || (navigator.maxTouchPoints || 0) > 0);

export const layout = {
  /** Current mode: 'auto' | 'desktop' | 'mobile'. */
  get mode() {
    return mode;
  },
  /** Set and persist the mode. */
  set(v) {
    if (!isValid(v)) return;
    mode = v;
    try {
      localStorage.setItem(KEY, v);
    } catch {}
  },
  /** Effective touch/mobile-UI decision, honoring the override. */
  get isTouchDevice() {
    if (mode === 'desktop') return false;
    if (mode === 'mobile') return true;
    return rawTouch;
  },
  /** True when the user explicitly forced desktop (split should ignore width). */
  get forceDesktop() {
    return mode === 'desktop';
  },
};
