// This computer's server, as the desktop app found it at start (board #323):
// `server_mode` (lib.rs) and every `server_mode_changed` event carry one
// snapshot {mode, gen, seq}; `newerMode` keeps only a strictly newer seq, so
// arrival order never matters. No polling. The wording rule is pure.

export interface ServerMode { mode: 'embedded' | 'gateway' | 'occupied' | 'starting' | 'failed' | string; url: string; reason?: string; gen?: number; seq?: number }

/** Keep `next` only when it is strictly newer (board #323 review): the read
 * and the events carry one `seq`, minted in Rust's lock, so whatever order
 * they ARRIVE in, an older snapshot never replaces a newer one. A snapshot
 * without a seq is never newer. */
export function newerMode(cur: ServerMode | null, next: ServerMode | null | undefined): ServerMode | null {
  if (!next || typeof next.seq !== 'number') return cur;
  return !cur || (cur.seq ?? 0) < next.seq ? next : cur;
}
export type HoverLine = { label: string; value: string; tone?: 'ok' | 'warn' | 'danger' };

/** The server card's "This computer" line, or null outside the desktop app.
 * The label says it is about this machine, not the connected server. */
export function localServerLine(m: ServerMode | null | undefined, t: (k: string) => string): HoverLine | null {
  if (!m || !m.mode) return null;
  const label = t('localServer');
  switch (m.mode) {
    case 'gateway': return { label, value: `${t('localServerGateway')} · ${m.url}` };
    case 'embedded': return { label, value: `${t('localServerEmbedded')} · ${m.url}` };
    // Achromatic: a moment on the way to embedded, not a warning.
    case 'starting': return { label, value: `${t('localServerStarting')} · ${m.url}` };
    case 'occupied': return { label, value: `${t('localServerOccupied')} · ${m.reason ?? m.url}`, tone: 'warn' };
    case 'failed': return { label, value: `${t('localServerFailed')} · ${m.reason ?? m.url}`, tone: 'danger' };
    default: return null;
  }
}
