<script>
  import { connect } from './ws.js';
  import Icon from './Icon.svelte';

  let { onConnected } = $props();

  let host = $state(localStorage.getItem('tmux_host') || '127.0.0.1');
  let port = $state(localStorage.getItem('tmux_port') || '9899');
  let token = $state(localStorage.getItem('tmux_token') || '');
  let error = $state('');
  let connecting = $state(false);
  let showToken = $state(false);

  // Auto-fill from local config in Tauri desktop app
  $effect(() => {
    if (window.__TAURI__) {
      window.__TAURI__.core.invoke('get_local_config').then(cfg => {
        if (!localStorage.getItem('tmux_token')) {
          host = cfg.host === '0.0.0.0' ? '127.0.0.1' : cfg.host;
          port = String(cfg.port);
          token = cfg.token;
        }
      }).catch(() => {});
    }
  });

  async function doConnect() {
    error = '';
    connecting = true;
    try {
      localStorage.setItem('tmux_host', host);
      localStorage.setItem('tmux_port', port);
      localStorage.setItem('tmux_token', token);
      await connect(host, parseInt(port), token);
      onConnected();
    } catch (e) {
      error = e.message;
    } finally {
      connecting = false;
    }
  }
</script>

<div class="wrapper">
  <div class="card">
    <div class="card-header">
      <div class="icon"><Icon name="command" size={36} /></div>
      <h2>tmux<span class="accent">mobile</span></h2>
      <p class="subtitle">Connect to your tmux server</p>
    </div>

    <div class="fields">
      <div class="field-row">
        <label>
          <span class="label-text">Host</span>
          <input type="text" bind:value={host} placeholder="127.0.0.1" />
        </label>
        <label class="port-field">
          <span class="label-text">Port</span>
          <input type="text" bind:value={port} placeholder="9899" />
        </label>
      </div>

      <label>
        <span class="label-text">Token</span>
        <div class="token-wrap">
          <span class="token-icon"><Icon name="key" size={13} /></span>
          <input type={showToken ? 'text' : 'password'} bind:value={token} placeholder="auth token" />
          <button class="eye-btn" type="button" onclick={() => showToken = !showToken}>
            <Icon name={showToken ? 'eye-off' : 'eye'} size={14} />
          </button>
        </div>
      </label>
    </div>

    {#if error}
      <div class="error">{error}</div>
    {/if}

    <button class="connect-btn" onclick={doConnect} disabled={connecting || !token}>
      {#if connecting}
        <span class="spinner"></span> Connecting…
      {:else}
        Connect
      {/if}
    </button>
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
    background: var(--surface);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
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
    font-size: 36px;
    color: var(--accent);
    filter: drop-shadow(0 0 12px rgba(0, 212, 255, 0.4));
    margin-bottom: 4px;
  }

  h2 {
    margin: 0;
    font-size: 22px;
    font-weight: 700;
    color: var(--text);
    letter-spacing: -0.5px;
  }
  .accent { color: var(--accent); }

  .subtitle {
    margin: 0;
    font-size: 13px;
    color: var(--text3);
  }

  .fields {
    display: flex;
    flex-direction: column;
    gap: 14px;
  }

  .field-row {
    display: flex;
    gap: 10px;
  }
  .field-row label { flex: 1; }
  .port-field { max-width: 90px; }

  label {
    display: flex;
    flex-direction: column;
    gap: 6px;
  }

  .label-text {
    font-size: 12px;
    font-weight: 500;
    color: var(--text3);
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }

  input {
    width: 100%;
    padding: 11px 14px;
    border: 1px solid var(--input-border);
    border-radius: 10px;
    background: var(--input-bg);
    color: var(--text);
    font-size: 15px;
    outline: none;
    transition: all 0.2s ease;
    -webkit-appearance: none;
  }
  input:focus {
    border-color: var(--accent);
    background: var(--accent-bg);
    box-shadow: 0 0 0 3px rgba(0, 212, 255, 0.08);
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
    font-size: 13px;
    pointer-events: none;
  }
  .token-wrap input { padding-left: 36px; padding-right: 36px; }
  .eye-btn {
    position: absolute; right: 8px; top: 50%; transform: translateY(-50%);
    background: none; border: none; color: var(--text3); cursor: pointer;
    padding: 4px; display: flex; -webkit-tap-highlight-color: transparent;
  }
  .eye-btn:active { color: var(--accent); }

  .connect-btn {
    width: 100%;
    padding: 13px;
    border: none;
    border-radius: 10px;
    background: var(--accent);
    color: #000;
    font-size: 15px;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.2s ease;
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

  .spinner {
    width: 16px; height: 16px;
    border: 2px solid rgba(0, 0, 0, 0.2);
    border-top-color: #000;
    border-radius: 50%;
    animation: spin 0.6s linear infinite;
  }
  @keyframes spin { to { transform: rotate(360deg); } }

  .error {
    color: var(--danger);
    font-size: 13px;
    padding: 10px 14px;
    background: var(--danger-bg);
    border: 1px solid rgba(255, 80, 80, 0.15);
    border-radius: 10px;
  }
</style>
