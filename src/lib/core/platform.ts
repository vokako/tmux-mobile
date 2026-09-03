// The ONE place the platform is decided (conventions/frontend.md, rule 3).
//
// Three answers, in the order they must be asked:
//   isAndroid       — the Tauri shell on Android (opens files through the
//                     AndroidFileOpener JS interface, never tauri-plugin-opener)
//   isTauri         — inside any Tauri shell (Android or desktop); false in a
//                     browser / PWA
//   isTauriDesktop  — the macOS shell: native page zoom, drag-drop paths,
//                     keyboard shortcuts panel
// `isAndroid` is checked BEFORE `isTauri` everywhere because Android IS a
// Tauri shell: a bare `isTauri` branch that reaches for a desktop plugin runs
// on the phone too. Until 2026-09-03 six files each re-derived these from
// `window.__TAURI_INTERNALS__` and a `/android/i` test, and the rule named
// symbols that existed nowhere — so it could not be enforced or tested.

export type PlatformFlags = {
  isAndroid: boolean;
  isTauri: boolean;
  isTauriDesktop: boolean;
};

type TauriGlobals = { __TAURI__?: unknown; __TAURI_INTERNALS__?: unknown };

/** The shell's marker globals, injected before any page script runs. */
export function hasTauriInternals(win: unknown): boolean {
  const w = win as TauriGlobals | null | undefined;
  return !!(w?.__TAURI__ || w?.__TAURI_INTERNALS__);
}

/** Pure: decide the flags for a given window + navigator (tests pass fakes). */
export function detectPlatform(win: unknown, nav: { userAgent?: string } | null | undefined): PlatformFlags {
  const android = /android/i.test(nav?.userAgent ?? '');
  const tauri = hasTauriInternals(win);
  return {
    // The Android app is a Tauri shell; a plain Android browser is not.
    isAndroid: android && tauri,
    isTauri: tauri,
    isTauriDesktop: tauri && !android,
  };
}

const flags = detectPlatform(
  typeof window === 'undefined' ? null : window,
  typeof navigator === 'undefined' ? null : navigator,
);

export const isAndroid: boolean = flags.isAndroid;
export const isTauri: boolean = flags.isTauri;
export const isTauriDesktop: boolean = flags.isTauriDesktop;

/**
 * The gate to `await` before importing or calling ANY `@tauri-apps/*` plugin.
 * Tauri 2 injects `__TAURI_INTERNALS__` before page scripts run, so today the
 * gate is already settled and awaiting it costs one microtask; it exists so
 * that every plugin call site has the same shape (`if (isTauri) { await
 * tauriReady; … }`) and so that a platform that ever needs a real wait — a
 * webview that surfaces the bridge late — changes this one line, not thirty
 * call sites. Callers that need the plugin MODULES keep their own promise
 * chained on this one (see `tauriPlugins` in Files.svelte).
 */
export const tauriReady: Promise<void> = Promise.resolve();
