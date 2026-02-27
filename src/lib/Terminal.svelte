<script>
  import { subscribe, unsubscribe, setOnPaneOutput, sendCommand, sendKeys } from './ws.js';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';
  import ChatView from './ChatView.svelte';
  import Icon from './Icon.svelte';
  import { detectParser } from './parsers.js';

  let { target, session, command = '', viewMode = 'terminal', onChatSupported = () => {} } = $props();

  let input = $state('');
  let paneContent = $state('');
  let termEl;
  let term;
  let fitAddon;
  let termAtBottom = $state(true);

  let parser = $derived(detectParser(paneContent, command));

  $effect(() => { if (paneContent) onChatSupported(!!parser); });

  let waitingForInput = $derived.by(() => {
    if (!paneContent || !parser) return false;
    return parser.isWaitingForInput(paneContent);
  });

  let statusInfo = $derived.by(() => {
    if (!paneContent || !parser) return null;
    return parser.extractStatus(paneContent);
  });

  $effect(() => {
    term = new Terminal({
      cursorBlink: false,
      cursorStyle: 'bar',
      disableStdin: true,
      fontSize: 14,
      fontFamily: "'SF Mono', Menlo, 'Courier New', monospace",
      theme: {
        background: '#0a0a0f',
        foreground: '#c9d1d9',
        cursor: '#00d4ff',
        selectionBackground: 'rgba(0, 212, 255, 0.18)',
        black: '#0a0a0f',
        brightBlack: '#484848',
        red: '#ff5050',
        brightRed: '#ff6b6b',
        green: '#4ade80',
        brightGreen: '#6ee7a0',
        yellow: '#fbbf24',
        brightYellow: '#fcd34d',
        blue: '#00d4ff',
        brightBlue: '#38bdf8',
        magenta: '#c084fc',
        brightMagenta: '#d8b4fe',
        cyan: '#22d3ee',
        brightCyan: '#67e8f9',
        white: '#c9d1d9',
        brightWhite: '#f1f5f9',
      },
      scrollback: 1000,
      convertEol: true,
    });

    fitAddon = new FitAddon();
    term.loadAddon(fitAddon);
    term.open(termEl);

    // Mobile touch scrolling — proxy touch events to xterm's viewport scrollTop
    const viewport = termEl.querySelector('.xterm-viewport');
    if (viewport) {
      let touchY = 0;
      termEl.addEventListener('touchstart', (e) => {
        touchY = e.touches[0].clientY;
      }, { passive: true });
      termEl.addEventListener('touchmove', (e) => {
        const dy = touchY - e.touches[0].clientY;
        touchY = e.touches[0].clientY;
        viewport.scrollTop += dy;
        e.preventDefault();
      }, { passive: false });
    }

    term.onScroll(() => {
      const buf = term.buffer.active;
      termAtBottom = buf.viewportY >= buf.baseY;
    });

    requestAnimationFrame(() => fitAddon.fit());

    const onResize = () => fitAddon.fit();
    window.addEventListener('resize', onResize);

    setOnPaneOutput((t, content) => {
      if (t === target) {
        paneContent = content;
        const buf = term.buffer.active;
        const atBottom = buf.viewportY >= buf.baseY;
        const prevViewport = buf.viewportY;
        term.reset();
        term.write(content, () => {
          if (!atBottom) {
            term.scrollToLine(Math.min(prevViewport, term.buffer.active.baseY));
          }
        });
      }
    });

    subscribe(target);

    return () => {
      window.removeEventListener('resize', onResize);
      unsubscribe(target);
      setOnPaneOutput(null);
      term.dispose();
    };
  });

  $effect(() => {
    if (viewMode === 'terminal' && fitAddon) {
      requestAnimationFrame(() => fitAddon.fit());
    }
  });

  async function handleSubmit() {
    if (!input.trim()) return;
    try {
      await sendCommand(target, input);
      input = '';
      // Reset textarea height
      const ta = document.querySelector('.chat-input-bar textarea');
      if (ta) ta.style.height = 'auto';
    } catch (_) {}
  }

  async function handleKeydown(e) {
    if (e.key === 'Enter' && !e.shiftKey) {
      e.preventDefault();
      await handleSubmit();
    }
  }

  function autoResize(e) {
    const el = e.target;
    el.style.height = 'auto';
    el.style.height = Math.min(el.scrollHeight, 120) + 'px';
  }

  async function sendSpecial(key) {
    try {
      await sendKeys(target, key, false);
    } catch (_) {}
  }
</script>

<div class="terminal">
  <div class="term-wrap" class:hidden={viewMode !== 'terminal'}>
    <div class="xterm-wrap" bind:this={termEl}></div>
    {#if !termAtBottom}
      <button class="scroll-btn" onclick={() => term?.scrollToBottom()}><Icon name="arrow-down" size={16} /></button>
    {/if}
    <div class="status-bar">{target}{#if command} · <span class:kiro={/^kiro/i.test(command)}>{command}</span>{/if}</div>
  </div>
  {#if viewMode === 'chat'}
    {#if statusInfo?.pct !== null || statusInfo?.tool}
      <div class="status-line">
        <span class="status-left">{target}{#if command} · <span class:kiro={/^kiro/i.test(command)}>{command}</span>{/if}</span>
        {#if statusInfo.pct !== null}
          <span class="status-pct">
            <span class="pct-bar"><span class="pct-fill" style="width:{statusInfo.pct}%;background:{statusInfo.pct < 50 ? '#4ade80' : statusInfo.pct < 80 ? '#fbbf24' : '#ff5050'}"></span></span>
            <span style="color:{statusInfo.pct < 50 ? '#4ade80' : statusInfo.pct < 80 ? '#fbbf24' : '#ff5050'}">{statusInfo.pct}%</span>
          </span>
        {/if}
      </div>
    {/if}
    <ChatView content={paneContent} />
  {/if}

  <div class="input-area">
    {#if viewMode === 'terminal'}
      <div class="input-bar">
        <div class="shortcuts">
          <button onclick={() => sendSpecial('C-c')}>⌃C</button>
          <button onclick={() => sendSpecial('C-d')}>⌃D</button>
          <button onclick={() => sendSpecial('C-z')}>⌃Z</button>
          <button onclick={() => sendSpecial('Tab')}>⇥</button>
          <button onclick={() => sendSpecial('Up')}>↑</button>
          <button onclick={() => sendSpecial('Down')}>↓</button>
        </div>
        <div class="cmd-row">
          <span class="prompt">❯</span>
          <input
            type="text"
            bind:value={input}
            onkeydown={handleKeydown}
            placeholder="command…"
            autocapitalize="off"
            autocomplete="off"
            autocorrect="off"
            spellcheck="false"
          />
          <button class="send" onclick={handleSubmit}><Icon name="send" size={14} /></button>
        </div>
      </div>
    {:else}
      <div class="input-bar chat-input-bar">
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
    background: #0a0a0f;
  }

  .status-line {
    display: flex;
    align-items: center;
    gap: 12px;
    padding: 6px 20px;
    flex-shrink: 0;
    background: rgba(255, 255, 255, 0.02);
    border-bottom: 1px solid rgba(255, 255, 255, 0.04);
    font-size: 12px;
    color: rgba(226, 232, 240, 0.45);
  }
  .status-left .kiro { color: #c084fc; }
  .status-tool { color: #c084fc; }
  .status-left {
    font-family: 'SF Mono', Menlo, monospace;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .status-bar {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    padding: 2px 8px;
    font-family: 'SF Mono', Menlo, monospace;
    font-size: 10px;
    color: rgba(226, 232, 240, 0.3);
    background: rgba(10, 10, 15, 0.7);
    pointer-events: none;
  }
  .status-bar .kiro { color: #c084fc; }
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
    background: rgba(255, 255, 255, 0.12);
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

  .xterm-wrap {
    height: 100%;
    padding: 4px 6px;
  }
  .xterm-wrap :global(.xterm) {
    height: 100%;
  }

  .scroll-btn {
    position: absolute;
    bottom: 12px;
    right: 16px;
    width: 36px; height: 36px;
    border: 1px solid rgba(255, 255, 255, 0.1);
    border-radius: 50%;
    background: rgba(12, 12, 20, 0.85);
    backdrop-filter: blur(10px);
    color: #00d4ff;
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
    background: rgba(255, 255, 255, 0.04);
    backdrop-filter: blur(20px);
    -webkit-backdrop-filter: blur(20px);
    border: 1px solid rgba(255, 255, 255, 0.07);
    border-radius: 14px;
    padding: 8px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .shortcuts {
    display: flex;
    gap: 5px;
    overflow-x: auto;
    -webkit-overflow-scrolling: touch;
    scrollbar-width: none;
  }
  .shortcuts::-webkit-scrollbar { display: none; }

  .shortcuts button {
    padding: 5px 10px;
    border: 1px solid rgba(255, 255, 255, 0.08);
    border-radius: 7px;
    background: rgba(255, 255, 255, 0.04);
    color: rgba(226, 232, 240, 0.45);
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
    background: rgba(0, 212, 255, 0.12);
    color: #00d4ff;
    border-color: rgba(0, 212, 255, 0.2);
    transform: translateY(1px);
    box-shadow: none;
  }

  .cmd-row {
    display: flex;
    align-items: center;
    gap: 8px;
  }

  .prompt {
    color: #00d4ff;
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
    color: #e2e8f0;
    font-family: 'SF Mono', Menlo, monospace;
    font-size: 15px;
    outline: none;
    -webkit-appearance: none;
  }
  .cmd-row input::placeholder { color: rgba(226, 232, 240, 0.2); }

  .cmd-row textarea {
    flex: 1;
    min-width: 0;
    padding: 8px 0;
    border: none;
    background: transparent;
    color: #e2e8f0;
    font-family: 'SF Mono', Menlo, monospace;
    font-size: 15px;
    outline: none;
    -webkit-appearance: none;
    resize: none;
    max-height: 120px;
    overflow-y: auto;
    line-height: 1.4;
    scrollbar-width: thin;
    scrollbar-color: rgba(255,255,255,0.1) transparent;
  }
  .cmd-row textarea::placeholder { color: rgba(226, 232, 240, 0.2); }

  .stop-btn {
    width: 34px; height: 34px;
    border: none;
    border-radius: 9px;
    background: rgba(255, 80, 80, 0.12);
    color: #ff5050;
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
    background: rgba(255, 80, 80, 0.25);
    transform: scale(0.92);
  }

  .send {
    width: 34px; height: 34px;
    border: none;
    border-radius: 9px;
    background: linear-gradient(135deg, #00d4ff 0%, #0099cc 100%);
    color: #000;
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
