import assert from 'node:assert/strict';
import test from 'node:test';
import { JSDOM } from 'jsdom';

const noop = () => {};
(globalThis as Record<string, unknown>).localStorage ??= { getItem: () => null, setItem: noop, removeItem: noop };
(globalThis as Record<string, unknown>).window ??= {
  addEventListener: noop, removeEventListener: noop, navigator: { language: 'en' },
  matchMedia: () => ({ matches: false, addEventListener: noop, removeEventListener: noop }),
};
(globalThis as Record<string, unknown>).document ??= {
  addEventListener: noop, removeEventListener: noop,
  documentElement: { style: { setProperty: noop, getPropertyValue: () => '' } },
};

test('Roster renders its existing live/stopped/menu states without another wrapper', { timeout: 60000 }, async () => {
  const { createServer } = await import('vite');
  const vite = await createServer({
    server: { middlewareMode: true, hmr: false }, appType: 'custom', logLevel: 'error',
    cacheDir: 'node_modules/.vite-roster-render-test',
    optimizeDeps: { noDiscovery: true, include: [] },
  });
  try {
    const Roster = (await vite.ssrLoadModule('/src/lib/hub/Roster.svelte')).default;
    const { render } = await vite.ssrLoadModule('svelte/server');
    const agents = [
      { name: 'runner', window: 1, agent: 'kiro', state: 'working', team: 'dev', since: 0 },
      { name: 'waiting', window: 2, agent: 'codex', state: 'waiting', team: 'dev/review', since: 0 },
    ];
    const view = (props: Record<string, unknown> = {}) => JSDOM.fragment(render(Roster, { props: {
      selected: 'fixture', roomReady: true, managedAgents: agents, stopped: ['paused'],
      selectedRow: { project: { path: '/fixture' }, live: false, slots: [{ window_name: 'paused', command: 'codex' }] },
      recipient: 'runner', unread: new Set(['runner']), stateLabel: (state: string) => state,
      onconfigure: noop, ...props,
    } }).body as string);
    const roster = view();
    assert.equal(roster.children.length, 1);
    assert.ok(roster.firstElementChild?.classList.contains('roster'));
    assert.equal(roster.querySelectorAll('.tgroup').length, 2);
    const cards = [...roster.querySelectorAll('.acard:not(.add)')];
    const runner = cards.find((card) => card.querySelector('.a-name')?.textContent === 'runner')!;
    const waiting = cards.find((card) => card.querySelector('.a-name')?.textContent === 'waiting')!;
    assert.ok(runner.querySelector('.st.live-dot'));
    assert.ok(runner.querySelector('.unread'));
    assert.ok(waiting.classList.contains('needs'));
    assert.ok(waiting.querySelector('.ac-needs'));
    assert.equal(waiting.querySelector('.st.live-dot'), null);
    assert.equal(roster.querySelector('.acard.off img')?.getAttribute('src'), '/assets/codex.svg');
    assert.ok(roster.querySelector('.acard.off .a-start'));

    const liveMenu = view({ menuFor: 'runner' });
    assert.equal(liveMenu.children.length, 2, 'roster and fixed menu are sibling roots');
    assert.deepEqual([...liveMenu.querySelectorAll('.a-menu [role="menuitem"]')].map((item) => item.textContent?.trim()),
      ['Message', 'Watch in terminal', 'Only its messages', 'Configure agent', 'Interrupt', 'Restart', 'Stop', 'Remove']);
    const stoppedMenu = view({ menuFor: 'paused' });
    assert.deepEqual([...stoppedMenu.querySelectorAll('.a-menu [role="menuitem"]')].map((item) => item.textContent?.trim()),
      ['Resume', 'Only its messages', 'Configure agent', 'Remove']);
    assert.ok(view({ managedAgents: [], stopped: [] }).querySelector('.acard.add'));
    assert.equal(view({ selected: '' }).querySelector('.roster'), null);
    assert.ok(view({ selected: '', menuFor: 'paused' }).querySelector('.a-menu'),
      'the fixed menu is outside the selected-only roster gate');
    assert.ok(view({ roomReady: false }).querySelector('.sk-cards[aria-hidden="true"]'));
  } finally {
    await vite.close();
  }
});
