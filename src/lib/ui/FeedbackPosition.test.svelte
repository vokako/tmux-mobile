<script lang="ts">
  import { feedbackPosition } from './feedback-position.ts';
  let { ready }: { ready: (hide: () => void) => void } = $props();
  let show = $state(true);
  let bounds = $state<HTMLElement | null>(null);
  let scrollport = $state<HTMLElement | null>(null);
  let trigger = $state<HTMLButtonElement | null>(null);
  $effect(() => ready(() => show = false));
</script>
{#if show}
  <div class="scrollport" bind:this={scrollport}>
    <div class="bounds" bind:this={bounds}>
      <button bind:this={trigger}>Copy</button>
      <div class="pop-layer" use:feedbackPosition={{ trigger, bounds, scrollport }}>Copy failed</div>
    </div>
  </div>
{/if}
