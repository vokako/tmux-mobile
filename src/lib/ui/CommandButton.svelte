<script lang="ts">
  import Icon from './Icon.svelte';
  import { hoverInfo } from './hover.ts';

  type Props = {
    label: string;
    variant?: 'primary' | 'secondary' | 'icon' | 'danger';
    iconOnly?: boolean;
    pressed?: boolean;
    expanded?: boolean;
    controls?: string;
    disabled?: boolean;
    destructiveConfirm?: boolean;
    onclick?: (event: MouseEvent) => void;
    element?: HTMLButtonElement | null;
  } & ({ pending?: false; icon?: string } | { pending: boolean; icon: string });
  let {
    label, icon = '', variant = 'secondary', iconOnly = false,
    pressed, expanded, controls,
    disabled = false, pending = false, destructiveConfirm = false,
    onclick = () => {}, element = $bindable(null),
  }: Props = $props();
  const compact = $derived(iconOnly || variant === 'icon');
  const solid = $derived(variant === 'primary' || (variant === 'danger' && destructiveConfirm));
  const engaged = $derived(variant === 'icon' && (pressed === true || expanded === true));
</script>

<button type="button" class="command-button"
  class:primary={solid} class:secondary={variant === 'secondary'}
  class:danger={variant === 'danger'} class:icon-only={compact} class:solid class:pending class:engaged
  disabled={disabled || pending} aria-label={label} aria-busy={pending || undefined}
  aria-pressed={pressed} aria-expanded={expanded} aria-controls={controls}
  bind:this={element}
  use:hoverInfo={() => compact ? { title: label } : null}
  onclick={(event) => { if (!disabled && !pending) onclick(event); }}>
  {#if icon}
    <span class="command-icon" class:spinning={pending} aria-hidden="true">
      <Icon name={pending ? 'refresh' : icon} />
    </span>
  {/if}
  {#if !compact}<span class="command-label">{label}</span>{/if}
</button>

<style>
  .command-button {
    display: inline-flex; align-items: center; justify-content: center; flex: none; gap: 8px;
    box-sizing: border-box; height: var(--control-height); min-width: var(--control-height);
    padding: 0 12px; border: 1px solid transparent; border-radius: var(--ui-radius-control);
    background: transparent; color: var(--text); cursor: pointer; letter-spacing: 0;
    font: 500 var(--fs-ui)/var(--control-line-height) var(--font-display);
    transition: background var(--t-fast), color var(--t-fast), box-shadow var(--t-fast);
    -webkit-tap-highlight-color: transparent;
  }
  .secondary { background: var(--surface); border-color: var(--control-border); }
  .icon-only { width: var(--control-height); padding: 0; }
  .danger { color: var(--danger-ink); }
  .solid { background: var(--accent-fill); color: var(--accent-fill-ink); }
  .solid.danger { background: var(--danger-fill); }
  .command-button:hover:not(:disabled) { background: var(--surface2); }
  .command-button.solid:hover:not(:disabled) {
    background: var(--accent-fill); box-shadow: inset 0 0 0 100px color-mix(in srgb, var(--control-overlay-light) 4%, transparent);
  }
  .command-button.solid.danger:hover:not(:disabled) { background: var(--danger-fill); }
  .command-button:active:not(:disabled) { background: var(--accent-bg); }
  .command-button.solid:active:not(:disabled) {
    background: var(--accent-fill); box-shadow: inset 0 0 0 100px color-mix(in srgb, var(--control-overlay-dark) 6%, transparent);
  }
  .command-button.solid.danger:active:not(:disabled) { background: var(--danger-fill); }
  .command-button.engaged,
  .command-button.engaged:hover:not(:disabled),
  .command-button.engaged:active:not(:disabled) {
    background: var(--accent-bg); color: var(--accent-ink);
  }
  .command-button:disabled:not(.pending) { opacity: var(--control-disabled-opacity); cursor: default; }
  .command-button.pending { opacity: 1; cursor: progress; }
  .command-button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .command-icon { display: inline-flex; width: var(--control-icon-size); height: var(--control-icon-size); flex: none; }
  .command-icon :global(svg) { width: 100%; height: 100%; }
  .command-label { white-space: nowrap; }
  .spinning { animation: spin 0.6s linear infinite; }
  @media (prefers-reduced-motion: reduce) {
    .command-button { transition: none; }
    .spinning { animation: none; }
  }
</style>
