import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./Board.svelte', import.meta.url), [new URL('../core/ws.ts', import.meta.url)]);
const issue = { id: 1, title: 'Keep this issue', body: 'Body', status: 'todo', assignee: '', editable: true,
  created_at: 100, updated_at: 100, notes: [] };
const counts = { todo: 1, doing: 0, review: 0, done: 0, total: 1 };
function rpc(extra: Record<string, (...args: any[]) => unknown>) {
  return {
    projectList: async () => ({ projects: [{ project: { id: 'fixture', session: 'fixture', name: 'Fixture' }, live: true }] }),
    hubRooms: async () => ({ rooms: {} }),
    boardCounts: async () => ({ counts: { fixture: counts } }),
    hubAgents: async () => ({ agents: [] }),
    boardList: async () => ({ issues: [issue] }),
    boardGet: async () => issue,
    ...extra,
  };
}
type App = Awaited<ReturnType<Awaited<typeof compiled>['mount']>>;
async function flush(app: App) { for (let i = 0; i < 8; i++) await app.flush(); }
async function askDelete(app: App) {
  await flush(app);
  app.document.querySelector<HTMLButtonElement>('.card')!.click(); await flush(app);
  app.document.querySelector<HTMLButtonElement>('[aria-label="Delete issue"]')!.click(); await app.flush();
}
const confirm = (app: App) => app.document.querySelector<HTMLButtonElement>('.dlg-actions button:last-child')!;

test('Board deletion stays retryable after rejection and pending Back cannot dismiss it (#167)', async context => {
  let reject!: (error: Error) => void, back!: () => boolean;
  let deletes = 0;
  const app = await (await compiled).mount(context, {
    props: { session: 'fixture', visible: true, onGoBack: (fn: typeof back) => back = fn },
    setup(window) { window.Element.prototype.getAnimations = () => []; },
    modules: [rpc({ boardDelete: () => ++deletes === 1 ? new Promise((_, no) => reject = no) : Promise.resolve({}) })],
  });
  try {
    await askDelete(app);
    confirm(app).click(); confirm(app).click(); await app.flush();
    assert.equal(deletes, 1);
    assert.equal(back(), true); await app.flush();
    assert.ok(app.document.querySelector('[role=alertdialog]'), 'pending Back is consumed by the dialog');
    reject(new Error('Delete denied')); await flush(app);
    assert.match(app.document.querySelector('[role=alertdialog] [role=alert]')?.textContent ?? '', /Delete denied/);
    confirm(app).click(); await flush(app);
    assert.equal(deletes, 2);
    assert.equal(app.document.querySelector('[role=alertdialog]'), null);
    assert.equal(app.document.querySelector('.detail'), null);
  } finally { await app.close(); }
});

test('a Board refresh failure after deletion cannot offer the destructive request again (#167)', async context => {
  let deletes = 0, failRead = true;
  const app = await (await compiled).mount(context, {
    props: { session: 'fixture', visible: true },
    setup(window) { window.Element.prototype.getAnimations = () => []; },
    modules: [rpc({
      boardDelete: async () => { deletes++; return {}; },
      boardList: async () => { if (deletes && failRead) throw new Error('Refresh denied'); return { issues: deletes ? [] : [issue] }; },
    })],
  });
  try {
    await askDelete(app); confirm(app).click(); await flush(app);
    assert.equal(app.document.querySelector('[role=alertdialog]'), null);
    assert.equal(app.document.querySelector('.detail'), null);
    assert.match(app.document.querySelector('.err')?.textContent ?? '', /Refresh denied/);
    failRead = false;
    await app.advance(8000); await flush(app);
    assert.equal(deletes, 1);
    assert.equal(app.document.querySelector('.err'), null);
  } finally { await app.close(); }
});

test('discarding a Board draft offers Keep editing, not Delete (#167)', async context => {
  const app = await (await compiled).mount(context, {
    props: { session: 'fixture', visible: true },
    setup(window) { window.Element.prototype.getAnimations = () => []; },
    modules: [rpc({})],
  });
  try {
    await flush(app);
    app.document.querySelector<HTMLButtonElement>('.card')!.click(); await flush(app);
    const title = app.document.querySelector<HTMLInputElement>('.d-title-input')!;
    title.value = 'Unsaved';
    title.dispatchEvent(new app.window.Event('input', { bubbles: true })); await app.flush();
    app.document.querySelector<HTMLButtonElement>('.d-head [aria-label="Back"], .detail [aria-label="Back"]')!.click();
    await app.flush();
    assert.equal(app.document.querySelector('.dlg-actions button:first-child')?.textContent?.trim(), 'Keep editing');
    assert.equal(app.document.querySelector('.dlg-actions button:last-child')?.textContent?.trim(), 'Discard');
  } finally { await app.close(); }
});

test('a late Board deletion cannot close a newer visit to the same room (#167)', async context => {
  let finish!: () => void;
  const app = await (await compiled).mount(context, {
    props: { session: 'fixture', visible: true },
    setup(window) { window.Element.prototype.getAnimations = () => []; },
    modules: [rpc({
      projectList: async () => ({ projects: ['fixture', 'other'].map(session => ({
        project: { id: session, name: session, session }, live: true,
      })) }),
      boardCounts: async () => ({ counts: { fixture: counts, other: counts } }),
      boardDelete: () => new Promise<void>(yes => finish = yes),
    })],
  });
  try {
    await askDelete(app); confirm(app).click(); await app.flush();
    for (const name of ['other', 'fixture']) {
      [...app.document.querySelectorAll<HTMLButtonElement>('button.proj-row')]
        .find(row => row.querySelector('.p-name')?.textContent === name)!.click();
      await flush(app);
    }
    app.document.querySelector<HTMLButtonElement>('.card')!.click(); await flush(app);
    finish(); await flush(app);
    assert.ok(app.document.querySelector('.detail'), 'completion does not navigate the newer visit');
    assert.equal(app.document.querySelector('[role=alertdialog]'), null);
  } finally { await app.close(); }
});
