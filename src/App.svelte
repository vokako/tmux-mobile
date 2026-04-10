<script>
  import Settings from './lib/Settings.svelte';
  import Sessions from './lib/Sessions.svelte';
  import Terminal from './lib/Terminal.svelte';
  import Files from './lib/Files.svelte';
  import Icon from './lib/Icon.svelte';
  import { connect, isConnected, disconnect, setOnDisconnect, subscribe as wsSubscribe, getMachineId, getHostname, findBestAddress, classifyAddress, ADDRESS_LABELS } from './lib/ws.js';

  // Tunable constants
  const KB_OPEN_THRESHOLD = 100; // px difference to detect keyboard open
  const SWIPE_MIN_DISTANCE = 120; // px minimum for tab swipe
  const SWIPE_MAX_ANGLE = 0.7; // vertical/horizontal ratio to reject diagonal swipes
  const RECONNECT_MAX_ATTEMPTS = 20;
  const SLIDE_ANIMATION_MS = 120;
  const OPTIMIZE_INTERVAL_MS = 10 * 60 * 1000; // 10 minutes

  let page = $state('settings');
  let connected = $state(false);
  let terminalTarget = $state('');
  let terminalSession = $state('');
  let terminalCommand = $state('');
  let viewMode = $state('terminal');
  let chatSupported = $state(false);
  let theme = $state(localStorage.getItem('tmux_theme') || 'system');
  let fontSize = $state(parseInt(localStorage.getItem('tmux_fontsize')) || 14);
  let showSettings = $state(false);
  let serverInfo = $state({ hostname: '', machineId: '' });
  let activeAddress = $state(localStorage.getItem('tmux_address') || '');
  let debugMode = $state(!!localStorage.getItem('tmux_debug'));
  let debugEl = $state(null);

  // Global debug log — writes directly to DOM to avoid reactivity issues
  window.__dbg = (msg) => {
    if (!debugEl) return;
    const ts = new Date().toLocaleTimeString('en', { hour12: false, fractionalSecondDigits: 2 });
    const div = document.createElement('div');
    div.textContent = `${ts} ${msg}`;
    debugEl.appendChild(div);
    // Keep max 40 lines
    while (debugEl.children.length > 40) debugEl.removeChild(debugEl.firstChild);
    debugEl.scrollTop = debugEl.scrollHeight;
  };

  // Keyboard height detection
  $effect(() => {
    // Android Tauri app: native event provides exact keyboard height
    let androidNativeKb = false;
    const nativeHandler = (e) => {
      androidNativeKb = true; // suppress visualViewport handler on Android
      const kbh = e.detail?.height || 0;
      if (kbh === 0 && window.innerHeight > fullHeight) fullHeight = window.innerHeight;
      const h = kbh > 0 ? (fullHeight - kbh) + 'px' : fullHeight + 'px';
      document.documentElement.style.setProperty('--app-height', h);
      document.documentElement.classList.toggle('keyboard-open', kbh > 0);
      // After layout updates, shift terminal and log
      requestAnimationFrame(() => {
        window.dispatchEvent(new CustomEvent('keyboard-shift', { detail: { kbHeight: kbh > 0 ? kbh : 0 } }));
        const main = document.querySelector('main');
        const termWrap = document.querySelector('.term-wrap');
        window.__dbg?.(`androidKb: kbh=${kbh} appH=${h} mainH=${main?.clientHeight} termH=${termWrap?.clientHeight} bodyH=${document.body.clientHeight}`);
      });
      if (kbh === 0) {
        setTimeout(() => window.dispatchEvent(new Event('terminal-refit')), 100);
      }
    };
    window.addEventListener('androidKeyboardHeight', nativeHandler);

    // Track the max known height (= full screen without keyboard)
    let fullHeight = window.innerHeight;
    window.__fullHeight = () => fullHeight;

    // Mobile browser: track visualViewport height so main always fits
    // the visible area (keyboard doesn't push nav off screen).
    const vv = window.visualViewport;
    const vpHandler = () => {
      if (!vv || androidNativeKb) return;
      const h = vv.height;
      // Update full height when viewport grows (keyboard closing)
      if (h > fullHeight) fullHeight = h;
      const kbOpen = h < fullHeight - KB_OPEN_THRESHOLD;
      const wasKbOpen = document.documentElement.classList.contains('keyboard-open');
      window.__dbg?.(`vpResize: vv.h=${h.toFixed(0)} fullH=${fullHeight} kbOpen=${kbOpen}${kbOpen !== wasKbOpen ? (kbOpen ? ' ⌨️OPEN' : ' ⌨️CLOSE') : ''}`);
      document.documentElement.style.setProperty('--app-height', h + 'px');
      document.documentElement.classList.toggle('keyboard-open', kbOpen);
      // After layout, shift terminal so cursor stays visible
      requestAnimationFrame(() => {
        const kbShift = kbOpen ? (fullHeight - h) : 0;
        window.dispatchEvent(new CustomEvent('keyboard-shift', { detail: { kbHeight: kbShift } }));
      });
      // When keyboard closes, terminal needs to re-fit to the larger container
      if (wasKbOpen && !kbOpen) {
        window.__dbg?.('⌨️CLOSE → dispatching terminal-refit');
        setTimeout(() => window.dispatchEvent(new Event('terminal-refit')), 100);
      }
      window.scrollTo(0, 0);
      document.documentElement.scrollTop = 0;
    };
    if (vv) {
      vv.addEventListener('resize', vpHandler);
      vv.addEventListener('scroll', vpHandler);
    }

    // Log focus/blur on inputs (keyboard open/close trigger)
    const onFocusIn = (e) => window.__dbg?.(`focusIn: ${e.target?.tagName}[${e.target?.className?.slice(0,20)}] activeEl=${document.activeElement?.tagName}`);
    const onFocusOut = (e) => window.__dbg?.(`focusOut: ${e.target?.tagName}[${e.target?.className?.slice(0,20)}]`);
    document.addEventListener('focusin', onFocusIn);
    document.addEventListener('focusout', onFocusOut);

    return () => {
      window.removeEventListener('androidKeyboardHeight', nativeHandler);
      document.removeEventListener('focusin', onFocusIn);
      document.removeEventListener('focusout', onFocusOut);
      if (vv) {
        vv.removeEventListener('resize', vpHandler);
        vv.removeEventListener('scroll', vpHandler);
      }
    };
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
    // Keep connected=true during reconnect to avoid UI flicker
    reconnecting = true;
    tryReconnect();
  });

  function cancelReconnect() {
    reconnecting = false;
    connected = false;
    clearTimeout(reconnectTimer);
    disconnect();
    page = 'settings';
  }

  function getAltAddresses() {
    const mid = localStorage.getItem('tmux_machine_id');
    const primary = localStorage.getItem('tmux_address');
    if (!mid) return [];
    try {
      const map = JSON.parse(localStorage.getItem('tmux_machines') || '{}');
      return (map[mid] || []).filter(a => a !== primary);
    } catch { return []; }
  }

  function tryReconnect(attempt = 0) {
    if (!reconnecting) return;
    const addr = localStorage.getItem('tmux_address');
    const token = localStorage.getItem('tmux_token') || '';
    if (!addr) { reconnecting = false; connected = false; page = 'settings'; return; }

    // Build address list: primary first, then alternates
    const allAddrs = [addr, ...getAltAddresses()];
    const useAddr = allAddrs[attempt % allAddrs.length];

    connect(useAddr, token).then(() => {
      if (!reconnecting) return;
      connected = true;
      reconnecting = false;
      serverInfo = { hostname: getHostname() || '', machineId: getMachineId() || '' };
      if (useAddr !== addr) { localStorage.setItem('tmux_address', useAddr); activeAddress = useAddr; }
      if (terminalTarget) wsSubscribe(terminalTarget);
    }).catch(() => {
      if (!reconnecting) return;
      if (attempt < RECONNECT_MAX_ATTEMPTS) {
        const delay = Math.min(1000 * (attempt + 1), 5000);
        reconnectTimer = setTimeout(() => tryReconnect(attempt + 1), delay);
      } else {
        reconnecting = false;
        connected = false;
        page = 'settings';
      }
    });
  }

  function onConnected() {
    connected = true;
    serverInfo = { hostname: getHostname() || '', machineId: getMachineId() || '' };
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

  // --- Optimal address selection ---
  let lastProbeTime = 0;
  let optimizing = $state(false);

  function getAllAddresses() {
    const mid = localStorage.getItem('tmux_machine_id');
    if (!mid) return [];
    try { return JSON.parse(localStorage.getItem('tmux_machines') || '{}')[mid] || []; }
    catch { return []; }
  }

  async function optimizeConnection() {
    const addrs = getAllAddresses();
    if (addrs.length <= 1 || optimizing) return;
    const current = localStorage.getItem('tmux_address');
    optimizing = true;
    try {
      const best = await findBestAddress(addrs);
      if (!best || best === current) return;
      // Only switch if the new address is higher priority (lower class number)
      if (classifyAddress(best) >= classifyAddress(current)) return;
      window.__dbg?.(`optimize: switching ${ADDRESS_LABELS[classifyAddress(current)]} → ${ADDRESS_LABELS[classifyAddress(best)]}`);
      localStorage.setItem('tmux_address', best);
      activeAddress = best;
      disconnect();
      const token = localStorage.getItem('tmux_token') || '';
      await connect(best, token);
      serverInfo = { hostname: getHostname() || '', machineId: getMachineId() || '' };
      if (terminalTarget) wsSubscribe(terminalTarget);
    } catch {
      // Switch failed — trigger normal reconnect which will try all addresses
      reconnecting = true;
      tryReconnect();
    } finally {
      optimizing = false;
      lastProbeTime = Date.now();
    }
  }

  // Auto-reconnect and restore state on page load
  let autoConnectAttempted = false;

  // Detect app resume (Android background → foreground) + periodic optimize
  $effect(() => {
    const handler = () => {
      if (document.visibilityState !== 'visible') return;
      if (!isConnected() && !reconnecting) {
        reconnecting = true;
        tryReconnect();
      } else if (isConnected() && Date.now() - lastProbeTime > OPTIMIZE_INTERVAL_MS) {
        optimizeConnection();
      }
    };
    document.addEventListener('visibilitychange', handler);
    // Periodic check while connected
    const interval = setInterval(() => {
      if (isConnected() && Date.now() - lastProbeTime > OPTIMIZE_INTERVAL_MS) {
        optimizeConnection();
      }
    }, OPTIMIZE_INTERVAL_MS);
    return () => { document.removeEventListener('visibilitychange', handler); clearInterval(interval); };
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
      serverInfo = { hostname: getHostname() || '', machineId: getMachineId() || '' };
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
    if (Math.abs(dx) < SWIPE_MIN_DISTANCE || dy > Math.abs(dx) * SWIPE_MAX_ANGLE) return;
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
    showSettings = false;
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
        setTimeout(() => { slideAnim = ''; }, SLIDE_ANIMATION_MS);
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
        {@const mid = serverInfo.machineId}
        {@const urls = mid ? (JSON.parse(localStorage.getItem('tmux_machines') || '{}')[mid] || []) : []}
        <div class="sp-conn">
          <div class="sp-conn-row">
            <div class="sp-conn-host">{serverInfo.hostname || 'unknown'}</div>
            {#if urls.length > 1}
              {@const currentType = ADDRESS_LABELS[classifyAddress(activeAddress)]}
              <button class="sp-optimize" onclick={optimizeConnection} disabled={optimizing}>
                {#if optimizing}
                  <span class="reconnect-spinner"></span> 嗅探中
                {:else}
                  {currentType} · 嗅探
                {/if}
              </button>
            {/if}
          </div>
          {#if urls.length > 1}
            <div class="sp-conn-urls">
              {#each urls as u}
                <button class="sp-conn-url" class:sp-conn-active={u === activeAddress} onclick={() => {
                  if (u !== activeAddress) {
                    localStorage.setItem('tmux_address', u);
                    activeAddress = u;
                    showSettings = false;
                    disconnect();
                    connect(u, localStorage.getItem('tmux_token') || '').then(() => {
                      serverInfo = { hostname: getHostname() || '', machineId: getMachineId() || '' };
                      if (terminalTarget) wsSubscribe(terminalTarget);
                    }).catch(() => { reconnecting = true; tryReconnect(); });
                  }
                }}>{u}</button>
              {/each}
            </div>
          {:else}
            <div class="sp-conn-addr">{activeAddress}</div>
          {/if}
          <div class="sp-conn-id">{mid?.slice(0, 8) || '—'}</div>
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
      <div class="sp-section">
        <div class="sp-inline">
          <div class="sp-label">Font</div>
          <div class="sp-font-row">
            <button class="sp-font-btn" onclick={() => { fontSize = Math.max(8, fontSize - 1); localStorage.setItem('tmux_fontsize', fontSize); }}>−</button>
            <span class="sp-font-val">{fontSize}</span>
            <button class="sp-font-btn" onclick={() => { fontSize = Math.min(24, fontSize + 1); localStorage.setItem('tmux_fontsize', fontSize); }}>+</button>
          </div>
          <div style="flex:1"></div>
          <div class="sp-label">Debug</div>
          <button class="sp-toggle" class:on={debugMode} onclick={() => { debugMode = !debugMode; localStorage.setItem('tmux_debug', debugMode ? '1' : ''); }}>
            <span class="sp-toggle-opt sp-toggle-off">Off</span>
            <span class="sp-toggle-opt sp-toggle-on">On</span>
          </button>
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
        <Files session={terminalSession} visible={page === 'files'} {fontSize} onGoBack={(fn) => filesGoBack = fn} />
      </div>
      <div class="page-layer" class:hidden={page !== 'terminal'}>
        <Terminal target={terminalTarget} session={terminalSession} command={terminalCommand} {viewMode} {fontSize} onChatSupported={(v) => chatSupported = v} onSwitchPane={(t, cmd) => { terminalTarget = t; terminalCommand = cmd || ''; }} onPaneExit={() => { terminalTarget = ''; page = 'sessions'; }} />
      </div>
    {/if}
  </div>

  {#if debugMode}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="debug-overlay"
      ontouchstart={(e) => {
        const el = e.currentTarget;
        // Only drag from the header area (top 24px)
        const rect = el.getBoundingClientRect();
        const ty = e.touches[0].clientY - rect.top;
        if (ty > 24) return; // let content scroll/select normally
        e.preventDefault();
        const startX = e.touches[0].clientX - el.offsetLeft;
        const startY = e.touches[0].clientY - el.offsetTop;
        const onMove = (ev) => { el.style.left = (ev.touches[0].clientX - startX) + 'px'; el.style.top = (ev.touches[0].clientY - startY) + 'px'; };
        const onEnd = () => { document.removeEventListener('touchmove', onMove); document.removeEventListener('touchend', onEnd); };
        document.addEventListener('touchmove', onMove, { passive: false });
        document.addEventListener('touchend', onEnd);
      }}
    >
      <div class="debug-header">DEBUG <button onclick={() => { if (debugEl) { navigator.clipboard.writeText(debugEl.innerText).catch(() => {}); } }}>copy</button> <button onclick={() => { if (debugEl) debugEl.innerHTML = ''; }}>clear</button></div>
      <div class="debug-content" bind:this={debugEl}></div>
    </div>
  {/if}
</main>

<style>
  .debug-overlay {
    position: fixed;
    top: 50px;
    left: 4px;
    width: 65vw;
    max-height: 40vh;
    display: flex;
    flex-direction: column;
    background: rgba(0, 0, 0, 0.85);
    color: #0f0;
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: 9px;
    line-height: 1.3;
    border-radius: 6px;
    z-index: 9999;
    border: 1px solid rgba(0, 255, 0, 0.2);
    user-select: text;
    -webkit-user-select: text;
  }
  .debug-header {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 4px 8px;
    font-weight: bold;
    font-size: 10px;
    color: #0f0;
    cursor: grab;
    border-bottom: 1px solid rgba(0, 255, 0, 0.15);
    flex-shrink: 0;
    touch-action: none;
    user-select: none;
    -webkit-user-select: none;
  }
  .debug-header button {
    background: none;
    border: 1px solid rgba(0, 255, 0, 0.3);
    color: #0f0;
    font-size: 9px;
    padding: 1px 6px;
    border-radius: 3px;
    cursor: pointer;
    user-select: none;
    -webkit-user-select: none;
  }
  .debug-content {
    padding: 4px 6px;
    overflow-y: auto;
    word-break: break-all;
    flex: 1;
    min-height: 0;
    -webkit-overflow-scrolling: touch;
  }
  :global(body) {
    margin: 0;
    font-family: -apple-system, BlinkMacSystemFont, 'Inter', 'Segoe UI', sans-serif;
    background: var(--bg);
    color: var(--text);
    overflow: hidden;
    height: var(--app-height, 100dvh);
    -webkit-font-smoothing: antialiased;
    position: fixed;
    width: 100%;
    top: 0;
    left: 0;
    overscroll-behavior: none;
    -webkit-overflow-scrolling: touch;
    user-select: none;
    -webkit-user-select: none;
  }
  :global(input), :global(textarea) { user-select: text; -webkit-user-select: text; }
  :global(.preview-body), :global(.md-render), :global(.code-preview), :global(.git-diff-body), :global(.info-body) { user-select: text; -webkit-user-select: text; }
  :global(*) { box-sizing: border-box; }
  :global(html) { overflow: hidden; overscroll-behavior: none; --sat: env(safe-area-inset-top); --sab: env(safe-area-inset-bottom); --app-height: 100dvh; }
  :global(body), main, nav, .settings-panel { transition: background-color 0.3s ease, color 0.3s ease; }
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
    height: var(--app-height, 100dvh);
    max-width: 100vw;
    overflow: hidden;
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
    border-radius: 14px; padding: 6px; min-width: 240px;
    box-shadow: 0 12px 40px rgba(0,0,0,0.35);
    animation: sp-in 0.15s ease;
  }
  @keyframes sp-in { from { opacity: 0; transform: translateY(-8px) scale(0.95); } to { opacity: 1; transform: none; } }
  .sp-conn {
    padding: 12px 14px; border-bottom: 1px solid var(--border2);
    display: flex; flex-direction: column; gap: 3px;
  }
  .sp-conn-host {
    font-size: 14px; font-weight: 600; color: var(--text);
  }
  .sp-conn-addr {
    font-size: 11px; font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    color: var(--text3);
  }
  .sp-conn-id {
    font-size: 10px; font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    color: var(--text3); opacity: 0.6;
  }
  .sp-conn-row {
    display: flex; align-items: center; gap: 8px;
  }
  .sp-optimize {
    display: flex; align-items: center; gap: 4px; margin-left: auto;
    padding: 3px 8px; border: 1px solid var(--border2); border-radius: 6px;
    background: none; color: var(--text3); font-size: 10px; font-weight: 500;
    cursor: pointer; -webkit-tap-highlight-color: transparent; white-space: nowrap;
  }
  .sp-optimize:active { background: var(--accent-bg); color: var(--accent); }
  .sp-optimize:disabled { opacity: 0.5; cursor: default; }
  .sp-conn-urls {
    display: flex; flex-direction: column; gap: 2px; margin-top: 4px;
  }
  .sp-conn-url {
    font-size: 12px; font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    color: var(--text3); padding: 6px 8px; border: 1px solid var(--border2); border-radius: 6px;
    background: none; text-align: left; cursor: pointer; -webkit-tap-highlight-color: transparent;
  }
  .sp-conn-url:active { background: var(--accent-bg); }
  .sp-conn-active { color: var(--accent); border-color: var(--accent); }
  .sp-section { padding: 10px 14px; border-bottom: 1px solid var(--border2); }
  .sp-section:last-of-type { border-bottom: none; }
  .sp-label { font-size: 10px; font-weight: 600; color: var(--text3); text-transform: uppercase; letter-spacing: 0.5px; margin-bottom: 6px; }
  .sp-inline { display: flex; align-items: center; gap: 10px; }
  .sp-inline .sp-label { margin-bottom: 0; }
  .sp-btns {
    display: inline-flex; gap: 2px; background: var(--pill-bg); border-radius: 8px; padding: 2px;
  }
  .sp-btns button {
    padding: 6px 14px; border: none; border-radius: 6px; background: transparent;
    color: var(--text3); font-size: 12px; font-weight: 500; cursor: pointer;
    -webkit-tap-highlight-color: transparent; transition: all 0.15s;
  }
  .sp-btns button.active { background: var(--accent-bg); color: var(--accent); }
  .sp-font-row {
    display: flex; align-items: center; gap: 4px;
  }
  .sp-font-btn {
    width: 28px; height: 28px; border: 1px solid var(--border); border-radius: 7px;
    background: var(--pill-bg); color: var(--text2); font-size: 15px; font-weight: 600;
    cursor: pointer; display: flex; align-items: center; justify-content: center;
    -webkit-tap-highlight-color: transparent;
  }
  .sp-font-btn:active { background: var(--accent-bg); color: var(--accent); }
  .sp-font-val {
    font-size: 13px; font-weight: 600; font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; color: var(--text2);
    min-width: 24px; text-align: center;
  }
  .sp-toggle {
    display: inline-flex; gap: 2px; background: var(--pill-bg); border-radius: 8px;
    padding: 2px; border: none; cursor: pointer; -webkit-tap-highlight-color: transparent; flex-shrink: 0;
  }
  .sp-toggle-opt {
    padding: 4px 10px; border-radius: 6px; font-size: 11px; font-weight: 500;
    color: var(--text3); transition: all 0.15s;
  }
  .sp-toggle.on .sp-toggle-on { background: var(--accent-bg); color: var(--accent); }
  .sp-toggle:not(.on) .sp-toggle-off { background: var(--accent-bg); color: var(--accent); }
  .sp-disconnect {
    width: calc(100% - 12px); margin: 6px; padding: 10px; border: 1px solid var(--danger);
    border-radius: 8px; background: none; color: var(--danger);
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
