import assert from 'node:assert/strict';
import test from 'node:test';

// Board #23's lesson: a source-contract can pass while the RENDERED tree still
// disappoints ("不要仅凭静态断言，核对…最终 DOM"). This file renders the REAL
// component through vite's SSR pipeline — the same compile the app ships — and
// asserts on the markup it actually emits.
//
// The embedded Board (the Hub drawer's board partition) must bring NO head of
// its own: the drawer head already names the project and carries the +, so the
// first thing under it is the board content. The standalone page keeps its
// page-head (hamburger, project title, +).

// Node lacks the browser globals a few modules touch at import time.
const noop = () => {};
(globalThis as Record<string, unknown>).localStorage ??= { getItem: () => null, setItem: noop, removeItem: noop };
(globalThis as Record<string, unknown>).window ??= {
  addEventListener: noop, removeEventListener: noop, dispatchEvent: () => true,
  location: { protocol: 'http:', host: 'localhost' }, navigator: { language: 'en' },
  setTimeout, clearTimeout, setInterval, clearInterval,
  matchMedia: () => ({ matches: false, addEventListener: noop, removeEventListener: noop }),
  localStorage: (globalThis as Record<string, unknown>).localStorage,
};
(globalThis as Record<string, unknown>).document ??= {
  addEventListener: noop, removeEventListener: noop,
  documentElement: { style: { setProperty: noop, getPropertyValue: () => '' } },
};
(globalThis as Record<string, unknown>).navigator ??= { language: 'en', userAgent: 'node' };

test('the embedded Board renders NO head row — the drawer head is the head (board #23, final DOM)', { timeout: 60000 }, async () => {
  const { createServer } = await import('vite');
  const vite = await createServer({
    server: { middlewareMode: true, hmr: false },
    logLevel: 'error',
    appType: 'custom',
    // A private cache so this run cannot fight the live dev server's
    // optimizer (a shared cacheDir crashed esbuild mid-scan).
    cacheDir: 'node_modules/.vite-render-test',
    optimizeDeps: { noDiscovery: true, include: [] },
  });
  try {
    const Board = (await vite.ssrLoadModule('/src/lib/hub/Board.svelte')).default;
    // svelte/server must come from the SAME module graph as the component —
    // a node-resolved second instance has null internal state and throws.
    const { render } = await vite.ssrLoadModule('svelte/server');

    const embedded = render(Board, { props: { session: 'proofsess', embedded: true } }).body as string;
    const standalone = render(Board, { props: { session: 'proofsess', embedded: false } }).body as string;

    // Embedded: no page-head, no h1 project title, no project sidebar — the
    // content root is the first real thing in the tree.
    assert.ok(!/class="page-head"/u.test(embedded), 'embedded emits no page-head row');
    assert.ok(!/<h1[^>]*>/u.test(embedded), 'embedded emits no project title');
    assert.ok(!/class="sidebar/u.test(embedded), 'embedded emits no project sidebar');
    // "Nothing between them" means no ELEMENT: svelte's SSR block markers are
    // comments whose exact spelling changes across svelte minors (5.38 wrote
    // <!--[-1-->, 5.53 writes <!--[!-->) — pinning one spelling made a routine
    // dependency refresh read as a layout regression.
    assert.match(embedded, /class="board-root[^"]*embedded[^"]*">(?:\s|<!--[^>]*-->)*<div class="bmain/u,
      'the root opens straight into bmain — nothing renders between them');
    assert.match(embedded, /class="board /u, 'the board content is there');

    // Standalone: the head survives untouched.
    assert.match(standalone, /class="page-head"/u, 'the page keeps its head');
    assert.match(standalone, /<h1[^>]*>/u, 'and its project title');
  } finally {
    await vite.close();
  }
});
