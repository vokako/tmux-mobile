// The local server's mode as reactive state (board #323); the types and
// the wording rule are in server-mode.ts.
import type { ServerMode } from './server-mode.ts';
export { localServerLine, type ServerMode } from './server-mode.ts';

type Tauri = { core?: { invoke(cmd: string): Promise<unknown> }; event?: { listen(name: string, fn: (e: { payload: unknown }) => void): Promise<unknown> } };

let current = $state<ServerMode | null>(null);
let started = false;

export const localServer = {
  /** The mode now; null outside the desktop app or before the first answer. */
  get mode(): ServerMode | null { return current; },
  /** Subscribe and read, once per page (idempotent). */
  start(ready: Promise<unknown>, tauri: () => Tauri | undefined) {
    if (started) return;
    started = true;
    void ready.then(async () => {
      const t = tauri();
      await t?.event?.listen('server_mode_changed', (e) => { current = (e.payload as ServerMode) ?? null; });
      const m = (await t?.core?.invoke('server_mode')) as ServerMode | null | undefined;
      // An event that landed during the read is newer than the read.
      if (current === null) current = m ?? null;
    }).catch(() => {});
  },
};
