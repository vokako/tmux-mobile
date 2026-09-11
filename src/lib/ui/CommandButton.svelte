<script lang="ts">
  import Icon from './Icon.svelte';
  import { hoverInfo } from './hover.ts';

  type Props = {
    label: string;
    variant?: 'primary' | 'secondary' | 'icon' | 'danger' | 'warn';
    iconOnly?: boolean;
    pressed?: boolean;
    expanded?: boolean;
    /** The disclosure control STANDS INSIDE the region it discloses (the Hub
     * sidebar's collapse, board #174): the region being visible is the whole
     * signal, so the button keeps its rest look instead of the engaged wash a
     * disclosure of content elsewhere wears (the roster's). */
    inside?: boolean;
    controls?: string;
    disabled?: boolean;
    destructiveConfirm?: boolean;
    onclick?: (event: MouseEvent) => void;
    element?: HTMLButtonElement | null;
  } & ({ pending?: false; icon?: string } | { pending: boolean; icon: string });
  let {
    label, icon = '', variant = 'secondary', iconOnly = false,
    pressed, expanded, inside = false, controls,
    disabled = false, pending = false, destructiveConfirm = false,
    onclick = () => {}, element = $bindable(null),
  }: Props = $props();
  const compact = $derived(iconOnly || variant === 'icon');
  const solid = $derived(variant === 'primary' || (variant === 'danger' && destructiveConfirm));
  const engaged = $derived(variant === 'icon' && (pressed === true || (expanded === true && !inside)));
  // A disclosure chevron turns instead of swapping (motion.md 4) — vertical
  // for a roster, horizontal for a side partition (board #174).
  const disclosure = $derived(!pending && expanded !== undefined && icon.startsWith('chevron-'));
</script>

<button type="button" class="command-button"
  class:primary={solid} class:secondary={variant === 'secondary'}
  class:danger={variant === 'danger'} class:warn={variant === 'warn'} class:icon-only={compact} class:solid class:pending class:engaged
  disabled={disabled || pending} aria-label={label} aria-busy={pending || undefined}
  aria-pressed={pressed} aria-expanded={expanded} aria-controls={controls}
  bind:this={element}
  use:hoverInfo={() => compact ? { title: label } : null}
  onclick={(event) => { if (!disabled && !pending) onclick(event); }}>
  {#if icon}
    <span class="command-icon" class:spinning={pending} class:flip={disclosure} class:on={disclosure && expanded} aria-hidden="true">
      <Icon name={pending ? 'refresh' : icon} />
    </span>
  {/if}
  {#if !compact}<span class="command-label">{label}</span>{/if}
</button>

<style>
  .command-button {
    position: relative; display: inline-flex; align-items: center; justify-content: center; flex: none; gap: 8px;
    box-sizing: border-box; height: var(--control-height); min-width: var(--control-height);
    padding: 0 12px; border: 0; border-radius: var(--ui-radius-pill);
    background: transparent; color: var(--text); cursor: pointer; letter-spacing: 0;
    font: 500 var(--fs-ui)/var(--control-line-height) var(--font-display);
    transition: color var(--t-fast);
    -webkit-tap-highlight-color: transparent;
  }
  .command-button::before {
    content: ''; position: absolute; inset: var(--control-paint-inset) 0;
    border-radius: inherit; background: var(--command-paint, transparent); pointer-events: none;
    box-shadow: inset 0 0 0 100px var(--command-overlay, transparent);
    transition: background var(--t-fast), box-shadow var(--t-fast);
  }
  .secondary { --command-paint: var(--control-surface); }
  .icon-only { width: var(--control-height); padding: 0; }
  .icon-only::before { inset: var(--control-paint-inset); }
  .danger { color: var(--danger-ink); }
  .warn { color: color-mix(in srgb, var(--status-warn) 80%, var(--text)); }
  .command-button.warn::before { background: none; box-shadow: none; }
  .solid { --command-paint: var(--accent-fill); color: var(--accent-fill-ink); }
  .solid.danger { --command-paint: var(--danger-fill); }
  .command-button:hover:not(:disabled) { --command-paint: var(--control-hover); }
  .command-button.solid:hover:not(:disabled) {
    --command-paint: var(--accent-fill);
    --command-overlay: color-mix(in srgb, var(--control-overlay-light) 4%, transparent);
  }
  .command-button.solid.danger:hover:not(:disabled) { --command-paint: var(--danger-fill); }
  .command-button:active:not(:disabled) { --command-paint: var(--control-surface); }
  .command-button.solid:active:not(:disabled) {
    --command-paint: var(--accent-fill);
    --command-overlay: color-mix(in srgb, var(--control-overlay-dark) 6%, transparent);
  }
  .command-button.solid.danger:active:not(:disabled) { --command-paint: var(--danger-fill); }
  .command-button.engaged,
  .command-button.engaged:hover:not(:disabled),
  .command-button.engaged:active:not(:disabled) {
    --command-paint: var(--control-surface); color: var(--accent-ink);
  }
  .command-button:disabled:not(.pending) { opacity: var(--control-disabled-opacity); cursor: default; }
  .command-button.pending { opacity: 1; cursor: progress; }
  .command-button:focus-visible { outline: none; }
  .command-button:focus-visible::before { outline: 2px solid var(--accent-ink); outline-offset: 2px; }
  .command-icon, .command-label { position: relative; z-index: 1; }
  .command-icon { display: inline-flex; width: var(--control-icon-size); height: var(--control-icon-size); flex: none; }
  .command-icon :global(svg) { width: 100%; height: 100%; }
  .command-label { white-space: nowrap; }
  .spinning { animation: spin 0.6s linear infinite; }
  @media (prefers-reduced-motion: reduce) {
    .command-button, .command-button::before { transition: none; }
    .spinning { animation: none; }
  }
</style>
