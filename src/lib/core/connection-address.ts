export type LocationLike = { protocol: string; host: string };

export const STANDALONE_WS_DEFAULT = 'ws://127.0.0.1:9899';

export function defaultConnectionAddress(location: LocationLike, dev: boolean): string {
  if (!dev || !location.host) return STANDALONE_WS_DEFAULT;
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
  return `${scheme}//${location.host}/ws`;
}
