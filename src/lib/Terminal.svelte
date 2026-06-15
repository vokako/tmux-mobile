<script>
  import { subscribe, unsubscribe, addPaneOutputListener, removePaneOutputListener, addPaneClosedListener, removePaneClosedListener, sendCommand, sendKeys, listPanes, listSessionsWithPanes, capturePane, resizePane, newWindow } from './ws.js';
  import { Terminal } from '@xterm/xterm';
  import { WebLinksAddon } from '@xterm/addon-web-links';
  import ChatView from './ChatView.svelte';
  import Icon from './Icon.svelte';
  import AgentChip from './AgentChip.svelte';
  import PanePicker from './PanePicker.svelte';
  import { t } from './i18n.svelte.js';
  import { detectParser } from './parsers.js';
  import { detectAgent, paneIsAgent, paneAgent, sessionHasAgent, AGENTS } from './agents.js';
  import { copyText } from './clipboard.js';

  // Timing constants
  const WINDOW_LIST_POLL_MS = 5000;
  // Max wait for server to echo our resize. If never confirmed (external resize
  // or slow tmux), client falls back to trusting server-reported dimensions.
  const RESIZE_CONFIRM_TIMEOUT_MS = 5000;
  const LONG_PRESS_MS = 500;
  const TOUCH_END_DELAY_MS = 500;

  // xterm.js fallback cell size ratios (used when render dimensions unavailable)
  const CELL_W_RATIO = 0.6;
  const CELL_H_RATIO = 1.2;

  // Touch scrolling physics
  const MOMENTUM_MAX_PX = 240;
  const MOMENTUM_FRICTION = 0.95;
  const MOMENTUM_MIN_V = 0.05;
  const SCROLLBAR_TOUCH_WIDTH = 30;

  // `embedded` = rendered inside a split-screen cell. The cell uses this
  // Terminal's OWN window-switcher bar as its header (same form as the
  // single-pane view), so when embedded we always show that bar and add a
  // close button to it. `onClose` (split only) closes the cell.
  // `active` (split only): is this the focused cell? Only the active cell
  // grabs DOM focus for its hidden xterm textarea — there is exactly ONE
  // focusable textarea per document, so N cells auto-focusing on mount/rebuild
  // fight each other and end up with input going nowhere. Single-pane is
  // always active.
  // `chromeless` = embedded with NO window-switcher bar (used by the desktop
  // agent grid, where each cell is pinned to one agent's pane — there is
  // nothing to switch to, so the bar would only steal vertical space).
  let { target, session, command: initialCommand = '', viewMode = 'terminal', fontSize = 14, embedded = false, active = true, chromeless = false, onChatSupported = () => {}, onSwitchPane = null, onPaneExit = () => {}, onClose = null } = $props();

  let input = $state('');
  let paneContent = $state('');
  let command = $state(initialCommand);
  $effect(() => { command = initialCommand; });
  let termEl;
  let term;
  let termAtBottom = $state(true);
  let hasNewContent = $state(false); // set when new output arrives while user is scrolled up
  let toastMsg = $state('');
  const isMobile = 'ontouchstart' in window || navigator.maxTouchPoints > 0;
  // Visual width of xterm's overlay scrollbar (passed to the Terminal ctor
  // below). It floats ON TOP of the content's right edge and does NOT
  // reserve layout space — calcFit deliberately uses the full width so the
  // pane gets the maximum column count; the trade-off is the scrollbar
  // briefly overlapping the last glyph.
  const SCROLLBAR_W = isMobile ? 20 : 14;
  let kbBlurTimer = null;
  let kbLocked = true; // true = keyboard must not show; false = keyboard allowed
  let unlockUntil = 0; // grace window after explicit unlock; auto-lock paths must respect it
  let unlockRetries = 0; // blur re-focus attempts inside the current grace window
  const UNLOCK_RETRY_MAX = 2;
  let endTouchScrollTimer = null;
  let kbTa = null; // set in $effect after term.open

  function unlockKeyboard() {
    clearTimeout(kbBlurTimer);
    clearTimeout(endTouchScrollTimer);
    kbLocked = false;
    unlockUntil = Date.now() + 1500;
    unlockRetries = 0;
    // inputmode is pinned to "text" at init; no toggle here.
    if (kbTa) {
      // If the textarea is somehow already focused while the IME is
      // hidden (e.g. user closed IME via the system keyboard's own button
      // or back gesture, leaving view focus untouched), a fresh focus()
      // is a no-op and the IME stays down. blur() first to make the next
      // focus a true focus transition.
      if (document.activeElement === kbTa) kbTa.blur();
      kbTa.focus();
    }
    window.__dbg?.('kb: unlock + focus');
  }

  function showToast(msg) {
    toastMsg = msg;
    setTimeout(() => { toastMsg = ''; }, 1500);
  }

  let theme = $state(document.documentElement.getAttribute('data-theme') || 'dark');

  $effect(() => {
    const obs = new MutationObserver(() => {
      theme = document.documentElement.getAttribute('data-theme') || 'dark';
    });
    obs.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    return () => obs.disconnect();
  });

  const darkTheme = {
    background: '#0a0a0f', foreground: '#c9d1d9', cursor: '#00d4ff',
    selectionBackground: 'rgba(0, 212, 255, 0.18)',
    black: '#0a0a0f', brightBlack: '#484848',
    red: '#ff5050', brightRed: '#ff6b6b',
    green: '#4ade80', brightGreen: '#6ee7a0',
    yellow: '#fbbf24', brightYellow: '#fcd34d',
    blue: '#00d4ff', brightBlue: '#38bdf8',
    magenta: '#c084fc', brightMagenta: '#d8b4fe',
    cyan: '#22d3ee', brightCyan: '#67e8f9',
    white: '#c9d1d9', brightWhite: '#f1f5f9',
  };
  const lightTheme = {
    background: '#f5f5f7', foreground: '#1a1a2e', cursor: '#0088cc',
    selectionBackground: 'rgba(0, 136, 204, 0.18)',
    black: '#f5f5f7', brightBlack: '#9ca3af',
    red: '#dc2626', brightRed: '#ef4444',
    green: '#16a34a', brightGreen: '#22c55e',
    yellow: '#ca8a04', brightYellow: '#eab308',
    blue: '#0088cc', brightBlue: '#2563eb',
    magenta: '#9333ea', brightMagenta: '#a855f7',
    cyan: '#0891b2', brightCyan: '#06b6d4',
    white: '#1a1a2e', brightWhite: '#0f0f1a',
  };

  function getTermTheme() {
    return theme === 'light' ? lightTheme : darkTheme;
  }

  // Sync theme when light/dark changes
  $effect(() => {
    if (!term) return;
    const t = getTermTheme();
    term.options.theme = t;
    if (termEl) {
      termEl.style.background = t.background;
    }
  });

  $effect(() => {
    if (!term) return;
    term.options.fontSize = fontSize;
    // xterm re-measures cell geometry on the next render, not synchronously.
    // Defer refit by two frames so calcFit reads the new cell width/height.
    // doResizeRef is set by the main $effect after term is created.
    requestAnimationFrame(() => {
      requestAnimationFrame(() => doResizeRef?.());
    });
  });
  let doResizeRef = null;

  let parser = $derived(detectParser('', command));

  $effect(() => { onChatSupported(!!parser); });

  // pane_output snapshots now carry `current_command` (server piggybacks it
  // on cursor reads — same tmux subprocess, zero extra cost). Update
  // `command` in the pane-output listener below; no separate polling RPC needed.

  let waitingForInput = $derived.by(() => {
    if (!paneContent || !parser) return false;
    return parser.isWaitingForInput(paneContent);
  });

  let statusInfo = $derived.by(() => {
    if (!paneContent || !parser) return null;
    return parser.extractStatus(paneContent);
  });

  // Window switcher
  let windowPanes = $state([]);
  let showWindowCmd = $state(localStorage.getItem('tmux_winswitcher') === '1');
  let showPanePicker = $state(false); // session-badge → jump-to-any-pane popover

  // Desktop split: when this cell becomes the active one, pull DOM focus to
  // its xterm textarea so keystrokes route here. (Single-pane: active is
  // always true; the mousedown/mount focus covers it.)
  $effect(() => {
    if (!embedded || isMobile || !active || !term) return;
    requestAnimationFrame(() => { try { term?.focus(); } catch {} });
  });
  let currentWindow = $derived(target.split(':')[1]?.split('.')[0] || '');

  // Other AI sessions — shown as chips in the expanded window-switcher so
  // users can jump between parallel coding-agent sessions (Kiro/Claude/…)
  // without backing out to the Sessions page. Loaded only when the switcher
  // is expanded to avoid an unnecessary RPC tick on every Terminal view.
  let otherAgentSessions = $state([]); // [{ name, pane, agent }]
  const OTHER_AGENT_MAX = 5;

  async function loadOtherAgentSessions() {
    try {
      // Single round-trip: sessions + all panes in one RPC instead of
      // listSessions + N × listPanes (which on a slow link stacked N+1
      // requests behind every poll tick).
      const { sessions, panes } = await listSessionsWithPanes();
      const cur = session;
      const panesBySession = new Map();
      for (const p of panes) {
        const arr = panesBySession.get(p.session);
        if (arr) arr.push(p); else panesBySession.set(p.session, [p]);
      }
      // Sort MRU-first (matches Sessions page order), filter current, keep
      // sessions that have last_opened so we don't surface never-used ones.
      const candidates = sessions
        .filter(s => s.name !== cur && s.last_opened)
        .sort((a, b) => (b.last_opened || 0) - (a.last_opened || 0));
      const results = [];
      for (const s of candidates) {
        if (results.length >= OTHER_AGENT_MAX) break;
        const sPanes = panesBySession.get(s.name) || [];
        // Prefer a pane currently running the agent; fall back to first pane.
        const p = sPanes.find(paneIsAgent) || (sessionHasAgent(sPanes) ? sPanes[0] : null);
        if (p) {
          const agent = paneAgent(p);
          if (agent) results.push({ name: s.name, pane: p, agent });
        }
      }
      otherAgentSessions = results;
    } catch {
      otherAgentSessions = [];
    }
  }

  // Group panes by window. Representative pane preference:
  //   agent pane > active pane > first listed.
  // The old "first listed" pick made a window whose layout is
  // [zsh | claude] show a zsh chip — the agent badge silently vanished.
  let windows = $derived.by(() => {
    const map = new Map();
    for (const p of windowPanes) {
      const cur = map.get(p.window);
      if (!cur) { map.set(p.window, p); continue; }
      const curScore = paneIsAgent(cur) ? 2 : cur.active ? 1 : 0;
      const pScore = paneIsAgent(p) ? 2 : p.active ? 1 : 0;
      if (pScore > curScore) map.set(p.window, p);
    }
    return [...map.values()];
  });

  // Agent (if any) running in the currently-shown window.
  let currentWinAgent = $derived.by(() => {
    const cur = windows.find(w => String(w.window) === currentWindow);
    if (!cur) return null;
    return paneAgent(cur);
  });

  // Whether the window switcher is worth showing at all. A lone plain shell
  // with no agent anywhere has nothing to switch to — showing an (almost)
  // empty bar just steals vertical space. Show it only when there's a real
  // choice: multiple windows, the current window is an agent, or other agent
  // sessions exist (the latter is only known while expanded, since we don't
  // poll cross-session data when collapsed).
  // Embedded (split cell) always shows the bar — it's the cell's header.
  // Standalone keeps the "only when there's something to switch to" rule.
  let showSwitcher = $derived(
    !chromeless &&
    (embedded || windows.length > 1 || !!currentWinAgent || otherAgentSessions.length > 0)
  );

  $effect(() => {
    if (!session || viewMode !== 'terminal') return;
    // Chromeless cells (agent grid) have no switcher, so the window-list poll
    // is pure waste — skip it entirely (N agent cells would otherwise each
    // poll listPanes every few seconds).
    if (chromeless) return;
    // In-flight guard: on a slow link a poll RPC can outlive the 5 s
    // interval; without the guard, ticks stack unbounded requests on a
    // link that is already struggling.
    let polling = false;
    const load = async () => {
      if (polling) return;
      polling = true;
      try {
        const p = await listPanes(session);
        windowPanes = p;
        // Cross-session AI chips: only when the switcher is expanded AND not
        // a split cell (in split mode every cell already shows the bar; we
        // don't want N cells each fanning out a listSessionsWithPanes tick).
        if (showWindowCmd && !embedded) await loadOtherAgentSessions();
      } catch {}
      polling = false;
    };
    load();
    const id = setInterval(load, WINDOW_LIST_POLL_MS);
    return () => clearInterval(id);
  });

  // When the user expands the switcher, fetch immediately (don't wait for
  // the next poll tick).
  $effect(() => {
    if (showWindowCmd && session && viewMode === 'terminal') {
      loadOtherAgentSessions();
    }
  });

  // Read xterm's actual rendered cell dimensions (falls back to font-size-based estimate
  // before first paint). Single source of truth for calcFit / touch mapping / momentum scroll.
  function cellSize(t) {
    if (!t) return { w: 0, h: 0 };
    const core = t._core;
    return {
      w: core?._renderService?.dimensions?.css?.cell?.width || (t.options.fontSize * CELL_W_RATIO),
      h: core?._renderService?.dimensions?.css?.cell?.height || (t.options.fontSize * CELL_H_RATIO),
    };
  }

  // Calculate optimal cols/rows based on current container size
  function calcFit() {
    if (!term || !termEl) return null;
    const { w: cellW, h: cellH } = cellSize(term);
    // Use the full container width. The overlay scrollbar floats ON TOP of
    // the rightmost column rather than reserving space — that's acceptable
    // (a brief overlap of the last glyph) in exchange for more usable
    // columns. Because cols = floor(w / cellW), cols × cellW ≤ w, so the
    // text's right edge never spills past the screen.
    const w = termEl.clientWidth;
    const h = termEl.clientHeight;
    if (!w || !h || !cellW || !cellH) return null;
    return { cols: Math.max(2, Math.floor(w / cellW)), rows: Math.max(1, Math.floor(h / cellH)) };
  }

  let touchScrolling = false; // set by touch handler, pauses content updates

  // ─── Mobile text selection ────────────────────────────────────────────────
  // First principle: a selection is an *object* (anchor + head, both inclusive
  // buffer-row/col), not a transient state of the touch handler. Once made
  // (long-press, double/triple-tap), it lives until the user explicitly copies
  // (toolbar) or cancels (tap outside, new long-press, pane switch).
  //
  // The two endpoints are independently draggable via handles. We never store
  // pre-sorted (start, end) in the source-of-truth — selStart/selEnd derive
  // them from anchor/head so a handle drag that crosses the other endpoint
  // just flips which one is "leading" without any swap bookkeeping.
  let selection = $state(null); // null | { anchor: {row, col}, head: {row, col} }
  let selUI = $state(null);     // pixel-space UI: { startX, startY, endX, endY, toolbarX, toolbarY, toolbarBelow, startInView, endInView, toolbarVisible }
  let isApplyingSelection = false; // guard onSelectionChange while we drive term.select ourselves
  // Toolbar button handlers — assigned inside the $effect that owns `term`,
  // `lastContent`, etc. The template guards on `selection != null`, which can
  // only happen after the effect has run, so the assignment is always live
  // when the buttons can be clicked.
  let copySelection = () => {};
  let clearSelection = () => {};

  function selStart(s) {
    if (!s) return null;
    const { anchor, head } = s;
    if (head.row < anchor.row || (head.row === anchor.row && head.col < anchor.col)) return head;
    return anchor;
  }
  function selEnd(s) {
    if (!s) return null;
    const { anchor, head } = s;
    if (head.row < anchor.row || (head.row === anchor.row && head.col < anchor.col)) return anchor;
    return head;
  }
  // Resize confirmation: after local resize, we expect server to echo cursor.w/cursor.h
  // matching pendingCols/pendingRows. Until confirmed, ignore server dims (stale).
  let pendingCols = 0, pendingRows = 0, pendingResizeTs = 0;

  // Count lines without allocating a split array
  function countLines(s) { let n = 1; for (let i = 0; i < s.length; i++) if (s[i] === '\n') n++; return n; }

  // Adapt ANSI RGB / 256-color codes so the terminal content stays readable
  // regardless of theme. The old behavior was a flat RGB inversion in light
  // mode, which produced near-black blocks (e.g. Kiro Tasks panel) and broke
  // hue. This version preserves hue + saturation; it only reshapes luminance
  // when a color would either (a) lack contrast against the terminal bg (FG),
  // or (b) clash with / blend into the terminal bg (BG block).
  //
  // Notes / limits:
  // - We process each ANSI color independently; FG and BG aren't re-balanced
  //   as pairs, so hand-picked FG/BG color combos can lose contrast in
  //   extreme cases (e.g. purple bg + yellow fg in light mode). Full pair
  //   handling would need a small SGR state machine; tracked in unresolved.
  // - 256-color indices 16..255 are converted through the standard palette and
  //   rewritten as truecolor. Indices 0..15 are left to xterm.js's theme.
  const TERM_BG_L_DARK = 0.02;      // WCAG L of dark theme bg (#0a0a0f)
  const TERM_BG_L_LIGHT = 0.91;     // WCAG L of light theme bg (#f5f5f7)
  const MIN_FG_CONTRAST = 3.5;      // WCAG AA large-text threshold
  const BG_CLASH_RATIO_DARK = 4.5;  // dark bg block > 4.5× term bg → too bright
  const BG_CLASH_RATIO_LIGHT = 1.8; // light bg block < 1/1.8× term bg → too dark
  const BG_BLEND_RATIO_LIGHT = 1.15;// light bg within 1.15× of term bg → invisible
  const HSL_L_BG_DARK = 0.30;
  const HSL_L_BG_LIGHT = 0.75;
  const HSL_L_FG_DARK = 0.72;
  const HSL_L_FG_LIGHT = 0.28;

  function _toLinChannel(c) {
    c /= 255;
    return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4);
  }
  function _luminance(r, g, b) {
    return 0.2126 * _toLinChannel(r) + 0.7152 * _toLinChannel(g) + 0.0722 * _toLinChannel(b);
  }
  function _contrast(l1, l2) {
    const a = Math.max(l1, l2), b = Math.min(l1, l2);
    return (a + 0.05) / (b + 0.05);
  }
  function _rgbToHsl(r, g, b) {
    r /= 255; g /= 255; b /= 255;
    const mx = Math.max(r, g, b), mn = Math.min(r, g, b);
    let h = 0, s = 0;
    const l = (mx + mn) / 2;
    if (mx !== mn) {
      const d = mx - mn;
      s = l > 0.5 ? d / (2 - mx - mn) : d / (mx + mn);
      if (mx === r) h = ((g - b) / d + (g < b ? 6 : 0)) / 6;
      else if (mx === g) h = ((b - r) / d + 2) / 6;
      else h = ((r - g) / d + 4) / 6;
    }
    return [h, s, l];
  }
  function _hslToRgb(h, s, l) {
    let r, g, b;
    if (s === 0) { r = g = b = l; }
    else {
      const hue2rgb = (p, q, t) => {
        if (t < 0) t += 1;
        if (t > 1) t -= 1;
        if (t < 1/6) return p + (q - p) * 6 * t;
        if (t < 0.5) return q;
        if (t < 2/3) return p + (q - p) * (2/3 - t) * 6;
        return p;
      };
      const q = l < 0.5 ? l * (1 + s) : l + s - l * s;
      const p = 2 * l - q;
      r = hue2rgb(p, q, h + 1/3);
      g = hue2rgb(p, q, h);
      b = hue2rgb(p, q, h - 1/3);
    }
    return [Math.round(r * 255), Math.round(g * 255), Math.round(b * 255)];
  }
  function _idx256ToRgb(n) {
    if (n < 16) return null;
    if (n >= 232) {
      const v = (n - 232) * 10 + 8;
      return [v, v, v];
    }
    const i = n - 16;
    return [Math.floor(i / 36) * 51, Math.floor((i % 36) / 6) * 51, (i % 6) * 51];
  }
  function _adjustColor(r, g, b, isBg, isDark) {
    const L = _luminance(r, g, b);
    const [h, s] = _rgbToHsl(r, g, b);
    if (isBg) {
      const termL = isDark ? TERM_BG_L_DARK : TERM_BG_L_LIGHT;
      if (isDark) {
        if ((L + 0.05) / (termL + 0.05) > BG_CLASH_RATIO_DARK) {
          return _hslToRgb(h, s, HSL_L_BG_DARK);
        }
        return [r, g, b];
      }
      const ratio = (termL + 0.05) / (L + 0.05);
      if (ratio > BG_CLASH_RATIO_LIGHT || ratio < BG_BLEND_RATIO_LIGHT) {
        return _hslToRgb(h, s, HSL_L_BG_LIGHT);
      }
      return [r, g, b];
    }
    const bgL = isDark ? TERM_BG_L_DARK : TERM_BG_L_LIGHT;
    if (_contrast(L, bgL) >= MIN_FG_CONTRAST) return [r, g, b];
    return _hslToRgb(h, s, isDark ? HSL_L_FG_DARK : HSL_L_FG_LIGHT);
  }

  // Compute xterm row (1-based) + required padding for cursor placement.
  // Shared by full-rewrite and cursor-only paths to ensure they stay in sync.
  function computeCursorLayout(content, cursor, rows) {
    const N = countLines(content);
    const trailing = cursor.t || 0;
    const paneStart = Math.max(0, N + trailing - cursor.h);
    const cursorLine = paneStart + cursor.y;
    const needAfter = Math.max(0, cursorLine + 1 - N);
    const contentLines = N + needAfter;
    const topPadCount = contentLines < rows ? rows - contentLines : 0;
    const totalWritten = topPadCount + contentLines;
    const sb = Math.max(0, totalWritten - rows);
    return {
      row: topPadCount + cursorLine - sb + 1,
      topPad: topPadCount > 0 ? '\n'.repeat(topPadCount) : '',
      afterPad: needAfter > 0 ? '\n'.repeat(needAfter) : '',
    };
  }

  // Write content + position cursor in xterm.js.
  // Color adaptation runs per line, with a Map cache keyed by (rawLine, theme).
  // Hit rate is high in streaming scenarios because most lines repeat verbatim
  // between snapshots — only the few changing lines re-run the regex pass.
  // We bound the cache so unbounded scrollback doesn't grow it forever.
  const _colorCache = new Map();
  const _COLOR_CACHE_MAX = 4000;
  function adaptLine(rawLine, isDark) {
    const key = (isDark ? 'd:' : 'l:') + rawLine;
    const hit = _colorCache.get(key);
    if (hit !== undefined) return hit;
    let out = rawLine.replace(/\x1b\[(3|4)8;2;(\d+);(\d+);(\d+)m/g, (_m, type, r, g, b) => {
      const isBg = type === '4';
      const [nr, ng, nb] = _adjustColor(+r, +g, +b, isBg, isDark);
      return `\x1b[${type}8;2;${nr};${ng};${nb}m`;
    });
    out = out.replace(/\x1b\[(3|4)8;5;(\d+)m/g, (m, type, n) => {
      const rgb = _idx256ToRgb(+n);
      if (!rgb) return m;
      const isBg = type === '4';
      const [nr, ng, nb] = _adjustColor(rgb[0], rgb[1], rgb[2], isBg, isDark);
      return `\x1b[${type}8;2;${nr};${ng};${nb}m`;
    });
    if (_colorCache.size >= _COLOR_CACHE_MAX) {
      // Drop oldest entry. Map iteration is insertion-ordered.
      const firstKey = _colorCache.keys().next().value;
      if (firstKey !== undefined) _colorCache.delete(firstKey);
    }
    _colorCache.set(key, out);
    return out;
  }
  function adaptColors(text) {
    const isDark = theme !== 'light';
    if (text.indexOf('\n') < 0) return adaptLine(text, isDark);
    // Split, adapt per line, rejoin. \n is preserved at line boundaries.
    const lines = text.split('\n');
    for (let i = 0; i < lines.length; i++) lines[i] = adaptLine(lines[i], isDark);
    return lines.join('\n');
  }

  // Coalesce high-frequency snapshots into one render per animation frame.
  // When token streams arrive at 30–60 Hz, multiple snapshots collapse into
  // a single xterm write — saves CPU, eliminates the "two writes painting at
  // once" jitter, and bounded latency is the rAF interval (~16 ms, well
  // below human flicker threshold).
  // The pending frame holds only the LATEST content/cursor (older snapshots
  // are intentionally dropped — they are replaced wholesale, not appended).
  let _pendingContent = null;
  let _pendingCursor = null;
  let _pendingRaf = 0;

  function _flushPending() {
    _pendingRaf = 0;
    const c = _pendingContent;
    const cur = _pendingCursor;
    _pendingContent = null;
    _pendingCursor = null;
    if (c == null) return;
    _writeToXtermNow(c, cur);
  }

  function writeToXterm(content, cursor) {
    _pendingContent = content;
    _pendingCursor = cursor;
    if (_pendingRaf) return;
    _pendingRaf = requestAnimationFrame(_flushPending);
  }

  function _writeToXtermNow(content, cursor) {
    if (!term || touchScrolling) return;
    // Reconcile terminal dimensions with server-reported ones.
    // If we have a pending local resize, only clear it when server echoes matching dims;
    // otherwise ignore stale dims. If no pending (or expired), trust server.
    if (cursor?.w && cursor?.h) {
      const pendingActive = pendingResizeTs && Date.now() - pendingResizeTs < RESIZE_CONFIRM_TIMEOUT_MS;
      if (pendingActive) {
        if (cursor.w === pendingCols && cursor.h === pendingRows) {
          pendingResizeTs = 0; // confirmed
        }
        // else: stale, ignore
      } else if (term.cols !== cursor.w || term.rows !== cursor.h) {
        term.resize(cursor.w, cursor.h);
        pendingResizeTs = 0;
      }
    }
    const buf = term.buffer.active;
    const atBottom = buf.viewportY >= buf.baseY;
    const prevViewport = buf.viewportY;

    let cursorSeq = '', topPad = '', afterPad = '';
    if (cursor) {
      const layout = computeCursorLayout(content, cursor, term.rows);
      topPad = layout.topPad;
      afterPad = layout.afterPad;
      if (layout.row > 0 && layout.row <= term.rows) {
        cursorSeq = `\x1b[${layout.row};${cursor.x + 1}H`;
      }
    }

    if (buf.baseY > 0) term.clear();
    // Build the body so each line ends with SGR reset + erase-to-EOL. This
    // overwrites the previous frame's cells *in place* — xterm never has a
    // "fully blank" intermediate state, so there is no visible flash.
    // Compare with the old `\x1b[2J` (clear-screen) which emptied every cell
    // before painting, producing a one-frame flicker on every snapshot.
    const adapted = adaptColors(content);
    const lines = adapted.split('\n');
    let body = '';
    for (let i = 0; i < lines.length; i++) {
      body += lines[i] + '\x1b[0m\x1b[K';
      if (i < lines.length - 1) body += '\n';
    }
    // topPad / afterPad are sequences of '\n'; we add \x1b[K after each so
    // any stale cells on those rows are wiped without flashing.
    const padTop = topPad ? topPad.replace(/\n/g, '\x1b[0m\x1b[K\n') : '';
    const padAft = afterPad ? afterPad.replace(/\n/g, '\x1b[0m\x1b[K\n') : '';
    // Synchronized Output (mode 2026): tell xterm to defer rendering until
    // the whole batch is parsed. Effectively wraps the entire frame in a
    // single render commit, avoiding any partial-paint glimpses.
    term.write('\x1b[?2026h\x1b[?25l\x1b[H' + padTop + body + padAft + cursorSeq + '\x1b[?25h\x1b[?2026l', () => {
      if (!term || touchScrolling) return;
      if (atBottom) {
        term.scrollToBottom();
      } else {
        term.scrollToLine(Math.min(prevViewport, term.buffer.active.baseY));
      }
    });
  }

  // xterm.js setup + subscription
  $effect(() => {
    touchScrolling = false; // reset on pane switch
    pendingCols = 0; pendingRows = 0; pendingResizeTs = 0;
    kbLocked = true;
    selection = null; selUI = null;
    keyQueue = []; // queued keys belong to the previous pane

    const estCellW = fontSize * CELL_W_RATIO;
    const estCellH = fontSize * CELL_H_RATIO;
    const containerW = termEl?.clientWidth || 300;
    const containerH = termEl?.clientHeight || 400;
    const initCols = Math.max(2, Math.floor(containerW / estCellW));
    const initRows = Math.max(1, Math.floor(containerH / estCellH));
    term = new Terminal({
      cols: initCols,
      rows: initRows,
      cursorBlink: true,
      cursorStyle: 'block',
      disableStdin: false,
      fontSize,
      fontFamily: "'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace",
      fontWeight: 300,
      fontWeightBold: 600,
      theme: getTermTheme(),
      scrollback: 500,
      convertEol: true,
      allowTransparency: false,
      scrollbar: { showScrollbar: true, width: SCROLLBAR_W },
    });

    term.open(termEl);
    term.loadAddon(new WebLinksAddon((e, url) => {
      e.preventDefault();
      if (window.__TAURI_INTERNALS__) {
        import('@tauri-apps/plugin-opener').then(m => m.openUrl(url)).catch(() => window.open(url, '_blank'));
      } else {
        window.open(url, '_blank');
      }
    }));
    termEl.style.background = getTermTheme().background;

    // Mobile keyboard control:
    //   We pin inputmode="text" for the whole session and gate IME via the
    //   focus state (kbLocked + onTaFocus). Earlier we toggled
    //   inputmode="none" ↔ "text" around explicit unlocks, but that hit a
    //   nasty Android InputMethodManager quirk: when the textarea was
    //   created with inputmode="none", the very first focus after the
    //   first switch-to-"text" was ignored — the IME's InputConnection had
    //   already cached "this view doesn't want the soft keyboard" and only
    //   reset on a full blur+focus cycle. Users had to tap the toggle 3
    //   times to open the keyboard on a fresh page load.
    //
    //   Defenses against accidental IME we still have:
    //     - tabindex="-1" on every shortcut button (no focus stealing).
    //     - onTaFocus blurs immediately whenever kbLocked=true, so even
    //       if something does focus the textarea we don't get the IME up.
    //
    // The xterm.js helper textarea is recreated when xterm rebuilds its
    // DOM (e.g. fontSize change), so re-pinning inputmode is cheap to
    // repeat from any path that might rebuild it; we currently only set
    // it once and rely on xterm not changing it.
    kbTa = isMobile ? termEl.querySelector('.xterm-helper-textarea') : null;
    if (kbTa) {
      kbTa.setAttribute('inputmode', 'text');
    }

    // Forward keyboard input to tmux — skip when input box is open
    let isPasting = false;
    let isComposing = false;
    if (isMobile) {
      const ta = termEl?.querySelector('.xterm-helper-textarea');
      if (ta) {
        ta.addEventListener('paste', () => {
          isPasting = true;
          // Safety reset: if onData never fires (xterm swallowed it, or paste
          // produced no data), the flag would persist and misclassify the
          // next keystroke as paste.
          setTimeout(() => { isPasting = false; }, 200);
        });
        ta.addEventListener('compositionstart', () => { isComposing = true; });
        ta.addEventListener('compositionend', () => { isComposing = false; });
        ta.addEventListener('input', () => {
          window.__dbg?.(`input: ta.input val=${JSON.stringify(ta.value).slice(0,30)} focused=${document.activeElement === ta} inputmode=${ta.getAttribute('inputmode')} locked=${kbLocked}`);
        });
      }
    }
    term.onData(data => {
      // Filter xterm.js terminal response sequences that leak through onData.
      // These are generated when pane content contains query sequences (e.g. \x1b[6n).
      // Without filtering, they create a feedback loop: response→tmux→capture→xterm→response.
      if (/^\x1b\[[\?>=]?[\d;]*c$/.test(data)) return; // DA1/DA2/DA3
      if (/^\x1b\[\d+;\d+R$/.test(data)) return;        // DSR cursor position
      if (/^\x1b\[\d+n$/.test(data)) return;             // DSR device status
      window.__dbg?.(`input: onData len=${data.length} paste=${isPasting} data=${JSON.stringify(data).slice(0,40)}`);
      // Mobile: force-clear xterm's hidden textarea after keyboard input to prevent
      // accumulation from auto-paired quotes/brackets. Skip paste so xterm.js can
      // fully process the pasted content. Also skip while an IME composition is in
      // progress — clearing textarea.value mid-composition breaks CJK/Japanese
      // input (e.g. drops pinyin the user is currently typing).
      if (isMobile && !isPasting && !isComposing) {
        requestAnimationFrame(() => {
          if (isComposing) return; // composition may have started in the meantime
          const ta = termEl?.querySelector('.xterm-helper-textarea');
          if (ta && ta.value) ta.value = '';
        });
      }
      isPasting = false;
      enqueueKeys(data, true);
    });
    // Block xterm from processing keys when input box is open
    term.attachCustomKeyEventHandler(() => true);

    // Desktop: forward Ctrl-key combos straight to tmux.
    //
    // Problem: xterm's input sink is a real <textarea>. On macOS WKWebView
    // (and to a lesser extent other browsers) the OS-level emacs text
    // bindings — Ctrl-A/E/K/U/W/D … — are consumed by the text field
    // BEFORE the keydown reaches JS, so xterm never emits onData and
    // Ctrl-C / Ctrl-U / Ctrl-D silently do nothing in the remote shell.
    //
    // Fix: a capture-phase keydown listener converts Ctrl+<letter> (and a
    // few friends) into the corresponding C0 control byte and sends it via
    // the same queue as normal input, then preventDefault +
    // stopImmediatePropagation so neither the OS binding nor xterm's own
    // handler also acts on it. Capture phase is essential: it runs before
    // the textarea's default action.
    let onDesktopKeydown = null;
    if (!isMobile) {
      onDesktopKeydown = (e) => {
        // Only the Ctrl modifier (allow Shift for Ctrl-Shift-letter → same
        // control byte). Cmd/Alt combos are left to the browser (copy/paste,
        // word nav) — tmux doesn't use them.
        if (!e.ctrlKey || e.metaKey || e.altKey) return;
        let byte = null;
        const k = e.key;
        if (k.length === 1) {
          const code = k.toLowerCase().charCodeAt(0);
          if (code >= 97 && code <= 122) {
            byte = String.fromCharCode(code - 96); // Ctrl-A=0x01 … Ctrl-Z=0x1a
          } else if (k === ' ') {
            byte = '\x00'; // Ctrl-Space = NUL
          } else if (k === '\\') {
            byte = '\x1c';
          } else if (k === ']') {
            byte = '\x1d';
          }
        }
        if (byte == null) return; // not a combo we translate — leave it alone
        e.preventDefault();
        e.stopImmediatePropagation();
        window.__dbg?.(`kb(desktop): Ctrl-${k} → 0x${byte.charCodeAt(0).toString(16)}`);
        enqueueKeys(byte, true);
      };
      // Capture phase on the wrapper so it fires before the textarea default.
      termEl.addEventListener('keydown', onDesktopKeydown, { capture: true });
      // Desktop has no on-screen keyboard / toggle, so focus the xterm sink
      // on click so ordinary typing (and our handler) works. Auto-focus on
      // mount ONLY for the active terminal — otherwise multiple split cells
      // race for the single document focus and input lands nowhere.
      const focusTerm = () => { try { term.focus(); } catch {} };
      termEl.addEventListener('mousedown', focusTerm);
      if (active) requestAnimationFrame(focusTerm);
      onDesktopKeydown._focusTerm = focusTerm; // kept for cleanup reference
    }

    let lastContent = '';
    let lastCursor = null;
    // Uses outer endTouchScrollTimer so unlockKeyboard() and effect cleanup can clear it.
    function endTouchScroll() {
      endTouchScrollTimer = null;
      // Selection holds the pin; releasing it would clear+rewrite and wipe
      // xterm's native selection visuals. Stay pinned until the selection is
      // cleared (toolbar copy, tap outside, pane switch).
      if (selection) return;
      touchScrolling = false;
      if (lastContent && termAtBottom) writeToXterm(lastContent, lastCursor);
    }
    function scheduleEndTouchScroll(ms) {
      clearTimeout(endTouchScrollTimer);
      endTouchScrollTimer = setTimeout(endTouchScroll, ms);
    }

    // Helper: convert touch coordinates to terminal cell (col, row in viewport)
    function touchToCell(clientX, clientY) {
      const rect = termEl.getBoundingClientRect();
      const { w: cellW, h: cellH } = cellSize(term);
      return {
        col: Math.min(term.cols - 1, Math.max(0, Math.floor((clientX - rect.left) / cellW))),
        row: Math.min(term.rows - 1, Math.max(0, Math.floor((clientY - rect.top) / cellH))),
      };
    }

    // Helper: find word boundaries at buffer row + col
    function wordBoundsAt(bufRow, col) {
      const line = term.buffer.active.getLine(bufRow);
      if (!line) return { start: col, end: col + 1 };
      const text = line.translateToString(false);
      if (col >= text.length || /\s/.test(text[col])) return { start: col, end: col + 1 };
      let start = col, end = col;
      while (start > 0 && !/\s/.test(text[start - 1])) start--;
      while (end < text.length - 1 && !/\s/.test(text[end + 1])) end++;
      return { start, end: end + 1 };
    }

    // Mobile touch: scrolling, scrollbar drag, long-press word selection
    let touchId = null; // track the initial touch to ignore extra fingers
    let touchY = 0, touchStartY = 0, accumulatedDy = 0, longPressTimer = null, didScroll = false;
    let lastMoveTime = 0, momentumId = null, totalDist = 0;
    let velocitySamples = []; // recent velocity samples for smoothing
    const lineHeight = () => cellSize(term).h || (fontSize * CELL_H_RATIO);

    let onScrollbar = false, scrollbarStartY = 0, scrollbarStartViewport = 0;
    // Touch mode: 'idle' | 'down' | 'scrollbar' | 'scroll' | 'longpress-select' | 'handle-drag'
    let touchMode = 'idle';
    let dragHandle = null; // 'start' | 'end' when touchMode === 'handle-drag'
    let handleGrabDx = 0;  // finger minus dragged endpoint cell-centre at grab time
    let handleGrabDy = 0;
    let edgeScrollId = null; // rAF loop for drag-at-edge auto-scroll
    let edgeScrollDir = 0;   // -1 up / +1 down / 0 none
    let lastDragX = 0, lastDragY = 0; // latest compensated drag point (px)
    const stopMomentum = () => { if (momentumId) { cancelAnimationFrame(momentumId); momentumId = null; } };

    // Cell at touch coords, allowing 1 cell of overshoot in each direction so
    // drags out of the visible area still hit the closest edge. Caller decides
    // whether to clamp; default clamp matches the legacy touchToCell behavior.
    // Recompute pixel positions for handles + toolbar from current selection
    // and viewport. Called whenever selection, scroll, resize, or render
    // geometry changes.
    function recomputeSelUI() {
      if (!selection || !term || !termEl) { selUI = null; return; }
      const { w: cellW, h: cellH } = cellSize(term);
      if (!cellW || !cellH) { selUI = null; return; }
      const buf = term.buffer.active;
      const top = buf.viewportY;
      const rows = term.rows;
      const cols = term.cols;
      const a = selStart(selection);
      const b = selEnd(selection);
      // viewport-relative rows; null if off-screen on that side
      const aRowV = a.row - top;
      const bRowV = b.row - top;
      const startInView = aRowV >= 0 && aRowV < rows;
      const endInView = bRowV >= 0 && bRowV < rows;
      // Handle anchor points (iOS-style lollipop):
      //   start handle anchored at the TOP-LEFT corner of the start cell —
      //     a 2px bar runs DOWN through the cell's left edge, with a dot
      //     ABOVE the line.
      //   end handle anchored at the BOTTOM-RIGHT corner of the end cell —
      //     a 2px bar runs UP through the cell's right edge, with a dot
      //     BELOW the line.
      // Stems align exactly with the cell border so the handle reads as part
      // of the selection rather than floating UI.
      const startX = a.col * cellW;
      const startY = aRowV * cellH;
      const endX = (b.col + 1) * cellW;
      const endY = (bRowV + 1) * cellH;
      // Edge-of-screen dot shifts. When the selection touches column 0 the
      // start dot would sit half off-screen with `translateX(-50%)`, leaving
      // a thin 6 px target the user can't reliably grab. Same on the right
      // edge for the end dot, which additionally collides with the
      // scrollbar's 30 px touch zone. Push the dot ~7 px inward in those
      // cases — the stem stays on the cell border (visual anchor preserved)
      // but the dot is fully in the touchable area.
      const DOT_R = 6; // approximately half the dot's visual diameter
      const startAtLeftEdge = a.col === 0;
      const endAtRightEdge = b.col >= cols - 1;
      const startDotShiftX = startAtLeftEdge ? DOT_R : 0;
      const endDotShiftX = endAtRightEdge ? -DOT_R : 0;
      // Toolbar placement: must clear the start handle's dot (which sits
      // ~14px ABOVE the start row) and the end handle's dot (~14px BELOW
      // the end row). We keep an extra 8px of breathing room so the user
      // can comfortably grab the dot without the toolbar getting in the way.
      const HANDLE_DOT_CLEARANCE = 22; // dot radius + gap
      let toolbarX, toolbarY, toolbarBelow = false, toolbarVisible = true;
      const rect = termEl.getBoundingClientRect();
      const innerW = rect.width;
      if (startInView) {
        // Center between start and (if same line) end; else over start col.
        const cx = a.row === b.row
          ? ((a.col + b.col + 1) / 2) * cellW
          : (a.col * cellW + cellW * Math.min(8, cols - a.col) / 2);
        toolbarX = cx;
        // Above the start row, beyond the start dot.
        toolbarY = aRowV * cellH - HANDLE_DOT_CLEARANCE;
        if (toolbarY < 8) {
          // Not enough room above — place below the end row, beyond the end dot.
          toolbarY = (Math.min(rows - 1, bRowV) + 1) * cellH + HANDLE_DOT_CLEARANCE;
          toolbarBelow = true;
        }
      } else if (endInView) {
        const cx = (b.col + 1) * cellW - cellW;
        toolbarX = cx;
        toolbarY = (bRowV + 1) * cellH + HANDLE_DOT_CLEARANCE;
        toolbarBelow = true;
      } else {
        toolbarVisible = false;
        toolbarX = 0;
        toolbarY = 0;
      }
      // Clamp toolbar X within container with 8px padding
      toolbarX = Math.max(48, Math.min(innerW - 48, toolbarX));
      selUI = { startX, startY, endX, endY, toolbarX, toolbarY, toolbarBelow, startInView, endInView, toolbarVisible, cellH, startDotShiftX, endDotShiftX, startAtLeftEdge, endAtRightEdge };
    }

    // Drive xterm.js native selection from our selection model. xterm.select
    // takes (col, row, length) where length is across rows and assumes fixed
    // cols; we compute it inclusive-of-end.
    function applySelectionToXterm() {
      if (!term) return;
      isApplyingSelection = true;
      try {
        if (!selection) { term.clearSelection(); return; }
        const a = selStart(selection), b = selEnd(selection);
        const len = (b.row - a.row) * term.cols + (b.col - a.col + 1);
        term.select(a.col, a.row, Math.max(1, len));
      } finally {
        isApplyingSelection = false;
      }
    }

    clearSelection = () => {
      if (!selection) return;
      selection = null;
      selUI = null;
      isApplyingSelection = true;
      try { term?.clearSelection(); } finally { isApplyingSelection = false; }
      // Resume content updates (selection had pinned them).
      if (touchMode === 'idle') {
        touchScrolling = false;
        if (lastContent && termAtBottom) writeToXterm(lastContent, lastCursor);
      }
    };

    copySelection = async () => {
      if (!term?.hasSelection()) return;
      const text = term.getSelection();
      if (!text) return;
      const ok = await copyText(text);
      showToast(ok ? t('copied') : t('copyFailed'));
      clearSelection();
    };

    // ─── Selection extension helpers ────────────────────────────────────────
    function setSelectionFromWord(bufRow, col) {
      const bounds = wordBoundsAt(bufRow, col);
      const a = { row: bufRow, col: bounds.start };
      const b = { row: bufRow, col: bounds.end - 1 };
      selection = { anchor: a, head: b };
      applySelectionToXterm();
      recomputeSelUI();
      touchScrolling = true; // pin content updates while selection is live
    }
    function moveHead(bufRow, col) {
      if (!selection) return;
      selection = { anchor: selection.anchor, head: { row: bufRow, col } };
      applySelectionToXterm();
      recomputeSelUI();
    }
    // Called once at grab time: rewrite the selection so the endpoint being
    // dragged becomes `head` and the stationary one becomes `anchor`. From
    // then on every touchmove just rewrites `head` — the anchor can never
    // move, so dragging one handle past the other flips the selection's
    // direction (native behavior) instead of perturbing the far end.
    //
    // The old code addressed endpoints by geometric role ('start'/'end')
    // per-move: after a crossover the roles swap, but dragHandle still said
    // 'end', so the NEXT move rewrote the wrong endpoint and both ends
    // visibly jumped.
    function beginEndpointDrag(which) {
      if (!selection) return;
      const a = selStart(selection), b = selEnd(selection);
      selection = which === 'start'
        ? { anchor: { ...b }, head: { ...a } }
        : { anchor: { ...a }, head: { ...b } };
    }

    // Cell from clientX/clientY in buffer-row coords (row is absolute, not viewport-relative)
    function touchToBufferCell(clientX, clientY) {
      const cell = touchToCell(clientX, clientY);
      return { row: term.buffer.active.viewportY + cell.row, col: cell.col };
    }

    // Map a (compensated) drag point to a buffer cell with edge snapping,
    // and move `head` there. Snap zones make line starts/ends reachable:
    // the first/last ~60% of a cell at each horizontal edge snaps to col 0
    // / last col — matching OS text selection, where dragging past the text
    // edge selects to the line boundary even though the finger can't
    // physically center on the first/last character.
    function applyHandleDragAt(px, py) {
      const rect = termEl.getBoundingClientRect();
      const { w: cellW } = cellSize(term);
      const x = px - rect.left;
      const cell = touchToCell(px, py);
      let col = cell.col;
      const EDGE_SNAP_PX = Math.max(10, cellW * 0.6);
      if (x <= EDGE_SNAP_PX) col = 0;
      else if (x >= rect.width - SCROLLBAR_TOUCH_WIDTH - EDGE_SNAP_PX) col = term.cols - 1;
      const row = term.buffer.active.viewportY + cell.row;
      moveHead(row, col);
    }

    // Auto-scroll while dragging a handle near the top/bottom edge — the
    // native way to extend a selection beyond the visible screen. Speed
    // ramps with proximity to the edge (1 px/frame deep in the zone is
    // ~1 row per 3 frames; pressed against the edge it's ~4 rows/frame...
    // we keep it gentle: 1 row per N frames scaling to 2 rows/frame).
    const EDGE_SCROLL_ZONE_PX = 36;
    function updateEdgeScroll(clientY) {
      const rect = termEl.getBoundingClientRect();
      const topDist = clientY - rect.top;
      const botDist = rect.bottom - clientY;
      let dir = 0;
      if (topDist < EDGE_SCROLL_ZONE_PX) dir = -1;
      else if (botDist < EDGE_SCROLL_ZONE_PX) dir = 1;
      edgeScrollDir = dir;
      if (dir !== 0 && !edgeScrollId) {
        let acc = 0;
        const tick = () => {
          if (edgeScrollDir === 0 || touchMode !== 'handle-drag' || !term) {
            edgeScrollId = null;
            return;
          }
          const rect2 = termEl.getBoundingClientRect();
          const dist = edgeScrollDir < 0
            ? Math.max(0, lastDragY + handleGrabDy - rect2.top)
            : Math.max(0, rect2.bottom - (lastDragY + handleGrabDy));
          // 0 px from edge → 2 rows/frame; at zone boundary → ~0.25
          const speed = 0.25 + (1 - Math.min(1, dist / EDGE_SCROLL_ZONE_PX)) * 1.75;
          acc += speed * edgeScrollDir;
          const lines = Math.trunc(acc);
          if (lines !== 0) {
            term.scrollLines(lines);
            acc -= lines;
            // Viewport moved under the stationary finger — re-map the
            // endpoint so the selection keeps extending row by row.
            applyHandleDragAt(lastDragX, lastDragY);
          }
          edgeScrollId = requestAnimationFrame(tick);
        };
        edgeScrollId = requestAnimationFrame(tick);
      }
    }
    function stopEdgeScroll() {
      edgeScrollDir = 0;
      if (edgeScrollId) { cancelAnimationFrame(edgeScrollId); edgeScrollId = null; }
    }

    // Hit-test handles. Each handle is a lollipop (dot + stem); the visible
    // dot is 12 px but the *touchable* zone is much larger so the user
    // doesn't have to aim. Capsule axis runs along the stem, with generous
    // buffer in both directions.
    //
    // Sizing rationale: a thumb-pad on a phone is ~44 px wide at the tip
    // and lays down a roughly oval contact patch. We use 28 px half-width
    // (= 56 px wide hit zone) and extend ±22 px past the dot end of the
    // capsule, which means anywhere in a ~56 × (cellH + 44) rectangle hits.
    //
    // Conflicts handled below:
    //   - Short single-row selection (start/end in same row, close
    //     together): the two capsules overlap. We split the overlap at the
    //     midpoint between start.X and end.X so each handle owns its half.
    //   - col 0:    extend start capsule LEFT to the container edge so a
    //               miss to the left of the dot still hits.
    //   - col cols-1: extend end capsule RIGHT to the container edge,
    //               which also swallows the scrollbar's 30 px touch zone
    //               for that range. handle is tested before scrollbar in
    //               onTouchStart so the priority is right.
    function hitHandle(clientX, clientY) {
      if (!selection || !selUI || !termEl) return null;
      const rect = termEl.getBoundingClientRect();
      const px = clientX - rect.left;
      const py = clientY - rect.top;
      const HIT_HALF_W = 28;       // half the touchable width, ≈ thumb pad
      const HIT_DOT_PAD = 22;      // buffer past the dot end of the capsule
      const cellH = selUI.cellH || 16;
      const innerW = rect.width;

      // Compute X overlap-resolution boundary. If both handles are in view
      // on the same row, anything between them belongs to whichever is
      // closer (split at midpoint).
      const sameRow =
        selection.anchor.row === selection.head.row &&
        selUI.startInView && selUI.endInView;
      const midX = sameRow ? (selUI.startX + selUI.endX) / 2 : null;

      if (selUI.startInView) {
        // X bounds with edge / overlap adjustments.
        let xMin = selUI.startAtLeftEdge ? 0 : selUI.startX - HIT_HALF_W;
        let xMax = selUI.startX + HIT_HALF_W;
        if (midX !== null) xMax = Math.min(xMax, midX);
        // Y bounds: stem runs DOWN through the selection's first row; the
        // dot now sits BELOW that (mirrors the end handle), so the dot-side
        // buffer extends past startY + cellH.
        const yMin = selUI.startY - HIT_DOT_PAD * 0.5;
        const yMax = selUI.startY + cellH + HIT_DOT_PAD;
        if (px >= xMin && px <= xMax && py >= yMin && py <= yMax) return 'start';
      }
      if (selUI.endInView) {
        let xMin = selUI.endX - HIT_HALF_W;
        let xMax = selUI.endAtRightEdge ? innerW : selUI.endX + HIT_HALF_W;
        if (midX !== null) xMin = Math.max(xMin, midX);
        const yMin = selUI.endY - cellH - HIT_DOT_PAD * 0.5;
        const yMax = selUI.endY + HIT_DOT_PAD;
        if (px >= xMin && px <= xMax && py >= yMin && py <= yMax) return 'end';
      }
      return null;
    }
    // Hit-test the toolbar copy button (handled by the button's own pointer
    // events; we just need to know to skip terminal-touch handling when the
    // touch lands on the toolbar).
    function isOnToolbar(target) {
      return !!(target && target.closest && target.closest('.sel-toolbar'));
    }
    // Hit-test whether a buffer-row/col is inside the current selection
    function isInsideSelection(bufRow, col) {
      if (!selection) return false;
      const a = selStart(selection), b = selEnd(selection);
      if (bufRow < a.row || bufRow > b.row) return false;
      if (a.row === b.row) return col >= a.col && col <= b.col;
      if (bufRow === a.row) return col >= a.col;
      if (bufRow === b.row) return col <= b.col;
      return true;
    }

    const onTouchStart = (e) => {
      stopMomentum();
      touchId = e.touches[0].identifier; // track this finger
      const cx = e.touches[0].clientX;
      const cy = e.touches[0].clientY;

      // Toolbar / handle hit-tests come first — they're tiny UI surfaces and
      // the rest of the terminal-touch logic must not run for them. Toolbar
      // buttons handle their own clicks; we just bow out.
      if (isOnToolbar(e.target)) {
        touchMode = 'idle';
        return;
      }
      if (selection) {
        const which = hitHandle(cx, cy);
        if (which) {
          touchMode = 'handle-drag';
          dragHandle = which;
          // Re-anchor so the grabbed endpoint is `head` — all subsequent
          // moves rewrite head only (see beginEndpointDrag).
          beginEndpointDrag(which);
          // Record the finger's offset from the dragged endpoint's CELL
          // CENTRE in both axes, so the first touchmove maps to exactly the
          // cell the endpoint is already on — zero snap. The old code only
          // compensated Y (the end-handle dot sits at the cell's right edge,
          // so X was off by up to a full column) and then "lifted" the point
          // one row above the finger, which guaranteed a one-row jump on the
          // first frame of every drag.
          const r = termEl.getBoundingClientRect();
          const { w: cw, h: ch } = cellSize(term);
          const ep = selection.head;
          const epCenterX = r.left + (ep.col + 0.5) * cw;
          const epCenterY = r.top + (ep.row - term.buffer.active.viewportY + 0.5) * ch;
          handleGrabDx = cx - epCenterX;
          handleGrabDy = cy - epCenterY;
          // Pin content updates while dragging. preventDefault on touchmove
          // (which is non-passive) blocks the page from scrolling.
          touchScrolling = true;
          return;
        }
      }

      // Scrollbar drag (right edge)
      const rect = termEl.getBoundingClientRect();
      onScrollbar = (rect.right - cx) < SCROLLBAR_TOUCH_WIDTH;
      if (onScrollbar) {
        touchMode = 'scrollbar';
        touchScrolling = true;
        scrollbarStartY = cy;
        scrollbarStartViewport = term.buffer.active.viewportY;
        return;
      }

      touchY = cy;
      touchStartY = touchY;
      accumulatedDy = 0;
      velocitySamples = [];
      totalDist = 0;
      lastMoveTime = Date.now();
      didScroll = false;
      touchMode = 'down';
      // Selection lives on. We DO allow scrolling within a selection — the
      // selection follows buffer rows, so scrolling just moves it. We do
      // NOT, however, kick off a new long-press while a selection exists;
      // long-press inside the selection is no-op (use handle to refine),
      // long-press outside cancels and starts a new selection.
      const startCX = cx, startCY = cy;
      longPressTimer = setTimeout(() => {
        if (touchMode !== 'down' || didScroll || !term) return;
        const textarea = termEl.querySelector('.xterm-helper-textarea');
        if (textarea) textarea.blur();
        const cell = touchToCell(startCX, startCY);
        const bufRow = term.buffer.active.viewportY + cell.row;
        // If a selection exists and the long-press lands inside it, ignore
        // (avoid surprising users who are aiming at handles).
        if (selection && isInsideSelection(bufRow, cell.col)) return;
        // New selection
        if (selection) clearSelection();
        setSelectionFromWord(bufRow, cell.col);
        touchMode = 'longpress-select';
        navigator.vibrate?.(15);
      }, LONG_PRESS_MS);
    };
    // Find the tracked touch by identifier (ignore extra fingers)
    const findTouch = (list) => { for (let i = 0; i < list.length; i++) if (list[i].identifier === touchId) return list[i]; return null; };
    const onTouchMove = (e) => {
      if (!term) return;
      const t0 = findTouch(e.touches);
      if (!t0) return; // not our finger
      // Scrollbar drag: map touch delta proportionally to scroll position
      if (touchMode === 'scrollbar') {
        const deltaY = t0.clientY - scrollbarStartY;
        const trackH = termEl.clientHeight;
        const totalScroll = term.buffer.active.baseY;
        if (totalScroll > 0 && trackH > 0) {
          const scrollTarget = scrollbarStartViewport + (deltaY / trackH) * totalScroll;
          term.scrollToLine(Math.max(0, Math.min(totalScroll, Math.round(scrollTarget))));
        }
        if (e.cancelable) e.preventDefault();
        return;
      }
      // Handle drag: the grabbed endpoint is `head` (re-anchored at grab
      // time); just track the finger. Grab-offset compensation in BOTH axes
      // means the mapped cell starts exactly where the endpoint already is.
      if (touchMode === 'handle-drag' && selection) {
        lastDragX = t0.clientX - handleGrabDx;
        lastDragY = t0.clientY - handleGrabDy;
        applyHandleDragAt(lastDragX, lastDragY);
        updateEdgeScroll(t0.clientY);
        if (e.cancelable) e.preventDefault();
        return;
      }
      // Long-press selection: extend from anchor word to current cell
      if (touchMode === 'longpress-select' && selection) {
        const { row, col } = touchToBufferCell(t0.clientX, t0.clientY);
        moveHead(row, col);
        if (e.cancelable) e.preventDefault();
        return;
      }
      // Normal content scroll
      const now = Date.now();
      const y = t0.clientY;
      const dy = touchY - y;
      const dt = Math.max(1, now - lastMoveTime);
      touchY = y;
      lastMoveTime = now;
      accumulatedDy += dy;
      totalDist += Math.abs(dy);
      const lh = lineHeight();
      // Track velocity in px/ms, keep last 5 samples within 100ms
      velocitySamples.push({ v: dy / dt, t: now });
      while (velocitySamples.length > 5 || (velocitySamples.length > 1 && now - velocitySamples[0].t > 100)) {
        velocitySamples.shift();
      }
      const lines = Math.trunc(accumulatedDy / lh);
      if (lines !== 0) {
        didScroll = true;
        if (longPressTimer) { clearTimeout(longPressTimer); longPressTimer = null; }
        if (touchMode === 'down') touchMode = 'scroll';
        touchScrolling = true;
        term.scrollLines(lines);
        accumulatedDy -= lines * lh;
        if (e.cancelable) e.preventDefault();
      }
    };
    const onTouchEnd = (e) => {
      const endedMode = touchMode;
      if (longPressTimer) { clearTimeout(longPressTimer); longPressTimer = null; }
      if (endedMode === 'scrollbar') {
        touchMode = 'idle';
        onScrollbar = false;
        scheduleEndTouchScroll(TOUCH_END_DELAY_MS);
        return;
      }
      if (endedMode === 'handle-drag') {
        touchMode = 'idle';
        dragHandle = null;
        stopEdgeScroll();
        // Selection persists; pinning persists
        return;
      }
      if (endedMode === 'longpress-select') {
        touchMode = 'idle';
        // Selection persists with current head; pinning persists
        return;
      }
      // 'down' (clean tap) or 'scroll' (released after scroll)
      if (endedMode === 'down') {
        // Clean tap. If a selection exists and the tap was outside it
        // (and not on a handle/toolbar — those bailed at touchstart),
        // cancel the selection. Otherwise no-op (keyboard never opens via
        // terminal tap; the toolbar/copy is the only commit action).
        if (selection) {
          const t0 = e.changedTouches?.[0];
          if (t0) {
            const { row, col } = touchToBufferCell(t0.clientX, t0.clientY);
            if (!isInsideSelection(row, col)) clearSelection();
          }
        }
        touchMode = 'idle';
        return;
      }
      // 'scroll' or anything that left touchScrolling=true
      touchMode = 'idle';
      if (touchScrolling && velocitySamples.length > 0) {
        // Weighted average of recent velocity samples (newer = heavier)
        let wSum = 0, wTotal = 0;
        for (let i = 0; i < velocitySamples.length; i++) {
          const w = i + 1;
          wSum += velocitySamples[i].v * w;
          wTotal += w;
        }
        const avgVelocity = wSum / wTotal; // px/ms
        const lh = lineHeight();
        // Cap velocity at 120px/frame equivalent, then convert to lines/frame
        const maxPxPerFrame = MOMENTUM_MAX_PX;
        const cappedPx = Math.max(-maxPxPerFrame, Math.min(maxPxPerFrame, avgVelocity * 16));
        let v = cappedPx / lh;
        if (Math.abs(v) > 0.1) {
          let acc = 0;
          const friction = MOMENTUM_FRICTION;
          const coast = () => {
            v *= friction;
            acc += v;
            const lines = Math.trunc(acc);
            if (lines !== 0) {
              term.scrollLines(lines);
              acc -= lines;
            }
            if (Math.abs(v) > MOMENTUM_MIN_V) {
              momentumId = requestAnimationFrame(coast);
            } else {
              momentumId = null;
              scheduleEndTouchScroll(200);
            }
          };
          momentumId = requestAnimationFrame(coast);
        } else {
          scheduleEndTouchScroll(TOUCH_END_DELAY_MS);
        }
      } else if (touchScrolling && !selection) {
        scheduleEndTouchScroll(TOUCH_END_DELAY_MS);
      }
    };
    const onTouchCancel = () => {
      if (longPressTimer) { clearTimeout(longPressTimer); longPressTimer = null; }
      onScrollbar = false;
      touchMode = 'idle';
      dragHandle = null;
      stopMomentum();
      stopEdgeScroll();
      // Don't blow away the selection on a stray cancel — but if we were
      // mid-handle-drag the user expects the partial drag to commit, which
      // it already has via moveHead() on the last touchmove.
      if (!selection) scheduleEndTouchScroll(100);
    };
    termEl.addEventListener('touchstart', onTouchStart, { passive: true });
    termEl.addEventListener('touchmove', onTouchMove, { passive: false });
    termEl.addEventListener('touchend', onTouchEnd, { passive: true });
    termEl.addEventListener('touchcancel', onTouchCancel, { passive: true });

    // Adopt selections that originate outside our touch flow — double-tap,
    // triple-tap (xterm.js handles those internally), keyboard Cmd+A on
    // desktop, mouse drag. Skip the events we triggered ourselves
    // (applySelectionToXterm sets isApplyingSelection=true).
    const onSelChange = term.onSelectionChange(() => {
      if (isApplyingSelection) return;
      if (!term.hasSelection()) {
        // Native cleared (e.g., user clicked outside on desktop). Drop our
        // model too. clearSelection() guards against re-clearing xterm.
        if (selection) {
          selection = null;
          selUI = null;
          if (touchMode === 'idle') {
            touchScrolling = false;
            if (lastContent && termAtBottom) writeToXterm(lastContent, lastCursor);
          }
        }
        return;
      }
      const pos = term.getSelectionPosition();
      if (!pos) return;
      // xterm's pos.end.x is exclusive (one past the last selected cell).
      // Convert to our inclusive model. If end.x === 0 the selection ends at
      // the start of a row, which means "include up to the previous row's
      // last cell"; clamp to col 0 anyway — visual difference is < 1 cell.
      const sRow = pos.start.y, sCol = pos.start.x;
      const eRow = pos.end.y;
      const eCol = Math.max(0, pos.end.x - 1);
      selection = { anchor: { row: sRow, col: sCol }, head: { row: eRow, col: eCol } };
      recomputeSelUI();
      touchScrolling = true; // pin while selection is live
    });
    // Safety net: if the app is backgrounded mid-selection or mid-scroll, touchcancel
    // may never fire and touchScrolling can stay stuck true, which freezes
    // writeToXterm. Reset transient gesture state whenever we regain visibility.
    const onVisible = () => {
      if (document.visibilityState !== 'visible') return;
      touchScrolling = false;
      onScrollbar = false;
      touchMode = 'idle';
      dragHandle = null;
      // Drop the selection — re-attaching to a clipboard from before
      // backgrounding is rarely useful and could surprise the user.
      if (selection) clearSelection();
      if (lastContent && termAtBottom) writeToXterm(lastContent, lastCursor);
    };
    document.addEventListener('visibilitychange', onVisible);

    // Mobile keyboard: opened only via the keyboard toggle button.
    // Tapping the terminal does NOT open the keyboard — users found stray taps
    // while reading scrollback (or near the selection handles) surprising.
    // Single layer: kbLocked flag, enforced by onTaFocus (it blurs whenever
    // a focus lands while locked). inputmode is pinned to "text" — see init
    // for the InputMethodManager bug that motivated removing the toggle.
    let onTaBlur, onTaFocus;

    if (kbTa) {
      onTaBlur = () => {
        clearTimeout(kbBlurTimer);
        kbBlurTimer = setTimeout(() => {
          if (Date.now() < unlockUntil && unlockRetries < UNLOCK_RETRY_MAX) {
            unlockRetries++;
            window.__dbg?.(`kb: blur timer skipped (grace retry ${unlockRetries}/${UNLOCK_RETRY_MAX})`);
            // Retry focus within grace window — the blur was likely system-initiated
            // (e.g., Android pad where IME didn't come up yet).
            if (kbTa && !kbLocked && document.activeElement !== kbTa) kbTa.focus();
            return;
          }
          kbLocked = true;
          window.__dbg?.('kb: blur timer → lock');
        }, 150);
        window.__dbg?.('kb: textarea blur (timer scheduled)');
      };
      kbTa.addEventListener('blur', onTaBlur);

      onTaFocus = () => {
        clearTimeout(kbBlurTimer);
        if (kbLocked) {
          window.__dbg?.('kb: textarea focus while LOCKED → blur!');
          kbTa.blur();
          return;
        }
        window.__dbg?.('kb: textarea focus (allowed)');
      };
      kbTa.addEventListener('focus', onTaFocus);
    }

    term.onScroll(() => {
      const buf = term.buffer.active;
      const wasAtBottom = termAtBottom;
      termAtBottom = buf.viewportY >= buf.baseY;
      // Returning to bottom → flush the latest snapshot we deferred while scrolled up
      if (!wasAtBottom && termAtBottom && lastContent && !touchScrolling) {
        writeToXterm(lastContent, lastCursor);
      }
      if (termAtBottom) hasNewContent = false;
      // Selection lives in buffer-row space; viewport scroll moves the
      // pixel-space handles. Recompute on every scroll tick.
      if (selection) recomputeSelUI();
    });

    // Resize tmux pane to fit screen.
    //
    // First principle: the terminal's (cols, rows) must always equal
    //   floor(termEl.clientWidth / cellW) × floor(termEl.clientHeight / cellH).
    // Everything else — keyboard open/close, orientation change, window
    // resize, flex reflow, safe-area shifts — is just a cause of container
    // size change. The only thing we actually need to observe is
    // termEl's box. ResizeObserver does exactly that, including cases the
    // old code relied on intermediate custom events for.
    //
    // Secondary detail: xterm computes real cell dimensions asynchronously
    // (after first render). Until then calcFit falls back to a font-size
    // estimate that can be off by 1-2 rows, which is why initial paint
    // sometimes left the bottom rows blank. We run one more fit on
    // term.onRender's first fire so the first real fit uses real metrics.
    function doResize() {
      const fit = calcFit();
      if (!fit) return;
      window.__dbg?.(`resize: fit=${fit.cols}x${fit.rows} cur=${term.cols}x${term.rows} elH=${termEl.clientHeight}`);
      if (fit.cols === term.cols && fit.rows === term.rows) {
        // Same dims but cell metrics may have changed (font size); refresh
        // selection UI either way.
        if (selection) recomputeSelUI();
        return;
      }
      pendingCols = fit.cols;
      pendingRows = fit.rows;
      pendingResizeTs = Date.now();
      resizePane(target, fit.cols, fit.rows).catch(() => {});
      term.resize(fit.cols, fit.rows);
      // Resize wipes xterm's buffer-row mapping — re-anchor our selection
      // before the rewrite so the visuals stay consistent.
      if (selection) {
        applySelectionToXterm();
        recomputeSelUI();
      }
      // Immediately rewrite content so display is clean during the ~200ms server catch-up
      if (lastContent) writeToXterm(lastContent, lastCursor);
    }
    doResizeRef = doResize;

    // Single observer for all container-size changes.
    const resizeObs = new ResizeObserver(() => doResize());
    resizeObs.observe(termEl);

    // First real paint → real cell metrics available → refit once so the
    // initial ResizeObserver tick (which ran with estimated metrics) gets
    // corrected. term.onRender fires on every render, so we disarm after
    // the first call.
    let firstRenderDone = false;
    const onFirstRender = term.onRender(() => {
      if (firstRenderDone) return;
      firstRenderDone = true;
      doResize();
    });

    // Re-measure once the bundled web fonts finish loading.
    //
    // xterm measures the monospace cell WIDTH at open() time. With the fonts
    // bundled as async woff2 (not installed system-wide), that first
    // measurement runs against the system fallback font, whose advance width
    // differs from Maple Mono. When Maple Mono then swaps in, xterm keeps the
    // stale (fallback) cell width — so on devices whose fallback is narrower
    // than Maple Mono (observed on some MIUI WebViews) the real glyphs are
    // wider than their cell and visually collide ("characters stuck together,
    // no gaps"); on devices whose fallback happens to match (vivo) it looked
    // fine. document.fonts.ready resolves after all @font-face loads settle;
    // we then clear xterm's cached glyph atlas + char-dimension cache and
    // refit so the cell geometry matches the actual font.
    let fontReadyHandled = false;
    const remeasureAfterFonts = () => {
      if (fontReadyHandled || !term) return;
      fontReadyHandled = true;
      try {
        // Force xterm to drop cached cell metrics + glyph atlas and recompute
        // against the now-loaded font. clearTextureAtlas exists on the render
        // service across the WebGL/canvas renderers; guard in case it doesn't.
        term._core?._renderService?.clearTextureAtlas?.();
        term._core?._charSizeService?.measure?.();
      } catch {}
      // Recompute cols/rows for the corrected cell size, then repaint.
      doResize();
      if (lastContent) writeToXterm(lastContent, lastCursor);
      term.refresh(0, term.rows - 1);
    };
    if (document.fonts?.ready) {
      document.fonts.ready.then(remeasureAfterFonts).catch(() => {});
      // Belt-and-suspenders: also fire when the specific family reports loaded,
      // in case `ready` resolved earlier against the fallback.
      document.fonts.load?.('14px "Maple Mono"').then(remeasureAfterFonts).catch(() => {});
    }

    // Keyboard state (lock/unlock) is driven by keyboard-shift events.
    // Resize is NOT — ResizeObserver handles any container change caused by
    // the keyboard. We keep this handler purely for the kbLocked lifecycle.
    let lastKbHeight = 0;
    const onKbShift = (e) => {
      if (!termEl || !term) return;
      termEl.style.marginTop = '0'; // remove legacy shift
      const kbH = e.detail?.kbHeight ?? 0;
      // When keyboard closes (was open, now 0), lock + blur to prevent accidental re-open
      // from shortcut buttons. Key scenario: user taps terminal (unlock), keyboard opens,
      // then keyboard dismissed by Android back/system — textarea stays focused but keyboard
      // is gone. Without this, any subsequent touch (shortcut button) causes IME to re-show.
      // Guard: only trigger on the open→close transition. A bare kbH=0 event (e.g., Android
      // pad where IME never actually rose) must NOT re-lock — that would kill the keyboard
      // toggle the user just pressed.
      if (kbTa && kbH === 0 && lastKbHeight > 0 && Date.now() >= unlockUntil) {
        kbLocked = true;
        if (document.activeElement === kbTa) kbTa.blur();
        window.__dbg?.('kb: keyboard-shift kbH=0 (was ' + lastKbHeight + ') → lock + blur');
      }
      lastKbHeight = kbH;
      // Keep the cursor area visible when the keyboard just appeared.
      // The actual resize has already been (or will be) picked up by
      // ResizeObserver; we just nudge scroll on the next frame so the
      // scroll is applied to the post-resize geometry.
      if (kbH > 0 && termAtBottom && term) {
        requestAnimationFrame(() => term?.scrollToBottom());
      }
    };
    window.addEventListener('keyboard-shift', onKbShift);

    // Reconnect recovery: the previous server's resize_tracker cleanup auto-fits the pane
    // back to an arbitrary size on disconnect. Clear stale pending confirmation and
    // re-send resize so the new server's tmux pane matches our terminal again.
    const onReconnected = () => {
      pendingCols = 0; pendingRows = 0; pendingResizeTs = 0;
      // Force doResize to actually send by invalidating the cur===fit check.
      // We do this by momentarily pretending term has different dims.
      if (term) {
        const fit = calcFit();
        if (fit) {
          pendingCols = fit.cols;
          pendingRows = fit.rows;
          pendingResizeTs = Date.now();
          resizePane(target, fit.cols, fit.rows).catch(() => {});
          // term.resize is a no-op if dims already match, which is fine.
          term.resize(fit.cols, fit.rows);
          if (lastContent) writeToXterm(lastContent, lastCursor);
        }
      }
      // Pull a fresh snapshot immediately rather than waiting up to 200 ms
      // for the server's subscribe loop to push the first frame. Otherwise
      // the user sees stale content right after the reconnect dialog
      // disappears, which feels broken on flaky networks.
      capturePane(target).then(r => {
        const c = r.output || r.content;
        if (c && c !== lastContent) {
          lastContent = c;
          paneContent = c;
          if (termAtBottom) writeToXterm(c, lastCursor);
          else hasNewContent = true;
        }
      }).catch(() => {});
    };
    window.addEventListener('ws-reconnected', onReconnected);

    // Named refs so cleanup removes EXACTLY this cell's listener — two cells
    // on the same target each register their own; removing by reference
    // leaves the other's intact.
    const onPaneOutputCb = (t, content, cursor, currentCommand) => {
      if (t !== target) return;
      if (cursor) lastCursor = cursor;
      // Pane's running command, only present on first push and on changes.
      // Drives the chat-parser detection (kiro / claude code / gemini).
      if (currentCommand !== undefined) {
        command = currentCommand;
      }
      if (content != null && content !== lastContent) {
        lastContent = content;
        paneContent = content;
        // Defer rendering while user is reading scrollback; flush on scroll-to-bottom
        if (termAtBottom) writeToXterm(content, lastCursor);
        else hasNewContent = true;
      } else if (cursor && term && lastContent && termAtBottom) {
        // Cursor-only update — share layout calc with writeToXterm so topPad offset matches
        const { row } = computeCursorLayout(lastContent, cursor, term.rows);
        if (row > 0 && row <= term.rows) {
          term.write(`\x1b[${row};${cursor.x + 1}H`);
        }
      }
    };
    const onPaneClosedCb = (t) => { if (t === target) onPaneExit(target); };
    addPaneOutputListener(target, onPaneOutputCb);
    addPaneClosedListener(target, onPaneClosedCb);

    subscribe(target);
    capturePane(target).then(r => {
      if (lastContent) return; // subscription already delivered content
      const c = r.output || r.content;
      if (c) {
        paneContent = c;
        lastContent = c;
        writeToXterm(c, lastCursor);
      }
    }).catch(() => {});

    return () => {
      resizeObs.disconnect();
      try { onFirstRender.dispose(); } catch {}
      try { onSelChange.dispose(); } catch {}
      doResizeRef = null;
      clearTimeout(endTouchScrollTimer);
      if (longPressTimer) clearTimeout(longPressTimer);
      clearTimeout(kbBlurTimer);
      if (kbTa && onTaBlur) kbTa.removeEventListener('blur', onTaBlur);
      if (kbTa && onTaFocus) kbTa.removeEventListener('focus', onTaFocus);
      stopMomentum();
      stopEdgeScroll();
      if (_pendingRaf) { cancelAnimationFrame(_pendingRaf); _pendingRaf = 0; }
      _pendingContent = null;
      _pendingCursor = null;
      window.removeEventListener('ws-reconnected', onReconnected);
      window.removeEventListener('keyboard-shift', onKbShift);
      document.removeEventListener('visibilitychange', onVisible);
      termEl.removeEventListener('touchstart', onTouchStart);
      termEl.removeEventListener('touchmove', onTouchMove);
      termEl.removeEventListener('touchend', onTouchEnd);
      termEl.removeEventListener('touchcancel', onTouchCancel);
      if (onDesktopKeydown) {
        termEl.removeEventListener('keydown', onDesktopKeydown, { capture: true });
        if (onDesktopKeydown._focusTerm) termEl.removeEventListener('mousedown', onDesktopKeydown._focusTerm);
      }
      // Server's resize_tracker auto-restores this window via `resize-window -A` on WS disconnect
      unsubscribe(target);
      removePaneOutputListener(target, onPaneOutputCb);
      removePaneClosedListener(target, onPaneClosedCb);
      try { term.dispose(); } catch {}
      term = null;
      copySelection = () => {};
      clearSelection = () => {};
    };
  });

  // Re-sync size when switching to terminal tab
  $effect(() => {
    if (viewMode === 'terminal' && term) {
      requestAnimationFrame(() => term.refresh(0, term.rows - 1));
    }
  });

  async function handleSubmit() {
    if (viewMode === 'chat') {
      if (!input.trim()) return;
      try {
        await sendCommand(target, input);
        noteSendSuccess();
        input = '';
        document.querySelectorAll('.input-bar textarea').forEach(ta => ta.style.height = 'auto');
      } catch (e) {
        noteSendFailure('chat send');
      }
      return;
    }
    // Terminal mode
    if (!input.trim()) {
      try { await sendKeys(target, 'Enter', false); noteSendSuccess(); }
      catch (e) { noteSendFailure('Enter'); }
      document.activeElement?.blur();
      return;
    }
    try {
      await sendKeys(target, input, true);
      noteSendSuccess();
      input = '';
      document.querySelectorAll('.input-bar textarea').forEach(ta => ta.style.height = 'auto');
    } catch (e) {
      noteSendFailure('submit');
    }
  }

  async function handleKeydown(e) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing && e.keyCode !== 229) {
      e.preventDefault();
      await handleSubmit();
    }
  }

  function autoResize(e) {
    const el = e.target;
    requestAnimationFrame(() => {
      el.style.height = 'auto';
      el.style.height = Math.min(el.scrollHeight, 120) + 'px';
    });
  }

  // Track consecutive send failures so the user gets a visible "unstable" toast
  // well before the heartbeat detector fires its 10-15s disconnect. One stray
  // failure is ignored (could be a single dropped packet). Two in a row on any
  // channel (shortcut, Enter, or raw key-through) surfaces a toast.
  let sendFailCount = 0;
  function noteSendFailure(label) {
    sendFailCount++;
    window.__dbg?.(`send fail (${label}) #${sendFailCount}`);
    if (sendFailCount === 2) showToast(t('connectionUnstable'));
  }
  function noteSendSuccess() {
    if (sendFailCount > 0) sendFailCount = 0;
  }

  // ─── Keystroke send queue ────────────────────────────────────────────────
  // One RPC per keystroke melts down on slow links: fast typing or the 80 ms
  // long-press repeat stacks dozens of in-flight send_keys, each competing
  // with pane snapshots for the link. Instead, only one send_keys is in
  // flight at a time; keys pressed meanwhile queue up, and consecutive
  // LITERAL chars merge into a single string (tmux send-keys -l applies it
  // as one write). Special keys can't merge (each is a distinct key name)
  // but still serialize through the queue so ordering with typed chars is
  // preserved.
  const KEY_QUEUE_MAX = 64;
  let keyQueue = [];
  let keySending = false;

  function enqueueKeys(keys, literal) {
    const last = keyQueue[keyQueue.length - 1];
    if (literal && last?.literal) {
      last.keys += keys;
    } else if (keyQueue.length >= KEY_QUEUE_MAX) {
      // Saturated (long-press repeat on a dead-slow link). Drop the newest —
      // dropping anything earlier would reorder the user's input.
      window.__dbg?.('input: key queue full — dropping key');
      return;
    } else {
      keyQueue.push({ keys, literal });
    }
    pumpKeyQueue();
  }

  async function pumpKeyQueue() {
    if (keySending) return;
    keySending = true;
    while (keyQueue.length > 0) {
      const item = keyQueue.shift();
      try {
        await sendKeys(target, item.keys, item.literal);
        noteSendSuccess();
      } catch (e) {
        window.__dbg?.(`input: sendKeys FAILED: ${e.message}`);
        noteSendFailure('key');
        // Drop everything queued behind the failure — replaying seconds-old
        // keystrokes after a reconnect is worse than losing them.
        keyQueue = [];
      }
    }
    keySending = false;
  }

  function sendSpecial(key) {
    enqueueKeys(key, false);
  }

  // Long-press repeat for shortcut keys
  let repeatTimer = null;
  let repeatInterval = null;

  function startRepeat(key) {
    const ta = termEl?.querySelector('.xterm-helper-textarea');
    window.__dbg?.(`kb: shortcut "${key}" locked=${kbLocked} inputmode=${ta?.getAttribute('inputmode')} focused=${document.activeElement === ta}`);
    navigator.vibrate?.(8); // haptic tick on press; silent during repeat interval
    sendSpecial(key);
    repeatTimer = setTimeout(() => {
      repeatInterval = setInterval(() => sendSpecial(key), 80);
    }, 400);
  }
  function stopRepeat() {
    clearTimeout(repeatTimer);
    clearInterval(repeatInterval);
    repeatTimer = null;
    repeatInterval = null;
  }

  // Svelte 5 registers touchstart as passive, so e.preventDefault() is ignored.
  // We need non-passive touchstart to prevent keyboard popup on shortcut buttons.
  function nonPassiveShortcuts(node) {
    let activeBtn = null;
    const onStart = (e) => {
      // preventDefault on ALL buttons (including kb-toggle) to prevent synthetic
      // mousedown from stealing focus away from xterm's textarea after ta.focus().
      const btn = e.target.closest('button');
      if (btn && node.contains(btn)) {
        e.preventDefault();
        btn.classList.add('pressed');
        activeBtn = btn;
      }
    };
    const onEnd = () => {
      if (activeBtn) { activeBtn.classList.remove('pressed'); activeBtn = null; }
    };
    node.addEventListener('touchstart', onStart, { passive: false });
    node.addEventListener('touchend', onEnd, { passive: true });
    node.addEventListener('touchcancel', onEnd, { passive: true });
    return { destroy() {
      node.removeEventListener('touchstart', onStart);
      node.removeEventListener('touchend', onEnd);
      node.removeEventListener('touchcancel', onEnd);
    }};
  }


</script>

<div class="terminal">
  {#if toastMsg}
    <div class="toast">{toastMsg}</div>
  {/if}
  {#if showSwitcher}
    {#if showWindowCmd || embedded}
      <!--
        Expanded switcher: a top-of-page horizontal tab bar that holds
        BOTH the current session's windows and up-to-5 other AI sessions
        as chips. One concept: "everywhere I can switch to".
      -->
      <div class="win-bar">
        <!-- Session name as a fixed tag at the far left of the switcher row,
             so it's always visible without stealing a whole row. The window
             chips scroll independently to its right. -->
        <button class="win-session" title={session} onclick={(e) => { e.stopPropagation(); showPanePicker = !showPanePicker; }}>
          <span class="win-session-name">{session}</span>
          <Icon name="chevron-down" size={9} />
        </button>
        {#if showPanePicker}
          <PanePicker
            currentTarget={target}
            onPick={(p) => {
              showPanePicker = false;
              if (`${p.session}:${p.window}.${p.pane}` !== target && onSwitchPane) {
                document.activeElement?.blur();
                touchScrolling = false;
                const fh = window.__fullHeight?.() || window.innerHeight;
                document.documentElement.style.setProperty('--app-height', fh + 'px');
                document.documentElement.classList.remove('keyboard-open');
                onSwitchPane(`${p.session}:${p.window}.${p.pane}`, p.current_command);
              }
            }}
            onClose={() => showPanePicker = false}
          />
        {/if}
        <div class="win-bar-scroll">
          {#each windows as w}
            {@const wAgent = paneAgent(w)}
            <AgentChip
              agent={wAgent}
              label={wAgent ? '' : (w.current_command || w.window_name)}
              variant={String(w.window) === currentWindow ? 'active' : 'default'}
              title={w.current_command || w.window_name}
              onclick={(e) => {
                e.stopPropagation();
                if (String(w.window) !== currentWindow && onSwitchPane) {
                  document.activeElement?.blur();
                  touchScrolling = false;
                  const fh = window.__fullHeight?.() || window.innerHeight;
                  document.documentElement.style.setProperty('--app-height', fh + 'px');
                  document.documentElement.classList.remove('keyboard-open');
                  onSwitchPane(`${w.session}:${w.window}.${w.pane}`, w.current_command);
                }
              }}
            />
          {/each}

          <AgentChip
            variant="add"
            iconName="plus"
            title="New window"
            onclick={async (e) => {
              e.stopPropagation();
              try {
                await newWindow(session);
                const ps = await listPanes(session);
                windowPanes = ps;
                const p = ps[ps.length - 1];
                if (p && onSwitchPane) {
                  document.activeElement?.blur();
                  touchScrolling = false;
                  const fh = window.__fullHeight?.() || window.innerHeight;
                  document.documentElement.style.setProperty('--app-height', fh + 'px');
                  document.documentElement.classList.remove('keyboard-open');
                  onSwitchPane(`${p.session}:${p.window}.${p.pane}`, p.current_command);
                }
              } catch {}
            }}
          />

          {#if otherAgentSessions.length > 0}
            <span class="win-sep" aria-hidden="true"></span>
            {#each otherAgentSessions as o}
              <AgentChip
                agent={o.agent}
                label={o.name}
                title={`${o.name}  (${o.agent.tag})`}
                onclick={(e) => {
                  e.stopPropagation();
                  if (onSwitchPane) {
                    document.activeElement?.blur();
                    touchScrolling = false;
                    const fh = window.__fullHeight?.() || window.innerHeight;
                    document.documentElement.style.setProperty('--app-height', fh + 'px');
                    document.documentElement.classList.remove('keyboard-open');
                    onSwitchPane(`${o.pane.session}:${o.pane.window}.${o.pane.pane}`, o.pane.current_command);
                  }
                }}
              />
            {/each}
          {/if}
        </div>

        {#if embedded && onClose}
          <!-- Split cell: close button instead of the collapse chevron
               (collapsing makes no sense — the bar IS the cell header). -->
          <button class="win-bar-collapse" aria-label="Close pane" onclick={(e) => { e.stopPropagation(); onClose(); }}>
            <Icon name="x" size={12} />
          </button>
        {:else}
          <button class="win-bar-collapse" aria-label="Collapse" onclick={() => { showWindowCmd = false; localStorage.setItem('tmux_winswitcher', '0'); }}>
            <Icon name="chevron-right" size={12} />
          </button>
        {/if}
      </div>
    {:else}
      {@const cur = windows.find(w => String(w.window) === currentWindow)}
      {@const curAgent = currentWinAgent}
      <!--
        Collapsed state: a single chip in the top-right corner using the
        exact chip visual language from the expanded bar. Conceptually the
        switcher hasn't become "something else" — it has been compressed
        to the right end of the bar.
      -->
      <div class="win-collapsed-anchor">
        <AgentChip
          agent={curAgent}
          label={curAgent ? '' : (cur?.current_command || cur?.window_name || '?')}
          chevron="left"
          onclick={() => { showWindowCmd = true; localStorage.setItem('tmux_winswitcher', '1'); }}
        />
      </div>
    {/if}
  {/if}

  <div class="term-wrap" class:hidden={viewMode !== 'terminal'}>
    <div class="xterm-wrap" bind:this={termEl}></div>
    {#if isMobile && selection && selUI}
      {#if selUI.startInView}
        <div class="sel-handle sel-handle-start" style="left: {selUI.startX}px; top: {selUI.startY}px; --cell-h: {selUI.cellH}px; --dot-shift-x: {selUI.startDotShiftX}px;" aria-hidden="true"></div>
      {/if}
      {#if selUI.endInView}
        <div class="sel-handle sel-handle-end" style="left: {selUI.endX}px; top: {selUI.endY}px; --cell-h: {selUI.cellH}px; --dot-shift-x: {selUI.endDotShiftX}px;" aria-hidden="true"></div>
      {/if}
      {#if selUI.toolbarVisible}
        <div class="sel-toolbar" class:below={selUI.toolbarBelow} style="left: {selUI.toolbarX}px; top: {selUI.toolbarY}px;">
          <button class="sel-toolbar-btn" onpointerdown={(e) => { e.stopPropagation(); e.preventDefault(); copySelection(); }}>{t('copy')}</button>
        </div>
      {/if}
    {/if}
    {#if !termAtBottom}
      <button class="scroll-btn" class:has-new={hasNewContent} onclick={() => term?.scrollToBottom()} aria-label={hasNewContent ? t('newOutput') : t('scrollToBottom')}>
        <Icon name="arrow-down" size={16} />
        {#if hasNewContent}<span class="new-dot"></span>{/if}
      </button>
    {/if}
  </div>
  {#if viewMode === 'chat'}
    <ChatView content={paneContent} {command} {fontSize} onSendKeys={(keys) => sendKeys(target, keys, false)} />
  {/if}

  <div class="input-area">
    {#if viewMode === 'terminal' && isMobile}
      <div class="input-bar">
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="shortcut-rows" use:nonPassiveShortcuts ontouchend={stopRepeat} ontouchcancel={stopRepeat} oncontextmenu={(e) => e.preventDefault()} onmouseup={stopRepeat}>
          <div class="shortcuts">
            <button tabindex="-1" ontouchstart={() => startRepeat('Escape')}>Esc</button>
            <button tabindex="-1" ontouchstart={() => startRepeat('C-d')}>^D</button>
            <button tabindex="-1" ontouchstart={() => startRepeat('C-a')}><Icon name="skip-left" size={13} /></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('Up')}><Icon name="arrow-up" size={13} /></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('C-e')}><Icon name="skip-right" size={13} /></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('BSpace')}><Icon name="delete" size={13} /></button>
          </div>
          <div class="shortcuts">
            <button tabindex="-1" ontouchstart={() => startRepeat('Tab')}>Tab</button>
            <button tabindex="-1" ontouchstart={() => startRepeat('C-c')}>^C</button>
            <button tabindex="-1" ontouchstart={() => startRepeat('Left')}><Icon name="arrow-left" size={13} /></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('Down')}><Icon name="arrow-down" size={13} /></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('Right')}><Icon name="arrow-right" size={13} /></button>
            <button class="kb-toggle" tabindex="-1" onpointerdown={(e) => {
              // Stop the touch from bubbling into terminal-touch handlers.
              // Note: we deliberately do NOT call e.preventDefault() here.
              // On Chrome Android, preventDefault on a pointerdown that
              // ends up driving focus() can consume the user-activation
              // token, leaving the IME refusing to honour showSoftInput.
              // Focus stealing is already prevented by tabindex="-1".
              e.stopPropagation();
              e.stopImmediatePropagation();
              const ta = kbTa;
              if (!ta) return;
              // Decide open/close from the REAL IME visibility, not from
              // our internal kbLocked flag. Two states could disagree:
              //   - User dismisses IME via the system keyboard's close
              //     button → IME hidden, but the textarea remains focused
              //     and our kbLocked stays false (no blur was issued).
              //     Reading kbLocked here would route us through the
              //     "close" branch, requiring a second tap to actually
              //     re-open. visualViewport is the source of truth.
              const kbOpen = document.documentElement.classList.contains('keyboard-open');
              if (!kbOpen) {
                window.__dbg?.('kb: toggle → open');
                unlockKeyboard();
              } else {
                window.__dbg?.('kb: toggle → close');
                // Cancel any pending unlock-grace retries so the blur
                // timer doesn't bounce focus back. See 73957f5.
                unlockUntil = 0;
                unlockRetries = 0;
                kbLocked = true;
                ta.blur();
              }
            }}><Icon name="keyboard" size={13} /></button>
          </div>
        </div>
      </div>
    {:else if viewMode === 'terminal'}
      <!-- Desktop: no input bar, keyboard goes directly to xterm.js -->
    {:else}
      <div class="input-bar chat-input-bar">
        <div class="input-status">
          <span class="status-left">{target}{#if command} · <span class:kiro={/^kiro/i.test(command)}>{command}</span>{/if}</span>
          {#if statusInfo?.pct != null}
            <span class="status-pct">
              <span class="pct-bar"><span class="pct-fill pct-{statusInfo.pct < 50 ? 'ok' : statusInfo.pct < 80 ? 'warn' : 'danger'}" style="width:{statusInfo.pct}%"></span></span>
              <span class="pct-{statusInfo.pct < 50 ? 'ok' : statusInfo.pct < 80 ? 'warn' : 'danger'}">{statusInfo.pct}%</span>
            </span>
          {/if}
        </div>
        <div class="cmd-row">
          {#if !waitingForInput}
            <button class="stop-btn" onclick={() => sendSpecial('C-c')} aria-label="Interrupt"><Icon name="stop" size={12} /></button>
          {/if}
          <textarea
            bind:value={input}
            onkeydown={handleKeydown}
            oninput={autoResize}
            placeholder={t('message')}
            autocapitalize="off"
            autocomplete="off"
            autocorrect="off"
            spellcheck="false"
            rows="1"
          ></textarea>
          <button class="send" onclick={handleSubmit}><Icon name="send" size={14} /></button>
        </div>
      </div>
    {/if}
  </div>
</div>

<style>
  .terminal {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    background: var(--bg);
    position: relative;
  }

  /* Collapsed: single chip floating top-right. The chip itself is an
     <AgentChip>, so its size/color come from that component. This wrapper
     only handles positioning. */
  .win-collapsed-anchor {
    position: absolute;
    top: 4px;
    right: 4px;
    z-index: 10;
  }

  /* Expanded: horizontal tab bar pinned to the top of the Terminal view.
     Holds current-session windows AND cross-session AI chips in one scroll
     strip. Chip visuals live in AgentChip. */
  .win-bar {
    display: flex;
    align-items: center;
    gap: 4px;
    padding: 3px 4px 3px 6px;
    border-bottom: 1px solid var(--border2);
    background: var(--surface);
    flex-shrink: 0;
    position: relative; /* anchor for the PanePicker popover */
  }
  .win-session {
    flex-shrink: 0;
    display: inline-flex;
    align-items: center;
    gap: 4px;
    max-width: 160px;
    padding: 3px 7px;
    border: none;
    border-radius: 999px;
    background: var(--accent-bg);
    color: var(--accent);
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'Maple Mono CJK', 'SF Mono', Menlo, monospace;
    font-size: 11px;
    font-weight: 600;
    white-space: nowrap;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .win-session:active { background: var(--accent); color: var(--bg); }
  .win-session-name {
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .win-bar-scroll {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 4px;
    overflow-x: auto;
    scrollbar-width: none;
    -webkit-overflow-scrolling: touch;
  }
  .win-bar-scroll::-webkit-scrollbar { display: none; }
  .win-bar-collapse {
    flex-shrink: 0;
    width: 24px; height: 24px;
    padding: 0;
    border: none;
    border-radius: 999px;
    background: transparent;
    color: var(--text3);
    cursor: pointer;
    display: flex; align-items: center; justify-content: center;
    -webkit-tap-highlight-color: transparent;
  }
  .win-bar-collapse:active { color: var(--accent); background: var(--surface2); }

  .win-sep {
    flex-shrink: 0;
    width: 1px;
    height: 14px;
    background: var(--border2);
    margin: 0 2px;
  }

  .input-status {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 2px 12px;
    font-size: 10px;
    color: var(--text3);
  }
  .status-left .kiro { color: var(--accent); }
  .status-left {
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: 10px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status-pct {
    display: flex;
    align-items: center;
    gap: 5px;
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace;
    font-weight: 500;
    font-size: 12px;
    margin-left: auto;
  }
  .pct-bar {
    width: 48px;
    height: 4px;
    background: var(--surface2);
    border-radius: 2px;
    overflow: hidden;
  }
  .pct-fill {
    display: block;
    height: 100%;
    border-radius: 2px;
    transition: width 0.3s ease, background 0.3s ease;
  }
  .pct-ok { color: var(--status-ok); }
  .pct-warn { color: var(--status-warn); }
  .pct-danger { color: var(--status-danger); }
  .pct-fill.pct-ok { background: var(--status-ok); }
  .pct-fill.pct-warn { background: var(--status-warn); }
  .pct-fill.pct-danger { background: var(--status-danger); }

  .term-wrap {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    position: relative;
  }
  .term-wrap.hidden {
    position: absolute;
    left: -9999px;
    visibility: hidden;
  }

  .toast {
    position: absolute;
    top: 50%;
    left: 50%;
    transform: translate(-50%, -50%);
    background: rgba(0, 0, 0, 0.8);
    color: #fff;
    padding: 8px 20px;
    border-radius: 8px;
    font-size: 13px;
    z-index: 20;
    pointer-events: none;
  }

  .xterm-wrap {
    height: 100%;
    transition: margin-top 0.15s ease;
  }
  /* Keep xterm scrollbar always visible and touch-friendly on mobile */
  .xterm-wrap :global(.xterm-scrollable-element > .invisible) {
    opacity: 0.6 !important;
    pointer-events: auto !important;
  }
  .xterm-wrap :global(.xterm-scrollable-element > .visible) {
    opacity: 1 !important;
  }
  .xterm-wrap :global(.slider) {
    min-height: 40px !important;
    border-radius: 4px !important;
  }

  /* ─── Mobile selection handles + toolbar ──────────────────────────────── */
  /* iOS-style lollipop. The .sel-handle root is positioned at the precise
     anchor point on the selection edge (no negative margins — let JS land
     the anchor exactly on the cell corner). The stem (::before) rides ALONG
     the cell's vertical edge for one row of height; the dot (::after) is
     attached to the FREE end of the stem, away from the selection. */
  .sel-handle {
    position: absolute;
    width: 0; height: 0;
    z-index: 8;
    pointer-events: none; /* hit-test done in JS */
  }
  /* The .sel-handle div is 0×0 and positioned exactly on the anchor (cell
     corner). Both ::before (stem) and ::after (dot) are positioned relative
     to that single point.
     Invariant: stem and dot are both centered on the anchor's X (translateX(-50%)).
     The dot's near edge meets the stem's far end with no gap. */
  .sel-handle::before {
    /* Stem: 2px wide, one cell tall. */
    content: '';
    position: absolute;
    width: 2px;
    height: var(--cell-h, 16px);
    background: var(--accent, #00d4ff);
    transform: translateX(-50%);
    left: 0;
  }
  .sel-handle::after {
    /* Dot: 12px circle. translateX(-50%) centers it on the anchor X.
       --dot-shift-x is normally 0; when the handle is at column 0 / cols-1,
       JS sets it to ±6 px so the dot stays fully inside the touchable
       area instead of half-clipped against the screen edge. */
    content: '';
    position: absolute;
    width: 12px;
    height: 12px;
    border-radius: 50%;
    background: var(--accent, #00d4ff);
    box-shadow: 0 1px 3px rgba(0,0,0,0.35);
    transform: translateX(calc(-50% + var(--dot-shift-x, 0px)));
    left: 0;
  }
  /* Start handle: anchor at cell top-left.
       stem occupies [0, +cellH] (down through selection's first row)
       dot occupies  [+cellH, +cellH+12] (BELOW the stem, same side as the
       end handle's dot — keeps the touch target below the fingertip so the
       endpoint stays visible while dragging) */
  .sel-handle-start::before { top: 0; }
  .sel-handle-start::after  { top: calc(var(--cell-h, 16px)); }
  /* End handle: anchor at cell bottom-right.
       stem occupies [-cellH, 0] (up into selection's last row)
       dot occupies  [0, +12] (below anchor, outside selection) */
  .sel-handle-end::before { top: calc(0px - var(--cell-h, 16px)); }
  .sel-handle-end::after  { top: 0; }

  .sel-toolbar {
    position: absolute;
    transform: translate(-50%, -100%);
    z-index: 9;
    background: rgba(20, 20, 28, 0.95);
    border: 1px solid var(--border, #2a2a3a);
    border-radius: 10px;
    padding: 4px;
    box-shadow: 0 6px 20px rgba(0,0,0,0.4);
    backdrop-filter: blur(10px); -webkit-backdrop-filter: blur(10px);
    display: flex;
    gap: 2px;
  }
  .sel-toolbar.below {
    transform: translate(-50%, 0);
  }
  :global(html[data-theme="light"]) .sel-toolbar {
    background: rgba(245, 245, 247, 0.95);
  }
  .sel-toolbar-btn {
    background: transparent;
    border: none;
    color: var(--accent, #00d4ff);
    font-size: 13px;
    font-weight: 500;
    padding: 6px 14px;
    border-radius: 6px;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    min-width: 56px;
    min-height: 32px;
  }
  .sel-toolbar-btn:active {
    background: rgba(0, 212, 255, 0.15);
  }

  .scroll-btn {
    position: absolute;
    bottom: 12px;
    right: 16px;
    width: 36px; height: 36px;
    background: rgba(10,10,15,0.85);
    border: 1px solid var(--border);
    border-radius: 10px;
    color: var(--accent);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 5;
    -webkit-tap-highlight-color: transparent;
    backdrop-filter: blur(8px); -webkit-backdrop-filter: blur(8px);
  }
  @supports (backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px)) {
    .scroll-btn { background: rgba(10,10,15,0.45); }
    :global(html[data-theme="light"]) .scroll-btn { background: rgba(245,245,247,0.45); }
  }
  :global(html[data-theme="light"]) .scroll-btn { background: rgba(245,245,247,0.85); }
  .scroll-btn:active { transform: scale(0.9); }
  .scroll-btn.has-new { border-color: var(--accent); color: var(--accent); }
  .new-dot {
    position: absolute; top: 4px; right: 4px;
    width: 8px; height: 8px; border-radius: 50%;
    background: var(--danger, #ff5050);
    box-shadow: 0 0 0 2px var(--bg);
  }

  .input-area {
    flex-shrink: 0;
    padding: 0 10px 10px;
    padding-bottom: max(10px, env(safe-area-inset-bottom));
  }
  :global(html.keyboard-open) .input-area { padding: 0 4px 2px; }

  .input-bar {
    background: rgba(10,10,15,0.65);
    backdrop-filter: blur(12px); -webkit-backdrop-filter: blur(12px);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
  :global(html[data-theme="light"]) .input-bar { background: rgba(245,245,247,0.65); }
  :global(html.keyboard-open) .input-bar {
    border-radius: 8px;
    padding: 4px 6px;
    gap: 4px;
  }

  .shortcut-rows {
    display: flex;
    flex-direction: column;
    gap: 3px;
  }

  .shortcuts {
    display: flex;
    gap: 3px;
  }
  .shortcuts::-webkit-scrollbar { display: none; }

  .shortcuts button {
    flex: 1;
    padding: 5px 0;
    border: 1px solid var(--input-border);
    border-radius: 7px;
    background: var(--input-bg);
    color: var(--text2);
    font-size: 12px;
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace;
    font-weight: 500;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    display: flex; align-items: center; justify-content: center;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2), 0 1px 0 rgba(255, 255, 255, 0.04) inset;
  }
  .shortcuts button:active,
  .shortcuts :global(button.pressed) {
    background: var(--accent-bg);
    color: var(--accent);
    border-color: var(--accent);
    transform: translateY(1px);
    box-shadow: none;
  }
  :global(html.keyboard-open) .shortcuts .kb-toggle {
    background: var(--accent-bg);
    color: var(--accent);
    border-color: var(--accent);
  }

  .cmd-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .prompt {
    color: var(--accent);
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: 15px;
    font-weight: 600;
    flex-shrink: 0;
    filter: drop-shadow(0 0 4px var(--accent-glow));
  }

  .cmd-row textarea {
    flex: 1;
    min-width: 0;
    padding: 8px 0;
    border: none;
    background: transparent;
    color: var(--text);
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: 15px;
    outline: none;
    -webkit-appearance: none;
    resize: none;
    max-height: 120px;
    overflow-y: auto;
    line-height: 1.4;
    scrollbar-width: thin;
    scrollbar-color: var(--border) transparent;
  }
  .cmd-row textarea::placeholder { color: var(--text3); }

  .stop-btn {
    width: 34px; height: 34px;
    border: none;
    border-radius: 9px;
    background: var(--danger-bg);
    color: var(--danger);
    font-size: 12px;
    cursor: pointer;
    flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
    transition: all 0.15s ease;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .stop-btn:active {
    background: var(--danger-bg);
    transform: scale(0.92);
  }

  .send {
    width: 34px; height: 34px;
    border: none;
    border-radius: 9px;
    background: var(--accent);
    color: var(--bg);
    font-size: 15px;
    cursor: pointer;
    flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
    transition: all 0.15s ease;
    display: flex;
    align-items: center;
    justify-content: center;
  }
  .send:active {
    transform: scale(0.92);
    filter: brightness(0.85);
  }
</style>
