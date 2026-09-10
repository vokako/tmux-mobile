import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./Files.svelte', import.meta.url), [new URL('../core/ws.ts', import.meta.url)]);
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
