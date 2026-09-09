import assert from 'node:assert/strict';
import test from 'node:test';
import type { TestContext } from 'node:test';
import { compileMount } from '../test/mount.ts';

let compilation: ReturnType<typeof compileMount> | undefined;
const compiledHub = () => compilation ??= compileMount(new URL('./Hub.svelte', import.meta.url), [
  new URL('../core/ws.ts', import.meta.url),
]);

function roomFixture() {
  const pushed = new Set<unknown>();
  return {
    pushed,
    rpc: {
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
    },
  };
}

async function characterize(context: TestContext, fixture: Awaited<ReturnType<typeof compileMount>>) {
  const { pushed, rpc } = roomFixture();
  const app = await fixture.mount(context, {
    props: { visible: true },
    modules: [rpc],
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
  const fixture = await compiledHub();
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

test('a pending attachment blocks Enter until the real Hub can send the complete body', { timeout: 60000 }, async (context) => {
  const fixture = await compiledHub();
  const started = performance.now();
  const { pushed, rpc } = roomFixture();
  const uploads: Array<{ path: string; body: string }> = [];
  const posts: Array<{ session: string; body: string }> = [];
  let releaseUpload!: () => void;
  const upload = new Promise<void>((resolve) => { releaseUpload = resolve; });
  const app = await fixture.mount(context, {
    props: { visible: true },
    modules: [{
      ...rpc,
      fsMkdir: async () => ({}),
      fsUpload: async (path: string, body: string) => {
        if (path.endsWith('/.gitignore')) return {};
        uploads.push({ path, body });
        await upload;
        return {};
      },
      hubPost: async (session: string, body: string) => { posts.push({ session, body }); return {}; },
    }],
  });
  try {
    for (let i = 0; i < 10 && app.document.querySelector('.to-name')?.textContent !== 'alice'; i++) {
      await app.flush();
    }
    assert.equal(app.document.querySelector('.to-name')?.textContent, 'alice');
    const input = app.document.querySelector<HTMLTextAreaElement>('.c-input')!;
    const picker = app.document.querySelector<HTMLInputElement>('input[type="file"]')!;
    const send = app.document.querySelector<HTMLButtonElement>('.send-btn')!;
    input.value = 'Inspect ';
    input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
    await app.flush();
    input.setSelectionRange(input.value.length, input.value.length);
    const file = new app.window.File(['payload'], 'report.txt', { type: 'text/plain' });
    Object.defineProperty(picker, 'files', { value: [file] });
    picker.dispatchEvent(new app.window.Event('change', { bubbles: true }));
    for (let i = 0; i < 10 && !uploads.length; i++) await app.flush();
    assert.equal(uploads.length, 1, 'the actual stageFiles pipeline reached the deferred upload');
    assert.equal(uploads[0]!.body, 'cGF5bG9hZA==');
    assert.equal(send.disabled, true);
    const enter = async () => {
      input.dispatchEvent(new app.window.KeyboardEvent('keydown', {
        key: 'Enter', bubbles: true, cancelable: true,
      }));
      await app.flush();
    };
    await enter();
    assert.deepEqual(posts, [], 'Enter must not leak the text while its attachment is uploading');

    releaseUpload();
    for (let i = 0; i < 10 && send.disabled; i++) await app.flush();
    assert.equal(send.disabled, false);
    assert.equal(input.value, 'Inspect [file:1]');
    await enter();
    assert.deepEqual(posts, [{ session: 'fixture', body: `@alice Inspect ${uploads[0]!.path}` }]);
    assert.equal(input.value, '');
    assert.equal(app.document.querySelectorAll('.pend-chip').length, 0);
  } finally {
    releaseUpload();
    await app.close();
  }
  assert.equal(pushed.size, 0);
  context.diagnostic(`attachment scenario after shared compilation ${(performance.now() - started).toFixed(1)}ms`);
});

test('the local Hub Back callback peels recipient before palette and stops at the compact floor', { timeout: 60000 }, async (context) => {
  const fixture = await compiledHub();
  const started = performance.now();
  const { rpc } = roomFixture();
  let back: () => boolean = () => assert.fail('Hub has not registered Back');
  let registrations = 0;
  const app = await fixture.mount(context, {
    props: {
      visible: true, mobile: true,
      onGoBack: (fn: () => boolean) => { back = fn; registrations++; },
    },
    modules: [{ ...rpc, modelsList: async () => ({ models: [] }) }],
  });
  try {
    for (let i = 0; i < 10 && app.document.querySelector('.to-name')?.textContent !== 'alice'; i++) {
      await app.flush();
    }
    assert.equal(app.document.querySelector('.to-name')?.textContent, 'alice');
    const historyLength = app.window.history.length;
    const published = back;
    const input = app.document.querySelector<HTMLTextAreaElement>('.c-input')!;
    input.value = '/';
    input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
    await app.flush();
    assert.ok(app.document.querySelector('.cmd-menu'));
    app.document.querySelector<HTMLButtonElement>('.to-chip')!.click();
    await app.flush();
    assert.ok(app.document.querySelector('.to-menu'));

    assert.equal(back(), true);
    await app.flush();
    assert.equal(app.document.querySelector('.to-menu'), null, 'recipient is peeled first');
    assert.ok(app.document.querySelector('.cmd-menu'), 'palette survives the first Back');
    assert.equal(back(), true);
    await app.flush();
    assert.equal(app.document.querySelector('.cmd-menu'), null);
    assert.equal(back(), true);
    await app.flush();
    assert.ok(app.document.querySelector('.sidebar.open'));
    assert.equal(back(), false);
    assert.equal(back(), false);
    assert.ok(app.document.querySelector('.sidebar.open'), 'the floor never closes itself');
    assert.equal(back, published);
    assert.equal(registrations, 1, 'state changes do not publish another callback');
    assert.equal(app.window.history.length, historyLength, 'local dispatch never pushes browser history');

    app.document.querySelector<HTMLElement>('.side-scrim')!.click();
    app.document.querySelector<HTMLButtonElement>('.to-chip')!.click();
    await app.flush();
    assert.ok(app.document.querySelector('.to-menu'), 'leave a live layer for the unmount check');
  } finally {
    await app.close();
  }
  assert.equal(back(), false, 'the disposed Hub has no registered layers');
  context.diagnostic(`Back scenario after shared compilation ${(performance.now() - started).toFixed(1)}ms`);
});

test('Hub Sidebar keeps row identity, free restore, confirmed purge and the compact floor', { timeout: 60000 }, async (context) => {
  const fixture = await compiledHub();
  const started = performance.now();
  const { rpc } = roomFixture();
  const row = (session: string, name: string, archived = false) => ({
    project: { id: `p-${session}`, session, name, path: `/${session}`, archived },
    live: !archived, slots: [],
  });
  const rows = [row('fixture', 'Fixture'), row('other', 'Other'), row('archived', 'Archived', true)];
  const operations: Array<{ method: string; id: string; archived?: boolean }> = [];
  let back: () => boolean = () => assert.fail('Back is not registered');
  const app = await fixture.mount(context, {
    props: { visible: true, mobile: true, onGoBack: (fn: () => boolean) => { back = fn; } },
    setup(window) {
      // Svelte's keyed-list bookkeeping queries animations; jsdom does not
      // render any. Chromium separately verifies the actual flip motion.
      window.Element.prototype.getAnimations = () => [];
    },
    modules: [{
      ...rpc,
      projectList: async () => ({ projects: rows }),
      projectDown: async (id: string) => {
        operations.push({ method: 'down', id });
        rows.find((r) => r.project.id === id)!.live = false;
        return {};
      },
      projectArchive: async (id: string, archived: boolean) => {
        operations.push({ method: 'archive', id, archived });
        rows.find((r) => r.project.id === id)!.project.archived = archived;
        return {};
      },
      projectDelete: async (id: string) => {
        operations.push({ method: 'purge', id });
        rows.splice(rows.findIndex((r) => r.project.id === id), 1);
        return {};
      },
    }],
  });
  const waitFor = async (predicate: () => boolean) => {
    for (let i = 0; i < 12 && !predicate(); i++) await app.flush();
    assert.ok(predicate(), 'the expected reactive state settled');
  };
  const click = async (selector: string) => {
    const target = app.document.querySelector<HTMLElement>(selector);
    assert.ok(target, selector);
    target.click();
    await app.flush();
  };
  const menuAction = async (label: string) => {
    const target = [...app.document.querySelectorAll<HTMLButtonElement>('.ctx button')]
      .find((button) => button.textContent?.trim() === label);
    assert.ok(target, label);
    target.click();
    await app.flush();
  };
  try {
    await waitFor(() => app.document.querySelectorAll('.proj-row').length === 2);
    assert.equal(back(), true);
    await app.flush();
    assert.ok(app.document.querySelector('.sidebar.open'));
    await click('[aria-label="Other"] .proj-pick');
    await waitFor(() => app.document.querySelector('.h1-text')?.textContent === 'Other');
    assert.equal(app.document.querySelector('.sidebar.open'), null, 'selection closes the compact sheet');
    assert.equal(back(), true);
    await app.flush();
    await click('[aria-label="Fixture"] .proj-pick');
    await waitFor(() => app.document.querySelector('.h1-text')?.textContent === 'Fixture');
    assert.equal(back(), true);
    await app.flush();
    assert.equal(back(), false, 'the open list is still the floor');

    await click('[aria-label="Other"] .row-menu');
    assert.equal(app.document.querySelector('.ctx-who')?.textContent, 'Other');
    assert.equal(app.document.querySelector('.h1-text')?.textContent, 'Fixture');
    await menuAction('Close');
    const closeTitle = app.document.querySelector('.dlg.confirm h2')?.textContent;
    await click('.dlg.confirm .primary');
    await waitFor(() => app.document.querySelector('.dlg.confirm') === null);
    assert.deepEqual(operations, [{ method: 'down', id: 'p-other' }],
      'the non-selected row action must not target the selected project');
    assert.ok(closeTitle?.includes('Other'));
    assert.equal(app.document.querySelector('.h1-text')?.textContent, 'Fixture');

    await click('.trash-bar');
    await click('.trash-row .t-act:not(.danger)');
    await waitFor(() => app.document.querySelector('[aria-label="Archived"]') !== null);
    assert.equal(app.document.querySelector('.dlg.confirm'), null, 'restore is not a destructive confirmation');
    assert.deepEqual(operations.at(-1), { method: 'archive', id: 'p-archived', archived: false });
    await click('[aria-label="Archived"] .row-menu');
    await menuAction('Delete');
    await click('.dlg.confirm .primary');
    await waitFor(() => app.document.querySelector('.trash-row') !== null);
    assert.deepEqual(operations.at(-1), { method: 'archive', id: 'p-archived', archived: true });
    await click('.trash-row .t-act.danger');
    assert.equal(operations.some((op) => op.method === 'purge'), false);
    assert.ok(app.document.querySelector('.dlg.confirm h2')?.textContent?.includes('Archived'));
    await click('.dlg.confirm .primary');
    await waitFor(() => app.document.querySelector('.trash-row') === null);
    assert.deepEqual(operations.at(-1), { method: 'purge', id: 'p-archived' });
  } finally {
    await app.close();
  }
  context.diagnostic(`Sidebar scenario after shared compilation ${(performance.now() - started).toFixed(1)}ms`);
});
