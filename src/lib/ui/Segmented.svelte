<script lang="ts" generics="T extends string | number | boolean">
  // The ONE segmented control (design-language.md §3 "Segmented rows").
  // Preferences used to spell seven of these by hand — three buttons, a
  // class:active each — and the travelling highlight (motion.md §1.14) needs
  // a key and a pill PER INSTANCE, so the dialect became a component: the
  // pill (`.slide-pill`, wash + ring) glides behind the chosen option on
  // `--t-move`; the buttons keep only their ink, which cross-fades on
  // `--t-fast` (`.state-ctl`). Values may be strings, numbers or booleans
  // (On/Off rows pass `true`/`false`).
  import { slideIndicator } from './indicator.ts';

  let {
    options,
    value,
    onchange,
    ariaLabel = undefined,
  }: {
    options: { value: T; label: string }[];
    value: T;
    onchange: (value: T) => void;
    /** The row's name for a screen reader (the visible label sits beside it). */
    ariaLabel?: string;
  } = $props();
</script>

<div class="segmented" role="group" aria-label={ariaLabel} use:slideIndicator={{ key: value, active: '.active' }}>
  <span class="slide-pill" aria-hidden="true"></span>
  {#each options as o (String(o.value))}
    <button type="button" class="state-ctl" class:active={o.value === value} aria-pressed={o.value === value}
      onclick={() => onchange(o.value)}>{o.label}</button>
  {/each}
</div>

<style>
  /* position: relative — the pill's containing block; the buttons sit above it. */
  .segmented { position: relative; display: flex; gap: 4px; flex-shrink: 0; }
  /* App control dialect: --ui-radius-control squares like every chip-btn /
     icon-btn (the 999px pills were Preferences' private language — owner,
     2026-08-25: "和其他页面画风不一样"). */
  .segmented button {
    position: relative; z-index: 1;
    height: var(--ui-control-height); padding: 3px 8px;
    border: 1px solid var(--border2); border-radius: var(--ui-radius-control);
    background: transparent; color: var(--text3);
    font-size: var(--ui-font-control); white-space: nowrap; cursor: pointer;
  }
  /* The chosen option keeps the accent INK only: the wash and the ring are the
     pill's, so they travel instead of switching on in place. */
  .segmented button.active { color: var(--accent); border-color: transparent; }
  .segmented button:active { border-color: var(--accent); color: var(--accent); }
  @media (max-width: 760px) { .segmented button { min-height: 32px; } }
  @media (max-width: 420px) { .segmented button { flex: 1; } }
</style>
