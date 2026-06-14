<script>
  import { detectParser, parseMessages, stripAnsi } from './parsers.js';
  import Icon from './Icon.svelte';
  import { t } from './i18n.svelte.js';
  import { copyText } from './clipboard.js';

  let { content = '', onSendKeys = null, command = '', fontSize = 14 } = $props();

  const SCROLL_BOTTOM_THRESHOLD = 40;
  const SCROLL_DELAYS_MS = [50, 150, 300, 500];

  // Simple LRU cache helper
  function cached(map, limit, key, compute) {
    let r = map.get(key);
    if (r === undefined) { r = compute(); map.set(key, r); if (map.size > limit) map.delete(map.keys().next().value); }
    return r;
  }

  let chatEl;

  // ANSI 256-color palette (basic 16 + 216 cube + 24 grayscale)
  const ansi256 = (() => {
    const c = [];
    const base = ['#0a0a0f','#ff5050','#4ade80','#fbbf24','#00d4ff','#c084fc','#22d3ee','#c9d1d9',
                  '#484848','#ff6b6b','#6ee7a0','#fcd34d','#38bdf8','#d8b4fe','#67e8f9','#f1f5f9'];
    for (let i = 0; i < 16; i++) c[i] = base[i];
    for (let i = 0; i < 216; i++) {
      const r = Math.floor(i / 36), g = Math.floor((i % 36) / 6), b = i % 6;
      c[16 + i] = `#${[r,g,b].map(v => (v ? v * 40 + 55 : 0).toString(16).padStart(2,'0')).join('')}`;
    }
    for (let i = 0; i < 24; i++) { const v = (i * 10 + 8).toString(16).padStart(2, '0'); c[232 + i] = `#${v}${v}${v}`; }
    return c;
  })();

  function ensureReadable(hex) {
    if (!hex) return hex;
    const r = parseInt(hex.slice(1,3), 16), g = parseInt(hex.slice(3,5), 16), b = parseInt(hex.slice(5,7), 16);
    const lum = (0.299 * r + 0.587 * g + 0.114 * b) / 255;
    if (lum >= 0.25) return hex;
    const t = 0.25 / Math.max(lum, 0.01);
    const clamp = v => Math.min(255, Math.round(v * Math.min(t, 3)));
    const rr = clamp(Math.max(r, 30)), gg = clamp(Math.max(g, 30)), bb = clamp(Math.max(b, 30));
    return `#${[rr,gg,bb].map(v => v.toString(16).padStart(2,'0')).join('')}`;
  }

  function ansiToHtml(s) {
    s = s.replace(/\x00(AGENT|UPROMPT|CCUSER|CCAGENT|CCTOOL|CCTOOLFAIL)\x00/g, '');
    let html = '', fg = null, bold = false;
    const parts = s.split(/(\x1b\[[\?]?[0-9;]*[a-zA-Z]|\x1b\][^\x07]*\x07)/);
    for (const part of parts) {
      const m = part.match(/^\x1b\[([\d;]*)m$/);
      if (m) {
        const codes = m[1].split(';').map(Number);
        for (let i = 0; i < codes.length; i++) {
          const c = codes[i];
          if (c === 0) { if (fg || bold) html += '</span>'; fg = null; bold = false; }
          else if (c === 1) { if (fg || bold) html += '</span>'; bold = true; html += `<span style="font-weight:700${fg ? ';color:'+ensureReadable(fg) : ''}">`; }
          else if (c >= 30 && c <= 37) { if (fg || bold) html += '</span>'; fg = ansi256[c - 30]; html += `<span style="color:${ensureReadable(fg)}${bold ? ';font-weight:700' : ''}">`; }
          else if (c >= 90 && c <= 97) { if (fg || bold) html += '</span>'; fg = ansi256[c - 90 + 8]; html += `<span style="color:${ensureReadable(fg)}${bold ? ';font-weight:700' : ''}">`; }
          else if (c === 38 && codes[i+1] === 5) { if (fg || bold) html += '</span>'; fg = ansi256[codes[i+2]] || null; if (fg) html += `<span style="color:${ensureReadable(fg)}${bold ? ';font-weight:700' : ''}">`; i += 2; }
          else if (c === 39) { if (fg || bold) html += '</span>'; fg = null; if (bold) html += '<span style="font-weight:700">'; }
        }
      } else if (/^\x1b/.test(part)) {
        // discard non-SGR
      } else {
        html += part.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
      }
    }
    if (fg || bold) html += '</span>';
    return html;
  }

  // Detect if a line is part of a diff block (Kiro CLI format: "+  NNN:" or "- NNN  :" or "  NNN, NNN:")
  function isDiffLine(line) {
    return /^[+\-]\s+\d+\s*:/.test(line) || /^\s*\d+,\s*\d+\s*:/.test(line);
  }

  // Detect if a line starts a tool call block
  // Must be specific to avoid false positives on explanatory text
  function isToolLine(line) {
    // Kiro: tool syntax anywhere in line (e.g. "I'll modify ... (using tool: write)")
    if (/\(using tool:\s*\w+\)/.test(line)) return true;
    // Kiro in-progress: require specific suffixes (not bare words)
    if (/^Searching in \d/.test(line)) return true;
    if (/^Reading file/.test(line)) return true;
    if (/^Looking up definition/.test(line)) return true;
    if (/^Found \d+ (result|match|file|reference)/.test(line)) return true;
    // Claude Code: specific action + argument patterns
    if (/^Searched for \d+/.test(line)) return true;
    if (/^Read \d+ file/.test(line)) return true;
    if (/^(Update|Bash|Write|Edit)\(\S+/.test(line)) return true;
    if (/^Wrote to \S/.test(line)) return true;
    if (/^Ran /.test(line)) return true;
    if (/^Created \S/.test(line)) return true;
    if (/^Deleted \S/.test(line)) return true;
    if (/^Edited \S/.test(line)) return true;
    return false;
  }

  function parseBlocks(text, rawText) {
    const blocks = [];
    const lines = text.split('\n');
    const rawLines = (rawText || text).split('\n');
    let i = 0;
    while (i < lines.length) {
      const trimmed = lines[i].trim();
      if (/^```/.test(trimmed)) {
        const lang = trimmed.replace(/^```/, '').trim();
        const codeLines = [];
        i++;
        while (i < lines.length && !/^```\s*$/.test(lines[i].trim())) { codeLines.push(rawLines[i]); i++; }
        blocks.push({ type: 'code', lang: lang || 'text', content: codeLines.join('\n') });
        i++; continue;
      }
      if (isToolLine(trimmed)) {
        const toolRaw = [rawLines[i]]; i++;
        while (i < lines.length) {
          const next = lines[i].trim();
          if (/^[✓❗⎿]/.test(next) || /Completed in/.test(next) || /^\d+ (files|entries|of \d)/.test(next)
            || /^User rejected/.test(next) || /^\d+\s+[+\-]/.test(next)) { toolRaw.push(rawLines[i]); i++; }
          else break;
        }
        // Extract label: Kiro "(using tool: X)" or Claude Code "Action(args)"
        const label = trimmed.match(/\(using tool:\s*(\w+)\)/)?.[1]
          || trimmed.match(/^(Searched|Read|Wrote|Update|Bash|Ran|Created|Deleted|Edited)\b/)?.[1]?.toLowerCase()
          || 'tool';
        const toolText = toolRaw.join('\n');
        const isError = /Interrupted|User rejected|failed|error/i.test(toolText);
        blocks.push({ type: 'tool', label, content: toolText, isError });
        continue;
      }
      // Diff block: consecutive lines matching diff pattern
      if (isDiffLine(trimmed)) {
        const diffRaw = [rawLines[i]]; i++;
        while (i < lines.length && (isDiffLine(lines[i].trim()) || !lines[i].trim())) {
          diffRaw.push(rawLines[i]); i++;
        }
        blocks.push({ type: 'diff', content: diffRaw.join('\n') });
        continue;
      }
      const textRaw = [rawLines[i]]; i++;
      while (i < lines.length) {
        const next = lines[i].trim();
        if (/^```/.test(next) || isToolLine(next) || isDiffLine(next)) break;
        textRaw.push(rawLines[i]); i++;
      }
      const t = textRaw.join('\n').trim();
      if (t) blocks.push({ type: 'text', content: t });
    }
    return blocks;
  }

  let parser = $derived(detectParser('', command));
  let parsed = $state({ messages: [], isThinking: false, isSummarizing: false });
  let lastParsedContent = '';
  $effect(() => {
    const p = parser;
    const c = content;
    if (!p || c === lastParsedContent) return;
    lastParsedContent = c;
    parsed = parseMessages(c, p);
  });
  let messages = $derived(parsed.messages);
  let showThinking = $state(false);
  let showSummarizing = $state(false);
  let thinkingTimer;
  let summarizingTimer;
  $effect(() => {
    const raw = parsed.isThinking;
    if (raw) {
      clearTimeout(thinkingTimer);
      showThinking = true;
    } else {
      thinkingTimer = setTimeout(() => showThinking = false, 600);
    }
  });
  $effect(() => {
    const raw = parsed.isSummarizing;
    if (raw) {
      clearTimeout(summarizingTimer);
      showSummarizing = true;
    } else {
      summarizingTimer = setTimeout(() => showSummarizing = false, 600);
    }
  });

  let isAtBottom = $state(true);

  function checkAtBottom() {
    if (!chatEl) return;
    isAtBottom = chatEl.scrollHeight - chatEl.scrollTop - chatEl.clientHeight < SCROLL_BOTTOM_THRESHOLD;
  }

  function scrollToBottom() {
    if (chatEl) { chatEl.scrollTop = chatEl.scrollHeight; isAtBottom = true; }
  }

  // Scroll on mount and new messages
  $effect(() => {
    messages;
    requestAnimationFrame(() => {
      if (isAtBottom) scrollToBottom();
      checkAtBottom();
    });
  });

  // Scroll to bottom when keyboard opens/closes (only if user was at bottom)
  function scheduleScroll() {
    if (!isAtBottom) return;
    for (const ms of SCROLL_DELAYS_MS) setTimeout(() => {
      if (isAtBottom) scrollToBottom();
      checkAtBottom();
    }, ms);
  }
  $effect(() => {
    const vv = window.visualViewport;
    if (vv) vv.addEventListener('resize', scheduleScroll);
    window.addEventListener('resize', scheduleScroll);
    const onFocus = (e) => { if (e.target.tagName === 'TEXTAREA') scheduleScroll(); };
    document.addEventListener('focusin', onFocus);
    return () => {
      if (vv) vv.removeEventListener('resize', scheduleScroll);
      window.removeEventListener('resize', scheduleScroll);
      document.removeEventListener('focusin', onFocus);
    };
  });

  function renderMarkdown(text) {
    let html = text.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    html = html.replace(/((?:^|\n)\|.+\|(?:\n\|.+\|)+)/g, (match) => {
      const rows = match.trim().split('\n').filter(r => r.trim());
      const dataRows = rows.filter(r => !/^\|[\s\-:|]+\|$/.test(r.trim()));
      if (dataRows.length === 0) return match;
      let table = '<table>';
      dataRows.forEach((row, i) => {
        const cells = row.split('|').filter((_, ci, arr) => ci > 0 && ci < arr.length - 1).map(c => c.trim());
        const tag = i === 0 ? 'th' : 'td';
        table += '<tr>' + cells.map(c => `<${tag}>${c}</${tag}>`).join('') + '</tr>';
      });
      return table + '</table>';
    });
    html = html.replace(/^(#{1,3})\s+(.+)$/gm, (_, h, t) => `<strong>${t}</strong>`);
    html = html.replace(/^[-*]\s+(.+)$/gm, '<li>$1</li>');
    html = html.replace(/((?:<li>.*<\/li>\n?)+)/g, '<ul>$1</ul>');
    html = html.replace(/^---+$/gm, '<hr>');
    html = html.replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>');
    html = html.replace(/\*(.+?)\*/g, '<em>$1</em>');
    html = html.replace(/`([^`]+)`/g, '<code>$1</code>');
    html = html.replace(/\[([^\]]+)\]\(([^)]+)\)/g, '<a href="$2" target="_blank" rel="noopener">$1</a>');
    html = html.replace(/\n{2,}/g, '<br><br>');
    html = html.replace(/\n/g, '<br>');
    return html;
  }

  // Memoize expensive render functions
  const _ansiCache = new Map();
  const _mdCache = new Map();
  const _blockCache = new Map();
  function cachedAnsiToHtml(s) { return cached(_ansiCache, 500, s, () => ansiToHtml(s)); }
  function cachedRenderMarkdown(s) { return cached(_mdCache, 200, s, () => renderMarkdown(s)); }
  function cachedParseBlocks(text, rawText) { return cached(_blockCache, 200, text, () => parseBlocks(text, rawText)); }

  let collapsedTools = $state({});
  function toggleTool(id) { collapsedTools[id] = !collapsedTools[id]; }

  let copyMsg = $state(null);
  let copiedMsg = $state(null);
  let copyTimer;
  function handleBubbleTap(mi, text) {
    copyMsg = copyMsg === mi ? null : mi;
    copiedMsg = null;
    clearTimeout(copyTimer);
    if (copyMsg !== null) copyTimer = setTimeout(() => copyMsg = null, 3000);
  }
  async function doCopy(text) {
    await copyText(text);
    copiedMsg = copyMsg;
    setTimeout(() => { copiedMsg = null; copyMsg = null; }, 1000);
  }

  async function selectModel(targetIdx, currentIdx) {
    if (!onSendKeys) return;
    const diff = targetIdx - currentIdx;
    const key = diff > 0 ? 'Down' : 'Up';
    for (let i = 0; i < Math.abs(diff); i++) {
      await onSendKeys(key);
      await new Promise(r => setTimeout(r, 50));
    }
    await onSendKeys('Enter');
  }

  async function selectPermission(index) {
    if (!onSendKeys) return;
    await onSendKeys(String(index));
  }
</script>

<div class="chat-wrap" style="--chat-font-size: {fontSize}px">
  <div class="chat" bind:this={chatEl} onscroll={checkAtBottom}>
    {#if messages.length === 0}
    <div class="empty">{t('noConversation')}</div>
  {:else}
    {#each messages as msg, mi (mi)}
      <div class="msg" class:user={msg.role === 'user'} class:agent={msg.role === 'agent'} class:system={msg.role === 'system' || msg.role === 'compact' || msg.role === 'model' || msg.role === 'model_done' || msg.role === 'permission'}>
        {#if msg.role === 'agent'}
          <div class="avatar"><Icon name="bot" size={14} /></div>
        {/if}
        {#if msg.role === 'user'}
          <div class="avatar user-avatar"><Icon name="user" size={14} /></div>
        {/if}
        {#if msg.role === 'permission'}
          <div class="permission-bubble">
            <div class="permission-header"><Icon name="info" size={13} /> {msg.text}</div>
            {#each msg.options || [] as opt}
              <button class="permission-item" class:permission-selected={opt.selected} onclick={() => selectPermission(opt.index)}>
                <span class="permission-index">{opt.index}.</span>
                <span class="permission-text">{opt.text}</span>
              </button>
            {/each}
          </div>
        {:else if msg.role === 'system'}
          <div class="system-bubble">
            <pre class="system-pre">{@html cachedAnsiToHtml(msg.rawText)}</pre>
          </div>
        {:else if msg.role === 'compact'}
          <div class="compact-bubble">
            <div class="compact-header"><Icon name="info" size={13} /> {t('conversationSummary')}</div>
            <div class="compact-body">{@html cachedRenderMarkdown(stripAnsi(msg.text))}</div>
          </div>
        {:else if msg.role === 'model'}
          <div class="model-bubble">
            <div class="model-header"><Icon name="gear" size={13} /> {t('selectModel')}</div>
            {#each msg.text.split('\n').filter(l => l.trim()) as item, idx}
              {@const allItems = msg.text.split('\n').filter(l => l.trim())}
              {@const currentIdx = allItems.findIndex(l => /^>/.test(l.trim()))}
              {@const selected = /^>/.test(item.trim())}
              {@const name = item.replace(/^>\s*\*?\s*/, '').replace(/\s+\d+\.\d+x.*/, '').trim()}
              {@const credits = item.match(/(\d+\.\d+x\s*credits)/)?.[1] || ''}
              {@const active = /\*/.test(item)}
              <button class="model-item" class:model-selected={selected} onclick={() => selectModel(idx, currentIdx)}>
                <span class="model-name">{name}{#if active} *{/if}</span>
                <span class="model-credits">{credits}</span>
              </button>
            {/each}
          </div>
        {:else if msg.role === 'model_done'}
          <div class="model-done">
            <Icon name="check" size={14} />
            <span class="model-done-name">{msg.text}</span>
          </div>
        {:else}
        <div class="bubble-wrap">
          <div class="bubble" class:user-bubble={msg.role === 'user'} class:agent-bubble={msg.role === 'agent'} role="button" tabindex="-1" onclick={() => handleBubbleTap(mi, msg.text)} onkeydown={(e) => { if (e.key === 'Enter') handleBubbleTap(mi, msg.text); }}>
          {#each cachedParseBlocks(msg.text, msg.rawText) as block, bi}
            {#if block.type === 'text'}
              {#if msg.role === 'user'}
                <div class="md-block">{@html cachedRenderMarkdown(stripAnsi(block.content))}</div>
              {:else}
                <div class="md-block">{@html cachedRenderMarkdown(stripAnsi(block.content))}</div>
              {/if}
            {:else if block.type === 'code'}
              <div class="code-block">
                <div class="code-header">{block.lang}</div>
                <pre><code>{@html cachedAnsiToHtml(block.content)}</code></pre>
              </div>
            {:else if block.type === 'tool'}
              <div class="tool-card" class:tool-error={block.isError}>
                <button class="tool-header" onclick={() => toggleTool(`${mi}-${bi}`)}>
                  <span class="tool-icon"><Icon name={block.isError ? "x" : "gear"} size={10} /></span>
                  <span class="tool-label">{block.label}</span>
                  <span class="tool-chevron" class:open={!collapsedTools[`${mi}-${bi}`]}>▸</span>
                </button>
                {#if !collapsedTools[`${mi}-${bi}`]}
                  <pre class="tool-body">{@html cachedAnsiToHtml(block.content)}</pre>
                {/if}
              </div>
            {:else if block.type === 'diff'}
              <div class="diff-block">
                {#each block.content.split('\n') as dline}
                  {@const clean = stripAnsi(dline)}
                  <div class="diff-line" class:diff-add={/^\+/.test(clean.trim())} class:diff-del={/^-/.test(clean.trim())} class:diff-ctx={/^\s*\d+,\s*\d+\s*:/.test(clean)}>{@html cachedAnsiToHtml(dline)}</div>
                {/each}
              </div>
            {/if}
          {/each}
        </div>
          {#if copyMsg === mi}
            <button class="copy-btn" class:copied={copiedMsg === mi} onclick={(e) => { e.stopPropagation(); doCopy(msg.text); }}>
              <Icon name={copiedMsg === mi ? "check" : "copy"} size={16} /> {t('copy')}
            </button>
          {/if}
        </div>
        {/if}
      </div>
    {/each}
    {#if showThinking}
      <div class="msg agent">
        <div class="avatar"><Icon name="bot" size={14} /></div>
        <div class="bubble agent-bubble thinking-bubble">
          <span class="thinking-spinner"></span>
          <span class="thinking-text">{t('thinking')}</span>
        </div>
      </div>
    {/if}
    {#if showSummarizing}
      <div class="msg agent">
        <div class="avatar"><Icon name="bot" size={14} /></div>
        <div class="bubble agent-bubble thinking-bubble">
          <span class="thinking-spinner"></span>
          <span class="thinking-text">{t('creatingSummary')}</span>
        </div>
      </div>
    {/if}
  {/if}
</div>
  {#if !isAtBottom}
    <button class="scroll-btn" onclick={scrollToBottom}><Icon name="arrow-down" size={16} /></button>
  {/if}
</div>

<style>
  .chat-wrap {
    flex: 1;
    min-height: 0;
    position: relative;
  }

  .chat {
    height: 100%;
    overflow-y: auto;
    -webkit-overflow-scrolling: touch;
    padding: 16px 12px;
    display: flex;
    flex-direction: column;
    gap: 12px;
    contain: layout style;
  }

  .scroll-btn {
    position: absolute;
    bottom: 12px;
    right: 16px;
    width: 36px; height: 36px;
    border: 1px solid var(--border);
    border-radius: 10px;
    background: rgba(10,10,15,0.85);
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

  .empty {
    text-align: center;
    color: var(--text3);
    font-size: 14px;
    padding: 40px 20px;
  }

  .msg {
    display: flex;
    gap: 8px;
    max-width: 88%;
    align-items: flex-start;
  }
  .msg.user { align-self: flex-end; flex-direction: row-reverse; }
  .msg.agent { align-self: flex-start; }
  .msg.system { align-self: stretch; max-width: 100%; }

  .avatar {
    width: 28px; height: 28px;
    border-radius: 50%;
    background: var(--surface2);
    display: flex;
    align-items: center;
    justify-content: center;
    font-size: 14px;
    flex-shrink: 0;
    margin-top: 2px;
  }

  .bubble {
    border-radius: 16px;
    padding: 10px 14px;
    font-size: var(--chat-font-size, 14px);
    line-height: 1.5;
    min-width: 40px;
    max-width: 100%;
    overflow: hidden;
    overflow-wrap: break-word;
    display: flex;
    flex-direction: column;
    gap: 8px;
  }

  .bubble-wrap {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: flex-start;
    min-width: 0;
    max-width: 100%;
  }
  .msg.user .bubble-wrap { align-items: flex-end; }

  .copy-btn {
    position: absolute;
    top: -8px;
    right: -8px;
    display: flex;
    align-items: center;
    padding: 8px 12px;
    border: 1px solid var(--input-border);
    border-radius: 8px;
    background: var(--bg);
    color: var(--text2);
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    z-index: 2;
    font-size: 12px;
    gap: 4px;
  }
  .msg.user .copy-btn { right: auto; left: -8px; }
  .copy-btn:active {
    background: var(--accent-bg);
    color: var(--accent);
  }
  .copy-btn.copied {
    background: var(--accent);
    color: #fff;
    border-color: var(--accent);
  }

  .user-bubble {
    background: #0a5a7a;
    color: #e2e8f0;
    border-bottom-right-radius: 4px;
  }

  .agent-bubble {
    background: var(--surface2);
    border: 1px solid var(--border);
    color: var(--text);
    border-bottom-left-radius: 4px;
  }

  .thinking-bubble {
    flex-direction: row;
    align-items: center;
    gap: 8px;
    padding: 10px 16px;
  }
  .thinking-spinner {
    width: 16px; height: 16px;
    border: 2px solid var(--input-border);
    border-top-color: var(--accent);
    border-radius: 50%;
    animation: spin 0.8s linear infinite;
    flex-shrink: 0;
  }
  @keyframes spin { to { transform: rotate(360deg); } }
  .thinking-text {
    color: var(--text3);
    font-size: 13px;
    font-style: italic;
  }

  .md-block { line-height: 1.6; }
  .agent-bubble .md-block { white-space: pre-wrap; }

  .system-bubble {
    width: 100%;
    border-radius: 12px;
    background: var(--accent-bg);
    border: 1px solid var(--accent);
    padding: 10px 14px;
    overflow-x: auto;
  }
  .system-pre {
    margin: 0;
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: 12px;
    line-height: 1.5;
    color: var(--text2);
    white-space: pre-wrap;
    word-break: break-word;
  }

  .compact-bubble {
    width: 100%;
    border-radius: 12px;
    background: var(--accent-bg);
    border: 1px solid var(--accent);
    overflow: hidden;
  }
  .compact-header {
    display: flex; align-items: center; gap: 6px;
    padding: 8px 14px; font-size: 12px; font-weight: 600; color: var(--accent);
    background: var(--accent-bg); border-bottom: 1px solid var(--accent);
  }
  .compact-body {
    padding: 10px 14px; font-size: var(--chat-font-size, 14px); line-height: 1.6; color: var(--text2);
  }
  .compact-body :global(h2) { font-size: var(--chat-font-size, 14px); font-weight: 700; color: var(--accent); margin: 10px 0 4px; }
  .compact-body :global(ul), .compact-body :global(ol) { padding-left: 16px; margin: 4px 0; }
  .compact-body :global(li) { margin: 2px 0; }
  .compact-body :global(p) { margin: 4px 0; }
  .compact-body :global(code) { background: var(--surface2); padding: 1px 4px; border-radius: 3px; font-size: 11px; }

  .model-bubble {
    width: 100%;
    border-radius: 12px;
    background: var(--surface);
    border: 1px solid var(--input-border);
    overflow: hidden;
  }
  .model-header {
    display: flex; align-items: center; gap: 6px;
    padding: 8px 14px; font-size: 12px; font-weight: 600; color: var(--text2);
    background: var(--surface); border-bottom: 1px solid var(--border);
  }
  .model-item {
    display: flex; justify-content: space-between; align-items: center; width: 100%;
    padding: 10px 14px; font-size: 13px; color: var(--text2);
    border: none; background: none; border-bottom: 1px solid var(--border2);
    cursor: pointer; -webkit-tap-highlight-color: transparent; text-align: left;
  }
  .model-item:last-child { border-bottom: none; }
  .model-item:active { background: var(--accent-bg); }
  .model-item.model-selected { background: var(--accent-bg); color: var(--accent); }
  .model-name { font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace; font-size: 12px; }
  .model-credits { font-size: 11px; color: var(--text3); }

  .model-done {
    display: flex; align-items: center; gap: 8px;
    padding: 10px 14px; border-radius: 10px;
    background: var(--accent-bg); border: 1px solid var(--accent);
    color: var(--accent); font-size: 13px;
  }
  .model-done-name { font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace; font-weight: 500; }

  .permission-bubble {
    width: 100%;
    border-radius: 12px;
    background: var(--surface);
    border: 1px solid var(--accent);
    overflow: hidden;
  }
  .permission-header {
    display: flex; align-items: center; gap: 6px;
    padding: 10px 14px; font-size: 13px; font-weight: 600; color: var(--accent);
    background: var(--accent-bg); border-bottom: 1px solid var(--accent);
  }
  .permission-item {
    display: flex; align-items: center; gap: 8px; width: 100%;
    padding: 12px 14px; font-size: 14px; color: var(--text);
    border: none; background: none; border-bottom: 1px solid var(--border2);
    cursor: pointer; -webkit-tap-highlight-color: transparent; text-align: left;
  }
  .permission-item:last-child { border-bottom: none; }
  .permission-item:active { background: var(--accent-bg); }
  .permission-item.permission-selected { background: var(--accent-bg); color: var(--accent); }
  .permission-index { font-weight: 600; color: var(--text3); min-width: 20px; }
  .permission-text { flex: 1; }
  .md-block :global(strong) { font-weight: 600; color: var(--text); }
  .md-block :global(em) { font-style: italic; color: var(--text2); }
  .md-block :global(code) {
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: 12px;
    background: var(--surface2);
    padding: 1px 5px;
    border-radius: 4px;
  }
  .md-block :global(a) { color: var(--accent); text-decoration: underline; }
  .md-block :global(table) {
    width: 100%;
    border-collapse: collapse;
    font-size: 13px;
    margin: 4px 0;
  }
  .md-block :global(th),
  .md-block :global(td) {
    padding: 5px 10px;
    border: 1px solid var(--input-border);
    text-align: left;
  }
  .md-block :global(th) {
    background: var(--code-bg);
    font-weight: 600;
    font-size: 12px;
    color: var(--text2);
  }
  .md-block :global(ul) {
    margin: 4px 0;
    padding-left: 18px;
  }
  .md-block :global(li) { margin: 2px 0; }
  .md-block :global(hr) {
    border: none;
    border-top: 1px solid var(--border);
    margin: 8px 0;
  }
  .user-bubble .md-block :global(code) { background: var(--surface2); }
  .user-bubble .md-block :global(a) { color: var(--accent); }
  .user-bubble .md-block :global(strong) { color: var(--text); }
  .user-bubble .md-block :global(em) { color: var(--text2); }
  .user-bubble .md-block :global(th),
  .user-bubble .md-block :global(td) { border-color: var(--border); }
  .user-bubble .md-block :global(th) { background: var(--surface2); }

  .code-block {
    border-radius: 10px;
    overflow: hidden;
    background: var(--code-bg);
    border: 1px solid var(--border);
  }
  .code-header {
    padding: 4px 10px;
    font-size: 11px;
    color: var(--text3);
    background: var(--surface);
    border-bottom: 1px solid var(--border2);
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace;
    text-transform: uppercase;
    letter-spacing: 0.5px;
  }
  .code-block pre {
    margin: 0;
    padding: 10px;
    overflow-x: auto;
    font-size: 12px;
    line-height: 1.5;
  }
  .code-block code {
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace;
    color: var(--text);
  }

  .tool-card {
    border-radius: 10px;
    overflow: hidden;
    border: 1px solid var(--border);
    background: var(--surface);
  }
  .tool-card.tool-error {
    border-color: var(--danger);
    background: var(--danger-bg);
  }
  .tool-card.tool-error .tool-header { background: var(--danger-bg); color: var(--danger); }
  .tool-card.tool-error .tool-body { color: var(--danger); }
  .tool-header {
    display: flex;
    align-items: center;
    gap: 5px;
    width: 100%;
    padding: 5px 8px;
    background: var(--surface);
    border: none;
    color: var(--text2);
    cursor: pointer;
    font-size: 11px;
    font-weight: 500;
    text-align: left;
    -webkit-tap-highlight-color: transparent;
  }
  .tool-header:active { background: var(--surface2); }
  .tool-label { flex: 1; }
  .tool-icon { font-size: 11px; }
  .tool-chevron {
    transition: transform 0.15s ease;
    font-size: 10px;
  }
  .tool-chevron.open { transform: rotate(90deg); }

  .tool-body {
    margin: 0;
    padding: 8px 10px;
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: 11px;
    color: var(--text2);
    line-height: 1.4;
    overflow-x: auto;
    border-top: 1px solid var(--border2);
  }

  .diff-block {
    border-radius: 10px;
    overflow: hidden;
    border: 1px solid var(--border);
    background: var(--code-bg);
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'Noto Sans Symbols 2', 'Symbols Nerd Font Mono', 'Maple Mono CJK', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: 12px;
    line-height: 1.5;
    overflow-x: auto;
  }
  .diff-line {
    padding: 1px 10px;
    white-space: pre;
  }
  .diff-add {
    background: rgba(74, 222, 128, 0.1);
    border-left: 3px solid var(--accent);
  }
  .diff-del {
    background: var(--danger-bg);
    border-left: 3px solid var(--danger);
  }
  .diff-ctx {
    color: var(--text3);
    border-left: 3px solid transparent;
  }
</style>
