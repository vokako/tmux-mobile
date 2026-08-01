<script>
  // SideHandle — the one resize affordance for the shared sidebar
  // (docs/design-docs/features/ui-unification.md). Drag live-updates
  // --sidebar-w on :root, release persists, double-click resets, arrow keys
  // nudge. This component and App's init read are the ONLY writers.
  const MIN = 180;
  const MAX = 420;
  const DEFAULT = 240;

  let dragging = $state(false);

  function current() {
    const v = parseInt(getComputedStyle(document.documentElement).getPropertyValue('--sidebar-w'), 10);
    return Number.isFinite(v) ? v : DEFAULT;
  }
  function apply(w) {
    const clamped = Math.min(MAX, Math.max(MIN, Math.round(w)));
    document.documentElement.style.setProperty('--sidebar-w', clamped + 'px');
    return clamped;
  }
  function persist(w) {
    localStorage.setItem('tmux_sidebar_w', String(w));
  }

  function onPointerDown(e) {
    e.preventDefault();
    const startX = e.clientX;
    const startW = current();
    dragging = true;
    const move = (ev) => apply(startW + (ev.clientX - startX));
    const up = (ev) => {
      dragging = false;
      persist(apply(startW + (ev.clientX - startX)));
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
      persist(apply(current() + (e.key === 'ArrowRight' ? 16 : -16)));
    }
  }
</script>

<!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions --
     role="separator" + tabindex IS the WAI-ARIA window-splitter pattern; the
     checker just doesn't know separators can be focusable movable splitters. -->
<div
  class="side-handle"
  class:dragging
  role="separator"
  aria-orientation="vertical"
  aria-label="Resize sidebar"
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
  }
  .side-handle:hover, .side-handle.dragging {
    background: linear-gradient(90deg, transparent 40%, var(--accent) 40%, var(--accent) 60%, transparent 60%);
    opacity: 0.6;
  }
  .side-handle:focus-visible { outline: none; background: var(--accent-bg); }
  /* Narrow layouts have no sidebar to resize (single-column pages keep their
     list full-width) — the handle disappears with the geometry it controls. */
  @media (max-width: 760px) {
    .side-handle { display: none; }
  }
</style>
