export type LocationLike = { protocol: string; host: string };

export const STANDALONE_WS_DEFAULT = 'ws://127.0.0.1:9899';

export function defaultConnectionAddress(location: LocationLike, dev: boolean): string {
  if (!dev || !location.host) return STANDALONE_WS_DEFAULT;
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${scheme}//${location.host}/ws`;
}

/** A typed address as a ws URL: a bare host gets the scheme the page needs
 * (an https page can only open wss://). One rule for every connect form. */
export function normalizeAddress(addr: string, protocol: string): string {
  const a = addr.trim();
  if (a.startsWith('ws://') || a.startsWith('wss://')) return a;
  return (protocol === 'https:' ? 'wss://' : 'ws://') + a;
}
