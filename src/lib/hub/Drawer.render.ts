import assert from 'node:assert/strict';
import test from 'node:test';
import { renderHarness, RENDER_TIMEOUT_MS } from '../test/ssr.ts';

// One warm server for the whole render tier (board #178): the environment
// and the svelte runtime are built at import, outside this suite's timer.
const h = await renderHarness();

test('Drawer renders its original window list, real partitions and single head', { timeout: RENDER_TIMEOUT_MS }, async () => {
  const Drawer = (await h.load('/src/lib/hub/Drawer.svelte')).default;
  const { render } = h;
  const agents = [
    { name: 'alice', window: 0, agent: 'kiro', managed: true, state: 'working' },
    { name: 'loose', window: 1, agent: 'claude', managed: false, state: 'idle' },
    { name: 'shell', window: 2, agent: null, managed: false, state: 'idle' },
    { name: 'notes', window: 3, agent: null, managed: false, state: 'idle' },
  ];
  const view = (props: Record<string, unknown> = {}) => h.fragment(render(Drawer, { props: {
    selected: 'fixture', agents, managedAgents: [agents[0]], stateLabel: (state: string) => state,
    ...props,
  } }).body as string);
  const empty = view();
  assert.equal(empty.children.length, 1);
  assert.equal(empty.firstElementChild?.tagName, 'SECTION');
  assert.equal(empty.querySelectorAll('.drawer-head').length, 1);
  assert.ok(empty.querySelector('.side-handle.on-left'));
  assert.ok(empty.querySelector('.term-body .empty'));
  assert.equal(empty.querySelectorAll('.win-pill:not(.more)').length, 2);
  assert.equal(empty.querySelector('.win-pill.more')?.textContent?.trim(), '+2');
  assert.equal(empty.querySelector('.direct-tag')?.textContent, 'direct');
  assert.ok(empty.querySelector('.win-pill .live-dot'));
  assert.equal(view({ winsExpanded: true }).querySelectorAll('.win-pill:not(.more)').length, 4);
  const selectedShell = view({ termTarget: 'fixture:2.0', termCommand: 'zsh' });
  assert.ok(selectedShell.querySelector('.win-pill.cur')?.textContent?.includes('2:shell'));
  assert.equal(selectedShell.querySelector('.win-pill.more')?.textContent?.trim(), '+1');
  assert.ok(selectedShell.querySelector('.xterm-wrap'), 'the real Terminal markup is embedded');
  const files = view({ drawerView: 'files' });
  assert.ok(files.querySelector('.term-body.off'));
  assert.ok(files.querySelector('.files-body'));
  assert.equal(files.querySelector('.board-body'), null);
  const board = view({ drawerView: 'board' });
  assert.ok(board.querySelector('.board-body .board-root'));
  assert.equal(board.querySelector('.board-body .page-head'), null, 'the real Board keeps its embedded head gate');
  assert.equal(board.querySelectorAll('.drawer-head').length, 1);
});
