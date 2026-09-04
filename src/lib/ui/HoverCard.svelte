<script lang="ts">
  // The ONE hover card (ui/hover.svelte.ts). Mounted once in App; placed like
  // every popover (menuPlacement from the target's rect, invisible until
  // measured) and it GROWS from its anchor corner (.pop-layer). Never
  // interactive: pointer-events stay off so it cannot steal the hover that
  // opened it, and an ancestor scroll or a key closes it.
  import { hoverCard } from './hover.svelte.ts';
  import { menuPlacement, popOrigin, viewBox } from './placement.ts';

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
  <div class="hover-card pop-layer" class:ready={h > 0} role="tooltip"
    style:left="{pos.x}px" style:top="{pos.y}px" style:--pop-origin={origin}
    bind:clientWidth={w} bind:clientHeight={h}>
    {#if cur.info.title}<div class="hc-title">{cur.info.title}</div>{/if}
    {#if cur.info.text}<div class="hc-text">{cur.info.text}</div>{/if}
    {#if cur.info.lines?.length}
      <dl class="hc-rows">
        {#each cur.info.lines as l (l.label)}
          <dt>{l.label}</dt><dd class:ok={l.tone === 'ok'} class:warn={l.tone === 'warn'} class:danger={l.tone === 'danger'} class:accent={l.tone === 'accent'}>{l.value}</dd>
        {/each}
      </dl>
    {/if}
    {#if cur.info.note}<div class="hc-note">{cur.info.note}</div>{/if}
  </div>
{/if}

<style>
  .hover-card {
    position: fixed; z-index: 70; max-width: 300px; min-width: 120px;
    background: var(--bg); border: 1px solid var(--border); border-radius: var(--ui-radius-panel);
    box-shadow: 0 10px 30px rgba(0, 0, 0, 0.4); padding: 8px 10px;
    display: flex; flex-direction: column; gap: 4px;
    font-size: var(--fs-ui); color: var(--text2);
  }
  /* Read-only: it must never take the pointer from the thing under it. */
  .hover-card.ready { pointer-events: none; }
  .hc-title { color: var(--text); font-weight: 600; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hc-text { color: var(--text2); line-height: 1.35; }
  .hc-rows { display: grid; grid-template-columns: auto 1fr; gap: 2px 10px; margin: 0; }
  .hc-rows dt { color: var(--text3); font-size: var(--fs-meta); white-space: nowrap; }
  .hc-rows dd { margin: 0; color: var(--text2); font-size: var(--fs-meta); font-family: ui-monospace, Menlo, monospace; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hc-rows dd.ok { color: var(--status-ok); }
  .hc-rows dd.warn { color: var(--status-warn); }
  .hc-rows dd.danger { color: var(--danger); }
  .hc-rows dd.accent { color: var(--accent); }
  .hc-note { color: var(--text3); font-size: var(--fs-meta); }
</style>
