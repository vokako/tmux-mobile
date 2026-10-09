export type LocationLike = { protocol: string; host: string };

export const STANDALONE_WS_DEFAULT = 'ws://127.0.0.1:9899';

/** The address a fresh page starts with (a saved one always wins — the
 * caller only asks when nothing is saved). Vite dev: its own origin's /ws
 * proxy. A page the GATEWAY served (it carries `<meta name="tmm-gateway">`,
 * board #323): that gateway, on the same origin — host, port and ws/wss
 * all come from the page's own URL, never from anything in the HTML. */
export function defaultConnectionAddress(location: LocationLike, dev: boolean, hosted = false): string {
  if (!location.host) return STANDALONE_WS_DEFAULT;
  const scheme = location.protocol === 'https:' ? 'wss:' : 'ws:';
  if (hosted && (location.protocol === 'http:' || location.protocol === 'https:')) return `${scheme}//${location.host}`;
  if (!dev) return STANDALONE_WS_DEFAULT;
  return `${scheme}//${location.host}/ws`;
}

/** Was this page served by a tmux-mobile gateway? */
export function hostedByGateway(doc: { querySelector: (s: string) => unknown } | undefined): boolean {
  return !!doc?.querySelector('meta[name="tmm-gateway"]');
}

export type ConnectFieldsState = { address: string; token: string; socket: string };
export type LocalConfig = { url?: string; token?: string; tmux_socket?: string | null };

/** What the desktop app may fill in from this machine's config (board
 * #323): a field only when nothing is SAVED for it and the person has not
 * changed it since the page opened (`initial` vs `current` — the config
 * arrives asynchronously). Address, token and socket are judged apart: a
 * saved address with no token keeps its address. */
export function localAutofill(saved: { address: string | null; token: string | null; socket: string | null }, initial: ConnectFieldsState, current: ConnectFieldsState, cfg: LocalConfig): Partial<ConnectFieldsState> {
  const out: Partial<ConnectFieldsState> = {};
  if (!saved.address && current.address === initial.address && cfg.url) out.address = cfg.url;
  if (!saved.token && current.token === initial.token && cfg.token) out.token = cfg.token;
  if (!saved.socket && current.socket === initial.socket && cfg.tmux_socket) out.socket = cfg.tmux_socket;
  return out;
}

/** A typed address as a ws URL: a bare host gets the scheme the page needs
 * (an https page can only open wss://). One rule for every connect form. */
export function normalizeAddress(addr: string, protocol: string): string {
  const a = addr.trim();
  if (a.startsWith('ws://') || a.startsWith('wss://')) return a;
  return (protocol === 'https:' ? 'wss://' : 'ws://') + a;
}
