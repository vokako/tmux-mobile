<script>
  import { subscribe, unsubscribe, setOnPaneOutput, sendCommand, sendKeys, paneCommand } from './ws.js';
  import Convert from 'ansi-to-html';
  import ChatView from './ChatView.svelte';
  import Icon from './Icon.svelte';
  import { detectParser } from './parsers.js';

  let { target, session, command: initialCommand = '', viewMode = 'terminal', onChatSupported = () => {} } = $props();

  let input = $state('');
  let paneContent = $state('');
  let command = $state(initialCommand);
  let termEl;
  let termAtBottom = $state(true);
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
      html = html.replace(/background-color:#(000|000000)\b/gi,
        'background-color:transparent');
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

  function checkAtBottom() {
    if (!termEl) return;
    const el = termEl;
    termAtBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 30;
  }

  function scrollToBottom() {
    if (termEl) termEl.scrollTop = termEl.scrollHeight;
  }

  // Auto-scroll when new content arrives and user is at bottom
  $effect(() => {
    termHtml; // track
    if (termAtBottom) requestAnimationFrame(scrollToBottom);
  });

  $effect(() => {
    let lastContent = '';
    setOnPaneOutput((t, content) => {
      if (t !== target || content === lastContent) return;
      lastContent = content;
      paneContent = content;
    });

    subscribe(target);

    return () => {
      unsubscribe(target);
      setOnPaneOutput(null);
    };
  });

  async function handleSubmit() {
    if (!input.trim()) {
      await sendKeys(target, 'Enter', false).catch(() => {});
      return;
    }
    try {
      await sendCommand(target, input);
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
      <div class="input-bar">
        <div class="input-status">
          <span class="status-left">{target}{#if command} · <span class:kiro={/^kiro/i.test(command)}>{command}</span>{/if}</span>
        </div>
        <div class="shortcuts">
          <button onclick={() => sendSpecial('C-c')}>^C</button>
          <button onclick={() => sendSpecial('C-d')}>^D</button>
          <button onclick={() => sendSpecial('Tab')}>Tab</button>
          <button onclick={() => sendSpecial('BSpace')}><Icon name="delete" size={13} /></button>
          <button onclick={() => sendSpecial('Left')}><Icon name="arrow-left" size={13} /></button>
          <button onclick={() => sendSpecial('Down')}><Icon name="arrow-down" size={13} /></button>
          <button onclick={() => sendSpecial('Up')}><Icon name="arrow-up" size={13} /></button>
          <button onclick={() => sendSpecial('Right')}><Icon name="arrow-right" size={13} /></button>
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
          <button class="send" onclick={handleSubmit}><Icon name="send" size={14} /></button>
        </div>
      </div>
    {:else}
      <div class="input-bar chat-input-bar">
        <div class="input-status">
          <span class="status-left">{target}{#if command} · <span class:kiro={/^kiro/i.test(command)}>{command}</span>{/if}</span>
          {#if statusInfo?.pct != null}
            <span class="status-pct">
              <span class="pct-bar"><span class="pct-fill" style="width:{statusInfo.pct}%;background:{statusInfo.pct < 50 ? '#4ade80' : statusInfo.pct < 80 ? '#fbbf24' : '#ff5050'}"></span></span>
              <span style="color:{statusInfo.pct < 50 ? '#4ade80' : statusInfo.pct < 80 ? '#fbbf24' : '#ff5050'}">{statusInfo.pct}%</span>
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

  .ansi-output {
    height: 100%;
    padding: 8px 10px;
    overflow-y: auto;
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

  .shortcuts {
    display: flex;
    gap: 3px;
    overflow-x: auto;
    -webkit-overflow-scrolling: touch;
    scrollbar-width: none;
  }
  .shortcuts::-webkit-scrollbar { display: none; }

  .shortcuts button {
    padding: 5px 8px;
    border: 1px solid var(--input-border);
    border-radius: 7px;
    background: var(--input-bg);
    color: var(--text2);
    font-size: 12px;
    font-family: 'SF Mono', Menlo, monospace;
    font-weight: 500;
    cursor: pointer;
    white-space: nowrap;
    flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
    transition: all 0.15s ease;
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
    font-family: 'SF Mono', Menlo, monospace;
    font-size: 15px;
    font-weight: 600;
    flex-shrink: 0;
    filter: drop-shadow(0 0 4px rgba(0, 212, 255, 0.3));
  }

  .cmd-row input {
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
  }
  .cmd-row input::placeholder { color: var(--text3); }

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
