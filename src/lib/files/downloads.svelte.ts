// The ONE record of this session's downloads (board #308). Every attempt
// registers a row here and its progress updates it; the Files feedback slot
// and the Downloads view both READ it, so the toast and the list can never
// tell two stories. Module-level: both Files instances (the page and the Hub
// drawer) and every attempt share it. In memory: the disk keeps what must
// outlive the app (finished files, resumable parts), and the view reads that
// from the shell.

import type { FeedbackValue } from '../ui/feedback-lifetime.ts';

export type DownloadState = 'downloading' | 'saving' | 'done' | 'failed' | 'paused';

export interface DownloadRow {
  /** The part id (native) or a session id (browser). */
  id: string;
  name: string;
  /** The file's path on the server: Retry and Resume start from it. */
  path: string;
  state: DownloadState;
  received: number;
  total: number;
  startedAt: number;
  /** Where this attempt's first progress report found the transfer (a
   * resume starts at the part's size); speed counts only what it moved. */
  base: { at: number; received: number } | null;
  finishedAt: number | null;
  savedPath: string | null;
  error: string | null;
  /** The part on disk was of an older version: this attempt began again. */
  restarted: boolean;
  /** A browser download: handed to the browser, no file path of ours. */
  requested: boolean;
}

const rows = $state<DownloadRow[]>([]);

export const downloads = {
  get rows(): readonly DownloadRow[] { return rows; },
  /** Rows a transfer is running for. */
  get active(): number { return rows.filter((r) => r.state === 'downloading' || r.state === 'saving').length; },
};

export function rowOf(id: string): DownloadRow | undefined {
  return rows.find((r) => r.id === id);
}

/** A new attempt. A row for the same id (a resume, a retry) is reused, so one
 * file is one row, moved back to the top. */
export function begin(id: string, name: string, path: string, now = Date.now()): DownloadRow {
  const old = rows.findIndex((r) => r.id === id);
  if (old >= 0) rows.splice(old, 1);
  rows.unshift({
    id, name, path, state: 'downloading', received: 0, total: 0, startedAt: now,
    base: null, finishedAt: null, savedPath: null, error: null,
    restarted: false, requested: false,
  });
  return rows[0]!;
}

export function progress(id: string, received: number, total: number, now = Date.now()) {
  const row = rowOf(id);
  if (!row) return;
  if (!row.base) row.base = { at: now, received };
  row.received = received;
  row.total = total;
}

export function restarted(id: string) {
  const row = rowOf(id);
  if (!row) return;
  row.restarted = true;
  row.received = 0;
  row.base = null;
}

export function saving(id: string) {
  const row = rowOf(id);
  if (row) row.state = 'saving';
}

export function done(id: string, savedPath: string | null, now = Date.now(), requested = false) {
  const row = rowOf(id);
  if (!row) return;
  Object.assign(row, { state: 'done', savedPath, finishedAt: now, requested, error: null });
  if (row.total) row.received = row.total;
}

/** The transfer stopped and its part stays on disk to be resumed. */
export function paused(id: string, error: string | null, now = Date.now()) {
  const row = rowOf(id);
  if (row) Object.assign(row, { state: 'paused', error, finishedAt: now });
}

export function failed(id: string, error: string, now = Date.now()) {
  const row = rowOf(id);
  if (row) Object.assign(row, { state: 'failed', error, finishedAt: now });
}

/** Cancelled or deleted: nothing of it is left to show. */
export function forget(id: string) {
  const at = rows.findIndex((r) => r.id === id);
  if (at >= 0) rows.splice(at, 1);
}

/** Bytes per second of the current attempt, or null before one second. */
export function speed(row: DownloadRow, now = Date.now()): number | null {
  if (!row.base) return null;
  const seconds = (now - row.base.at) / 1000;
  if (seconds < 1 || row.received <= row.base.received) return null;
  return (row.received - row.base.received) / seconds;
}

export interface FeedbackStrings { downloading: string; changed: string; saving: string }

/** The feedback slot's progress value for a row: the slot shows the store,
 * it keeps no copy of its own. */
export function feedbackOf(row: DownloadRow, s: FeedbackStrings): FeedbackValue {
  return {
    kind: 'progress', glyph: 'download',
    message: row.state === 'saving' ? s.saving : row.restarted ? s.changed : s.downloading,
    detail: row.path,
    progress: row.state === 'saving' || !row.total ? null : (row.received / row.total) * 100,
  };
}

/** Test seam: the store is module state. */
export function resetForTests() { rows.splice(0, rows.length); }
