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

function contextMenu(app: App, element: Element) {
  element.dispatchEvent(new app.window.MouseEvent('contextmenu', { bubbles: true, cancelable: true, clientX: 30, clientY: 50 }));
}
function menuAction(app: App, label: string) {
  const element = [...app.document.querySelectorAll<HTMLButtonElement>('.ctx button')].find(b => b.textContent?.trim() === label);
  assert.ok(element, `context action: ${label}`); return element;
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
    assert.equal(app.document.querySelector('.copy-toast'), null);
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
