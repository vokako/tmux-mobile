// One server, as something the app can hold (board #335 ②).
//
// Phase ① made the transport an object; a connection still knows nothing about
// WHICH server it reaches — deliberately, because identity is storage's and
// `app/servers.ts` is its authority. A runtime is the join: a saved
// `ServerEntry` plus the connection and bound API that serve it, plus the
// capabilities that server answered for itself.
//
// Three identity rules, which are the reason this module exists rather than a
// field on Connection:
//
//   - `id` is `ServerEntry.id`, always, and it is IMMUTABLE for the object's
//     lifetime. Every view, store slot and persisted key in phase ② keys off
//     it, so a runtime whose id could change under its holders would quietly
//     re-point them at another machine.
//   - an address is a candidate ROUTE to a server, never a second identity
//     (board #55: one machine is one entry, however many addresses answer for
//     it). Nothing here keys anything by URL or host.
//   - the server that answers decides. If the machine that authenticates is not
//     this entry's machine, the dial does not become a connected runtime under
//     this id — it reports the canonical entry for the machine that DID answer
//     and closes the socket, because a live link routing another machine's
//     panes into this entry's views is the multi-server version of the bug
//     #335 ① fixed inside one connection.
//
// What a runtime deliberately does NOT own yet:
//
//   - RECONNECT. `app/reconnect.ts` reads `tmux_address`, `tmux_token`,
//     `tmux_machine_id` and `tmux_machines` off the live unprefixed keys, i.e.
//     off whichever server is current, so a second runtime would reconnect to
//     the wrong machine. It needs one scoped target reader, which is ②b's
//     commit; faking it here with a key-remapping storage proxy would be a
//     second mechanism to delete two commits later.
//   - FOCUS. There is no "current" here, exactly as in the connection
//     registry. Which runtime a page looks at is the context provider's (②a-3)
//     and App's (②b-1).
import type { Connection } from '../core/connection.ts';
import type { ConnectionSlot } from '../core/connection-registry.ts';
import type { WsApi } from '../core/ws-api.ts';
import type { BackendInfo } from '../core/agents.ts';
import type { ServerId } from './refs.ts';
import { adoptHostname, loadServers, recordServer, saveServers, type ServerEntry } from './servers.ts';

type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

/** What this server answered about itself. `null` means "not answered yet",
 * which is not the same as "no" — the Hub tabs may not be hidden on a timeout.
 * Both survive a reconnect of the same server: a runtime's server cannot
 * change, so the reset App does today (the module socket could move to another
 * server between connects) is not needed, and each successful dial re-probes
 * anyway, so an upgraded or downgraded server self-heals. */
export interface ServerCaps {
  /** Does this server have the Hub (the desktop half)? */
  hub: boolean | null;
  /** The backends it can spawn, normalized the way `agents.ts` normalizes
   * them: a present but empty list counts as unknown, not as "none". */
  backends: BackendInfo[] | null;
}

export type DialResult =
  /** Authenticated, and the machine that answered is this entry's. `entry` is
   * the canonical entry as `recordServer` left it (fresh address and token). */
  | { ok: true; entry: ServerEntry }
  /** Authenticated — but as a DIFFERENT server. `entry` is the canonical entry
   * for the machine that answered, so the caller can include that one instead;
   * this runtime's socket is already closed and its entry untouched. */
  | { ok: false; reason: 'elsewhere'; entry: ServerEntry; machineId: string; hostname: string }
  /** Never got there: no socket, no auth, a timeout. */
  | { ok: false; reason: 'failed'; error: unknown }
  /** The runtime was dropped while dialling, or there is no saved entry to
   * dial any more. */
  | { ok: false; reason: 'gone' };

export interface ServerRuntime {
  /** `ServerEntry.id`, immutable for this object's lifetime. */
  readonly id: ServerId;
  /** This runtime's connection and its bound RPCs — the only transport it
   * has, so a call made through a runtime cannot land on another server. */
  readonly connection: Connection;
  readonly api: WsApi;
  /** The saved entry, read fresh so a rename or a new active address shows
   * through. Falls back to the last known one if it has been removed. */
  entry(): ServerEntry;
  /** Connect and authenticate, then canonicalize the identity. */
  dial(): Promise<DialResult>;
  /** Ask the server what it can do. Safe to call repeatedly. */
  probeCaps(): Promise<ServerCaps>;
  caps(): ServerCaps;
  /** The machine the live socket reported, else the entry's saved stamp. */
  machineId(): string;
  /** The hostname the live socket reported, else the entry's adopted one. */
  hostname(): string;
  /** Terminal, like `Connection.dispose`: the connection is released and every
   * answer still in flight is dropped instead of being published. Idempotent,
   * and it touches no other runtime. */
  dispose(): void;
}

export interface ServerRuntimeDeps {
  /** The identity authority. Every write here goes through `servers.ts`. */
  storage: Store;
  /** From the connection registry, which owns this object's lifetime. */
  slot: ConnectionSlot;
  connectTimeoutMs?: number;
}

export function createServerRuntime(entry: ServerEntry, deps: ServerRuntimeDeps): ServerRuntime {
  const { storage, slot, connectTimeoutMs } = deps;
  const { connection, api } = slot;
  const id = entry.id;
  let known: ServerEntry = entry;
  let caps: ServerCaps = { hub: null, backends: null };
  let dead = false;

  /** The saved entry for this id. A removed entry leaves the last known one,
   * so a caller still has a name to show while it tears the runtime down. */
  function current(): ServerEntry {
    const found = loadServers(storage).find((s) => s.id === id);
    if (found) known = found;
    return known;
  }

  /** Board #310's rule, for a server that is not necessarily the current one:
   * the entry of the machine that just authenticated adopts its hostname as a
   * DEFAULT name. App's version falls back to "the current entry" because a
   * pre-#55 entry might not know its machine yet; here the fallback is this
   * runtime's own entry, since "current" is meaningless for a runtime that
   * nobody is looking at. */
  function adopt(machine: string, host: string) {
    if (!host) return;
    const servers = loadServers(storage);
    const target = (machine && servers.find((s) => s.machineId === machine))
      || servers.find((s) => s.id === id);
    if (!target) return;
    const next = adoptHostname(servers, target.id, host);
    if (next === servers) return;
    saveServers(storage, next);
    if (target.id === id) known = next.find((s) => s.id === id) ?? known;
  }

  async function dial(): Promise<DialResult> {
    if (dead) return { ok: false, reason: 'gone' };
    const target = loadServers(storage).find((s) => s.id === id);
    // Nothing saved under this id: the entry was removed (or merged away)
    // while the runtime was idle. Dialling the last known address would mint a
    // connection to a server the registry no longer lists.
    if (!target) return { ok: false, reason: 'gone' };
    known = target;
    let reported: string | null;
    try {
      reported = await connection.connect(target.address, target.token, connectTimeoutMs);
    } catch (error) {
      return { ok: false, reason: 'failed', error };
    }
    if (dead) return { ok: false, reason: 'gone' };
    const machine = reported || connection.getMachineId() || '';
    const host = connection.getHostname() || '';
    // RECORD, never activate: `recordServer` folds this connection into the
    // machine's canonical entry (and absorbs the twin an address-only entry
    // leaves behind) without moving CURRENT — recording and activating are
    // different acts, and a runtime is never allowed to steal the focus.
    const { entry: canonical } = recordServer(storage, {
      address: target.address,
      token: target.token,
      ...(target.socket ? { socket: target.socket } : {}),
      ...(machine ? { machineId: machine } : {}),
    });
    if (canonical.id !== id) {
      // Either this entry knew its machine and a different one answered, or it
      // knew none and the machine that answered already has an entry. Both are
      // the same fact: this is not the server this runtime is for.
      connection.disconnect();
      adopt(machine, host);
      return { ok: false, reason: 'elsewhere', entry: canonical, machineId: machine, hostname: host };
    }
    known = canonical;
    adopt(machine, host);
    return { ok: true, entry: canonical };
  }

  async function probeHub(): Promise<void> {
    try {
      await api.hubRooms();
      if (!dead) caps = { ...caps, hub: true };
    } catch (e: any) {
      // Only a definitive server answer (method not found: no Hub) may flip
      // the flag off. A timeout or a reconnect blip keeps the current value,
      // because false unmounts the always-mounted Hub and destroys the state
      // it exists to preserve. App needed a switch-generation guard here; a
      // runtime does not — the answer came over THIS runtime's connection, so
      // it cannot be about another server.
      if (!dead && e?.code === -32601) caps = { ...caps, hub: false };
    }
  }

  async function probeBackends(): Promise<void> {
    try {
      const r = await api.backendsList();
      const list = r?.backends;
      if (!dead) caps = { ...caps, backends: Array.isArray(list) && list.length > 0 ? list : null };
    } catch {
      if (!dead) caps = { ...caps, backends: null };
    }
  }

  async function probeCaps(): Promise<ServerCaps> {
    await Promise.all([probeHub(), probeBackends()]);
    return caps;
  }

  function dispose() {
    dead = true;
    connection.dispose();
  }

  return {
    id,
    connection,
    api,
    entry: current,
    dial,
    probeCaps,
    caps: () => caps,
    machineId: () => connection.getMachineId() || current().machineId || '',
    hostname: () => connection.getHostname() || current().hostname || '',
    dispose,
  };
}
