<!--
  Unified chip used for every "switchable thing" in the app:
  - MRU chips on the Sessions page
  - Current-session windows + cross-session chips in the Terminal top bar
  - Collapsed-state single chip in the Terminal top bar

  One component, one visual language. All size / spacing / color are
  defined here, not in the consuming pages.

  Variants:
    variant="default" — the idle state (muted border, transparent bg)
    variant="active"  — currently selected (accent border + accent bg)
    variant="add"     — the `+ new window` ghost button (dashed border)

  Slots: takes either an agent entry (renders icon) or a `label` prop
  (renders monospace text). Agents take priority.
-->
<script>
  import Icon from './Icon.svelte';

  let {
    agent = null,         // Agent entry from agents.js: { tag, icon, iconSize }
    label = '',           // Monospace text fallback (command name, session name)
    variant = 'default',  // 'default' | 'active' | 'add'
    iconName = '',        // Lucide icon name, used by 'add' and collapsed chip
    chevron = '',         // '' | 'up' | 'down' — small indicator appended at end
    title = '',           // HTML title attribute
    onclick = () => {},
  } = $props();
</script>

<button
  class="chip chip-{variant}"
  title={title || label || agent?.tag || ''}
  onclick={(e) => onclick(e)}
>
  {#if agent}
    <img class="chip-icon" class:claude={agent.tag === 'Claude'} src={agent.icon} alt={agent.tag} />
  {:else if iconName}
    <Icon name={iconName} size={12} />
  {/if}
  {#if label}
    <span class="chip-label">{label}</span>
  {/if}
  {#if chevron}
    <Icon name="chevron-{chevron}" size={10} />
  {/if}
</button>

<style>
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 5px;
    /* 24 px chip with line-height 1.3 leaves ~4 px slack for g/p/y
       descenders at 11 px font. Don't force line-height: 1 — glyphs clip. */
    padding: 3px 8px;
    height: 24px;
    border: 1px solid var(--border2);
    border-radius: 999px;
    background: var(--input-bg);
    color: var(--text2);
    font-size: 11px;
    font-weight: 500;
    line-height: 1.3;
    cursor: pointer;
    flex-shrink: 0;
    white-space: nowrap;
    max-width: 140px;
    box-sizing: border-box;
    -webkit-tap-highlight-color: transparent;
    transition: border-color 0.15s ease, background 0.15s ease, color 0.15s ease;
  }
  .chip:active {
    background: var(--accent-bg);
    border-color: var(--accent);
    color: var(--accent);
  }

  .chip-active {
    background: var(--accent-bg);
    border-color: var(--accent);
    color: var(--accent);
  }
  .chip-active .chip-icon { opacity: 1; }

  .chip-add {
    background: transparent;
    border-style: dashed;
    color: var(--text3);
    padding: 3px 9px;
  }
  .chip-add:active {
    color: var(--accent);
    border-color: var(--accent);
    background: var(--accent-bg);
  }

  .chip-icon {
    width: 12px;
    height: 12px;
    flex-shrink: 0;
    opacity: 0.85;
  }
  .chip-icon.claude {
    width: 14px;
    height: 14px;
    filter: brightness(0.9);
  }

  .chip-label {
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--font-ui);
  }
</style>
