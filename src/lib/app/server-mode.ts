// This computer's server, as the desktop app found it at start (board #323):
// `server_mode` (lib.rs) answers what the start probe decided. Pure, so the
// wording rule is testable.

export interface ServerMode { mode: 'embedded' | 'gateway' | 'occupied' | 'starting' | 'failed' | string; url: string; reason?: string }
export type HoverLine = { label: string; value: string; tone?: 'ok' | 'warn' | 'danger' };

/** The server card's "This computer" line, or null outside the desktop app.
 * The label says it is about this machine, not the connected server. */
export function localServerLine(m: ServerMode | null | undefined, t: (k: string) => string): HoverLine | null {
  if (!m || !m.mode) return null;
  const label = t('localServer');
  switch (m.mode) {
    case 'gateway': return { label, value: `${t('localServerGateway')} · ${m.url}` };
    case 'embedded': return { label, value: `${t('localServerEmbedded')} · ${m.url}` };
    case 'starting': return { label, value: `${t('localServerStarting')} · ${m.url}`, tone: 'warn' };
    case 'occupied': return { label, value: `${t('localServerOccupied')} · ${m.reason ?? m.url}`, tone: 'warn' };
    case 'failed': return { label, value: `${t('localServerFailed')} · ${m.reason ?? m.url}`, tone: 'danger' };
    default: return null;
  }
}
