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
    // panel when dragged LEFT, so the delta is inverted.
    edge = 'right',
    label = 'Resize sidebar',
  } = $props();
  const MIN = $derived(min);
  const MAX = $derived(max);
  const DEFAULT = $derived(def);
  const sign = $derived(edge === 'left' ? -1 : 1);

  let dragging = $state(false);

  function current() {
    const v = parseInt(getComputedStyle(document.documentElement).getPropertyValue(varName), 10);
    return Number.isFinite(v) ? v : DEFAULT;
  }
  function apply(w) {
    const clamped = Math.min(MAX, Math.max(MIN, Math.round(w)));
    document.documentElement.style.setProperty(varName, clamped + 'px');
    return clamped;
  }
  function persist(w) {
    localStorage.setItem(storeKey, String(w));
  }

  function onPointerDown(e) {
    e.preventDefault();
    const startX = e.clientX;
    const startW = current();
    dragging = true;
    const move = (ev) => apply(startW + sign * (ev.clientX - startX));
    const up = (ev) => {
      dragging = false;
      persist(apply(startW + sign * (ev.clientX - startX)));
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
    if (e.key === 'ArrowLeft' || e.key === 'ArrowRight') {
      e.preventDefault();
      persist(apply(current() + sign * (e.key === 'ArrowRight' ? 16 : -16)));
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
  role="separator"
  aria-orientation="vertical"
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
  .side-handle:focus-visible { outline: none; background: var(--accent-bg); opacity: 1; }
  @media (prefers-reduced-motion: reduce) { .side-handle { transition: none; } }
  /* Narrow layouts have no sidebar to resize (single-column pages keep their
     list full-width) — the handle disappears with the geometry it controls. */
  @media (max-width: 760px) {
    .side-handle { display: none; }
  }
</style>
