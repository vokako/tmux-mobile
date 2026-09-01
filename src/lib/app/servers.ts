// Named server configurations — the multi-server registry (board #55).
//
// The client always had exactly ONE server in its keys (`tmux_address` /
// `tmux_token` / `tmux_socket`), with `tmux_address_history` as an unnamed
// recents list behind the Settings form. This module makes the servers a
// first-class REGISTRY (`tmux_servers`, named entries with stable ids) while
// deliberately keeping the old keys as the ACTIVE MIRROR: everything that
// already reads them — ws.ts, the reconnect machine, deep links, the Settings
// form, the boot auto-connect — keeps working unchanged, and a downgraded
// client sees exactly the single-server world it expects.
//
// Switching servers is a PLAN of storage writes (`applySwitch`), applied and
// then followed by a full reload: the boot path (auto-connect + tmux_state
// restore) is the ONE place that knows how to bring the app up against a
// server, and every in-memory cache keyed to the old server (Hub room cache,
// mounted terminals, Files parked cwds, Team state) is reset by construction
// rather than by a sweep of per-component invalidations that would each be a
// cross-server contamination bug waiting to regress. Per-server nav state is
// parked under `tmux_state::<id>` so what you were looking at on server A is
// still there when you come back from server B (the "恢复目标不能串" half),
// and `tmux_machine_id` is parked the same way so A's failover set is never
// consulted while connected to B. `tmux_machines` itself stays GLOBAL — it is
// keyed by machineId, so entries cannot contaminate each other by design.
//
// Framework-free and storage-injected so migration, upsert and the switch
// plan are unit-testable without a browser.

export interface ServerEntry {
  id: string;
  name: string;
  /** The ACTIVE address — the one the last successful connect used. The same
   *  machine's other addresses stay in `tmux_machines[machineId]`, which is
   *  the existing failover set; this entry only picks the starting point. */
  address: string;
  token: string;
  socket?: string;
  /** The server's machine identity (ws.ts `getMachineId()` after connect).
   *  One machine = ONE entry, however many LAN/Tailscale/WAN addresses it
   *  answers on — address is how you REACH a server, machineId is WHICH
   *  server it is (lead review, board #55). Absent until first connect. */
  machineId?: string;
}

export const SERVERS_KEY = 'tmux_servers';
export const CURRENT_KEY = 'tmux_server_current';
/** Per-server parked nav state: `tmux_state::<id>` (the LIVE one stays in
 *  `tmux_state`, unprefixed — older code reads it there). */
export const STATE_PREFIX = 'tmux_state::';
/** Per-server parked machine id, same shape. */
export const MACHINE_PREFIX = 'tmux_machine_id::';
export const MAX_SERVERS = 16;

type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

export function serverId(): string {
  return Date.now().toString(36) + Math.random().toString(36).slice(2, 8);
}

/** A short human name out of a ws address: host without scheme/port/path.
 *  `ws://192.168.1.5:9899` → `192.168.1.5`; `wss://mac.tail.ts.net/ws` →
 *  `mac.tail.ts.net`. The fallback keeps whatever was typed. */
export function hostLabel(address: string): string {
  const m = /^wss?:\/\/([^/:?#]+)/.exec(address.trim());
  return m?.[1] || address.trim();
}

function sanitize(raw: unknown): ServerEntry[] {
  if (!Array.isArray(raw)) return [];
  const seen = new Set<string>();
  const out: ServerEntry[] = [];
  for (const e of raw) {
    if (!e || typeof e !== 'object') continue;
    const { id, name, address, token, socket, machineId } = e as Record<string, unknown>;
    if (typeof id !== 'string' || !id || seen.has(id)) continue;
    if (typeof address !== 'string' || !address) continue;
    seen.add(id);
    out.push({
      id,
      name: typeof name === 'string' && name ? name : hostLabel(address),
      address,
      token: typeof token === 'string' ? token : '',
      ...(typeof socket === 'string' && socket ? { socket } : {}),
      ...(typeof machineId === 'string' && machineId ? { machineId } : {}),
    });
  }
  return out.slice(0, MAX_SERVERS);
}

export function loadServers(storage: Store): ServerEntry[] {
  try { return sanitize(JSON.parse(storage.getItem(SERVERS_KEY) || '[]')); }
  catch { return []; }
}

export function saveServers(storage: Store, servers: ServerEntry[]): void {
  storage.setItem(SERVERS_KEY, JSON.stringify(servers.slice(0, MAX_SERVERS)));
}

export function currentServerId(storage: Store): string {
  return storage.getItem(CURRENT_KEY) || '';
}

/**
 * One-time, idempotent migration from the single-server keys.
 *
 * Runs before the boot auto-connect. If the registry already exists it is a
 * no-op (re-running must never duplicate or reorder). Otherwise the CURRENT
 * connection (`tmux_address`/`tmux_token`/`tmux_socket`) becomes the first,
 * current entry — the "don't lose the current user" clause — stamped with
 * `tmux_machine_id`, and the Settings recents (`tmux_address_history`) follow
 * as additional named entries — EXCEPT addresses `tmux_machines` attributes
 * to a machine that already has an entry: those are the SAME server's
 * LAN/Tailscale/WAN alternates (the failover set), and splitting them into
 * "servers" would break the multi-address semantics this feature must not
 * touch (lead review). A history machine seen for the first time yields one
 * entry (its first-seen address as the active one); unattributed addresses
 * dedupe by address as before. A client with no stored connection at all
 * migrates to an empty registry and the Settings form remains the front door.
 */
export function migrateServers(storage: Store): ServerEntry[] {
  if (storage.getItem(SERVERS_KEY) != null) return loadServers(storage);
  const servers: ServerEntry[] = [];
  // machineId → its addresses (the failover map is the identity authority).
  let machines: Record<string, string[]> = {};
  try {
    const raw = JSON.parse(storage.getItem('tmux_machines') || '{}');
    if (raw && typeof raw === 'object') machines = raw;
  } catch { /* unreadable map — address-level dedupe still applies */ }
  const ownerOf = (addr: string): string => {
    for (const [mid, addrs] of Object.entries(machines)) {
      if (Array.isArray(addrs) && addrs.includes(addr)) return mid;
    }
    return '';
  };
  const covered = (addr: string): boolean => {
    const mid = ownerOf(addr);
    return servers.some((s) => s.address === addr || (mid && s.machineId === mid));
  };

  const addr = storage.getItem('tmux_address') || '';
  if (addr) {
    const mid = storage.getItem('tmux_machine_id') || ownerOf(addr);
    const cur: ServerEntry = {
      id: serverId(),
      name: hostLabel(addr),
      address: addr,
      token: storage.getItem('tmux_token') || '',
      ...(mid ? { machineId: mid } : {}),
    };
    const socket = storage.getItem('tmux_socket') || '';
    if (socket) cur.socket = socket;
    servers.push(cur);
    storage.setItem(CURRENT_KEY, cur.id);
  }
  try {
    const hist = JSON.parse(storage.getItem('tmux_address_history') || '[]');
    if (Array.isArray(hist)) {
      for (const h of hist) {
        const a = typeof h === 'string' ? h : (h && typeof h.address === 'string' ? h.address : '');
        if (!a || covered(a)) continue;
        const mid = ownerOf(a);
        servers.push({
          id: serverId(), name: hostLabel(a), address: a,
          token: typeof h?.token === 'string' ? h.token : '',
          ...(mid ? { machineId: mid } : {}),
        });
      }
    }
  } catch { /* malformed history — the current entry alone is the migration */ }
  saveServers(storage, servers.slice(0, MAX_SERVERS));
  return loadServers(storage);
}

/**
 * Record a connection the user just made by hand (the Settings form, a deep
 * link) or that just authenticated. Identity is the MACHINE when known: a
 * `machineId` match updates that entry in place — active address, token,
 * socket — however different the address looks (LAN vs Tailscale vs WAN is
 * the same server). Otherwise it matches by address (and stamps a provided
 * machineId onto a pre-connect entry), and only then creates a new entry.
 * Existing names are kept — a rename must survive reconnects. Returns the
 * updated list.
 */
export function upsertServer(
  storage: Store,
  conn: { address: string; token: string; socket?: string; machineId?: string },
): ServerEntry[] {
  const servers = loadServers(storage);
  let entry = conn.machineId ? servers.find((s) => s.machineId === conn.machineId) : undefined;
  if (!entry) entry = servers.find((s) => s.address === conn.address);
  if (entry) {
    entry.address = conn.address;               // the address that just worked is the active one
    entry.token = conn.token;
    if (conn.socket) entry.socket = conn.socket; else delete entry.socket;
    if (conn.machineId) entry.machineId = conn.machineId;
  } else {
    entry = {
      id: serverId(), name: hostLabel(conn.address),
      address: conn.address, token: conn.token,
      ...(conn.socket ? { socket: conn.socket } : {}),
      ...(conn.machineId ? { machineId: conn.machineId } : {}),
    };
    servers.push(entry);
  }
  saveServers(storage, servers);
  storage.setItem(CURRENT_KEY, entry.id);
  return servers;
}

export function renameServer(storage: Store, id: string, name: string): ServerEntry[] {
  const servers = loadServers(storage);
  const entry = servers.find((s) => s.id === id);
  const trimmed = name.trim();
  if (entry && trimmed) { entry.name = trimmed; saveServers(storage, servers); }
  return servers;
}

/** Remove an entry and its parked per-server state. The CURRENT entry is not
 *  removable — the switcher offers removal only on the others, and refusing
 *  here keeps a stale UI honest. */
export function removeServer(storage: Store, id: string): ServerEntry[] {
  if (id === currentServerId(storage)) return loadServers(storage);
  const servers = loadServers(storage).filter((s) => s.id !== id);
  saveServers(storage, servers);
  storage.removeItem(STATE_PREFIX + id);
  storage.removeItem(MACHINE_PREFIX + id);
  return servers;
}

/**
 * Every storage write of a server switch, in one place.
 *
 * Parks the leaving server's live nav state and machine id under its own id,
 * restores the target's parked pair (or clears them — a first visit starts
 * fresh and the machine id arrives on connect), points the active-mirror
 * keys (`tmux_address`/`tmux_token`/`tmux_socket`) at the target, marks it
 * current, and clears `tmux_disconnected` (switching IS the intent to
 * connect). The caller then reloads: the boot path auto-connects from the
 * mirror keys and restores `tmux_state`, exactly as a fresh open would —
 * one code path, no per-component cache invalidation to forget.
 *
 * Returns false when the target does not exist or is already current.
 */
export function applySwitch(storage: Store, toId: string): boolean {
  const servers = loadServers(storage);
  const target = servers.find((s) => s.id === toId);
  const fromId = currentServerId(storage);
  if (!target || toId === fromId) return false;

  if (fromId) {
    const liveState = storage.getItem('tmux_state');
    if (liveState != null) storage.setItem(STATE_PREFIX + fromId, liveState);
    else storage.removeItem(STATE_PREFIX + fromId);
    const liveMachine = storage.getItem('tmux_machine_id');
    if (liveMachine != null) storage.setItem(MACHINE_PREFIX + fromId, liveMachine);
    else storage.removeItem(MACHINE_PREFIX + fromId);
  }

  const parkedState = storage.getItem(STATE_PREFIX + toId);
  if (parkedState != null) storage.setItem('tmux_state', parkedState);
  else storage.removeItem('tmux_state');
  // The target's machine identity: the entry's own machineId when known (it
  // is the SAME fact the parked key held, and fresher), else the parked one,
  // else cleared — never the leaving server's, so A's failover set is never
  // consulted while connected to B.
  const targetMachine = target.machineId || storage.getItem(MACHINE_PREFIX + toId);
  if (targetMachine) storage.setItem('tmux_machine_id', targetMachine);
  else storage.removeItem('tmux_machine_id');

  storage.setItem('tmux_address', target.address);
  storage.setItem('tmux_token', target.token);
  if (target.socket) storage.setItem('tmux_socket', target.socket);
  else storage.removeItem('tmux_socket');
  storage.setItem(CURRENT_KEY, toId);
  storage.removeItem('tmux_disconnected');
  return true;
}
