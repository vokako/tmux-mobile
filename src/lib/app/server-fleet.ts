// Which servers the app is holding at once (board #335 ②).
//
// Three layers, one job each, and the fleet is the wiring between them:
//
//   `core/connection-registry.ts`  object LIFETIME — one connection per key
//   `app/servers.ts`               IDENTITY — which saved entry is which
//   `app/server-fleet.ts`          the set of runtimes the app holds
//
// Like `registry.ensure`, `include` never dials: it hands back an idle runtime,
// because only the caller knows whether now is the moment to contact a server.
// And like the registry, it has no "current" — a fleet is a set, not a focus.
// Phase ②a's production fleet still holds exactly one member; the aggregate
// switch that fills it is ③.
//
// One machine, one runtime. `servers.ts` already guarantees one ENTRY per
// machine, so keying by `ServerEntry.id` is normally enough, but a caller
// holding a stale entry (a list read before a merge) must not be able to open
// a second client for a machine that already has one.
import { createConnectionRegistry, type ConnectionRegistry } from '../core/connection-registry.ts';
import type { ServerId } from './refs.ts';
import type { ServerEntry } from './servers.ts';
import { createServerRuntime, type ServerRuntime } from './server-runtime.ts';

type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

export interface ServerFleetDeps {
  storage: Store;
  /** Normally the fleet's own. Injectable so a test (and ②b's App) can share
   * one registry, since object lifetime is the registry's to own. */
  registry?: ConnectionRegistry;
  connectTimeoutMs?: number;
}

export interface ServerFleet {
  /** The runtime for this entry, created idle on first ask. Returns the
   * EXISTING runtime when one already serves that entry — or that machine. */
  include(entry: ServerEntry): ServerRuntime;
  get(id: ServerId): ServerRuntime | undefined;
  /** The runtime serving a machine, by the id its entry or its live socket
   * reports. Empty machine ids never match. */
  byMachine(machineId: string): ServerRuntime | undefined;
  ids(): ServerId[];
  all(): ServerRuntime[];
  /** Drop one server: its runtime is disposed and its connection released.
   * Every other runtime, including a dial in flight, is untouched. */
  drop(id: ServerId): void;
  dropAll(): void;
}

export function createServerFleet(deps: ServerFleetDeps): ServerFleet {
  const { storage, connectTimeoutMs } = deps;
  const registry = deps.registry ?? createConnectionRegistry();
  const runtimes = new Map<ServerId, ServerRuntime>();

  function byMachine(machineId: string): ServerRuntime | undefined {
    if (!machineId) return undefined;
    for (const rt of runtimes.values()) if (rt.machineId() === machineId) return rt;
    return undefined;
  }

  function include(entry: ServerEntry): ServerRuntime {
    const held = runtimes.get(entry.id);
    if (held) return held;
    const twin = entry.machineId ? byMachine(entry.machineId) : undefined;
    if (twin) return twin;
    const runtime = createServerRuntime(entry, {
      storage,
      slot: registry.ensure(entry.id),
      // A dial can prove the entry was absorbed into another machine's. The
      // runtime disposes itself; the fleet forgets the key, so neither map is
      // left pointing at a server that no longer exists.
      onIdentityLost: () => { if (runtimes.get(entry.id) === runtime) drop(entry.id); },
      ...(connectTimeoutMs == null ? {} : { connectTimeoutMs }),
    });
    runtimes.set(entry.id, runtime);
    return runtime;
  }

  function drop(id: ServerId): void {
    const runtime = runtimes.get(id);
    if (!runtime) return;
    runtimes.delete(id);
    // The runtime's dispose releases the connection; removing the key from the
    // registry keeps the two maps from disagreeing about what is alive.
    runtime.dispose();
    registry.remove(id);
  }

  function dropAll(): void {
    for (const id of [...runtimes.keys()]) drop(id);
  }

  return {
    include,
    get: (id) => runtimes.get(id),
    byMachine,
    ids: () => [...runtimes.keys()],
    all: () => [...runtimes.values()],
    drop,
    dropAll,
  };
}
