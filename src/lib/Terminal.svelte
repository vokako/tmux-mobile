<script>
  import { subscribe, unsubscribe, setOnPaneOutput, sendCommand, sendKeys, paneCommand, listPanes, resizePane } from './ws.js';
  import Convert from 'ansi-to-html';
  import ChatView from './ChatView.svelte';
  import Icon from './Icon.svelte';
  import { detectParser } from './parsers.js';

  let { target, session, command: initialCommand = '', viewMode = 'terminal', onChatSupported = () => {}, onSwitchPane = null } = $props();

  let input = $state('');
  let paneContent = $state('');
  let command = $state('');
  $effect(() => { command = initialCommand; });
  let termEl;
  let termAtBottom = $state(true);
  let measureEl;

  // Debounce timer for resize
  let resizeTimer;

  function doResize() {
    if (!termEl || !measureEl) return;
    const charW = measureEl.getBoundingClientRect().width;
    if (!charW) return;
    // termEl has 10px padding on each side (padding: 8px 10px)
    const innerW = termEl.clientWidth - 20;
    const innerH = termEl.clientHeight - 16;
    const cols = Math.max(40, Math.floor(innerW / charW));
    const rows = Math.max(10, Math.floor(innerH / (13 * 1.35))); // font-size 13 × line-height 1.35
    resizePane(target, cols, rows).catch(() => {});
  }

  function scheduleResize() {
    clearTimeout(resizeTimer);
    resizeTimer = setTimeout(doResize, 300);
  }
  let theme = $state(document.documentElement.getAttribute('data-theme') || 'dark');

  $effect(() => {
    const obs = new MutationObserver(() => {
      theme = document.documentElement.getAttribute('data-theme') || 'dark';
    });
    obs.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    return () => obs.disconnect();
  });

  const darkConvert = new Convert({ newline: true, escapeXML: true });
  const lightConvert = new Convert({ newline: true, escapeXML: true,
    colors: {
      0: '#1a1a2e', 1: '#dc2626', 2: '#16a34a', 3: '#ca8a04',
      4: '#2563eb', 5: '#9333ea', 6: '#0891b2', 7: '#4b5563',
      8: '#6b7280', 9: '#ef4444', 10: '#22c55e', 11: '#eab308',
      12: '#3b82f6', 13: '#a855f7', 14: '#06b6d4', 15: '#1a1a2e',
    }
  });

  let termHtml = $state('');
  let lastRendered = '';
  $effect(() => {
    const content = paneContent;
    const t = theme;
    if (content === lastRendered) return;
    lastRendered = content;
    let html = (t === 'light' ? lightConvert : darkConvert).toHtml(content);
    if (t === 'light') {
      html = html.replace(/color:#(fff|ffffff|eee|eeeeee|ddd|dddddd|ccc|cccccc|bbb|bbbbbb|aaa|aaaaaa|AAA|FFF)\b/gi,
        'color:#4b5563');
      // Strip all dark backgrounds
      html = html.replace(/background-color:#[0-4][0-9a-f]{5}\b/gi,
        'background-color:transparent');
      html = html.replace(/background-color:#[0-9a-f]{3}\b/gi, (m) => {
        const hex = m.split('#')[1];
        const r = parseInt(hex[0], 16), g = parseInt(hex[1], 16), b = parseInt(hex[2], 16);
        return (r + g + b < 24) ? 'background-color:transparent' : m;
      });
    }
    termHtml = html;
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

  function checkAtBottom() {
    if (!termEl) return;
    const el = termEl;
    termAtBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
  }

  function scrollToBottom() {
    if (termEl) {
      termEl.scrollTop = termEl.scrollHeight;
      termAtBottom = true;
    }
  }

  // Scroll to bottom on tab switch or first render
  $effect(() => {
    viewMode; // track tab switches
    requestAnimationFrame(scrollToBottom);
  });

  // Auto-scroll when new content arrives and user is at bottom
  $effect(() => {
    termHtml; // track
    if (termAtBottom) requestAnimationFrame(scrollToBottom);
    // Also recheck atBottom after content change
    requestAnimationFrame(checkAtBottom);
  });

  // Scroll to bottom when keyboard opens/closes
  function scheduleScroll() {
    // Always scroll + recheck, keyboard changes layout so old atBottom is stale
    for (const ms of [50, 150, 300, 500]) setTimeout(() => {
      scrollToBottom();
      checkAtBottom();
    }, ms);
  }
  $effect(() => {
    const vv = window.visualViewport;
    if (vv) vv.addEventListener('resize', scheduleScroll);
    window.addEventListener('resize', scheduleScroll);
    // Also listen for focus on any textarea in this component
    const onFocus = (e) => { if (e.target.tagName === 'TEXTAREA') scheduleScroll(); };
    document.addEventListener('focusin', onFocus);
    return () => {
      if (vv) vv.removeEventListener('resize', scheduleScroll);
      window.removeEventListener('resize', scheduleScroll);
      document.removeEventListener('focusin', onFocus);
    };
  });

  // Resize pane when target changes or on viewport resize
  $effect(() => {
    target; // track target changes (switching panes)
    // Initial resize after mount — wait for layout
    const t = setTimeout(doResize, 200);
    return () => clearTimeout(t);
  });

  $effect(() => {
    const vv = window.visualViewport;
    const onVpResize = () => scheduleResize();
    if (vv) vv.addEventListener('resize', onVpResize);
    else window.addEventListener('resize', onVpResize);
    return () => {
      if (vv) vv.removeEventListener('resize', onVpResize);
      else window.removeEventListener('resize', onVpResize);
    };
  });

  $effect(() => {
    let lastContent = '';
    let first = true;
    setOnPaneOutput((t, content) => {
      if (t !== target || content === lastContent) return;
      lastContent = content;
      paneContent = content;
      if (first) { first = false; requestAnimationFrame(scrollToBottom); }
    });

    subscribe(target);

    return () => {
      unsubscribe(target);
      setOnPaneOutput(null);
    };
  });

  async function handleSubmit() {
    if (viewMode === 'chat') {
      // Chat: always send text + Enter
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
      // Empty: send Enter, blur to dismiss keyboard
      await sendKeys(target, 'Enter', false).catch(() => {});
      document.activeElement?.blur();
      return;
    }
    try {
      // Has text: send as literal keys, no Enter, keep keyboard open
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
</script>

<div class="terminal">
  {#if windows.length > 1}
    <div class="win-switcher" class:expanded={showWindowCmd}>
      {#each windows as w}
        <button
          class="win-tab"
          class:active={String(w.window) === currentWindow}
          onclick={(e) => {
            e.stopPropagation();
            if (String(w.window) === currentWindow) { showWindowCmd = !showWindowCmd; }
            else if (onSwitchPane) { onSwitchPane(`${w.session}:${w.window}.${w.pane}`, w.current_command); showWindowCmd = false; }
          }}
        >
          <span class="win-num">{w.window}</span>
          {#if showWindowCmd}<span class="win-cmd">{w.current_command}</span>{/if}
        </button>
      {/each}
    </div>
  {/if}
  <!-- Hidden span to measure monospace character width -->
  <span class="char-measure" bind:this={measureEl} aria-hidden="true">M</span>

  <div class="term-wrap" class:hidden={viewMode !== 'terminal'}>
    <div class="ansi-output" bind:this={termEl} onscroll={checkAtBottom}>{@html termHtml}</div>
    {#if !termAtBottom}
      <button class="scroll-btn" onclick={scrollToBottom}><Icon name="arrow-down" size={16} /></button>
    {/if}
  </div>
  {#if viewMode === 'chat'}
    <ChatView content={paneContent} {command} onSendKeys={(keys) => sendKeys(target, keys, false)} />
  {/if}

  <div class="input-area">
    {#if viewMode === 'terminal'}
      <!-- svelte-ignore a11y_no_static_element_interactions -->
      <div class="shortcut-rows" onmousedown={(e) => e.preventDefault()} ontouchstart={(e) => e.preventDefault()}>
        <div class="shortcuts">
          <button onclick={() => sendSpecial('Tab')}>Tab</button>
          <button onclick={() => sendSpecial('C-c')}>^C</button>
          <button onclick={() => sendSpecial('C-a')}><Icon name="skip-left" size={13} /></button>
          <button onclick={() => sendSpecial('Up')}><Icon name="arrow-up" size={13} /></button>
          <button onclick={() => sendSpecial('C-e')}><Icon name="skip-right" size={13} /></button>
          <button onclick={() => sendSpecial('BSpace')}><Icon name="delete" size={13} /></button>
        </div>
        <div class="shortcuts">
          <button onclick={() => sendKeys(target, '/', true).catch(() => {})}>/</button>
          <button onclick={() => sendSpecial('C-d')}>^D</button>
          <button onclick={() => sendSpecial('Left')}><Icon name="arrow-left" size={13} /></button>
          <button onclick={() => sendSpecial('Down')}><Icon name="arrow-down" size={13} /></button>
          <button onclick={() => sendSpecial('Right')}><Icon name="arrow-right" size={13} /></button>
          <button class="sk-empty" aria-hidden="true"></button>
        </div>
      </div>
      <div class="input-bar">
        <div class="input-status">
          <span class="status-left">{target}{#if command} · <span class:kiro={/^kiro/i.test(command)}>{command}</span>{/if}</span>
        </div>
        <div class="cmd-row">
          <span class="prompt">❯</span>
          <textarea
            bind:value={input}
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
      </div>
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
    font-family: 'SF Mono', Menlo, monospace;
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
    font-family: 'SF Mono', Menlo, monospace;
    font-size: 10px;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status-pct {
    display: flex;
    align-items: center;
    gap: 5px;
    font-family: 'SF Mono', Menlo, monospace;
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
    position: relative;
  }
  .term-wrap.hidden {
    position: absolute;
    left: -9999px;
    visibility: hidden;
  }

  .char-measure {
    position: absolute;
    visibility: hidden;
    pointer-events: none;
    font-family: 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: 13px;
    line-height: 1.35;
    white-space: pre;
  }

  .ansi-output {
    height: 100%;
    padding: 8px 10px;
    overflow-y: auto;
    overscroll-behavior: contain;
    -webkit-overflow-scrolling: touch;
    font-family: 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: 13px;
    line-height: 1.35;
    white-space: pre-wrap;
    word-break: break-all;
    color: var(--text);
    scrollbar-width: thin;
    scrollbar-color: var(--border) transparent;
    contain: content;
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

  .input-bar {
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 14px;
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
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
    font-family: 'SF Mono', Menlo, monospace;
    font-weight: 500;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    display: flex; align-items: center; justify-content: center;
    box-shadow: 0 1px 2px rgba(0, 0, 0, 0.2), 0 1px 0 rgba(255, 255, 255, 0.04) inset;
  }
  .shortcuts button.sk-empty {
    visibility: hidden;
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
    font-family: 'SF Mono', Menlo, monospace;
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
    font-family: 'SF Mono', Menlo, monospace;
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
