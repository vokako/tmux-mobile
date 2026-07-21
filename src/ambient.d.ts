// App-wide ambient globals. The ONE place window augmentations live —
// don't re-declare these per-module.
export {};

declare global {
  interface Window {
    /** Debug sink installed by the in-app debug overlay (App.svelte). */
    __dbg?: (msg: string) => void;
    /** Present inside the Tauri shell (desktop/Android), absent in browsers. */
    __TAURI__?: { core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<any> } };
    __TAURI_INTERNALS__?: unknown;
  }
}
