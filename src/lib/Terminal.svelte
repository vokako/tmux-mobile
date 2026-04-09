<script>
  import { subscribe, unsubscribe, setOnPaneOutput, sendCommand, sendKeys, paneCommand, listPanes, capturePane, resizePane } from './ws.js';
  import { Terminal } from '@xterm/xterm';
  import ChatView from './ChatView.svelte';
  import Icon from './Icon.svelte';
  import { detectParser } from './parsers.js';

  let { target, session, command: initialCommand = '', viewMode = 'terminal', fontSize = 14, onChatSupported = () => {}, onSwitchPane = null } = $props();

  let input = $state('');
  let paneContent = $state('');
  let command = $state(initialCommand);
  $effect(() => { command = initialCommand; });
  let termEl;
  let term;
  let termAtBottom = $state(true);
  let toastMsg = $state('');
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
    window.dispatchEvent(new Event('terminal-refit'));
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
  let showWindowCmd = $state(localStorage.getItem('tmux_winswitcher') === '1');
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

  // Calculate optimal cols/rows — use max height to prevent tmux resize on keyboard open
  let maxContainerH = 0;
  function calcFit() {
    if (!term || !termEl) return null;
    const core = term._core;
    const cellW = core?._renderService?.dimensions?.css?.cell?.width || (term.options.fontSize * 0.6);
    const cellH = core?._renderService?.dimensions?.css?.cell?.height || (term.options.fontSize * 1.2);
    const w = termEl.clientWidth;
    const clientH = termEl.clientHeight;
    if (clientH > maxContainerH) maxContainerH = clientH;
    const h = isMobile ? Math.max(clientH, maxContainerH) : clientH;
    if (!w || !h || !cellW || !cellH) return null;
    return { cols: Math.max(2, Math.floor(w / cellW)), rows: Math.max(1, Math.floor(h / cellH)) };
  }

  let touchScrolling = false; // set by touch handler, pauses content updates

  // Count lines without allocating a split array
  function countLines(s) { let n = 1; for (let i = 0; i < s.length; i++) if (s[i] === '\n') n++; return n; }

  // Write content + position cursor in xterm.js
  function adaptColorsForLight(text) {
    if (theme !== 'light') return text;
    // Invert all RGB colors: both foreground (38;2) and background (48;2)
    return text.replace(/\x1b\[(3|4)8;2;(\d+);(\d+);(\d+)m/g, (m, type, r, g, b) => {
      return `\x1b[${type}8;2;${255 - +r};${255 - +g};${255 - +b}m`;
    });
  }

  function writeToXterm(content, cursor) {
    if (!term || touchScrolling) return;
    if (cursor?.w && cursor?.h && (term.cols !== cursor.w || term.rows !== cursor.h)) {
      term.resize(cursor.w, cursor.h);
    }
    const buf = term.buffer.active;
    const atBottom = buf.viewportY >= buf.baseY;
    const prevViewport = buf.viewportY;

    const N = countLines(content);
    const trailing = cursor?.t || 0;

    // trailing = empty lines trimmed from capture, need to add back
    let cursorSeq = '';
    let afterPad = '';
    let topPad = '';
    if (cursor) {
      const paneStart = Math.max(0, N + trailing - cursor.h);
      const cursorLine = paneStart + cursor.y;
      // Only pad enough after content to reach cursor, not all trailing
      const needAfter = Math.max(0, cursorLine + 1 - N);
      afterPad = needAfter > 0 ? '\n'.repeat(needAfter) : '';
      // Total visible content lines
      const contentLines = N + needAfter;
      // Pad top so visible pane area fills the screen from bottom
      if (contentLines < term.rows) {
        topPad = '\n'.repeat(term.rows - contentLines);
      }
      const topPadCount = topPad.length;
      const totalWritten = topPadCount + contentLines;
      const sb = Math.max(0, totalWritten - term.rows);
      const row = topPadCount + cursorLine - sb + 1;
      if (row > 0 && row <= term.rows) {
        cursorSeq = `\x1b[${row};${cursor.x + 1}H`;
      }
    }

    if (buf.baseY > 0) term.clear();
    term.write('\x1b[?25l\x1b[2J\x1b[H' + topPad + adaptColorsForLight(content) + afterPad + cursorSeq + '\x1b[?25h', () => {
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
    termEl.style.background = getTermTheme().background;

    // Forward keyboard input to tmux — skip when input box is open
    term.onData(data => {
      // Filter xterm.js terminal response sequences that leak through onData.
      // These are generated when pane content contains query sequences (e.g. \x1b[6n).
      // Without filtering, they create a feedback loop: response→tmux→capture→xterm→response.
      if (/^\x1b\[[\?>=]?[\d;]*c$/.test(data)) return; // DA1/DA2/DA3
      if (/^\x1b\[\d+;\d+R$/.test(data)) return;        // DSR cursor position
      if (/^\x1b\[\d+n$/.test(data)) return;             // DSR device status
      // Mobile: force-clear xterm's hidden textarea after each input to prevent
      // accumulation. Auto-paired quotes (typing " inserts "" with cursor in middle)
      // break xterm.js's textarea clearing, causing subsequent chars to accumulate.
      if (isMobile) {
        requestAnimationFrame(() => {
          const ta = termEl?.querySelector('.xterm-helper-textarea');
          if (ta && ta.value) ta.value = '';
        });
      }
      sendKeys(target, data, true).catch(() => {});
    });
    // Block xterm from processing keys when input box is open
    term.attachCustomKeyEventHandler(() => true);

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
    let lastMoveTime = 0, momentumId = null, totalDist = 0;
    let velocitySamples = []; // recent velocity samples for smoothing
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
        setTimeout(() => { if (term) term.options.disableStdin = false; }, 300);
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
      velocitySamples = [];
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
          const scrollTarget = scrollbarStartViewport + (deltaY / trackH) * totalScroll;
          term.scrollToLine(Math.max(0, Math.min(totalScroll, Math.round(scrollTarget))));
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
      // Track velocity in px/ms, keep last 5 samples within 100ms
      velocitySamples.push({ v: dy / dt, t: now });
      while (velocitySamples.length > 5 || (velocitySamples.length > 1 && now - velocitySamples[0].t > 100)) {
        velocitySamples.shift();
      }
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
        const maxPxPerFrame = 240;
        const cappedPx = Math.max(-maxPxPerFrame, Math.min(maxPxPerFrame, avgVelocity * 16));
        let v = cappedPx / lh;
        if (Math.abs(v) > 0.1) {
          let acc = 0;
          const friction = 0.95;
          const coast = () => {
            v *= friction;
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
        } else {
          setTimeout(endTouchScroll, 500);
        }
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
        const containerH = termEl.parentElement?.clientHeight || 0;
        const core = term._core;
        const cellH = core?._renderService?.dimensions?.css?.cell?.height || (term.options.fontSize * 1.2);
        const terminalH = term.rows * cellH;
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
        const N = countLines(lastContent);
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

  // Long-press repeat for shortcut keys
  let repeatTimer = null;
  let repeatInterval = null;

  function startRepeat(key) {
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


</script>

<div class="terminal">
  {#if toastMsg}
    <div class="toast">{toastMsg}</div>
  {/if}
  {#if windows.length > 1}
    {#if showWindowCmd}
      <div class="win-switcher expanded">
        <button class="win-collapse" onclick={() => { showWindowCmd = false; localStorage.setItem('tmux_winswitcher', '0'); }}><Icon name="arrow-up" size={12} /></button>
        {#each windows as w}
          {@const info = (w.pane_title || '') + ' ' + (w.current_command || '')}
          {@const aiTag = info.match(/kiro/i) ? 'Kiro' : info.match(/claude/i) ? 'Claude' : info.match(/openclaw/i) ? 'OpenClaw' : ''}
          <button
            class="win-tab"
            class:active={String(w.window) === currentWindow}
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
          >
            {#if aiTag === 'Kiro'}
              <img class="win-ai-icon" src="/assets/kiro.svg" alt="Kiro" />
            {:else if aiTag === 'Claude'}
              <img class="win-ai-icon claude" src="/assets/claude.svg" alt="Claude" />
            {:else if aiTag === 'OpenClaw'}
              <img class="win-ai-icon" src="/assets/openclaw.svg" alt="OpenClaw" />
            {:else}
              <span class="win-cmd">{w.current_command || w.window_name}</span>
            {/if}
          </button>
        {/each}
      </div>
    {:else}
      {@const cur = windows.find(w => String(w.window) === currentWindow)}
      {@const curAi = cur ? ((cur.pane_title || '') + ' ' + (cur.current_command || '')).match(/kiro/i) ? 'Kiro' : ((cur.pane_title || '') + ' ' + (cur.current_command || '')).match(/claude/i) ? 'Claude' : ((cur.pane_title || '') + ' ' + (cur.current_command || '')).match(/openclaw/i) ? 'OpenClaw' : '' : ''}
      <button class="win-toggle" onclick={() => { showWindowCmd = true; localStorage.setItem('tmux_winswitcher', '1'); }}>
        {#if curAi === 'Kiro'}
          <img class="win-ai-icon" src="/assets/kiro.svg" alt="Kiro" />
        {:else if curAi === 'Claude'}
          <img class="win-ai-icon claude" src="/assets/claude.svg" alt="Claude" />
        {:else if curAi === 'OpenClaw'}
          <img class="win-ai-icon" src="/assets/openclaw.svg" alt="OpenClaw" />
        {:else}
          <span class="win-toggle-cmd">{cur?.current_command || cur?.window_name || '?'}</span>
        {/if}
      </button>
    {/if}
  {/if}

  <div class="term-wrap" class:hidden={viewMode !== 'terminal'}>
    <div class="xterm-wrap" bind:this={termEl}></div>
    {#if !termAtBottom}
      <button class="scroll-btn" onclick={() => term?.scrollToBottom()}><Icon name="arrow-down" size={16} /></button>
    {/if}
  </div>
  {#if viewMode === 'chat'}
    <ChatView content={paneContent} {command} {fontSize} onSendKeys={(keys) => sendKeys(target, keys, false)} />
  {/if}

  <div class="input-area">
    {#if viewMode === 'terminal' && isMobile}
      <div class="input-bar">
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="shortcut-rows" onpointerdown={(e) => e.preventDefault()} ontouchend={(e) => { e.preventDefault(); stopRepeat(); }} ontouchcancel={stopRepeat} oncontextmenu={(e) => e.preventDefault()} onmouseup={stopRepeat}>
          <div class="shortcuts">
            <button ontouchstart={(e) => { e.preventDefault(); startRepeat('Escape'); }}>Esc</button>
            <button ontouchstart={(e) => { e.preventDefault(); startRepeat('C-d'); }}>^D</button>
            <button ontouchstart={(e) => { e.preventDefault(); startRepeat('C-a'); }}><Icon name="skip-left" size={13} /></button>
            <button ontouchstart={(e) => { e.preventDefault(); startRepeat('Up'); }}><Icon name="arrow-up" size={13} /></button>
            <button ontouchstart={(e) => { e.preventDefault(); startRepeat('C-e'); }}><Icon name="skip-right" size={13} /></button>
            <button ontouchstart={(e) => { e.preventDefault(); startRepeat('BSpace'); }}><Icon name="delete" size={13} /></button>
          </div>
          <div class="shortcuts">
            <button ontouchstart={(e) => { e.preventDefault(); startRepeat('Tab'); }}>Tab</button>
            <button ontouchstart={(e) => { e.preventDefault(); startRepeat('C-c'); }}>^C</button>
            <button ontouchstart={(e) => { e.preventDefault(); startRepeat('Left'); }}><Icon name="arrow-left" size={13} /></button>
            <button ontouchstart={(e) => { e.preventDefault(); startRepeat('Down'); }}><Icon name="arrow-down" size={13} /></button>
            <button ontouchstart={(e) => { e.preventDefault(); startRepeat('Right'); }}><Icon name="arrow-right" size={13} /></button>
            <button onpointerdown={(e) => { e.stopPropagation(); e.stopImmediatePropagation(); requestAnimationFrame(() => { const ta = termEl?.querySelector('.xterm-helper-textarea'); if (ta && document.activeElement === ta) { ta.blur(); } else if (ta) { ta.focus(); } }); }}><Icon name="keyboard" size={13} /></button>
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
  .win-toggle {
    position: absolute; top: 8px; right: 8px; z-index: 10;
    width: auto; min-width: 36px; height: 36px; border: 1px solid var(--border);
    border-radius: 10px; background: rgba(10,10,15,0.85); color: var(--accent);
    cursor: pointer; display: flex; align-items: center; justify-content: center; padding: 0 8px;
    -webkit-tap-highlight-color: transparent;
    backdrop-filter: blur(8px); -webkit-backdrop-filter: blur(8px);
  }
  @supports (backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px)) {
    .win-toggle { background: rgba(10,10,15,0.45); }
    :global(html[data-theme="light"]) .win-toggle { background: rgba(245,245,247,0.45); }
  }
  :global(html[data-theme="light"]) .win-toggle { background: rgba(245,245,247,0.85); }
  .win-toggle:active { background: var(--accent-bg); color: var(--accent); }
  .win-toggle-cmd { font-size: 10px; font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace; max-width: 60px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .win-badge {
    font-size: 11px; font-weight: 700;
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
  }
  .win-switcher {
    position: absolute; top: 8px; right: 8px; z-index: 10;
    display: flex; flex-direction: column; gap: 2px;
    background: rgba(10,10,15,0.85); border: 1px solid var(--border);
    border-radius: 10px; padding: 4px;
    max-height: 50%; overflow-y: auto;
    backdrop-filter: blur(12px); -webkit-backdrop-filter: blur(12px);
    box-shadow: 0 4px 16px rgba(0,0,0,0.3);
  }
  @supports (backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px)) {
    .win-switcher { background: rgba(10,10,15,0.5); }
    :global(html[data-theme="light"]) .win-switcher { background: rgba(245,245,247,0.5); }
  }
  :global(html[data-theme="light"]) .win-switcher { background: rgba(245,245,247,0.85); }
  .win-collapse {
    padding: 4px; border: none; border-radius: 6px;
    background: none; color: var(--text3); cursor: pointer;
    display: flex; align-items: center; justify-content: center;
    -webkit-tap-highlight-color: transparent;
  }
  .win-collapse:active { color: var(--accent); }
  .win-tab {
    display: flex; align-items: center; justify-content: center; gap: 8px;
    padding: 7px 6px; border: none; border-radius: 8px;
    background: none; color: var(--text2); font-size: 12px;
    cursor: pointer; white-space: nowrap;
    -webkit-tap-highlight-color: transparent;
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
  }
  .win-tab.active { background: var(--accent-bg); color: var(--accent); }
  .win-tab:active { background: var(--surface2); }
  .win-num { font-weight: 600; min-width: 14px; text-align: center; }
  .win-cmd { color: inherit; font-size: 10px; max-width: 100px; overflow: hidden; text-overflow: ellipsis; }
  .win-ai-icon { height: 14px; width: auto; }
  .win-tab .win-ai-icon { opacity: 0.5; }
  .win-tab.active .win-ai-icon { opacity: 1; }
  .win-ai-icon.claude { filter: brightness(0.9); }

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
    font-family: 'Maple Mono NF CN', 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: 10px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status-pct {
    display: flex;
    align-items: center;
    gap: 5px;
    font-family: 'Maple Mono NF CN', 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
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
    font-family: 'Maple Mono NF CN', 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    font-weight: 500;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    display: flex; align-items: center; justify-content: center;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2), 0 1px 0 rgba(255, 255, 255, 0.04) inset;
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
    font-family: 'Maple Mono NF CN', 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
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
    font-family: 'Maple Mono NF CN', 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
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
