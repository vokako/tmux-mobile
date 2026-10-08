// THE server switch (board 315), framework-free so its contract is testable
// against two fake servers. App wires the effects; this owns the ORDER, the
// one-intent rule and the source/candidate split. No reload: the shell stays,
// the caller unmounts the server-bound tree when `onstate` reports a switch,
// and remounts it in `comeUp`. Nothing in memory crosses (the state-ownership
// table is websocket-client.md §#55).
//
// Order, each step finished before the next:
//   1. leave guards (unsaved editors ask in their own dialogs; a save or
//      delete in flight finishes first) — a cancel changes nothing at all;
//   2. one intent: the reconnect loop stops, the content tree unmounts
//      (onstate), every older async loses ownership (`owns(intent)`);
//   3. running downloads are suspended (part kept) and awaited, the tree's
//      unmount persistence runs (afterUnmount), THEN the leaving server is
//      parked — only a server we were really connected to, and only once;
//   4. the socket closes, the in-memory per-server state resets;
//   5. connect to the target WITHOUT touching the mirror keys or CURRENT
//      (a reload mid-switch boots the server we left — honest);
//   6. only after auth: record by the identity the server reported, point
//      every parked key at that canonical entry, write the mirror, come up.
// A failure keeps `from` (identity + its park) frozen: Retry and Back reuse
// it and never park again, so the source's state cannot be overwritten by a
// half-switched world.

import { activateSwitched, parkFrom, type ServerEntry } from './servers.ts';

type Store = Pick<Storage, 'getItem' | 'setItem' | 'removeItem'>;

/** A saved entry, or a candidate from Add server (`id: ''` until auth). */
export type SwitchTarget = Pick<ServerEntry, 'address' | 'token' | 'socket'> & { id: string; name: string };

export interface SwitchState {
  intent: number;
  /** The server the user was connected to when this switch began (frozen). */
  from: { id: string; name: string } | null;
  target: SwitchTarget;
  phase: 'connecting' | 'failed';
  error: string;
}

export interface SwitchDeps {
  storage: Store;
  /** Connected to a server right now (not mid-switch). */
  connected(): boolean;
  currentId(): string;
  currentName(): string;
  /** Ask every leave guard; false = stay. */
  confirmLeave(): Promise<boolean>;
  stopReconnect(): void;
  /** The switch state changed: non-null = the content tree must be unmounted. */
  onstate(state: SwitchState | null): void;
  suspendDownloads(): Promise<void>;
  /** Resolves once the unmounted tree's teardown has run. */
  afterUnmount(): Promise<void>;
  disconnect(): void;
  resetMemory(): void;
  connect(address: string, token: string): Promise<unknown>;
  setSocket(socket: string): Promise<unknown>;
  machineId(): string;
  /** Authenticated and activated: remount and bring the app up. */
  comeUp(target: SwitchTarget, entry: ServerEntry): void;
}

export function createServerSwitch(d: SwitchDeps) {
  let seq = 0;
  let guarding = false;
  let state: SwitchState | null = null;
  const set = (next: SwitchState | null) => { state = next; d.onstate(next); };

  async function switchTo(target: SwitchTarget): Promise<void> {
    if (state?.phase === 'connecting' || guarding) return; // one intent at a time
    const fromConnected = d.connected() && !state;
    if (fromConnected && target.id && target.id === d.currentId()) return;
    if (fromConnected) {
      guarding = true;
      let ok = false;
      try { ok = await d.confirmLeave(); } finally { guarding = false; }
      if (!ok) return;
    }
    const intent = ++seq;
    d.stopReconnect();
    const from = fromConnected ? { id: d.currentId(), name: d.currentName() } : state?.from ?? null;
    set({ intent, from, target, phase: 'connecting', error: '' });
    if (fromConnected && from) {
      await d.suspendDownloads();
      await d.afterUnmount();
      parkFrom(d.storage, from.id);
    }
    if (intent !== seq) return;
    d.disconnect();
    d.resetMemory();
    try {
      await d.connect(target.address, target.token);
      if (intent !== seq) return;
      if (target.socket) await d.setSocket(target.socket).catch(() => {});
      if (intent !== seq) return;
      const mid = d.machineId();
      const entry = activateSwitched(d.storage, {
        address: target.address, token: target.token,
        ...(target.socket ? { socket: target.socket } : {}),
        ...(mid ? { machineId: mid } : {}),
      });
      set(null);
      d.comeUp(target, entry);
    } catch (e) {
      if (intent !== seq) return;
      d.disconnect();
      set({ intent, from, target, phase: 'failed', error: String((e as Error)?.message || e) });
    }
  }

  return {
    switchTo,
    get state() { return state; },
    /** The current intent: an async begun under it compares on resolve. */
    get intent() { return seq; },
    owns(intent: number) { return intent === seq && !state; },
    /** Another path (the connect page) took the socket: older asyncs lose it. */
    supersede() { seq++; },
    retry() { if (state?.phase === 'failed') return switchTo(state.target); },
    /** Back to the server the switch left, from its frozen identity. */
    back(entries: readonly ServerEntry[]) {
      const id = state?.phase === 'failed' ? state.from?.id : '';
      const entry = id ? entries.find((s) => s.id === id) : undefined;
      if (entry) return switchTo(entry);
    },
  };
}
