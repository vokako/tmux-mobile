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
    agents = [],          // Aggregated entries: [{ agent, count }]
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
  title={title || (agents.length
    ? `${label ? `${label} · ` : ''}${agents.map(item => `${item.agent.tag}${item.count > 1 ? ` ×${item.count}` : ''}`).join(', ')}`
    : label || agent?.tag || '')}
  onclick={(e) => onclick(e)}
>
  {#if agents.length}
    <span class="chip-agents">
      {#each agents as item (item.agent.tag)}
        <span class="chip-agent">
          <img class="chip-icon" class:claude={item.agent.tag === 'Claude'} src={item.agent.icon} alt={item.agent.tag} />
          {#if item.count > 1}<span class="chip-count">{item.count}</span>{/if}
        </span>
      {/each}
    </span>
  {:else if agent}
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
  .chip-agents { display: inline-flex; align-items: center; gap: 3px; overflow: visible; }
  .chip-agent { position: relative; display: inline-flex; flex-shrink: 0; }
  .chip-count {
    position: absolute;
    top: -5px;
    right: -5px;
    min-width: 11px;
    height: 11px;
    padding: 0 2px;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    border-radius: 999px;
    background: var(--accent);
    color: var(--bg);
    font-size: 7px;
    font-weight: 700;
    line-height: 1;
    font-variant-numeric: tabular-nums;
    box-shadow: 0 0 0 1px var(--input-bg);
  }

  .chip-label {
    overflow: hidden;
    text-overflow: ellipsis;
    font-family: var(--font-ui);
  }
</style>
