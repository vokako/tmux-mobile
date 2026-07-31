<script>
  import Settings from './lib/app/Settings.svelte';
  import Sessions from './lib/sessions/Sessions.svelte';
  import Terminal from './lib/terminal/Terminal.svelte';
  import SplitView from './lib/sessions/SplitView.svelte';
  import Files from './lib/files/Files.svelte';
  import Team from './lib/team/Team.svelte';
  import Hub from './lib/hub/Hub.svelte';
  import Icon from './lib/ui/Icon.svelte';
  import InstallPrompt from './lib/ui/InstallPrompt.svelte';
  import Preferences from './lib/app/Preferences.svelte';
  import { copyText } from './lib/core/clipboard.ts';
  import { teamStatus } from './lib/core/ws.ts';
  import { connect, isConnected, disconnect, setOnDisconnect, subscribe as wsSubscribe, resubscribeActive as wsResubscribeActive, getMachineId, getHostname, findBestAddress, classifyAddress, ADDRESS_LABELS, isAddressViable, noteAddressUnreachable, listPanes } from './lib/core/ws.ts';
  import { t } from './lib/core/i18n.svelte.ts';
  import { layout } from './lib/app/layout.svelte.ts';
  import { teamState } from './lib/core/team.svelte.ts';
  import { applyMonoVar } from './lib/app/fonts.svelte.ts';
  import { normalizeUiZoom, stepUiZoom, UI_ZOOM_DEFAULT } from './lib/app/ui-zoom.ts';
  import { createReconnectMachine } from './lib/app/reconnect.ts';
  import { cycleItem, shortcutFromEvent } from './lib/app/shortcuts.ts';
  import { isShortcutInputTarget, shortcuts } from './lib/app/shortcuts.svelte.ts';
  import { markWindowRead, stopAgentNotifications, syncAgentNotifications } from './lib/core/agent-notifications.svelte.ts';
  import { installExternalLinkHandler } from './lib/core/external-links.ts';

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
  // Which working context Files should follow: a terminal pane or the team.
  let teamSession = $state('');     // active team's tmux session (reported by Team)
  let workContext = $state('terminal'); // 'terminal' | 'team'
  let filesSession = $derived(workContext === 'team' && teamSession ? teamSession : terminalSession);
  // Team (team multi-agent bus) is desktop-server-only. We probe once per
  // connection: team_status rejects with method-not-found when the server has
  // no bus, so a resolved probe means the tab should appear.
  // Availability lives in the shared teamState (team.svelte.js) so session
  // classification (Sessions, PanePicker) uses the same gate as the tab.
  let teamAvailable = $derived(teamState.available);
  // Imperative handle on the always-mounted Team component (bind:this), so the
  // Sessions page can jump straight to a given team's chat via its exported
  // selectTeam(). A function call (not a prop change) so clicking the same team
  // session twice still re-selects it; nulled automatically on unmount.
  let teamRef = $state(null);
  async function probeTeam() {
    try { await teamStatus(); teamState.available = true; teamState.probed = true; }
    catch (e) {
      // Only a definitive server answer (method-not-found: no team bus) may
      // flip the flag off. Transient failures (RPC timeout, reconnect blip)
      // keep the current value — flipping to false unmounts the always-mounted
      // Team component and destroys the state it exists to preserve.
      if (e?.code === -32601) { teamState.available = false; teamState.probed = true; }
    }
  }
  // The Team page-layer only mounts when teamAvailable, so page === 'team'
  // without it would render an empty main area (the state restore sets `page`
  // before the probe resolves, and a reconnect can land on a busless server).
  // Once the probe has definitively answered "no bus", fall back to Sessions.
  // While the probe is still pending we leave `page` alone — a brief blank
  // beats kicking the user off the tab they were on.
  $effect(() => {
    if (page === 'team' && teamState.probed && !teamState.available) page = 'sessions';
  });

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
  // The Hub is the desktop three-column view: needs a wide non-touch client
  // AND the server-side bus (hub_* degrades method-not-found without it, same
  // probe as Team).
  let hubEligible = $derived(splitEligible && teamAvailable);
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
  let theme = $state(localStorage.getItem('tmux_theme') || 'system');
  let fontSize = $state(parseInt(localStorage.getItem('tmux_fontsize')) || 14);
  const isTauriDesktop = !!window.__TAURI_INTERNALS__ && !/android/i.test(navigator.userAgent);
  const initialUiZoom = normalizeUiZoom(localStorage.getItem('tmux_ui_zoom'));
  let uiZoom = $state(initialUiZoom);
  let zoomApplyVersion = 0;
  let zoomApplyQueue = Promise.resolve();
  const currentWebview = isTauriDesktop
    ? import('@tauri-apps/api/webview').then(({ getCurrentWebview }) => getCurrentWebview())
    : null;
  const FONT_MIN = 6, FONT_MAX = 40;
  // Terminal font size is independent from desktop UI zoom. Changing it
  // flows to every Terminal instance and triggers xterm's deferred re-fit.
  function setFontSize(n) {
    const v = Math.max(FONT_MIN, Math.min(FONT_MAX, n));
    if (v === fontSize) return;
    fontSize = v;
    localStorage.setItem('tmux_fontsize', v);
  }
  async function setUiZoom(value) {
    if (!isTauriDesktop) return;
    const next = normalizeUiZoom(value);
    uiZoom = next;
    localStorage.setItem('tmux_ui_zoom', String(next));
    const version = ++zoomApplyVersion;
    zoomApplyQueue = zoomApplyQueue.then(async () => {
      try {
        const webview = await currentWebview;
        await webview.setZoom(next);
        if (version !== zoomApplyVersion) return;
        requestAnimationFrame(() => requestAnimationFrame(() => {
          document.documentElement.style.setProperty('--app-height', window.innerHeight + 'px');
          window.dispatchEvent(new CustomEvent('app-zoom-change', { detail: { scale: next } }));
        }));
      } catch (error) {
        if (version === zoomApplyVersion) {
          uiZoom = UI_ZOOM_DEFAULT;
          localStorage.setItem('tmux_ui_zoom', String(UI_ZOOM_DEFAULT));
        }
        window.__dbg?.('zoom: failed ' + error);
      }
    });
    await zoomApplyQueue;
  }

  if (isTauriDesktop) setUiZoom(initialUiZoom);

  let showSettings = $state(false);
  // Apply the persisted custom terminal font (if any) before first paint of
  // the terminal — rewrites --font-mono inline; a no-op for the default.
  applyMonoVar();
  let serverInfo = $state({ hostname: '', machineId: '' });
  let activeAddress = $state(localStorage.getItem('tmux_address') || '');
  let prefAddresses = $derived.by(() => {
    if (!serverInfo.machineId) return [];
    try {
      const machines = JSON.parse(localStorage.getItem('tmux_machines') || '{}');
      return Array.isArray(machines[serverInfo.machineId]) ? machines[serverInfo.machineId] : [];
    } catch {
      return [];
    }
  });
  let debugMode = $state(!!localStorage.getItem('tmux_debug'));
  let debugEl = $state(null);
  const DEBUG_POSITION_KEY = 'tmux_debug_position';
  let debugPosition = $state((() => {
    try {
      const value = JSON.parse(localStorage.getItem(DEBUG_POSITION_KEY) || 'null');
      return Number.isFinite(value?.left) && Number.isFinite(value?.top) ? value : null;
    } catch { return null; }
  })());

  function clampDebugPosition(element, left, top) {
    const rect = element.getBoundingClientRect();
    const margin = 4;
    return {
      left: Math.max(margin, Math.min(left, window.innerWidth - rect.width - margin)),
      top: Math.max(margin, Math.min(top, window.innerHeight - 28 - margin)),
    };
  }

  function keepDebugPanelVisible(element) {
    const clamp = () => {
      if (!debugPosition) return;
      const next = clampDebugPosition(element, debugPosition.left, debugPosition.top);
      if (next.left === debugPosition.left && next.top === debugPosition.top) return;
      debugPosition = next;
      localStorage.setItem(DEBUG_POSITION_KEY, JSON.stringify(next));
    };
    requestAnimationFrame(clamp);
    window.addEventListener('resize', clamp);
    window.addEventListener('app-zoom-change', clamp);
    return {
      destroy() {
        window.removeEventListener('resize', clamp);
        window.removeEventListener('app-zoom-change', clamp);
      },
    };
  }

  function startDebugPointerDrag(event) {
    if (event.pointerType === 'touch' || event.button !== 0 || event.target.closest('button')) return;
    const element = event.currentTarget.parentElement;
    const rect = element.getBoundingClientRect();
    const offsetX = event.clientX - rect.left;
    const offsetY = event.clientY - rect.top;
    const onMove = (moveEvent) => {
      if (moveEvent.buttons === 0) { onEnd(); return; }
      debugPosition = clampDebugPosition(element, moveEvent.clientX - offsetX, moveEvent.clientY - offsetY);
    };
    const onEnd = () => {
      document.removeEventListener('pointermove', onMove);
      document.removeEventListener('pointerup', onEnd);
      document.removeEventListener('pointercancel', onEnd);
      if (debugPosition) localStorage.setItem(DEBUG_POSITION_KEY, JSON.stringify(debugPosition));
    };
    event.preventDefault();
    document.addEventListener('pointermove', onMove);
    document.addEventListener('pointerup', onEnd);
    document.addEventListener('pointercancel', onEnd);
  }

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

  // Remove the legacy CSS zoom. Desktop UI scaling now uses WKWebView's
  // native pageZoom through Tauri, so viewport geometry and visual scale stay
  // in the same coordinate system.
  if (window.__TAURI_INTERNALS__) {
    document.documentElement.style.zoom = '';
    localStorage.removeItem('tmux_zoom');
  }

  // Keep every ordinary app link out of the embedded WebView. File-preview
  // iframes need their own handler because events do not cross documents.
  $effect(() => installExternalLinkHandler(document));

  // Keyboard height detection
  $effect(() => {
    // Android Tauri app: native event provides exact keyboard height
    let androidNativeKb = false;
    let pendingNativeKb = 0;
    let pendingNativeTimer = 0;
    const hasFocusedTextInput = () => {
      const activeTag = document.activeElement?.tagName;
      return activeTag === 'TEXTAREA' || activeTag === 'INPUT';
    };
    const applyNativeKeyboardHeight = (kbh) => {
      pendingNativeKb = 0;
      clearTimeout(pendingNativeTimer);
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
    const nativeHandler = (e) => {
      androidNativeKb = true; // suppress visualViewport handler on Android
      const kbh = e.detail?.height || 0;
      if (kbh > 0 && !hasFocusedTextInput()) {
        // The IME can become visible during the keyboard-toggle pointer event,
        // one task before xterm's hidden textarea receives focus. Dropping this
        // one-shot native height leaves the terminal at full-screen size until
        // the user closes and reopens the keyboard. Keep the stale-event guard,
        // but defer the value briefly so focusin can validate and apply it.
        pendingNativeKb = kbh;
        clearTimeout(pendingNativeTimer);
        pendingNativeTimer = setTimeout(() => {
          if (pendingNativeKb === kbh && hasFocusedTextInput()) applyNativeKeyboardHeight(kbh);
          else if (pendingNativeKb === kbh) pendingNativeKb = 0;
        }, 500);
        window.__dbg?.(`androidKb: DEFER kbh=${kbh} (activeEl=${document.activeElement?.tagName})`);
        return;
      }
      applyNativeKeyboardHeight(kbh);
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
    const onFocusIn = (e) => {
      window.__dbg?.(`focusIn: ${e.target?.tagName}[${e.target?.className?.slice(0,20)}] activeEl=${document.activeElement?.tagName}`);
      if (pendingNativeKb > 0 && hasFocusedTextInput()) {
        const kbh = pendingNativeKb;
        window.__dbg?.(`androidKb: applying deferred kbh=${kbh} on focusin`);
        applyNativeKeyboardHeight(kbh);
      }
    };
    const onFocusOut = (e) => window.__dbg?.(`focusOut: ${e.target?.tagName}[${e.target?.className?.slice(0,20)}]`);
    document.addEventListener('focusin', onFocusIn);
    document.addEventListener('focusout', onFocusOut);

    return () => {
      clearTimeout(pendingNativeTimer);
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
    let resizeFrame = 0;
    const syncDesktopHeight = () => {
      if (isTouch) return;
      document.documentElement.style.setProperty('--app-height', window.innerHeight + 'px');
    };
    const onResize = () => {
      wideEnough = window.innerWidth >= SPLIT_MIN_WIDTH;
      syncDesktopHeight();
      // WKWebView can deliver the resize event before its layout viewport has
      // adopted the new window size. Measure again on the next frame so a
      // maximized/restored macOS window cannot retain the previous height.
      cancelAnimationFrame(resizeFrame);
      resizeFrame = requestAnimationFrame(syncDesktopHeight);
    };
    onResize(); // set the correct height on mount, not just on the first resize
    window.addEventListener('resize', onResize);
    window.addEventListener('pageshow', onResize);
    if (!isTouch) window.visualViewport?.addEventListener('resize', onResize);
    return () => {
      cancelAnimationFrame(resizeFrame);
      window.removeEventListener('resize', onResize);
      window.removeEventListener('pageshow', onResize);
      if (!isTouch) window.visualViewport?.removeEventListener('resize', onResize);
    };
  });

  // Desktop Cmd/Ctrl +/-/0 scales the complete WebView. Terminal font size
  // remains an independent setting; changing UI scale therefore does not
  // replace xterm's renderer or disturb its hidden textarea focus.
  $effect(() => {
    if (!isTauriDesktop) return;
    const onKey = (e) => {
      if (!(e.metaKey || e.ctrlKey) || e.altKey) return;
      if (e.key === '=' || e.key === '+') {
        e.preventDefault(); setUiZoom(stepUiZoom(uiZoom, 1));
      } else if (e.key === '-' || e.key === '_') {
        e.preventDefault(); setUiZoom(stepUiZoom(uiZoom, -1));
      } else if (e.key === '0') {
        e.preventDefault(); setUiZoom(UI_ZOOM_DEFAULT);
      }
    };
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

  // Persist nav state for restore on reload. splitLayout/splitCells are only
  // meaningful on desktop; a desktop-saved state degrades to single-pane on a
  // phone because restore re-gates on splitEligible.
  $effect(() => {
    if (connected && terminalTarget) {
      localStorage.setItem('tmux_state', JSON.stringify({
        page, terminalTarget, terminalSession, terminalCommand,
        splitLayout, splitCells
      }));
    }
  });

  // Reconnect UI state, mirrored from the reconnect machine
  // (src/lib/app/reconnect.ts — framework-free, unit-tested; owns every
  // timer, the retry/backoff/probe strategy, and the stuck-watchdog).
  let reconnecting = $state(false);
  let reconnectAttempt = $state(0);   // 1-indexed when visible; 0 means not attempting
  let reconnectClass = $state('');    // LAN / Tailscale / WAN label for the current try

  const reconnectMachine = createReconnectMachine({
    connect,
    findBestAddress,
    isAddressViable,
    noteAddressUnreachable,
    classifyAddress,
    addressLabels: ADDRESS_LABELS,
    storage: localStorage,
    maxAttempts: RECONNECT_MAX_ATTEMPTS,
    watchdogMs: RECONNECT_WATCHDOG_MS,
    onStateChange: (st) => {
      reconnecting = st.reconnecting;
      reconnectAttempt = st.attempt;
      reconnectClass = st.label;
    },
    onSuccess: (useAddr, primaryAddr) => onReconnectSuccess(useAddr, primaryAddr),
    onGiveUp: () => { connected = false; page = 'settings'; },
  });

  setOnDisconnect(() => {
    // Keep connected=true during reconnect to avoid UI flicker
    reconnectMachine.start();
  });

  function cancelReconnect() {
    reconnectMachine.cancel();
    connected = false;
    disconnect();
    page = 'settings';
  }

  function onReconnectSuccess(useAddr, primaryAddr) {
    connected = true;
    serverInfo = { hostname: getHostname() || '', machineId: getMachineId() || '' };
    if (useAddr !== primaryAddr) { localStorage.setItem('tmux_address', useAddr); activeAddress = useAddr; }
    resubscribeAll();
    probeTeam();
    syncAgentNotifications();
    // Tell Terminal to reset stale resize state + re-fit against the new server.
    window.dispatchEvent(new Event('ws-reconnected'));
  }

  function onConnected() {
    reconnectMachine.cancel();
    connected = true;
    serverInfo = { hostname: getHostname() || '', machineId: getMachineId() || '' };
    page = 'sessions';
    localStorage.removeItem('tmux_disconnected');
    // A manual connect from Settings reaches a server with EMPTY subscription
    // state, while Terminals stay mounted across the disconnect and keep their
    // refcounts — so their subscribe() calls will never re-send the wire
    // message themselves. Without this, the terminal shows a frozen snapshot
    // (send_keys still works — it's a plain RPC) until a full reload.
    // Mirrors onReconnectSuccess; on a first-ever connect both are no-ops.
    resubscribeAll();
    probeTeam();
    syncAgentNotifications();
    window.dispatchEvent(new Event('ws-reconnected'));
  }

  function readTarget(target) {
    const match = /^(.+):(\d+)\./.exec(target || '');
    if (match) markWindowRead(match[1], Number(match[2]));
  }

  function openTerminal(session, target, command = '') {
    terminalSession = session;
    terminalTarget = target;
    terminalCommand = command;
    workContext = 'terminal';
    page = 'terminal';
    readTarget(target);
    navPush();
  }

  // The shown pane died (Ctrl-D, process exit). Stay in the terminal and
  // fall back to the previous pane of the same session — switcher order,
  // nearest below the closed window/pane index, else the first after it.
  // Only when the session has no panes left (or is gone entirely) return
  // to Sessions, and only if the user is actually looking at the terminal:
  // pane_closed can arrive while they're on another tab (Terminal stays
  // mounted in a hidden page-layer), and yanking them out of Files for a
  // background pane death would be hostile.
  async function paneExitFallback(closed) {
    let remaining = [];
    try {
      const m = /:(\d+)\.(\d+)$/.exec(closed || '');
      const cw = m ? Number(m[1]) : -1;
      const cp = m ? Number(m[2]) : -1;
      remaining = (await listPanes(terminalSession))
        .filter((p) => `${p.session}:${p.window}.${p.pane}` !== closed)
        .sort((a, b) => a.window - b.window || a.pane - b.pane);
      const before = remaining.filter((p) => p.window < cw || (p.window === cw && p.pane < cp));
      const pick = before[before.length - 1] || remaining[0];
      if (pick) {
        terminalTarget = `${pick.session}:${pick.window}.${pick.pane}`;
        terminalCommand = pick.current_command || '';
        readTarget(terminalTarget);
        return;
      }
    } catch {} // list_panes fails when the whole session died with the pane
    terminalTarget = '';
    if (page === 'terminal') page = 'sessions';
  }

  // Jump to the Team tab and select a specific room (from a team session row in
  // Sessions). Team stays mounted (see the page-layer below), so selecting the
  // room just reloads that room's chat — no full remount.
  function openTeam(room) {
    page = 'team';
    workContext = 'team';
    if (room) teamRef?.selectTeam(room);
    navPush();
  }

  function doDisconnect() {
    reconnectMachine.cancel();
    disconnect();
    stopAgentNotifications();
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
      syncAgentNotifications();
      window.dispatchEvent(new Event('ws-reconnected'));
    } catch {
      // Switch failed — trigger normal reconnect which will try all addresses
      reconnectMachine.start();
    } finally {
      optimizing = false;
      lastProbeTime = Date.now();
    }
  }

  // Deep-link connect: a shareable URL can pre-fill the connection and auto-jump
  // in, e.g.  https://app/?addr=ws://host:9899&token=XXX  (also accepts
  // address=/server=, an optional socket=, and params inside the #hash). We copy
  // them into the same localStorage keys the normal flow uses — so the existing
  // auto-connect effect below picks them up — then STRIP them from the URL so the
  // token isn't left sitting in the address bar or re-applied on refresh.
  // (Caveat: a token in a link can still land in browser/proxy history before we
  // strip it — only share such links over trusted channels.)
  function consumeConnectUrlParams() {
    try {
      const hashQ = location.hash.includes('?') ? location.hash.split('?')[1] : '';
      const q = new URLSearchParams(location.search || hashQ);
      const addrRaw = q.get('addr') || q.get('address') || q.get('server');
      const token = q.get('token');
      const socket = q.get('socket');
      if (!addrRaw && token == null) return;
      if (addrRaw) {
        let a = addrRaw.trim();
        if (!/^wss?:\/\//.test(a)) a = (location.protocol === 'https:' ? 'wss://' : 'ws://') + a;
        localStorage.setItem('tmux_address', a);
        try {
          const hist = JSON.parse(localStorage.getItem('tmux_address_history') || '[]')
            .map(h => (typeof h === 'string' ? { address: h, token: '' } : h));
          const entry = { address: a, token: token || '' };
          localStorage.setItem('tmux_address_history',
            JSON.stringify([entry, ...hist.filter(h => h.address !== a)].slice(0, 8)));
        } catch {}
      }
      if (token != null) localStorage.setItem('tmux_token', token);
      if (socket) localStorage.setItem('tmux_socket', socket);
      localStorage.removeItem('tmux_disconnected'); // explicit intent to connect
      history.replaceState(null, '', location.pathname + location.hash.split('?')[0]);
    } catch { /* malformed URL — ignore, fall back to saved/settings */ }
  }
  consumeConnectUrlParams();

  // Copy the CURRENT connection as a deep link (consumed by consumeConnectUrlParams
  // on the other device). Plain clipboard copy with a brief ✓ — no share sheet,
  // no prompt (copyText already falls back to execCommand on http).
  let linkCopied = $state(false);
  async function shareConnectionLink() {
    const addr = localStorage.getItem('tmux_address') || activeAddress;
    if (!addr) return;
    const token = localStorage.getItem('tmux_token') || '';
    const socket = localStorage.getItem('tmux_socket') || '';
    const params = new URLSearchParams();
    params.set('addr', addr);
    if (token) params.set('token', token);
    if (socket) params.set('socket', socket);
    const link = `${location.origin}${location.pathname}?${params.toString()}`;
    await copyText(link);
    linkCopied = true;
    setTimeout(() => linkCopied = false, 1500);
  }

  // Auto-reconnect and restore state on page load
  let autoConnectAttempted = false;

  // Detect app resume (Android background → foreground) + periodic optimize
  $effect(() => {
    const handler = () => {
      if (document.visibilityState !== 'visible') return;
      if (!isConnected() && !reconnectMachine.isActive()) {
        reconnectMachine.start();
      } else if (isConnected()) {
        // A suspended mobile WebView can resume with WebSocket.readyState still
        // OPEN even though pane pushes stopped while it was backgrounded. The
        // server treats subscribe as idempotent and resets its change detector,
        // guaranteeing a fresh pane_output without perturbing local refcounts.
        resubscribeAll();
        window.__dbg?.('resume: re-subscribed active panes');
        if (Date.now() - lastProbeTime > OPTIMIZE_INTERVAL_MS) optimizeConnection();
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
      syncAgentNotifications();
      try {
        const s = JSON.parse(localStorage.getItem('tmux_state') || '{}');
        if (s.terminalTarget) {
          terminalTarget = s.terminalTarget;
          terminalSession = s.terminalSession || '';
          terminalCommand = s.terminalCommand || '';
          page = s.page || 'terminal';
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
      if (page === 'files') { page = 'terminal'; return; }
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
    if (hubEligible) t.push('hub');
    t.push('terminal');
    if (teamAvailable) t.push('team');
    t.push('files');
    return t;
  });

  let slideAnim = $state('');

  function switchTab(target) {
    if (slideAnim) return;
    showSettings = false;
    const t = tabs();
    const curName = page;
    if (target === curName) return;
    const fromIdx = t.indexOf(curName);
    const toIdx = t.indexOf(target);
    // Apply page change immediately
    if (target === 'terminal') { page = 'terminal'; workContext = 'terminal'; }
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

  $effect(() => {
    if (!isTauriDesktop) return;
    const onShortcut = (event) => {
      if (isShortcutInputTarget(event.target)) return;
      const action = shortcuts.action(shortcutFromEvent(event));
      if (!action) return;

      const consume = () => {
        event.preventDefault();
        event.stopPropagation();
      };
      if (action === 'previousPage' || action === 'nextPage') {
        const available = tabs();
        const current = page;
        if (!available.includes(current)) return;
        const step = action === 'previousPage' ? -1 : 1;
        consume();
        switchTab(cycleItem(available, current, step));
      } else if (action === 'openTerminal') {
        if (!terminalTarget) return;
        consume();
        switchTab('terminal');
      } else if (action === 'openFiles') {
        consume();
        switchTab('files');
      } else if ((action === 'previousWindow' || action === 'nextWindow') && page === 'terminal') {
        consume();
        window.dispatchEvent(new CustomEvent('terminal-window-shortcut', {
          detail: { direction: action === 'previousWindow' ? -1 : 1 },
        }));
      }
    };
    window.addEventListener('keydown', onShortcut, { capture: true });
    return () => window.removeEventListener('keydown', onShortcut, { capture: true });
  });
</script>

<main>
  <nav>
    {#if connected}
      <img class="nav-icon" src={iconSrc} alt="" width="28" height="28" />
      <div class="nav-pills">
        <button tabindex="-1" class:active={page === 'sessions'} onclick={() => switchTab('sessions')}>
          {t('sessions')}
        </button>
        {#if hubEligible}
          <button tabindex="-1" class:active={page === 'hub'} onclick={() => switchTab('hub')}>
            {t('hub')}
          </button>
        {/if}
        <button tabindex="-1" class:active={page === 'terminal'} onclick={() => switchTab('terminal')}>
          {t('terminal')}
        </button>
        {#if teamAvailable}
          <button tabindex="-1" class:active={page === 'team'} onclick={() => switchTab('team')}>
            {t('team')}
          </button>
        {/if}
        <button tabindex="-1" class:active={page === 'files'} onclick={() => switchTab('files')}>
          {t('files')}
        </button>
      </div>
      <div class="nav-right">
        <button tabindex="-1" class="gear-btn" class:active={showSettings} onclick={() => showSettings = !showSettings}><Icon name="gear" size={16} /></button>
      </div>
    {:else}
      <div class="brand">
        <img class="logo" src={iconSrc} alt="" width="24" height="24" />
        <span class="brand-text">tmux<span class="brand-accent">mobile</span></span>
      </div>
      <div class="nav-right">
        <button tabindex="-1" class="gear-btn" class:active={showSettings} onclick={() => showSettings = !showSettings}><Icon name="gear" size={16} /></button>
      </div>
    {/if}
  </nav>

  {#if showSettings}
    <Preferences {connected} {theme} {fontSize} {debugMode} {serverInfo} {activeAddress} addresses={prefAddresses}
      {optimizing} {linkCopied}
      onClose={() => showSettings = false}
      onTheme={setTheme}
      {uiZoom} showUiZoom={isTauriDesktop} showShortcuts={isTauriDesktop} onUiZoom={setUiZoom}
      onFontSize={setFontSize}
      onDebug={(value) => { debugMode = value; localStorage.setItem('tmux_debug', value ? '1' : ''); }}
      onOptimize={optimizeConnection}
      onShare={shareConnectionLink}
      onAddress={(address) => {
        localStorage.setItem('tmux_address', address);
        activeAddress = address;
        disconnect();
        connect(address, localStorage.getItem('tmux_token') || '').then(() => {
          serverInfo = { hostname: getHostname() || '', machineId: getMachineId() || '' };
          resubscribeAll();
        }).catch(() => { reconnectMachine.start(); });
      }}
      onDisconnect={() => { showSettings = false; doDisconnect(); }}
      onConnectionSetup={() => { showSettings = false; page = 'settings'; }} />
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
      <Sessions {openTerminal} {openTeam} activeTarget={terminalTarget} visible={page === 'sessions'} />
    {/if}
    <!-- Team is kept mounted (like Files/Terminal below) and merely hidden when
         inactive, so switching tabs preserves its state — the selected team
         (activeRoom), loaded history, scroll position, and the embedded agent
         terminals all survive. Putting it in the {#if} chain above would
         destroy + recreate it on every tab switch, resetting activeRoom to the
         first team and reloading everything. Gated on teamAvailable so it never
         mounts on a server without the team bus (e.g. mobile). The visible prop
         pauses its polling while hidden and triggers a refresh when shown. -->
    {#if hubEligible}
      <!-- Hub (agents-v2 desktop three-column view): kept mounted like Team so
           the selected project, chat scroll, and embedded terminal survive tab
           switches. Desktop-eligible only (needs width + the bus): mobile
           keeps the tab layout untouched. -->
      <div class="page-layer" class:hidden={page !== 'hub'}>
        <Hub visible={page === 'hub'} {fontSize} openTerminal={(s, tgt, cmd) => openTerminal(s, tgt, cmd)} />
      </div>
    {/if}
    {#if teamAvailable}
      <div class="page-layer" class:hidden={page !== 'team'}>
        <Team bind:this={teamRef} visible={page === 'team'} currentSession={terminalSession} {fontSize} openTerminal={(s, tgt, cmd) => openTerminal(s, tgt, cmd)} onTeamSession={(s) => teamSession = s} />
      </div>
    {/if}
    <div class="page-layer" class:hidden={page !== 'files'}>
      <Files session={filesSession} visible={page === 'files'} {fontSize} onGoBack={(fn) => filesGoBack = fn} />
    </div>
    <div class="page-layer" class:hidden={page !== 'terminal'}>
      {#if terminalTarget}
        <div class="terminal-body" class:split-capable={splitEligible}>
          {#if splitActive}
            <!-- In split mode the layout control floats top-right (no single
                 win-bar to host it; cells have their own headers). In single-
                 pane mode the control lives in the Terminal's chip bar. -->
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
            <Terminal target={terminalTarget} session={terminalSession} command={terminalCommand} {fontSize}
              splitEligible={splitEligible} {splitActive} {splitLayout} onSetLayout={setLayout}
              onSwitchPane={(t, cmd) => { terminalTarget = t; terminalSession = t.split(':')[0]; terminalCommand = cmd || ''; readTarget(t); }} onPaneExit={paneExitFallback} />
          {/if}
        </div>
      {:else}
        <div class="terminal-empty">
          <Icon name="terminal" size={22} />
          <span>{t('noTerminalSelected')}</span>
        </div>
      {/if}
    </div>
  </div>

  {#if debugMode}
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="debug-overlay" use:keepDebugPanelVisible
      style:left={debugPosition ? debugPosition.left + 'px' : undefined}
      style:top={debugPosition ? debugPosition.top + 'px' : undefined}
      ontouchstart={(e) => {
        const el = e.currentTarget;
        // Only drag from the header area (top 24px)
        const rect = el.getBoundingClientRect();
        const ty = e.touches[0].clientY - rect.top;
        if (ty > 24) return; // let content scroll/select normally
        e.preventDefault();
        const startX = e.touches[0].clientX - el.offsetLeft;
        const startY = e.touches[0].clientY - el.offsetTop;
        const onMove = (ev) => {
          ev.preventDefault();
          debugPosition = clampDebugPosition(el, ev.touches[0].clientX - startX, ev.touches[0].clientY - startY);
        };
        const onEnd = () => {
          document.removeEventListener('touchmove', onMove);
          document.removeEventListener('touchend', onEnd);
          document.removeEventListener('touchcancel', onEnd);
          if (debugPosition) localStorage.setItem(DEBUG_POSITION_KEY, JSON.stringify(debugPosition));
        };
        document.addEventListener('touchmove', onMove, { passive: false });
        document.addEventListener('touchend', onEnd);
        document.addEventListener('touchcancel', onEnd);
      }}
    >
      <div class="debug-header" onpointerdown={startDebugPointerDrag}>DEBUG <button onclick={() => { if (debugEl) copyText(debugEl.innerText); }}>copy</button> <button onclick={() => { if (debugEl) debugEl.innerHTML = ''; }}>clear</button> <button onclick={() => { debugMode = false; localStorage.removeItem('tmux_debug'); }}>✕</button></div>
      <div class="debug-content" bind:this={debugEl}></div>
    </div>
  {/if}

  <InstallPrompt />
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
    -webkit-app-region: no-drag;
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
  main, nav { transition: background-color 0.3s ease, color 0.3s ease; }

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
  .gear-btn.active { color: var(--accent); background: var(--accent-bg); }


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
  .terminal-empty {
    flex: 1; min-height: 0;
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: 9px; color: var(--text3); font-size: 13px;
  }
  .terminal-empty :global(svg) { opacity: 0.65; }

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
