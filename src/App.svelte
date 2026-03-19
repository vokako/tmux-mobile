<script>
  import Settings from './lib/Settings.svelte';
  import Sessions from './lib/Sessions.svelte';
  import Terminal from './lib/Terminal.svelte';
  import Files from './lib/Files.svelte';
  import Icon from './lib/Icon.svelte';
  import { connect, isConnected, disconnect, setOnDisconnect, subscribe as wsSubscribe } from './lib/ws.js';

  let page = $state('settings');
  let connected = $state(false);
  let terminalTarget = $state('');
  let terminalSession = $state('');
  let terminalCommand = $state('');
  let viewMode = $state('terminal');
  let chatSupported = $state(false);
  let theme = $state(localStorage.getItem('tmux_theme') || 'system');
  let showSettings = $state(false);

  // Android keyboard height — only from native Tauri event, not browser
  $effect(() => {
    const handler = (e) => {
      document.documentElement.style.setProperty('--keyboard-height', (e.detail?.height || 0) + 'px');
    };
    window.addEventListener('androidKeyboardHeight', handler);
    return () => window.removeEventListener('androidKeyboardHeight', handler);
  });

  function setTheme(t) {
    theme = t;
    localStorage.setItem('tmux_theme', t);
    applyTheme();
  }

  let isDark = $state(true);

  function applyTheme() {
    isDark = theme === 'dark' || (theme === 'system' && window.matchMedia('(prefers-color-scheme: dark)').matches);
    document.documentElement.setAttribute('data-theme', isDark ? 'dark' : 'light');
  }

  let iconSrc = $derived(isDark ? '/assets/icon-dark.svg' : '/assets/icon-light.svg');

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

  let reconnecting = $state(false);
  let reconnectTimer = null;

  setOnDisconnect(() => {
    if (manualDisconnect) {
      manualDisconnect = false;
      connected = false;
      return;
    }
    connected = false;
    reconnecting = true;
    tryReconnect();
  });

  function cancelReconnect() {
    reconnecting = false;
    clearTimeout(reconnectTimer);
    disconnect();
    page = 'settings';
  }

  function tryReconnect(attempt = 0) {
    if (!reconnecting) return;
    const addr = localStorage.getItem('tmux_address');
    const token = localStorage.getItem('tmux_token') || '';
    if (!addr) { reconnecting = false; page = 'settings'; return; }
    connect(addr, token).then(() => {
      if (!reconnecting) return;
      connected = true;
      reconnecting = false;
      if (terminalTarget) wsSubscribe(terminalTarget);
    }).catch(() => {
      if (!reconnecting) return;
      if (attempt < 20) {
        const delay = Math.min(1000 * (attempt + 1), 5000);
        reconnectTimer = setTimeout(() => tryReconnect(attempt + 1), delay);
      } else {
        reconnecting = false;
        page = 'settings';
      }
    });
  }

  function onConnected() {
    connected = true;
    page = 'sessions';
    localStorage.removeItem('tmux_disconnected');
  }

  function openTerminal(session, target, command = '') {
    terminalSession = session;
    terminalTarget = target;
    terminalCommand = command;
    page = 'terminal';
    viewMode = 'terminal';
    navPush();
  }

  function doDisconnect() {
    reconnecting = false;
    clearTimeout(reconnectTimer);
    manualDisconnect = true;
    disconnect();
    connected = false;
    page = 'settings';
    localStorage.removeItem('tmux_state');
    localStorage.setItem('tmux_disconnected', '1');
  }

  // Auto-reconnect and restore state on page load
  let autoConnectAttempted = false;

  // Detect app resume (Android background → foreground)
  $effect(() => {
    const handler = () => {
      if (document.visibilityState === 'visible' && !isConnected() && !reconnecting) {
        connected = false;
        reconnecting = true;
        tryReconnect();
      }
    };
    document.addEventListener('visibilitychange', handler);
    return () => document.removeEventListener('visibilitychange', handler);
  });

  $effect(() => {
    if (autoConnectAttempted || connected) return;
    const addr = localStorage.getItem('tmux_address');
    const token = localStorage.getItem('tmux_token');
    if (!addr || !token) return;
    if (localStorage.getItem('tmux_disconnected')) return;
    autoConnectAttempted = true;

    const timeout = setTimeout(() => { page = 'settings'; }, 5000);

    connect(addr, token).then(() => {
      clearTimeout(timeout);
      connected = true;
      try {
        const s = JSON.parse(localStorage.getItem('tmux_state') || '{}');
        if (s.terminalTarget) {
          terminalTarget = s.terminalTarget;
          terminalSession = s.terminalSession || '';
          terminalCommand = s.terminalCommand || '';
          page = s.page || 'terminal';
          viewMode = 'terminal';
        } else {
          page = 'sessions';
        }
      } catch { page = 'sessions'; }
    }).catch(() => {
      clearTimeout(timeout);
      page = 'settings';
    });
  });
  // Android back gesture via history API
  // Every navigation push a state, back pops it naturally
  let filesGoBack = $state(null);

  function navPush() { history.pushState({ app: true }, ''); }

  $effect(() => {
    const handler = (e) => {
      if (!e.state?.app) {
        // Reached bottom of our stack, re-push to prevent exit
        navPush();
        return;
      }
      if (page === 'files' && filesGoBack && filesGoBack()) return;
      if (page === 'files' || (page === 'terminal' && viewMode === 'chat')) {
        viewMode = 'terminal'; page = 'terminal'; return;
      }
      if (page === 'terminal') { page = 'sessions'; return; }
      if (showSettings) { showSettings = false; return; }
      // At sessions root, re-push to prevent exit
      navPush();
    };
    window.addEventListener('popstate', handler);
    // Seed one entry
    navPush();
    return () => window.removeEventListener('popstate', handler);
  });
  // Swipe left/right to switch tabs with slide animation
  const tabs = $derived(() => {
    const t = ['sessions'];
    if (terminalTarget) {
      t.push('terminal');
      if (chatSupported) t.push('chat');
      t.push('files');
    }
    return t;
  });

  let swipeX = 0;
  let swipeY = 0;
  let swipeDir = 0;
  let slideAnim = $state('');

  function curTabIdx() {
    const t = tabs();
    const cur = page === 'terminal' ? (viewMode === 'chat' ? 'chat' : 'terminal') : page;
    return t.indexOf(cur);
  }

  function onPageTouchStart(e) {
    if (slideAnim) return;
    swipeX = e.touches[0].clientX;
    swipeY = e.touches[0].clientY;
    swipeDir = 0;
  }

  function onPageTouchEnd(e) {
    if (slideAnim) return;
    const dx = e.changedTouches[0].clientX - swipeX;
    const dy = Math.abs(e.changedTouches[0].clientY - swipeY);
    if (Math.abs(dx) < 120 || dy > Math.abs(dx) * 0.7) return;
    if (page === 'files' && dx > 0 && swipeX < 40) return;

    const t = tabs();
    const idx = curTabIdx();
    const dir = dx < 0 ? 1 : -1;
    const next = t[idx + dir];
    if (!next) return;
    switchTab(next);
  }

  function switchTab(target) {
    if (slideAnim) return;
    const t = tabs();
    const curName = page === 'terminal' ? (viewMode === 'chat' ? 'chat' : 'terminal') : page;
    if (target === curName) return;
    const fromIdx = t.indexOf(curName);
    const toIdx = t.indexOf(target);
    // Apply page change immediately
    if (target === 'chat') { page = 'terminal'; viewMode = 'chat'; }
    else if (target === 'terminal') { page = 'terminal'; viewMode = 'terminal'; }
    else { page = target; }
    navPush();
    // Single slide-in animation from the correct direction
    if (fromIdx >= 0 && toIdx >= 0) {
      slideAnim = toIdx > fromIdx ? 'slide-in-right' : 'slide-in-left';
      requestAnimationFrame(() => {
        setTimeout(() => { slideAnim = ''; }, 120);
      });
    }
  }
</script>

<main>
  <nav>
    {#if connected}
      <img class="nav-icon" src={iconSrc} alt="" width="28" height="28" />
      <div class="nav-pills">
        <button class:active={page === 'sessions'} onclick={() => switchTab('sessions')}>
          Sessions
        </button>
        {#if terminalTarget}
          <button class:active={page === 'terminal' && viewMode === 'terminal'} onclick={() => switchTab('terminal')}>
            Terminal
          </button>
        {/if}
        {#if terminalTarget && chatSupported}
          <button class:active={page === 'terminal' && viewMode === 'chat'} onclick={() => switchTab('chat')}>
            Chat
          </button>
        {/if}
        {#if terminalTarget}
          <button class:active={page === 'files'} onclick={() => switchTab('files')}>
            Files
          </button>
        {/if}
      </div>
      <div class="nav-right">
        <button class="gear-btn" onclick={() => showSettings = !showSettings}><Icon name="gear" size={16} /></button>
      </div>
    {:else}
      <div class="brand">
        <img class="logo" src={iconSrc} alt="" width="24" height="24" />
        <span class="brand-text">tmux<span class="brand-accent">mobile</span></span>
      </div>
      <div class="nav-right">
        <button class="gear-btn" onclick={() => showSettings = !showSettings}><Icon name="gear" size={16} /></button>
      </div>
    {/if}
  </nav>

  {#if showSettings}
    <div class="settings-panel">
      {#if connected}
        <div class="sp-section">
          <div class="sp-label">Connection</div>
          <div class="sp-info">{localStorage.getItem('tmux_address')}</div>
        </div>
      {/if}
      <div class="sp-section">
        <div class="sp-label">Theme</div>
        <div class="sp-btns">
          <button class:active={theme === 'system'} onclick={() => setTheme('system')}>Auto</button>
          <button class:active={theme === 'light'} onclick={() => setTheme('light')}>Light</button>
          <button class:active={theme === 'dark'} onclick={() => setTheme('dark')}>Dark</button>
        </div>
      </div>
      {#if connected}
      <button class="sp-disconnect" onclick={() => { showSettings = false; doDisconnect(); }}>Disconnect</button>
      {/if}
    </div>
    <button class="sp-overlay" onclick={() => showSettings = false} aria-label="Close settings"></button>
  {/if}

  {#if reconnecting && page !== 'settings'}
    <div class="reconnect-bar">
      <span class="reconnect-spinner"></span> Reconnecting...
      <button class="reconnect-cancel" onclick={cancelReconnect}>Cancel</button>
    </div>
  {/if}

  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="page {slideAnim}" class:page-terminal={page === 'terminal'}
    ontouchstart={onPageTouchStart} ontouchend={onPageTouchEnd}>
    {#if page === 'settings'}
      <Settings {onConnected} />
    {:else if page === 'sessions'}
      <Sessions {openTerminal} activeTarget={terminalTarget} visible={page === 'sessions'} />
    {/if}
    {#if terminalTarget}
      <div class="page-layer" class:hidden={page !== 'files'}>
        <Files session={terminalSession} onGoBack={(fn) => filesGoBack = fn} />
      </div>
      <div class="page-layer" class:hidden={page !== 'terminal'}>
        <Terminal target={terminalTarget} session={terminalSession} command={terminalCommand} {viewMode} onChatSupported={(v) => chatSupported = v} onSwitchPane={(t, cmd) => { terminalTarget = t; terminalCommand = cmd || ''; }} />
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
    --status-ok: #4ade80; --status-warn: #fbbf24; --status-danger: #ff5050;
    --nav-bg: rgba(12,12,20,0.95); --pill-bg: rgba(255,255,255,0.04);
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
    --status-ok: #16a34a; --status-warn: #ca8a04; --status-danger: #e53e3e;
    --nav-bg: rgba(245,245,247,0.95); --pill-bg: rgba(0,0,0,0.03);
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
    gap: 8px;
    padding: 8px 12px;
    padding-top: calc(8px + var(--sat));
    background: var(--nav-bg);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    z-index: 10;
  }

  .nav-pills {
    display: flex;
    align-items: center;
    gap: 2px;
    background: var(--pill-bg);
    border-radius: 10px;
    padding: 2px;
  }
  .nav-icon { margin-right: 6px; flex-shrink: 0; margin-top: -2px; margin-bottom: -2px; }

  .nav-pills button {
    padding: 7px 10px;
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
    margin-left: auto;
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
    border-radius: 8px; background: var(--bg2); color: var(--danger);
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
    width: 24px; height: 24px;
    filter: drop-shadow(0 0 6px rgba(0, 212, 255, 0.4));
  }
  :global(html[data-theme="light"]) .logo {
    filter: brightness(0.7) drop-shadow(0 0 4px rgba(0, 136, 204, 0.3));
  }
  .brand-text {
    font-weight: 600;
    font-size: 15px;
    color: var(--text2);
    letter-spacing: -0.3px;
  }
  .brand-accent { color: var(--accent); }

  .reconnect-bar {
    display: flex; align-items: center; justify-content: center; gap: 8px;
    padding: 6px; background: var(--bg2); border-bottom: 1px solid var(--accent); color: var(--accent);
    font-size: 12px; font-weight: 500; flex-shrink: 0;
  }
  .reconnect-spinner {
    width: 12px; height: 12px; border: 2px solid var(--border);
    border-top-color: var(--accent); border-radius: 50%;
    animation: reconnect-spin 0.6s linear infinite;
  }
  @keyframes reconnect-spin { to { transform: rotate(360deg); } }
  .reconnect-cancel {
    margin-left: auto; padding: 2px 10px; border: 1px solid var(--accent);
    border-radius: 6px; background: none; color: var(--accent); font-size: 11px;
    font-weight: 600; cursor: pointer; -webkit-tap-highlight-color: transparent;
  }

  .page {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    position: relative;
  }
  .page-terminal { background: var(--bg); }

  /* Swipe transition animations */
  .page { will-change: transform; }
  .page.slide-in-left   { animation: slideInLeft 0.12s linear; }
  .page.slide-in-right  { animation: slideInRight 0.12s linear; }
  @keyframes slideInLeft  { from { transform: translateX(-40%); } to { transform: none; } }
  @keyframes slideInRight { from { transform: translateX(40%); } to { transform: none; } }
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
