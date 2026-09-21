<script lang="ts">
  // The ONE hover card (ui/hover.svelte.ts). Mounted once in App; placed like
  // every popover (menuPlacement from the target's rect, invisible until
  // measured) and it GROWS from its anchor corner (.pop-layer). Never
  // interactive: pointer-events stay off so it cannot steal the hover that
  // opened it, and an ancestor scroll or a key closes it.
  import { hoverCard } from './hover.svelte.ts';
  import { menuPlacement, popOrigin, viewBox } from './placement.ts';
  import InfoRows from './InfoRows.svelte';

  let w = $state(0);
  let h = $state(0);
  const cur = $derived(hoverCard.current);
  const pos = $derived(cur ? menuPlacement(cur.anchor, { w, h }, viewBox(), 8, 8, cur.align) : { x: 0, y: 0 });
  const origin = $derived(cur ? popOrigin(cur.anchor, pos, cur.align) : 'top left');

  $effect(() => {
    if (!cur) return;
    const off = () => hoverCard.hide();
    window.addEventListener('scroll', off, true);
    window.addEventListener('keydown', off, true);
    window.addEventListener('resize', off);
    return () => {
      window.removeEventListener('scroll', off, true);
      window.removeEventListener('keydown', off, true);
      window.removeEventListener('resize', off);
    };
  });
</script>

{#if cur}
  <div class="hover-card menu-surface pop-layer" class:ready={h > 0} role="tooltip"
    style:left="{pos.x}px" style:top="{pos.y}px" style:--pop-origin={origin}
    bind:offsetWidth={w} bind:offsetHeight={h}>
    {#if cur.info.title}<div class="hc-title">{cur.info.title}</div>{/if}
    <InfoRows info={cur.info} />
  </div>
{/if}

<style>
  .hover-card {
    position: fixed; z-index: 70; width: max-content;
    max-width: min(300px, calc(100vw / var(--ui-zoom, 1) - 16px));
    min-width: min(120px, calc(100vw / var(--ui-zoom, 1) - 16px));
    padding: 8px 10px;
    display: flex; flex-direction: column; gap: 4px;
    font-size: var(--fs-ui); color: var(--text2);
  }
  /* Read-only: it must never take the pointer from the thing under it. */
  .hover-card.ready { pointer-events: none; }
  .hc-title { color: var(--text); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
</style>
