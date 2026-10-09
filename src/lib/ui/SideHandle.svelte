<script>
  // SideHandle — the ONE resize affordance for every draggable divider
  // (docs/design-docs/features/ui-unification.md). Drag live-updates a CSS
  // custom property on :root, release persists it, double-click resets, arrow
  // keys nudge. Defaults describe the shared sidebar; the Hub's chat/terminal
  // divider passes its own variable, bounds and `left` edge instead of forking
  // a second implementation.
  let {
    varName = '--sidebar-w',
    storeKey = 'tmux_sidebar_w',
    min = 180,
    max = 420,
    def = 240,
    // Which edge of the parent the handle sits on. A `left` handle grows the
    // panel when dragged LEFT, so the delta is inverted. A `top` handle
    // (board #324, the scratch terminal from the bottom) works on the Y axis:
    // dragging UP grows the panel, Up/Down nudge, and it is a horizontal
    // separator for assistive tech.
    edge = 'right',
    label = 'Resize sidebar',
    /** Stay shown on narrow layouts (a panel that exists there too). */
    always = false,
  } = $props();
  const MIN = $derived(min);
  const DEFAULT = $derived(def);
  const vertical = $derived(edge === 'top');
  const sign = $derived(edge === 'left' || edge === 'top' ? -1 : 1);
  // Never larger than the viewport can hold (zoom-corrected), whatever was
  // stored before the window shrank.
  const room = () => (vertical ? window.innerHeight : window.innerWidth) / (parseFloat(getComputedStyle(document.documentElement).getPropertyValue('--ui-zoom')) || 1) - 80;
  const cap = () => Math.max(MIN, Math.min(max, room()));

  let dragging = $state(false);

  function current() {
    const v = parseInt(getComputedStyle(document.documentElement).getPropertyValue(varName), 10);
    return Number.isFinite(v) ? v : DEFAULT;
  }
  function apply(w) {
    const clamped = Math.min(cap(), Math.max(MIN, Math.round(w)));
    document.documentElement.style.setProperty(varName, clamped + 'px');
    return clamped;
  }
  function persist(w) {
    localStorage.setItem(storeKey, String(w));
  }

  function onPointerDown(e) {
    e.preventDefault();
    const at = (ev) => (vertical ? ev.clientY : ev.clientX);
    const start = at(e);
    const startW = current();
    dragging = true;
    const move = (ev) => apply(startW + sign * (at(ev) - start));
    const up = (ev) => {
      dragging = false;
      persist(apply(startW + sign * (at(ev) - start)));
      window.removeEventListener('pointermove', move);
      window.removeEventListener('pointerup', up);
    };
    window.addEventListener('pointermove', move);
    window.addEventListener('pointerup', up);
  }

  function onDblClick() {
    persist(apply(DEFAULT));
  }

  function onKeyDown(e) {
    const [less, more] = vertical ? ['ArrowUp', 'ArrowDown'] : ['ArrowLeft', 'ArrowRight'];
    if (e.key === less || e.key === more) {
      e.preventDefault();
      persist(apply(current() + sign * (e.key === more ? 16 : -16)));
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions --
     role="separator" + tabindex IS the WAI-ARIA window-splitter pattern; the
     checker just doesn't know separators can be focusable movable splitters. -->
<div
  class="side-handle"
  class:dragging
  class:on-left={edge === 'left'}
  class:on-top={vertical}
  class:always
  role="separator"
  aria-orientation={vertical ? 'horizontal' : 'vertical'}
  aria-label={label}
  tabindex="0"
  onpointerdown={onPointerDown}
  ondblclick={onDblClick}
  onkeydown={onKeyDown}
></div>

<style>
  .side-handle {
    position: absolute;
    top: 0; bottom: 0; right: -3px;
    width: 6px;
    cursor: col-resize;
    z-index: 5;
    touch-action: none;
    /* The accent line is always painted; hover/drag only turn its opacity up,
       so the reveal is one cross-fade on --t-fast (motion.md: transform and
       opacity only). */
    background: linear-gradient(90deg, transparent 40%, var(--accent) 40%, var(--accent) 60%, transparent 60%);
    opacity: 0;
    transition: opacity var(--t-fast);
  }
  .side-handle:hover, .side-handle.dragging { opacity: 0.6; }
  .side-handle.on-left { right: auto; left: -3px; }
  .side-handle.on-top {
    top: -3px; bottom: auto; left: 0; right: 0; width: auto; height: 6px; cursor: row-resize;
    background: linear-gradient(180deg, transparent 40%, var(--accent) 40%, var(--accent) 60%, transparent 60%);
  }
  .side-handle:focus-visible { outline: none; background: var(--accent-bg); opacity: 1; }
  @media (prefers-reduced-motion: reduce) { .side-handle { transition: none; } }
  /* Narrow layouts have no sidebar to resize (single-column pages keep their
     list full-width) — the handle disappears with the geometry it controls. */
  @media (max-width: 760px) {
    .side-handle:not(.always) { display: none; }
  }
</style>
