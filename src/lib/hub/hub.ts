// Pure display logic for the Hub view — testable with node --test, no Svelte.
import type { HubAgent } from '../core/ws.ts';

/** Derived-state → dot color (theme token values are resolved in CSS; these
 * are the fallback literals used inline for dynamic dots). */
export function stateDotColor(state: string): string {
  switch (state) {
    case 'working': return 'var(--status-ok)';
    case 'waiting': return 'var(--status-warn)';
    case 'blocked':
    case 'stuck':
    case 'failed': return 'var(--status-danger)';
    case 'shell': return 'var(--text3)';
    default: return 'var(--status-sleep)'; // idle
  }
}

/** Merge new chat messages into the feed without duplicates, oldest first.
 * Pushes and cursor polls overlap; identity is the message id when present,
 * else (ts, from, body). */
export function mergeMessages<T extends { id?: string; ts?: number; from?: string; body?: string }>(
  existing: T[],
  incoming: T[],
): T[] {
  const key = (m: T) => m.id ?? `${m.ts}|${m.from}|${m.body}`;
  const seen = new Set(existing.map(key));
  const merged = existing.slice();
  for (const m of incoming) {
    if (!seen.has(key(m))) {
      seen.add(key(m));
      merged.push(m);
    }
  }
  merged.sort((a, b) => (a.ts ?? 0) - (b.ts ?? 0));
  return merged;
}

/** Backend identity colors (the prototype's palette): consistent across
 * sidebar, cards and registry so a glance identifies who is who. */
export function backendColor(backend: string | null | undefined): string {
  switch (backend) {
    case 'kiro': return '#a78bfa';
    case 'claude': return '#fb923c';
    case 'codex': return '#94a3b8';
    case 'kimi': return '#4ade80';
    default: return '#818cf8';
  }
}

export interface StatuslineWindow {
  window: number;
  label: string;
  current: boolean;
}

/** tmux's own notation: `2:reviewer*` marks the current window. Windows are
 * listed in index order; the `*` suffix goes to the window the terminal
 * column is showing. */
export function statuslineWindows(agents: HubAgent[], termTarget: string): StatuslineWindow[] {
  const m = /^.+:(\d+)\.\d+$/.exec(termTarget || '');
  const cur = m ? Number(m[1]) : -1;
  return agents
    .slice()
    .sort((a, b) => a.window - b.window)
    .map((a) => ({
      window: a.window,
      label: `${a.window}:${a.name}${a.window === cur ? '*' : ''}`,
      current: a.window === cur,
    }));
}
