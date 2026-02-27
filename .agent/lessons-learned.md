# Lessons Learned: Terminal Output → Chat UI Parsing

## Core Challenge
Parsing raw terminal output (with ANSI escape sequences) from a tmux pane into structured chat messages. The terminal is a flat stream of characters — no semantic structure, no API, just rendered text.

## Key Insights

### 1. ANSI Colors Are the Best Semantic Markers
**Problem**: Text-only parsing (regex on stripped text) is fragile. A `>` character appears in code, markdown quotes, and actual prompts.

**Solution**: Use ANSI color codes as semantic delimiters BEFORE stripping them.
- Kiro CLI colors `>` differently: color 93 (purple) for user prompt, color 141 (light purple) for agent response
- Insert `\x00AGENT\x00` / `\x00UPROMPT\x00` markers using color-specific regex, then strip ANSI for text parsing
- This cleanly separates "user typed this" from "agent said this" from "code contains >"

**Gotcha**: The color reset `\e[39m` after `>` is NOT always present. Sometimes the next color starts immediately (e.g., `\e[38;5;93m> \e[38;5;240mhint text`). Make the reset optional in the regex: `(\x1b\[39m)?`

### 2. Soft-Wrapped Lines vs Real Newlines
**Problem**: tmux `capture-pane` wraps output at screen width. A single long user message becomes multiple lines, breaking message boundary detection.

**Solution**: Use `capture-pane -J` flag to join soft-wrapped lines. This gives the original line breaks, not screen-width artifacts.

**Impact**: Without `-J`, a user message like "帮我分析一下这个项目" gets split into 3 lines, and lines 2-3 get misclassified as system output.

### 3. System Hints vs User Input
**Problem**: Kiro CLI shows a placeholder hint at the prompt (`5% > Need help with features?`). This has the user prompt marker but isn't real user input.

**Solution**: After extracting text following the UPROMPT marker, check if the raw text starts with an ANSI escape (`\x1b[`). Real user-typed text has NO ANSI codes. System hints are always colored.

### 4. Preamble Skipping
**Problem**: Before the first conversation, the terminal shows shell prompts, `kiro-cli` header, MCP init lines, warnings. These shouldn't appear in chat.

**Solution**: `started` flag — skip all lines until the first `user` or `agent` classification. Also explicitly skip known patterns: `○`, `⠋`, `kiro-cli`, `Warning:`, `--More--`.

### 5. Message Boundary Detection
**Problem**: When does one message end and the next begin?

**Kiro CLI pattern**:
- `XX% !> text` = user input (percentage + optional `!` + `>` + text)
- `>` or `> text` (color 141) = agent response start — each `>` is a NEW bubble
- `▸ Credits: X.XX • Time: Xs` = end of turn
- Empty line after user message = user message complete
- Lines without markers after user = continuation (tmux word wrap)
- Lines without markers after flush = system output (slash command results)

### 6. Thinking Spinner Handling
**Problem**: `⠋ Thinking...` / `⠸ Thinking...` lines with braille spinners cause chat bubbles to flicker as the spinner character changes every frame.

**Solution**: Filter spinner lines (`/^[⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏]\s*Thinking/i`), track `isThinking` state, show a CSS spinner animation instead. Reset `isThinking` when real content appears.

### 7. ANSI Color Rendering in Chat Bubbles
**Problem**: Agent responses contain colored text (file paths in purple, tool names in gray, etc.). Stripping ANSI loses this information.

**Solution**: Keep raw ANSI in message `rawText`, convert to HTML spans at render time with `ansiToHtml()`. Support full 256-color palette. Apply `ensureReadable()` to brighten dark colors (luminance < 25%) for readability on dark backgrounds.

### 8. Diff Block Detection
**Problem**: Kiro CLI shows file diffs with `+ NNN:` and `- NNN:` format. These look similar to markdown lists (`- item`).

**Solution**: Require `:` after the line number: `/^[+\-]\s+\d+\s*:/`. This prevents `- 100dvh 放在...` from being misclassified as a diff line.

## Parser Architecture for Extensibility

Each parser implements:
```js
{
  name: 'kiro-cli',
  detect(raw) {},           // Does this pane run this tool?
  insertMarkers(raw) {},    // Replace ANSI color markers with semantic tokens
  classifyLine(trimmed, rawLine) {},  // Classify each line by type
  extractStatus(raw) {},    // Extract status bar info (context %, tool name)
  isWaitingForInput(raw) {} // Is the prompt visible?
}
```

The generic `parseMessages(raw, parser)` function handles the state machine (flush, started, isThinking) — parser only classifies individual lines.

To add a new CLI tool (e.g., Claude Code):
1. Create a new parser object with the 5 methods
2. Add to `const parsers = [kiroParser, newParser]`
3. No changes needed in ChatView.svelte or Terminal.svelte

## Common Pitfalls
- **Don't strip ANSI too early** — you lose semantic information
- **Don't assume `\e[39m` reset** — colors can chain without reset
- **Test with real tmux output** — `tmux capture-pane -p -e` to see actual ANSI codes, `cat -v` to inspect
- **Account for tmux screen width** — use `-J` flag or messages get split
- **Empty lines matter** — they're message boundaries, not just whitespace
- **Scrollback can lose colors** — always have text-only fallback parsing
