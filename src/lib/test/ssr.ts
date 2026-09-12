// The ONE way a render test compiles a component through Vite SSR
// (docs/conventions/testing.md § render tests; board #177).
//
// Vite 6's `createServer` in middleware mode opens its OWN http server for
// the HMR websocket on the fixed default port 24678 — even with
// `server.hmr: false`, which only stops the update messages. Two `npm test`
// runs on one machine (the launch checkout and a worktree, lead 2026-09-12)
// fought over that port and one of them stalled: every render/mount case
// timed out at 60 s behind "WebSocket server error: Port 24678 is already in
// use". `server.ws: false` is what stops the socket. A node --test run needs
// neither, and concurrent suites are normal on a shared host, so no harness
// may hold a fixed port; `harness.source.test.ts` pins that every render
// test comes through here.
import type { InlineConfig, ViteDevServer } from 'vite';

/** Options a render test may add: a private optimizer cache (never the live
 * `node_modules/.vite`), `configFile`/`plugins` when the fixture compiles
 * Svelte itself, `ssr.noExternal` for packages Node cannot load as CJS. */
export type SsrOptions = Pick<InlineConfig, 'cacheDir' | 'configFile' | 'plugins' | 'ssr'> & { cacheDir: string };

export async function ssrServer(options: SsrOptions): Promise<ViteDevServer> {
  const { createServer } = await import('vite');
  return createServer({
    ...options,
    appType: 'custom', logLevel: 'error',
    // No listener at all: middleware mode has no http server, `hmr: false`
    // sends no updates, `ws: false` opens no websocket server.
    server: { middlewareMode: true, hmr: false, ws: false },
    optimizeDeps: { noDiscovery: true, include: [] },
  });
}
