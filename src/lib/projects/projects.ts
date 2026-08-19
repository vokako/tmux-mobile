// Display logic for the Projects section. Pure functions live here so the
// component stays markup: the row ordering and the window-chip rules are the
// parts worth testing, and `node --test` can reach them without a DOM.

import { AGENTS, paneAgent, type Agent, type PaneLike } from '../core/agents.ts';

export type SlotKind = 'shell' | 'agent';

export interface Slot {
  ord: number;
  window_name: string;
  cwd: string;
  kind: SlotKind;
  command?: string;
  auto_run: boolean;
  first_seen_at: number;
  /** Absent until the window has survived long enough to be restorable. */
  settled_at?: number;
}

export interface Project {
  id: string;
  name: string;
  path: string;
  session: string;
  adopted: boolean;
  autostart: boolean;
  created_at: number;
  last_up_at?: number;
  last_seen_at?: number;
  archived: boolean;
  /** The bus room this project's conversation lives in. Recorded on the project
   * (schema v8) rather than derived, so a rename cannot orphan the chat. */
  room?: string;
}

export interface ProjectRow {
  project: Project;
  slots: Slot[];
  live: boolean;
}

/** A window the user can expect back after `up` (see capture::SETTLE_SECS). */
export function isRestorable(slot: Slot): boolean {
  return typeof slot.settled_at === 'number';
}

/** Minimal pane shape needed to build a live window chip. */
export type ChipPane = PaneLike & {
  session: string;
  window: number;
  pane: number;
  active?: boolean;
};

export interface WindowChip {
  name: string;
  /** tmux window index — null for a declared window that is not running. */
  window: number | null;
  /** `session:window.pane`, or null when there is nothing to open yet. */
  target: string | null;
  agentIcon: string | null;
  agentTag: string | null;
}

/** The AGENTS entry for a backend name we stored on a slot (`kiro`, `codex`…). */
export function agentByBackend(backend: string | null | undefined): Agent | null {
  if (!backend) return null;
  return AGENTS.find((a) => a.tag.toLowerCase() === backend.toLowerCase()) ?? null;
}

/**
 * Chips for a LIVE project: one per tmux window, taken from its active pane, so
 * tapping one opens exactly that window. This is the source of truth while the
 * session exists — including windows that have not settled into the declaration
 * yet, which you can still want to jump into.
 */
export function liveWindowChips(panes: ChipPane[]): WindowChip[] {
  const byWindow = new Map<number, ChipPane>();
  for (const p of panes) {
    const seen = byWindow.get(p.window);
    if (!seen || (p.active && !seen.active)) byWindow.set(p.window, p);
  }
  return [...byWindow.entries()]
    .sort((a, b) => a[0] - b[0])
    .map(([window, pane]) => {
      const agent = paneAgent(pane);
      return {
        name: pane.window_name || String(window),
        window,
        target: `${pane.session}:${pane.window}.${pane.pane}`,
        agentIcon: agent?.icon ?? null,
        agentTag: agent?.tag ?? null,
      };
    });
}

/**
 * Chips for a project that is DOWN: the windows `up` would recreate, in window
 * order. Unsettled windows are left out on purpose — showing a window we would
 * not restore would promise something the reconciler does not deliver. They have
 * no target: there is nothing to open until the project is up.
 */
export function declaredWindowChips(slots: Slot[]): WindowChip[] {
  return slots
    .filter(isRestorable)
    .slice()
    .sort((a, b) => a.ord - b.ord)
    .map((s) => {
      const agent = s.kind === 'agent' ? agentByBackend(s.command) : null;
      return {
        name: s.window_name,
        window: null,
        target: null,
        agentIcon: agent?.icon ?? null,
        agentTag: agent?.tag ?? null,
      };
    });
}

/**
 * Order projects by the CONVERSATION, newest first — "把我们最近的对话默认排在最上
 * 面" (owner, 2026-08-19). `talk` maps room id → newest message timestamp (ms),
 * from `hub_rooms`.
 *
 * Why the conversation and not `last_seen_at`: the capturer writes `last_seen_at`
 * on every tick, so for a LIVE project it always means "just now" — every live
 * project floats to the top and their order is whichever one tmux was captured
 * last, which is no signal at all. A message timestamp only moves when somebody
 * actually says something.
 *
 * Projects nobody has talked in keep the old rule underneath (live first, then
 * when tmux last had the session, then creation): there is no conversation to
 * order them by, and they should not outrank one that exists.
 *
 * The `talk` argument is optional so the Projects page — which is about tmux and
 * declarations, not conversations — keeps its own ordering.
 */
export function sortRows(rows: ProjectRow[], talk: Record<string, number> = {}): ProjectRow[] {
  // seconds in the projects table, milliseconds on the bus: normalise before any
  // comparison, or a creation time looks like 1970 next to a message.
  const seen = (r: ProjectRow) =>
    (r.project.last_seen_at ?? r.project.last_up_at ?? r.project.created_at ?? 0) * 1000;
  const said = (r: ProjectRow) => talk[r.project.room ?? `proj:${r.project.session}`] ?? 0;
  return rows.slice().sort((a, b) => {
    const [sa, sb] = [said(a), said(b)];
    if (sa !== sb) return sb - sa;          // both talked, or one did: newest wins
    if (a.live !== b.live) return a.live ? -1 : 1;
    return seen(b) - seen(a);
  });
}

/**
 * Path label that fits a phone row: full path while it is short, otherwise the
 * last two segments with a leading ellipsis. The full path stays available as
 * the row's title attribute.
 */
export function shortPath(path: string, max = 34): string {
  if (path.length <= max) return path;
  const parts = path.replace(/\/+$/, '').split('/').filter(Boolean);
  if (parts.length <= 2) return path;
  return `…/${parts.slice(-2).join('/')}`;
}

/** Compact age label, matching the Sessions page vocabulary. */
export function ageLabel(unixSec: number | undefined, nowSec = Date.now() / 1000): string {
  if (!unixSec) return '';
  const d = Math.max(0, Math.floor(nowSec) - unixSec);
  if (d < 60) return 'now';
  if (d < 3600) return `${Math.round(d / 60)}m`;
  if (d < 86400) return `${Math.round(d / 3600)}h`;
  if (d < 86400 * 7) return `${Math.round(d / 86400)}d`;
  const date = new Date(unixSec * 1000);
  return `${date.getMonth() + 1}/${date.getDate()}`;
}
