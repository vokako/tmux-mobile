<script>
  // The ONE read-only info body — free text, label/value rows (each value
  // optionally toned like a status dot), and a quiet note. HoverCard shows it
  // on hover for pointer devices; ContextMenu shows the SAME body under its
  // heading when a menu opens by touch, where hover does not exist (owner,
  // 2026-09-21, board #223: "手机上因为没有悬停窗口 所以选项卡里给我展示一下
  // agent信息"). One renderer, so the two surfaces cannot drift.
  /** @type {{ info: { text?: string, lines?: {label: string, value: string, tone?: string}[], note?: string } }} */
  let { info } = $props();
</script>

{#if info.text}<div class="ir-text">{info.text}</div>{/if}
{#if info.lines?.length}
  <dl class="ir-rows">
    {#each info.lines as l (l.label)}
      <dt>{l.label}</dt><dd class:ok={l.tone === 'ok'} class:warn={l.tone === 'warn'} class:danger={l.tone === 'danger'} class:accent={l.tone === 'accent'}>{l.value}</dd>
    {/each}
  </dl>
{/if}
{#if info.note}<div class="ir-note">{info.note}</div>{/if}

<style>
  .ir-text { color: var(--text2); line-height: 1.35; }
  .ir-rows { display: grid; grid-template-columns: auto 1fr; gap: 2px 10px; margin: 0; }
  .ir-rows dt { color: var(--text3); font-size: var(--fs-meta); white-space: nowrap; }
  .ir-rows dd { margin: 0; color: var(--text2); font-size: var(--fs-meta); font-family: var(--font-mono); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .ir-rows dd.ok { color: var(--status-ok); }
  .ir-rows dd.warn { color: var(--status-warn); }
  .ir-rows dd.danger { color: var(--danger); }
  .ir-rows dd.accent { color: var(--accent); }
  .ir-note { color: var(--text3); font-size: var(--fs-meta); }
</style>
