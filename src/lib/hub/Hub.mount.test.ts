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

test('saved all restores through a fresh mount and room revisit without delivering (#171)', { timeout: 60000 }, async (context) => {
  const fixture = await compiledHub();
  const { rpc } = roomFixture();
  let saved = JSON.stringify({ fixture: 'all', other: '' });
  for (let reload = 0; reload < 2; reload++) {
    const app = await fixture.mount(context, {
      props: { visible: true },
      setup(window) {
        window.Element.prototype.getAnimations = () => [];
        window.localStorage.setItem('tmux_hub_project', 'fixture');
        window.localStorage.setItem('tmux_hub_lead', saved);
      },
      modules: [{
        ...rpc,
        projectList: async () => ({ projects: ['fixture', 'other'].map((session) => ({
          project: { id: session, name: session, session, path: `/${session}` }, live: true, slots: [],
        })) }),
        hubPost: () => assert.fail('restoring a destination must not post'),
        hubCommand: () => assert.fail('restoring a destination must not execute a command'),
        hubAgentInterrupt: () => assert.fail('restoring a destination must not interrupt'),
      }],
    });
    try {
      for (let i = 0; i < 12 && app.document.querySelectorAll('.acard:not(.add)').length < 2; i++) await app.flush();
      const input = app.document.querySelector<HTMLTextAreaElement>('.c-input')!;
      assert.equal(input.placeholder, 'Message every agent…', 'a fresh client restores broadcast');
      for (const room of ['other', 'fixture']) {
        app.document.querySelector<HTMLElement>(`[aria-label="${room}"] .proj-pick`)!.click();
        for (let i = 0; i < 12 && app.document.querySelector('.h1-text')?.textContent !== room; i++) await app.flush();
      }
      assert.equal(input.placeholder, 'Message every agent…', 'cached room restore keeps broadcast');
      await app.advance(10000);
      assert.equal(input.placeholder, 'Message every agent…', 'roster polling does not reseat a lead');
      saved = app.window.localStorage.getItem('tmux_hub_lead')!;
      assert.deepEqual(JSON.parse(saved), { fixture: 'all', other: '' });
    } finally { await app.close(); }
  }
});

const selectedCard = (document: Document) =>
  document.querySelector('.agent-select[aria-pressed="true"]')?.closest<HTMLElement>('.acard')?.dataset.agent ?? '';
const stripCard = (document: Document, name: string) =>
  document.querySelector<HTMLElement>(`.acard[data-agent="${name}"]`)!;

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
      const found = stripCard(app.document, name)?.querySelector<HTMLButtonElement>('.agent-select');
      assert.ok(found, `${name} is rendered by the real Hub`);
      return found;
    };
    const click = async (name: string) => {
      card(name).dispatchEvent(new app.window.MouseEvent('click', { bubbles: true }));
      await app.flush();
    };
    const recipient = () => selectedCard(app.document);

    assert.equal(recipient(), 'alice');
    await click('bob');
    assert.equal(recipient(), 'bob');
    await click('bob');
    assert.equal(recipient(), '', 'reselecting a card immediately chooses record-only');
    await click('all');
    assert.equal(recipient(), 'all');
    await click('alice');
    assert.equal(recipient(), 'alice');
    await app.advance(260);
    assert.equal(app.document.querySelector('.a-menu, .to-chip, .to-menu, .int-pill'), null,
      '#168 retires the delayed menu, recipient popup and send arm whole');
    const roster = app.document.querySelector('.roster')!;
    const composer = app.document.querySelector('.composer')!;
    assert.ok(roster.compareDocumentPosition(composer) & app.window.Node.DOCUMENT_POSITION_FOLLOWING);
    assert.equal(pushed.size, 1);
  } finally {
    await app.close();
  }
  assert.equal(pushed.size, 0, 'unmount releases the real push subscription');
}

test('the real Hub strip selects one destination and deselects to record-only without a tap timer', { timeout: 60000 }, async (context) => {
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
    for (let i = 0; i < 10 && selectedCard(app.document) !== 'alice'; i++) {
      await app.flush();
    }
    assert.equal(selectedCard(app.document), 'alice');
    const input = app.document.querySelector<HTMLTextAreaElement>('.c-input')!;
    const picker = app.document.querySelector<HTMLInputElement>('input[type="file"]')!;
    const send = app.document.querySelector<HTMLButtonElement>('.composer-actions button:last-child')!;
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

test('the local Hub Back callback peels ContextMenu before palette and stops at the compact floor', { timeout: 60000 }, async (context) => {
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
    for (let i = 0; i < 10 && selectedCard(app.document) !== 'alice'; i++) {
      await app.flush();
    }
    assert.equal(selectedCard(app.document), 'alice');
    const historyLength = app.window.history.length;
    const published = back;
    const input = app.document.querySelector<HTMLTextAreaElement>('.c-input')!;
    input.value = '/';
    input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
    await app.flush();
    assert.ok(app.document.querySelector('.cmd-menu'));
    stripCard(app.document, 'alice').querySelector('.agent-select')!.dispatchEvent(new app.window.MouseEvent('contextmenu', { bubbles: true }));
    await app.flush();
    assert.ok(app.document.querySelector('.ctx'));

    assert.equal(back(), true);
    await app.flush();
    assert.equal(app.document.querySelector('.ctx'), null, 'ContextMenu is peeled first');
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
    stripCard(app.document, 'alice').querySelector('.agent-select')!.dispatchEvent(new app.window.MouseEvent('contextmenu', { bubbles: true }));
    await app.flush();
    assert.ok(app.document.querySelector('.ctx'), 'leave a live layer for the unmount check');
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

test('a confirmation consumes Escape without also closing the earlier-mounted Hub drawer (#155)', { timeout: 60000 }, async context => {
  const { rpc } = roomFixture();
  const app = await (await compiledHub()).mount(context, {
    props: { visible: true },
    modules: [{
      ...rpc,
      fsCwd: async () => ({ path: '/fixture' }),
      fsList: async () => ({ entries: [] }),
      getPrefs: async () => ({}),
      getBookmarks: async () => ({ bookmarks: [] }),
      gitCmd: async () => ({ code: 1 }),
    }],
  });
  try {
    for (let i = 0; i < 12 && !app.document.querySelector('.h1-text'); i++) await app.flush();
    const files = [...app.document.querySelectorAll<HTMLButtonElement>('.page-head button')]
      .find(button => button.getAttribute('aria-label') === 'Files');
    assert.ok(files);
    files.click();
    for (let i = 0; i < 12 && !app.document.querySelector('.drawer .file-list'); i++) await app.flush();
    assert.ok(app.document.querySelector('.drawer'));
    app.document.querySelector<HTMLButtonElement>('.drawer [aria-label="New item"]')!.click();
    await app.flush();
    const origin = app.document.querySelector<HTMLInputElement>('.drawer .new-item input')!;
    origin.focus();
    app.document.querySelector<HTMLButtonElement>('.row-menu')!.click();
    await app.flush();
    app.document.activeElement!.dispatchEvent(new app.window.KeyboardEvent('keydown', {
      key: 'Escape', bubbles: true, cancelable: true,
    }));
    await app.flush();
    assert.equal(app.document.querySelector('.ctx'), null);
    assert.ok(app.document.querySelector('.drawer'), '#164: a focused project menu cannot close the Files drawer below it');
    app.document.querySelector<HTMLButtonElement>('.row-menu')!.click();
    await app.flush();
    const close = [...app.document.querySelectorAll<HTMLButtonElement>('.ctx button')]
      .find(button => button.textContent?.trim() === 'Close');
    assert.ok(close);
    close.click();
    await app.flush();
    assert.ok(app.document.querySelector('[aria-modal="true"]'));
    app.document.activeElement!.dispatchEvent(new app.window.KeyboardEvent('keydown', {
      key: 'Escape', bubbles: true, cancelable: true,
    }));
    await app.flush();
    assert.equal(app.document.querySelector('[aria-modal="true"]'), null);
    assert.ok(app.document.querySelector('.drawer'), 'the underlying capture handler yields to the modal');
    assert.equal(app.document.activeElement, origin, '#164: menu-to-confirm transfers the live focus origin');
  } finally { await app.close(); }
});

test('Roster ContextMenu filters without selecting and a stopped surface never resumes on tap', { timeout: 60000 }, async (context) => {
  const fixture = await compiledHub();
  const started = performance.now();
  const { rpc } = roomFixture();
  const restarts: Array<[string, string]> = [];
  let resumed = false;
  const app = await fixture.mount(context, {
    props: { visible: true },
    setup(window) {
      // The stopped card leaves a keyed list; jsdom has no active animations.
      window.Element.prototype.getAnimations = () => [];
    },
    modules: [{
      ...rpc,
      projectList: async () => ({ projects: [{
        project: { id: 'fixture', name: 'Fixture', session: 'fixture', path: '/fixture' },
        live: true, slots: [{ window_name: 'paused', kind: 'agent', command: 'codex' }],
      }] }),
      hubAgents: async () => ({ agents: [
        ...(await rpc.hubAgents()).agents,
        ...(resumed ? [{ name: 'paused', window: 2, managed: true, agent: 'codex', state: 'idle' }] : []),
      ] }),
      hubAgentRestart: async (session: string, name: string) => {
        restarts.push([session, name]); resumed = true; return {};
      },
    }],
  });
  try {
    for (let i = 0; i < 10 && !app.document.querySelector('.acard.off'); i++) await app.flush();
    const off = app.document.querySelector<HTMLElement>('.acard.off')!;
    assert.ok(off);
    off.querySelector<HTMLButtonElement>('.agent-select')!.click();
    await app.flush();
    assert.deepEqual(restarts, []);
    assert.equal(app.document.querySelector('.ctx-who')?.textContent, 'paused');
    app.window.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await app.flush();
    assert.equal(app.document.querySelector('.ctx'), null);
    off.querySelector<HTMLButtonElement>('.agent-select')!.click();
    await app.flush();
    const restart = [...app.document.querySelectorAll<HTMLButtonElement>('.ctx button')]
      .find((button) => button.textContent?.includes('Resume'))!;
    assert.ok(restart);
    restart.click();
    for (let i = 0; i < 10 && app.document.querySelector('.acard.off'); i++) await app.flush();
    assert.deepEqual(restarts, [['fixture', 'paused']]);
    assert.equal(app.document.querySelector('.acard.off'), null);
    const bob = [...app.document.querySelectorAll<HTMLElement>('.acard')]
      .find((card) => card.querySelector('.a-name')?.textContent === 'bob')!;
    assert.ok(bob);
    const recipientBeforeFilter = selectedCard(app.document);
    for (const shouldFilter of [true, false]) {
      bob.querySelector('.agent-select')!.dispatchEvent(new app.window.MouseEvent('contextmenu', { bubbles: true }));
      await app.flush();
      const filter = [...app.document.querySelectorAll<HTMLButtonElement>('.ctx button')]
        .find((button) => /Only its messages|Show everything/.test(button.textContent ?? ''))!;
      assert.ok(filter);
      filter.click();
      await app.flush();
      assert.equal(!!app.document.querySelector('.filter-pill'), shouldFilter);
      assert.equal(selectedCard(app.document), recipientBeforeFilter, 'filtering never changes delivery');
    }
  } finally {
    await app.close();
  }
  context.diagnostic(`Roster scenario after shared compilation ${(performance.now() - started).toFixed(1)}ms`);
});

// Composer extraction characterization: use the same compiled Hub and RPC door.
// Clipboard events are synthetic; native insertion and geometry belong to Chromium.
function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

async function composerFixture(context: TestContext, extra: Record<string, (...args: any[]) => unknown> = {}, mobile = false, storage: Record<string, string> = {}) {
  const fixture = await compiledHub();
  const { rpc } = roomFixture();
  const app = await fixture.mount(context, {
    props: { visible: true, mobile },
    setup(window) {
      window.Element.prototype.getAnimations = () => [];
      Object.defineProperty(window.performance, 'now', { value: () => window.Date.now() });
      for (const [key, value] of Object.entries(storage)) window.localStorage.setItem(key, value);
    },
    modules: [{
      ...rpc,
      projectList: async () => ({ projects: ['fixture', 'other'].map((session) => ({
        project: { id: session, name: session, session, path: `/${session}` },
        live: true, slots: [],
      })) }),
      modelsList: async () => ({ models: ['test-model'] }),
      fsMkdir: async () => ({}),
      ...extra,
    }],
  });
  const wait = async (predicate: () => boolean) => {
    for (let i = 0; i < 20 && !predicate(); i++) await app.flush();
    assert.ok(predicate(), 'composer state settled');
  };
  await wait(() => selectedCard(app.document) === 'alice');
  const input = app.document.querySelector<HTMLTextAreaElement>('.c-input')!;
  const send = app.document.querySelector<HTMLButtonElement>('.composer-actions button:last-child')!;
  const text = async (value: string) => {
    input.value = value;
    input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
    await app.flush();
  };
  const key = async (key: string, options: KeyboardEventInit = {}) => {
    const event = new app.window.KeyboardEvent('keydown', { key, bubbles: true, cancelable: true, ...options });
    input.dispatchEvent(event);
    await app.flush();
    return event.defaultPrevented;
  };
  const room = async (name: string) => {
    app.document.querySelector<HTMLElement>(`[aria-label="${name}"] .proj-pick`)!.click();
    await wait(() => app.document.querySelector('.h1-text')?.textContent === name);
  };
  const to = async (name: string) => {
    const target = name === 'everyone' ? 'all' : name === 'note' ? '' : name;
    const current = selectedCard(app.document);
    if (current === target) return;
    const button = stripCard(app.document, target || current)?.querySelector<HTMLButtonElement>('.agent-select');
    assert.ok(button, name);
    button.click();
    await app.flush();
  };
  const paste = async (files: File[], words = '') => {
    const event = new app.window.Event('paste', { bubbles: true, cancelable: true });
    Object.defineProperty(event, 'clipboardData', { value: {
      files, items: [], getData: (type: string) => type === 'text/plain' ? words : '',
    } });
    input.dispatchEvent(event);
    await app.flush();
    return event.defaultPrevented;
  };
  return { ...app, input, send, text, key, room, to, wait, paste };
}

test('Composer keeps readline caret, palette, room drafts and all three destinations', { timeout: 60000 }, async (context) => {
  const posts: Array<[string, string]> = [];
  const app = await composerFixture(context, {
    hubPost: async (session: string, body: string) => { posts.push([session, body]); return {}; },
  });
  try {
    app.input.focus();
    await app.text('one two');
    app.input.setSelectionRange(4, 4);
    assert.equal(await app.key('k', { ctrlKey: true }), true);
    assert.equal(String(app.input.value), 'one ');
    assert.equal(app.input.selectionStart, 4, 'caret lands after Svelte writes the value');
    await app.room('other');
    await app.key('y', { ctrlKey: true });
    assert.equal(String(app.input.value), 'two', 'kill buffer survives project switches');
    await app.room('fixture');
    assert.equal(String(app.input.value), 'one ', 'draft restored from the leaving room');
    await app.text('/');
    await app.key('Tab');
    assert.notEqual(app.input.value, '/');
    assert.equal(app.document.activeElement, app.input);
    assert.equal(app.input.selectionStart, app.input.value.length);
    await app.text('hello');
    assert.equal(await app.key('Enter', { shiftKey: true }), false);
    assert.equal(await app.key('Enter', { isComposing: true }), false);
    assert.deepEqual(posts, []);
    await app.key('Enter');
    await app.wait(() => posts.length === 1);
    await app.to('everyone');
    await app.text('broadcast');
    app.send.click();
    await app.wait(() => posts.length === 2);
    await app.to('note');
    await app.text('record');
    app.send.click();
    await app.wait(() => posts.length === 3);
    assert.deepEqual(posts, [['fixture', '@alice hello'], ['fixture', '@all broadcast'], ['fixture', 'record']]);
    await app.room('other');
    await app.room('fixture');
    assert.equal(selectedCard(app.document), '', 'an explicit room recipient persists');
  } finally { await app.close(); }
});

test('body mentions mark cards without replacing the selected delivery target (#168)', { timeout: 60000 }, async (context) => {
  const posts: string[] = [];
  const app = await composerFixture(context, {
    hubPost: async (_session: string, body: string) => { posts.push(body); return {}; },
  });
  try {
    await app.text('@bob Review the change.');
    assert.equal(selectedCard(app.document), 'alice');
    assert.equal(stripCard(app.document, 'bob').querySelector('.agent-mention')?.textContent, '@');
    app.send.click();
    await app.wait(() => posts.length === 1);
    assert.deepEqual(posts, ['@alice @bob Review the change.']);
    await app.to('note');
    await app.text('@all Report progress.');
    for (const name of ['all', 'alice', 'bob']) {
      assert.equal(stripCard(app.document, name).querySelector('.agent-mention')?.textContent, '@');
    }
    assert.equal(selectedCard(app.document), '');
    app.send.click();
    await app.wait(() => posts.length === 2);
    assert.equal(posts[1], '@all Report progress.');
  } finally { await app.close(); }
});

test('desktop double-click focuses an agent and toggles its reading filter without a delay (#173)', { timeout: 60000 }, async (context) => {
  const app = await composerFixture(context);
  const double = async () => {
    const button = stripCard(app.document, 'bob').querySelector<HTMLButtonElement>('.agent-select')!;
    for (const [type, detail] of [['click', 1], ['click', 2], ['dblclick', 2]] as const) {
      button.dispatchEvent(new app.window.MouseEvent(type, { detail, bubbles: true, cancelable: true }));
      await app.flush();
    }
  };
  try {
    assert.equal(selectedCard(app.document), 'alice');
    await double();
    assert.equal(selectedCard(app.document), 'bob');
    assert.ok(app.document.querySelector('.filter-pill'));
    assert.ok(stripCard(app.document, 'bob').classList.contains('filtered'));
    await double();
    assert.equal(selectedCard(app.document), 'bob');
    assert.equal(app.document.querySelector('.filter-pill'), null);
    assert.equal(stripCard(app.document, 'bob').classList.contains('filtered'), false);
  } finally { await app.close(); }
});

test('stopped-card double-click only filters and closes its click menu (#173)', { timeout: 60000 }, async (context) => {
  const app = await composerFixture(context, {
    projectList: async () => ({ projects: [{
      project: { id: 'fixture', name: 'fixture', session: 'fixture', path: '/fixture' },
      live: true, slots: [{ window_name: 'paused', kind: 'agent', command: 'codex' }],
    }] }),
  });
  try {
    const button = stripCard(app.document, 'paused').querySelector<HTMLButtonElement>('.agent-select')!;
    button.dispatchEvent(new app.window.MouseEvent('click', { detail: 1, bubbles: true }));
    await app.flush();
    assert.ok(app.document.querySelector('.ctx'));
    button.dispatchEvent(new app.window.MouseEvent('click', { detail: 2, bubbles: true }));
    button.dispatchEvent(new app.window.MouseEvent('dblclick', { detail: 2, bubbles: true }));
    await app.flush();
    assert.equal(selectedCard(app.document), 'alice');
    assert.equal(app.document.querySelector('.ctx'), null);
    assert.ok(app.document.querySelector('.filter-pill'));
    assert.ok(stripCard(app.document, 'paused').classList.contains('filtered'));
  } finally { await app.close(); }
});

test('Watch routes the clicked agent on phone and narrow desktop without selecting it (#173)', { timeout: 60000 }, async (context) => {
  for (const mobile of [true, false]) {
    const routes: unknown[][] = [];
    const { rpc } = roomFixture();
    const panes = [
      { session: 'fixture', window: 0, pane: 0, active: true, current_command: 'kiro', window_name: 'alice', pane_title: 'alice', current_path: '/fixture', width: 80, height: 24 },
      { session: 'fixture', window: 1, pane: 0, active: true, current_command: 'codex', window_name: 'bob', pane_title: 'bob', current_path: '/fixture', width: 80, height: 24 },
    ];
    const app = await (await compiledHub()).mount(context, {
      props: { visible: true, mobile, openTerminal: (...args: unknown[]) => routes.push(args) },
      setup(window) {
        window.Element.prototype.getAnimations = () => [];
        const media = window.matchMedia.bind(window);
        window.matchMedia = query => Object.assign(media(query), { matches: query === '(max-width: 760px)' || (mobile && query === '(any-pointer: coarse)') });
      },
      modules: [{
        ...rpc,
        listSessionsWithPanes: async () => ({ panes }),
      }],
    });
    try {
      for (let i = 0; i < 12 && selectedCard(app.document) !== 'alice'; i++) await app.flush();
      if (mobile) {
        stripCard(app.document, 'bob').querySelector('.agent-select')!.dispatchEvent(new app.window.MouseEvent('contextmenu', { bubbles: true }));
        await app.flush();
        const watch = [...app.document.querySelectorAll<HTMLButtonElement>('.ctx button')].find(button => button.textContent?.includes('Watch in terminal'))!;
        assert.ok(watch); watch.click();
      } else {
        const watch = stripCard(app.document, 'bob').querySelector<HTMLButtonElement>('.agent-watch button');
        assert.ok(watch); watch.click();
      }
      await app.flush();
      assert.deepEqual(routes, [['fixture', 'fixture:1.0', 'codex']]);
      assert.equal(selectedCard(app.document), 'alice');
      panes.splice(0, 1);
      await app.advance(20000);
      if (mobile) {
        stripCard(app.document, 'alice').querySelector('.agent-select')!.dispatchEvent(new app.window.MouseEvent('contextmenu', { bubbles: true }));
        await app.flush();
        [...app.document.querySelectorAll<HTMLButtonElement>('.ctx button')].find(button => button.textContent?.includes('Watch in terminal'))!.click();
      } else stripCard(app.document, 'alice').querySelector<HTMLButtonElement>('.agent-watch button')!.click();
      await app.flush();
      assert.equal(routes.length, 1, 'a missing pane must not reuse the previously watched agent');
    } finally { await app.close(); }
  }
});

test('roster disclosure keeps its cards, remembers each room, and holds order during interaction (#168)', { timeout: 60000 }, async (context) => {
  const agents = [
    { name: 'alice', window: 0, managed: true, agent: 'kiro', state: 'idle', since: 1000 },
    { name: 'bob', window: 1, managed: true, agent: 'codex', state: 'running', since: 10, team: 'review' },
    { name: 'charlie', window: 2, managed: true, agent: 'codex', state: 'waiting', since: 20, team: 'review/backend' },
  ];
  const interrupts: string[] = [];
  const app = await composerFixture(context, {
    hubAgents: async () => ({ agents }),
    hubAgentInterrupt: async (_session: string, name: string) => { interrupts.push(name); return {}; },
  });
  const order = () => [...app.document.querySelectorAll<HTMLElement>('.acard[data-agent]')].map((node) => node.dataset.agent);
  const toggle = () => app.document.querySelector<HTMLButtonElement>('.roster-toggle button')!;
  try {
    // Owner #176 moves the bulk destination after identities, not ahead of them.
    assert.deepEqual(order(), ['charlie', 'bob', 'alice', 'all']);
    const alice = stripCard(app.document, 'alice');
    const list = app.document.querySelector('.cards')!;
    assert.doesNotMatch(list.textContent!, /running|waiting|idle/u);
    assert.equal(toggle().getAttribute('aria-expanded'), 'false');
    toggle().click(); await app.flush();
    assert.equal(toggle().getAttribute('aria-expanded'), 'true');
    assert.equal(stripCard(app.document, 'alice'), alice, 'expansion does not remount cards');
    assert.equal(app.document.querySelector('.cards'), list);
    await app.room('other');
    assert.equal(toggle().getAttribute('aria-expanded'), 'false');
    await app.room('fixture');
    assert.equal(toggle().getAttribute('aria-expanded'), 'true', 'each room restores its own disclosure');

    agents[0]!.state = 'running'; agents[0]!.since = 5000;
    agents[2]!.state = 'idle'; agents[2]!.since = 6000;
    await app.advance(5000);
    assert.deepEqual(order(), ['alice', 'bob', 'charlie', 'all'], 'an idle expanded monitor follows new turn order');
    assert.ok(stripCard(app.document, 'alice').querySelector('.agent-stop button'));
    assert.equal(stripCard(app.document, 'charlie').querySelector('.agent-stop'), null,
      'new turn state updates Stop availability');
    toggle().click(); await app.flush();
    assert.deepEqual(order(), ['alice', 'bob', 'charlie', 'all'], 'collapse adopts the new turn order');

    const bobStop = stripCard(app.document, 'bob').querySelector<HTMLButtonElement>('.agent-stop button')!;
    bobStop.dispatchEvent(new app.window.Event('pointerdown', { bubbles: true }));
    agents[1]!.since = 10000;
    await app.advance(5000);
    assert.deepEqual(order(), ['alice', 'bob', 'charlie', 'all'], 'a pressed Stop cannot move to another card');
    bobStop.click(); await app.flush();
    assert.deepEqual(interrupts, ['bob']);
    assert.equal(selectedCard(app.document), 'alice');
    assert.deepEqual(order(), ['bob', 'alice', 'charlie', 'all']);
  } finally { await app.close(); }
});

test('a restored expanded roster reorders live unless a pointer or focus holds it (#168)', { timeout: 60000 }, async (context) => {
  const agents = [
    { name: 'alice', window: 0, managed: true, agent: 'kiro', state: 'running', since: 10 },
    { name: 'bob', window: 1, managed: true, agent: 'codex', state: 'running', since: 20 },
  ];
  const app = await composerFixture(context, { hubAgents: async () => ({ agents }) }, false,
    { tmux_hub_roster_expanded: JSON.stringify({ fixture: true }) });
  const order = () => [...app.document.querySelectorAll<HTMLElement>('.acard[data-agent]')].map((node) => node.dataset.agent);
  try {
    assert.equal(app.document.querySelector('.roster-toggle button')!.getAttribute('aria-expanded'), 'true');
    assert.deepEqual(order(), ['bob', 'alice', 'all']);
    agents[0]!.since = 40;
    await app.advance(5000);
    assert.deepEqual(order(), ['alice', 'bob', 'all']);
    const list = app.document.querySelector('.cards')!;
    list.dispatchEvent(new app.window.Event('pointerenter'));
    agents.push({ name: 'new', window: 2, managed: true, agent: 'codex', state: 'running', since: 50 });
    await app.advance(5000);
    assert.deepEqual(order(), ['alice', 'bob', 'new', 'all'], 'new members append without moving a pointed-at target');
    list.dispatchEvent(new app.window.Event('pointerleave'));
    await app.flush();
    assert.deepEqual(order(), ['new', 'alice', 'bob', 'all'], 'pointerleave releases order without a timer');
    stripCard(app.document, 'alice').querySelector<HTMLButtonElement>('.agent-select')!.focus();
    agents[1]!.since = 60;
    await app.advance(5000);
    assert.deepEqual(order(), ['new', 'alice', 'bob', 'all'], 'keyboard focus holds its target');
    app.input.focus(); await app.flush();
    assert.deepEqual(order(), ['bob', 'new', 'alice', 'all'], 'blur releases the new turn order immediately');
  } finally { await app.close(); }
});

test('roster releases a touch press without click and reconciles focus after a control disappears (#168)', { timeout: 60000 }, async (context) => {
  const agents = [
    { name: 'alice', window: 0, managed: true, agent: 'kiro', state: 'running', since: 10 },
    { name: 'bob', window: 1, managed: true, agent: 'codex', state: 'running', since: 20 },
  ];
  const job = deferred<object>();
  const app = await composerFixture(context, {
    hubAgents: async () => ({ agents }),
    hubAgentInterrupt: () => job.promise,
  });
  const order = () => [...app.document.querySelectorAll<HTMLElement>('.acard[data-agent]')].map((node) => node.dataset.agent);
  const pointer = (element: Element, type: string) => {
    const event = new app.window.Event(type, { bubbles: true });
    Object.defineProperty(event, 'pointerType', { value: 'touch' });
    element.dispatchEvent(event);
  };
  try {
    const stop = stripCard(app.document, 'bob').querySelector<HTMLButtonElement>('.agent-stop button')!;
    stop.click(); await app.flush();
    assert.equal(stop.disabled, true);
    pointer(stop, 'pointerdown');
    agents[0]!.since = 40;
    await app.advance(5000);
    assert.deepEqual(order(), ['bob', 'alice', 'all']);
    pointer(stop, 'pointerup');
    await app.flush();
    assert.deepEqual(order(), ['alice', 'bob', 'all'], 'disabled touch targets may release without any click');
    job.resolve({}); await app.flush();

    stop.focus(); await app.flush();
    agents[1]!.since = 50;
    await app.advance(5000);
    assert.deepEqual(order(), ['alice', 'bob', 'all'], 'focused controls keep their location');
    agents[0]!.state = 'idle'; agents[0]!.since = 80;
    agents[1]!.state = 'idle'; agents[1]!.since = 90;
    await app.advance(5000);
    assert.equal(stop.isConnected, false);
    assert.deepEqual(order(), ['bob', 'alice', 'all'], 'DOM removal does not guarantee focusout');
    stripCard(app.document, 'alice').querySelector<HTMLButtonElement>('.agent-select')!.focus();
    agents.splice(0, 1);
    await app.advance(5000);
    agents.push({ name: 'new', window: 2, managed: true, agent: 'codex', state: 'running', since: 100 });
    await app.advance(5000);
    assert.deepEqual(order(), ['new', 'bob', 'all'], 'removing a focused member does not leave order locked');
  } finally { job.resolve({}); await app.close(); }
});

test('send never interrupts; double Ctrl+C mirrors only the selected busy card (#168)', { timeout: 60000 }, async (context) => {
  const interrupts: Array<[string, string]> = [];
  const app = await composerFixture(context, {
    hubAgents: async () => ({ agents: [
      { name: 'alice', window: 0, managed: true, agent: 'kiro', state: 'running' },
      { name: 'bob', window: 1, managed: true, agent: 'codex', state: 'blocked' },
      { name: 'idle', window: 2, managed: true, agent: 'codex', state: 'idle' },
      { name: 'shell', window: 3, managed: false, state: 'running' },
    ] }),
    hubAgentInterrupt: async (session: string, name: string) => { interrupts.push([session, name]); return {}; },
  }, true);
  const ctrlC = () => app.key('c', { ctrlKey: true });
  try {
    assert.equal(await app.key('Enter'), false, 'compact Enter remains a newline');
    assert.equal(app.send.disabled, true);
    app.send.click();
    await app.flush();
    await ctrlC();
    assert.deepEqual(interrupts, [], 'empty Send cannot be the first keyboard activation');
    await app.advance(3001);
    await ctrlC();
    assert.deepEqual(interrupts, [], 'an expired first key cannot fire');
    await app.key('Escape');
    await ctrlC();
    assert.deepEqual(interrupts, []);
    await app.text('copy me');
    assert.equal(await app.key('c', { ctrlKey: true }), false, 'nonempty Ctrl+C is native copy');
    await app.text('');
    await ctrlC();
    await app.to('bob');
    await ctrlC();
    assert.deepEqual(interrupts, [], 'recipient change resets the key sequence');
    await app.room('other');
    await ctrlC();
    assert.deepEqual(interrupts, [], 'room change resets the key sequence');
    await app.key('c', { ctrlKey: true, repeat: true });
    await app.key('c', { ctrlKey: true, isComposing: true });
    assert.deepEqual(interrupts, [], 'repeat/composition cannot fire Stop');
    await ctrlC();
    await app.wait(() => interrupts.length === 1);
    assert.deepEqual(interrupts, [['other', 'alice']]);
    await app.to('everyone');
    await ctrlC();
    await ctrlC();
    await app.wait(() => interrupts.length === 3);
    assert.deepEqual(interrupts.slice(1), [['other', 'alice'], ['other', 'bob']],
      '@all excludes idle and unmanaged windows');
    await app.to('idle');
    await ctrlC();
    await ctrlC();
    await app.to('note');
    assert.equal(app.send.disabled, true);
    await ctrlC();
    await ctrlC();
    assert.equal(interrupts.length, 3);
    assert.equal(app.document.querySelector('.int-pill, .to-chip'), null);
  } finally { await app.close(); }
});

test('a peer Stop captures its room, deduplicates pending clicks and never selects or kills (#168)', { timeout: 60000 }, async (context) => {
  const jobs: Array<{ session: string; name: string; task: ReturnType<typeof deferred<object>> }> = [];
  const app = await composerFixture(context, {
    hubAgents: async () => ({ agents: [
      { name: 'alice', window: 0, managed: true, agent: 'kiro', state: 'running' },
      { name: 'bob', window: 1, managed: true, agent: 'codex', state: 'working' },
    ] }),
    hubAgentInterrupt: (session: string, name: string) => {
      const task = deferred<object>();
      jobs.push({ session, name, task });
      return task.promise;
    },
    hubAgentStop: () => assert.fail('Stop response must not kill the process'),
  });
  const stop = (name: string) => stripCard(app.document, name).querySelector<HTMLButtonElement>('.agent-stop button')!;
  try {
    stop('bob').click();
    stop('bob').click();
    await app.flush();
    assert.deepEqual(jobs.map(({ session, name }) => [session, name]), [['fixture', 'bob']]);
    assert.equal(selectedCard(app.document), 'alice');
    assert.equal(stop('bob').disabled, true);
    assert.equal(stop('alice').disabled, false);
    await app.room('other');
    assert.equal(stop('bob').disabled, false, 'another room has its own pending identity');
    stop('bob').click();
    await app.flush();
    jobs[0]!.task.resolve({});
    await app.flush();
    assert.equal(stop('bob').disabled, true, 'old room completion cannot clear the new job');
    jobs[1]!.task.resolve({});
    await app.flush();
    assert.equal(stop('bob').disabled, false);
    assert.equal(selectedCard(app.document), 'alice');
    assert.deepEqual(jobs.map(({ session, name }) => [session, name]), [['fixture', 'bob'], ['other', 'bob']]);
  } finally {
    for (const job of jobs) job.task.resolve({});
    await app.close();
  }
});

test('Composer paste keeps Office words, stages files once and isolates overlapping generations', { timeout: 60000 }, async (context) => {
  const uploads: Array<{ path: string; job: ReturnType<typeof deferred<object>> }> = [];
  const posts: string[] = [];
  const app = await composerFixture(context, {
    fsUpload: async (path: string) => {
      if (path.endsWith('/.gitignore')) return {};
      const job = deferred<object>();
      uploads.push({ path, job });
      return job.promise;
    },
    hubPost: async (_session: string, body: string) => { posts.push(body); return {}; },
  });
  try {
    const file = (name: string) => new app.window.File(['bytes'], name, { type: 'text/plain' });
    const image = new app.window.File(['rendering'], 'selection.png', { type: 'image/png' });
    assert.equal(await app.paste([image], 'Quarterly results'), false, 'Office words retain native insertion');
    assert.equal(uploads.length, 0, 'a picture of the text is never staged');
    await app.text('left right');
    app.input.setSelectionRange(5, 5);
    assert.equal(await app.paste([file('first.txt')], '/first.txt'), true);
    await app.wait(() => uploads.length === 1);
    await app.paste([file('second.txt')]);
    await app.wait(() => uploads.length === 2);
    uploads[0]!.job.resolve({});
    await app.wait(() => app.input.value.includes('[file:1]'));
    assert.equal(app.input.value, 'left [file:1]right', 'token uses the click-time caret');
    assert.equal(app.send.disabled, true, 'the second same-room job keeps the gate closed');
    await app.room('other');
    await app.text('new room');
    assert.equal(app.send.disabled, false, 'an old job does not lock the new generation');
    await app.paste([file('third.txt')]);
    await app.wait(() => uploads.length === 3);
    uploads[1]!.job.resolve({});
    await app.flush();
    assert.equal(app.send.disabled, true, 'old finally cannot unlock the new job');
    assert.equal(app.input.value, 'new room');
    uploads[2]!.job.reject(new Error('upload denied'));
    await app.wait(() => app.document.querySelector('.pend-chip.err') !== null);
    assert.match(app.document.querySelector('.pend-why')!.textContent!, /upload denied/);
    assert.equal(app.send.disabled, true);
    await app.key('Enter');
    assert.deepEqual(posts, [], 'failed chips block the key path as well as the button');
    app.document.querySelector<HTMLElement>('.pend-x')!.click();
    await app.flush();
    assert.equal(app.send.disabled, false);
    await app.paste([file('fourth.txt')]);
    await app.wait(() => uploads.length === 4);
    uploads[3]!.job.resolve({});
    await app.wait(() => app.input.value.includes('[file:1]'));
    assert.equal(app.document.activeElement, app.input, 'completed staging restores focus');
    assert.equal(app.document.querySelectorAll('.pend-chip').length, 1);
  } finally {
    for (const { job } of uploads) job.resolve({});
    await app.close();
  }
});

test('Composer post and command failures never restore into the next room', { timeout: 60000 }, async (context) => {
  const requests: Array<{ method: string; session: string; body: string; job: ReturnType<typeof deferred<object>> }> = [];
  const call = (method: string, session: string, body: string) => {
    const job = deferred<object>();
    requests.push({ method, session, body, job });
    return job.promise;
  };
  const app = await composerFixture(context, {
    hubPost: (session: string, body: string) => call('post', session, body),
    hubCommand: (session: string, target: string, body: string) => call(`command:${target}`, session, body),
  });
  try {
    for (const [index, draft, method] of [[0, 'hello', 'post'], [1, '/clear', 'command:alice']] as const) {
      await app.room('fixture');
      await app.text(draft);
      app.send.click();
      await app.wait(() => requests.length === index + 1);
      assert.equal(app.input.value, '');
      assert.equal(requests[index]!.session, 'fixture');
      assert.equal(requests[index]!.method, method);
      await app.room('other');
      await app.text(`next ${index}`);
      requests[index]!.job.reject(new Error('send rejected'));
      await app.flush();
      assert.equal(app.input.value, `next ${index}`);
    }
    await app.text('same room');
    app.send.click();
    await app.wait(() => requests.length === 3);
    requests[2]!.job.reject(new Error('retry'));
    await app.wait(() => app.input.value === 'same room');
  } finally {
    for (const { job } of requests) job.resolve({});
    await app.close();
  }
});

test('Feed keeps native selection, Copy/Raw dismissal, path intents and room-local choices', { timeout: 60000 }, async (context) => {
  const fixture = await compiledHub();
  const { rpc, pushed } = roomFixture();
  const copied: string[] = [];
  const routes: unknown[][] = [];
  const boards: unknown[][] = [];
  const body = 'Reply with **formatting** and [source](/fixture/source.txt).';
  const app = await fixture.mount(context, {
    props: { visible: true, mobile: true,
      openFilesTab: (...args: unknown[]) => { routes.push(args); },
      openBoardTab: (...args: unknown[]) => { boards.push(args); },
    },
    setup(window) {
      window.Element.prototype.getAnimations = () => [];
      // jsdom has no canvas renderer. Do not invent glyph widths here:
      // native Chromium owns the actual measurement/anchor characterization.
      window.HTMLCanvasElement.prototype.getContext = () => null;
      Object.defineProperty(window.navigator, 'clipboard', { value: {
        writeText: async (text: string) => { copied.push(text); },
      } });
    },
    modules: [{
      ...rpc,
      projectList: async () => ({ projects: ['fixture', 'other'].map((session) => ({
        project: { id: session, name: session, session, path: `/${session}` }, live: true, slots: [],
      })) }),
      modelsList: async () => ({ models: [] }),
      hubLog: async () => ({ messages: [
        { id: 'question', seq: 1, ts: 100, from: 'human', body: '@alice Review this.' },
        { id: 'reply', seq: 2, ts: 200, from: 'alice', body },
        { id: 'board', seq: 3, ts: 300, from: 'alice', body: '[tmm] board #7 todo → doing — Feed extraction' },
      ], has_more: false }),
      hubActivity: async () => ({ events: [
        { id: 1, ts: 250, window: 'alice', kind: 'tool', tool: 'Read', text: 'source.ts' },
        { id: 2, ts: 251, window: 'bob', kind: 'tool', tool: 'Bash', text: 'npm test' },
      ], has_more: false }),
    }],
  });
  try {
    for (let i = 0; i < 12 && app.document.querySelectorAll('.msg').length !== 2; i++) await app.flush();
    const reply = () => app.document.querySelector<HTMLElement>('.msg:not(.me)')!;
    const bubble = () => reply().querySelector<HTMLElement>('.bubble')!;
    const actions = () => reply().querySelector<HTMLElement>('.m-acts');
    assert.ok(reply());
    assert.equal(reply().parentElement?.classList.contains('feed'), true);
    const selection = app.window.getSelection()!;
    const range = app.document.createRange();
    range.selectNodeContents(reply().querySelector('.m-body')!);
    selection.addRange(range);
    bubble().click();
    await app.flush();
    assert.equal(actions(), null, 'a real Selection object wins over the bubble click');
    selection.removeAllRanges();
    const hold = new app.window.MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    Object.defineProperty(hold, 'pointerType', { value: 'touch' });
    bubble().dispatchEvent(hold);
    assert.equal(hold.defaultPrevented, false, 'contextmenu is a passive mark, not a replacement menu');
    bubble().click();
    await app.flush();
    assert.equal(actions(), null, 'the compatibility click is consumed once');
    bubble().click();
    await app.flush();
    assert.ok(actions());
    const rawButton = actions()!.querySelector<HTMLButtonElement>('button:last-child')!;
    assert.ok(rawButton.classList.contains('command-button'), '#166: shared command states, not private bubble paint');
    assert.equal(rawButton.getAttribute('aria-label'), 'Raw');
    assert.equal(rawButton.getAttribute('aria-pressed'), 'false');
    actions()!.querySelector<HTMLElement>('button:last-child')!.click();
    await app.flush();
    assert.equal(reply().querySelector('.raw')?.textContent, body);
    assert.equal(rawButton.getAttribute('aria-pressed'), 'true');
    assert.ok(rawButton.classList.contains('engaged'), 'selected source view has the shared visible state');
    const composer = app.document.querySelector<HTMLTextAreaElement>('.c-input')!;
    composer.value = '/';
    composer.dispatchEvent(new app.window.Event('input', { bubbles: true }));
    await app.flush();
    assert.ok(app.document.querySelector('.cmd-menu'));
    app.window.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await app.flush();
    assert.equal(actions(), null);
    assert.equal(app.document.querySelector('.cmd-menu'), null, 'one capture callback closes both surviving territories');
    assert.ok(reply().querySelector('.raw'), 'Escape closes the actions, never the raw reading mode');
    reply().querySelector<HTMLElement>('.m-meta')!.click();
    await app.flush();
    actions()!.querySelector<HTMLElement>('button:first-child')!.click();
    await app.flush();
    assert.deepEqual(copied, [body], 'copy uses exact source rather than rendered text');
    await app.advance(1499);
    assert.ok(actions());
    await app.advance(1);
    assert.equal(actions(), null);
    const head = app.document.querySelector<HTMLElement>('.s-head')!;
    head.click();
    await app.flush();
    assert.equal(head.getAttribute('aria-expanded'), 'false');
    app.document.querySelector<HTMLElement>('[aria-label="other"] .proj-pick')!.click();
    for (let i = 0; i < 12 && app.document.querySelector('.h1-text')?.textContent !== 'other'; i++) await app.flush();
    assert.equal(reply().querySelector('.raw'), null, 'room reset clears raw without remounting the whole Feed');
    assert.equal(app.document.querySelector('.s-head')?.getAttribute('aria-expanded'), 'false',
      'the same tool group retains its explicit disclosure choice across rooms');
    const link = reply().querySelector<HTMLAnchorElement>('a')!;
    for (const [type, options] of [
      ['click', {}], ['click', { metaKey: true }], ['auxclick', { button: 1 }],
    ] as const) {
      const event = new app.window.MouseEvent(type, { bubbles: true, cancelable: true, ...options });
      link.dispatchEvent(event);
      await app.flush();
      assert.equal(event.defaultPrevented, true);
      assert.equal(actions(), null, 'path routing wins over Copy/Raw toggling');
    }
    assert.deepEqual(routes, Array.from({ length: 3 }, () => ['other', '', '/fixture/source.txt']));
    app.document.querySelector<HTMLElement>('.sys-jump')!.click();
    await app.flush();
    assert.deepEqual(boards, [['other', 7]], 'the feed board row carries the clicked issue id');
  } finally { await app.close(); }
  assert.equal(pushed.size, 0);
});

test('Chat header commands report drawer state and preserve compact navigation (#166)', { timeout: 60000 }, async context => {
  const fixture = await compiledHub();
  for (const compact of [false, true]) {
    const { rpc } = roomFixture();
    const routes: unknown[] = [];
    const app = await fixture.mount(context, {
      props: { visible: true, mobile: compact, openFilesTab: (...args: unknown[]) => routes.push(args) },
      setup(window) { window.Element.prototype.getAnimations = () => []; },
      modules: [{
        ...rpc, fsList: async () => ({ entries: [] }), fsCwd: async () => ({ path: '/fixture' }),
        getPrefs: async () => ({}), getBookmarks: async () => ({ bookmarks: [] }),
        gitCmd: async () => ({ code: 1 }),
      }],
    });
    try {
      for (let i = 0; i < 12 && !app.document.querySelector('.h1-text')?.textContent; i++) await app.flush();
      const files = app.document.querySelector<HTMLButtonElement>('.page-head [aria-label="Files"]')!;
      assert.ok(files.classList.contains('command-button'));
      assert.equal(files.getAttribute('aria-expanded'), compact ? null : 'false');
      assert.equal(files.getAttribute('aria-controls'), null, 'a closed/unmounted drawer has no dangling relationship');
      files.click();
      for (let i = 0; i < 12; i++) await app.flush();
      if (compact) {
        assert.equal(app.document.querySelector('.drawer'), null);
        assert.equal(routes.length, 1, 'phone navigates instead of pretending to disclose a drawer');
      } else {
        assert.equal(files.getAttribute('aria-expanded'), 'true');
        assert.ok(app.document.getElementById(files.getAttribute('aria-controls')!)?.querySelector('.drawer'));
        app.document.querySelector<HTMLButtonElement>('.drawer-head [aria-label="Close"]')!.click();
        for (let i = 0; i < 12; i++) await app.flush();
        assert.equal(files.getAttribute('aria-expanded'), 'false');
      }
    } finally { await app.close(); }
  }
});

test('Chat picker commands retain choice, disabled start and the original spawn intent (#166)', { timeout: 60000 }, async context => {
  const { rpc } = roomFixture();
  const spawned: unknown[] = [];
  const app = await (await compiledHub()).mount(context, {
    props: { visible: true },
    modules: [{
      ...rpc,
      registryList: async () => ({ agents: [{ name: 'helper', backend: 'kiro' }] }),
      hubSpawn: async (...args: unknown[]) => { spawned.push(args); return {}; },
    }],
  });
  try {
    for (let i = 0; i < 12 && !app.document.querySelector('.roster-add button'); i++) await app.flush();
    app.document.querySelector<HTMLButtonElement>('.roster-add button')!.click();
    await app.flush();
    const start = app.document.querySelector<HTMLButtonElement>('.dlg-actions .command-button.primary')!;
    assert.ok(start);
    assert.ok(start.disabled, 'no selection still means no spawn');
    const choice = app.document.querySelector<HTMLButtonElement>('.dlg-agents .agent-pick')!;
    assert.equal(choice.getAttribute('aria-pressed'), 'false');
    choice.click();
    await app.flush();
    assert.equal(choice.getAttribute('aria-pressed'), 'true');
    assert.equal(start.disabled, false);
    assert.ok(start.querySelector('svg')?.children.length, 'the command uses an existing icon, not a blank glyph');
    start.click();
    await app.flush();
    assert.deepEqual(spawned, [['fixture', 'helper', '']]);
    assert.equal(app.document.querySelector('.dlg-agents'), null, 'the existing spawn path closes the picker');
  } finally { await app.close(); }
});

test('Feed observes its box once across data updates and releases the observer on unmount', { timeout: 60000 }, async (context) => {
  const fixture = await compiledHub();
  const { rpc, pushed } = roomFixture();
  const observers: Array<{ targets: Set<Element>; disconnected: boolean; fire: () => void }> = [];
  const app = await fixture.mount(context, {
    props: { visible: true },
    setup(window) {
      window.HTMLCanvasElement.prototype.getContext = () => null;
      window.document.hasFocus = () => true;
      Object.assign(window, {
        ResizeObserver: class {
          targets = new Set<Element>();
          disconnected = false;
          fire: () => void;
          constructor(callback: ResizeObserverCallback) {
            // This spy proves registration/scheduling, not browser geometry.
            this.fire = () => callback([], this as unknown as ResizeObserver);
            observers.push(this);
          }
          observe(target: Element) { this.targets.add(target); }
          unobserve(target: Element) { this.targets.delete(target); }
          disconnect() { this.disconnected = true; this.targets.clear(); }
        },
      });
    },
    modules: [rpc],
  });
  let owner: typeof observers[number] | undefined;
  try {
    for (let i = 0; i < 10 && !selectedCard(app.document); i++) await app.flush();
    const feed = app.document.querySelector('.feed')!;
    const owning = observers.filter((observer) => observer.targets.has(feed));
    assert.equal(owning.length, 1);
    owner = owning[0]!;
    owner.fire();
    await app.flush();
    const message = app.window.JSON.parse(JSON.stringify({
      id: 'new', seq: 1, room: 'proj:fixture', from: 'alice', ts: Date.now(), body: 'A new reply.',
    }));
    for (const listener of pushed) (listener as (message: unknown) => void)(message);
    for (let i = 0; i < 10 && !app.document.querySelector('.msg'); i++) await app.flush();
    assert.ok(app.document.querySelector('.msg'));
    assert.deepEqual(observers.filter((observer) => observer.targets.has(feed)), [owner],
      'data changes update the snapshot, not the observer lifetime');
    assert.equal(owner.disconnected, false);
  } finally { await app.close(); }
  assert.equal(owner?.disconnected, true);
  assert.equal(pushed.size, 0);
});

test('Feed settles image load/error before reapplying live tail intent', { timeout: 60000 }, async (context) => {
  const fixture = await compiledHub();
  const { rpc } = roomFixture();
  const app = await fixture.mount(context, {
    props: { visible: true },
    setup(window) { window.HTMLCanvasElement.prototype.getContext = () => null; },
    modules: [{
      ...rpc,
      hubLog: async () => ({ messages: [
        { id: 'image', seq: 1, ts: 100, from: 'alice', body: '![](/fixture/chart.png)' },
      ], has_more: false }),
      fsDownloadHttp: async () => ({ url: 'https://mount.test/chart.png' }),
    }],
  });
  try {
    for (let i = 0; i < 10; i++) await app.flush();
    const feed = app.document.querySelector<HTMLElement>('.feed')!;
    const image = feed.querySelector<HTMLImageElement>('.ci')!;
    assert.ok(image);
    // Observe the write and DOM-update order only. jsdom supplies no image
    // dimensions; cold/cached pixel gaps and history intent belong to Chromium.
    // Each tail write records what the image slot showed at that moment:
    // the picture, the grey reference (a re-sign in flight, board #175), or
    // the warn-coloured failed reference.
    const writes: string[] = [];
    Object.defineProperty(feed, 'scrollTop', {
      configurable: true, get: () => 0,
      set: () => { writes.push(feed.querySelector('.ci-ref.failed') ? 'failed' : feed.querySelector('.ci-ref') ? 'pending' : 'image'); },
    });
    image.dispatchEvent(new app.window.Event('load'));
    assert.deepEqual(writes, [], 'the capture handler must wait for the component update');
    await app.flush();
    assert.ok(writes.length > 0, 'a loaded image reapplies the live tail');
    assert.ok(writes.every((w) => w === 'image'));
    writes.length = 0;
    // First error: the signature is re-minted once (board #175) — the slot
    // shows the grey reference while the new url is fetched.
    image.dispatchEvent(new app.window.Event('error'));
    assert.deepEqual(writes, []);
    await app.flush();
    assert.ok(writes.length > 0, 'a failed load also changes the content height');
    assert.ok(writes.every((w) => w === 'pending'), 'the re-sign fallback is rendered before measuring/writing the tail');
    for (let i = 0; i < 4; i++) await app.flush();
    const retried = feed.querySelector<HTMLImageElement>('.ci')!;
    assert.ok(retried, 'the re-signed image mounts');
    writes.length = 0;
    // Second error: the file is unreachable — the failed reference, no loop.
    retried.dispatchEvent(new app.window.Event('error'));
    assert.deepEqual(writes, []);
    await app.flush();
    assert.ok(writes.length > 0);
    assert.ok(writes.every((w) => w === 'failed'), 'the failed fallback is rendered before measuring/writing the tail');
  } finally { await app.close(); }
});

test('Files opened from a project Chat starts at the DECLARED project path, not the active pane cwd (board #181)', { timeout: 60000 }, async context => {
  // Owner, 2026-09-12: "你应该默认从我们选定的项目路径去跑". fs_cwd answers with
  // whatever pane happens to be active in tmux (a worktree here); the project
  // declares /declared. Declaration over pane accident (tenets 7/9).
  const fixture = await compiledHub();
  const { rpc } = roomFixture();
  const listed: string[] = [];
  const stated: string[] = [];
  const routes: unknown[][] = [];
  const mountHub = (mobile: boolean) => fixture.mount(context, {
    props: { visible: true, mobile, openFilesTab: (...args: unknown[]) => routes.push(args) },
    setup(window) { window.Element.prototype.getAnimations = () => []; window.HTMLCanvasElement.prototype.getContext = () => null; },
    modules: [{
      ...rpc,
      projectList: async () => ({ projects: [{ project: { id: 'fixture', name: 'Fixture', session: 'fixture', path: '/declared' }, live: true, slots: [] }] }),
      hubLog: async () => ({ messages: [{ id: 'm1', seq: 1, ts: 100, from: 'alice', body: 'see [the plan](docs/plan.md)' }], has_more: false }),
      fsCwd: async () => ({ path: '/pane/worktree' }),
      fsList: async (path: string) => { listed.push(path); return { path, entries: path === '/declared' ? [{ name: 'docs', path: '/declared/docs', type: 'dir', size: 0 }] : [] }; },
      fsStat: async (path: string) => { stated.push(path); return { path, is_text: true, readable: true, writable: false, size: 1, mime_hint: 'text/markdown' }; },
      fsRead: async () => ({ content: '# plan' }),
      getPrefs: async () => ({}), setPref: async () => ({}), getBookmarks: async () => ({ bookmarks: [] }), gitCmd: async () => ({ code: 1 }),
    }],
  });
  const app = await mountHub(false);
  try {
    const settle = async (n = 12) => { for (let i = 0; i < n; i++) await app.flush(); };
    for (let i = 0; i < 12 && !app.document.querySelector('.h1-text')?.textContent; i++) await app.flush();
    const files = () => app.document.querySelector<HTMLButtonElement>('.page-head [aria-label="Files"]')!;
    // 1. Open Files: the first listing is the declared path.
    files().click();
    await settle();
    assert.ok(app.document.querySelector('.drawer .file-list'), 'the Files partition opened');
    assert.equal(listed[0], '/declared', `first listing is the project path, got ${listed.join(' → ')}`);
    assert.ok(!listed.includes('/pane/worktree'), 'the pane cwd is never the default');
    // 2. Browse into docs, close, reopen: the parked position still wins.
    app.document.querySelector<HTMLButtonElement>('.drawer .file-row .file-main')!.click();
    await settle();
    assert.equal(listed.at(-1), '/declared/docs');
    // Closing withdraws over moveMs (#174): the Drawer unmounts — and parks
    // its position — only after the move.
    const close = async () => { files().click(); await settle(); await app.advance(400); await settle(); };
    await close();
    assert.equal(app.document.querySelector('.drawer'), null);
    listed.length = 0;
    files().click(); await settle();       // reopen
    assert.equal(listed[0], '/declared/docs', 'a parked browse position wins over the default (file-handling.md)');
    assert.ok(!listed.includes('/declared'), 'and the default does not yank it back');
    await close();
    // 3. A relative path reference in chat resolves against the project path.
    const link = app.document.querySelector<HTMLAnchorElement>('.feed a[href="docs/plan.md"]')!;
    assert.ok(link, 'the relative reference rendered as a link');
    link.click();
    await settle();
    assert.equal(stated.at(-1), '/declared/docs/plan.md', `relative refs resolve against the project, got ${stated.join(' → ')}`);
  } finally { await app.close(); }
  // 4. The phone route hands the declared path to the Files page.
  const phone = await mountHub(true);
  try {
    for (let i = 0; i < 12 && !phone.document.querySelector('.h1-text')?.textContent; i++) await phone.flush();
    phone.document.querySelector<HTMLButtonElement>('.page-head [aria-label="Files"]')!.click();
    for (let i = 0; i < 6; i++) await phone.flush();
    assert.deepEqual(routes.at(-1), ['fixture', '/declared'], 'compact: the page is asked to start at the project path');
  } finally { await phone.close(); }
});
