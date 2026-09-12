<script lang="ts">
  import type { Snippet } from 'svelte';
  import Icon from './Icon.svelte';
  import CommandButton from './CommandButton.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import type { FeedbackValue } from './feedback-lifetime.ts';

  let { value, ondismiss, actions }: {
    value: FeedbackValue | null;
    ondismiss?: () => void;
    actions?: Snippet;
  } = $props();
  const percent = $derived(typeof value?.progress === 'number' && Number.isFinite(value.progress)
    ? Math.max(0, Math.min(100, Math.round(value.progress))) : null);
</script>

{#if value}
  <div class="operation-feedback menu-surface" class:error={value.kind === 'error'}
    role={value.kind === 'error' ? 'alert' : 'status'}>
    <span class="feedback-icon" class:spinning={value.kind === 'progress' && percent === null} aria-hidden="true">
      <Icon name={value.kind === 'error' ? 'info' : value.kind === 'progress' ? 'refresh' : 'check'} />
    </span>
    <div class="feedback-body">
      <span class="feedback-message" class:config-error={value.kind === 'error'}>{value.message}</span>
      {#if value.detail}<span class="feedback-detail">{value.detail}</span>{/if}
      {#if value.kind === 'progress'}
        {#if percent === null}
          <span role="progressbar" aria-label={value.message} aria-valuemin="0" aria-valuemax="100"></span>
        {:else}
          <span class="feedback-progress">
            <progress value={percent} max="100" aria-label={value.message}></progress>
            <span class="feedback-percent">{percent}%</span>
          </span>
        {/if}
      {/if}
    </div>
    {#if actions || ondismiss}
      <div class="feedback-actions">
        {@render actions?.()}
        {#if ondismiss}<CommandButton variant="icon" icon="x" label={t('close')} onclick={ondismiss} />{/if}
      </div>
    {/if}
  </div>
{/if}

<style>
  .operation-feedback {
    box-sizing: border-box; display: flex; flex-wrap: wrap; align-items: center; gap: calc(2 * var(--ui-gap));
    padding: calc(2 * var(--ui-gap)) calc(3 * var(--ui-gap));
    font: var(--fs-ui)/1.5 var(--font-ui); color: var(--text);
    max-width: 100%; max-height: var(--feedback-max-height, none); overflow: auto;
  }
  .feedback-icon { display: inline-flex; flex: none; width: var(--control-icon-size); height: var(--control-icon-size); }
  .feedback-body { min-width: min(100%, calc(2 * var(--control-height))); flex: 1; display: flex; flex-direction: column; }
  .feedback-message, .feedback-detail { overflow-wrap: anywhere; }
  .feedback-detail { color: var(--text2); font: var(--fs-sub)/1.5 var(--font-mono); }
  .error .feedback-icon { color: var(--danger-ink); }
  .feedback-actions { display: flex; flex-wrap: wrap; flex: none; max-width: 100%; margin-inline-start: auto; align-items: center; gap: var(--ui-gap); }
  .feedback-progress { display: flex; align-items: center; gap: calc(2 * var(--ui-gap)); }
  .feedback-progress progress {
    appearance: none; flex: 1; min-width: 0; height: var(--ui-gap); border: none;
    border-radius: var(--ui-radius-pill); overflow: hidden; background: var(--surface);
  }
  progress::-webkit-progress-bar { background: var(--surface); }
  progress::-webkit-progress-value { background: var(--accent); }
  progress::-moz-progress-bar { background: var(--accent); }
  .feedback-percent { flex: none; width: 4ch; text-align: end; font: var(--fs-sub)/1.5 var(--font-mono); }
  .spinning { animation: spin 0.6s linear infinite; }
  @media (prefers-reduced-motion: reduce) { .spinning { animation: none; } }
</style>
