import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';
import type { ProjectRow } from '../projects/projects.ts';
import type { SidebarPane } from './sidebar.ts';

const noop = () => {};
(globalThis as Record<string, unknown>).localStorage ??= { getItem: () => null, setItem: noop, removeItem: noop };
(globalThis as Record<string, unknown>).window ??= {
  addEventListener: noop, removeEventListener: noop,
  navigator: { language: 'en' },
  matchMedia: () => ({ matches: false, addEventListener: noop, removeEventListener: noop }),
  localStorage: (globalThis as Record<string, unknown>).localStorage,
};
(globalThis as Record<string, unknown>).document ??= {
  addEventListener: noop, removeEventListener: noop,
  documentElement: { style: { setProperty: noop, getPropertyValue: () => '' } },
};

test('Sidebar renders live/closed rows and the compact sheet without another wrapper', { timeout: 60000 }, async () => {
  const { createServer } = await import('vite');
  const vite = await createServer({
    server: { middlewareMode: true, hmr: false },
    logLevel: 'error', appType: 'custom',
    cacheDir: 'node_modules/.vite-sidebar-render-test',
    optimizeDeps: { noDiscovery: true, include: [] },
  });
  try {
    const Sidebar = (await vite.ssrLoadModule('/src/lib/hub/Sidebar.svelte')).default;
    const { render } = await vite.ssrLoadModule('svelte/server');
    const tick = 1_800_000_000_000;
    const project = (session: string) => ({
      id: session, session, name: session, path: `/${session}`, created_at: 1,
      last_seen_at: tick / 1000, adopted: false, autostart: false, archived: false,
    });
    const rows: ProjectRow[] = [
      { project: project('alpha'), live: true, slots: [] },
      { project: project('closed'), live: false, slots: [{
        ord: 0, window_name: 'paused', cwd: '/closed', kind: 'agent', command: 'codex',
        auto_run: true, first_seen_at: 1,
      }] },
    ];
    const panes: SidebarPane[] = Array.from({ length: 5 }, (_, window) => ({
      session: 'alpha', window, window_name: `agent${window}`,
      active: true, current_command: 'kiro', pane_title: '',
    }));
    const view = (extra: Record<string, unknown> = {}) => JSDOM.fragment(render(Sidebar, { props: {
      rows, panes, selected: 'alpha', tick, talkMap: { 'proj:alpha': tick - 300_000 },
      agentStates: { 'alpha:agent0': 'working' },
      trash: [{ project: { ...project('archived'), archived: true }, live: false, slots: [] }],
      ...extra,
    } }).body as string);

    const desktop = view();
    assert.equal(desktop.children.length, 1);
    assert.equal(desktop.firstElementChild?.tagName, 'ASIDE');
    assert.equal(desktop.querySelectorAll('.proj-row').length, 2);
    const alpha = desktop.querySelector('.proj-row[aria-label="alpha"]')!;
    assert.ok(alpha.classList.contains('open'));
    assert.equal(alpha.querySelector('.side-age')?.textContent, '5m',
      'the conversation timestamp wins over the newer tmux observation');
    assert.equal(alpha.querySelectorAll('.side-win').length, 4);
    assert.ok(alpha.querySelector('.side-win-dot.live-dot'));
    const closed = desktop.querySelector('.proj-row[aria-label="closed"]')!;
    assert.equal(closed.querySelector('.side-win-name')?.textContent, 'paused');
    assert.equal(closed.querySelector('img')?.getAttribute('src'), '/assets/codex.svg');
    assert.ok(closed.querySelector('.side-wins.dim'));
    assert.equal(closed.querySelector('.side-win-dot'), null);
    assert.ok(desktop.querySelector('.side-h'));
    assert.ok(desktop.querySelector('.side-handle'));
    assert.ok(desktop.querySelector('.trash-bar'));
    assert.equal(desktop.querySelector('.trash-row'), null, 'the recycle bin starts folded');

    const compact = view({ compact: true, open: true });
    assert.equal(compact.children.length, 2, 'scrim and aside stay sibling roots');
    assert.ok(compact.querySelector('.side-scrim'));
    assert.ok(compact.querySelector('.sidebar.side-sheet.sheet.open'));
    assert.equal(compact.querySelector('.side-handle'), null);
    assert.equal(view({ compact: true }).querySelector('.side-scrim'), null);
  } finally {
    await vite.close();
  }
});
