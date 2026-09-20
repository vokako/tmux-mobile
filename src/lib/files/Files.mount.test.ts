import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./Files.test.svelte', import.meta.url), [new URL('../core/ws.ts', import.meta.url)]);
const entries = [
  { name: 'AGENTS.md', path: '/fixture/AGENTS.md', type: 'file', size: 1200 },
  { name: 'next.md', path: '/fixture/next.md', type: 'file', size: 100 },
];
function rpc(extra: Record<string, (...args: any[]) => unknown> = {}) {
  return {
    fsCwd: async () => ({ path: '/fixture' }),
    fsList: async () => ({ path: '/fixture', entries }),
    fsStat: async (path: string) => ({ path, is_text: true, writable: true, readable: true, size: 100, mime_hint: 'text/markdown' }),
    fsRead: async (path: string) => ({ content: path.endsWith('AGENTS.md') ? '# Rules\n\n[Next](next.md)' : '# Next' }),
    getBookmarks: async () => ({ bookmarks: [] }), getPrefs: async () => ({}),
    saveBookmarks: async () => ({}), setPref: async () => ({}),
    gitCmd: async () => ({ code: 0, stdout: '.git' }),
    ...extra,
  };
}
type App = Awaited<ReturnType<Awaited<typeof compiled>['mount']>>;
async function settle(app: App) { for (let i = 0; i < 8; i++) await app.flush(); }
function button(app: App, label: string) {
  const element = app.document.querySelector<HTMLButtonElement>(`button[aria-label="${label}"]`);
  assert.ok(element, label); return element;
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

test('rejected Files delete retains its dialog/error and retries the captured target (#167)', async context => {
  const first = deferred<object>();
  const deletes: string[] = [];
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' },
    modules: [rpc({ fsDelete: (path: string) => {
      deletes.push(path); return deletes.length === 1 ? first.promise : Promise.resolve({});
    } })],
  });
  try {
    await settle(app);
    button(app, 'Delete: next.md').click(); await app.flush();
    button(app, 'Delete').click(); await app.flush();
    first.reject(new Error('permission denied')); await settle(app);
    const dialog = app.document.querySelector('[role=alertdialog]');
    assert.ok(dialog, 'a rejected delete must not close its confirmation');
    assert.match(dialog.querySelector('[role=alert]')?.textContent ?? '', /permission denied/);
    assert.match(dialog.textContent ?? '', /next\.md/);
    assert.equal(button(app, 'Delete').disabled, false);
    button(app, 'Delete').click(); await settle(app);
    assert.deepEqual(deletes, ['/fixture/next.md', '/fixture/next.md']);
    assert.equal(app.document.querySelector('[role=alertdialog]'), null);
  } finally { first.resolve({}); await app.close(); }
});

test('Files delete is single-flight before paint and pending Cancel/Escape/backdrop/Back are consumed (#167)', async context => {
  const pending = deferred<object>();
  let calls = 0;
  let back!: () => boolean;
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture', onGoBack: (fn: typeof back) => { back = fn; } },
    modules: [rpc({ fsDelete: () => { calls++; return pending.promise; } })],
  });
  try {
    await settle(app);
    button(app, 'Delete: next.md').click(); await app.flush();
    const confirm = button(app, 'Delete');
    const cancel = button(app, 'Cancel');
    confirm.click(); confirm.click();
    cancel.click();
    app.document.querySelector<HTMLElement>('.dlg-backdrop')!.click();
    assert.equal(back(), true);
    await app.flush();
    assert.equal(calls, 1, 'executor guards before disabled reaches the DOM');
    const escape = new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true });
    app.window.dispatchEvent(escape);
    assert.equal(escape.defaultPrevented, true);
    assert.ok(app.document.querySelector('[role=alertdialog]'), 'pending Back cannot dismiss or navigate');
    assert.equal(button(app, 'Cancel').disabled, true);
    assert.equal(app.document.querySelector('.bc-scroll')?.textContent?.includes('fixture'), true);
    pending.resolve({}); await settle(app);
    assert.equal(app.document.querySelector('[role=alertdialog]'), null);
  } finally { pending.resolve({}); await app.close(); }
});

test('Files mutation success closes confirmation even when listing refresh fails (#167)', async context => {
  let deleted = false;
  let calls = 0;
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' },
    modules: [rpc({
      fsDelete: async () => { deleted = true; calls++; return {}; },
      fsList: async () => { if (deleted) throw new Error('refresh unavailable'); return { path: '/fixture', entries }; },
    })],
  });
  try {
    await settle(app);
    button(app, 'Delete: next.md').click(); await app.flush();
    button(app, 'Delete').click(); await settle(app);
    assert.equal(app.document.querySelector('[role=alertdialog]'), null, 'a refresh failure cannot offer another delete');
    assert.match(app.document.querySelector('.error')?.textContent ?? '', /refresh unavailable/);
    assert.equal(app.document.querySelector('[aria-label="Delete: next.md"]'), null, 'the confirmed-deleted row cannot invite a retry');
    assert.equal(calls, 1);
  } finally { await app.close(); }
});

for (const outcome of ['success', 'failure'] as const) {
  test(`stale Files delete ${outcome} cannot replace a newer session/view/dialog (#167)`, async context => {
    const pending = deferred<object>();
    const deletes: string[] = [];
    const listed: string[] = [];
    let update!: (next: Record<string, unknown>) => void;
    const app = await (await compiled).mount(context, {
      props: { visible: true, session: 'fixture', register: (fn: typeof update) => { update = fn; } },
      modules: [rpc({
        fsDelete: (path: string) => { deletes.push(path); return pending.promise; },
        fsList: async (path: string) => { listed.push(path); return { path, entries: path === '/fixture' ? entries : [
          { name: 'new.md', path: '/new/new.md', type: 'file', size: 100 },
        ] }; },
      })],
    });
    try {
      await settle(app);
      button(app, 'Delete: next.md').click(); await app.flush();
      button(app, 'Delete').click(); await app.flush();
      update({ session: 'new', root: '/new', navRequest: { path: '/new', n: 1 } }); await settle(app);
      assert.equal(app.document.querySelector('[role=alertdialog]'), null, 'old confirmation belongs to its old view');
      button(app, 'Delete: new.md').click(); await app.flush();
      const before = listed.length;
      if (outcome === 'success') pending.resolve({}); else pending.reject(new Error('old failure'));
      await settle(app);
      assert.match(app.document.querySelector('[role=alertdialog]')?.textContent ?? '', /new\.md/);
      assert.doesNotMatch(app.document.body.textContent ?? '', /old failure/);
      assert.equal(listed.length, before, 'old completion cannot refresh the new listing');
      assert.deepEqual(deletes, ['/fixture/next.md']);
    } finally { pending.resolve({}); await app.close(); }
  });
}

test('every Files toolbar action has a localized accessible name (#157)', async context => {
  const app = await (await compiled).mount(context, { props: { visible: true, session: 'fixture' }, modules: [rpc()] });
  try {
    await settle(app);
    const buttons = [...app.document.querySelectorAll<HTMLButtonElement>('.toolbar button')];
    assert.equal(buttons.length, 9);
    assert.ok(buttons.every(b => !!b.getAttribute('aria-label')), 'no unnamed tool');
    for (const label of ['Session directory', 'Refresh', 'New item', 'Upload files', 'Show hidden files', 'Bookmark directory', 'Bookmarks', 'Recent files', 'Git']) {
      button(app, label);
    }
  } finally { await app.close(); }
});

test('Files toggle/disclosure tools announce state, including empty panels (#157)', async context => {
  const hidden: boolean[] = [];
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' },
    modules: [rpc({ fsList: async (_path: string, show: boolean) => { hidden.push(show); return { path: '/fixture', entries }; } })],
  });
  try {
    await settle(app);
    button(app, 'Show hidden files').click(); await settle(app);
    assert.equal(button(app, 'Show hidden files').getAttribute('aria-pressed'), 'true');
    assert.equal(hidden.at(-1), true);
    button(app, 'Bookmarks').click(); await app.flush();
    const bookmarks = button(app, 'Bookmarks');
    assert.equal(bookmarks.getAttribute('aria-expanded'), 'true');
    assert.match(app.document.getElementById(bookmarks.getAttribute('aria-controls')!)?.textContent ?? '', /No bookmarks/);
    button(app, 'Recent files').click(); await app.flush();
    assert.equal(bookmarks.getAttribute('aria-expanded'), 'false');
    assert.match(app.document.querySelector('.bookmarks-panel')?.textContent ?? '', /No recent files/);
  } finally { await app.close(); }
});

test('Files shared Back keeps the existing linked-preview history (#157)', async context => {
  const app = await (await compiled).mount(context, { props: { visible: true, session: 'fixture' }, modules: [rpc()] });
  try {
    await settle(app);
    app.document.querySelector<HTMLButtonElement>('.file-main')!.click(); await settle(app);
    assert.equal(app.document.querySelector('.preview-name')?.textContent, 'AGENTS.md');
    app.document.querySelector<HTMLAnchorElement>('.md-render a')!.dispatchEvent(new app.window.MouseEvent('click', { bubbles: true, cancelable: true }));
    await settle(app);
    assert.equal(app.document.querySelector('.preview-name')?.textContent, 'next.md');
    button(app, 'Back').click(); await settle(app);
    assert.equal(app.document.querySelector('.preview-name')?.textContent, 'AGENTS.md');
    button(app, 'Back').click(); await settle(app);
    assert.equal(app.document.querySelector('.preview-header'), null);
    assert.ok(app.document.querySelector('.file-list'));
  } finally { await app.close(); }
});

test('a failed bookmark read is not presented as an empty collection (#157)', async context => {
  let reject!: (reason: Error) => void;
  let calls = 0;
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' },
    modules: [rpc({ getBookmarks: () => ++calls === 1 ? new Promise((_, no) => reject = no) : Promise.resolve({ bookmarks: [] }) })],
  });
  try {
    await settle(app);
    button(app, 'Bookmarks').click(); await app.flush();
    assert.match(app.document.querySelector('.bookmarks-panel')?.textContent ?? '', /Loading/);
    reject(new Error('Read failed')); await settle(app);
    assert.match(app.document.querySelector('[role=alert]')?.textContent ?? '', /Read failed/);
    assert.doesNotMatch(app.document.querySelector('.bookmarks-panel')?.textContent ?? '', /No bookmarks/);
    app.document.querySelector<HTMLButtonElement>('.bookmarks-panel [aria-label="Refresh"]')!.click(); await settle(app);
    assert.match(app.document.querySelector('.bookmarks-panel')?.textContent ?? '', /No bookmarks/);
  } finally { await app.close(); }
});

test('the File/Folder choice reaches the existing create path and preserves its IME guard (#157)', async context => {
  const writes: string[] = [];
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' },
    modules: [rpc({ fsMkdir: async (path: string) => { writes.push(path); return {}; } })],
  });
  try {
    await settle(app); button(app, 'New item').click(); await app.flush();
    [...app.document.querySelectorAll<HTMLButtonElement>('.segmented button')].find(b => b.textContent === 'Folder')!.click();
    const input = app.document.querySelector<HTMLInputElement>('.new-item input')!;
    input.value = 'fresh';
    input.dispatchEvent(new app.window.Event('input', { bubbles: true })); await app.flush();
    input.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Enter', isComposing: true, bubbles: true }));
    assert.deepEqual(writes, []);
    input.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Enter', bubbles: true }));
    await settle(app);
    assert.deepEqual(writes, ['/fixture/fresh']);
    assert.equal(app.document.querySelector('.new-item'), null);
  } finally { await app.close(); }
});

test('a row action retains its row path, not the open preview path (#157)', async context => {
  const renames: string[][] = [];
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' },
    modules: [rpc({ fsRename: async (from: string, to: string) => { renames.push([from, to]); return {}; } })],
    setup(window) { window.localStorage.setItem('tmux_layout_mode', 'desktop'); },
  });
  try {
    await settle(app); app.document.querySelector<HTMLButtonElement>('.file-main')!.click(); await settle(app);
    button(app, 'Rename: next.md').click(); await app.flush();
    const input = app.document.querySelector<HTMLInputElement>('.new-item input')!;
    input.value = 'renamed.md';
    input.dispatchEvent(new app.window.Event('input', { bubbles: true })); await app.flush();
    button(app, 'Rename').click(); await settle(app);
    assert.deepEqual(renames, [['/fixture/next.md', '/fixture/renamed.md']]);
  } finally { await app.close(); }
});

function contextMenu(app: App, element: Element) {
  element.dispatchEvent(new app.window.MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 30, clientY: 50 }));
}
function menuAction(app: App, label: string) {
  const element = [...app.document.querySelectorAll<HTMLButtonElement>('.ctx button')].find(b => b.textContent?.trim() === label);
  assert.ok(element, `context action: ${label}`); return element;
}

async function copyRow(app: App, index = 0) {
  contextMenu(app, app.document.querySelectorAll('.file-row')[index]!);
  await app.flush();
  menuAction(app, 'Copy path').click();
  await settle(app);
}
const copySuccess = (app: App) => [...app.document.querySelectorAll('.copy-toast, [role=status]')]
  .some(node => node.textContent?.trim() === 'Copied');

for (const copied of [true, false]) {
  test(`Files copy ${copied ? 'success has no actions' : 'error has a working Close'} (#167 review)`, async context => {
    const app = await (await compiled).mount(context, {
      props: { visible: true, session: 'fixture' }, modules: [rpc()],
      setup(window) {
        window.document.execCommand = () => false;
        Object.defineProperty(window.navigator, 'clipboard', { value: {
          writeText: async () => { if (!copied) throw Error('denied'); },
        } });
      },
    });
    try {
      await settle(app); await copyRow(app);
      const feedback = app.document.querySelector('.operation-feedback')!;
      assert.ok(feedback);
      assert.equal(feedback.querySelectorAll('.feedback-actions').length, copied ? 0 : 1);
      if (!copied) {
        button(app, 'Close').click(); await settle(app);
        assert.equal(app.document.querySelector('.operation-feedback'), null);
      }
    } finally { await app.close(); }
  });
}

test('Files download progress has no dismissal/action slot; failure has Close (#167 review)', async context => {
  const pending = deferred<object>();
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' },
    modules: [rpc({ fsDownloadHttp: () => pending.promise })],
  });
  try {
    await settle(app); button(app, 'Download: AGENTS.md').click(); await settle(app);
    assert.equal(app.document.querySelector('.operation-feedback .feedback-actions'), null,
      'hiding progress is not cancellation and must not be offered');
    pending.reject(Error('download failed')); await settle(app);
    const feedback = app.document.querySelector('.operation-feedback');
    assert.match(feedback?.textContent ?? '', /download failed/);
    assert.equal(feedback?.querySelectorAll('button').length, 1, 'no empty Open action');
    button(app, 'Close').click(); await settle(app);
    assert.equal(app.document.querySelector('.operation-feedback'), null);
  } finally { pending.reject(Error('fixture closed')); await app.close(); }
});

test('old copy completion cannot replace a newer failed copy (#167)', async context => {
  const old = deferred<void>();
  let calls = 0;
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' }, modules: [rpc()],
    setup(window) {
      window.document.execCommand = () => false;
      Object.defineProperty(window.navigator, 'clipboard', { value: {
        writeText: () => ++calls === 1 ? old.promise : Promise.reject(Error('denied')),
      } });
    },
  });
  try {
    await settle(app);
    await copyRow(app);
    await copyRow(app, 1);
    old.resolve(); await settle(app);
    assert.equal(copySuccess(app), false, 'A success must not replace B failure');
    assert.match(app.document.body.textContent ?? '', /Copy failed/);
    await app.advance(10000);
    assert.match(app.document.body.textContent ?? '', /Copy failed/, 'errors persist');
  } finally { old.resolve(); await app.close(); }
});

test('copy success expires at 1500ms and its old expiry cannot clear a newer completion (#167)', async context => {
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' }, modules: [rpc()],
    setup(window) {
      Object.defineProperty(window.navigator, 'clipboard', { value: { writeText: async () => {} } });
    },
  });
  try {
    await settle(app);
    await copyRow(app);
    await app.advance(1000);
    await copyRow(app, 1);
    await app.advance(500);
    assert.equal(copySuccess(app), true, 'A expiry leaves B visible');
    await app.advance(999);
    assert.equal(copySuccess(app), true, 'B retains the full completion lifetime');
    await app.advance(1);
    assert.equal(copySuccess(app), false);
  } finally { await app.close(); }
});

for (const outcome of ['success', 'failure'] as const) {
  test(`copy ${outcome} from a hidden Files context never reappears (#167)`, async context => {
    const old = deferred<void>();
    let update!: (next: Record<string, unknown>) => void;
    const app = await (await compiled).mount(context, {
      props: { visible: true, session: 'fixture', register: (fn: typeof update) => { update = fn; } },
      modules: [rpc()],
      setup(window) {
        window.document.execCommand = () => false;
        Object.defineProperty(window.navigator, 'clipboard', { value: { writeText: () => old.promise } });
      },
    });
    try {
      await settle(app); await copyRow(app);
      update({ visible: false }); await settle(app);
      update({ visible: true }); await settle(app);
      if (outcome === 'success') old.resolve(); else old.reject(Error('denied'));
      await settle(app);
      assert.equal(copySuccess(app), false);
      assert.doesNotMatch(app.document.body.textContent ?? '', /Copy failed/);
    } finally { old.resolve(); await app.close(); }
  });
}

test('measured overflow retains the same trailing disclosure action and controlled state (#164)', async context => {
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' }, modules: [rpc()],
    setup(window) {
      // Synthetic layout outputs exercise wiring only; Chromium owns geometry.
      Object.defineProperty(window.HTMLElement.prototype, 'clientWidth', {
        configurable: true, get() { return this.classList.contains('toolbar') ? 390 : 0; },
      });
      const computed = window.getComputedStyle.bind(window);
      window.getComputedStyle = element => {
        const style = computed(element);
        return element.classList.contains('toolbar') ? new Proxy(style, {
          get(target, key) {
            if (key === 'paddingLeft' || key === 'paddingRight') return '6px';
            if (key === 'columnGap') return '2px';
            if (key === 'getPropertyValue') return (name: string) => name === '--control-height' ? '44px' : target.getPropertyValue(name);
            return Reflect.get(target, key);
          },
        }) : style;
      };
    },
  });
  try {
    await settle(app);
    const more = button(app, 'More file actions');
    assert.equal(app.document.querySelector('.toolbar [aria-label="Recent files"]'), null);
    more.click(); await app.flush();
    assert.ok(app.document.getElementById(more.getAttribute('aria-controls')!));
    assert.equal(menuAction(app, 'Recent files').getAttribute('aria-checked'), 'false');
    menuAction(app, 'Recent files').click(); await app.flush();
    assert.match(app.document.querySelector('.bookmarks-panel')?.textContent ?? '', /No recent files/);
    more.click(); await app.flush();
    assert.equal(menuAction(app, 'Recent files').getAttribute('aria-checked'), 'true');
  } finally { await app.close(); }
});

test('row context actions use the clicked file even beside another preview (#164)', async context => {
  const renames: string[][] = [];
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' },
    modules: [rpc({ fsRename: async (from: string, to: string) => { renames.push([from, to]); return {}; } })],
    setup(window) { window.localStorage.setItem('tmux_layout_mode', 'desktop'); },
  });
  try {
    await settle(app);
    app.document.querySelector<HTMLButtonElement>('.file-main')!.click(); await settle(app);
    contextMenu(app, app.document.querySelectorAll('.file-row')[1]!); await app.flush();
    assert.equal(app.document.querySelector('.ctx-who')?.textContent, 'next.md');
    menuAction(app, 'Rename').click(); await app.flush();
    const input = app.document.querySelector<HTMLInputElement>('.new-item input')!;
    input.value = 'renamed.md'; input.dispatchEvent(new app.window.Event('input', { bubbles: true }));
    await app.flush(); button(app, 'Rename').click(); await settle(app);
    assert.deepEqual(renames, [['/fixture/next.md', '/fixture/renamed.md']]);
  } finally { await app.close(); }
});

test('context Delete still requires confirmation and a stale menu cannot act after Back (#164)', async context => {
  const deletes: string[] = [];
  let back!: () => boolean;
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture', onGoBack: (fn: () => boolean) => back = fn },
    modules: [rpc({ fsDelete: async (path: string) => { deletes.push(path); return {}; } })],
  });
  try {
    await settle(app);
    const origin = app.document.querySelectorAll<HTMLButtonElement>('.file-main')[1]!;
    origin.focus();
    contextMenu(app, app.document.querySelectorAll('.file-row')[1]!); await app.flush();
    const oldDelete = menuAction(app, 'Delete');
    assert.equal(back(), true); await app.flush();
    // Reattach the old node so Svelte's delegated handler actually executes.
    // A click on a detached node would be a false-positive stale-action test.
    app.document.querySelector('.files')!.append(oldDelete);
    oldDelete.dispatchEvent(new app.window.MouseEvent('click', { bubbles: true }));
    await app.flush();
    assert.equal(app.document.querySelector('[role=alertdialog]'), null);
    oldDelete.remove();
    contextMenu(app, app.document.querySelectorAll('.file-row')[1]!); await app.flush();
    menuAction(app, 'Delete').click(); await app.flush();
    assert.deepEqual(deletes, []);
    assert.match(app.document.querySelector('[role=alertdialog]')?.textContent ?? '', /next\.md/);
    button(app, 'Cancel').click(); await app.flush();
    assert.deepEqual(deletes, []);
    assert.equal(app.document.activeElement, origin, 'Cancel returns past the dismissed menu to the row');
  } finally { await app.close(); }
});

test('blank-directory context reuses toolbar modes and native preview selection is untouched (#164)', async context => {
  const hidden: boolean[] = [];
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' },
    modules: [rpc({ fsList: async (_path: string, show: boolean) => { hidden.push(show); return { path: '/fixture', entries }; } })],
  });
  try {
    await settle(app);
    contextMenu(app, app.document.querySelector('.file-list')!); await app.flush();
    const toggle = menuAction(app, 'Show hidden files');
    assert.equal(toggle.getAttribute('aria-checked'), 'false');
    toggle.click(); await settle(app);
    assert.equal(hidden.at(-1), true);
    assert.equal(button(app, 'Show hidden files').getAttribute('aria-pressed'), 'true');
    const rowHold = new app.window.MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    Object.defineProperty(rowHold, 'pointerType', { value: 'touch' });
    app.document.querySelector('.file-row')!.dispatchEvent(rowHold);
    assert.equal(rowHold.defaultPrevented, true, 'the list hold is the shared app gesture, not browser chrome');
    app.document.querySelector<HTMLButtonElement>('.file-main')!.click(); await settle(app);
    const event = new app.window.MouseEvent('contextmenu', { bubbles: true, cancelable: true });
    Object.defineProperty(event, 'pointerType', { value: 'touch' });
    app.document.querySelector('.md-render')!.dispatchEvent(event);
    assert.equal(event.defaultPrevented, false);
    assert.equal(app.document.querySelector('.ctx'), null);
  } finally { await app.close(); }
});

test('Copy path cannot report success when both clipboard paths fail (#164)', async context => {
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' }, modules: [rpc()],
    setup(window) {
      window.document.execCommand = () => false;
      Object.defineProperty(window.navigator, 'clipboard', { value: { writeText: async () => { throw Error('denied'); } } });
    },
  });
  try {
    await settle(app);
    contextMenu(app, app.document.querySelector('.file-row')!); await app.flush();
    menuAction(app, 'Copy path').click(); await settle(app);
    assert.equal(copySuccess(app), false);
    assert.match(app.document.body.textContent ?? '', /Copy failed/);
  } finally { await app.close(); }
});

test('without a declared root Files follows the pane cwd; with one it starts there and never asks fs_cwd (board #181)', async context => {
  const fixture = await compiled;
  const listed: string[] = [];
  let cwdAsked = 0;
  const modules = () => [rpc({
    fsCwd: async () => { cwdAsked++; return { path: '/pane' }; },
    fsList: async (path: string) => { listed.push(path); return { path, entries: [] }; },
  })];
  // No project declaration (a direct/adopted session): the pane cwd is the fallback.
  const loose = await fixture.mount(context, { props: { visible: true, session: 'loose' }, modules: modules() });
  try {
    await settle(loose);
    assert.equal(listed[0], '/pane');
    assert.ok(cwdAsked >= 1);
  } finally { await loose.close(); }
  listed.length = 0; cwdAsked = 0;
  // A project: its declared path is the start, and fs_cwd is not consulted.
  const declared = await fixture.mount(context, { props: { visible: true, session: 'proj', root: '/declared' }, modules: modules() });
  try {
    await settle(declared);
    assert.equal(listed[0], '/declared');
    assert.equal(cwdAsked, 0, 'the declaration is the truth; the pane is not asked');
  } finally { await declared.close(); }
});

test('a video previews as a stream: one stream-signed /dl URL, no bytes through the RPC, no size gate (board #182)', async context => {
  // Owner 2026-09-12: "文件的预览里边，应该加入视频的流式播放的预览能力". A 2 GB
  // film is far over PREVIEW_SIZE_LIMIT and MAX_READ_SIZE; it must never be
  // read — the <video> element streams ranges from /dl itself.
  const signed: unknown[][] = [];
  const bytes: string[] = [];
  const videoEntries = [{ name: 'demo.mp4', path: '/fixture/demo.mp4', type: 'file', size: 2 * 1024 ** 3 }];
  const app = await (await compiled).mount(context, { props: { visible: true, session: 'fixture' }, modules: [rpc({
    fsList: async () => ({ path: '/fixture', entries: videoEntries }),
    fsStat: async (path: string) => ({ path, is_text: false, writable: true, readable: true, size: 2 * 1024 ** 3, mime_hint: 'video/mp4' }),
    fsDownloadHttp: async (...args: unknown[]) => { signed.push(args); return { url: 'https://h/dl?path=%2Ffixture%2Fdemo.mp4&exp=9&sig=s&stream=1', name: 'demo.mp4' }; },
    fsDownload: async (path: string) => { bytes.push(path); return { data: '' }; },
    fsRead: async (path: string) => { bytes.push(path); return { content: '' }; },
  })] });
  try {
    await settle(app);
    app.document.querySelector<HTMLButtonElement>('.file-row .file-main')!.click();
    await settle(app);
    const video = app.document.querySelector<HTMLVideoElement>('.preview-body video, .media-preview video');
    assert.ok(video, 'the preview is a <video>, not the info page');
    assert.equal(video.getAttribute('src'), 'https://h/dl?path=%2Ffixture%2Fdemo.mp4&exp=9&sig=s&stream=1');
    assert.equal(JSON.stringify(signed), JSON.stringify([['/fixture/demo.mp4', { stream: true }]]), 'signed once, for a stream lifetime');
    assert.deepEqual(bytes, [], 'no fs_download / fs_read for a stream');
  } finally { await app.close(); }
});

test('an image preview opens the one Lightbox; Back closes it; a trackpad pinch zooms the image, not the page (board #188)', async context => {
  // Owner 2026-09-12: "预览图片的时候，要能够点击图片全屏放大，最好图片这种增加在图片上的
  // 触摸板两指放大手势，不是把整个页面放大".
  const chain: { back: (() => boolean) | null } = { back: null };
  const app = await (await compiled).mount(context, { props: { visible: true, session: 'fixture', onGoBack: (fn: () => boolean) => { chain.back = fn; } }, modules: [rpc({
    fsList: async () => ({ path: '/fixture', entries: [{ name: 'shot.png', path: '/fixture/shot.png', type: 'file', size: 10 }] }),
    fsStat: async (path: string) => ({ path, is_text: false, writable: true, readable: true, size: 10, mime_hint: 'image/png' }),
    fsDownload: async () => ({ data: 'eA==' }),
  })] });
  try {
    await settle(app);
    app.document.querySelector<HTMLButtonElement>('.file-row .file-main')!.click();
    await settle(app);
    const open = app.document.querySelector<HTMLButtonElement>('.image-open')!;
    assert.ok(open, 'the picture is a button');
    assert.equal(app.document.querySelector('.lb'), null);
    open.click(); await settle(app);
    const lb = app.document.querySelector<HTMLImageElement>('.lb .lb-img');
    assert.ok(lb, 'the Lightbox opened');
    assert.equal(lb.getAttribute('src'), 'data:image/png;base64,eA==');
    assert.ok(chain.back, 'Files registered its Back chain');
    assert.equal(chain.back!(), true, 'Back closes the viewer first');
    await settle(app);
    assert.equal(app.document.querySelector('.lb'), null);
    assert.ok(app.document.querySelector('.image-open'), 'and the preview is still there');
    // A trackpad pinch arrives as ctrl+wheel: the image takes it, the page does not zoom.
    const pinch = new app.window.WheelEvent('wheel', { deltaY: -40, ctrlKey: true, bubbles: true, cancelable: true });
    app.document.querySelector('.image-open')!.dispatchEvent(pinch);
    await settle(app);
    assert.equal(pinch.defaultPrevented, true, 'the page keeps its zoom');
    assert.ok(app.document.querySelector('.lb'), 'the viewer takes the gesture');
  } finally { await app.close(); }
});

test('Back/Forward: the browser pair heads the path row, disabled at the ends (board #187)', async context => {
  // Owner 2026-09-12: "文件夹浏览的能不能加一个类似浏览器后退前进的按钮，方便我跳转位置后快速回来".
  const listed: string[] = [];
  const app = await (await compiled).mount(context, { props: { visible: true, session: 'fixture' }, modules: [rpc({
    fsList: async (path: string) => { listed.push(path); return { path, entries: path === '/fixture'
      ? [{ name: 'docs', path: '/fixture/docs', type: 'dir', size: 0 }] : [] }; },
  })] });
  try {
    await settle(app);
    const back = () => button(app, 'Back'), fwd = () => button(app, 'Forward');
    assert.ok(back().closest('.bc-path-row') && fwd().closest('.bc-path-row'), 'the pair lives at the head of the path row, not in the tools bar (#164 overflow)');
    assert.equal(app.document.querySelectorAll('.toolbar button').length, 9, 'the tools bar is untouched');
    assert.equal(back().disabled, true, 'nothing behind at the entry point');
    assert.equal(fwd().disabled, true, 'nothing ahead');
    app.document.querySelector<HTMLButtonElement>('.file-row .file-main')!.click();
    await settle(app);
    assert.equal(listed.at(-1), '/fixture/docs');
    assert.equal(back().disabled, false, 'a step to retrace');
    assert.equal(fwd().disabled, true);
    back().click(); await settle(app);
    assert.equal(listed.at(-1), '/fixture', 'Back retraces the step');
    assert.equal(back().disabled, true);
    assert.equal(fwd().disabled, false, 'the place we left is ahead');
    fwd().click(); await settle(app);
    assert.equal(listed.at(-1), '/fixture/docs', 'Forward returns there');
    assert.equal(fwd().disabled, true);
    assert.equal(back().disabled, false);
  } finally { await app.close(); }
});

test('a path segment has the one context menu (Copy path) and a text selection does not navigate (board #187)', async context => {
  // Owner 2026-09-12: "这个路径最好可以复制，包括文件夹文件的名字我也可以选中复制".
  const copied: string[] = [];
  const listed: string[] = [];
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' },
    modules: [rpc({ fsList: async (path: string) => { listed.push(path); return { path, entries }; } })],
    setup(window) {
      window.localStorage.setItem('tmux_layout_mode', 'desktop');
      Object.defineProperty(window.navigator, 'clipboard', { value: { writeText: async (text: string) => { copied.push(text); } } });
    },
  });
  try {
    await settle(app);
    const segs = app.document.querySelectorAll<HTMLButtonElement>('.bc-path-row .bc-seg');
    assert.equal(segs.length, 2, 'root + fixture');
    // Right-click the "fixture" crumb: the same context-menu mechanism as a row, offering Copy path of THAT segment.
    contextMenu(app, segs[1]!); await app.flush();
    assert.equal(app.document.querySelector('.ctx-who')?.textContent, 'fixture');
    menuAction(app, 'Copy path').click(); await settle(app);
    assert.deepEqual(copied, ['/fixture']);
    assert.equal(app.document.querySelectorAll('.ctx-item, .menu-item').length, 0, 'the menu closed');
    // A tap that ends a text selection is a copy gesture, not a navigation.
    const before = listed.length;
    const name = app.document.querySelector('.file-row .file-name')!;
    const range = app.document.createRange(); range.selectNodeContents(name);
    const sel = app.window.getSelection()!; sel.removeAllRanges(); sel.addRange(range);
    assert.equal(sel.isCollapsed, false);
    app.document.querySelector<HTMLButtonElement>('.file-row .file-main')!.click(); await settle(app);
    assert.equal(app.document.querySelector('.preview-header'), null, 'no preview opened over a selection');
    segs[1]!.click(); await settle(app);
    assert.equal(listed.length, before, 'no directory navigation over a selection');
    sel.removeAllRanges();
    segs[1]!.click(); await settle(app);
    assert.equal(listed.length, before + 1, 'without a selection the crumb navigates');
  } finally { await app.close(); }
});

test('crumb and row menus copy the NAME or the full PATH (board #191)', async context => {
  const copied: string[] = [];
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' }, modules: [rpc()],
    setup(window) {
      window.localStorage.setItem('tmux_layout_mode', 'desktop');
      Object.defineProperty(window.navigator, 'clipboard', { value: { writeText: async (text: string) => { copied.push(text); } } });
    },
  });
  try {
    await settle(app);
    const crumb = app.document.querySelectorAll<HTMLButtonElement>('.bc-path-row .bc-seg')[1]!;
    contextMenu(app, crumb); await app.flush();
    menuAction(app, 'Copy name').click(); await settle(app);
    contextMenu(app, crumb); await app.flush();
    menuAction(app, 'Copy path').click(); await settle(app);
    contextMenu(app, app.document.querySelector('.file-row')!); await app.flush();
    menuAction(app, 'Copy name').click(); await settle(app);
    assert.deepEqual(copied, ['fixture', '/fixture', 'AGENTS.md']);
  } finally { await app.close(); }
});


test('an upload reports each file, counts it uploaded only when the server answers, and names failures (board #214)', async context => {
  // Owner 2026-09-20: "文件上传要有个进度或者提示，让我知道传上去了没有". Before, a
  // successful upload only refreshed the listing; the user could not tell.
  const uploads: { path: string; done: ReturnType<typeof deferred<object>> }[] = [];
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' },
    modules: [rpc({ fsUpload: (path: string) => { const done = deferred<object>(); uploads.push({ path, done }); return done.promise; } })],
    setup(window) {
      window.localStorage.setItem('tmux_layout_mode', 'desktop');
      // A file input cannot be driven in a test; the batch is handed to the
      // picker's onchange with the files defined on it.
      const click = window.HTMLInputElement.prototype.click;
      window.HTMLInputElement.prototype.click = function (this: HTMLInputElement) {
        if (this.type !== 'file') return click.call(this);
        const files = [new window.File(['abc'], 'one.txt'), new window.File(['defg'], 'two.txt')];
        Object.defineProperty(this, 'files', { value: files });
        this.onchange?.(new window.Event('change'));
      };
    },
  });
  const feedback = () => app.document.querySelector('.files-feedback [role=status], .files-feedback [role=alert]')?.textContent ?? '';
  try {
    await settle(app);
    button(app, 'Upload files').click();
    // jsdom's FileReader completes over setImmediate hops (a macrotask each).
    const macrotask = () => new Promise<void>((resolve) => setImmediate(resolve));
    for (let i = 0; i < 20 && uploads.length < 1; i++) { await macrotask(); await app.flush(); }
    assert.equal(uploads.length, 1, 'files go one at a time');
    assert.equal(uploads[0]!.path, '/fixture/one.txt');
    assert.match(feedback(), /Uploading one\.txt \(1\/2\)/);
    assert.match(feedback(), /Sending/, 'the read is done; the send is a discrete beat');
    assert.equal(app.document.querySelector('.files-feedback progress'), null, 'no invented percentage while sending');
    uploads[0]!.done.resolve({});
    for (let i = 0; i < 20 && uploads.length < 2; i++) { await macrotask(); await app.flush(); }
    assert.match(feedback(), /Uploading two\.txt \(2\/2\)/, 'the first is counted only after the server answered');
    uploads[1]!.done.reject(new Error('disk full'));
    await settle(app);
    const alert = app.document.querySelector('.files-feedback [role=alert]');
    assert.ok(alert, 'a failure is an alert, not a silent refresh');
    assert.match(alert.textContent ?? '', /1 uploaded, 1 failed: two\.txt/, 'the failed file is named');
    assert.match(alert.textContent ?? '', /disk full/);
  } finally { for (const u of uploads) u.done.resolve({}); await app.close(); }
});

test('a clean upload batch ends in a success line that names the file (board #214)', async context => {
  const app = await (await compiled).mount(context, {
    props: { visible: true, session: 'fixture' },
    modules: [rpc({ fsUpload: async () => ({ ok: true }) })],
    setup(window) {
      window.localStorage.setItem('tmux_layout_mode', 'desktop');
      window.HTMLInputElement.prototype.click = function (this: HTMLInputElement) {
        Object.defineProperty(this, 'files', { value: [new window.File(['abc'], 'one.txt')] });
        this.onchange?.(new window.Event('change'));
      };
    },
  });
  try {
    await settle(app);
    button(app, 'Upload files').click();
    for (let i = 0; i < 20 && !/Uploaded one\.txt/.test(app.document.querySelector('.files-feedback')?.textContent ?? ''); i++) {
      await new Promise<void>((resolve) => setImmediate(resolve)); await app.flush();
    }
    const status = app.document.querySelector('.files-feedback [role=status]');
    assert.match(status?.textContent ?? '', /Uploaded one\.txt/);
    assert.match(status?.textContent ?? '', /\/fixture/, 'and where it landed');
  } finally { await app.close(); }
});
