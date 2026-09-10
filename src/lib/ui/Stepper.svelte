<script lang="ts">
  import CommandButton from './CommandButton.svelte';
  let {
    value, label, min = -Infinity, max = Infinity, step = 1, disabled = false,
    decreaseLabel, increaseLabel, format = (n: number) => String(n), onchange,
  }: {
    value: number; label: string; min?: number; max?: number; step?: number; disabled?: boolean;
    decreaseLabel: string; increaseLabel: string; format?: (value: number) => string;
    onchange: (value: number) => void;
  } = $props();
  function change(direction: number) {
    if (disabled) return;
    const next = Math.min(max, Math.max(min, value + direction * step));
    if (next !== value) onchange(next);
  }
</script>

<div class="control-stepper" role="group" aria-label={label}>
  <CommandButton variant="icon" icon="minus" label={decreaseLabel}
    disabled={disabled || value <= min} onclick={() => change(-1)} />
  <output>{format(value)}</output>
  <CommandButton variant="icon" icon="plus" label={increaseLabel}
    disabled={disabled || value >= max} onclick={() => change(1)} />
</div>

<style>
  .control-stepper { display: inline-flex; align-items: center; gap: 4px; }
  output { min-width: 48px; text-align: center; font: var(--fs-body)/var(--control-line-height) var(--font-mono); color: var(--text); }
</style>
