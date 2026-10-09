// The local server's mode as reactive state (board #323); the types and
// the wording rule are in server-mode.ts.
import { newerMode, type ServerMode } from './server-mode.ts';
export { localServerLine, newerMode, type ServerMode } from './server-mode.ts';

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
      // ONE entry for both: only a strictly newer seq applies.
      await t?.event?.listen('server_mode_changed', (e) => { current = newerMode(current, e.payload as ServerMode); });
      current = newerMode(current, (await t?.core?.invoke('server_mode')) as ServerMode | null | undefined);
    }).catch(() => {});
  },
};
