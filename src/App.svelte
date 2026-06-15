<script>
  import Settings from './lib/Settings.svelte';
  import Sessions from './lib/Sessions.svelte';
  import Terminal from './lib/Terminal.svelte';
  import SplitView from './lib/SplitView.svelte';
  import Files from './lib/Files.svelte';
  import Team from './lib/Team.svelte';
  import Icon from './lib/Icon.svelte';
  import { copyText } from './lib/clipboard.js';
  import { teamStatus } from './lib/ws.js';
  import { connect, isConnected, disconnect, setOnDisconnect, subscribe as wsSubscribe, resubscribeActive as wsResubscribeActive, getMachineId, getHostname, findBestAddress, classifyAddress, ADDRESS_LABELS, isAddressViable, noteAddressUnreachable } from './lib/ws.js';
  import { t, i18n, setLocale } from './lib/i18n.svelte.js';
  import { layout } from './lib/layout.svelte.js';

  // Tunable constants
  const KB_OPEN_THRESHOLD = 100; // px difference to detect keyboard open
  const RECONNECT_MAX_ATTEMPTS = 10;           // total attempts before giving up
  const RECONNECT_WATCHDOG_MS = 180000;        // hard cap: if still reconnecting after 3min, force reset
  const SLIDE_ANIMATION_MS = 120;
  const OPTIMIZE_INTERVAL_MS = 10 * 60 * 1000; // 10 minutes

  let page = $state('settings');
  let connected = $state(false);
  let terminalTarget = $state('');
  let terminalSession = $state('');
  let terminalCommand = $state('');
  let viewMode = $state('terminal');
  // Which working context Files should follow: a terminal pane or the team.
  let teamSession = $state('');     // active team's tmux session (reported by Team)
  let workContext = $state('terminal'); // 'terminal' | 'team'
  let filesSession = $derived(workContext === 'team' && teamSession ? teamSession : terminalSession);
  // Team (team multi-agent bus) is desktop-server-only. We probe once per
  // connection: team_status rejects with method-not-found when the server has
  // no bus, so a resolved probe means the tab should appear.
  let teamAvailable = $state(false);
  async function probeTeam() {
    try { await teamStatus(); teamAvailable = true; }
    catch { teamAvailable = false; }
  }

  // ─── Split-screen (desktop + wide only) ────────────────────────────────
  // splitLayout 1 = the single-pane path (mobile + default desktop), exactly
  // as before. 2/3/4/6 tile that many independent Terminal cells via
  // SplitView. The single `terminalTarget` above stays the source of truth
  // for the Files page, nav pills, and the narrow-screen fallback; cell 0
  // mirrors it.
  let splitLayout = $state(1);
  let splitCells = $state([]);   // [{ id, target, session, command }]
  let activeCellId = $state(null);
  let splitMenuOpen = $state(false);   // the top-right layout popover
  let nextCellId = 0;
  const SPLIT_MIN_WIDTH = 900;
  let wideEnough = $state(typeof window !== 'undefined' && window.innerWidth >= SPLIT_MIN_WIDTH);
  let splitEligible = $derived(!layout.isTouchDevice && (layout.forceDesktop || wideEnough));
  let splitActive = $derived(splitEligible && splitLayout > 1 && splitCells.length > 0);

  function setLayout(n) {
    if (!splitEligible || n <= 1) {
      splitLayout = 1;
      splitCells = [];
      activeCellId = null;
      return;
    }
    // Seed cell 0 from the current single pane; pad to n with empty cells.
    const base = splitCells.length
      ? splitCells.slice()
      : (terminalTarget
          ? [{ id: nextCellId++, target: terminalTarget, session: terminalSession, command: terminalCommand }]
          : []);
    const next = base.slice(0, n);
    while (next.length < n) next.push({ id: nextCellId++, target: '', session: '', command: '' });
    splitLayout = n;
    splitCells = next;
    if (activeCellId == null || !next.some(c => c.id === activeCellId)) {
      activeCellId = next[0]?.id ?? null;
    }
  }
  function assignCell(id, target, session, command = '') {
    splitCells = splitCells.map(c => c.id === id ? { ...c, target, session, command } : c);
    // Keep the single-pane mirror pointed at the active cell so the
    // narrow-screen fallback and Files page follow what the user is using.
    if (id === activeCellId && target) {
      terminalTarget = target; terminalSession = session; terminalCommand = command;
    }
  }
  function closeCell(id) {
    splitCells = splitCells.map(c => c.id === id ? { ...c, target: '', session: '', command: '' } : c);
  }
  function cellPaneExit(id) { closeCell(id); }

  // Re-subscribe after a reconnect / address switch: the server forgot all
  // subscriptions, so re-send the wire `subscribe` for every target with a
  // live refcount. This does NOT touch refcounts (the Terminals are still
  // mounted — only the server-side state was lost), so it's safe to call on
  // every reconnect without leaking counts.
  function resubscribeAll() {
    wsResubscribeActive();
  }
  // Chat view is disabled (placeholder kept so parser / ChatView code still
  // compiles, to be re-enabled later if wanted). While this is false the
  // chat tab, tab swipe target, and auto-switch effect are all hidden.
  const chatSupported = false;
  let theme = $state(localStorage.getItem('tmux_theme') || 'system');
  let fontSize = $state(parseInt(localStorage.getItem('tmux_fontsize')) || 14);
  const FONT_MIN = 6, FONT_MAX = 40;
  // Single source of truth for font-size changes (settings panel + the
  // desktop cmd/ctrl +/- shortcut both route through here). Changing
  // fontSize flows to Terminal as a prop, which re-fits xterm's cell
  // geometry properly — unlike browser page zoom (cmd +/-), which scales
  // the whole page without telling xterm to re-measure, leaving the cell
  // grid misaligned (the "height looks wrong" bug).
  function setFontSize(n) {
    const v = Math.max(FONT_MIN, Math.min(FONT_MAX, n));
    if (v === fontSize) return;
    fontSize = v;
    localStorage.setItem('tmux_fontsize', v);
  }
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

  // Cmd+/- zoom for desktop app
  if (window.__TAURI_INTERNALS__) {
    let zoomLevel = parseFloat(localStorage.getItem('tmux_zoom') || '1');
    document.documentElement.style.zoom = zoomLevel;
    document.addEventListener('keydown', (e) => {
      if (!e.metaKey && !e.ctrlKey) return;
      if (e.key === '=' || e.key === '+') { e.preventDefault(); zoomLevel = Math.min(2, zoomLevel + 0.1); }
      else if (e.key === '-') { e.preventDefault(); zoomLevel = Math.max(0.5, zoomLevel - 0.1); }
      else if (e.key === '0') { e.preventDefault(); zoomLevel = 1; }
      else return;
      document.documentElement.style.zoom = zoomLevel;
      localStorage.setItem('tmux_zoom', zoomLevel);
    });
  }

  // Intercept external link clicks → open in system browser instead of in-app navigation
  document.addEventListener('click', (e) => {
    const a = e.target.closest('a[href]');
    if (!a) return;
    const href = a.getAttribute('href');
    if (href && /^https?:\/\//.test(href)) {
      e.preventDefault();
      if (window.__TAURI_INTERNALS__) {
        import('@tauri-apps/plugin-opener').then(m => m.openUrl(href)).catch(() => window.open(href, '_blank'));
      } else {
        window.open(href, '_blank');
      }
    }
  });

  // Keyboard height detection
  $effect(() => {
    // Android Tauri app: native event provides exact keyboard height
    let androidNativeKb = false;
    const nativeHandler = (e) => {
      androidNativeKb = true; // suppress visualViewport handler on Android
      const kbh = e.detail?.height || 0;
      // Guard: ignore keyboard-open events when no text input is focused
      // (Android OnGlobalLayoutListener can fire stale heights during layout transitions)
      const activeTag = document.activeElement?.tagName;
      if (kbh > 0 && activeTag !== 'TEXTAREA' && activeTag !== 'INPUT') {
        window.__dbg?.(`androidKb: IGNORED kbh=${kbh} (activeEl=${activeTag})`);
        return;
      }
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
      // No explicit refit needed — updating --app-height changes the terminal
      // container size, which Terminal.svelte's ResizeObserver picks up.
    };
    window.addEventListener('androidKeyboardHeight', nativeHandler);

    // Track the max known height (= full screen without keyboard)
    let fullHeight = window.innerHeight;
    window.__fullHeight = () => fullHeight;

    // Mobile browser: track visualViewport height so main always fits
    // the visible area (keyboard doesn't push nav off screen).
    //
    // Desktop must NOT run this. A desktop window still has a
    // window.visualViewport, and resizing/zooming the window fires its
    // `resize` event — pinning `--app-height` to vv.height there both
    // (a) false-trips the `kbOpen` heuristic when the window shrinks past
    // the threshold, and (b) under page zoom hands back a scaled height
    // that diverges from the real layout box, so the terminal computes the
    // wrong row count (width was unaffected because it's pure flex). On
    // desktop we leave `--app-height` at its CSS default (100dvh) and let
    // the terminal's ResizeObserver refit on window resize.
    const isTouch = 'ontouchstart' in window || navigator.maxTouchPoints > 0;
    const vv = isTouch ? window.visualViewport : null;
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
      // No explicit refit needed — --app-height already reflects the
      // visible viewport, so Terminal.svelte's ResizeObserver picks up
      // the container size change on both keyboard open and close.
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

  // Track window width so split-screen collapses to single below the
  // threshold (and re-enables when widened). Cheap; desktop only matters.
  //
  // ALSO drive --app-height from window.innerHeight on desktop. The CSS
  // default is `100dvh`, but macOS WKWebView does NOT recompute `dvh` when
  // the window is enlarged — it stays pinned at the smaller value, so the
  // layout box never grows, the terminal's ResizeObserver never fires, and a
  // black gap opens below the terminal. (Chromium recomputes dvh, which is
  // why the browser is fine.) Writing innerHeight on every resize makes the
  // layout box track the window exactly, which the ResizeObserver then picks
  // up to refit xterm. Mobile is excluded: its height is owned by the
  // visualViewport handler above (keyboard-aware), and touching it here would
  // fight that handler.
  $effect(() => {
    const isTouch = 'ontouchstart' in window || navigator.maxTouchPoints > 0;
    const onResize = () => {
      wideEnough = window.innerWidth >= SPLIT_MIN_WIDTH;
      if (!isTouch) document.documentElement.style.setProperty('--app-height', window.innerHeight + 'px');
    };
    onResize(); // set the correct height on mount, not just on the first resize
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });

  // Desktop: route cmd/ctrl +/-/0 to the app's font-size logic instead of
  // letting the WebView page-zoom. Page zoom scales the DOM without telling
  // xterm to re-measure its cell grid, so the terminal renders with a
  // mismatched cell height/width (the "height is wrong after cmd+-" bug).
  // Driving fontSize re-fits xterm correctly via the Terminal prop. Mobile
  // has no such shortcut, so this only matters on desktop.
  $effect(() => {
    const isTouch = 'ontouchstart' in window || navigator.maxTouchPoints > 0;
    if (isTouch) return;
    const onKey = (e) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
      // Equals/Plus (zoom in), Minus (zoom out), 0 (reset). Match by key so
      // it works across layouts; include numpad variants.
      if (e.key === '=' || e.key === '+') {
        e.preventDefault(); setFontSize(fontSize + 1); // reads current $state
      } else if (e.key === '-' || e.key === '_') {
        e.preventDefault(); setFontSize(fontSize - 1);
      } else if (e.key === '0') {
        e.preventDefault(); setFontSize(14);
      }
    };
    // Capture phase so we beat the WebView's built-in zoom handler.
    window.addEventListener('keydown', onKey, { capture: true });
    return () => window.removeEventListener('keydown', onKey, { capture: true });
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

  // Persist nav state for restore on reload. splitLayout/splitCells are only
  // meaningful on desktop; a desktop-saved state degrades to single-pane on a
  // phone because restore re-gates on splitEligible.
  $effect(() => {
    if (connected && terminalTarget) {
      localStorage.setItem('tmux_state', JSON.stringify({
        page, viewMode, terminalTarget, terminalSession, terminalCommand,
        splitLayout, splitCells
      }));
    }
  });

  let manualDisconnect = false;

  let reconnecting = $state(false);
  let reconnectAttempt = $state(0);   // 1-indexed when visible; 0 means not attempting
  let reconnectClass = $state('');    // LAN / Tailscale / WAN label for the current try
  let reconnectTimer = null;
  let reconnectWatchdog = null;

  function clearReconnectTimers() {
    clearTimeout(reconnectTimer);
    clearTimeout(reconnectWatchdog);
    reconnectTimer = null;
    reconnectWatchdog = null;
    reconnectAttempt = 0;
    reconnectClass = '';
  }

  function armReconnectWatchdog() {
    // Hard cap: if reconnecting never finishes (stuck promise, platform WebSocket hang),
    // force-reset to settings so user can escape without killing the app.
    clearTimeout(reconnectWatchdog);
    reconnectWatchdog = setTimeout(() => {
      if (!reconnecting) return;
      window.__dbg?.('reconnect: watchdog fired — force reset');
      reconnecting = false;
      clearTimeout(reconnectTimer);
      connected = false;
      page = 'settings';
    }, RECONNECT_WATCHDOG_MS);
  }

  setOnDisconnect(() => {
    if (manualDisconnect) {
      manualDisconnect = false;
      connected = false;
      return;
    }
    // Keep connected=true during reconnect to avoid UI flicker
    reconnecting = true;
    armReconnectWatchdog();
    tryReconnect();
  });

  function cancelReconnect() {
    reconnecting = false;
    reconnectAttempt = 0;
    reconnectClass = '';
    connected = false;
    clearReconnectTimers();
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

  function onReconnectSuccess(useAddr, primaryAddr) {
    connected = true;
    reconnecting = false;
    clearReconnectTimers();
    window.__dbg?.('reconnect: success');
    serverInfo = { hostname: getHostname() || '', machineId: getMachineId() || '' };
    if (useAddr !== primaryAddr) { localStorage.setItem('tmux_address', useAddr); activeAddress = useAddr; }
    resubscribeAll();
    probeTeam();
    // Tell Terminal to reset stale resize state + re-fit against the new server.
    window.dispatchEvent(new Event('ws-reconnected'));
  }

  async function tryReconnect(attempt = 0) {
    if (!reconnecting) return;
    const primary = localStorage.getItem('tmux_address');
    const token = localStorage.getItem('tmux_token') || '';
    if (!primary) { reconnecting = false; clearReconnectTimers(); connected = false; page = 'settings'; return; }

    const allAddrs = [primary, ...getAltAddresses()];
    let useAddr;

    // First attempt with multiple candidates: parallel probe → pick first reachable.
    // Avoids burning 3s × N timeouts cycling through dead addresses serially.
    if (attempt === 0 && allAddrs.length > 1) {
      window.__dbg?.(`reconnect: probing ${allAddrs.length} addresses in parallel`);
      try {
        const best = await findBestAddress(allAddrs);
        if (!reconnecting) return; // cancelled mid-probe
        useAddr = best || allAddrs[0];
      } catch {
        useAddr = allAddrs[0];
      }
    } else {
      // Round-robin, but skip addresses that recently failed a probe or
      // connect (LAN/Tailscale IPs while on cellular keep failing until a
      // network change, which clears the memory in ws.js). If everything
      // is in cooldown, fall back to plain round-robin — a total outage
      // shouldn't stop us from retrying at all.
      const viable = allAddrs.filter(isAddressViable);
      const pool = viable.length > 0 ? viable : allAddrs;
      useAddr = pool[attempt % pool.length];
    }

    window.__dbg?.(`reconnect: attempt ${attempt + 1}/${RECONNECT_MAX_ATTEMPTS} → ${useAddr}`);
    reconnectAttempt = attempt + 1;
    reconnectClass = ADDRESS_LABELS[classifyAddress(useAddr)] || '';

    // Per-attempt connect timeout scales with address class: LAN is fast and
    // should fail fast; WAN (public internet, slow cellular, far regions)
    // legitimately needs more time for TCP + TLS handshake.
    const cls = classifyAddress(useAddr);
    const attemptTimeout = cls === 0 ? 2000 : cls === 1 ? 3000 : 5000;

    connect(useAddr, token, attemptTimeout).then(() => {
      if (!reconnecting) return;
      onReconnectSuccess(useAddr, primary);
    }).catch((e) => {
      if (!reconnecting) return;
      window.__dbg?.(`reconnect: failed (${e.message})`);
      // Reachability failures (timeout / refused, NOT auth errors) feed the
      // same cooldown memory the prober uses, so the next attempts skip
      // this address instead of re-burning its timeout.
      if (/timeout|connection failed|closed during auth/i.test(e.message || '')) {
        noteAddressUnreachable(useAddr);
      }
      if (attempt + 1 < RECONNECT_MAX_ATTEMPTS) {
        const delay = Math.min(500 * (attempt + 1), 3000); // tighter backoff since timeouts are short
        reconnectTimer = setTimeout(() => tryReconnect(attempt + 1), delay);
      } else {
        window.__dbg?.('reconnect: gave up');
        reconnecting = false;
        clearReconnectTimers();
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
    probeTeam();
  }

  function openTerminal(session, target, command = '') {
    terminalSession = session;
    terminalTarget = target;
    terminalCommand = command;
    workContext = 'terminal';
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
    // Only probe addresses that would be a strict UPGRADE over the current
    // class (LAN < Tailscale < WAN). Probing peers/downgrades is pure
    // waste: we'd never switch to them (the check below rejects ≥ current
    // class), and when the phone is on WAN the LAN/Tailscale addresses are
    // unreachable — each probe is a phantom connection attempt hanging
    // until its 3 s timeout, polluting logs and radio wakeups every cycle.
    const curClass = classifyAddress(current);
    if (curClass === 0) { lastProbeTime = Date.now(); return; } // already best
    const candidates = addrs.filter(a => classifyAddress(a) < curClass);
    if (candidates.length === 0) { lastProbeTime = Date.now(); return; }
    optimizing = true;
    try {
      const best = await findBestAddress(candidates);
      if (!best || best === current) return;
      // Only switch if the new address is higher priority (lower class number)
      if (classifyAddress(best) >= curClass) return;
      window.__dbg?.(`optimize: switching ${ADDRESS_LABELS[classifyAddress(current)]} → ${ADDRESS_LABELS[classifyAddress(best)]}`);
      localStorage.setItem('tmux_address', best);
      activeAddress = best;
      disconnect();
      const token = localStorage.getItem('tmux_token') || '';
      await connect(best, token);
      serverInfo = { hostname: getHostname() || '', machineId: getMachineId() || '' };
      resubscribeAll();
      window.dispatchEvent(new Event('ws-reconnected'));
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
      probeTeam();
      try {
        const s = JSON.parse(localStorage.getItem('tmux_state') || '{}');
        if (s.terminalTarget) {
          terminalTarget = s.terminalTarget;
          terminalSession = s.terminalSession || '';
          terminalCommand = s.terminalCommand || '';
          page = s.page || 'terminal';
          viewMode = 'terminal';
          // Restore split layout only on eligible (desktop + wide) clients;
          // a desktop-saved state silently stays single-pane on a phone.
          if (splitEligible && s.splitLayout > 1 && Array.isArray(s.splitCells) && s.splitCells.length) {
            splitCells = s.splitCells;
            splitLayout = s.splitLayout;
            nextCellId = Math.max(0, ...s.splitCells.map(c => c.id ?? 0)) + 1;
            activeCellId = s.splitCells[0]?.id ?? null;
          }
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
      if (page === 'terminal' || page === 'team') { page = 'sessions'; return; }
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
    if (teamAvailable) t.push('team');
    if (terminalTarget) {
      t.push('terminal');
      if (chatSupported) t.push('chat');
      t.push('files');
    }
    return t;
  });

  let slideAnim = $state('');

  function switchTab(target) {
    if (slideAnim) return;
    showSettings = false;
    const t = tabs();
    const curName = page === 'terminal' ? (viewMode === 'chat' ? 'chat' : 'terminal') : page;
    if (target === curName) return;
    const fromIdx = t.indexOf(curName);
    const toIdx = t.indexOf(target);
    // Apply page change immediately
    if (target === 'chat') { page = 'terminal'; viewMode = 'chat'; workContext = 'terminal'; }
    else if (target === 'terminal') { page = 'terminal'; viewMode = 'terminal'; workContext = 'terminal'; }
    else { page = target; if (target === 'team') workContext = 'team'; }
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
        <button tabindex="-1" class:active={page === 'sessions'} onclick={() => switchTab('sessions')}>
          {t('sessions')}
        </button>
        {#if terminalTarget}
          <button tabindex="-1" class:active={page === 'terminal' && viewMode === 'terminal'} onclick={() => switchTab('terminal')}>
            {t('terminal')}
          </button>
        {/if}
        {#if terminalTarget && chatSupported}
          <button tabindex="-1" class:active={page === 'terminal' && viewMode === 'chat'} onclick={() => switchTab('chat')}>
            {t('chat')}
          </button>
        {/if}
        {#if teamAvailable}
          <button tabindex="-1" class:active={page === 'team'} onclick={() => switchTab('team')}>
            {t('team')}
          </button>
        {/if}
        {#if terminalTarget}
          <button tabindex="-1" class:active={page === 'files'} onclick={() => switchTab('files')}>
            {t('files')}
          </button>
        {/if}
      </div>
      <div class="nav-right">
        <button tabindex="-1" class="gear-btn" onclick={() => showSettings = !showSettings}><Icon name="gear" size={16} /></button>
      </div>
    {:else}
      <div class="brand">
        <img class="logo" src={iconSrc} alt="" width="24" height="24" />
        <span class="brand-text">tmux<span class="brand-accent">mobile</span></span>
      </div>
      <div class="nav-right">
        <button tabindex="-1" class="gear-btn" onclick={() => showSettings = !showSettings}><Icon name="gear" size={16} /></button>
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
                  <span class="reconnect-spinner"></span> {t('sniffing')}
                {:else}
                  {currentType} · {t('sniff')}
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
                      resubscribeAll();
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
      <div class="sp-rows">
        <div class="sp-row">
          <span class="sp-label">{t('theme')}</span>
          <div class="sp-btns">
            <button class:active={theme === 'system'} onclick={() => setTheme('system')}>{t('themeAuto')}</button>
            <button class:active={theme === 'light'} onclick={() => setTheme('light')}>{t('themeLight')}</button>
            <button class:active={theme === 'dark'} onclick={() => setTheme('dark')}>{t('themeDark')}</button>
          </div>
        </div>
        <div class="sp-row">
          <span class="sp-label">{t('language')}</span>
          <div class="sp-btns">
            <button class:active={i18n.lang === 'en'} onclick={() => setLocale('en')}>EN</button>
            <button class:active={i18n.lang === 'zh'} onclick={() => setLocale('zh')}>中文</button>
          </div>
        </div>
        <div class="sp-row">
          <span class="sp-label">{t('font')}</span>
          <div class="sp-font-row">
            <button class="sp-font-btn" onclick={() => setFontSize(fontSize - 1)}>−</button>
            <span class="sp-font-val">{fontSize}</span>
            <button class="sp-font-btn" onclick={() => setFontSize(fontSize + 1)}>+</button>
          </div>
        </div>
        <div class="sp-row">
          <span class="sp-label">{t('layout')}</span>
          <div class="sp-btns">
            <button class:active={layout.mode === 'auto'} onclick={() => layout.set('auto')}>{t('layoutAuto')}</button>
            <button class:active={layout.mode === 'desktop'} onclick={() => layout.set('desktop')}>{t('layoutDesktop')}</button>
            <button class:active={layout.mode === 'mobile'} onclick={() => layout.set('mobile')}>{t('layoutMobile')}</button>
          </div>
        </div>
        <div class="sp-row">
          <span class="sp-label">{t('debug')}</span>
          <button class="sp-toggle" class:on={debugMode} onclick={() => { debugMode = !debugMode; localStorage.setItem('tmux_debug', debugMode ? '1' : ''); }}>
            <span class="sp-toggle-opt sp-toggle-off">{t('off')}</span>
            <span class="sp-toggle-opt sp-toggle-on">{t('on')}</span>
          </button>
        </div>
      </div>
      {#if connected}
      <button class="sp-disconnect" onclick={() => { showSettings = false; doDisconnect(); }}>{t('disconnect')}</button>
      {/if}
    </div>
    <button class="sp-overlay" onclick={() => showSettings = false} aria-label="Close settings"></button>
  {/if}

  {#if reconnecting && page !== 'settings'}
    <div class="reconnect-bar">
      <span class="reconnect-spinner"></span>
      <span>{t('reconnecting')}{#if reconnectAttempt} ({reconnectAttempt}/{RECONNECT_MAX_ATTEMPTS}{#if reconnectClass} · {reconnectClass}{/if}){/if}</span>
      <button class="reconnect-cancel" onclick={cancelReconnect}>{t('cancel')}</button>
    </div>
  {/if}

  <div class="page {slideAnim}" class:page-terminal={page === 'terminal'}>
    {#if page === 'settings'}
      <Settings {onConnected} />
    {:else if page === 'sessions'}
      <Sessions {openTerminal} activeTarget={terminalTarget} visible={page === 'sessions'} />
    {:else if page === 'team'}
      <Team visible={page === 'team'} currentSession={terminalSession} {fontSize} openTerminal={(s, tgt, cmd) => openTerminal(s, tgt, cmd)} onTeamSession={(s) => teamSession = s} />
    {/if}
    {#if terminalTarget}
      <div class="page-layer" class:hidden={page !== 'files'}>
        <Files session={filesSession} visible={page === 'files'} {fontSize} onGoBack={(fn) => filesGoBack = fn} />
      </div>
      <div class="page-layer" class:hidden={page !== 'terminal'}>
        <div class="terminal-body" class:split-capable={splitEligible && viewMode === 'terminal'}>
          {#if splitEligible && viewMode === 'terminal'}
            <!-- Split-layout control: a single floating icon (top-right) that
                 opens a small popover, instead of a full-width toolbar row. -->
            <div class="split-control">
              <button class="split-toggle" class:on={splitActive} title={t('split')} onclick={() => splitMenuOpen = !splitMenuOpen}>
                <Icon name="layout" size={15} />
              </button>
              {#if splitMenuOpen}
                <div class="split-menu">
                  {#each [1, 2, 3, 4, 6] as n}
                    <button class="split-opt" class:active={(n === 1 && !splitActive) || (splitActive && splitLayout === n)} onclick={() => { setLayout(n); splitMenuOpen = false; }}>{n}</button>
                  {/each}
                </div>
                <button class="split-menu-backdrop" aria-label="close" onclick={() => splitMenuOpen = false}></button>
              {/if}
            </div>
          {/if}
          {#if splitActive}
            <SplitView cells={splitCells} layout={splitLayout} {activeCellId} {fontSize}
              onActivate={(id) => activeCellId = id}
              onAssign={assignCell}
              onCloseCell={closeCell}
              onPaneExit={cellPaneExit} />
          {:else}
            <Terminal target={terminalTarget} session={terminalSession} command={terminalCommand} {viewMode} {fontSize} onSwitchPane={(t, cmd) => { terminalTarget = t; terminalSession = t.split(':')[0]; terminalCommand = cmd || ''; }} onPaneExit={() => { terminalTarget = ''; page = 'sessions'; }} />
          {/if}
        </div>
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
      <div class="debug-header">DEBUG <button onclick={() => { if (debugEl) copyText(debugEl.innerText); }}>copy</button> <button onclick={() => { if (debugEl) debugEl.innerHTML = ''; }}>clear</button> <button onclick={() => { debugMode = false; localStorage.removeItem('tmux_debug'); }}>✕</button></div>
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
    font-family: var(--font-mono);
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
    font-family: var(--font-ui);
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
  /* Form controls don't inherit font-family by default (UA gives them Arial/
     system), so without this every <button>/<input>/<select> would ignore
     --font-ui. Inherit it; the few controls that must be monospace (command
     inputs, path fields, code editor) set --font-mono explicitly themselves. */
  :global(button), :global(input), :global(textarea), :global(select) { font-family: inherit; }
  :global(.preview-body), :global(.md-render), :global(.code-preview), :global(.git-diff-body), :global(.info-body), :global(.bubble) { user-select: text; -webkit-user-select: text; }
  :global(*) { box-sizing: border-box; }
  :global(html) {
    overflow: hidden; overscroll-behavior: none;
    --sat: env(safe-area-inset-top); --sab: env(safe-area-inset-bottom); --app-height: 100dvh;
    /* Two font roles. --font-mono: code, terminal output, file paths, data —
       fixed-width matters (alignment, glyph identity). Includes the bundled
       Maple Mono (Latin) + Maple Mono CJK so code/paths render identically on
       every device. --font-ui: everything else — labels, buttons, chat prose,
       Chinese body text — proportional, using each platform's native UI font
       (PingFang / YaHei / Noto Sans CJK for Chinese). Never put the monospace
       CJK face in --font-ui: it makes Chinese UI text look like code. */
    --font-mono: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace;
    --font-ui: -apple-system, BlinkMacSystemFont, 'Inter', 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', 'Noto Sans CJK SC', 'Noto Sans SC', sans-serif;
  }
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
    padding: 12px 14px; margin-bottom: 4px; border-bottom: 1px solid var(--border2);
    display: flex; flex-direction: column; gap: 3px;
  }
  .sp-conn-host {
    font-size: 14px; font-weight: 600; color: var(--text);
  }
  .sp-conn-addr {
    font-size: 11px; font-family: var(--font-mono);
    color: var(--text3);
  }
  .sp-conn-id {
    font-size: 10px; font-family: var(--font-mono);
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
    font-size: 12px; font-family: var(--font-mono);
    color: var(--text3); padding: 6px 8px; border: 1px solid var(--border2); border-radius: 6px;
    background: none; text-align: left; cursor: pointer; -webkit-tap-highlight-color: transparent;
  }
  .sp-conn-url:active { background: var(--accent-bg); }
  .sp-conn-active { color: var(--accent); border-color: var(--accent); }
  /* Settings rows: one item per line, label left, control right-aligned.
     A shared row grid keeps every control's right edge flush so the panel
     reads as an aligned table rather than a stack of ad-hoc layouts. */
  .sp-rows { padding: 6px; display: flex; flex-direction: column; }
  .sp-row {
    display: flex; align-items: center; justify-content: space-between;
    gap: 12px; min-height: 40px; padding: 4px 8px;
  }
  .sp-row + .sp-row { border-top: 1px solid var(--border2); }
  .sp-label {
    font-size: 11px; font-weight: 600; color: var(--text3);
    text-transform: uppercase; letter-spacing: 0.5px; white-space: nowrap;
  }
  .sp-btns {
    display: inline-flex; gap: 2px; background: var(--pill-bg); border-radius: 8px; padding: 2px;
  }
  .sp-btns button {
    padding: 6px 12px; border: none; border-radius: 6px; background: transparent;
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
    font-size: 13px; font-weight: 600; font-family: var(--font-mono); color: var(--text2);
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

  .terminal-body { flex: 1; min-height: 0; position: relative; display: flex; flex-direction: column; }

  /* Split-layout control: a single floating icon in the terminal's top-right
     corner (no full-width toolbar row). Opens a small popover with 1/2/3/4/6. */
  .split-control { position: absolute; top: 6px; right: 8px; z-index: 12; }
  .split-toggle {
    width: 28px; height: 28px; padding: 0;
    border: 1px solid var(--border); border-radius: 8px;
    background: var(--surface); color: var(--text3);
    cursor: pointer; display: flex; align-items: center; justify-content: center;
    -webkit-tap-highlight-color: transparent;
    backdrop-filter: blur(8px); -webkit-backdrop-filter: blur(8px);
  }
  .split-toggle:hover { color: var(--text2); }
  .split-toggle.on { color: var(--accent); border-color: var(--accent); background: var(--accent-bg); }
  .split-menu {
    position: absolute; top: 34px; right: 0; z-index: 13;
    display: flex; gap: 2px; padding: 3px;
    background: var(--bg); border: 1px solid var(--border); border-radius: 10px;
    box-shadow: 0 8px 28px rgba(0,0,0,0.35);
  }
  .split-opt {
    min-width: 28px; height: 28px;
    border: none; border-radius: 6px; background: transparent;
    color: var(--text3); font-size: 13px; font-weight: 600; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .split-opt:hover { background: var(--surface2); color: var(--text2); }
  .split-opt.active { background: var(--accent-bg); color: var(--accent); }
  .split-menu-backdrop { position: fixed; inset: 0; z-index: 12; background: transparent; border: none; cursor: default; }
</style>
