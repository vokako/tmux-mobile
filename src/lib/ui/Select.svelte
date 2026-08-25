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

  interface Option { value: string; label?: string; hint?: string; icon?: string }

  let {
    value = $bindable(''),
    options = [] as (string | Option)[],
    disabled = false,
    /** Match the denser field dialect (Team's template editor) instead of the
     * default one (the agent editor's inputs). A dropdown that lines up with
     * neither is what "左右和上方没有对齐" looked like. */
    dense = false,
    /** COMBOBOX mode: the trigger is a real text input — the value stays free
     * text (a model id we cannot enumerate is still typeable), and the menu is
     * the suggestion list, filtered as you type. Built for the agent editor's
     * model field, which used a native <datalist> — the OS popup this
     * component exists to remove (owner, 2026-08-24: "模型选择下拉框明显不对"). */
    editable = false,
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
  let inputEl: HTMLInputElement | null = $state(null);
  let menuEl: HTMLDivElement | null = $state(null);
  let anchor = $state<AnchorRect | null>(null);
  let menuH = $state(0);
  /** Keyboard cursor while open; -1 until the user arrows. */
  let cursor = $state(-1);

  /** Editable mode filters as you type — substring, case-blind, the typed
   * value itself excluded from being "filtered away" logic-wise: an empty or
   * fully-typed value shows the whole list, which is how the field doubles as
   * a browser. */
  const shown = $derived(
    !editable ? norm : (() => {
      const q = value.trim().toLowerCase();
      if (!q) return norm;
      const hit = norm.filter((o) => (o.label ?? o.value).toLowerCase().includes(q) || o.value.toLowerCase().includes(q));
      // The full value matching exactly one option means the user picked it
      // (or finished typing it): offer everything again rather than a
      // one-row menu that just repeats the field.
      return hit.length === 1 && hit[0]!.value === value ? norm : hit;
    })(),
  );

  // EXACTLY as wide as the field: with the menu right-aligned to the trigger,
  // equal widths make both edges line up, which is what a field-shaped picker
  // has to do. A max-width clamp would break that on the wide side, so the only
  // clamp is menuPlacement's viewport one.
  const fieldW = $derived(anchor ? Math.round(anchor.right - anchor.left) : 0);

  // The menu is styled to the FIELD's width, so the placement math uses that
  // number directly. Measuring the box back (bind:clientWidth) fed
  // menuPlacement a width MINUS border and — on desktop, where scrollbars are
  // classic and take real space — minus ~17px of scrollbar whenever the list
  // was long enough to scroll: right-aligned as `anchor.right - w`, the menu
  // overshot the field's right edge by exactly that much (owner, 2026-08-25:
  // "桌面端…下拉框…左右位置偏了"). Overlay-scrollbar platforms never showed it.
  const pos = $derived(anchor ? menuPlacement(anchor, { w: fieldW, h: menuH }, viewBox(), 4) : { x: 0, y: 0 });

  function show() {
    if (disabled || !(triggerEl ?? inputEl)) return;
    anchor = anchorOf((triggerEl ?? inputEl)!);
    menuH = 0;
    cursor = shown.findIndex((o) => o.value === value);
    open = true;
  }
  function hide() { open = false; }
  function pick(v: string) {
    open = false;
    if (v === value) return;
    value = v;
    onchange(v);
  }

  // Dismissal, on everything that means "I moved on": a click elsewhere,
  // Escape, a scroll under the anchor, a resize.
  $effect(() => {
    if (!open) return;
    const onDown = (e: PointerEvent) => {
      // THIS instance's boxes, not the class names: `.closest('.sel-trigger')`
      // matched ANY Select in the app, so with three side by side (the agent
      // editor) opening one and tapping another left the first hanging open
      // (owner, 2026-08-25: "在其他地方点击之后就应该回收了，不应该一直显示
      // 展开在那里"). Another instance is OUTSIDE like everything else.
      const t = e.target as Node | null;
      if (t && (menuEl?.contains(t) || triggerEl?.contains(t) || inputEl?.contains(t))) return;
      hide();
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === 'Escape') { hide(); e.stopPropagation(); (editable ? inputEl : triggerEl)?.focus(); return; }
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const step = e.key === 'ArrowDown' ? 1 : -1;
        cursor = shown.length ? (cursor + step + shown.length) % shown.length : -1;
        return;
      }
      // Space stays typeable in a text field; it only picks for the button.
      if (e.key === 'Enter' || (e.key === ' ' && !editable)) {
        if (cursor >= 0 && shown[cursor]) { e.preventDefault(); pick(shown[cursor]!.value); }
        else if (editable && e.key === 'Enter') hide();
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

{#if editable}
  <!-- COMBOBOX: a real input wearing the trigger's exact clothes, so the two
       modes are indistinguishable at rest. The chevron rides inside the same
       box (pointer-events: none) and still says "there is a list here". -->
  <span class="sel-combo">
    <input class="sel-trigger combo" class:open class:dense bind:this={inputEl}
      {disabled} {placeholder} bind:value
      role="combobox" aria-haspopup="listbox" aria-expanded={open} aria-controls="sel-combo-list"
      aria-label={ariaLabel || undefined}
      autocomplete="off" autocapitalize="off" spellcheck="false"
      oninput={() => { if (!open) show(); cursor = -1; }}
      onclick={() => { if (!open) show(); }}
      onkeydown={(e) => { if (!open && (e.key === 'ArrowDown' || e.key === 'ArrowUp')) { e.preventDefault(); show(); } }} />
    <span class="combo-chev"><Icon name="chevron-down" size={11} /></span>
  </span>
{:else}
<button class="sel-trigger" class:open class:dense bind:this={triggerEl} type="button"
  {disabled} aria-haspopup="listbox" aria-expanded={open} aria-label={ariaLabel || undefined}
  onclick={() => (open ? hide() : show())}>
  {#if current?.icon}<img class="so-ico" src={current.icon} alt="" />{/if}
  <span class="sel-value" class:ph={!label}>{label || placeholder}</span>
  <Icon name="chevron-down" size={11} />
</button>
{/if}

{#if open && shown.length}
  <div class="sel-menu" class:ready={menuH > 0} role="listbox" tabindex="-1" id="sel-combo-list"
    style:left="{pos.x}px" style:top="{pos.y}px" style:width="{fieldW}px"
    bind:this={menuEl} bind:clientHeight={menuH}>
    {#each shown as o, i (o.value)}
      <button class="sel-opt" class:sel={o.value === value} class:cur={i === cursor}
        role="option" aria-selected={o.value === value} type="button"
        onclick={() => pick(o.value)} onpointerenter={() => (cursor = i)}>
        {#if o.icon}<img class="so-ico" src={o.icon} alt="" />{/if}
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
    border-radius: var(--ui-radius-control); padding: 8px 12px; color: var(--text);
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
  .sel-trigger.dense { padding: 6px 9px; border-radius: var(--ui-radius-control); font-size: var(--fs-ui); }
  .sel-trigger:hover:not(:disabled), .sel-trigger.open { border-color: var(--accent); }
  .sel-trigger:disabled { opacity: 0.5; cursor: default; }
  .sel-value { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sel-value.ph { color: var(--text3); }
  .sel-trigger :global(svg) { flex: none; color: var(--text3); }
  /* Combobox clothes: the input IS the trigger, the chevron rides inside its
     right padding so the box still promises a list. */
  .sel-combo { position: relative; display: block; width: 100%; }
  .sel-trigger.combo { display: block; outline: none; padding-right: 26px; cursor: text; }
  .sel-trigger.combo::placeholder { color: var(--text3); }
  .combo-chev {
    position: absolute; right: 9px; top: 50%; transform: translateY(-50%);
    display: grid; place-items: center; pointer-events: none; color: var(--text3);
  }

  /* Same popover dialect as the Hub's menus: one menu language app-wide. */
  .sel-menu {
    position: fixed; z-index: 40; max-height: 46vh; overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: var(--ui-radius-panel);
    box-shadow: 0 12px 34px rgba(0, 0, 0, 0.45); padding: 5px;
    display: flex; flex-direction: column; gap: 2px;
    opacity: 0; transition: opacity var(--t-fast) ease;
  }
  .sel-menu.ready { opacity: 1; }
  .sel-opt {
    display: flex; align-items: center; gap: 8px; min-height: 36px; width: 100%; text-align: left;
    background: none; border: none; border-radius: var(--ui-radius-control); color: var(--text2);
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
  /* An option's icon (a backend logo): sized to the text line, never stretched. */
  .so-ico { flex: none; width: 15px; height: 15px; border-radius: 3px; object-fit: contain; }

  /* Touch contract: a menu row is a tap target. */
  @media (max-width: 760px) {
    .sel-opt { min-height: 44px; }
  }
  /* iOS (only) zooms a focused control below 16px. On Android this bump made
     a Select disagree with the fields beside it — and .dense (0,2,0) beat the
     old media rule anyway, so dense triggers never bumped while inputs did
     (owner, 2026-08-24: "字号还是偏大不一致"). Gate on the iOS family and
     include dense, so where the zoom exists everything bumps TOGETHER. */
  @supports (-webkit-touch-callout: none) {
    @media (max-width: 760px) {
      .sel-trigger, .sel-trigger.dense { font-size: var(--fs-input-touch); }
    }
  }
</style>
