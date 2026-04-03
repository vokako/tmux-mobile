<script>
  import { subscribe, unsubscribe, setOnPaneOutput, sendCommand, sendKeys, paneCommand, listPanes, capturePane, resizePane } from './ws.js';
  import { Terminal } from '@xterm/xterm';
  import ChatView from './ChatView.svelte';
  import Icon from './Icon.svelte';
  import { detectParser } from './parsers.js';

  let { target, session, command: initialCommand = '', viewMode = 'terminal', onChatSupported = () => {}, onSwitchPane = null } = $props();

  let input = $state('');
  let paneContent = $state('');
  let command = $state(initialCommand);
  let directMode = $state(false);
  $effect(() => { command = initialCommand; });
  let termEl;
  let term;
  let termAtBottom = $state(true);
  let toastMsg = $state('');
  let inputEl;
  const isMobile = 'ontouchstart' in window || navigator.maxTouchPoints > 0;

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
    black: '#1a1a2e', brightBlack: '#6b7280',
    red: '#dc2626', brightRed: '#ef4444',
    green: '#16a34a', brightGreen: '#22c55e',
    yellow: '#ca8a04', brightYellow: '#eab308',
    blue: '#0088cc', brightBlue: '#2563eb',
    magenta: '#9333ea', brightMagenta: '#a855f7',
    cyan: '#0891b2', brightCyan: '#06b6d4',
    white: '#e2e8f0', brightWhite: '#f8fafc',
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

  let parser = $derived(detectParser(paneContent, command));

  $effect(() => { onChatSupported(!!parser); });

  // Poll pane command every 3s to detect kiro start/exit
  $effect(() => {
    const poll = () => paneCommand(target).then(r => { command = r.command || ''; }).catch(() => {});
    poll();
    const id = setInterval(poll, 3000);
    return () => clearInterval(id);
  });

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
  let showWindowCmd = $state(false);
  let currentWindow = $derived(target.split(':')[1]?.split('.')[0] || '');

  // Group panes by window
  let windows = $derived.by(() => {
    const map = new Map();
    for (const p of windowPanes) {
      if (!map.has(p.window)) map.set(p.window, p);
    }
    return [...map.values()];
  });

  $effect(() => {
    if (!session) return;
    const load = () => listPanes(session).then(p => { windowPanes = p; }).catch(() => {});
    load();
    const id = setInterval(load, 5000);
    return () => clearInterval(id);
  });

  // Calculate optimal cols/rows — on mobile, use max observed container height
  // (not keyboard-shrunk height) to keep terminal size stable across keyboard open/close
  let maxContainerH = 0;
  function calcFit() {
    if (!term || !termEl) return null;
    const core = term._core;
    const cellW = core?._renderService?.dimensions?.css?.cell?.width || (term.options.fontSize * 0.6);
    const cellH = core?._renderService?.dimensions?.css?.cell?.height || (term.options.fontSize * 1.2);
    const w = termEl.clientWidth;
    const clientH = termEl.clientHeight;
    if (clientH > maxContainerH) maxContainerH = clientH;
    // On mobile, use max seen height so keyboard doesn't shrink the terminal
    const h = isMobile ? Math.max(clientH, maxContainerH) : clientH;
    if (!w || !h || !cellW || !cellH) return null;
    return { cols: Math.max(2, Math.floor(w / cellW)), rows: Math.max(1, Math.floor(h / cellH)) };
  }

  let touchScrolling = false; // set by touch handler, pauses content updates

  // Write content + position cursor in xterm.js
  function writeToXterm(content, cursor) {
    if (!term || touchScrolling) return;
    // Sync xterm cols/rows when pane size changes (tmux controls the size)
    if (cursor?.w && cursor?.h && (term.cols !== cursor.w || term.rows !== cursor.h)) {
      term.resize(cursor.w, cursor.h);
    }
    const buf = term.buffer.active;
    const atBottom = buf.viewportY >= buf.baseY;
    const prevViewport = buf.viewportY;
    // Compute correct xterm screen row for cursor:
    // Content includes scrollback (-S -200) + visible pane (trimmed).
    // cursor.y is relative to the visible pane, not the content.
    // We map: paneStart = N + trailing - paneHeight, cursorLine = paneStart + cursor.y
    // Then adjust for xterm scrollback overflow.
    let cursorSeq = '';
    let padLines = '';
    if (cursor) {
      const N = content.split('\n').length;
      const trailing = cursor.t || 0;
      const paneStart = Math.max(0, N + trailing - cursor.h);
      const cursorLine = paneStart + cursor.y; // 0-indexed content line
      // Pad so cursor line exists in content
      let pad = Math.max(0, cursorLine + 1 - N);
      // Recompute row with padding; add more if cursor row exceeds screen
      let total = N + pad;
      let sb = Math.max(0, total - term.rows);
      let row = cursorLine - sb + 1; // 1-indexed screen row
      if (row > term.rows) {
        pad += row - term.rows;
        total = N + pad;
        sb = Math.max(0, total - term.rows);
        row = cursorLine - sb + 1;
      }
      if (pad > 0) padLines = '\n'.repeat(pad);
      if (row > 0 && row <= term.rows) {
        cursorSeq = `\x1b[${row};${cursor.x + 1}H`;
      }
    }
    // Clear screen + scrollback, write content, position cursor
    term.write('\x1b[?25l\x1b[2J\x1b[3J\x1b[H' + content + padLines + cursorSeq + '\x1b[?25h', () => {
      // Skip scroll adjustment if user is touch-scrolling (async callback race)
      if (touchScrolling) return;
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
    // Pre-calculate initial size before terminal opens (avoids race with keyboard auto-focus)
    const fontSize = 14;
    const estCellW = fontSize * 0.6;
    const estCellH = fontSize * 1.2;
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
      fontFamily: "'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace",
      fontWeight: 300,
      fontWeightBold: 600,
      theme: getTermTheme(),
      scrollback: 500,
      convertEol: true,
      allowTransparency: false,
      scrollbar: { showScrollbar: true, width: isMobile ? 20 : 14 },
    });

    term.open(termEl);

    // Forward keyboard input to tmux — skip when input box is open
    term.onData(data => {
      if (directMode && inputEl) return;
      // Filter xterm.js device attribute responses (DA1/DA2/DA3)
      if (/^\x1b\[[\?>=]?[\d;]*c$/.test(data)) return;
      sendKeys(target, data, true).catch(() => {});
    });
    // Block xterm from processing keys when input box is open
    term.attachCustomKeyEventHandler(() => !directMode);

    let lastContent = '';
    let lastCursor = null;
    function endTouchScroll() {
      touchScrolling = false;
      if (lastContent) writeToXterm(lastContent, lastCursor);
    }

    // Helper: convert touch coordinates to terminal cell (col, row in viewport)
    function touchToCell(clientX, clientY) {
      const rect = termEl.getBoundingClientRect();
      const core = term._core;
      const cellW = core?._renderService?.dimensions?.css?.cell?.width || (term.options.fontSize * 0.6);
      const cellH = core?._renderService?.dimensions?.css?.cell?.height || (term.options.fontSize * 1.2);
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
    let touchY = 0, touchStartY = 0, accumulatedDy = 0, longPressTimer = null, didScroll = false;
    let velocity = 0, lastMoveTime = 0, momentumId = null, totalDist = 0;
    const lineHeight = () => (termEl?.clientHeight || 384) / (term?.rows || 24);

    let onScrollbar = false, scrollbarStartY = 0, scrollbarStartViewport = 0;
    let isSelecting = false, selectionAnchor = null, selectionRange = null;
    const stopMomentum = () => { if (momentumId) { cancelAnimationFrame(momentumId); momentumId = null; } };

    const onTouchStart = (e) => {
      stopMomentum();
      // Tap while selection active → copy if on selection, else just clear
      if (isSelecting) {
        const cell = touchToCell(e.touches[0].clientX, e.touches[0].clientY);
        const bufRow = term.buffer.active.viewportY + cell.row;
        // Hit-test: is tap within selected area?
        let onSel = false;
        if (selectionRange) {
          const { sRow, sCol, eRow, eCol } = selectionRange;
          if (bufRow >= sRow && bufRow <= eRow) {
            if (sRow === eRow) onSel = cell.col >= sCol && cell.col <= eCol;
            else if (bufRow === sRow) onSel = cell.col >= sCol;
            else if (bufRow === eRow) onSel = cell.col <= eCol;
            else onSel = true;
          }
        }
        if (onSel && term.hasSelection()) {
          const sel = term.getSelection();
          if (sel) navigator.clipboard.writeText(sel).then(() => showToast('Copied')).catch(() => {});
        }
        term.clearSelection();
        isSelecting = false;
        selectionAnchor = null;
        selectionRange = null;
        // Temporarily disable stdin so this tap doesn't open the keyboard
        term.options.disableStdin = true;
        setTimeout(() => { if (term) term.options.disableStdin = directMode; }, 300);
        endTouchScroll();
        return;
      }
      // Scrollbar drag
      const rect = termEl.getBoundingClientRect();
      const touchX = e.touches[0].clientX;
      onScrollbar = (rect.right - touchX) < 30;
      if (onScrollbar) {
        touchScrolling = true;
        scrollbarStartY = e.touches[0].clientY;
        scrollbarStartViewport = term.buffer.active.viewportY;
        return;
      }

      touchY = e.touches[0].clientY;
      touchStartY = touchY;
      accumulatedDy = 0;
      velocity = 0;
      totalDist = 0;
      lastMoveTime = Date.now();
      touchScrolling = false;
      didScroll = false;
      // Long press: 500ms hold without scroll → select word at touch point
      const startCX = e.touches[0].clientX;
      const startCY = e.touches[0].clientY;
      longPressTimer = setTimeout(() => {
        if (!didScroll && term) {
          const textarea = termEl.querySelector('.xterm-helper-textarea');
          if (textarea) textarea.blur();
          const cell = touchToCell(startCX, startCY);
          const bufRow = term.buffer.active.viewportY + cell.row;
          const bounds = wordBoundsAt(bufRow, cell.col);
          term.select(bounds.start, bufRow, bounds.end - bounds.start);
          isSelecting = true;
          selectionAnchor = { col: bounds.start, endCol: bounds.end, bufRow };
          selectionRange = { sRow: bufRow, sCol: bounds.start, eRow: bufRow, eCol: bounds.end - 1 };
          touchScrolling = true; // pause content updates during selection
        }
      }, 500);
    };
    const onTouchMove = (e) => {
      if (!term) return;
      // Scrollbar drag: map touch delta proportionally to scroll position
      if (onScrollbar) {
        const deltaY = e.touches[0].clientY - scrollbarStartY;
        const trackH = termEl.clientHeight;
        const totalScroll = term.buffer.active.baseY;
        if (totalScroll > 0 && trackH > 0) {
          const target = scrollbarStartViewport + (deltaY / trackH) * totalScroll;
          term.scrollToLine(Math.max(0, Math.min(totalScroll, Math.round(target))));
        }
        if (e.cancelable) e.preventDefault();
        return;
      }
      // Selection drag: extend from anchor word to current cell
      if (isSelecting && selectionAnchor) {
        const cell = touchToCell(e.touches[0].clientX, e.touches[0].clientY);
        const bufRow = term.buffer.active.viewportY + cell.row;
        let sCol, sRow, eCol, eRow, len;
        if (bufRow < selectionAnchor.bufRow || (bufRow === selectionAnchor.bufRow && cell.col < selectionAnchor.col)) {
          sCol = cell.col; sRow = bufRow;
          eCol = selectionAnchor.endCol - 1; eRow = selectionAnchor.bufRow;
          len = (eRow - sRow) * term.cols + (selectionAnchor.endCol - cell.col);
        } else {
          sCol = selectionAnchor.col; sRow = selectionAnchor.bufRow;
          eCol = cell.col; eRow = bufRow;
          len = (eRow - sRow) * term.cols + (cell.col + 1 - selectionAnchor.col);
        }
        term.select(sCol, sRow, Math.max(1, len));
        selectionRange = { sRow, sCol, eRow, eCol };
        if (e.cancelable) e.preventDefault();
        return;
      }
      // Normal content scroll
      const now = Date.now();
      const y = e.touches[0].clientY;
      const dy = touchY - y;
      const dt = Math.max(1, now - lastMoveTime);
      touchY = y;
      lastMoveTime = now;
      accumulatedDy += dy;
      totalDist += Math.abs(dy);
      const lh = lineHeight();
      velocity = (dy / lh) / dt * 16;
      const lines = Math.trunc(accumulatedDy / lh);
      if (lines !== 0) {
        didScroll = true;
        if (longPressTimer) { clearTimeout(longPressTimer); longPressTimer = null; }
        touchScrolling = true;
        term.scrollLines(lines);
        accumulatedDy -= lines * lh;
        if (e.cancelable) e.preventDefault();
      }
    };
    const onTouchEnd = () => {
      if (onScrollbar) { onScrollbar = false; setTimeout(endTouchScroll, 500); return; }
      if (longPressTimer) { clearTimeout(longPressTimer); longPressTimer = null; }
      // Selection active → keep visible, tap on it to copy
      if (isSelecting) return;
      if (touchScrolling && Math.abs(velocity) > 0.05) {
        // Momentum: cap by both speed and swipe distance
        const lh = lineHeight();
        const distLines = totalDist / lh;
        const distCap = Math.min(6, distLines * 0.5);
        const speedV = Math.max(-6, Math.min(6, velocity * 16));
        let v = Math.sign(speedV) * Math.min(Math.abs(speedV), distCap);
        let acc = 0;
        const coast = () => {
          v *= 0.97;
          acc += v;
          const lines = Math.trunc(acc);
          if (lines !== 0) {
            term.scrollLines(lines);
            acc -= lines;
          }
          if (Math.abs(v) > 0.05) {
            momentumId = requestAnimationFrame(coast);
          } else {
            momentumId = null;
            setTimeout(endTouchScroll, 200);
          }
        };
        momentumId = requestAnimationFrame(coast);
      } else if (touchScrolling) {
        setTimeout(endTouchScroll, 500);
      }
    };
    const onTouchCancel = () => {
      if (longPressTimer) { clearTimeout(longPressTimer); longPressTimer = null; }
      onScrollbar = false;
      isSelecting = false;
      selectionAnchor = null;
      selectionRange = null;
      stopMomentum();
      setTimeout(endTouchScroll, 100);
    };
    termEl.addEventListener('touchstart', onTouchStart, { passive: true });
    termEl.addEventListener('touchmove', onTouchMove, { passive: false });
    termEl.addEventListener('touchend', onTouchEnd, { passive: true });
    termEl.addEventListener('touchcancel', onTouchCancel, { passive: true });

    term.onScroll(() => {
      const buf = term.buffer.active;
      termAtBottom = buf.viewportY >= buf.baseY;
    });

    // Resize tmux pane to fit screen
    let lastFitCols = 0, lastFitRows = 0;
    function doResize() {
      const fit = calcFit();
      if (!fit || (fit.cols === lastFitCols && fit.rows === lastFitRows)) return;
      lastFitCols = fit.cols;
      lastFitRows = fit.rows;
      resizePane(target, fit.cols, fit.rows).catch(() => {});
      term.resize(fit.cols, fit.rows);
    }
    requestAnimationFrame(doResize);

    // Debounced resize — only on real window size changes, not keyboard open/close.
    // Track window.innerWidth/Height (stable when keyboard opens) instead of
    // visualViewport (shrinks when keyboard opens, causing double-shift).
    let lastWinW = window.innerWidth, lastWinH = window.innerHeight;
    let resizeTimer = null;
    const onResize = () => {
      const ww = window.innerWidth, wh = window.innerHeight;
      // Skip if only height changed (likely keyboard open/close on mobile)
      if (isMobile && ww === lastWinW && wh !== lastWinH) return;
      lastWinW = ww; lastWinH = wh;
      clearTimeout(resizeTimer);
      resizeTimer = setTimeout(doResize, 300);
    };
    window.addEventListener('resize', onResize);

    // Shift xterm up when keyboard opens so cursor (at bottom) stays visible
    const onKbShift = (e) => {
      if (!termEl || !term) return;
      const kbh = e.detail?.kbHeight || 0;
      if (kbh > 0) {
        // Calculate how much the terminal canvas overflows the shrunk container
        const containerH = termEl.parentElement?.clientHeight || 0; // .term-wrap height
        const core = term._core;
        const cellH = core?._renderService?.dimensions?.css?.cell?.height || (term.options.fontSize * 1.2);
        const terminalH = term.rows * cellH; // actual canvas height
        const overflow = terminalH - containerH;
        termEl.style.marginTop = overflow > 0 ? `-${overflow}px` : '0';
      } else {
        termEl.style.marginTop = '0';
      }
    };
    window.addEventListener('keyboard-shift', onKbShift);

    // Re-fit when keyboard closes (container grows back, terminal needs to match)
    const onRefit = () => {
      lastFitCols = 0; lastFitRows = 0; // force recalc
      doResize();
    };
    window.addEventListener('terminal-refit', onRefit);

    setOnPaneOutput((t, content, cursor) => {
      if (t !== target) return;
      if (cursor) lastCursor = cursor;
      if (content != null && content !== lastContent) {
        lastContent = content;
        paneContent = content;
        writeToXterm(content, lastCursor);
      } else if (cursor && term && lastContent) {
        // Cursor-only update — use same row calculation as writeToXterm
        const N = lastContent.split('\n').length;
        const trailing = cursor.t || 0;
        const paneStart = Math.max(0, N + trailing - cursor.h);
        const cursorLine = paneStart + cursor.y;
        const pad = Math.max(0, cursorLine + 1 - N);
        const total = N + pad;
        const sb = Math.max(0, total - term.rows);
        let row = cursorLine - sb + 1;
        if (row > 0 && row <= term.rows) {
          term.write(`\x1b[${row};${cursor.x + 1}H`);
        }
      }
    });

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
      clearTimeout(resizeTimer);
      stopMomentum();
      window.removeEventListener('resize', onResize);
      window.removeEventListener('terminal-refit', onRefit);
      window.removeEventListener('keyboard-shift', onKbShift);
      termEl.removeEventListener('touchstart', onTouchStart);
      termEl.removeEventListener('touchmove', onTouchMove);
      termEl.removeEventListener('touchend', onTouchEnd);
      termEl.removeEventListener('touchcancel', onTouchCancel);
      // Server kills the control-mode client on WS disconnect → tmux auto-restores size
      unsubscribe(target);
      setOnPaneOutput(null);
      term.dispose();
      term = null;
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
        input = '';
        document.querySelectorAll('.input-bar textarea').forEach(ta => ta.style.height = 'auto');
      } catch (_) {}
      return;
    }
    // Terminal mode
    if (!input.trim()) {
      await sendKeys(target, 'Enter', false).catch(() => {});
      document.activeElement?.blur();
      return;
    }
    try {
      await sendKeys(target, input, true);
      input = '';
      document.querySelectorAll('.input-bar textarea').forEach(ta => ta.style.height = 'auto');
    } catch (_) {}
  }

  async function handleKeydown(e) {
    if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) {
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

  async function sendSpecial(key) {
    try {
      await sendKeys(target, key, false);
    } catch (_) {}
  }

  // When input box opens: disable xterm stdin (so its hidden textarea is removed)
  // and focus our textarea. When closed: re-enable xterm stdin.
  $effect(() => {
    if (!term) return;
    term.options.disableStdin = directMode;
    if (directMode && inputEl) {
      requestAnimationFrame(() => inputEl.focus());
    }
  });

</script>

<div class="terminal">
  {#if toastMsg}
    <div class="toast">{toastMsg}</div>
  {/if}
  {#if windows.length > 1}
    <div class="win-switcher" class:expanded={showWindowCmd}>
      {#each windows as w}
        <button
          class="win-tab"
          class:active={String(w.window) === currentWindow}
          onclick={(e) => {
            e.stopPropagation();
            if (String(w.window) === currentWindow) { showWindowCmd = !showWindowCmd; }
            else if (onSwitchPane) {
              // Dismiss keyboard, reset layout and scroll state before switching pane
              document.activeElement?.blur();
              directMode = false;
              touchScrolling = false;
              // Use saved full height (not current innerHeight which may be keyboard-shrunk)
              const fh = window.__fullHeight?.() || window.innerHeight;
              document.documentElement.style.setProperty('--app-height', fh + 'px');
              document.documentElement.classList.remove('keyboard-open');
              onSwitchPane(`${w.session}:${w.window}.${w.pane}`, w.current_command);
              showWindowCmd = false;
            }
          }}
        >
          <span class="win-num">{w.window}</span>
          {#if showWindowCmd}<span class="win-cmd">{w.current_command}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}

  <div class="term-wrap" class:hidden={viewMode !== 'terminal'}>
    <div class="xterm-wrap" bind:this={termEl}></div>
    {#if !termAtBottom}
      <button class="scroll-btn" onclick={() => term?.scrollToBottom()}><Icon name="arrow-down" size={16} /></button>
    {/if}
  </div>
  {#if viewMode === 'chat'}
    <ChatView content={paneContent} {command} onSendKeys={(keys) => sendKeys(target, keys, false)} />
  {/if}

  <div class="input-area">
    {#if viewMode === 'terminal' && isMobile}
      <div class="input-bar">
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="shortcut-rows" onmousedown={(e) => e.preventDefault()} ontouchstart={(e) => e.preventDefault()}>
          <div class="shortcuts">
            <button onclick={() => sendSpecial('Escape')}>Esc</button>
            <button onclick={() => sendSpecial('C-d')}>^D</button>
            <button onclick={() => sendSpecial('C-a')}><Icon name="skip-left" size={13} /></button>
            <button onclick={() => sendSpecial('Up')}><Icon name="arrow-up" size={13} /></button>
            <button onclick={() => sendSpecial('C-e')}><Icon name="skip-right" size={13} /></button>
            <button onclick={() => sendSpecial('BSpace')}><Icon name="delete" size={13} /></button>
          </div>
          <div class="shortcuts">
            <button onclick={() => sendSpecial('Tab')}>Tab</button>
            <button onclick={() => sendSpecial('C-c')}>^C</button>
            <button onclick={() => sendSpecial('Left')}><Icon name="arrow-left" size={13} /></button>
            <button onclick={() => sendSpecial('Down')}><Icon name="arrow-down" size={13} /></button>
            <button onclick={() => sendSpecial('Right')}><Icon name="arrow-right" size={13} /></button>
            <button class:sk-active={directMode} ontouchstart={(e) => e.stopPropagation()} onmousedown={(e) => e.stopPropagation()} onclick={() => { directMode = !directMode; if (directMode) requestAnimationFrame(() => inputEl?.focus()); }}><Icon name="chat" size={13} /></button>
          </div>
        </div>
        {#if directMode}
        <div class="cmd-row">
          <span class="prompt">❯</span>
          <textarea
            bind:value={input}
            bind:this={inputEl}
            onkeydown={handleKeydown}
            oninput={autoResize}
            placeholder="command…"
            autocapitalize="off"
            autocomplete="off"
            autocorrect="off"
            spellcheck="false"
            rows="1"
          ></textarea>
          <button class="send" ontouchstart={(e) => { if (input.trim()) e.preventDefault(); }} onmousedown={(e) => { if (input.trim()) e.preventDefault(); }} onclick={handleSubmit}><Icon name={input.trim() ? "arrow-right" : "send"} size={14} /></button>
        </div>
        {/if}
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
            placeholder="message…"
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

  /* Window switcher */
  .win-switcher {
    position: absolute; top: 8px; right: 8px; z-index: 10;
    display: flex; flex-direction: column; gap: 2px;
    background: var(--bg); border: 1px solid var(--border);
    border-radius: 8px; padding: 2px; opacity: 0.85;
    max-height: 50%; overflow-y: auto;
  }
  .win-switcher.expanded { opacity: 1; }
  .win-tab {
    display: flex; align-items: center; gap: 6px;
    padding: 4px 8px; border: none; border-radius: 6px;
    background: none; color: var(--text2); font-size: 11px;
    cursor: pointer; white-space: nowrap;
    -webkit-tap-highlight-color: transparent;
    font-family: 'Maple Mono NF CN', 'SF Mono', Menlo, monospace;
  }
  .win-tab.active { background: var(--accent-bg); color: var(--accent); }
  .win-tab:active { background: var(--surface2); }
  .win-num { font-weight: 600; min-width: 14px; text-align: center; }
  .win-cmd { color: var(--text3); font-size: 10px; max-width: 120px; overflow: hidden; text-overflow: ellipsis; }

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
    font-family: 'Maple Mono NF CN', 'SF Mono', Menlo, monospace;
    font-size: 10px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status-pct {
    display: flex;
    align-items: center;
    gap: 5px;
    font-family: 'Maple Mono NF CN', 'SF Mono', Menlo, monospace;
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

  .scroll-btn {
    position: absolute;
    bottom: 12px;
    right: 16px;
    width: 36px; height: 36px;
    background: var(--bg);
    border: 1px solid var(--input-border);
    border-radius: 50%;
    color: var(--accent);
    font-size: 16px;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 5;
    -webkit-tap-highlight-color: transparent;
    box-shadow: 0 2px 8px rgba(0, 0, 0, 0.4);
  }
  .scroll-btn:active { transform: scale(0.9); }

  .input-area {
    flex-shrink: 0;
    padding: 0 10px 10px;
    padding-bottom: max(10px, env(safe-area-inset-bottom));
  }
  :global(html.keyboard-open) .input-area { padding: 0 4px 2px; }

  .input-bar {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }
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
    font-family: 'Maple Mono NF CN', 'SF Mono', Menlo, monospace;
    font-weight: 500;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    display: flex; align-items: center; justify-content: center;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2), 0 1px 0 rgba(255, 255, 255, 0.04) inset;
  }
  .shortcuts button.sk-empty {
    visibility: hidden;
  }
  .shortcuts button.sk-active {
    background: var(--accent-bg);
    color: var(--accent);
    border-color: var(--accent);
  }
  .shortcuts button:active {
    background: var(--accent-bg);
    color: var(--accent);
    border-color: var(--accent);
    transform: translateY(1px);
    box-shadow: none;
  }

  .cmd-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .prompt {
    color: var(--accent);
    font-family: 'Maple Mono NF CN', 'SF Mono', Menlo, monospace;
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
    font-family: 'Maple Mono NF CN', 'SF Mono', Menlo, monospace;
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
