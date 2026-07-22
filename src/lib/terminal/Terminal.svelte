<script>
  import { subscribe, unsubscribe, addPaneOutputListener, removePaneOutputListener, addPaneClosedListener, removePaneClosedListener, sendKeys, pasteText, listPanes, capturePane, resizePane, newWindow } from '../core/ws.ts';
  import { Terminal } from '@xterm/xterm';
  import { WebLinksAddon } from '@xterm/addon-web-links';
  import Icon from '../ui/Icon.svelte';
  import AgentChip from '../ui/AgentChip.svelte';
  import { otherTerminalSessionHasNotification, terminalNotificationForWindow } from '../core/agent-notifications.svelte.ts';
  import PanePicker from '../sessions/PanePicker.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { detectAgent, paneIsAgent, paneAgent, AGENTS } from '../core/agents.ts';
  import { copyText } from '../core/clipboard.ts';
  import { fonts } from '../app/fonts.svelte.ts';
  import { terminalPrefs } from '../app/terminal-prefs.svelte.ts';
  import { adaptAnsiColors } from './ansi-colors.ts';
  import { compactLineGeometry } from './terminal-line-geometry.ts';
  import { selStart, selEnd } from './selection-model.ts';
  import { countLines, computeCursorLayout } from './cursor-layout.ts';
  import { restoreViewportAfterPaneSwitch } from './terminal-viewport.ts';
  import { cycleItem } from '../app/shortcuts.ts';
  import { encodeTerminalShortcut } from './terminal-keyboard.ts';
  import { openExternalUrl } from '../core/external-links.ts';

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
  let { target, session, command: initialCommand = '', fontSize = 14, embedded = false, active = true, chromeless = false, onSwitchPane = null, onPaneExit = () => {}, onClose = null, splitEligible = false, splitActive = false, splitLayout = 1, onSetLayout = null } = $props();
  let splitMenuOpen = $state(false);

  // svelte-ignore state_referenced_locally — intentional: seeded from the
  // prop (hence the name), then kept fresh by the $effect below and by
  // pane_output pushes carrying current_command.
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
  // briefly overlapping the last glyph. Kept narrow so it doesn't feel
  // visually heavy; on mobile it stays a touch wider for fingertip drag.
  const SCROLLBAR_W = isMobile ? 12 : 8;
  let kbBlurTimer = null;
  let kbLocked = true; // true = keyboard must not show; false = keyboard allowed
  let unlockUntil = 0; // grace window after explicit unlock; auto-lock paths must respect it
  let unlockRetries = 0; // blur re-focus attempts inside the current grace window
  const UNLOCK_RETRY_MAX = 2;
  let endTouchScrollTimer = null;
  let kbTa = null; // set in $effect after term.open
  let ctrlArmed = $state(false);

  function preparePaneSwitch() {
    document.activeElement?.blur();
    touchScrolling = false;
    restoreViewportAfterPaneSwitch({
      isMobile,
      fullHeight: window.__fullHeight?.() || window.innerHeight,
      root: document.documentElement,
    });
  }

  $effect(() => {
    target;
    ctrlArmed = false;
  });

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
    // Overlay scrollbar slider — kept very translucent so it barely occludes
    // terminal content at rest, lifting slightly on hover/drag.
    scrollbarSliderBackground: 'rgba(201, 209, 217, 0.12)',
    scrollbarSliderHoverBackground: 'rgba(201, 209, 217, 0.28)',
    scrollbarSliderActiveBackground: 'rgba(201, 209, 217, 0.40)',
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
    scrollbarSliderBackground: 'rgba(26, 26, 46, 0.12)',
    scrollbarSliderHoverBackground: 'rgba(26, 26, 46, 0.26)',
    scrollbarSliderActiveBackground: 'rgba(26, 26, 46, 0.38)',
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
    term.options.fontFamily = fonts.stack; // follows the custom-font setting live
    term.options.lineHeight = terminalPrefs.lineHeight;
    // xterm re-measures cell geometry on the next render, not synchronously.
    // Defer refit by two frames so calcFit reads the new cell width/height.
    // doResizeRef is set by the main $effect after term is created.
    requestAnimationFrame(() => {
      requestAnimationFrame(() => doResizeRef?.());
    });
  });
  let doResizeRef = null;

  // pane_output snapshots now carry `current_command` (server piggybacks it
  // on cursor reads — same tmux subprocess, zero extra cost). Update
  // `command` in the pane-output listener below; no separate polling RPC needed.

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

  $effect(() => {
    if (!active || chromeless) return;
    const onWindowShortcut = (event) => {
      const items = windows;
      if (!onSwitchPane || items.length < 2) return;
      const current = items.find(item => String(item.window) === currentWindow) || items[0];
      const next = cycleItem(items, current, event.detail.direction);
      if (!next || String(next.window) === currentWindow) return;
      preparePaneSwitch();
      onSwitchPane(`${next.session}:${next.window}.${next.pane}`, next.current_command);
    };
    window.addEventListener('terminal-window-shortcut', onWindowShortcut);
    return () => window.removeEventListener('terminal-window-shortcut', onWindowShortcut);
  });

  // Agent (if any) running in the currently-shown window.
  let currentWinAgent = $derived.by(() => {
    const cur = windows.find(w => String(w.window) === currentWindow);
    if (!cur) return null;
    return paneAgent(cur);
  });

  // The switcher is always shown (except chromeless agent-grid cells, which
  // have no chrome at all). It isn't just for switching windows — it also
  // carries the session picker, the new-window button, and the split-layout
  // control, so hiding it for a single-window session strands those too.
  // The collapsed state is a floating chip that steals no vertical space.
  let showSwitcher = $derived(!chromeless);

  $effect(() => {
    if (!session) return;
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
      } catch {}
      polling = false;
    };
    load();
    const id = setInterval(load, WINDOW_LIST_POLL_MS);
    return () => clearInterval(id);
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

  function syncCompactLineGeometry() {
    if (!term || !termEl) return;
    const core = term._core;
    const dimensions = core?._renderService?.dimensions;
    const devicePixelRatio = core?._coreBrowserService?.dpr || window.devicePixelRatio || 1;
    const geometry = compactLineGeometry(
      dimensions?.device?.char?.height,
      dimensions?.css?.cell?.height,
      devicePixelRatio,
      term.options.lineHeight,
    );
    termEl.classList.toggle('compact-lines', !!geometry);
    if (!geometry) {
      termEl.style.removeProperty('--xterm-char-height');
      termEl.style.removeProperty('--xterm-line-offset');
      return;
    }
    termEl.style.setProperty('--xterm-char-height', `${geometry.charCssHeight}px`);
    termEl.style.setProperty('--xterm-line-offset', `${geometry.offset}px`);
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

  // Resize confirmation: after local resize, we expect server to echo cursor.w/cursor.h
  // matching pendingCols/pendingRows. Until confirmed, ignore server dims (stale).
  let pendingCols = 0, pendingRows = 0, pendingResizeTs = 0;



  // Write content + position cursor in xterm.js. The color adapter tracks
  // effective SGR foreground/background pairs across the complete snapshot.
  let lastColorInput = '';
  let lastColorTheme = '';
  let lastColorOutput = '';
  function adaptColors(text) {
    const terminalTheme = getTermTheme();
    const themeKey = `${terminalTheme.foreground}/${terminalTheme.background}`;
    if (text === lastColorInput && themeKey === lastColorTheme) return lastColorOutput;
    lastColorInput = text;
    lastColorTheme = themeKey;
    lastColorOutput = adaptAnsiColors(text, terminalTheme);
    return lastColorOutput;
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

    let cursorSeq = '', afterPad = '';
    if (cursor) {
      const layout = computeCursorLayout(content, cursor, term.rows);
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
    // afterPad is a sequence of '\n'; we add \x1b[K after each so any stale
    // cells on those rows are wiped without flashing. After the body,
    // erase-below (\x1b[0J) wipes rows a previous, taller frame painted —
    // content is top-aligned, so everything below it must be blank.
    const padAft = afterPad ? afterPad.replace(/\n/g, '\x1b[0m\x1b[K\n') : '';
    // Synchronized Output (mode 2026): tell xterm to defer rendering until
    // the whole batch is parsed. Effectively wraps the entire frame in a
    // single render commit, avoiding any partial-paint glimpses.
    term.write('\x1b[?2026h\x1b[?25l\x1b[H' + body + padAft + '\x1b[0m\x1b[0J' + cursorSeq + '\x1b[?25h\x1b[?2026l', () => {
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
      lineHeight: terminalPrefs.lineHeight,
      // Literal stack, NOT var(--font-mono): this string is consumed by xterm.js
      // for canvas/WebGL glyph measurement, not parsed as CSS, so a CSS custom
      // property would not resolve here. fonts.stack = the same stack the CSS
      // var carries (user's custom family first when set).
      fontFamily: fonts.stack,
      fontWeight: 'normal',
      fontWeightBold: 'bold',
      theme: getTermTheme(),
      scrollback: 500,
      convertEol: true,
      allowTransparency: false,
      scrollbar: { showScrollbar: true, width: SCROLLBAR_W },
    });

    term.open(termEl);
    term.loadAddon(new WebLinksAddon((e, url) => {
      e.preventDefault();
      void openExternalUrl(url).catch((error) => {
        console.error('Failed to open terminal link', error);
      });
    }));

    // Box-drawing / block-element glyphs (█ ▀ ▄ ▐▛…) must fill the ENTIRE
    // cell rect or any lineHeight > 1 tears contiguous ASCII art (e.g. the
    // Claude Code logo) into stripes — the DOM renderer draws them as font
    // text, whose ink stays font-sized inside the taller cell. The WebGL
    // addon rasterizes those codepoints to the cell rect instead
    // (customGlyphs, on by default), the same trick kitty/wezterm use.
    // Falls back to the DOM renderer wherever WebGL is unavailable.
    const webglOwner = term;
    (async () => {
      try {
        const { WebglAddon } = await import('@xterm/addon-webgl');
        if (term !== webglOwner) return; // pane switched while importing
        const addon = new WebglAddon();
        addon.onContextLoss(() => { addon.dispose(); }); // DOM renderer takes over
        webglOwner.loadAddon(addon);
      } catch { /* stay on the DOM renderer */ }
    })();

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
    let lastInputComposing = false; // per-event composition signal (see input listener)
    let onTextInsert = null;
    {
      // Desktop included: printable keys are routed through the textarea
      // (see attachCustomKeyEventHandler), so paste/composition tracking and
      // the force-clear below apply on every platform.
      const ta = termEl?.querySelector('.xterm-helper-textarea');
      if (ta) {
        // Capture phase: xterm's own paste handler is registered on this
        // textarea BEFORE ours and emits onData SYNCHRONOUSLY from inside
        // it — a same-phase listener would set the flag after onData
        // already ran and the paste would be misrouted as keystrokes.
        ta.addEventListener('paste', () => {
          isPasting = true;
          // Safety reset: if onData never fires (xterm swallowed it, or paste
          // produced no data), the flag would persist and misclassify the
          // next keystroke as paste.
          setTimeout(() => { isPasting = false; }, 200);
        }, { capture: true });
        ta.addEventListener('compositionstart', () => { isComposing = true; });
        // Also reset lastInputComposing: Chromium's commit order is
        // input(insertCompositionText) → compositionend with no trailing input
        // event, so the flag would stay true and permanently suppress the
        // auto-pair clear for IMEs that DO fire composition events (GBoard).
        ta.addEventListener('compositionend', () => { isComposing = false; lastInputComposing = false; });
        ta.addEventListener('input', (e) => {
          // Some Android IMEs (suggestion-bar keyboards common on pads, e.g.
          // Samsung Keyboard) drive the field with insertCompositionText
          // input events WITHOUT ever firing compositionstart, so the
          // isComposing flag above stays false for them. Track composition
          // per-event as a second signal: while the IME is mid-word we must
          // not force-clear the textarea below, or the IME's InputConnection
          // desyncs from the real field content and later edits garble.
          // Tracked per-event (not sticky) so IMEs that also never fire
          // compositionend still get the auto-pair clear once they commit
          // via a plain insertText.
          lastInputComposing = !!(e.isComposing || (e.inputType || '').startsWith('insertComposition'));
          window.__dbg?.(`input: ta.input type=${e.inputType} composing=${lastInputComposing} val=${JSON.stringify(ta.value).slice(0,30)} focused=${document.activeElement === ta} locked=${kbLocked}`);
        });
        // Forward plain text insertions ourselves: printable keydowns are
        // handed back to the browser (see attachCustomKeyEventHandler) so
        // CJK IMEs can convert punctuation. xterm v6 handles insertText
        // only SOMETIMES (`!e.composed || !_keyDownSeen` — i.e. exactly the
        // no-keydown IME commits WKWebView produces for CJK punctuation),
        // so leaving both handlers live sent those characters TWICE. This
        // listener sits on termEl in the CAPTURE phase — parent capture
        // runs before the textarea target listeners — and claims the event
        // with stopImmediatePropagation so xterm never sees it: exactly ONE
        // forwarder for every non-composition insertText, on every engine.
        // e.data carries the IME-converted text (， 。). Composition input
        // stays with xterm's CompositionHelper (→ onData): skip while
        // EITHER composition signal is set (Chromium commits as
        // insertCompositionText, WebKit as insertFromComposition — both
        // filtered by inputType — but an IME that commits via plain
        // insertText before compositionend must not be sent twice).
        // Paste stays with xterm's paste handler (insertFromPaste).
        // Named + registered with capture on termEl (which survives pane
        // switches): removed in the effect cleanup below, like
        // onHardwareKeydown, so re-runs don't stack forwarders.
        onTextInsert = (e) => {
          if (e.target !== ta) return;
          const composing = !!(e.isComposing || (e.inputType || '').startsWith('insertComposition'));
          if (e.inputType === 'insertText' && e.data && !composing && !isComposing) {
            e.stopImmediatePropagation();
            lastInputComposing = false;
            window.__dbg?.(`input: forward insertText ${JSON.stringify(e.data).slice(0,20)}`);
            ta.value = '';
            const ctrlByte = ctrlArmed && e.data.length === 1 && /[a-z]/i.test(e.data)
              ? String.fromCharCode(e.data.toLowerCase().charCodeAt(0) - 96)
              : null;
            if (ctrlByte != null) ctrlArmed = false;
            enqueueKeys(ctrlByte ?? e.data, true);
          }
        };
        termEl.addEventListener('input', onTextInsert, { capture: true });
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
      // Force-clear xterm's hidden textarea after keyboard input to prevent
      // accumulation from auto-paired quotes/brackets. Applies to desktop
      // too: printable keys are routed through the textarea input pipeline
      // (see attachCustomKeyEventHandler) so the same accumulation applies.
      // Skip paste so xterm.js can fully process the pasted content. Also
      // skip while an IME composition is in progress — clearing
      // textarea.value mid-composition breaks CJK/Japanese input (e.g.
      // drops pinyin the user is currently typing).
      if (!isPasting && !isComposing) {
        requestAnimationFrame(() => {
          // Re-check both signals here: onData fires synchronously inside the
          // textarea's input dispatch BEFORE our own input listener updates
          // lastInputComposing, so only this post-dispatch check sees the
          // current event's composition state.
          if (isComposing || lastInputComposing) return;
          const ta = termEl?.querySelector('.xterm-helper-textarea');
          if (ta && ta.value) ta.value = '';
        });
      }
      const ctrlByte = ctrlArmed && !isPasting && data.length === 1 && /[a-z]/i.test(data)
        ? String.fromCharCode(data.toLowerCase().charCodeAt(0) - 96)
        : null;
      if (ctrlByte != null) ctrlArmed = false;
      if (isPasting) {
        isPasting = false;
        // Paste is NOT keystrokes: sent as keys, every line separator acts
        // as an Enter press and each pasted line executes. Route it through
        // tmux's paste buffer instead — `paste-buffer -p` wraps the block
        // in bracketed-paste markers exactly when the pane app enabled
        // mode ?2004 (shells, agent TUIs), matching what a real terminal
        // emits; legacy apps still get the raw text. xterm has already
        // normalized paste line endings to \r, same as a real terminal.
        pasteText(target, data)
          .then(noteSendSuccess)
          .catch((e) => {
            // Pre-paste_text server (-32601 method-not-found): fall back to
            // the old keystroke path rather than dropping the paste.
            if (e.code === -32601) { enqueueKeys(data, true); return; }
            noteSendFailure('paste');
          });
        return;
      }
      enqueueKeys(ctrlByte ?? data, true);
    });
    // Plain printable keys must go through the browser's input pipeline
    // (textarea `input` event), NOT xterm's keydown fast path. CJK IMEs
    // convert punctuation (， 。 、) at the input stage WITHOUT composition
    // events; xterm's keydown handler sees the raw ASCII key (`,` `.`),
    // emits it and calls preventDefault — the IME conversion is silently
    // dropped and the user gets English punctuation. Returning false hands
    // the key back to the browser; the inserted (possibly IME-converted)
    // text then reaches onData via the input event, same as mobile typing.
    // Modified combos and named keys (Enter, arrows, F-keys: key.length > 1)
    // keep xterm's keydown path; Ctrl/Alt combos are claimed even earlier by
    // the capture-phase hardware handler below.
    term.attachCustomKeyEventHandler((event) => {
      if (event.type !== 'keydown' && event.type !== 'keypress') return true;
      if (event.ctrlKey || event.altKey || event.metaKey) return true;
      return (event.key?.length ?? 0) !== 1;
    });

    // Forward every unclaimed hardware Ctrl / Option combination straight to
    // tmux. Browser/WKWebView textareas otherwise consume editing bindings
    // such as Ctrl+X / Ctrl+F and Option dead keys before xterm emits onData.
    // Touch capability is not a reliable hardware-keyboard test: desktop
    // Chromium and WKWebView may expose touch APIs, and phones can have a
    // physical keyboard. App shortcuts run first in window capture.
    const onHardwareKeydown = (event) => {
      const data = encodeTerminalShortcut(event);
      if (!data) return;
      event.preventDefault();
      event.stopImmediatePropagation();
      window.__dbg?.(`kb(hardware): passthrough ${event.code || event.key} → ${JSON.stringify(data)}`);
      enqueueKeys(data, true);
    };
    termEl.addEventListener('keydown', onHardwareKeydown, { capture: true });

    let focusTerm = null;
    if (!isMobile) {
      // Desktop has no on-screen keyboard / toggle, so focus the xterm sink
      // on click so ordinary typing (and our handler) works. Auto-focus on
      // mount ONLY for the active terminal — otherwise multiple split cells
      // race for the single document focus and input lands nowhere.
      focusTerm = () => { try { term.focus(); } catch {} };
      termEl.addEventListener('mousedown', focusTerm);
      if (active) requestAnimationFrame(focusTerm);
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
    // writeToXterm. A suspended WebView can also keep its WebSocket OPEN while
    // pane pushes stop, so visibility recovery must pull a fresh snapshot rather
    // than merely repainting the pre-suspend cache.
    let followedTailBeforeHide = true;
    let resumeGeneration = 0;
    const onVisible = () => {
      if (document.visibilityState !== 'visible') {
        followedTailBeforeHide = termAtBottom;
        resumeGeneration++;
        return;
      }
      const generation = ++resumeGeneration;
      const resumeAtTail = followedTailBeforeHide;
      touchScrolling = false;
      onScrollbar = false;
      touchMode = 'idle';
      dragHandle = null;
      stopMomentum();
      stopEdgeScroll();
      // Drop the selection — re-attaching to a clipboard from before
      // backgrounding is rarely useful and could surprise the user.
      if (selection) clearSelection();
      if (resumeAtTail) {
        // Layout/keyboard changes while suspended can make xterm emit a stale
        // scroll position on resume. Preserve the user's intent (live tail)
        // rather than interpreting that geometry change as manual scrollback.
        termAtBottom = true;
        hasNewContent = false;
        term?.scrollToBottom();
      }
      doResizeRef?.();
      if (lastContent && resumeAtTail) writeToXterm(lastContent, lastCursor);
      term?.refresh(0, term.rows - 1);

      // App.svelte re-sends the wire subscription on the same visibility event.
      // This explicit snapshot closes the gap even if that OPEN socket is only
      // half-alive: a successful send_keys/capture response proves the RPC path
      // and immediately converges the display to tmux's actual contents.
      capturePane(target).then(r => {
        if (generation !== resumeGeneration || !term) return;
        const content = r.output ?? r.content;
        if (content == null) return;
        const changed = content !== lastContent;
        lastContent = content;
        if (resumeAtTail) {
          writeToXterm(content, lastCursor);
          requestAnimationFrame(() => term?.scrollToBottom());
        } else if (changed) {
          hasNewContent = true;
        }
      }).catch(() => {});
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
    let resizeSendTimer = 0;
    function queuePaneResize(cols, rows) {
      pendingCols = cols;
      pendingRows = rows;
      pendingResizeTs = Date.now();
      clearTimeout(resizeSendTimer);
      resizeSendTimer = setTimeout(() => {
        resizeSendTimer = 0;
        resizePane(target, pendingCols, pendingRows).catch(() => {});
      }, 120);
    }

    function doResize() {
      syncCompactLineGeometry();
      const fit = calcFit();
      if (!fit) return;
      window.__dbg?.(`resize: fit=${fit.cols}x${fit.rows} cur=${term.cols}x${term.rows} elH=${termEl.clientHeight}`);
      if (fit.cols === term.cols && fit.rows === term.rows) {
        // Same dims but cell metrics may have changed (font size); refresh
        // selection UI either way.
        if (selection) recomputeSelUI();
        return;
      }
      queuePaneResize(fit.cols, fit.rows);
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
    const onAppZoom = () => doResize();
    window.addEventListener('app-zoom-change', onAppZoom);

    // First real paint → real cell metrics available → refit once so the
    // initial ResizeObserver tick (which ran with estimated metrics) gets
    // corrected. term.onRender fires on every render, so we disarm after
    // the first call.
    let firstRenderDone = false;
    const onFirstRender = term.onRender(() => {
      syncCompactLineGeometry();
      if (firstRenderDone) return;
      firstRenderDone = true;
      doResize();
    });

    // Re-measure once webfonts settle.
    //
    // The TEXT font is now a system family (or a locally-installed custom
    // font), so cell-width measurement at open() is already correct — the
    // historical "characters stuck together" class of bugs (async text
    // webfont swapping in after xterm measured the fallback) is gone by
    // construction. Only the two bundled SYMBOL fonts load async; they don't
    // drive cell metrics, but their glyphs land in the atlas, so refresh
    // once after document.fonts.ready to repaint any tofu drawn before the
    // symbol fonts decoded. Kept cheap: atlas clear + repaint, no refit.
    let fontReadyHandled = false;
    const remeasureAfterFonts = () => {
      if (fontReadyHandled || !term) return;
      fontReadyHandled = true;
      try {
        term._core?._renderService?.clearTextureAtlas?.();
      } catch {}
      term.refresh(0, term.rows - 1);
    };
    if (document.fonts?.ready) {
      document.fonts.ready.then(remeasureAfterFonts).catch(() => {});
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
          queuePaneResize(fit.cols, fit.rows);
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
      // Drives the window-switcher agent icons and status-bar highlight.
      if (currentCommand !== undefined) {
        command = currentCommand;
      }
      if (content != null && content !== lastContent) {
        lastContent = content;
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
        lastContent = c;
        writeToXterm(c, lastCursor);
      }
    }).catch(() => {});

    return () => {
      resizeObs.disconnect();
      clearTimeout(resizeSendTimer);
      window.removeEventListener('app-zoom-change', onAppZoom);
      try { onFirstRender.dispose(); } catch {}
      try { onSelChange.dispose(); } catch {}
      doResizeRef = null;
      termEl?.classList.remove('compact-lines');
      termEl?.style.removeProperty('--xterm-char-height');
      termEl?.style.removeProperty('--xterm-line-offset');
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
      termEl.removeEventListener('keydown', onHardwareKeydown, { capture: true });
      if (onTextInsert) termEl.removeEventListener('input', onTextInsert, { capture: true });
      if (focusTerm) termEl.removeEventListener('mousedown', focusTerm);
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

  // Re-sync size when the terminal (re)appears
  $effect(() => {
    if (term) {
      requestAnimationFrame(() => term.refresh(0, term.rows - 1));
    }
  });

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

  function toggleCtrl() {
    stopRepeat();
    ctrlArmed = !ctrlArmed;
    navigator.vibrate?.(8);
  }

  // Long-press repeat for shortcut keys
  let repeatTimer = null;
  let repeatInterval = null;

  function startRepeat(key) {
    ctrlArmed = false;
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
        Expanded switcher: a top-of-page horizontal tab bar for the current
        session's windows. The session chip opens the all-session picker.
      -->
      <div class="win-bar">
        <!-- Session name as a fixed tag at the far left of the switcher row,
             so it's always visible without stealing a whole row. The window
             chips scroll independently to its right. -->
        <AgentChip
          attention={otherTerminalSessionHasNotification(session)}
          label={session}
          variant="active"
          title={session}
          onclick={(e) => { e.stopPropagation(); showPanePicker = !showPanePicker; }}
        />
        {#if showPanePicker}
          <PanePicker
            currentTarget={target}
            onPick={(p) => {
              showPanePicker = false;
              if (`${p.session}:${p.window}.${p.pane}` !== target && onSwitchPane) {
                preparePaneSwitch();
                onSwitchPane(`${p.session}:${p.window}.${p.pane}`, p.current_command);
              }
            }}
            onClose={() => showPanePicker = false}
          />
        {/if}
        <div class="win-bar-scroll">
          {#each windows as w}
            {@const wAgent = paneAgent(w)}
            {@const notice = terminalNotificationForWindow(w.session, w.window)}
            <AgentChip
              attention={!!notice}
              urgent={notice && notice.kind !== 'completed'}
              agent={wAgent}
              label={wAgent ? '' : (w.current_command || w.window_name)}
              variant={String(w.window) === currentWindow ? 'active' : 'default'}
              title={w.current_command || w.window_name}
              onclick={(e) => {
                e.stopPropagation();
                if (String(w.window) !== currentWindow && onSwitchPane) {
                  preparePaneSwitch();
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
                  preparePaneSwitch();
                  onSwitchPane(`${p.session}:${p.window}.${p.pane}`, p.current_command);
                }
              } catch {}
            }}
          />

        </div>

        {#if !embedded && splitEligible && onSetLayout}
          <div class="win-split">
            <button class="win-bar-collapse win-split-btn" class:on={splitActive} title={t('split')} aria-label={t('split')}
              onclick={(e) => { e.stopPropagation(); splitMenuOpen = !splitMenuOpen; }}>
              <Icon name="layout" size={12} />
            </button>
            {#if splitMenuOpen}
              <button class="win-split-backdrop" aria-label="close" onclick={(e) => { e.stopPropagation(); splitMenuOpen = false; }}></button>
              <div class="win-split-menu">
                {#each [1, 2, 3, 4, 6] as n}
                  <button class="win-split-opt" class:active={(n === 1 && !splitActive) || (splitActive && splitLayout === n)}
                    onclick={(e) => { e.stopPropagation(); onSetLayout(n); splitMenuOpen = false; }}>{n}</button>
                {/each}
              </div>
            {/if}
          </div>
        {/if}
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
          attention={!!terminalNotificationForWindow(cur?.session, cur?.window)}
          urgent={terminalNotificationForWindow(cur?.session, cur?.window)?.kind !== 'completed'}
          agent={curAgent}
          label={curAgent ? '' : (cur?.current_command || cur?.window_name || '?')}
          chevron="left"
          onclick={() => { showWindowCmd = true; localStorage.setItem('tmux_winswitcher', '1'); }}
        />
      </div>
    {/if}
  {/if}

  <div class="term-wrap">
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
  {#if !chromeless}
  <div class="input-area">
    {#if isMobile}
      <div class="input-bar">
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="shortcut-rows" use:nonPassiveShortcuts ontouchend={stopRepeat} ontouchcancel={stopRepeat} oncontextmenu={(e) => e.preventDefault()} onmouseup={stopRepeat}>
          <div class="shortcuts">
            <button tabindex="-1" ontouchstart={() => startRepeat('Escape')}><span>Esc</span></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('Tab')}><span>Tab</span></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('C-a')}><span><Icon name="skip-left" size={13} /></span></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('Up')}><span><Icon name="arrow-up" size={13} /></span></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('C-e')}><span><Icon name="skip-right" size={13} /></span></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('BSpace')}><span><Icon name="delete" size={13} /></span></button>
          </div>
          <div class="shortcuts">
            <button class="modifier" class:active={ctrlArmed} aria-pressed={ctrlArmed} tabindex="-1" ontouchstart={toggleCtrl}><span>Ctrl</span></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('C-c')}><span>^C</span></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('Left')}><span><Icon name="arrow-left" size={13} /></span></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('Down')}><span><Icon name="arrow-down" size={13} /></span></button>
            <button tabindex="-1" ontouchstart={() => startRepeat('Right')}><span><Icon name="arrow-right" size={13} /></span></button>
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
            }}><span><Icon name="keyboard" size={13} /></span></button>
          </div>
        </div>
      </div>
    {:else}
      <!-- Desktop: no input bar, keyboard goes directly to xterm.js -->
    {/if}
  </div>
  {/if}
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
     Holds current-session windows; chip visuals live in AgentChip. */
  .win-bar {
    display: flex;
    align-items: center;
    gap: var(--ui-gap);
    min-height: var(--ui-bar-height);
    padding: var(--ui-bar-padding);
    box-sizing: border-box;
    border-bottom: 1px solid var(--border2);
    background: var(--surface);
    flex-shrink: 0;
    position: relative; /* anchor for the PanePicker popover */
  }
  .win-bar-scroll {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--ui-gap);
    overflow-x: auto;
    scrollbar-width: none;
    -webkit-overflow-scrolling: touch;
  }
  .win-bar-scroll::-webkit-scrollbar { display: none; }
  .win-bar-collapse {
    flex-shrink: 0;
    width: var(--ui-control-height); height: var(--ui-control-height);
    padding: 0;
    border: none;
    border-radius: var(--ui-radius-pill);
    background: transparent;
    color: var(--text3);
    cursor: pointer;
    display: flex; align-items: center; justify-content: center;
    -webkit-tap-highlight-color: transparent;
  }
  .win-bar-collapse:active { color: var(--accent); background: var(--surface2); }

  .win-split { position: relative; flex-shrink: 0; display: flex; }
  .win-split-btn.on { color: var(--accent); }
  .win-split-backdrop { position: fixed; inset: 0; z-index: 40; border: none; background: none; }
  .win-split-menu {
    position: absolute; top: 100%; right: 0; z-index: 41; margin-top: 4px;
    display: flex; gap: 3px; padding: 4px;
    background: var(--bg); border: 1px solid var(--border); border-radius: 9px;
    box-shadow: 0 8px 24px rgba(0,0,0,0.35);
  }
  .win-split-opt {
    width: 26px; height: 26px; padding: 0;
    border: 1px solid var(--border2); border-radius: 6px;
    background: var(--input-bg); color: var(--text2);
    font-size: 12px; font-weight: 600; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .win-split-opt.active { border-color: var(--accent); color: var(--accent); background: var(--accent-bg); }
  .win-split-opt:active { border-color: var(--accent); }


  .term-wrap {
    flex: 1;
    min-height: 0;
    overflow: hidden;
    position: relative;
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
  :global(.xterm-wrap.compact-lines .xterm-glyph) {
    display: inline-block;
    height: var(--xterm-char-height) !important;
    line-height: var(--xterm-char-height) !important;
    transform: translateY(calc(-1 * var(--xterm-line-offset)));
    font-weight: inherit !important;
    text-decoration: inherit;
    text-decoration-color: inherit;
  }
  /* xterm scrollbar: invisible at rest, fades in only while the user is
     interacting (xterm toggles `.invisible` ↔ `.visible` on hover/active
     scroll). On mobile we keep `pointer-events: auto` on the resting state
     so a finger can still grab the slider area, even though it's barely
     drawn. */
  .xterm-wrap :global(.xterm-scrollable-element > .invisible) {
    opacity: 0 !important;
    pointer-events: auto !important;
    transition: opacity 0.35s ease 0.2s !important;
  }
  .xterm-wrap :global(.xterm-scrollable-element > .visible) {
    opacity: 1 !important;
    transition: opacity 0.12s ease !important;
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
    height: var(--ui-control-height);
    padding: 0;
    box-sizing: border-box;
  }
  .shortcuts::-webkit-scrollbar { display: none; }

  .shortcuts button {
    flex: 1;
    height: var(--ui-control-height);
    padding: 0;
    box-sizing: border-box;
    border: 1px solid var(--input-border);
    border-radius: var(--ui-radius-pill);
    background: var(--input-bg);
    color: var(--text2);
    font-size: var(--ui-font-control);
    line-height: 1.3;
    font-family: var(--font-mono);
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
  .shortcuts button > span {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    transform: translateY(1px);
  }
  .shortcuts button.modifier.active {
    background: var(--accent-bg);
    color: var(--accent);
    border-color: var(--accent);
    box-shadow: none;
  }
  :global(html.keyboard-open) .shortcuts .kb-toggle {
    background: var(--accent-bg);
    color: var(--accent);
    border-color: var(--accent);
  }


</style>
