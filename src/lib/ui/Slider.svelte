<script lang="ts">
  import CommandButton from './CommandButton.svelte';
  let { value, min, max, step, label, resetLabel, defaultValue, disabled = false,
    format = (n: number) => String(n), onchange }:
    { value: number; min: number; max: number; step: number; label: string; resetLabel: string;
      defaultValue: number; disabled?: boolean; format?: (value: number) => string;
      onchange: (value: number) => void } = $props();
</script>

<div class="control-slider">
  <!-- Native ranges sanitize value immediately, so constraints come first. -->
  <input type="range" {min} {max} {step} {value} {disabled} aria-label={label}
    oninput={(e) => {
      const requested = e.currentTarget.valueAsNumber;
      e.currentTarget.value = String(value);
      if (!disabled) onchange(requested);
    }} />
  <output>{format(value)}</output>
  <CommandButton variant="icon" icon="undo" label={resetLabel}
    disabled={disabled || value === defaultValue} onclick={() => onchange(defaultValue)} />
</div>

<style>
  .control-slider { display: flex; align-items: center; gap: 8px; min-width: 0; }
  input { flex: 1; min-width: 0; height: var(--control-height); margin: 0; accent-color: var(--accent-ink); }
  output { min-width: 48px; color: var(--text); text-align: right; font: var(--fs-body)/var(--control-line-height) var(--font-mono); }
</style>
