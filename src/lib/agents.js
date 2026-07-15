// Known coding-agent CLIs that tmux-mobile treats as "AI sessions".
// Adding a new agent = one entry here + the matching icon in /assets/.
//
// Detection is intentionally loose (case-insensitive substring) because tmux's
// pane_current_command reports the short process name (e.g. "kiro-cli-chat")
// while pane_title often carries the full argv line. We check both.

export const AGENTS = [
  { tag: 'Kiro',     match: /kiro/i,     icon: '/assets/kiro.svg',     iconSize: 14 },
  // Claude Code's binary is a version-named symlink
  // (~/.local/share/claude/versions/2.1.141), so pane_current_command
  // reports "2.1.141" — no "claude" anywhere. The pane_title carries
  // "Claude Code" only when the shell doesn't overwrite the title (many
  // setups pin it to the hostname). Detect EITHER the word or a bare
  // semver-looking process name at the start of the command field.
  { tag: 'Claude',   match: /claude|^\d+\.\d+\.\d+(?:\s|$)/i, icon: '/assets/claude.svg', iconSize: 16 },
  { tag: 'Codex',    match: /codex/i,    icon: '/assets/codex.svg',    iconSize: 14 },
  { tag: 'OpenClaw', match: /openclaw/i, icon: '/assets/openclaw.svg', iconSize: 14 },
];

// Return the matching AGENTS entry for a blob of text (current_command,
// pane_title, or a combination), or null if none match.
//
// When several agents match, the one whose match sits EARLIEST in the text
// wins — not the one listed first in AGENTS. paneText orders its parts
// shallow→deep (command, title, then the pane's process chain from the
// shell downward), so an early match is the process the user actually
// launched, while a late match is a subprocess. Real case: codex spawning
// a "kiro-web-search" MCP tool put "kiro" deep in the chain and the
// array-order rule painted the session as Kiro.
export function detectAgent(text) {
  if (!text) return null;
  let best = null;
  let bestIdx = Infinity;
  for (const a of AGENTS) {
    const idx = text.search(a.match);
    if (idx >= 0 && idx < bestIdx) {
      best = a;
      bestIdx = idx;
    }
  }
  return best;
}

// All detection-relevant text for a pane, in one place. `child_cmd` is the
// pane shell's descendant argv reported by the server — the only reliable
// signal for interpreter-launched CLIs (codex runs as plain "node"; claude
// as a bare version number). current_command/pane_title alone miss those.
export function paneText(p) {
  if (!p) return '';
  return (p.current_command || '') + ' ' + (p.pane_title || '') + ' ' + (p.child_cmd || '');
}

// Agent entry for a pane (or null).
export function paneAgent(p) {
  return detectAgent(paneText(p));
}

export function paneChipLabel(p, fallback = '') {
  if (paneAgent(p)) return '';
  return p?.current_command || p?.window_name || fallback;
}

// Convenience: "is this pane running an AI CLI?"
export function paneIsAgent(p) {
  return paneAgent(p) !== null;
}

// Convenience: "does any pane in this session run an AI CLI?" Requires the
// caller to have already fetched panes[sessionName].
export function sessionHasAgent(panes) {
  return Array.isArray(panes) && panes.some(paneIsAgent);
}
