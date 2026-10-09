<script lang="ts">
  // The staged-attachment strip (board #25, moved out of Composer by #329 so
  // the Board's editors show the SAME chips): an image is its thumbnail with
  // its token number and ✕, a file a named chip, a failure a red chip with
  // its reason. `lead` renders first in the row (the composer's reply chip,
  // board #290) and wears the same chip species: the .pend-* rules below
  // reach it through the row.
  import type { Snippet } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import type { Attachment } from './attachments.svelte.ts';

  let { pending = [], onremove = () => {}, onpreview = () => {}, lead }: {
    pending?: Attachment[];
    onremove?: (i: number) => void;
    onpreview?: (url: string) => void;
    lead?: Snippet;
  } = $props();
</script>

{#if pending.length || lead}
  <div class="pend-row">
    {@render lead?.()}
    {#each pending as a, i (a.key)}
      {#if a.error}
        <span class="pend-chip err appear-pop" title={`${a.name} — ${a.error}`}>
          <Icon name="info" size={12} />
          <span class="pend-name">{a.name}</span>
          <span class="pend-why">{a.error}</span>
          <button class="pend-x" aria-label={t('hubRemoveAttachment')}
            onclick={() => onremove(i)}>
            <Icon name="x" size={11} />
          </button>
        </span>
      {:else if a.kind === 'image'}
        <span class="pend-thumb appear-pop" title={`[img:${a.n}] ${a.name}`}>
          <button class="pend-view" aria-label={a.name}
            onclick={() => onpreview(a.thumb)}>
            <img src={a.thumb} alt={a.name} />
          </button>
          <span class="pend-n">{a.n}</span>
          <button class="pend-x on-img" aria-label={t('hubRemoveAttachment')}
            onclick={() => onremove(i)}>
            <Icon name="x" size={10} />
          </button>
        </span>
      {:else}
        <span class="pend-chip appear-pop" title={`[file:${a.n}] ${a.path}`}>
          <Icon name="file" size={12} />
          <span class="pend-name">{a.name}</span>
          <button class="pend-x" aria-label={t('hubRemoveAttachment')}
            onclick={() => onremove(i)}>
            <Icon name="x" size={11} />
          </button>
        </span>
      {/if}
    {/each}
  </div>
{/if}

<style>
  .pend-row { display: flex; flex-wrap: wrap; gap: 6px; padding-block: 5px; }
  .pend-row :global(.pend-chip) {
    display: inline-flex; align-items: center; gap: 5px; max-width: 100%; padding: 3px 7px;
    border: 1px solid var(--border); border-radius: 6px; background: var(--surface);
    color: var(--text2); font-size: var(--fs-sub);
  }
  .pend-row :global(.pend-chip.err) { color: var(--status-danger); border-color: var(--status-danger); }
  .pend-row :global(.pend-name) { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pend-row :global(.pend-why) { max-width: 15em; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pend-row :global(.pend-x) {
    display: grid; place-items: center; width: var(--control-height); height: var(--control-height);
    flex: none; padding: 0; border: 0; border-radius: var(--ui-radius-control); background: transparent; color: inherit;
  }
  .pend-thumb { display: flex; align-items: center; position: relative; }
  .pend-view { padding: 0; border: 0; background: transparent; }
  .pend-view img { display: block; max-height: 48px; max-width: 100px; }
  .pend-n { position: absolute; left: 2px; top: 2px; font-size: var(--fs-micro); }
</style>
