// App-wide ambient globals. The ONE place window augmentations live —
// don't re-declare these per-module.
export {};

declare global {
  interface Window {
    /** Present inside the Tauri shell (desktop/Android), absent in browsers. */
    __TAURI__?: { core: { invoke: (cmd: string, args?: Record<string, unknown>) => Promise<any> } };
    __TAURI_INTERNALS__?: unknown;
  }

  interface ImportMetaEnv {
    readonly DEV: boolean;
  }

  interface ImportMeta {
    readonly env: ImportMetaEnv;
  }
}
