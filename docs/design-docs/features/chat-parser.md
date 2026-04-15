# Chat Parser Architecture

## Context
Need to render CLI agent output (Kiro CLI, Claude Code) as a messaging UI from raw terminal output.

## Decision
Pluggable parser registry (`parsers.js`) with `detectParser()` auto-selection. Parse ANSI color codes as semantic markers BEFORE stripping them.

## Parsers

### Kiro CLI Parser (active)
- Detection: `/kiro/i` test on pane command
- Color 93 (purple) = user prompt `>`
- Color 141 (light purple) = agent response `>`
- Color 240 (gray) = system hint (skipped)
- Line types: skip, user, agent, system, tool, tool_result, thinking, summarizing, compact_start, turn_end, empty, continuation, reset

### Claude Code Parser (disabled, `detect()` returns false)
- Detection: disabled — uses terminal mode instead
- Uses RGB true-color ANSI codes:
  - User prompt: gray `❯` on bg #373737 with white text
  - Agent response: white `⏺`
  - Tool completed: green `⏺`
  - Tool in-progress: gray `⏺`
  - Tool rejected: red/pink `⏺`
- Ghost suggestion detection (reverse video `\x1b[7m`)

## How It Works
- Insert marker tokens using color-specific regex, then strip ANSI for text parsing
- Message roles: user, agent, system (slash command output)
- Block types: text (markdown), code (fenced), tool (collapsible), diff (red/green lines)
- ANSI→HTML: 256-color palette, dark color readability adjustment (`ensureReadable`)

## Alternatives Considered
- **Text-only regex parsing**: Rejected — `>` appears in code, markdown quotes, and prompts. Too fragile.
- **tmux control mode**: Rejected — not all agents support it, adds complexity.

## Trade-offs
- Tightly coupled to specific CLI color schemes (breaks if agent changes colors)
- Requires `-e` flag on capture_pane (ANSI escapes must be preserved)
- Requires `-J` flag (soft-wrapped lines must be joined or messages get split)

## Lessons Learned
- Never strip ANSI too early — you lose semantic information needed for parsing
- The color reset `\e[39m` after `>` is NOT always present — make it optional in regex
- Real user-typed text has NO ANSI codes; system hints are always colored
- Braille spinner characters (`⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏`) should be filtered and replaced with CSS animation
- Test with real tmux output: `tmux capture-pane -p -e` to see actual ANSI codes
