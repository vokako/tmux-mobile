// Known coding-agent CLIs that tmux-mobile treats as "AI sessions".
// Adding a new agent = one entry here + the matching icon in /assets/.
//
// Detection is intentionally loose (case-insensitive substring) because tmux's
// pane_current_command reports the short process name (e.g. "kiro-cli-chat")
// while pane_title often carries the full argv line. We check both.

export const AGENTS = [
  { tag: 'Kiro',     match: /kiro/i,     icon: '/assets/kiro.svg',     iconSize: 14 },
  { tag: 'Claude',   match: /claude/i,   icon: '/assets/claude.svg',   iconSize: 16 },
  { tag: 'OpenClaw', match: /openclaw/i, icon: '/assets/openclaw.svg', iconSize: 14 },
];

// Return the matching AGENTS entry for a blob of text (current_command,
// pane_title, or a combination), or null if none match.
export function detectAgent(text) {
  if (!text) return null;
  for (const a of AGENTS) if (a.match.test(text)) return a;
  return null;
}

// Convenience: "is this pane running an AI CLI?"
export function paneIsAgent(p) {
  if (!p) return false;
  return detectAgent((p.current_command || '') + ' ' + (p.pane_title || '')) !== null;
}

// Convenience: "does any pane in this session run an AI CLI?" Requires the
// caller to have already fetched panes[sessionName].
export function sessionHasAgent(panes) {
  return Array.isArray(panes) && panes.some(paneIsAgent);
}
