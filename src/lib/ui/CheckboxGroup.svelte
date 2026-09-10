<script lang="ts">
  import Icon from './Icon.svelte';
  let { label, options, value = [], disabled = false, onchange }:
    { label: string; options: readonly { value: string; label: string }[]; value?: readonly string[];
      disabled?: boolean; onchange: (value: string[]) => void } = $props();
  function change(option: string, checked: boolean) {
    if (disabled) return;
    onchange(checked ? value.includes(option) ? [...value] : [...value, option] : value.filter(v => v !== option));
  }
</script>

<fieldset class="checkbox-group" {disabled}>
  <legend>{label}</legend>
  <div class="checkbox-options">
    {#each options as option (option.value)}
      <label class="checkbox-option">
        <input type="checkbox" checked={value.includes(option.value)}
          onchange={(e) => {
            const requested = e.currentTarget.checked;
            e.currentTarget.checked = value.includes(option.value);
            change(option.value, requested);
          }} />
        <span class="check-box" aria-hidden="true">
          {#if value.includes(option.value)}<Icon name="check" size={12} />{/if}
        </span>
        <span>{option.label}</span>
      </label>
    {/each}
  </div>
</fieldset>

<style>
  .checkbox-group { margin: 0; padding: 0; border: 0; min-width: 0; }
  legend { padding: 0; margin-bottom: 8px; color: var(--text); font: 600 var(--fs-ui)/1.4 var(--font-ui); }
  .checkbox-options { display: flex; flex-wrap: wrap; gap: 0 16px; }
  .checkbox-option {
    position: relative; display: inline-flex; align-items: center; gap: 8px;
    min-height: var(--control-height); min-width: var(--control-height); max-width: 100%;
    color: var(--text); font: 400 var(--fs-body)/1.4 var(--font-ui); cursor: pointer;
  }
  .checkbox-option > span:last-child { overflow-wrap: anywhere; }
  input { position: absolute; width: 1px; height: 1px; margin: 0; opacity: 0; }
  .check-box {
    display: grid; place-items: center; flex: none; box-sizing: border-box;
    width: 16px; height: 16px; border: 1px solid var(--control-border); border-radius: 4px;
    background: var(--input-bg); color: var(--accent-fill-ink);
    transition: background var(--t-fast), border-color var(--t-fast);
  }
  input:checked + .check-box { background: var(--accent-fill); border-color: var(--accent-fill); }
  input:focus-visible + .check-box { outline: 2px solid var(--accent); outline-offset: 2px; }
  input:disabled ~ span { opacity: var(--control-disabled-opacity); cursor: default; }
  @media (prefers-reduced-motion: reduce) { .check-box { transition: none; } }
</style>
