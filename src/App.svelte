<script>
  import Settings from './lib/Settings.svelte';
  import Sessions from './lib/Sessions.svelte';
  import Terminal from './lib/Terminal.svelte';
  import Files from './lib/Files.svelte';
  import Icon from './lib/Icon.svelte';
  import { connect, isConnected, disconnect, setOnDisconnect } from './lib/ws.js';

  let page = $state('settings');
  let connected = $state(false);
  let terminalTarget = $state('');
  let terminalSession = $state('');
  let terminalCommand = $state('');
  let viewMode = $state('terminal');
  let chatSupported = $state(false);

  $effect(() => {
    if (!chatSupported && viewMode === 'chat') viewMode = 'terminal';
  });

  setOnDisconnect(() => {
    connected = false;
    page = 'settings';
  });

  function onConnected() {
    connected = true;
    page = 'sessions';
  }

  function openTerminal(session, target, command = '') {
    terminalSession = session;
    terminalTarget = target;
    terminalCommand = command;
    page = 'terminal';
    viewMode = 'terminal';
  }

  function doDisconnect() {
    disconnect();
    connected = false;
    page = 'settings';
  }

  // Auto-reconnect on page load if credentials are saved
  $effect(() => {
    const host = localStorage.getItem('tmux_host');
    const port = localStorage.getItem('tmux_port');
    const token = localStorage.getItem('tmux_token');
    if (host && port && token && !connected) {
      connect(host, parseInt(port), token).then(() => {
        onConnected();
      }).catch(() => {});
    }
  });
</script>

<main>
  <nav>
    {#if connected}
      <div class="nav-pills">
        <button class:active={page === 'sessions'} onclick={() => page = 'sessions'}>
          <Icon name="sessions" size={13} /> Sessions
        </button>
        {#if terminalTarget}
          <button class:active={page === 'terminal' && viewMode === 'terminal'} onclick={() => { page = 'terminal'; viewMode = 'terminal'; }}>
            <Icon name="terminal" size={13} /> Terminal
          </button>
        {/if}
        {#if terminalTarget && chatSupported}
          <button class:active={page === 'terminal' && viewMode === 'chat'} onclick={() => { page = 'terminal'; viewMode = 'chat'; }}>
            <Icon name="chat" size={13} /> Chat
          </button>
        {/if}
        {#if terminalTarget}
          <button class:active={page === 'files'} onclick={() => page = 'files'}>
            <Icon name="files" size={13} /> Files
          </button>
        {/if}
      </div>
      <div class="nav-right">
        <span class="status-dot"></span>
      </div>
    {:else}
      <div class="brand">
        <span class="logo"><Icon name="command" size={20} /></span>
        <span class="brand-text">tmux<span class="brand-accent">mobile</span></span>
      </div>
    {/if}
  </nav>

  <div class="page" class:page-terminal={page === 'terminal'}>
    {#if page === 'settings'}
      <Settings {onConnected} />
    {:else if page === 'sessions'}
      <Sessions {openTerminal} activeTarget={terminalTarget} onDisconnect={doDisconnect} />
    {:else if page === 'files'}
      <Files session={terminalSession} />
    {/if}
    {#if terminalTarget}
      <div class="page-layer" class:hidden={page !== 'terminal'}>
        <Terminal target={terminalTarget} session={terminalSession} command={terminalCommand} {viewMode} onChatSupported={(v) => chatSupported = v} />
      </div>
    {/if}
  </div>
</main>

<style>
  :global(body) {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, 'Inter', 'Segoe UI', sans-serif;
    background: #0a0a0f;
    color: #e2e8f0;
    overflow: hidden;
    height: 100vh;
    height: 100dvh;
    -webkit-font-smoothing: antialiased;
    position: fixed;
    width: 100%;
    top: 0;
    left: 0;
    overscroll-behavior: none;
    -webkit-overflow-scrolling: touch;
  }
  :global(*) { box-sizing: border-box; }
  :global(html) { overscroll-behavior: none; }
  :global(::selection) { background: rgba(0, 212, 255, 0.25); }

  main {
    display: flex;
    flex-direction: column;
    height: 100vh;
    height: 100dvh;
    max-width: 100vw;
    overflow: hidden;
    background: linear-gradient(180deg, #0a0a0f 0%, #0f0f18 50%, #12121a 100%);
  }

  nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    background: rgba(12, 12, 20, 0.8);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    border-bottom: 1px solid rgba(255, 255, 255, 0.06);
    flex-shrink: 0;
    z-index: 10;
  }

  .nav-pills {
    display: flex;
    gap: 2px;
    background: rgba(255, 255, 255, 0.04);
    border-radius: 10px;
    padding: 2px;
  }

  .nav-pills button {
    padding: 6px 8px;
    border: none;
    border-radius: 8px;
    background: transparent;
    color: rgba(226, 232, 240, 0.5);
    cursor: pointer;
    font-size: 13px;
    font-weight: 500;
    transition: all 0.2s ease;
    -webkit-tap-highlight-color: transparent;
    display: flex;
    align-items: center;
    gap: 4px;
    white-space: nowrap;
  }
  .nav-pills button:active { transform: scale(0.97); }
  .nav-pills button.active {
    background: rgba(0, 212, 255, 0.12);
    color: #00d4ff;
    box-shadow: 0 0 12px rgba(0, 212, 255, 0.1);
  }
  .nav-pills button:disabled { opacity: 0.3; cursor: default; }

  .nav-right {
    display: flex;
    align-items: center;
    gap: 10px;
  }

  .status-dot {
    width: 7px; height: 7px;
    border-radius: 50%;
    background: #00d4ff;
    box-shadow: 0 0 8px rgba(0, 212, 255, 0.6);
    animation: pulse 2s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 0;
  }
  .logo {
    font-size: 20px;
    color: #00d4ff;
    filter: drop-shadow(0 0 6px rgba(0, 212, 255, 0.4));
  }
  .brand-text {
    font-weight: 600;
    font-size: 15px;
    color: rgba(226, 232, 240, 0.7);
    letter-spacing: -0.3px;
  }
  .brand-accent { color: #00d4ff; }

  .page {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    position: relative;
  }
  .page-terminal { background: #0a0a0f; }
  .page-layer {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    z-index: 1;
  }
  .page-layer.hidden {
    visibility: hidden;
    pointer-events: none;
  }
</style>
