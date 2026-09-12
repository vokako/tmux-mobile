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
  import { menuHeightLimit, menuPlacement, pointAnchor, popOrigin, viewBox, scrollMovesTrigger } from './placement.ts';
  import { activeModal } from './modal.ts';
  import { nextMenuIndex } from './menu-navigation.ts';
  const menuId = $props.id();

  /**
   * @typedef {{ label: string, icon?: string, hint?: string, checked?: boolean, title?: string,
   *             danger?: boolean, warn?: boolean, disabled?: boolean,
   *             onselect: () => void }} MenuItem
   *
   * `checked` marks the CURRENT choice when the menu is a pick-one over a
   * control that is not a field (the split layout, the Team switcher) — the
   * same trailing check Select draws, so a menu and a dropdown say "you are
   * here" in one glyph. `hint` is Select's secondary text (a count, a path);
   * `title` is the hover tooltip for what the label elides (a raw id).
   */
  let {
    /** Client coordinates of the pointer, or null when closed. May instead
     * carry `{ anchor, align }` — an element's AnchorRect (already
     * zoom-corrected via anchorOf) and 'left' for the dropdown reading, used
     * by the title caret (board #32). A plain `{x, y}` keeps the pointer
     * default: left-aligned at the pointer.
     * An optional `trigger` (the element whose click opened the menu) is not
     * "outside": its pointerdown is left alone so the trigger's own click
     * can TOGGLE the menu closed instead of closing-and-reopening it.
     * `keepTriggerClear` with an anchor caps the menu to one side of the
     * trigger, keeping a native second click reachable on short viewports. */
    at = null,
    /** @type {MenuItem[]} */ items = [],
    /** Optional heading — usually the name of what was clicked. */
    who = '',
    id = menuId,
    oncancel = () => {},
  } = $props();

  let el = $state(null);
  let w = $state(0);
  let h = $state(0);
  let cursor = $state(-1);
  const hasIcons = $derived(items.some((item) => !!item.icon));
  let restoreFocus = () => {};
  function selectItem(item) {
    if (item.disabled) return;
    restoreFocus();
    item.onselect();
    oncancel();
  }

  // Measured before it is placed: an unmeasured menu would be positioned from a
  // zero height and jump. Hidden for that one frame, exactly like the agent menu.
  //
  // Alignment defaults by ANCHOR KIND: a menu on a trigger RECT right-aligns
  // to it (the dot-menu dialect), but a menu on a POINT puts its TOP-LEFT
  // corner at the pointer — the OS convention (owner, 2026-09-07: "选项卡展示
  // 的都是点击点位是选项卡的右上点…应该都为左上角点"). A caller may still
  // say `align` explicitly either way.
  const align = $derived(at ? (at.align ?? (at.anchor ? 'right' : 'left')) : 'right');
  const heightLimit = $derived(at?.keepTriggerClear && at.anchor
    ? menuHeightLimit(at.anchor, viewBox()) : undefined);
  const pos = $derived(at
    ? menuPlacement(at.anchor ?? pointAnchor(at.x, at.y), { w, h }, viewBox(), 6, 8, align)
    : { x: 0, y: 0 });

  $effect(() => {
    if (!at) {
      cursor = -1;
      return;
    }
    if (!el) return;
    const menu = el;
    const previousFocus = document.activeElement;
    const modal = activeModal(document);
    if (!modal || modal.contains(menu)) menu.focus({ preventScroll: true });
    const restore = () => {
      const modal = activeModal(document);
      if (previousFocus instanceof HTMLElement && previousFocus.isConnected
        && (document.activeElement === document.body || menu.contains(document.activeElement))
        && (!modal || modal.contains(previousFocus))) previousFocus.focus({ preventScroll: true });
    };
    restoreFocus = restore;
    // Dismissal, all four ways. `pointerdown` rather than click, so the menu goes
    // away on the press that starts somewhere else instead of waiting for its
    // release.
    const outside = (e) => {
      if (at.trigger?.contains?.(e.target)) return;
      if (el && !el.contains(e.target)) oncancel();
    };
    const onKey = (e) => {
      const modal = activeModal(document);
      if (modal && !modal.contains(menu)) return;
      if (!menu.contains(document.activeElement) || e.isComposing || e.keyCode === 229) return;
      if (e.key === 'Tab') { oncancel(); return; }
      if (e.key === 'Escape') {
        e.preventDefault();
        oncancel();
        return;
      }
      const usable = items.filter((i) => !i.disabled);
      if (!usable.length) return;
      if (['ArrowDown', 'ArrowUp', 'Home', 'End'].includes(e.key)) {
        e.preventDefault();
        const step = e.key === 'ArrowDown' || e.key === 'Home' ? 1 : -1;
        const from = e.key === 'Home' || e.key === 'End' ? -1 : cursor;
        cursor = nextMenuIndex(from, step, items.length, i => !items[i].disabled);
        menu.querySelectorAll('button')[cursor]?.scrollIntoView?.({ block: 'nearest' });
        return;
      }
      if (e.key === 'Enter' || e.key === ' ') {
        const it = items[cursor];
        if (it && !it.disabled) {
          e.preventDefault();
          selectItem(it);
        }
      }
    };
    // Capture, so a scroll in ANY ancestor closes it — the menu is a fixed layer
    // and would otherwise stay behind while its subject scrolls away. The
    // menu's own scroll (a long list under max-height) is not "moving away".
    const onScroll = (e) => {
      if (el && e.target instanceof Node && el.contains(e.target)) return;
      if (at?.anchor && at.trigger?.isConnected && !scrollMovesTrigger(e.target, at.trigger)) return;
      oncancel();
    };
    window.addEventListener('pointerdown', outside, true);
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', oncancel);
    window.addEventListener('scroll', onScroll, true);
    return () => {
      window.removeEventListener('pointerdown', outside, true);
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', oncancel);
      window.removeEventListener('scroll', onScroll, true);
      restore();
      if (restoreFocus === restore) restoreFocus = () => {};
    };
  });
</script>

{#if at && items.length}
  <div class="ctx menu-surface menu-list pop-layer" class:ready={h > 0} bind:this={el} role="menu" tabindex="-1" {id} aria-label={who || undefined}
    aria-activedescendant={cursor >= 0 && items[cursor] ? `${id}-${cursor}` : undefined}
    style:left="{pos.x}px" style:top="{pos.y}px"
    style:max-height={heightLimit === undefined ? undefined : `${heightLimit}px`}
    style:--pop-origin={at ? popOrigin(at.anchor ?? pointAnchor(at.x, at.y), pos, align) : undefined}
    bind:offsetWidth={w} bind:offsetHeight={h}>
    {#if who}<div class="ctx-who menu-heading data" title={who}>{who}</div>{/if}
    {#each items as it, i (it.label)}
      <button class="menu-item" type="button" tabindex="-1" role={it.checked === undefined ? 'menuitem' : 'menuitemcheckbox'} aria-checked={it.checked}
        id={`${id}-${i}`}
        class:danger={it.danger} class:warn={it.warn} class:cur={i === cursor}
        disabled={it.disabled} title={it.title}
        onpointerenter={() => (cursor = i)}
        onclick={() => selectItem(it)}>
        {#if hasIcons}<span class="menu-icon" aria-hidden="true">{#if it.icon}<Icon name={it.icon} size={14} />{/if}</span>{/if}
        <span class="ctx-label menu-label">{it.label}</span>
        {#if it.hint}<span class="ctx-hint menu-hint data">{it.hint}</span>{/if}
        {#if it.checked}<span class="ctx-check menu-check" aria-hidden="true"><Icon name="check" size={12} /></span>{/if}
      </button>
    {/each}
  </div>
{/if}

<style>
  .ctx {
    position: fixed; z-index: 60; width: max-content;
    min-width: min(156px, calc(100vw / var(--ui-zoom, 1) - 16px));
    max-width: min(260px, calc(100vw / var(--ui-zoom, 1) - 16px));
    max-height: calc(100vh / var(--ui-zoom, 1) - 16px); overflow-y: auto;
    /* Invisible until measured, then grows from its anchor corner: the shared
       .pop-layer atom (app.css) owns opacity/pointer-events/transform. */
  }
</style>
