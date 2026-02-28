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
  let theme = $state(localStorage.getItem('tmux_theme') || 'system');
  let showSettings = $state(false);

  // Android keyboard height
  $effect(() => {
    const handler = (e) => {
      document.documentElement.style.setProperty('--keyboard-height', (e.detail?.height || 0) + 'px');
    };
    window.addEventListener('androidKeyboardHeight', handler);
    // Fallback: visualViewport for iOS/browser
    const vv = window.visualViewport;
    if (vv) {
      let initH = vv.height;
      const onResize = () => {
        if (vv.height > initH) initH = vv.height;
        const kb = Math.max(0, initH - vv.height);
        if (!window.__ANDROID_KEYBOARD_HEIGHT__) {
          document.documentElement.style.setProperty('--keyboard-height', (kb > 50 ? kb : 0) + 'px');
        }
      };
      vv.addEventListener('resize', onResize);
      return () => { window.removeEventListener('androidKeyboardHeight', handler); vv.removeEventListener('resize', onResize); };
    }
    return () => window.removeEventListener('androidKeyboardHeight', handler);
  });

  function setTheme(t) {
    theme = t;
    localStorage.setItem('tmux_theme', t);
    applyTheme();
  }

  function applyTheme() {
    const isDark = theme === 'dark' || (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    document.documentElement.setAttribute('data-theme', isDark ? 'dark' : 'light');
  }

  $effect(() => {
    applyTheme();
    const mq = window.matchMedia('(prefers-color-scheme: dark)');
    const handler = () => { if (theme === 'system') applyTheme(); };
    mq.addEventListener('change', handler);
    return () => mq.removeEventListener('change', handler);
  });

  $effect(() => {
    if (!chatSupported && viewMode === 'chat') viewMode = 'terminal';
  });

  // Persist nav state for restore on reload
  $effect(() => {
    if (connected && terminalTarget) {
      localStorage.setItem('tmux_state', JSON.stringify({
        page, viewMode, terminalTarget, terminalSession, terminalCommand
      }));
    }
  });

  let manualDisconnect = false;

  setOnDisconnect(() => {
    connected = false;
    if (!manualDisconnect) {
      // Network disconnect — try auto-reconnect after delay
      setTimeout(() => {
        const host = localStorage.getItem('tmux_host');
        const port = localStorage.getItem('tmux_port');
        const token = localStorage.getItem('tmux_token');
        if (host && port && token && !connected) {
          connect(host, parseInt(port), token).then(() => {
            connected = true;
          }).catch(() => { page = 'settings'; });
        }
      }, 1000);
    } else {
      page = 'settings';
      manualDisconnect = false;
    }
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
    manualDisconnect = true;
    disconnect();
    connected = false;
    page = 'settings';
    localStorage.removeItem('tmux_state');
  }

  // Auto-reconnect and restore state on page load
  let autoConnectAttempted = false;
  $effect(() => {
    if (autoConnectAttempted || connected) return;
    const host = localStorage.getItem('tmux_host');
    const port = localStorage.getItem('tmux_port');
    const token = localStorage.getItem('tmux_token');
    if (host && port && token) {
      autoConnectAttempted = true;
      connect(host, parseInt(port), token).then(() => {
        connected = true;
        const saved = localStorage.getItem('tmux_state');
        if (saved) {
          try {
            const s = JSON.parse(saved);
            if (s.terminalTarget) {
              terminalTarget = s.terminalTarget;
              terminalSession = s.terminalSession || '';
              terminalCommand = s.terminalCommand || '';
              page = s.page || 'terminal';
              viewMode = s.viewMode || 'terminal';
            } else {
              page = 'sessions';
            }
          } catch { page = 'sessions'; }
        } else {
          page = 'sessions';
        }
      }).catch(() => {
        autoConnectAttempted = false;
        // Stay on settings, credentials might be wrong
      });
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
        <button class="gear-btn" onclick={() => showSettings = !showSettings}><Icon name="gear" size={16} /></button>
      </div>
    {:else}
      <div class="brand">
        <span class="logo"><Icon name="command" size={20} /></span>
        <span class="brand-text">tmux<span class="brand-accent">mobile</span></span>
      </div>
    {/if}
  </nav>

  {#if showSettings}
    <div class="settings-panel">
      <div class="sp-section">
        <div class="sp-label">Connection</div>
        <div class="sp-info">{localStorage.getItem('tmux_host')}:{localStorage.getItem('tmux_port')}</div>
      </div>
      <div class="sp-section">
        <div class="sp-label">Theme</div>
        <div class="sp-btns">
          <button class:active={theme === 'system'} onclick={() => setTheme('system')}>Auto</button>
          <button class:active={theme === 'light'} onclick={() => setTheme('light')}>Light</button>
          <button class:active={theme === 'dark'} onclick={() => setTheme('dark')}>Dark</button>
        </div>
      </div>
      <button class="sp-disconnect" onclick={() => { showSettings = false; doDisconnect(); }}>Disconnect</button>
    </div>
    <button class="sp-overlay" onclick={() => showSettings = false}></button>
  {/if}

  <div class="page" class:page-terminal={page === 'terminal'}>
    {#if page === 'settings'}
      <Settings {onConnected} />
    {:else if page === 'sessions'}
      <Sessions {openTerminal} activeTarget={terminalTarget} visible={page === 'sessions'} />
    {/if}
    {#if terminalTarget}
      <div class="page-layer" class:hidden={page !== 'files'}>
        <Files session={terminalSession} />
      </div>
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
    background: var(--bg);
    color: var(--text);
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
  :global(html) { overscroll-behavior: none; --sat: env(safe-area-inset-top); --sab: env(safe-area-inset-bottom); --keyboard-height: 0px; }
  :global(html[data-theme="dark"]) {
    --bg: #0a0a0f; --bg2: #0f0f18; --bg3: #12121a;
    --text: #e2e8f0; --text2: rgba(226,232,240,0.5); --text3: rgba(226,232,240,0.3);
    --border: rgba(255,255,255,0.06); --border2: rgba(255,255,255,0.04);
    --surface: rgba(255,255,255,0.03); --surface2: rgba(255,255,255,0.06);
    --accent: #00d4ff; --accent-bg: rgba(0,212,255,0.12); --accent-glow: rgba(0,212,255,0.1);
    --danger: #ff5050; --danger-bg: rgba(255,80,80,0.08);
    --nav-bg: rgba(12,12,20,0.8); --pill-bg: rgba(255,255,255,0.04);
    --input-bg: rgba(255,255,255,0.04); --input-border: rgba(255,255,255,0.08);
    --code-bg: rgba(255,255,255,0.05);
  }
  :global(html[data-theme="light"]) {
    --bg: #f5f5f7; --bg2: #eeeef0; --bg3: #e8e8ec;
    --text: #1a1a2e; --text2: rgba(26,26,46,0.55); --text3: rgba(26,26,46,0.35);
    --border: rgba(0,0,0,0.08); --border2: rgba(0,0,0,0.05);
    --surface: rgba(0,0,0,0.02); --surface2: rgba(0,0,0,0.04);
    --accent: #0088cc; --accent-bg: rgba(0,136,204,0.08); --accent-glow: rgba(0,136,204,0.06);
    --danger: #e53e3e; --danger-bg: rgba(229,62,62,0.06);
    --nav-bg: rgba(245,245,247,0.9); --pill-bg: rgba(0,0,0,0.03);
    --input-bg: rgba(0,0,0,0.02); --input-border: rgba(0,0,0,0.08);
    --code-bg: rgba(0,0,0,0.03);
  }
  :global(::selection) { background: rgba(0, 212, 255, 0.25); }

  main {
    display: flex;
    flex-direction: column;
    height: 100vh;
    height: 100dvh;
    max-width: 100vw;
    overflow: hidden;
    padding-bottom: var(--keyboard-height);
    background: linear-gradient(180deg, var(--bg) 0%, var(--bg2) 50%, var(--bg3) 100%);
  }

  nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 8px 12px;
    padding-top: calc(8px + var(--sat));
    background: var(--nav-bg);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    z-index: 10;
  }

  .nav-pills {
    display: flex;
    gap: 2px;
    background: var(--pill-bg);
    border-radius: 10px;
    padding: 2px;
  }

  .nav-pills button {
    padding: 6px 8px;
    border: none;
    border-radius: 8px;
    background: transparent;
    color: var(--text2);
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
    background: var(--accent-bg);
    color: var(--accent);
    box-shadow: 0 0 12px var(--accent-glow);
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
    background: var(--accent);
    box-shadow: 0 0 8px var(--accent-glow);
    animation: pulse 2s ease-in-out infinite;
  }

  @keyframes pulse {
    0%, 100% { opacity: 1; }
    50% { opacity: 0.5; }
  }

  .gear-btn {
    padding: 6px; border: none; border-radius: 8px; background: none;
    color: var(--text3); cursor: pointer; display: flex;
    -webkit-tap-highlight-color: transparent;
  }
  .gear-btn:active { color: var(--accent); }

  .sp-overlay {
    position: fixed; inset: 0; background: rgba(0,0,0,0.3); z-index: 20;
    border: none; cursor: default;
  }
  .settings-panel {
    position: absolute; top: 48px; right: 8px; z-index: 21;
    background: var(--bg); border: 1px solid var(--border);
    border-radius: 12px; padding: 12px; min-width: 220px;
    box-shadow: 0 8px 24px rgba(0,0,0,0.3);
  }
  .sp-section { padding: 8px 0; border-bottom: 1px solid var(--border2); }
  .sp-section:last-of-type { border-bottom: none; }
  .sp-label { font-size: 11px; font-weight: 600; color: var(--text3); text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 6px; }
  .sp-info { font-size: 13px; font-family: 'SF Mono', Menlo, monospace; color: var(--text2); }
  .sp-btns {
    display: flex; gap: 4px; background: var(--pill-bg); border-radius: 8px; padding: 2px;
  }
  .sp-btns button {
    padding: 5px 12px; border: none; border-radius: 6px; background: transparent;
    color: var(--text3); font-size: 12px; font-weight: 500; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .sp-btns button.active { background: var(--accent-bg); color: var(--accent); }
  .sp-disconnect {
    width: 100%; margin-top: 8px; padding: 10px; border: 1px solid var(--danger);
    border-radius: 8px; background: var(--danger-bg); color: var(--danger);
    font-size: 13px; font-weight: 600; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }

  .brand {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 0;
  }
  .logo {
    font-size: 20px;
    color: var(--accent);
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
  .page-terminal { background: var(--bg); }
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
