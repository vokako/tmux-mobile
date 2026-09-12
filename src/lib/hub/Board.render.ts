import assert from 'node:assert/strict';
import test from 'node:test';
import { renderHarness, RENDER_TIMEOUT_MS } from '../test/ssr.ts';

// One warm server for the whole render tier (board #178): the environment
// and the svelte runtime are built at import, outside this suite's timer.
const h = await renderHarness();

// Board #23's lesson: a source-contract can pass while the RENDERED tree still
// disappoints ("不要仅凭静态断言，核对…最终 DOM"). This file renders the REAL
// component through vite's SSR pipeline — the same compile the app ships — and
// asserts on the markup it actually emits.
//
// The embedded Board (the Hub drawer's board partition) must bring NO head of
// its own: the drawer head already names the project and carries the +, so the
// first thing under it is the board content. The standalone page keeps its
// page-head (hamburger, project title, +).

test('the embedded Board renders NO head row — the drawer head is the head (board #23, final DOM)', { timeout: RENDER_TIMEOUT_MS }, async () => {
  const Board = (await h.load('/src/lib/hub/Board.svelte')).default;
  // svelte/server must come from the SAME module graph as the component —
  // a node-resolved second instance has null internal state and throws.
  const { render } = h;

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
});
