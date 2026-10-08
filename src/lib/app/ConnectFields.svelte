<script lang="ts">
  // The connection FIELDS (address with its recents, token, tmux socket),
  // shared by the disconnected connect page and the Add server dialog (board
  // 315), so the two ways to name a server cannot drift: one field set, one
  // recents list, one way to forget a recent.
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { flip } from 'svelte/animate';
  import { moveMs } from '../ui/motion.ts';

  type HistoryEntry = { address: string; token: string };

  let {
    address = $bindable(''), token = $bindable(''), socket = $bindable(''),
    history = $bindable<HistoryEntry[]>([]),
    /** Focus the address field when mounted (the dialog). */
    autofocus = false,
    onenter = () => {},
  }: {
    address?: string; token?: string; socket?: string; history?: HistoryEntry[];
    autofocus?: boolean; onenter?: () => void;
  } = $props();

  let showToken = $state(false);
  let showHistory = $state(false);

  function focusIf(el: HTMLInputElement) { if (autofocus) el.focus(); }
  function enter(e: KeyboardEvent) {
    if (e.key !== 'Enter' || e.isComposing || e.keyCode === 229) return;
    e.preventDefault();
    onenter();
  }
  function forget(addr: string) {
    history = history.filter(x => x.address !== addr);
    localStorage.setItem('tmux_address_history', JSON.stringify(history));
    try {
      const machines = JSON.parse(localStorage.getItem('tmux_machines') || '{}');
      for (const mid in machines) {
        machines[mid] = machines[mid].filter((u: string) => u !== addr);
        if (!machines[mid].length) delete machines[mid];
      }
      localStorage.setItem('tmux_machines', JSON.stringify(machines));
    } catch {}
  }
</script>

<div class="fields">
  <label>
    <span class="label-text">{t('address')}</span>
    <div class="addr-wrap">
      <input type="text" bind:value={address} placeholder="ws://host:port" autocapitalize="off" autocomplete="off" use:focusIf onkeydown={enter} />
      {#if history.length > 1}
        <button class="hist-btn" class:open={showHistory} aria-expanded={showHistory}
          onclick={() => showHistory = !showHistory}><span class="flip" class:on={showHistory}><Icon name="arrow-down" size={13} /></span></button>
      {/if}
    </div>
    {#if showHistory && history.length}
      <div class="hist-list appear-rise">
        {#each history as h (h.address)}
          <div class="hist-row" animate:flip={{ duration: moveMs() }}>
            <button class="hist-item" onclick={() => { address = h.address; token = h.token; showHistory = false; }}>{h.address}</button>
            <button class="hist-del" onclick={(e) => { e.stopPropagation(); forget(h.address); }}><Icon name="x" size={11} /></button>
          </div>
        {/each}
      </div>
    {/if}
  </label>

  <label>
    <span class="label-text">{t('token')}</span>
    <div class="token-wrap">
      <span class="token-icon"><Icon name="key" size={13} /></span>
      <input type={showToken ? 'text' : 'password'} bind:value={token} placeholder="auth token" onkeydown={enter} />
      <button class="eye-btn" type="button" onclick={() => showToken = !showToken}>
        <Icon name={showToken ? 'eye-off' : 'eye'} size={14} />
      </button>
    </div>
  </label>

  <label>
    <span class="label-text">{t('tmuxSocket')} <span style="font-weight:400;text-transform:none;letter-spacing:0">{t('tmuxSocketHint')}</span></span>
    <input type="text" bind:value={socket} placeholder="/tmp/tmux-1000/default" autocapitalize="off" autocomplete="off" onkeydown={enter} />
  </label>
</div>

<style>
  .fields {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .addr-wrap { position: relative; }
  .addr-wrap input { padding-right: 36px; }
  /* Centred BY a transform, so the arrow's 180° turn lives on the inner .flip
     wrapper, never on the button (motion.md: a resting transform is not a
     thing to animate over). */
  .hist-btn {
    position: absolute; right: 8px; top: 50%; transform: translateY(-50%);
    background: none; border: none; color: var(--text3); cursor: pointer;
    padding: 4px; display: flex; -webkit-tap-highlight-color: transparent;
    transition: color var(--t-fast);
  }
  .hist-btn:active { color: var(--accent); }
  .hist-list {
    display: flex; flex-direction: column;
    border: 1px solid var(--input-border); border-radius: var(--ui-radius-control);
    overflow: hidden; margin-top: 2px;
  }
  .hist-row {
    display: flex; align-items: center;
    border-bottom: 1px solid var(--border2); min-width: 0;
  }
  .hist-row:last-child { border-bottom: none; }
  .hist-item {
    flex: 1; min-width: 0; padding: 9px 14px; border: none; background: none;
    color: var(--text); font-size: var(--fs-body); text-align: left; cursor: pointer;
    font-family: var(--font-mono);
    -webkit-tap-highlight-color: transparent;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    transition: background var(--t-fast), color var(--t-fast);
  }
  .hist-item:active { background: var(--accent-bg); color: var(--accent); }
  .hist-del {
    padding: 8px 10px; border: none; background: none;
    color: var(--text3); cursor: pointer; -webkit-tap-highlight-color: transparent;
    transition: color var(--t-fast);
  }
  .hist-del:active { color: var(--danger); }

  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .label-text {
    font-size: var(--fs-sub);
    font-weight: 500;
    color: var(--text3);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  input {
    width: 100%;
    padding: 11px 14px;
    border: 1px solid var(--input-border);
    border-radius: var(--ui-radius-control);
    background: var(--input-bg);
    color: var(--text);
    /* The iOS no-auto-zoom threshold; see --fs-input-touch in app.css. */
    font-size: var(--fs-input-touch);
    outline: none;
    transition: border-color var(--t-fast) ease, background var(--t-fast) ease, box-shadow var(--t-fast) ease;
    -webkit-appearance: none;
    appearance: none;
  }
  input:focus {
    border-color: var(--accent);
    background: var(--accent-bg);
    box-shadow: 0 0 0 3px var(--accent-glow);
  }
  input::placeholder { color: var(--text3); }

  .token-wrap {
    position: relative;
  }
  .token-icon {
    position: absolute;
    left: 12px;
    top: 50%;
    transform: translateY(-50%);
    font-size: var(--fs-body);
    pointer-events: none;
  }
  .token-wrap input { padding-left: 36px; padding-right: 36px; }
  .eye-btn {
    position: absolute; right: 8px; top: 50%; transform: translateY(-50%);
    background: none; border: none; color: var(--text3); cursor: pointer;
    padding: 4px; display: flex; -webkit-tap-highlight-color: transparent;
    transition: color var(--t-fast);
  }
  .eye-btn:active { color: var(--accent); }
  /* The press scale rides the inner svg: the button's own transform is its
     vertical centring and must not be animated over. */
  .eye-btn :global(svg) { transition: transform var(--t-fast); }
  .eye-btn:active :global(svg) { transform: scale(0.9); }
</style>
