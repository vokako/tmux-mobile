<script>
  // One context menu for the whole app: right-click on the desktop, long press on
  // a phone (owner, 2026-08-20: "还有很多地方增加右键点击操作，和手机长按").
  //
  // It is deliberately the SAME popover dialect as the Hub's agent menu and the
  // shared Select — a fixed layer placed by `menuPlacement`, dismissed by an
  // outside pointerdown / Escape / any ancestor scroll / a resize — because a
  // second menu language would read as a second kind of menu. The only difference
  // is what it is anchored to: a pointer instead of a trigger's rect.
  import Icon from './Icon.svelte';
  import { menuPlacement, pointAnchor, viewBox } from './placement.ts';

  /**
   * @typedef {{ label: string, icon?: string, danger?: boolean, disabled?: boolean,
   *             onselect: () => void }} MenuItem
   */
  let {
    /** Client coordinates of the pointer, or null when closed. */
    at = null,
    /** @type {MenuItem[]} */ items = [],
    /** Optional heading — usually the name of what was clicked. */
    who = '',
    oncancel = () => {},
  } = $props();

  let el = $state(null);
  let w = $state(0);
  let h = $state(0);
  let cursor = $state(-1);

  // Measured before it is placed: an unmeasured menu would be positioned from a
  // zero height and jump. Hidden for that one frame, exactly like the agent menu.
  const pos = $derived(at ? menuPlacement(pointAnchor(at.x, at.y), { w, h }, viewBox()) : { x: 0, y: 0 });

  $effect(() => {
    if (!at) {
      cursor = -1;
      return;
    }
    // Dismissal, all four ways. `pointerdown` rather than click, so the menu goes
    // away on the press that starts somewhere else instead of waiting for its
    // release.
    const outside = (e) => {
      if (el && !el.contains(e.target)) oncancel();
    };
    const onKey = (e) => {
      if (e.key === 'Escape') {
        e.preventDefault();
        oncancel();
        return;
      }
      const usable = items.filter((i) => !i.disabled);
      if (!usable.length) return;
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const step = e.key === 'ArrowDown' ? 1 : -1;
        cursor = (cursor + step + items.length) % items.length;
        // Skip a disabled row rather than parking the cursor on it.
        let guard = 0;
        while (items[cursor]?.disabled && guard++ < items.length) {
          cursor = (cursor + step + items.length) % items.length;
        }
        return;
      }
      if (e.key === 'Enter' || e.key === ' ') {
        const it = items[cursor];
        if (it && !it.disabled) {
          e.preventDefault();
          it.onselect();
          oncancel();
        }
      }
    };
    window.addEventListener('pointerdown', outside, true);
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', oncancel);
    // Capture, so a scroll in ANY ancestor closes it — the menu is a fixed layer
    // and would otherwise stay behind while its subject scrolls away.
    window.addEventListener('scroll', oncancel, true);
    return () => {
      window.removeEventListener('pointerdown', outside, true);
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', oncancel);
      window.removeEventListener('scroll', oncancel, true);
    };
  });
</script>

{#if at && items.length}
  <div class="ctx" class:ready={h > 0} bind:this={el} role="menu" tabindex="-1"
    style:left="{pos.x}px" style:top="{pos.y}px"
    bind:clientWidth={w} bind:clientHeight={h}>
    {#if who}<div class="ctx-who">{who}</div>{/if}
    {#each items as it, i (it.label)}
      <button role="menuitem" class:danger={it.danger} class:cur={i === cursor}
        disabled={it.disabled}
        onpointerenter={() => (cursor = i)}
        onclick={() => { it.onselect(); oncancel(); }}>
        {#if it.icon}<Icon name={it.icon} size={12} />{/if}{it.label}
      </button>
    {/each}
  </div>
{/if}

<style>
  .ctx {
    position: fixed; z-index: 60; min-width: 156px; max-width: 260px;
    background: var(--bg); border: 1px solid var(--border); border-radius: 12px;
    box-shadow: 0 14px 38px rgba(0, 0, 0, 0.45); padding: 5px;
    display: flex; flex-direction: column; gap: 1px;
    /* Invisible until measured, so it cannot be seen at the wrong place. */
    opacity: 0; pointer-events: none;
  }
  .ctx.ready { opacity: 1; pointer-events: auto; }
  .ctx-who {
    padding: 3px 9px 5px; font-size: var(--fs-meta); color: var(--text3);
    font-family: ui-monospace, Menlo, monospace;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .ctx button {
    display: flex; align-items: center; gap: 7px; width: 100%; text-align: left;
    background: none; border: 0; border-radius: 7px; color: var(--text2);
    font-size: var(--ui-font-control); padding: 6px 9px; cursor: pointer;
  }
  /* Hover and the keyboard cursor are ONE highlight: two would read as two
     selections. Same rule as Select and the composer's command palette. */
  .ctx button:hover, .ctx button.cur { background: var(--surface2); color: var(--text); }
  .ctx button.danger { color: var(--danger); }
  .ctx button.danger:hover, .ctx button.danger.cur { background: var(--danger-bg); }
  .ctx button:disabled { opacity: 0.45; cursor: default; }
  .ctx button :global(svg) { flex: none; }
  /* A phone needs a real target; the desktop stays compact. */
  @media (max-width: 760px) {
    .ctx button { min-height: 40px; font-size: var(--fs-body); }
  }
</style>
