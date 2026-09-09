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
  //
  // Every needle is WORD-BOUNDED (\b): these are short brand names that ride
  // inside ordinary words — "omp" lives in "compose", "kiro" in a window
  // named after the kirocrew project — and a substring hit painted plain
  // shells as agents. `-`, `.` and `/` are boundaries, so `kiro-cli-chat`,
  // `codex.js` and `/bin/omp` still match.
  { tag: 'Kimi',     match: /\bkimi\b/i,     icon: '/assets/kimi.svg',     iconSize: 14 },
  { tag: 'Kiro',     match: /\bkiro\b/i,     icon: '/assets/kiro.svg',     iconSize: 14 },
  // Claude Code's binary is a version-named symlink
  // (~/.local/share/claude/versions/2.1.141), so pane_current_command
  // reports "2.1.141" — no "claude" anywhere. The pane_title carries
  // "Claude Code" only when the shell doesn't overwrite the title (many
  // setups pin it to the hostname). Detect EITHER the word or a bare
  // semver-looking process name at the start of the command field.
  { tag: 'Claude',   match: /\bclaude\b|^\d+\.\d+\.\d+(?:\s|$)/i, icon: '/assets/claude.svg', iconSize: 14 },
  { tag: 'Codex',    match: /\bcodex\b/i,    icon: '/assets/codex.svg',    iconSize: 14 },
  { tag: 'Grok',     match: /\bgrok\b/i,     icon: '/assets/grok.svg',     iconSize: 14 },
  { tag: 'OpenClaw', match: /\bopenclaw\b/i, icon: '/assets/openclaw.svg', iconSize: 14 },
  // oh-my-pi's CLI: a single `omp` binary (ELF, so pane_current_command says
  // "omp" directly). The word boundary is what keeps docker-compose panes
  // from wearing its icon.
  { tag: 'OMP',      match: /\bomp\b/i,      icon: '/assets/omp.svg',      iconSize: 14 },
];

// ── The server's backend list (board #130) ──────────────────────────────
// The backends this server can spawn, with their resource names, come from
// `backends_list` once per connection (App.svelte fetches it beside probeHub
// on every connect/reconnect success and hands it here).
// `null` = not fetched yet, or an OLDER server without the method: every
// reader below falls back to the client's last hand-kept list, frozen at the
// shape that pre-#130 servers had. The detection-only CLIs (kimi, openclaw)
// are never in the server list — they are recognised in panes, not spawned —
// so their avatars stay in the fallback switch on purpose.
export interface BackendInfo {
  name: string;
  /** Avatar path, `/assets/<name>.svg`. */
  icon: string;
  /** Colour token NAME, `--backend-<name>` (values live in app.css). */
  color: string;
  /** Reasoning-effort levels the CLI accepts; '' (default) is the editor's. */
  efforts: string[];
}
let served: BackendInfo[] | null = null;
export function setServedBackends(list: BackendInfo[] | null): void {
  served = Array.isArray(list) && list.length > 0 ? list : null;
}
function servedBackend(backend: string | null | undefined): BackendInfo | null {
  const key = (backend ?? '').toLowerCase();
  return served?.find((b) => b.name === key) ?? null;
}
// Pre-#130 servers: the list the client validated against by hand. Frozen —
// a new backend appears through the server's list, not here.
const FALLBACK_BACKENDS = ['kiro', 'claude', 'codex', 'grok', 'omp'];
const FALLBACK_EFFORTS: Record<string, string[]> = {
  kiro: ['low', 'medium', 'high', 'xhigh', 'max'],
  claude: ['low', 'medium', 'high', 'xhigh', 'max'],
  codex: ['minimal', 'low', 'medium', 'high', 'xhigh'],
  grok: ['low', 'medium', 'high', 'xhigh'],
};
/** Backend names a registry agent can run on, in the server's order. */
export function spawnableBackends(): string[] {
  return served?.map((b) => b.name) ?? FALLBACK_BACKENDS;
}
/** The backend an absent field means — the server's first entry. */
export function defaultBackend(): string {
  return spawnableBackends()[0] ?? FALLBACK_BACKENDS[0]!;
}
/** The effort levels a backend's editor offers ('' default is added by the caller). */
export function backendEfforts(backend: string | null | undefined): readonly string[] {
  return servedBackend(backend)?.efforts ?? FALLBACK_EFFORTS[backend ?? ''] ?? [];
}
/** The backend's colour token NAME (`--backend-x`), or null when it has none. */
export function backendColorToken(backend: string | null | undefined): string | null {
  return servedBackend(backend)?.color ?? null;
}

// Backend id → the backend's avatar icon, for agent AVATARS (roster cards,
// registry rows, presets): the logo says which CLI an agent runs on at a
// glance, where a colored initial said nothing (owner, 2026-08-21: "agent的
// icon可以用backend的logo，不用字母了"). The served list answers first; the
// switch is the older-server fallback plus the detection-only CLIs. Null for
// a backend we ship no avatar for — callers keep the lettered fallback.
export function backendIcon(backend: string | null | undefined): string | null {
  const s = servedBackend(backend);
  if (s) return s.icon;
  switch ((backend ?? '').toLowerCase()) {
    case 'kiro': return '/assets/kiro.svg';
    case 'claude': return '/assets/claude.svg';
    case 'codex': return '/assets/codex.svg';
    case 'grok': return '/assets/grok.svg';
    case 'kimi': return '/assets/kimi.svg';
    case 'omp': return '/assets/omp.svg';
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
