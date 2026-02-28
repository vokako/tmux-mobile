// Chat parsers for different CLI tools.
// Each parser exports: { name, detect, markers, parseMessages }
//
// To add a new tool:
// 1. Create a new parser object following the interface below
// 2. Add it to the `parsers` array
// 3. The first parser whose detect() returns true is used

function stripAnsi(s) {
  return s.replace(/\x1b\[[0-9;]*[a-zA-Z]/g, '').replace(/\x1b\][^\x07]*\x07/g, '').replace(/\x1b\[[\?]?[0-9;]*[a-zA-Z]/g, '');
}

// ─── Kiro CLI parser ───

const kiroParser = {
  name: 'kiro-cli',

  // Detect if this pane is running kiro-cli
  detect(raw, command = '') {
    if (/kiro/i.test(command)) return true;
    const clean = stripAnsi(raw);
    return /\d+%\s*!?\s*>/.test(clean);
  },

  // Insert semantic markers using ANSI color codes before stripping
  //   color 93  = user prompt ">" (purple)
  //   color 141 = agent response ">" (light purple)
  insertMarkers(raw) {
    let marked = raw.replace(/\x1b\[38;5;141m>\s?(\x1b\[39m)?/g, '\x00AGENT\x00');
    marked = marked.replace(/\x1b\[38;5;93m>\s?(\x1b\[39m)?/g, '\x00UPROMPT\x00');
    return marked;
  },

  // Classify a line. Returns { type, text?, rawText? } or null to skip.
  // Types: 'skip', 'user', 'agent', 'system', 'tool', 'tool_result', 'thinking', 'turn_end', 'empty', 'continuation'
  classifyLine(trimmed, rawLine) {
    // Init/status lines
    if (/^[○⠋]/.test(trimmed) || trimmed === 'kiro-cli' || /^✓.*loaded in/.test(trimmed)) return { type: 'skip' };
    if (/^--More--$/.test(trimmed)) return { type: 'skip' };
    if (/^Warning:/.test(trimmed)) return { type: 'skip' };

    // Thinking spinner
    if (/^[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]\s*Thinking/i.test(trimmed)) return { type: 'thinking' };

    // Compact summary borders
    if (/^═{4,}/.test(trimmed)) return { type: 'skip' };
    if (/^CONVERSATION SUMMARY$/.test(trimmed)) return { type: 'skip' };
    if (/^✔\s*Conversation compacted/.test(trimmed)) return { type: 'compact_start' };
    if (/conversation history has been replaced/.test(trimmed)) return { type: 'compact_end' };

    // Model selector
    if (/^Select model/.test(trimmed)) return { type: 'model_header' };
    if (/^Using\s+(\S+)/.test(trimmed)) {
      return { type: 'model_confirmed', text: trimmed.replace(/^Using\s+/, '').trim() };
    }
    if (/^>\s*\*?\s*\S+.*credits/i.test(trimmed)) return { type: 'model_selected', text: trimmed };
    if (/^\s{2,}\S+.*credits/i.test(trimmed)) return { type: 'model_item', text: trimmed };

    // Credits = end of turn
    if (/^▸\s*Credits:/.test(trimmed)) return { type: 'turn_end' };

    // User input (via color marker)
    if (trimmed.includes('\x00UPROMPT\x00')) {
      const text = trimmed.replace(/^.*\x00UPROMPT\x00\s*/, '').trim();
      const raw = rawLine.replace(/^.*\x00UPROMPT\x00\s*/, '');
      // Skip empty prompts and system hints (text starts with ANSI color = not real user input)
      if (!text || /^\x1b\[/.test(raw)) return { type: 'skip' };
      return { type: 'user', text, rawText: raw };
    }

    // Agent marker (via color marker) — but check if it's a model selector line
    if (trimmed.includes('\x00AGENT\x00')) {
      const afterMarker = trimmed.replace(/^.*\x00AGENT\x00\s*/, '').trim();
      if (/^\*?\s*\S+.*\d+\.\d+x\s*credits/i.test(afterMarker)) {
        return { type: 'model_selected', text: '> ' + afterMarker };
      }
      const text = afterMarker;
      const raw = rawLine.replace(/^.*\x00AGENT\x00\s*/, '');
      return { type: 'agent', text, rawText: raw };
    }

    // Fallback user input (no color, e.g. scrollback)
    const userMatch = trimmed.match(/^\d+%\s*!?\s*>\s*(.*)/);
    if (userMatch) {
      const text = userMatch[1].trim();
      return text ? { type: 'user', text, rawText: rawLine } : { type: 'skip' };
    }

    // Model selector items (non-selected)
    if (/^\S+.*\d+\.\d+x\s*credits/i.test(trimmed) && /\.\.$/.test(trimmed)) {
      return { type: 'model_item', text: '  ' + trimmed };
    }

    // Empty line
    if (!trimmed) return { type: 'empty' };

    // Tool call
    if (/\(using tool:/.test(trimmed) || /^(Searching|Reading|Looking up|Search |Found \d|Searching for)/.test(trimmed)) {
      return { type: 'tool' };
    }

    // Tool result
    if (/^[✓❗]/.test(trimmed) || /- Completed in/.test(trimmed)) {
      return { type: 'tool_result' };
    }

    return { type: 'continuation' };
  },

  // Extract status info from pane content
  extractStatus(raw) {
    const clean = stripAnsi(raw);
    const lines = clean.split('\n');
    let pct = null;
    for (let i = lines.length - 1; i >= 0; i--) {
      const m = lines[i].trim().match(/^(\d+)%\s/);
      if (m) { pct = parseInt(m[1]); break; }
    }
    return { pct, tool: 'kiro-cli' };
  },

  // Detect if pane is waiting for user input
  isWaitingForInput(raw) {
    const tail = raw.slice(-500);
    // Color 93 = user prompt ">" — may or may not have \e[39m after
    if (/\x1b\[38;5;93m>\s?(\x1b\[[\d;]*m)?\s*(\S.*)?$/.test(tail)) return true;
    const clean = stripAnsi(tail);
    const lines = clean.split('\n').filter(l => l.trim());
    const last = lines.at(-1)?.trim() || '';
    return /^\d+%\s*!?\s*>/.test(last);
  },
};

// ─── Parser registry ───

const parsers = [kiroParser];

export function detectParser(raw, command = '') {
  return parsers.find(p => p.detect(raw, command)) || null;
}

// ─── Generic message builder (works with any parser) ───

export function parseMessages(raw, parser) {
  if (!raw || !parser) return { messages: [], isThinking: false };

  const marked = parser.insertMarkers(raw);
  const rawLines = marked.split('\n');
  const cleanLines = rawLines.map(l => stripAnsi(l));
  const messages = [];
  let current = null;
  let isThinking = false;
  let started = false;

  function flush() {
    if (current && current.lines.some(l => l.trim())) {
      messages.push({
        ...current,
        text: current.lines.join('\n').trim(),
        rawText: current.rawLines.join('\n').trim(),
      });
    }
    current = null;
  }

  for (let i = 0; i < cleanLines.length; i++) {
    const line = cleanLines[i];
    const rawLine = rawLines[i];
    const trimmed = line.trim();
    const cls = parser.classifyLine(trimmed, rawLine);

    switch (cls.type) {
      case 'skip': continue;
      case 'thinking': isThinking = true; continue;
      case 'turn_end': isThinking = false; flush(); continue;
      case 'compact_start':
        isThinking = false; started = true; flush();
        current = { role: 'compact', lines: [], rawLines: [] };
        continue;
      case 'compact_end':
        flush(); continue;
      case 'model_header':
        isThinking = false; started = true; flush();
        current = { role: 'model', lines: [], rawLines: [] };
        continue;
      case 'model_confirmed':
        isThinking = false; started = true; flush();
        // Remove preceding model selector card
        while (messages.length && messages[messages.length - 1].role === 'model') messages.pop();
        current = { role: 'model_done', lines: [cls.text], rawLines: [rawLine] };
        flush();
        continue;
      case 'model_selected':
      case 'model_item':
        if (current?.role === 'model') { current.lines.push(cls.text); current.rawLines.push(rawLine); }
        continue;
      case 'user':
        isThinking = false; started = true; flush();
        current = { role: 'user', lines: [cls.text], rawLines: [cls.rawText] };
        continue;
      case 'agent':
        isThinking = false; started = true; flush();
        current = { role: 'agent', lines: cls.text ? [cls.text] : [], rawLines: cls.text ? [cls.rawText] : [] };
        continue;
      case 'empty':
        if (!started) continue;
        if (current?.role === 'user') { flush(); }
        else if (current) { current.lines.push(''); current.rawLines.push(''); }
        continue;
      case 'tool':
        isThinking = false;
        if (!started) continue;
        if (!current || current.role !== 'agent') { flush(); current = { role: 'agent', lines: [], rawLines: [] }; }
        current.lines.push(line); current.rawLines.push(rawLine);
        continue;
      case 'tool_result':
        if (current?.role === 'agent') { current.lines.push(line); current.rawLines.push(rawLine); }
        continue;
      case 'continuation':
        isThinking = false;
        if (!started) continue;
        if (current) {
          current.lines.push(line); current.rawLines.push(rawLine);
        } else {
          current = { role: 'system', lines: [line], rawLines: [rawLine] };
        }
        continue;
    }
  }
  flush();
  return { messages, isThinking };
}

export { stripAnsi };
