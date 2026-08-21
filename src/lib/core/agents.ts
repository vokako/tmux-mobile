// Known coding-agent CLIs that tmux-mobile treats as "AI sessions".
// Adding a new agent = one entry here + the matching icon in /assets/.
// Icons are lobehub-style AVATARS (visible on light AND dark themes):
// @lobehub/icons is React-only and its static packages ship no avatar
// files, so we compose them ourselves as static SVGs — circle filled with
// the brand's AVATAR_BACKGROUND + the official mark scaled by
// AVATAR_ICON_MULTIPLE (constants from @lobehub/icons es/<Name>/style.js,
// MIT). White-background avatars get a hairline ring so they read on
// light surfaces.
//
// Detection is intentionally loose (case-insensitive substring) because tmux's
// pane_current_command reports the short process name (e.g. "kiro-cli-chat")
// while pane_title often carries the full argv line. We check both.

export interface Agent {
  tag: string;
  match: RegExp;
  icon: string;
  iconSize: number;
}
// Minimal pane shape needed for detection; real panes (ws.ts TmuxPane)
// satisfy it, and so do partial objects in tests.
export type PaneLike = {
  current_command?: string;
  pane_title?: string;
  child_cmd?: string;
  window_name?: string;
} | null | undefined;

export const AGENTS: Agent[] = [
  // Kimi Code runs as `kimi-code`. It must match BEFORE the /kiro/ entry
  // can fire: a kimi pane's child chain typically contains its
  // "kiro-web-search" helper, and "kimi" in current_command always sits
  // earlier in the pane text than any child-chain "kiro".
  { tag: 'Kimi',     match: /kimi/i,     icon: '/assets/kimi.svg',     iconSize: 14 },
  { tag: 'Kiro',     match: /kiro/i,     icon: '/assets/kiro.svg',     iconSize: 14 },
  // Claude Code's binary is a version-named symlink
  // (~/.local/share/claude/versions/2.1.141), so pane_current_command
  // reports "2.1.141" — no "claude" anywhere. The pane_title carries
  // "Claude Code" only when the shell doesn't overwrite the title (many
  // setups pin it to the hostname). Detect EITHER the word or a bare
  // semver-looking process name at the start of the command field.
  { tag: 'Claude',   match: /claude|^\d+\.\d+\.\d+(?:\s|$)/i, icon: '/assets/claude.svg', iconSize: 14 },
  { tag: 'Codex',    match: /codex/i,    icon: '/assets/codex.svg',    iconSize: 14 },
  { tag: 'Grok',     match: /grok/i,     icon: '/assets/grok.svg',     iconSize: 14 },
  { tag: 'OpenClaw', match: /openclaw/i, icon: '/assets/openclaw.svg', iconSize: 14 },
];

// Backend id → the backend's avatar icon, for agent AVATARS (roster cards,
// registry rows, presets): the logo says which CLI an agent runs on at a
// glance, where a colored initial said nothing (owner, 2026-08-21: "agent的
// icon可以用backend的logo，不用字母了"). Falls back to null for a backend we
// ship no avatar for — callers keep the lettered fallback.
export function backendIcon(backend: string | null | undefined): string | null {
  switch ((backend ?? '').toLowerCase()) {
    case 'kiro': return '/assets/kiro.svg';
    case 'claude': return '/assets/claude.svg';
    case 'codex': return '/assets/codex.svg';
    case 'grok': return '/assets/grok.svg';
    case 'kimi': return '/assets/kimi.svg';
    case 'openclaw': return '/assets/openclaw.svg';
    default: return null;
  }
}

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
export function detectAgent(text: string | null | undefined): Agent | null {
  if (!text) return null;
  let best: Agent | null = null;
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
export function paneText(p: PaneLike): string {
  if (!p) return '';
  return (p.current_command || '') + ' ' + (p.pane_title || '') + ' ' + (p.child_cmd || '');
}

// Agent entry for a pane (or null).
export function paneAgent(p: PaneLike): Agent | null {
  return detectAgent(paneText(p));
}

export function paneChipLabel(p: PaneLike, fallback = ''): string {
  if (paneAgent(p)) return '';
  return p?.current_command || p?.window_name || fallback;
}

// Convenience: "is this pane running an AI CLI?"
export function paneIsAgent(p: PaneLike): boolean {
  return paneAgent(p) !== null;
}

// Convenience: "does any pane in this session run an AI CLI?" Requires the
// caller to have already fetched panes[sessionName].
export function sessionHasAgent(panes: PaneLike[] | null | undefined): boolean {
  return Array.isArray(panes) && panes.some(paneIsAgent);
}
