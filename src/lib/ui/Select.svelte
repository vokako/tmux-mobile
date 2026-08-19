<script lang="ts">
  // The app's ONE dropdown. A native <select> pops the OS menu — a different
  // font, a different palette, a different animation, and on desktop WKWebView
  // a separate window entirely — which is exactly the seam the owner asked to
  // remove ("现在我看好像用的系统的下拉菜单，尽量保证我们 ui 统一一致",
  // 2026-08-19). Team's roster picker had already hand-rolled one for the same
  // reason; this is that idea extracted so there is one implementation.
  //
  // It reuses the popover mechanics of the Hub's agent menu: a fixed layer
  // placed from the trigger's rect, because these fields live inside scrolling
  // panels and an absolutely-positioned menu would be clipped by them.
  import Icon from './Icon.svelte';
  import { anchorOf, menuPlacement, viewBox, type AnchorRect } from './placement.ts';

  interface Option { value: string; label?: string; hint?: string }

  let {
    value = $bindable(''),
    options = [] as (string | Option)[],
    disabled = false,
    /** Match the denser field dialect (Team's template editor) instead of the
     * default one (the agent editor's inputs). A dropdown that lines up with
     * neither is what "左右和上方没有对齐" looked like. */
    dense = false,
    placeholder = '',
    ariaLabel = '',
    onchange = (_v: string) => {},
  } = $props();

  const norm = $derived(
    options.map((o): Option => (typeof o === 'string' ? { value: o } : o)),
  );
  const current = $derived(norm.find((o) => o.value === value));
  const label = $derived(current?.label ?? current?.value ?? '');

  let open = $state(false);
  let triggerEl: HTMLButtonElement | null = $state(null);
  let anchor = $state<AnchorRect | null>(null);
  let menuW = $state(0);
  let menuH = $state(0);
  /** Keyboard cursor while open; -1 until the user arrows. */
  let cursor = $state(-1);

  const pos = $derived(anchor ? menuPlacement(anchor, { w: menuW, h: menuH }, viewBox(), 4) : { x: 0, y: 0 });

  function show() {
    if (disabled || !triggerEl) return;
    anchor = anchorOf(triggerEl);
    menuW = 0; menuH = 0;
    cursor = norm.findIndex((o) => o.value === value);
    open = true;
  }
  function hide() { open = false; }
  function pick(v: string) {
    open = false;
    if (v === value) return;
    value = v;
    onchange(v);
  }

  // EXACTLY as wide as the field: with the menu right-aligned to the trigger,
  // equal widths make both edges line up, which is what a field-shaped picker
  // has to do. A max-width clamp would break that on the wide side, so the only
  // clamp is menuPlacement's viewport one.
  const fieldW = $derived(anchor ? Math.round(anchor.right - anchor.left) : 0);

  // Dismissal, on everything that means "I moved on": a click elsewhere,
  // Escape, a scroll under the anchor, a resize.
  $effect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      const t = e.target as Element | null;
      if (!t?.closest?.('.sel-menu, .sel-trigger')) hide();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { hide(); e.stopPropagation(); triggerEl?.focus(); return; }
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const step = e.key === 'ArrowDown' ? 1 : -1;
        cursor = (cursor + step + norm.length) % norm.length;
        return;
      }
      if (e.key === 'Enter' || e.key === ' ') {
        if (cursor >= 0 && norm[cursor]) { e.preventDefault(); pick(norm[cursor]!.value); }
      }
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', hide);
    window.addEventListener('scroll', hide, true);   // capture: any ancestor
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', hide);
      window.removeEventListener('scroll', hide, true);
    };
  });
</script>

<button class="sel-trigger" class:open class:dense bind:this={triggerEl} type="button"
  {disabled} aria-haspopup="listbox" aria-expanded={open} aria-label={ariaLabel || undefined}
  onclick={() => (open ? hide() : show())}>
  <span class="sel-value" class:ph={!label}>{label || placeholder}</span>
  <Icon name="chevron-down" size={11} />
</button>

{#if open}
  <div class="sel-menu" class:ready={menuH > 0} role="listbox" tabindex="-1"
    style:left="{pos.x}px" style:top="{pos.y}px" style:width="{fieldW}px"
    bind:clientWidth={menuW} bind:clientHeight={menuH}>
    {#each norm as o, i (o.value)}
      <button class="sel-opt" class:sel={o.value === value} class:cur={i === cursor}
        role="option" aria-selected={o.value === value} type="button"
        onclick={() => pick(o.value)} onpointerenter={() => (cursor = i)}>
        <span class="so-label">{o.label ?? o.value}</span>
        {#if o.hint}<span class="so-hint">{o.hint}</span>{/if}
        {#if o.value === value}<Icon name="check" size={12} />{/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  /* The trigger wears the app's INPUT dialect, so a dropdown sits in a form
     next to text fields without announcing itself as a different species. */
  /* Metrically IDENTICAL to the app's text inputs — same padding, radius, type
     step and line box — so a dropdown in a form row lines up with the field
     beside it on every edge. */
  .sel-trigger {
    display: flex; align-items: center; gap: 6px; width: 100%;
    background: var(--input-bg); border: 1px solid var(--input-border);
    border-radius: 9px; padding: 8px 12px; color: var(--text);
    font-size: var(--fs-body); font-family: inherit;
    /* `normal`, not a ratio: an <input> computes its line box from `normal`,
       so matching the keyword is what makes the two boxes the same height.
       Inheriting the page's 1.5 made the trigger ~4px taller than the field
       next to it, which is the misalignment the owner saw. */
    line-height: normal;
    cursor: pointer; text-align: left;
    transition: border-color var(--t-fast) ease;
    -webkit-tap-highlight-color: transparent;
  }
  /* The other field dialect in the app (Team's template editor). */
  .sel-trigger.dense { padding: 6px 9px; border-radius: 7px; font-size: var(--fs-ui); }
  .sel-trigger:hover:not(:disabled), .sel-trigger.open { border-color: var(--accent); }
  .sel-trigger:disabled { opacity: 0.5; cursor: default; }
  .sel-value { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sel-value.ph { color: var(--text3); }
  .sel-trigger :global(svg) { flex: none; color: var(--text3); }

  /* Same popover dialect as the Hub's menus: one menu language app-wide. */
  .sel-menu {
    position: fixed; z-index: 40; max-height: 46vh; overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: 11px;
    box-shadow: 0 12px 34px rgba(0, 0, 0, 0.45); padding: 5px;
    display: flex; flex-direction: column; gap: 2px;
    opacity: 0; transition: opacity var(--t-fast) ease;
  }
  .sel-menu.ready { opacity: 1; }
  .sel-opt {
    display: flex; align-items: center; gap: 8px; min-height: 36px; width: 100%; text-align: left;
    background: none; border: none; border-radius: 8px; color: var(--text2);
    padding: 6px 10px; font-size: var(--ui-font-control); cursor: pointer;
    font-family: ui-monospace, Menlo, monospace;
  }
  /* Hover and the keyboard cursor are the SAME highlight — two different ones
     read as two selections. */
  .sel-opt:hover, .sel-opt.cur { background: var(--surface2); color: var(--text); }
  .sel-opt.sel { color: var(--accent); }
  .sel-opt :global(svg) { margin-left: auto; flex: none; color: var(--accent); }
  .so-label { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .so-hint { font-size: var(--fs-meta); color: var(--text3); font-family: inherit; }

  /* Touch contract: a menu row is a tap target. */
  @media (max-width: 760px) {
    .sel-opt { min-height: 44px; }
    /* The trigger is an input, and iOS zooms a focused control below this. */
    .sel-trigger { font-size: var(--fs-input-touch); }
  }
</style>
