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
    return /kiro/i.test(command);
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
    if (/^[○⠋]/.test(trimmed) || /^✓.*loaded in/.test(trimmed)) return { type: 'skip' };
    if (trimmed === 'kiro-cli') return { type: 'reset' };
    if (/^--More--$/.test(trimmed)) return { type: 'skip' };
    if (/^Warning:/.test(trimmed)) return { type: 'skip' };

    // Thinking spinner
    if (/^[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]\s*Thinking/i.test(trimmed)) return { type: 'thinking' };

    // Summarizing spinner
    if (/^[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]\s*Creating summary/i.test(trimmed)) return { type: 'summarizing' };

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

// ─── Claude Code parser ───

const claudeCodeParser = {
  name: 'claude-code',

  // Detect if this pane is running Claude Code
  detect(raw, command = '') {
    if (/claude/i.test(command)) return true;
    const clean = stripAnsi(raw);
    // Fallback: check for Claude Code banner in content
    if (/Claude Code v\d/.test(clean)) return true;
    // Fallback: ⏺ marker is Claude Code's unique prompt character
    if (/^⏺/m.test(clean)) return true;
    return false;
  },

  // Insert semantic markers using RGB true-color ANSI codes before stripping
  //   User prompt: gray ❯ on bg #373737 with white text
  //     \x1b[38;2;80;80;80m\x1b[48;2;55;55;55m❯ \x1b[38;2;255;255;255m<text>
  //   Ghost suggestion (NOT real input): ❯ \x1b[7m<char>\x1b[0;2m<rest>
  //   Agent response: \x1b[38;2;255;255;255m⏺ (white)
  //   Tool completed:  \x1b[38;2;78;186;101m⏺ (green)
  //   Tool in-progress: \x1b[38;2;153;153;153m⏺ (gray) — often while reading/searching
  //   Tool rejected:   \x1b[38;2;255;107;128m⏺ (red/pink)
  insertMarkers(raw) {
    let marked = raw;
    // Mark real user prompts (must have bg color 48;2;55;55;55 to distinguish from ghost)
    marked = marked.replace(/\x1b\[38;2;80;80;80m\x1b\[48;2;55;55;55m❯\s?\x1b\[38;2;255;255;255m/g, '\x00CCUSER\x00');
    // Match ⏺ with optional preceding reset/bold codes (more permissive)
    const esc = '(?:\\x1b\\[[0-9;]*m)*'; // optional preceding ANSI codes
    // Mark agent text responses (white ⏺)
    marked = marked.replace(new RegExp(esc + '\\x1b\\[38;2;255;255;255m⏺\\x1b\\[39m', 'g'), '\x00CCAGENT\x00');
    // Mark tool completed (green ⏺)
    marked = marked.replace(new RegExp(esc + '\\x1b\\[38;2;78;186;101m⏺\\x1b\\[39m', 'g'), '\x00CCTOOL\x00');
    // Mark tool in-progress (gray ⏺) — treat same as tool
    marked = marked.replace(/\x1b\[38;2;153;153;153m⏺?\x1b\[39m/g, (m) => {
      return m.includes('⏺') ? '\x00CCTOOL\x00' : m;
    });
    // Mark tool rejected/error (red ⏺)
    marked = marked.replace(new RegExp(esc + '\\x1b\\[38;2;255;107;128m⏺\\x1b\\[39m', 'g'), '\x00CCTOOLFAIL\x00');
    return marked;
  },

  classifyLine(trimmed, rawLine) {
    // Banner / reset — Claude Code header (both plain and box formats)
    // Plain: "Claude Code v2.1.69"
    // Box:   "╭─── Claude Code v2.1.69 ───╮"
    if (/Claude Code v[\d.]+/.test(trimmed)) return { type: 'reset' };
    // Box border lines (╭╮╰╯│)
    if (/^[╭╰]─/.test(trimmed) || /^│/.test(trimmed)) return { type: 'skip' };
    // ASCII art logo lines (orange blocks) — both standalone and inside box
    if (/[▐▝▘▜▛█]{2,}/.test(trimmed) && !/\x00CC/.test(trimmed)) return { type: 'skip' };
    // Welcome line
    if (/^Welcome (to|back)\b/.test(trimmed)) return { type: 'skip' };
    // Hint lines
    if (/^\/model to try\b/.test(trimmed)) return { type: 'skip' };
    if (/^Tips for getting/.test(trimmed)) return { type: 'skip' };
    if (/^(Run \/init|No recent activity|Recent activity)/.test(trimmed)) return { type: 'skip' };
    // Accept edits status
    if (/^⏵/.test(trimmed)) return { type: 'skip' };

    // Separator lines (gray ─── )
    if (/^─{4,}/.test(trimmed)) return { type: 'skip' };
    // Dashed separator (╌╌╌)
    if (/^╌{4,}/.test(trimmed)) return { type: 'skip' };

    // Ghost suggestion / empty prompt — has reverse video \x1b[7m, NO bg color
    // These show up as: ❯ <text> after ANSI stripping, but raw has \x1b[7m
    if (/^❯(\s|$)/.test(trimmed) && /\x1b\[7m/.test(rawLine) && !/\x1b\[48;2;55;55;55m/.test(rawLine)) {
      return { type: 'skip' };
    }
    // Bare ❯ without any ANSI (scrollback artifact)
    if (/^❯\s*$/.test(trimmed) && !/\x1b\[48;2;55;55;55m/.test(rawLine)) {
      return { type: 'skip' };
    }

    // Bottom status lines
    if (/^\?\s*(for shortcuts|for help)/.test(trimmed)) return { type: 'skip' };
    if (/^esc\s+to\s+interrupt/.test(trimmed)) return { type: 'skip' };
    if (/^Enter to confirm/.test(trimmed)) return { type: 'skip' };
    if (/^Esc to cancel/.test(trimmed)) return { type: 'skip' };

    // Thinking/Simmering — orange star symbols ✳✶✻ (during processing)
    if (/^[✳✶✷✸✹✺]\s*(Simmering|Thinking|Brewing|Steeping)/i.test(trimmed)) return { type: 'thinking' };

    // Turn end — "✻ Cooked for Ns" (gray, after completion)
    if (/^✻\s*Cooked\b/.test(trimmed)) return { type: 'turn_end' };

    // Model selector
    if (/^Select model$/.test(trimmed)) return { type: 'model_header' };
    // Model selector description/items/effort bar — skip all lines within selector
    if (/^Switch between Claude models/.test(trimmed)) return { type: 'model_item', text: trimmed };
    if (/^\d+\.\s+(Default|Sonnet|Opus|Haiku)\b/.test(trimmed)) return { type: 'model_item', text: trimmed };
    if (/^❯\s*\d+\.\s+(Default|Sonnet|Opus|Haiku)\b/.test(trimmed)) return { type: 'model_selected', text: trimmed };
    if (/^▌/.test(trimmed)) return { type: 'model_item', text: trimmed }; // effort bar
    if (/^(← →|For other)/.test(trimmed)) return { type: 'model_item', text: trimmed };

    // Permission prompt (edit/bash/tool confirmation)
    if (/^Do you want to (proceed|make this edit)/.test(trimmed)) return { type: 'permission_header', text: trimmed };
    if (/^This command requires approval/.test(trimmed)) return { type: 'skip' };
    if (/^(Bash command|Edit file)$/.test(trimmed)) return { type: 'skip' };
    // Permission options — ❯ N. selected or N. unselected
    {
      const selM = trimmed.match(/^❯\s*(\d+)\.\s+(.+)/);
      if (selM) return { type: 'permission_selected', index: parseInt(selM[1]), text: selM[2].trim() };
      const optM = trimmed.match(/^(\d+)\.\s+(Yes\b|No\b|Yes,).*/);
      if (optM) return { type: 'permission_option', index: parseInt(optM[1]), text: optM[0].replace(/^\d+\.\s+/, '').trim() };
    }

    // User input (via marker — real submitted input with bg color)
    if (trimmed.includes('\x00CCUSER\x00')) {
      let text = trimmed.replace(/^.*\x00CCUSER\x00\s*/, '').trim();
      let raw = rawLine.replace(/^.*\x00CCUSER\x00\s*/, '');
      // Strip trailing reset codes and bg artifacts
      text = stripAnsi(text).trim();
      // Skip /model commands (handled as model_header)
      if (/^\/model\b/.test(text)) return { type: 'skip' };
      if (!text) return { type: 'skip' };
      return { type: 'user', text, rawText: raw };
    }
    // User input continuation (white on dark bg, no ❯) — second line of multi-line input
    if (/\x1b\[38;2;255;255;255m\x1b\[48;2;55;55;55m/.test(rawLine) && !trimmed.includes('\x00CC')) {
      const text = stripAnsi(trimmed).trim();
      if (!text) return { type: 'skip' };
      return { type: 'user_continuation', text, rawText: rawLine };
    }

    // Agent text response (white ⏺)
    if (trimmed.includes('\x00CCAGENT\x00')) {
      const text = trimmed.replace(/^.*\x00CCAGENT\x00\s*/, '').trim();
      const raw = rawLine.replace(/^.*\x00CCAGENT\x00\s*/, '');
      return { type: 'agent', text, rawText: raw };
    }

    // Tool call — completed (green ⏺) or in-progress (gray ⏺)
    if (trimmed.includes('\x00CCTOOL\x00')) {
      const text = trimmed.replace(/^.*\x00CCTOOL\x00\s*/, '').trim();
      const raw = rawLine.replace(/^.*\x00CCTOOL\x00\s*/, '');
      return { type: 'tool', text, rawText: raw };
    }

    // Tool rejected/error (red ⏺)
    if (trimmed.includes('\x00CCTOOLFAIL\x00')) {
      const text = trimmed.replace(/^.*\x00CCTOOLFAIL\x00\s*/, '').trim();
      const raw = rawLine.replace(/^.*\x00CCTOOLFAIL\x00\s*/, '');
      return { type: 'tool', text, rawText: raw };
    }

    // Model confirmation after /model command (⎿  Kept model as... / Set model to...)
    if (/^⎿\s+(Kept model as|Set model to)\s/.test(trimmed)) {
      const m = trimmed.match(/(Kept model as|Set model to)\s+([\w.-]+)/);
      return { type: 'model_confirmed', text: m ? m[2] : 'model' };
    }

    // Tool sub-items (⎿ indented lines)
    if (/^⎿\s/.test(trimmed)) return { type: 'tool_result' };
    // Indented lines after tool rejection (User rejected...)
    if (/^User rejected/.test(trimmed)) return { type: 'tool_result' };

    // Diff lines within tool results (numbered: " N +..." or " N  ...")
    if (/^\d+\s+[+\-]/.test(trimmed)) return { type: 'tool_result' };

    // Empty line
    if (!trimmed) return { type: 'empty' };

    // Model/price info lines (in model selector context)
    if (/\$[\d.]+\/\$[\d.]+\s+per\s+Mtok/.test(trimmed)) return { type: 'model_item', text: trimmed };
    if (/per\s+Mtok$/.test(trimmed)) return { type: 'model_item', text: trimmed };

    // Fallback: ⏺ at line start without a color marker (e.g. terminal without RGB true-color support,
    // or banner has scrolled out of view so detect() fell through to ⏺ detection)
    if (/^⏺/.test(trimmed)) {
      const text = trimmed.replace(/^⏺\s*/, '').trim();
      return { type: 'agent', text, rawText: rawLine.replace(/^.*⏺\s*/, '') };
    }

    return { type: 'continuation' };
  },

  // Extract status info from pane content
  extractStatus(raw) {
    const clean = stripAnsi(raw);
    // Check if thinking/simmering
    if (/[✳✶✷✸✹✺]\s*(Simmering|Thinking|Brewing)/i.test(clean)) {
      return { thinking: true, tool: 'claude-code' };
    }
    return { tool: 'claude-code' };
  },

  // Detect if pane is waiting for user input
  isWaitingForInput(raw) {
    const tail = raw.slice(-500);
    // Idle prompt: ❯ with \x1b[7m (reverse video cursor) or ? for shortcuts
    if (/\?\s*for shortcuts/.test(stripAnsi(tail))) return true;
    // Check for the prompt ❯ at end without thinking indicator
    const clean = stripAnsi(tail);
    const lines = clean.split('\n').filter(l => l.trim());
    const last = lines.at(-1)?.trim() || '';
    if (/^❯\s*$/.test(last)) return true;
    if (/^\?\s*for shortcuts/.test(last)) return true;
    return false;
  },
};

// ─── Parser registry ───

const parsers = [claudeCodeParser, kiroParser];

export function detectParser(raw, command = '') {
  return parsers.find(p => p.detect(raw, command)) || null;
}

// ─── Generic message builder (works with any parser) ───

export function parseMessages(raw, parser) {
  if (!raw || !parser) return { messages: [], isThinking: false, isSummarizing: false };

  const marked = parser.insertMarkers(raw);
  const rawLines = marked.split('\n');
  const cleanLines = rawLines.map(l => stripAnsi(l));
  // Strip semantic markers from lines for display (markers are only used by classifyLine)
  function cleanMarkers(s) { return s.replace(/\x00\w+\x00/g, ''); }
  const messages = [];
  let current = null;
  let isThinking = false;
  let isSummarizing = false;
  let started = false;
  let lastRole = null;

  function flush() {
    if (current && current.lines.some(l => l.trim())) {
      lastRole = current.role;
      // Remove leading/trailing empty lines, collapse internal empty lines for user messages
      let lines = current.lines;
      let rawLines = current.rawLines;
      if (current.role === 'user') {
        lines = lines.filter(l => l.trim());
        rawLines = rawLines.filter((_, i) => current.lines[i]?.trim());
      }
      // For user messages, join single newlines as spaces (tmux line wrapping)
      const text = current.role === 'user'
        ? lines.join(' ').replace(/\s+/g, ' ').trim()
        : lines.join('\n').trim();
      const rawText = current.role === 'user'
        ? rawLines.join(' ').replace(/\s+/g, ' ').trim()
        : rawLines.join('\n').trim();
      messages.push({
        ...current,
        lines,
        rawLines,
        text,
        rawText,
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
      case 'reset':
        flush(); messages.length = 0; current = null; started = false; isThinking = false; isSummarizing = false;
        continue;
      case 'thinking': isThinking = true; continue;
      case 'summarizing': isSummarizing = true; continue;
      case 'turn_end': isThinking = false; isSummarizing = false; flush(); continue;
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
      case 'permission_header':
        isThinking = false; started = true; flush();
        current = { role: 'permission', lines: [cls.text], rawLines: [rawLine], options: [] };
        continue;
      case 'permission_selected':
      case 'permission_option':
        if (current?.role === 'permission') {
          current.options.push({ index: cls.index, text: cls.text, selected: cls.type === 'permission_selected' });
        }
        continue;
      case 'user':
        isThinking = false; started = true; flush();
        lastRole = null;
        current = { role: 'user', lines: [cls.text], rawLines: [cls.rawText] };
        continue;
      case 'agent':
        isThinking = false; started = true;
        // Each white ⏺ is a separate bubble
        flush();
        lastRole = null;
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
        current.lines.push(cls.text != null ? cls.text : cleanMarkers(line));
        current.rawLines.push(cls.rawText != null ? cls.rawText : cleanMarkers(rawLine));
        continue;
      case 'user_continuation':
        if (current?.role === 'user') { current.lines.push(cls.text || cleanMarkers(line)); current.rawLines.push(cls.rawText || cleanMarkers(rawLine)); }
        continue;
      case 'tool_result':
        if (current?.role === 'agent') {
          current.lines.push(cls.text != null ? cls.text : cleanMarkers(line));
          current.rawLines.push(cls.rawText != null ? cls.rawText : cleanMarkers(rawLine));
        }
        continue;
      case 'continuation':
        isThinking = false;
        if (!started) continue;
        if (current) {
          current.lines.push(cleanMarkers(line)); current.rawLines.push(cleanMarkers(rawLine));
        } else if (lastRole === 'user') {
          // Multi-line user input with blank lines — re-open user bubble
          current = { role: 'user', lines: [cleanMarkers(line)], rawLines: [cleanMarkers(rawLine)] };
        } else {
          current = { role: 'system', lines: [cleanMarkers(line)], rawLines: [cleanMarkers(rawLine)] };
        }
        continue;
    }
  }
  flush();
  return { messages, isThinking, isSummarizing };
}

export { stripAnsi };
