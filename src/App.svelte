<script>
  import Settings from './lib/app/Settings.svelte';
  import Sessions from './lib/sessions/Sessions.svelte';
  import Terminal from './lib/terminal/Terminal.svelte';
  import SplitView from './lib/sessions/SplitView.svelte';
  import Files from './lib/files/Files.svelte';
  import Hub from './lib/hub/Hub.svelte';
  import AgentsPage from './lib/hub/AgentsPage.svelte';
  import Board from './lib/hub/Board.svelte';
  import Icon from './lib/ui/Icon.svelte';
  import SideHandle from './lib/ui/SideHandle.svelte';
  import InstallPrompt from './lib/ui/InstallPrompt.svelte';
  import Preferences from './lib/app/Preferences.svelte';
  import { copyText } from './lib/core/clipboard.ts';
  import { teamStatus } from './lib/core/ws.ts';
  import { connect, isConnected, disconnect, setOnDisconnect, subscribe as wsSubscribe, resubscribeActive as wsResubscribeActive, getMachineId, getHostname, findBestAddress, classifyAddress, ADDRESS_LABELS, isAddressViable, noteAddressUnreachable, listPanes, listSessions } from './lib/core/ws.ts';
  import { t } from './lib/core/i18n.svelte.ts';
  import { layout } from './lib/app/layout.svelte.ts';
  import { teamState } from './lib/core/team.svelte.ts';
  import { applyFontVars } from './lib/app/fonts.svelte.ts';
  import { normalizeUiZoom, stepUiZoom, UI_ZOOM_DEFAULT } from './lib/app/ui-zoom.ts';
  import { agentsLivesInSettings, defaultPage, restoreNav, retarget } from './lib/app/nav-state.ts';
  import { RAIL_DRAG_THRESHOLD, RAIL_GAP, RAIL_ORDER_KEY, parseRailOrder, railDropAt, railDropIndex, railDropOffset, railOrderToStore, visibleRailSlots } from './lib/app/nav-order.ts';
  import { createReconnectMachine } from './lib/app/reconnect.ts';
  import { cycleItem, shortcutFromEvent } from './lib/app/shortcuts.ts';
  import { isShortcutInputTarget, shortcuts } from './lib/app/shortcuts.svelte.ts';
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
  // Files follows the session the user touched LAST — opening a terminal pane
  // OR selecting a chat project both set it. It used to derive from the
  // terminal alone, so browsing a project in the chat never moved the Files
  // tab while switching terminal panes did (owner, 2026-08-22: "chat里的路径
  // 没有刷新到文件 terminal好像就会刷新路径").
  let filesSession = $state('');
  let filesNavReq = $state(null);
  let boardIssueReq = $state(null); // a feed board-line tap: which issue to open // {path, n} — the Hub drawer's "open in Files tab" handover
  $effect(() => { if (terminalSession) filesSession = terminalSession; });
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
    if (page === 'team') page = 'hub'; // Team tab retired — the Hub replaced it
    // Sessions tab retired 2026-08-18: the list is Terminal's sidebar now, so
    // a persisted/deep-linked 'sessions' lands on the terminal it belonged to.
    if (page === 'sessions') page = 'terminal';
    // Same for the Hub: it needs the bus. Only redirect once the probe answered.
    if ((page === 'hub' || page === 'agents' || page === 'board') && teamState.probed && !teamState.available) page = 'terminal';
    // On touch the agent configuration is a Settings CATEGORY, so `agents` as a
    // page has no tab icon and no swipe stop — whatever route set it (a saved
    // state, a deep link, an older build), it becomes Settings opened there.
    if (page === 'agents' && agentsLivesInSettings(layout.isTouchDevice)) {
      page = 'prefs';
      prefsOpenReq = { tab: 'agents', n: ++prefsOpenSeq };
    }
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
  // Shared sidebar width (ui-unification.md): the shell owns the geometry,
  // pages consume var(--sidebar-w), SideHandle is the only other writer.
  $effect(() => {
    const saved = parseInt(localStorage.getItem('tmux_sidebar_w') || '', 10);
    if (saved >= 180 && saved <= 420) {
      document.documentElement.style.setProperty('--sidebar-w', saved + 'px');
    }
  });

  // Overlay geometry vars for fixed-position panels (Preferences): they must
  // clear whatever shell chrome exists — top bar when disconnected, the left
  // rail on connected desktop, nothing on connected mobile.
  $effect(() => {
    const root = document.documentElement.style;
    root.setProperty('--shell-top', connected ? '0px' : '49px');
    root.setProperty('--shell-left', connected && !layout.isTouchDevice ? '46px' : '0px');
  });
  // The Hub needs only the server-side bus (hub_* degrades method-not-found
  // without it, same probe as Team). Its LAYOUT adapts: three columns on
  // desktop, a single chat column on touch devices.
  let hubEligible = $derived(teamAvailable);
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
    const next = normalizeUiZoom(value);
    uiZoom = next;
    localStorage.setItem('tmux_ui_zoom', String(next));
    if (!isTauriDesktop) {
      // Web / Android: CSS zoom on the root scales the whole interface.
      // Two compensations keep the geometry honest:
      // - --ui-zoom lets the full-height containers divide their pixel
      //   --app-height back down (the writers keep writing raw innerHeight);
      // - the terminal host counter-zooms to 1.0 so xterm's cell metrics
      //   stay in physical pixels — terminal text size is its OWN setting,
      //   and mixing scaled cell rects with unscaled clientHeight would
      //   mis-fit cols×rows.
      document.documentElement.style.zoom = String(next);
      document.documentElement.style.setProperty('--ui-zoom', String(next));
      requestAnimationFrame(() => {
        window.dispatchEvent(new CustomEvent('app-zoom-change', { detail: { scale: next } }));
      });
      return;
    }
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

  if (isTauriDesktop || initialUiZoom !== 1) setUiZoom(initialUiZoom);

  // Settings is a PAGE (ui-unification.md "Settings as a page"), not a modal.
  // The gear toggles into it and back to where you were.
  let pageBeforePrefs = 'terminal';
  let prefsPushed = false;
  function togglePrefs() {
    if (page === 'prefs') {
      if (prefsPushed) history.back();
      else { page = pageBeforePrefs; slidePage('slide-in-left'); }
    } else {
      pageBeforePrefs = page; page = 'prefs'; slidePage('slide-in-right');
      navPush(); prefsPushed = true;
    }
  }
  // Apply the persisted custom fonts (if any) before first paint — rewrites
  // --font-mono/--font-ui/--font-display inline; a no-op for the defaults.
  applyFontVars();
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

  // A project rename renames its tmux session, so every target that names the
  // old session stops resolving — including the one on screen. Remap them in
  // place instead of dropping the user on a dead pane.
  $effect(() => {
    const onRenamed = (e) => {
      const { from, to } = e.detail ?? {};
      if (!from || !to || from === to) return;
      const move = (target) => retarget(target ?? '', from, to);
      if (terminalSession === from) terminalSession = to;
      terminalTarget = move(terminalTarget);
      splitCells = splitCells.map((c) => ({
        ...c,
        session: c.session === from ? to : c.session,
        target: move(c.target),
      }));
    };
    window.addEventListener('project-renamed', onRenamed);
    return () => window.removeEventListener('project-renamed', onRenamed);
  });

  // Persist nav state for restore on reload. It used to save ONLY when a
  // terminal target existed, so reading the chat and refreshing dropped you back
  // on the device's default tab — the state was there, just never written
  // (owner, 2026-08-19: "每次切换或者刷新都会变"). The tab is worth remembering on
  // its own; the terminal fields ride along when there are any.
  // splitLayout/splitCells are only meaningful on desktop; a desktop-saved state
  // degrades to single-pane on a phone because restore re-gates on splitEligible.
  $effect(() => {
    if (!connected) return;
    localStorage.setItem('tmux_state', JSON.stringify({
      page, terminalTarget, terminalSession, terminalCommand,
      splitLayout, splitCells,
    }));
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
    // Tell Terminal to reset stale resize state + re-fit against the new server.
    window.dispatchEvent(new Event('ws-reconnected'));
  }

  function onConnected() {
    reconnectMachine.cancel();
    connected = true;
    serverInfo = { hostname: getHostname() || '', machineId: getMachineId() || '' };
    page = 'terminal';
    localStorage.removeItem('tmux_disconnected');
    // A manual connect from Settings reaches a server with EMPTY subscription
    // state, while Terminals stay mounted across the disconnect and keep their
    // refcounts — so their subscribe() calls will never re-send the wire
    // message themselves. Without this, the terminal shows a frozen snapshot
    // (send_keys still works — it's a plain RPC) until a full reload.
    // Mirrors onReconnectSuccess; on a first-ever connect both are no-ops.
    resubscribeAll();
    probeTeam();
    window.dispatchEvent(new Event('ws-reconnected'));
  }

  function openTerminal(session, target, command = '') {
    const from = page;
    terminalSession = session;
    terminalTarget = target;
    terminalCommand = command;
    page = 'terminal';
    navPush();
    if (from === 'hub') jumpedFrom = 'hub';
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
        return;
      }
    } catch {} // list_panes fails when the whole session died with the pane
    terminalTarget = '';
    // Nothing left to show: the empty state offers the session list (which
    // used to BE the fallback page).
    if (page === 'terminal' && layout.isTouchDevice) sessListOpen = true;
  }

  // Jump to the Team tab and select a specific room (from a team session row in
  function doDisconnect() {
    reconnectMachine.cancel();
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
      try {
        const s = JSON.parse(localStorage.getItem('tmux_state') || '{}');
        // A saved target names a session that may not exist any more — killed
        // while we were away, or RENAMED (a project rename renames its session).
        // Restoring it anyway lands the user on a pane that cannot be captured,
        // so check first and come back to the tab without a target instead.
        if (s.terminalTarget && s.terminalSession) {
          listSessions()
            .then((sessions) => {
              // list_sessions answers with a bare array (list_sessions_with_panes
              // is the one that wraps).
              if (!(sessions ?? []).some((x) => x.name === s.terminalSession)) {
                terminalTarget = '';
                terminalSession = '';
                terminalCommand = '';
                splitCells = splitCells.filter((c) => c.session !== s.terminalSession);
              }
            })
            .catch(() => {});
        }
        if (s.terminalTarget) {
          terminalTarget = s.terminalTarget;
          terminalSession = s.terminalSession || '';
          terminalCommand = s.terminalCommand || '';
          // Restore split layout only on eligible (desktop + wide) clients;
          // a desktop-saved state silently stays single-pane on a phone.
          if (splitEligible && s.splitLayout > 1 && Array.isArray(s.splitCells) && s.splitCells.length) {
            splitCells = s.splitCells;
            splitLayout = s.splitLayout;
            nextCellId = Math.max(0, ...s.splitCells.map(c => c.id ?? 0)) + 1;
            activeCellId = s.splitCells[0]?.id ?? null;
          }
        }
        // The tab is restored whether or not a terminal came with it; only an
        // unknown name falls back to the device default. On touch a saved
        // `agents` resolves to Settings opened at its Agents category — there is
        // no Agents tab to land on there.
        const nav = restoreNav(s.page, layout.isTouchDevice);
        page = nav.page;
        if (nav.settingsTab) prefsOpenReq = { tab: nav.settingsTab, n: ++prefsOpenSeq };
      } catch { page = defaultPage(layout.isTouchDevice); }
    }).catch(() => {
      clearTimeout(timeout);
      page = 'settings';
    });
  });
  // Android back gesture via history API
  // Every navigation push a state, back pops it naturally
  let filesGoBack = $state(null);
  let hubGoBack = $state(null);
  let agentsGoBack = $state(null);
  let prefsGoBack = $state(null);
  // A request from the Hub's agent menu to open one agent's CONFIG editor:
  // {name, n} — n increments so asking for the same agent twice still fires.
  let agentsEditReq = $state(null);
  // A one-shot "open this Settings category" request, same {tab, n} shape. The
  // sequence is a plain counter, NOT read back off the state: one of the writers
  // is the page-redirect $effect, and reading the state it writes would make
  // that effect depend on itself.
  let prefsOpenSeq = 0;
  let prefsOpenReq = $state(null);
  /** The ONE way into the agent configuration, wherever it lives on this device:
   *  a page of its own on the desktop rail, a Settings category on a phone
   *  (nav-state's agentsLivesInSettings). Every caller — the Hub's "configure
   *  agent" item and the restore path — goes through here, so the two devices
   *  cannot drift apart. */
  function openAgentsConfig(name = null) {
    if (name) agentsEditReq = { name, n: (agentsEditReq?.n ?? 0) + 1 };
    if (agentsLivesInSettings(layout.isTouchDevice)) {
      prefsOpenReq = { tab: 'agents', n: ++prefsOpenSeq };
      if (page !== 'prefs') togglePrefs();
    } else {
      switchTab('agents');
    }
  }
  let boardGoBack = $state(null);
  // A cross-page JUMP from the chat (header toggles, a feed board-line tap,
  // opening a pane full-screen) remembers where it came from, so the phone's
  // back at the target's FLOOR returns to the conversation instead of the
  // terminal root (owner, 2026-08-29: "从聊天跳转到terminal或者board或者file，
  // 我返回 应该回到聊天对话页面"). Any deliberate navigation — a tab tap, a
  // swipe — clears it: the slot only survives while you stay where the jump
  // landed.
  let jumpedFrom = $state(null);

  function navPush() { history.pushState({ app: true }, ''); }

  $effect(() => {
    const handler = (e) => {
      if (!e.state?.app) {
        // Reached bottom of our stack, re-push to prevent exit
        navPush();
        return;
      }
      if (page === 'files' && filesGoBack && filesGoBack()) return;
      if (page === 'files' && jumpedFrom) { switchTab(jumpedFrom); return; }
      if (page === 'files') { page = 'terminal'; return; }
      // Chat and Agents peel their own layers (menus, dialogs, drawer,
      // editor) exactly like Files peels its views — a back with nothing to
      // peel falls through to the re-push, so it never reads as the browser
      // navigating away (owner, 2026-08-24: "像是网页刷新了。像文件管理页面
      // 就很好"). The consumed pop re-pushes so the NEXT back has an entry
      // to spend no matter how many layers were peeled.
      if (page === 'hub' && hubGoBack && hubGoBack()) { navPush(); return; }
      if (page === 'agents' && agentsGoBack && agentsGoBack()) { navPush(); return; }
      if (page === 'board' && boardGoBack && boardGoBack()) { navPush(); return; }
      if (page === 'board' && jumpedFrom) { switchTab(jumpedFrom); return; }
      // Board with nothing to peel is like Hub: the drawer is its compact
      // floor (lifted by its own chain above), so a fall-through just
      // re-pushes — it never dumps the reader on the terminal (board #47).
      // Terminal is the root on a phone: back closes the session sheet if it
      // is open, otherwise there is nowhere below it (re-push prevents exit).
      if (page === 'terminal' && sessListOpen) { sessListOpen = false; navPush(); return; }
      // A terminal you JUMPED INTO from the chat is not the root: back
      // returns to the conversation (switchTab re-pushes and slides left).
      if (page === 'terminal' && jumpedFrom) { switchTab(jumpedFrom); return; }
      // Settings peels its open category back to the list first; only a
      // back with nothing to peel leaves the page.
      if (page === 'prefs' && prefsGoBack && prefsGoBack()) { return; }
      if (page === 'prefs') { page = pageBeforePrefs; slidePage('slide-in-left'); prefsPushed = false; return; }
      // At sessions root, re-push to prevent exit
      navPush();
    };
    window.addEventListener('popstate', handler);
    // Seed one entry
    navPush();
    return () => window.removeEventListener('popstate', handler);
  });
  // ── The rail's icon ORDER is the user's, and remembered ─────────────────
  // Press a rail icon and drag it up or down (owner, 2026-08-29: "在桌面版的左
  // 侧侧边栏 可以鼠标长按页面 icon 来调整上下顺序 这个顺序也会在客户端记录下
  // 来"). DESKTOP RAIL ONLY: the phone's bottom bar is thumb geography rather
  // than a preference, and press-and-drag is the gesture the terminal already
  // spends on scrolling. The rules that regress silently — what a saved order
  // may say, where a page a saved order never heard of goes, what a drop moves
  // — are pure functions in lib/app/nav-order.ts with their own tests.
  const RAIL_ITEMS = {
    hub:      { icon: 'chat',     label: 'hub' },
    terminal: { icon: 'terminal', label: 'terminal' },
    files:    { icon: 'files',    label: 'files' },
    board:    { icon: 'layout',   label: 'board' },
    agents:   { icon: 'bot',      label: 'agentsTitle' },
    prefs:    { icon: 'gear',     label: 'settings' },
  };
  let railOrder = $state(parseRailOrder(localStorage.getItem(RAIL_ORDER_KEY)));
  // Only what is AVAILABLE is rendered, but the order keeps every page: hub /
  // board / agents need the server bus, and a page the bus turned off must come
  // back where the user put it rather than at the default position.
  let railSlots = $derived(visibleRailSlots(
    railOrder,
    (p) => (p === 'hub' || p === 'board' || p === 'agents' ? hubEligible : true),
  ));
  /** The live drag: null, or the carried slot plus the geometry snapshotted at
   *  drag start. Snapshotted because the carried icon moves by `transform`,
   *  which reflows nothing — re-measuring mid-drag could only add jitter. */
  let railDrag = $state(null);
  let railPress = null;        // pre-threshold bookkeeping; deliberately not reactive
  let railClickGuard = false;  // a drag must not ALSO switch pages on release

  function setRailOrder(next) {
    railOrder = next;
    const raw = railOrderToStore(next);
    if (raw) localStorage.setItem(RAIL_ORDER_KEY, raw);
    else localStorage.removeItem(RAIL_ORDER_KEY);
  }
  function railPointerDown(e, slot) {
    if (e.button !== 0 || e.pointerType === 'touch') return;
    // Cleared on every fresh press, so a guard left behind by a drag whose
    // click never arrived cannot swallow the NEXT click.
    railClickGuard = false;
    railPress = { slot, y: e.clientY, el: e.currentTarget };
    // Captured at once so a fast drag off the 34px button keeps reporting here
    // instead of to whatever it passes over.
    e.currentTarget.setPointerCapture(e.pointerId);
  }
  function railPointerMove(e) {
    if (!railPress) return;
    const dy = e.clientY - railPress.y;
    if (!railDrag) {
      // A press that moves a few px is a CLICK — mouse jitter must not reorder
      // the rail, and switching pages is what the icon is for almost always.
      if (Math.abs(dy) < RAIL_DRAG_THRESHOLD) return;
      const nav = railPress.el.closest('.rail');
      if (!nav) return;
      railDrag = {
        slot: railPress.slot,
        dy,
        navTop: nav.getBoundingClientRect().top,
        rects: [...nav.querySelectorAll('[data-rail-slot]')].map((el) => {
          const r = el.getBoundingClientRect();
          return { slot: el.dataset.railSlot, top: r.top, bottom: r.bottom };
        }),
        idx: 0,
      };
    }
    railDrag.dy = dy;
    railDrag.idx = railDropIndex(railDrag.rects, e.clientY);
  }
  function railPointerUp() {
    if (railDrag) {
      // Committed with the SAME index the insertion line was drawn from, so
      // the icon can only land where the line said it would.
      setRailOrder(railDropAt(railOrder, railDrag.slot, railDrag.rects, railDrag.idx));
      railClickGuard = true;
    }
    railPress = null;
    railDrag = null;
  }
  /** Abandon a drag without reordering — and still swallow the click, because
   *  the pointer travelled: the user was dragging, not picking a page. */
  function railCancelDrag() {
    if (railDrag) railClickGuard = true;
    railPress = null;
    railDrag = null;
  }
  function railActivate(slot) {
    if (railClickGuard) { railClickGuard = false; return; }
    if (slot === 'prefs') togglePrefs();
    else switchTab(slot);
  }
  // Escape abandons a drag and a resize invalidates its snapshotted rects —
  // the same dismissal contract every transient layer here follows. The
  // window-level pointerup is a safety net: if the captured button stops
  // existing mid-drag (the bus dropping hides three icons), the nav never sees
  // the release and a carried icon would be stranded on screen.
  $effect(() => {
    if (!railDrag) return;
    const onKey = (e) => { if (e.key === 'Escape') { e.preventDefault(); railCancelDrag(); } };
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', railCancelDrag);
    window.addEventListener('pointerup', railPointerUp, true);
    return () => {
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', railCancelDrag);
      window.removeEventListener('pointerup', railPointerUp, true);
    };
  });

  // Swipe left/right to switch tabs with slide animation
  const tabs = $derived(() => {
    // On DESKTOP the rail IS the order, so keyboard previous/next page follows
    // the icons the user arranged rather than a second, hidden sequence. The
    // gap is not a page and the gear is not a cycle stop (it toggles).
    if (!layout.isTouchDevice) return railSlots.filter((s) => s !== RAIL_GAP && s !== 'prefs');
    // Same order as the tab bar: files sits LEFT of the agent
    // config (owner, 2026-08-28: "手机下边的栏文件放到agent配置左侧吧").
    // Agents is NOT a stop here: on touch it is a Settings category, and a
    // swipe that reaches a page with no icon is a page you cannot get back to.
    // Owner-set order (2026-08-29): chat, board, files, terminal — settings
    // stays the row's last, non-cycling stop.
    const t = [];
    if (hubEligible) t.push('hub');
    if (hubEligible) t.push('board');
    t.push('files');
    t.push('terminal');
    return t;
  });

  // The session list is Terminal's sidebar, not a page of its own (owner,
  // 2026-08-18: "sessions 页面差不多相当于 terminal 的侧边栏"). On a wide
  // screen it is a column; on a phone it slides over the terminal, because
  // the terminal wants the whole screen there.
  let sessListOpen = $state(false);
  // The Terminal sheet's own condition (touch AND ≤760px — the old media
  // gate, now expressed where the class is applied; see app.css .side-sheet).
  let narrowVp = $state(typeof window !== 'undefined' && window.matchMedia('(max-width: 760px)').matches);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 760px)');
    const fn = () => { narrowVp = mq.matches; };
    mq.addEventListener('change', fn);
    return () => mq.removeEventListener('change', fn);
  });
  let slideAnim = $state('');

  function switchTab(target) {
    if (slideAnim) return;
    const t = tabs();
    const curName = page;
    if (target === curName) return;
    jumpedFrom = null; // deliberate navigation stands the return slot down
    const fromIdx = t.indexOf(curName);
    const toIdx = t.indexOf(target);
    // Apply page change immediately
    page = target;
    navPush();
    // Single slide-in animation from the correct direction — TOUCH LAYOUTS
    // ONLY. It is the visual half of the swipe gesture: content follows the
    // finger, so the direction means something. On a desktop the tabs are a
    // rail of buttons with no horizontal motion behind them, and pages slid
    // sideways for no reason (owner report) — worse on the wide three-column
    // pages, where a whole workspace lurches.
    if (fromIdx >= 0 && toIdx >= 0) slidePage(toIdx > fromIdx ? 'slide-in-right' : 'slide-in-left');
  }

  /** The one page-slide trigger (touch only): every page-level navigation —
   * tab switches, entering/leaving Settings — speaks the same 120ms grammar:
   * deeper/rightward enters from the right, back/leftward from the left. */
  function slidePage(dir) {
    if (!layout.isTouchDevice || slideAnim) return;
    slideAnim = dir;
    requestAnimationFrame(() => {
      setTimeout(() => { slideAnim = ''; }, SLIDE_ANIMATION_MS);
    });
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

<main class:with-rail={connected && !layout.isTouchDevice}>
  <!-- Shell chrome. Three shapes, one per context (docs/design-docs/features/app-shell.md):
       · connected desktop  → a left icon RAIL (the whole top bar is gone —
         vertical space goes to the terminal, switching is one always-visible click)
       · connected mobile   → a bottom TAB BAR in thumb reach, hidden while the
         keyboard is up (html.keyboard-open) so immersive typing costs nothing
       · disconnected       → the top brand bar with the gear (both platforms) -->
  {#if !connected}
    <nav class="topbar">
      <div class="brand">
        <img class="logo" src={iconSrc} alt="" width="24" height="24" />
        <span class="brand-text">tmux<span class="brand-accent">mobile</span></span>
      </div>
      <div class="nav-right">
        <button tabindex="-1" class="gear-btn" class:active={page === 'prefs'} onclick={togglePrefs}><Icon name="gear" size={16} /></button>
      </div>
    </nav>
  {:else if !layout.isTouchDevice}
    <!-- The icons are the USER's order (railOrder → nav-order.ts), so this is a
         list rather than six hardcoded buttons. The brand is not part of it: it
         is the app's mark, not a page. The gap between the two shipped groups —
         where you WORK above, where you CONFIGURE below (owner, 2026-08-19) —
         is a MEMBER of the order, so it moves as icons move around it and a
         drag across it is an ordinary drop instead of a silent refusal. -->
    <nav
      class="rail"
      class:reordering={!!railDrag}
      onpointermove={railPointerMove}
      onpointerup={railPointerUp}
      onpointercancel={railCancelDrag}
    >
      <img class="rail-brand" src={iconSrc} alt="" width="26" height="26" draggable="false" />
      {#each railSlots as slot (slot)}
        {#if slot === RAIL_GAP}
          <div class="rail-spacer" data-rail-slot={slot}></div>
        {:else}
          <button
            tabindex="-1"
            class="rail-btn"
            class:active={page === slot}
            class:dragging={railDrag?.slot === slot}
            data-rail-slot={slot}
            title={t(RAIL_ITEMS[slot].label)}
            style:transform={railDrag?.slot === slot ? `translateY(${railDrag.dy}px)` : null}
            onpointerdown={(e) => railPointerDown(e, slot)}
            onclick={() => railActivate(slot)}
          ><Icon name={RAIL_ITEMS[slot].icon} size={17} /></button>
        {/if}
      {/each}
      {#if railDrag}
        <div
          class="rail-drop"
          style:top="{(railDropOffset(railDrag.rects, railDrag.idx) ?? railDrag.navTop) - railDrag.navTop}px"
          aria-hidden="true"
        ></div>
      {/if}
    </nav>
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
        <Hub visible={page === 'hub'} {fontSize} mobile={layout.isTouchDevice} openTerminal={(s, tgt, cmd) => openTerminal(s, tgt, cmd)} onSelectSession={(s) => { if (s) filesSession = s; }} onGoBack={(fn) => hubGoBack = fn} openAgentConfig={(name) => openAgentsConfig(name)} openFilesTab={(s, path) => { if (s) filesSession = s; if (path) filesNavReq = { path, n: (filesNavReq?.n ?? 0) + 1 }; switchTab('files'); jumpedFrom = 'hub'; }} openBoardTab={(s, issue) => { if (s) filesSession = s; if (issue) boardIssueReq = { session: s, id: issue, n: (boardIssueReq?.n ?? 0) + 1 }; switchTab('board'); jumpedFrom = 'hub'; }} />
      </div>
    {/if}
    <!-- The Agents PAGE exists where Agents is a page: the desktop rail. On
         touch the only instance is the one Settings embeds, so this layer is not
         mounted there — two live copies would both load and both claim a back
         chain. -->
    {#if hubEligible && !agentsLivesInSettings(layout.isTouchDevice)}
      <div class="page-layer" class:hidden={page !== 'agents'}>
        <AgentsPage visible={page === 'agents'} onGoBack={(fn) => agentsGoBack = fn} editRequest={agentsEditReq} />
      </div>
    {/if}
    {#if page === 'prefs'}
    <div class="page-layer">
    <Preferences {connected} {theme} {fontSize} {debugMode} {serverInfo} {activeAddress} addresses={prefAddresses}
      {optimizing} {linkCopied}
      onClose={togglePrefs}
      onTheme={setTheme}
      {uiZoom} showUiZoom={true} showShortcuts={isTauriDesktop} onUiZoom={setUiZoom}
      onFontSize={setFontSize}
      onDebug={(value) => { debugMode = value; localStorage.setItem('tmux_debug', value ? '1' : ''); }}
      onOptimize={optimizeConnection}
      onShare={shareConnectionLink}
      onGoBack={(fn) => prefsGoBack = fn}
      onDrill={() => navPush()}
      showAgents={hubEligible && agentsLivesInSettings(layout.isTouchDevice)}
      agentsEditRequest={agentsEditReq}
      openRequest={prefsOpenReq}
      onAddress={(address) => {
        localStorage.setItem('tmux_address', address);
        activeAddress = address;
        disconnect();
        connect(address, localStorage.getItem('tmux_token') || '').then(() => {
          serverInfo = { hostname: getHostname() || '', machineId: getMachineId() || '' };
          resubscribeAll();
        }).catch(() => { reconnectMachine.start(); });
      }}
      onDisconnect={() => { page = 'settings'; doDisconnect(); }}
      onConnectionSetup={() => { page = 'settings'; }} />
    </div>
    {/if}
    <div class="page-layer" class:hidden={page !== 'files'}>
      <Files session={filesSession} visible={page === 'files'} {fontSize} onGoBack={(fn) => filesGoBack = fn} navRequest={filesNavReq} />
    </div>
    <div class="page-layer" class:hidden={page !== 'board'}>
      {#if hubEligible}
        <Board session={filesSession} visible={page === 'board'} onGoBack={(fn) => boardGoBack = fn} issueRequest={boardIssueReq} jumped={!!jumpedFrom} />
      {/if}
    </div>
    <div class="page-layer term-page" class:hidden={page !== 'terminal'}>
      <!-- The session/window list: a column beside the terminal on a wide
           screen, a slide-over sheet on a phone (opened from the switcher's
           session tag). Kept MOUNTED so its polling and expansion state
           survive; `visible` pauses the poll while the page is hidden. -->
      <aside class="term-side" class:side-sheet={layout.isTouchDevice && narrowVp} class:sheet={layout.isTouchDevice} class:open={layout.isTouchDevice && narrowVp && sessListOpen}>
        {#if !layout.isTouchDevice}<SideHandle />{/if}
        <Sessions {openTerminal} activeTarget={terminalTarget}
          visible={page === 'terminal' && (!layout.isTouchDevice || sessListOpen)}
          onPick={() => sessListOpen = false}
          chips={false} />
      </aside>
      {#if layout.isTouchDevice && sessListOpen}
        <button class="side-scrim" aria-label={t('close')} onclick={() => sessListOpen = false}></button>
      {/if}
      <div class="term-main">
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
              visible={page === 'terminal'}
              onActivate={(id) => activeCellId = id}
              onAssign={assignCell}
              onCloseCell={closeCell}
              onPaneExit={cellPaneExit} />
          {:else}
            <Terminal target={terminalTarget} session={terminalSession} command={terminalCommand} {fontSize}
              visible={page === 'terminal'}
              splitEligible={splitEligible} {splitActive} {splitLayout} onSetLayout={setLayout}
              onOpenSessions={layout.isTouchDevice ? () => sessListOpen = true : null}
              onSwitchPane={(t, cmd) => { terminalTarget = t; terminalSession = t.split(':')[0]; terminalCommand = cmd || ''; }} onPaneExit={paneExitFallback} />
          {/if}
        </div>
      {:else}
        <!-- No pane selected. The HEADER still renders: every other page keeps
             its `.page-head` when its detail pane is empty (Chat, Agents,
             Settings), and dropping it here made the Terminal tab look like a
             different app the moment nothing was open (owner, 2026-08-19). -->
        <div class="page-head">
          <h1>{t('terminal')}</h1>
        </div>
        <div class="terminal-empty">
          <Icon name="terminal" size={22} />
          <span>{t('noTerminalSelected')}</span>
          <button class="chip-btn" onclick={() => sessListOpen = true}>
            <Icon name="sessions" size={14} />{t('sessions')}
          </button>
        </div>
      {/if}
      </div>
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
  {#if connected && layout.isTouchDevice}
    <nav class="tabbar">

      {#if hubEligible}
        <button tabindex="-1" class:active={page === 'hub'} onclick={() => switchTab('hub')}>
          <Icon name="chat" size={19} /><span>{t('hub')}</span>
        </button>
        <button tabindex="-1" class:active={page === 'board'} onclick={() => switchTab('board')}>
          <Icon name="layout" size={19} /><span>{t('board')}</span>
        </button>
      {/if}
      <button tabindex="-1" class:active={page === 'files'} onclick={() => switchTab('files')}>
        <Icon name="files" size={19} /><span>{t('files')}</span>
      </button>
      <button tabindex="-1" class:active={page === 'terminal'} onclick={() => switchTab('terminal')}>
        <Icon name="terminal" size={19} /><span>{t('terminal')}</span>
      </button>
      <!-- No Agents icon here: on a phone the agent configuration is a CATEGORY
           OF SETTINGS (nav-state's agentsLivesInSettings), because this row had
           one tab too many (owner, 2026-08-29: "不用单独在底下一行展示了，现在
           看着有点多底下的标签"). The desktop rail keeps it as a page. -->
      <button tabindex="-1" class:active={page === 'prefs'} onclick={togglePrefs}>
        <Icon name="gear" size={19} /><span>{t('settings')}</span>
      </button>
    </nav>
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
    font-size: var(--fs-micro);
    line-height: 1.3;
    border-radius: var(--ui-radius-control);
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
    font-size: var(--fs-meta);
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
    font-size: var(--fs-micro);
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
    height: calc(var(--app-height, 100dvh) / var(--ui-zoom, 1));
    max-width: calc(100vw / var(--ui-zoom, 1));
    /* The shell reserves the top inset ONCE, for every screen. Android draws
       edge-to-edge (enforced from targetSdk 35), so without this the topmost
       header sits under the status bar. It cannot live on the page headers:
       only the disconnected `.topbar` used to carry it, so every CONNECTED
       page — which renders its own `.page-head` / win-bar instead — overlapped
       the notification strip. `box-sizing: border-box` is global, so the
       padding comes out of the content box and the terminal's fit math still
       adds up. Mirrors `.tabbar`'s `padding-bottom: var(--sab)` at the bottom.
       `--sat` is `env(safe-area-inset-top)`, overridden by an inline value that
       MainActivity pushes in from WindowInsetsCompat — WebView only forwards
       systemBars() to CSS from M136 (fullscreen) / M144 (always), so on older
       WebViews the Kotlin path is the only source. Both land on this one var. */
    padding-top: var(--sat);
    overflow: hidden;
    background: linear-gradient(180deg, var(--bg) 0%, var(--bg2) 50%, var(--bg3) 100%);
  }

  /* Disconnected top bar (brand + gear), both platforms. The safe area is
     `main`'s job now, so this is plain padding. */
  .topbar {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 8px 12px;
    background: var(--nav-bg);
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    z-index: 10;
  }

  /* Connected desktop: the left icon rail. The top bar is gone — every page
     gets its vertical space back and switching is one always-visible click. */
  .rail {
    position: fixed;
    left: 0; top: 0; bottom: 0;
    width: 46px;
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 4px;
    padding: 10px 0 12px;
    background: var(--nav-bg);
    border-right: 1px solid var(--border);
    z-index: 12;
  }
  .rail-brand { border-radius: var(--ui-radius-control); margin-bottom: 8px; flex: none; }
  .rail-btn {
    width: 34px; height: 32px;
    display: grid; place-items: center;
    border: none; border-radius: var(--ui-radius-control); background: none;
    color: var(--text3); cursor: pointer;
    transition: color var(--t-fast), background var(--t-fast);
    -webkit-tap-highlight-color: transparent;
  }
  .rail-btn:hover { color: var(--text); background: var(--surface2); }
  .rail-btn.active { color: var(--accent); background: var(--accent-bg); }
  .rail-spacer { flex: 1; }

  /* Reordering the rail. Two cues, because one is not enough to say both WHAT
     is moving and WHERE it lands: the pressed icon is CARRIED (it follows the
     pointer by transform, which reflows nothing, so the drop geometry stays the
     rects snapshotted at drag start) and wears the accent selection it already
     uses for "active"; the insertion point is an accent LINE on the edge the
     icon would push down. Hover is suppressed on the icons it passes over —
     mid-drag, a highlight under the cursor reads as a second selection. */
  .rail.reordering { cursor: grabbing; user-select: none; }
  .rail.reordering .rail-btn { cursor: grabbing; }
  .rail.reordering .rail-btn:not(.dragging):hover { color: var(--text3); background: none; }
  .rail-btn.dragging {
    color: var(--accent); background: var(--accent-bg);
    position: relative; z-index: 2;
    box-shadow: 0 2px 10px rgba(0, 0, 0, 0.35);
  }
  .rail-drop {
    position: absolute; left: 7px; right: 7px; height: 2px;
    background: var(--accent); border-radius: 1px;
    z-index: 1; pointer-events: none;
  }
  main.with-rail { padding-left: 46px; }

  /* Connected mobile: bottom tab bar in thumb reach. Hidden while the
     keyboard is up so immersive typing (terminal, editor) costs nothing —
     the ONLY writer of that class is App's viewport handler. */
  .tabbar {
    display: flex;
    align-items: stretch;
    flex-shrink: 0;
    background: var(--nav-bg);
    border-top: 1px solid var(--border);
    padding-bottom: var(--sab);
    z-index: 10;
  }
  :global(html.keyboard-open) .tabbar { display: none; }
  .tabbar button {
    flex: 1;
    display: flex; flex-direction: column; align-items: center; gap: 2px;
    padding: 7px 0 5px;
    border: none; background: none;
    color: var(--text3); cursor: pointer;
    font-size: var(--fs-meta);
    -webkit-tap-highlight-color: transparent;
  }
  .tabbar button.active { color: var(--accent); }
  .tabbar button:active { transform: scale(0.95); }

  .nav-right {
    display: flex;
    align-items: center;
    gap: 10px;
    margin-left: auto;
  }

  .gear-btn {
    padding: 6px; border: none; border-radius: var(--ui-radius-control); background: none;
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
    font-size: var(--fs-title);
    color: var(--text2);
    letter-spacing: -0.3px;
  }
  .brand-accent { color: var(--accent); }

  .reconnect-bar {
    display: flex; align-items: center; justify-content: center; gap: 8px;
    padding: 6px; background: var(--bg2); border-bottom: 1px solid var(--accent); color: var(--accent);
    font-size: var(--fs-ui); font-weight: 500; flex-shrink: 0;
  }
  .reconnect-spinner {
    width: 12px; height: 12px; border: 2px solid var(--border);
    border-top-color: var(--accent); border-radius: 50%;
    animation: reconnect-spin 0.6s linear infinite;
  }
  @keyframes reconnect-spin { to { transform: rotate(360deg); } }
  .reconnect-cancel {
    margin-left: auto; padding: 2px 10px; border: 1px solid var(--accent);
    border-radius: var(--ui-radius-control); background: none; color: var(--accent); font-size: var(--fs-sub);
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
  /* Terminal = session list + terminal, one page (the Sessions tab was
     retired into this). Wide: two columns. Phone: the list slides over,
     because the terminal needs the whole screen there. */
  /* Specificity note: .page-layer sets display:flex later in this sheet, so
     the merged layout must qualify with BOTH classes to win. */
  /* The left column is THE shared sidebar (ui-unification.md §1): same
     `var(--sidebar-w)` geometry and the same SideHandle as Chat / Agents /
     Files, so switching rail tabs no longer makes the left region jump. It
     was pinned at 280px because this column used to render project CARDS
     with a tray of pane pills that wrapped one-per-line at 240 — dense mode
     retired both, so the exception expired with them (owner, 2026-08-19). */
  .page-layer.term-page { display: grid; grid-template-columns: var(--sidebar-w) minmax(0, 1fr); min-height: 0; }
  .term-side { position: relative; display: flex; flex-direction: column; min-width: 0; min-height: 0; border-right: 1px solid var(--border); background: var(--bg2); }
  .term-main { position: relative; display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  @media (max-width: 760px) {
    .page-layer.term-page { grid-template-columns: minmax(0, 1fr); }
    /* The sheet geometry/motion is the SHARED .side-sheet dialect in app.css
       (owner, 2026-08-30: one drawer for Chat/Terminal/Board). */
  }

  /* Swipe transition animations */
  /* will-change ONLY while the 120ms slide runs (slideAnim clears right
     after): a permanent `will-change: transform` makes .page the CONTAINING
     BLOCK for every position:fixed popover inside it. On the phone .page
     starts at viewport x=0 so nothing showed, but on desktop the rail pads
     .page 46px right — and every fixed menu (the shared Select's list, the
     agent dot menu, context menus) rendered 46px right of where the
     viewport-space math put it (owner, 2026-08-25: "下拉框整体往右偏了").
     No popover can be open during the tab slide, so the transient containing
     block is harmless. */
  .page.slide-in-left, .page.slide-in-right { will-change: transform; }
  .page.slide-in-left   { animation: slideInLeft 0.12s linear; }
  .page.slide-in-right  { animation: slideInRight 0.12s linear; }
  @keyframes slideInLeft  { from { transform: translateX(-40%); } to { transform: none; } }
  @keyframes slideInRight { from { transform: translateX(40%); } to { transform: none; } }
  .page-layer {
    position: absolute;
    inset: 0;
    display: flex;
    flex-direction: column;
    /* NO z-index here. With one, each page layer becomes a stacking context
       that CAPS every fixed overlay inside it at that value — a dialog's
       z-index: 31 lost to the tabbar's z-index: 10, so on the phone every
       bottom sheet's action row (create project, close/delete confirms) sat
       BEHIND the tabbar once fixed layers anchored to the true viewport
       ("手机上窗口被下边被最下边挡住了，按钮看不到，包括关闭确认框", owner
       2026-08-25). Layers are visibility-toggled, never z-fought, so the
       stacking context bought nothing. */
    z-index: auto;
  }
  .page-layer.hidden {
    visibility: hidden;
    pointer-events: none;
  }

  .terminal-body { flex: 1; min-height: 0; position: relative; display: flex; flex-direction: column; }
  .terminal-empty {
    flex: 1; min-height: 0;
    display: flex; flex-direction: column; align-items: center; justify-content: center;
    gap: 9px; color: var(--text3); font-size: var(--fs-body);
  }
  .terminal-empty :global(svg) { opacity: 0.65; }

  /* Split-layout control: a single floating icon in the terminal's top-right
     corner (no full-width toolbar row). Opens a small popover with 1/2/3/4/6. */
  .split-control { position: absolute; top: 6px; right: 8px; z-index: 12; }
  .split-toggle {
    width: 28px; height: 28px; padding: 0;
    border: 1px solid var(--border); border-radius: var(--ui-radius-row);
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
    background: var(--bg); border: 1px solid var(--border); border-radius: var(--ui-radius-panel);
    box-shadow: 0 8px 28px rgba(0,0,0,0.35);
  }
  .split-opt {
    min-width: 28px; height: 28px;
    border: none; border-radius: var(--ui-radius-control); background: transparent;
    color: var(--text3); font-size: var(--fs-body); font-weight: 600; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .split-opt:hover { background: var(--surface2); color: var(--text2); }
  .split-opt.active { background: var(--accent-bg); color: var(--accent); }
  .split-menu-backdrop { position: fixed; inset: 0; z-index: 12; background: transparent; border: none; cursor: default; }
</style>
