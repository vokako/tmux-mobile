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

test('Roster double-click filters without a menu and a stopped surface never resumes', { timeout: 60000 }, async (context) => {
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
    off.click();
    await app.flush();
    assert.deepEqual(restarts, []);
    assert.equal(app.document.querySelector('.am-who')?.textContent, 'paused');
    app.window.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await app.flush();
    assert.equal(app.document.querySelector('.a-menu'), null);
    off.querySelector<HTMLButtonElement>('.a-start')!.click();
    for (let i = 0; i < 10 && app.document.querySelector('.acard.off'); i++) await app.flush();
    assert.deepEqual(restarts, [['fixture', 'paused']]);
    assert.equal(app.document.querySelector('.acard.off'), null);
    const bob = [...app.document.querySelectorAll<HTMLElement>('.acard')]
      .find((card) => card.querySelector('.a-name')?.textContent === 'bob')!;
    assert.ok(bob);
    for (const shouldFilter of [true, false]) {
      bob.click();
      bob.click();
      bob.dispatchEvent(new app.window.MouseEvent('dblclick', { bubbles: true }));
      await app.flush();
      await app.advance(260);
      assert.equal(app.document.querySelector('.a-menu'), null, 'double-click cancels the pending menu');
      assert.equal(!!app.document.querySelector('.filter-pill'), shouldFilter);
      assert.equal(app.document.querySelector('.to-name')?.textContent, 'bob');
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

async function composerFixture(context: TestContext, extra: Record<string, (...args: any[]) => unknown> = {}, mobile = false) {
  const fixture = await compiledHub();
  const { rpc } = roomFixture();
  const app = await fixture.mount(context, {
    props: { visible: true, mobile },
    setup(window) { window.Element.prototype.getAnimations = () => []; },
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
  await wait(() => app.document.querySelector('.to-name')?.textContent === 'alice');
  const input = app.document.querySelector<HTMLTextAreaElement>('.c-input')!;
  const send = app.document.querySelector<HTMLButtonElement>('.send-btn')!;
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
    app.document.querySelector<HTMLElement>('.to-chip')!.click();
    await app.flush();
    const button = [...app.document.querySelectorAll<HTMLButtonElement>('.to-menu button')]
      .find((element) => element.textContent?.trim() === name);
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
    assert.equal(app.document.querySelector('.to-name')?.textContent, 'note', 'an explicit room recipient persists');
  } finally { await app.close(); }
});

test('Composer interrupt mixes button and Ctrl+C, expires, disarms and respects destination', { timeout: 60000 }, async (context) => {
  const interrupts: Array<[string, string]> = [];
  const app = await composerFixture(context, {
    hubAgentInterrupt: async (session: string, name: string) => { interrupts.push([session, name]); return {}; },
  }, true);
  const armed = () => !!app.document.querySelector('.int-pill');
  try {
    assert.equal(await app.key('Enter'), false, 'compact Enter remains a newline');
    app.send.click();
    await app.flush();
    assert.equal(armed(), true);
    await app.advance(2999);
    assert.equal(armed(), true);
    await app.advance(1);
    assert.equal(armed(), false);
    await app.key('c', { ctrlKey: true });
    await app.key('Escape');
    assert.equal(armed(), false);
    app.send.click();
    await app.text('copy me');
    assert.equal(armed(), false);
    assert.equal(await app.key('c', { ctrlKey: true }), false, 'nonempty Ctrl+C is native copy');
    await app.text('');
    app.send.click();
    await app.flush();
    await app.to('bob');
    assert.equal(armed(), false, 'changing recipient disarms synchronously');
    app.send.click();
    await app.flush();
    await app.room('other');
    assert.equal(armed(), false);
    app.send.click();
    await app.flush();
    await app.key('c', { ctrlKey: true });
    await app.wait(() => interrupts.length === 1);
    assert.deepEqual(interrupts, [['other', 'alice']]);
    await app.to('everyone');
    await app.key('c', { ctrlKey: true });
    app.send.click();
    await app.wait(() => interrupts.length === 3);
    assert.deepEqual(interrupts.slice(1), [['other', 'alice'], ['other', 'bob']]);
    await app.to('note');
    assert.equal(app.send.disabled, true);
    await app.key('c', { ctrlKey: true });
    assert.equal(armed(), false);
    assert.equal(interrupts.length, 3);
  } finally { await app.close(); }
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
    actions()!.querySelector<HTMLElement>('button:last-child')!.click();
    await app.flush();
    assert.equal(reply().querySelector('.raw')?.textContent, body);
    app.document.querySelector<HTMLElement>('.to-chip')!.click();
    await app.flush();
    app.window.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await app.flush();
    assert.equal(actions(), null);
    assert.equal(app.document.querySelector('.to-menu'), null, 'one capture callback closes both existing territories');
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
    for (let i = 0; i < 10 && !app.document.querySelector('.to-name'); i++) await app.flush();
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
