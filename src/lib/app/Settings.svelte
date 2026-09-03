<script lang="ts">
  import { connect, disconnect } from '../core/ws.ts';
  import { defaultConnectionAddress } from '../core/connection-address.ts';
  import { activateConnected } from './servers.ts';
  import Icon from '../ui/Icon.svelte';
  import { copyText } from '../core/clipboard.ts';
  import { t } from '../core/i18n.svelte.ts';
  import { flip } from 'svelte/animate';
  import { moveMs } from '../ui/motion.ts';

  type HistoryEntry = { address: string; token: string };

  let { onConnected }: { onConnected: () => void } = $props();

  let address = $state(localStorage.getItem('tmux_address') || defaultConnectionAddress(location, import.meta.env.DEV));
  let token = $state(localStorage.getItem('tmux_token') || '');
  let socket = $state(localStorage.getItem('tmux_socket') || '');
  let error = $state('');
  let connecting = $state(false);
  let showToken = $state(false);
  let showHistory = $state(false);

  let history = $state<HistoryEntry[]>((() => {
    const raw = JSON.parse(localStorage.getItem('tmux_address_history') || '[]');
    // Migrate: old format was string[], new is {address, token}[]
    return raw.map((h: string | HistoryEntry) => typeof h === 'string' ? { address: h, token: '' } : h);
  })());

  // Migrate old host+port (one-time)
  const oldHost = localStorage.getItem('tmux_host');
  if (oldHost) {
    const oldPort = localStorage.getItem('tmux_port');
    address = `ws://${oldHost}:${oldPort || '9899'}`;
    localStorage.removeItem('tmux_host');
    localStorage.removeItem('tmux_port');
  }

  // Auto-fill from local config in Tauri desktop app
  $effect(() => {
    if (window.__TAURI__) {
      window.__TAURI__.core.invoke('get_local_config').then((cfg: { host: string; port: number; token: string; tmux_socket?: string }) => {
        if (!localStorage.getItem('tmux_token')) {
          const h = cfg.host === '0.0.0.0' ? '127.0.0.1' : cfg.host;
          address = `ws://${h}:${cfg.port}`;
          token = cfg.token;
          if (cfg.tmux_socket) socket = cfg.tmux_socket;
        }
      }).catch(() => {});
    }
  });

  function normalizeAddress(addr: string): string {
    let a = addr.trim();
    if (!a.startsWith('ws://') && !a.startsWith('wss://')) {
      // Auto-detect: HTTPS page requires wss://, otherwise ws://
      const secure = location.protocol === 'https:';
      a = (secure ? 'wss://' : 'ws://') + a;
    }
    return a;
  }

  function saveHistory(addr: string, tok: string) {
    const entry = { address: addr.trim(), token: tok || '' };
    history = [entry, ...history.filter(h => h.address !== entry.address)].slice(0, 8);
    localStorage.setItem('tmux_address_history', JSON.stringify(history));
  }

  let cancelled = false;

  async function doConnect() {
    error = '';
    connecting = true;
    cancelled = false;
    try {
      const url = normalizeAddress(address);
      localStorage.setItem('tmux_address', url);
      localStorage.setItem('tmux_token', token);
      if (socket.trim()) localStorage.setItem('tmux_socket', socket.trim());
      else localStorage.removeItem('tmux_socket');
      saveHistory(url, token);
      await connect(url, token);
      if (cancelled) return;
      if (socket.trim()) {
        const { setSocket } = await import('../core/ws.ts');
        await setSocket(socket.trim()).catch(() => {});
      }
      // Save machine_id → address mapping
      try {
        const { getMachineId } = await import('../core/ws.ts');
        const mid = getMachineId?.();
        if (mid) {
          const map = JSON.parse(localStorage.getItem('tmux_machines') || '{}');
          const addrs = map[mid] || [];
          if (!addrs.includes(url)) addrs.push(url);
          map[mid] = addrs.slice(-8);
          localStorage.setItem('tmux_machines', JSON.stringify(map));
          // The LIVE tmux_machine_id is deliberately NOT written here:
          // activateConnected owns that key. A pre-write would poison the
          // park on the different-server path — parkAndPoint would file the
          // NEW machine's id under the OLD server's parking slot (lead
          // blocker #2, board #55). The map above is safe: it is keyed by
          // machineId and never read through the live key.
        }
        // The multi-server registry (board #55): a successful connect RECORDS
        // by machine identity and then asks whether this was a different
        // server. Same machine (a LAN/Tailscale alternate) — nothing to
        // activate, the socket swap was the whole event. A DIFFERENT server —
        // the old one's live state is parked under its id and the app must
        // REBOOT through the boot path: Hub room caches, mounted terminals
        // and Files cwds are old-server memory that no in-place connect can
        // reset (lead blocker). The mirror keys already point here — this
        // form wrote them before dialing.
        const act = activateConnected(localStorage, {
          address: url, token,
          ...(socket.trim() ? { socket: socket.trim() } : {}),
          ...(mid ? { machineId: mid } : {}),
        });
        if (act.reload) { location.reload(); return; }
      } catch {}
      onConnected();
    } catch (e) {
      if (!cancelled) error = (e as Error).message;
    } finally {
      connecting = false;
    }
  }

  function cancelConnect() {
    cancelled = true;
    connecting = false;
    disconnect();
  }

  // Build a deep link that pre-fills + auto-connects (consumed by App.svelte's
  // consumeConnectUrlParams). Includes the token so the link connects on its own.
  let shared = $state('');
  function buildShareUrl() {
    const url = normalizeAddress(address);
    const params = new URLSearchParams();
    params.set('addr', url);
    if (token) params.set('token', token);
    if (socket.trim()) params.set('socket', socket.trim());
    return `${location.origin}${location.pathname}?${params.toString()}`;
  }
  async function shareLink() {
    await copyText(buildShareUrl());
    shared = t('linkCopied');
    setTimeout(() => shared = '', 2000);
  }
</script>

<div class="wrapper">
  <div class="card">
    <div class="card-header">
      <div class="icon"><img class="icon-dark" src="/assets/icon-dark.svg" alt="" width="72" height="72" /><img class="icon-light" src="/assets/icon-light.svg" alt="" width="72" height="72" /></div>
      <h2>tmux<span class="accent">mobile</span></h2>
      <p class="subtitle">{t('connectTitle')}</p>
    </div>

    <div class="fields">
      <label>
        <span class="label-text">{t('address')}</span>
        <div class="addr-wrap">
          <input type="text" bind:value={address} placeholder="ws://host:port" autocapitalize="off" autocomplete="off" />
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
                <button class="hist-del" onclick={(e) => { e.stopPropagation(); const addr = h.address; history = history.filter(x => x.address !== addr); localStorage.setItem('tmux_address_history', JSON.stringify(history)); try { const machines = JSON.parse(localStorage.getItem('tmux_machines') || '{}'); for (const mid in machines) { machines[mid] = machines[mid].filter((u: string) => u !== addr); if (!machines[mid].length) delete machines[mid]; } localStorage.setItem('tmux_machines', JSON.stringify(machines)); } catch {} }}><Icon name="x" size={11} /></button>
              </div>
            {/each}
          </div>
        {/if}
      </label>

      <label>
        <span class="label-text">{t('token')}</span>
        <div class="token-wrap">
          <span class="token-icon"><Icon name="key" size={13} /></span>
          <input type={showToken ? 'text' : 'password'} bind:value={token} placeholder="auth token" />
          <button class="eye-btn" type="button" onclick={() => showToken = !showToken}>
            <Icon name={showToken ? 'eye-off' : 'eye'} size={14} />
          </button>
        </div>
      </label>

      <label>
        <span class="label-text">{t('tmuxSocket')} <span style="font-weight:400;text-transform:none;letter-spacing:0">{t('tmuxSocketHint')}</span></span>
        <input type="text" bind:value={socket} placeholder="/tmp/tmux-1000/default" autocapitalize="off" autocomplete="off" />
      </label>
    </div>

    {#if error}
      <div class="error appear">{error}</div>
    {/if}

    {#if connecting}
      <div class="connect-row">
        <button class="connect-btn connecting" disabled>
          <span class="spinner"></span> {t('connecting')}
        </button>
        <button class="cancel-btn" onclick={cancelConnect}>{t('cancel')}</button>
      </div>
    {:else}
      <button class="connect-btn" onclick={doConnect} disabled={!address}>
        {t('connect')}
      </button>
    {/if}

    <button class="share-btn" onclick={shareLink} disabled={!address} title={t('shareLink')}>
      <Icon name="copy" size={13} /> {shared || t('shareLink')}
    </button>

    <a class="about-link" href="https://github.com/vokako/tmux-mobile" target="_blank" rel="noopener">
      {t('about')}
    </a>
  </div>
</div>

<style>
  .wrapper {
    flex: 1;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 24px 16px;
  }

  .card {
    width: 100%;
    max-width: 380px;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 16px;
    padding: 32px 24px;
    display: flex;
    flex-direction: column;
    gap: 24px;
    box-shadow: 0 8px 32px rgba(0, 0, 0, 0.3), 0 0 0 1px rgba(255, 255, 255, 0.03) inset;
  }

  .card-header {
    text-align: center;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 6px;
  }

  .icon {
    font-size: var(--fs-hero);
    color: var(--accent);
    filter: drop-shadow(0 0 12px var(--accent-glow));
    margin-bottom: 4px;
  }
  .icon-light { display: none; }
  :global(html[data-theme="light"]) .icon-dark { display: none; }
  :global(html[data-theme="light"]) .icon-light { display: inline; }

  h2 {
    margin: 0;
    font-family: var(--font-display);
    font-size: var(--fs-display);
    font-weight: 700;
    color: var(--text);
    letter-spacing: -0.5px;
  }
  .accent { color: var(--accent); }

  .subtitle {
    margin: 0;
    font-size: var(--fs-ui);
    color: var(--text3);
  }

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

  .connect-btn {
    width: 100%;
    padding: 13px;
    border: none;
    border-radius: var(--ui-radius-row);
    background: var(--accent-fill);
    color: var(--accent-fill-ink);
    font-size: var(--fs-title);
    font-weight: 600;
    cursor: pointer;
    transition: transform var(--t-fast) ease, filter var(--t-fast) ease, opacity var(--t-fast) ease;
    -webkit-tap-highlight-color: transparent;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 8px;
    letter-spacing: -0.2px;
  }
  .connect-btn:active:not(:disabled) {
    transform: scale(0.98);
    filter: brightness(0.9);
  }
  .connect-btn:disabled {
    opacity: 0.4;
    cursor: default;
  }
  .connect-row { display: flex; gap: 8px; }
  .connect-row .connect-btn { flex: 1; }
  .share-btn {
    width: 100%; margin-top: 8px; padding: 10px;
    border: 1px solid var(--border); border-radius: var(--ui-radius-panel);
    background: none; color: var(--text2); font-size: var(--fs-ui); font-weight: 600;
    cursor: pointer; -webkit-tap-highlight-color: transparent;
    display: flex; align-items: center; justify-content: center; gap: 6px;
    transition: color var(--t-fast), border-color var(--t-fast), transform var(--t-fast);
  }
  .share-btn:active:not(:disabled) { transform: scale(0.98); color: var(--accent); border-color: var(--accent); }
  .share-btn:disabled { opacity: 0.4; cursor: default; }
  .about-link {
    display: block; text-align: center; margin-top: 18px;
    color: var(--text3); font-size: var(--fs-ui); text-decoration: none;
    -webkit-tap-highlight-color: transparent;
    transition: color var(--t-fast);
  }
  .about-link:active { color: var(--accent); }
  .cancel-btn {
    padding: 13px 20px; border: 1px solid var(--border); border-radius: var(--ui-radius-panel);
    background: none; color: var(--text2); font-size: var(--fs-body); font-weight: 600;
    cursor: pointer; -webkit-tap-highlight-color: transparent;
  }

  .spinner {
    width: 16px; height: 16px;
    border: 2px solid rgba(0, 0, 0, 0.2);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .spinner { animation: none; } }

  .error {
    color: var(--danger);
    font-size: var(--fs-ui);
    padding: 10px 14px;
    background: var(--danger-bg);
    border: 1px solid color-mix(in srgb, var(--danger) 15%, transparent);
    border-radius: var(--ui-radius-row);
  }
</style>
