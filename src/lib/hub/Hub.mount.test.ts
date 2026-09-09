import assert from 'node:assert/strict';
import test from 'node:test';
import type { TestContext } from 'node:test';
import { compileMount } from '../test/mount.ts';

async function characterize(context: TestContext, fixture: Awaited<ReturnType<typeof compileMount>>) {
  const pushed = new Set<unknown>();
  const app = await fixture.mount(context, {
    props: { visible: true },
    modules: [{
      projectList: async () => ({ projects: [{
        project: { id: 'fixture', name: 'Fixture', session: 'fixture', path: '/fixture' },
        live: true, slots: [],
      }] }),
      listSessionsWithPanes: async () => ({ panes: [] }),
      hubRooms: async () => ({ rooms: {}, states: {} }),
      registryList: async () => ({ agents: [] }),
      teamsList: async () => ({ teams: [] }),
      hubAgents: async () => ({ agents: [
        { name: 'alice', window: 0, managed: true, agent: 'kiro', state: 'idle' },
        { name: 'bob', window: 1, managed: true, agent: 'codex', state: 'idle' },
      ] }),
      hubLog: async () => ({ messages: [], has_more: false }),
      hubActivity: async () => ({ events: [], has_more: false }),
      addTeamMessageListener: (fn: unknown) => { pushed.add(fn); },
      removeTeamMessageListener: (fn: unknown) => { pushed.delete(fn); },
    }],
  });
  try {
    for (let i = 0; i < 10 && app.document.querySelectorAll('.acard:not(.add)').length < 2; i++) {
      await app.flush();
    }
    const card = (name: string) => {
      const found = [...app.document.querySelectorAll<HTMLElement>('.acard:not(.add)')]
        .find((element) => element.querySelector('.a-name')?.textContent === name);
      assert.ok(found, `${name} is rendered by the real Hub`);
      return found;
    };
    const click = async (name: string) => {
      card(name).dispatchEvent(new app.window.MouseEvent('click', { bubbles: true }));
      await app.flush();
    };
    const recipient = () => app.document.querySelector('.to-name')?.textContent;
    const menu = () => app.document.querySelector('.a-menu .am-who')?.textContent ?? null;

    assert.equal(recipient(), 'alice');
    await click('bob');
    assert.equal(recipient(), 'bob');
    assert.equal(menu(), null, 'first click selects without opening a menu');
    await click('bob');
    await app.advance(259);
    assert.equal(menu(), null);
    await app.advance(1);
    assert.equal(menu(), 'bob', 'the selected card opens its menu at 260ms');

    app.document.body.dispatchEvent(new app.window.Event('pointerdown', { bubbles: true }));
    await app.flush();
    assert.equal(menu(), null);
    await click('bob');
    await app.advance(100);
    await click('alice');
    assert.equal(recipient(), 'alice', 'another card is not swallowed by the pending timer');
    await app.advance(260);
    assert.equal(menu(), null, 'the old card menu was cancelled, not delayed');
    assert.equal(pushed.size, 1);
  } finally {
    await app.close();
  }
  assert.equal(pushed.size, 0, 'unmount releases the real push subscription');
}

test('the real Hub defers the selected card menu and cancels it for another card', { timeout: 60000 }, async (context) => {
  const fixture = await compileMount(new URL('./Hub.svelte', import.meta.url), [
    new URL('../core/ws.ts', import.meta.url),
  ]);
  // Repeat the same characterization, not extra scenarios: fresh realm and
  // cleanup must work with a reused bundle, and their cost is measured apart.
  const times = [];
  for (let i = 0; i < 3; i++) {
    const start = performance.now();
    await characterize(context, fixture);
    times.push((performance.now() - start).toFixed(1));
  }
  context.diagnostic(`client compile ${fixture.compileMs.toFixed(1)}ms; fresh-DOM runs ${times.join(', ')}ms; RSS ${(process.memoryUsage().rss / 1024 ** 2).toFixed(1)}MiB; peak RSS ${(process.resourceUsage().maxRSS / 1024).toFixed(1)}MiB`);
});
