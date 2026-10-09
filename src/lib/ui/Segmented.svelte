<script lang="ts" generics="T extends string | number | boolean">
  // The ONE segmented control (design-language.md §3 "Segmented rows").
  // Preferences used to spell seven of these by hand — three buttons, a
  // class:active each — and the travelling highlight (motion.md §1.14) needs
  // a key and a pill PER INSTANCE, so the dialect became a component: the
  // pill (`.slide-pill.control`, compact neutral surface) glides on
  // `--t-move`; the buttons keep only their ink, which cross-fades on
  // `--t-fast` (`.state-ctl`). Values may be strings, numbers or booleans
  // (On/Off rows pass `true`/`false`).
  import { slideIndicator } from './indicator.ts';
  import Icon from './Icon.svelte';

  let {
    options,
    value,
    onchange,
    ariaLabel = undefined,
    disabled = false,
  }: {
    /** An option with an `icon` draws that glyph instead of its text, and the
     * `label` becomes its accessible name (board #326: two edges the owner
     * wanted as icons — "上面的按钮不用写'bottom'之类的文字了 你用两个小图标去
     * 做状态切换"). Icons and words do not mix inside one row: the pill would
     * travel between cells of two different widths. */
    options: { value: T; label: string; icon?: string }[];
    value: T;
    onchange: (value: T) => void;
    /** The row's name for a screen reader (the visible label sits beside it). */
    ariaLabel?: string;
    disabled?: boolean;
  } = $props();
  const iconic = $derived(options.every((o) => !!o.icon));
</script>

<div class="segmented" class:iconic role="group" aria-label={ariaLabel} use:slideIndicator={{ key: value, active: '.active' }}>
  <span class="slide-pill control" aria-hidden="true"></span>
  {#each options as o (String(o.value))}
    <button type="button" class="state-ctl" class:active={o.value === value} aria-pressed={o.value === value} {disabled}
      aria-label={o.icon ? o.label : undefined} title={undefined}
      onclick={() => { if (!disabled) onchange(o.value); }}
    >{#if o.icon}<Icon name={o.icon} size={14} />{:else}{o.label}{/if}</button>
  {/each}
</div>

<style>
  /* position: relative — the pill's containing block; the buttons sit above it. */
  .segmented {
    position: relative; display: grid; grid-auto-flow: column; grid-auto-columns: 1fr; gap: 0; flex-shrink: 0;
    height: var(--control-height); border-radius: var(--control-paint-radius);
  }
  .segmented::before {
    content: ''; position: absolute; inset: var(--control-paint-inset) 0;
    border-radius: inherit; background: var(--control-surface); pointer-events: none;
  }
  .segmented button {
    position: relative; z-index: 1;
    height: var(--control-height); min-width: var(--control-height); padding: 0 8px;
    border: 0; border-radius: var(--control-paint-radius);
    background: transparent; color: var(--text);
    font: var(--fs-body)/var(--control-line-height) var(--font-ui);
    white-space: nowrap; cursor: pointer;
  }
  /* Weight complements the single moving surface when color is unavailable. */
  .segmented button.active { font-weight: 600; }
  /* An icon row: square cells (the glyph carries the meaning, the accessible
     name carries the words) and the ink families the rest of the app uses for
     icon-only controls — at rest muted, chosen is full ink. */
  .segmented.iconic button { display: grid; place-items: center; padding: 0; color: var(--text2); }
  .segmented.iconic button.active { color: var(--text); }
  .segmented button:active:not(:disabled) { color: var(--accent-ink); }
  .segmented button:disabled { opacity: var(--control-disabled-opacity); cursor: default; }
</style>
