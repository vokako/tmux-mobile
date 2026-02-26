<script>
  import Settings from './lib/Settings.svelte';
  import Sessions from './lib/Sessions.svelte';
  import Terminal from './lib/Terminal.svelte';
  import { isConnected, disconnect, setOnDisconnect } from './lib/ws.js';

  let page = $state('settings');
  let connected = $state(false);
  let terminalTarget = $state('');
  let terminalSession = $state('');

  setOnDisconnect(() => {
    connected = false;
    page = 'settings';
  });

  function onConnected() {
    connected = true;
    page = 'sessions';
  }

  function openTerminal(session, target) {
    terminalSession = session;
    terminalTarget = target;
    page = 'terminal';
  }

  function doDisconnect() {
    disconnect();
    connected = false;
    page = 'settings';
  }
</script>

<main>
  <nav>
    {#if connected}
      <div class="nav-pills">
        <button class:active={page === 'sessions'} onclick={() => page = 'sessions'}>
          <span class="nav-icon">⬡</span> Sessions
        </button>
        <button class:active={page === 'terminal'} onclick={() => page = 'terminal'} disabled={!terminalTarget}>
          <span class="nav-icon">⏣</span> Terminal
        </button>
      </div>
      <div class="nav-right">
        <span class="status-dot"></span>
        <button class="disconnect" onclick={doDisconnect}>✕</button>
      </div>
    {:else}
      <div class="brand">
        <span class="logo">⌘</span>
        <span class="brand-text">tmux<span class="brand-accent">mobile</span></span>
      </div>
    {/if}
  </nav>

  <div class="page" class:page-terminal={page === 'terminal'}>
    {#if page === 'settings'}
      <Settings {onConnected} />
    {:else if page === 'sessions'}
      <Sessions {openTerminal} />
    {:else if page === 'terminal'}
      <Terminal target={terminalTarget} session={terminalSession} />
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
    -webkit-font-smoothing: antialiased;
  }
  :global(*) { box-sizing: border-box; }
  :global(::selection) { background: rgba(0, 212, 255, 0.25); }

  main {
    display: flex;
    flex-direction: column;
    height: 100vh;
    max-width: 100vw;
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
    gap: 4px;
    background: rgba(255, 255, 255, 0.04);
    border-radius: 10px;
    padding: 3px;
  }

  .nav-pills button {
    padding: 6px 14px;
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
    gap: 5px;
  }
  .nav-pills button:active { transform: scale(0.97); }
  .nav-pills button.active {
    background: rgba(0, 212, 255, 0.12);
    color: #00d4ff;
    box-shadow: 0 0 12px rgba(0, 212, 255, 0.1);
  }
  .nav-pills button:disabled { opacity: 0.3; cursor: default; }
  .nav-icon { font-size: 11px; }

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

  .disconnect {
    width: 28px; height: 28px;
    border: none;
    border-radius: 8px;
    background: rgba(255, 255, 255, 0.05);
    color: rgba(226, 232, 240, 0.4);
    cursor: pointer;
    font-size: 12px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.2s ease;
    -webkit-tap-highlight-color: transparent;
  }
  .disconnect:active {
    background: rgba(255, 80, 80, 0.15);
    color: #ff5050;
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
  }
  .page-terminal { background: #0a0a0f; }
</style>
