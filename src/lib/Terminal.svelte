<script>
  import { subscribe, unsubscribe, setOnPaneOutput, sendCommand, sendKeys } from './ws.js';
  import { Terminal } from '@xterm/xterm';
  import { FitAddon } from '@xterm/addon-fit';

  let { target, session } = $props();

  let input = $state('');
  let termEl;
  let term;
  let fitAddon;

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

    requestAnimationFrame(() => fitAddon.fit());

    const onResize = () => fitAddon.fit();
    window.addEventListener('resize', onResize);

    setOnPaneOutput((t, content) => {
      if (t === target) {
        term.reset();
        term.write(content);
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

  async function handleSubmit() {
    if (!input.trim()) return;
    try {
      await sendCommand(target, input);
      input = '';
    } catch (_) {}
  }

  async function handleKeydown(e) {
    if (e.key === 'Enter') {
      e.preventDefault();
      await handleSubmit();
    }
  }

  async function sendSpecial(key) {
    try {
      await sendKeys(target, key, false);
    } catch (_) {}
  }
</script>

<div class="terminal">
  <div class="xterm-wrap" bind:this={termEl}></div>

  <div class="input-area">
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
        <button class="send" onclick={handleSubmit}>⏎</button>
      </div>
    </div>
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

  .xterm-wrap {
    flex: 1;
    min-height: 0;
    padding: 4px 6px;
  }
  .xterm-wrap :global(.xterm) {
    height: 100%;
  }

  .input-area {
    flex-shrink: 0;
    padding: 0 10px 10px;
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
