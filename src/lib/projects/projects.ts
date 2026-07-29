// Display logic for the Projects section. Pure functions live here so the
// component stays markup: the row ordering and the window-chip rules are the
// parts worth testing, and `node --test` can reach them without a DOM.

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
}

export interface ProjectRow {
  project: Project;
  slots: Slot[];
  live: boolean;
}

export interface SnapshotMeta {
  id: number;
  at: number;
  windows: string[];
}

/** A window the user can expect back after `up` (see capture::SETTLE_SECS). */
export function isRestorable(slot: Slot): boolean {
  return typeof slot.settled_at === 'number';
}

/**
 * Chips under a project row: the windows `up` would recreate, in window order.
 * Unsettled windows are left out on purpose — showing a window we would not
 * restore would promise something the reconciler does not deliver.
 */
export function windowChips(slots: Slot[]): { name: string; agent: string | null }[] {
  return slots
    .filter(isRestorable)
    .slice()
    .sort((a, b) => a.ord - b.ord)
    .map((s) => ({
      name: s.window_name,
      agent: s.kind === 'agent' ? s.command || null : null,
    }));
}

/**
 * Live projects first (they are what you are working in), then by recency.
 * `last_seen_at` is written by the capturer, so it means "when tmux last had
 * this session", which is a better recency signal than creation time.
 */
export function sortRows(rows: ProjectRow[]): ProjectRow[] {
  const recency = (r: ProjectRow) =>
    r.project.last_seen_at ?? r.project.last_up_at ?? r.project.created_at;
  return rows.slice().sort((a, b) => {
    if (a.live !== b.live) return a.live ? -1 : 1;
    return recency(b) - recency(a);
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
