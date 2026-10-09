// Board #324: the scratch panel's lifecycle — lazy ensure on open, focus in
// and back, close hides (the session lives on), Kill ends it after a
// confirmation, an ended session offers an explicit re-open, and a server
// switch drops every pending completion and never ensures again by itself.
import assert from 'node:assert/strict';
import test from 'node:test';
import type { TestContext } from 'node:test';
import { compileMount } from '../test/mount.ts';

let compiled: ReturnType<typeof compileMount> | undefined;
// The real Terminal needs xterm, which this tier does not paint; a stub
// component stands in and records the target it was given.
const fixture = () => compiled ??= compileMount(new URL('./ScratchPanel.test.svelte', import.meta.url), [
  new URL('../core/ws.ts', import.meta.url),
  new URL('../terminal/Terminal.svelte', import.meta.url),
]);
const terminalStub = {
  default: (anchor: Comment, props: { target: string; visible: boolean }) => {
    const el = anchor.ownerDocument.createElement('div');
    el.className = 'xterm-wrap';
    el.dataset.target = props.target;
    el.innerHTML = '<textarea class="xterm-helper-textarea"></textarea>';
    anchor.before(el);
  },
};

function rpc(over: Record<string, unknown> = {}) {
  const calls: string[] = [];
  return {
    calls,
    mod: {
      scratchSession: async () => { calls.push('ensure'); return { session: 'tmm-scratch', target: 'tmm-scratch:1.1' }; },
      scratchKill: async () => { calls.push('kill'); return { killed: true }; },
      subscribe: () => {}, unsubscribe: () => {}, capturePane: async () => ({ output: '' }),
      addPaneOutputListener: () => {}, removePaneOutputListener: () => {},
      addPaneClosedListener: () => {}, removePaneClosedListener: () => {},
      ...over,
    } as Record<string, (...a: any[]) => unknown>,
  };
}
async function mount(context: TestContext, mod: Record<string, (...a: any[]) => unknown>) {
  const f = await fixture();
  return f.mount(context, {
    modules: [mod, terminalStub as unknown as Record<string, (...a: any[]) => unknown>],
    setup(w) { w.Element.prototype.getAnimations = () => []; w.HTMLCanvasElement.prototype.getContext = () => null; },
  });
}
const until = async (app: { flush: () => Promise<void> }, ok: () => boolean, n = 30) => { for (let i = 0; i < n && !ok(); i++) await app.flush(); };

test('closed: inert, hidden, nothing ensured; open ensures once and mounts the Terminal on the pane target', { timeout: 60000 }, async (context) => {
  const r = rpc();
  const app = await mount(context, r.mod);
  try {
    const panel = () => app.document.querySelector<HTMLElement>('.scratch')!;
    const inert = () => panel().hasAttribute('inert') || (panel() as any).inert === true;
    assert.ok(inert(), 'a closed panel is not reachable');
    assert.equal(panel().getAttribute('aria-hidden'), 'true');
    assert.deepEqual(r.calls, [], 'nothing until the reader opens it');
    app.document.querySelector<HTMLButtonElement>('.opener')!.focus();
    app.window.__scratch.open = true;
    await until(app, () => !!app.document.querySelector('.scratch .xterm-wrap'));
    assert.deepEqual(r.calls, ['ensure']);
    assert.equal(app.document.querySelector<HTMLElement>('.scratch .xterm-wrap')!.dataset.target, 'tmm-scratch:1.1', 'the concrete pane target');
    assert.ok(!inert());
    await until(app, () => app.document.activeElement?.className === 'xterm-helper-textarea', 6);
    assert.equal(app.document.activeElement?.className, 'xterm-helper-textarea', 'focus goes into the terminal once it is mounted and shown');
    assert.ok(panel().classList.contains('open'));
    // Close hides; reopening does not ensure again (the session lives on).
    app.window.__scratch.open = false;
    for (let i = 0; i < 4; i++) await app.flush();
    assert.ok(inert());
    assert.equal(app.document.activeElement?.className, 'opener', 'focus returns to where it was');
    assert.ok(app.document.querySelector('.scratch .xterm-wrap'), 'the Terminal stays mounted while hidden');
    app.window.__scratch.open = true;
    for (let i = 0; i < 4; i++) await app.flush();
    assert.deepEqual(r.calls, ['ensure'], 'no second ensure for a session that is still there');
  } finally { await app.close(); }
});

test('Escape closes from the head, Kill asks first then ends the session, an ended session re-opens only on request', { timeout: 60000 }, async (context) => {
  const r = rpc();
  const app = await mount(context, r.mod);
  try {
    app.window.__scratch.open = true;
    await until(app, () => !!app.document.querySelector('.scratch .xterm-wrap'));
    const head = app.document.querySelector<HTMLElement>('.scratch-head')!;
    head.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    await app.flush();
    assert.equal(app.window.__scratch.open, false, 'Escape on the head closes');
    app.window.__scratch.open = true;
    for (let i = 0; i < 4; i++) await app.flush();
    app.document.querySelector<HTMLButtonElement>('.scratch-head [aria-label="Kill scratch session"]')!.click();
    await until(app, () => !!app.document.querySelector('.confirm-dialog, [role="alertdialog"], dialog[open]'));
    assert.deepEqual(r.calls, ['ensure'], 'nothing is killed before the confirmation');
    const confirm = [...app.document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes('Kill scratch session') && !b.closest('.scratch-head'))!;
    confirm.click();
    await until(app, () => r.calls.includes('kill') && app.window.__scratch.open === false);
    assert.deepEqual(r.calls, ['ensure', 'kill']);
    app.window.__scratch.open = true;
    await until(app, () => !!app.document.querySelector('.scratch-state'));
    assert.match(app.document.querySelector('.scratch-state')!.textContent!, /Session ended/u, 'an ended session says so');
    assert.deepEqual(r.calls, ['ensure', 'kill'], 'no background recreate');
    [...app.document.querySelectorAll<HTMLButtonElement>('.scratch-state button')][0]!.click();
    await until(app, () => r.calls.length === 3);
    assert.deepEqual(r.calls, ['ensure', 'kill', 'ensure'], '"Open again" is the explicit re-ensure');
  } finally { await app.close(); }
});

test('a server switch drops a pending ensure and never ensures by itself; the edge switch moves the panel', { timeout: 60000 }, async (context) => {
  let answer: ((v: unknown) => void) | null = null;
  const r = rpc({ scratchSession: () => { r.calls.push('ensure'); return new Promise((res) => { answer = res; }); } });
  const app = await mount(context, r.mod);
  try {
    app.window.__scratch.open = true;
    await until(app, () => !!answer);
    // The switch starts: the panel is closed and no longer live; then the
    // keyed tree remounts for the new server.
    app.window.__scratch.live = false;
    app.window.__scratch.open = false;
    await app.flush();
    answer!({ session: 'tmm-scratch', target: 'tmm-scratch:0.0' });
    for (let i = 0; i < 4; i++) await app.flush();
    assert.equal(app.document.querySelector('.scratch .xterm-wrap'), null, 'the old server\'s answer mounted nothing');
    app.window.__scratch.bump();
    app.window.__scratch.live = true;
    for (let i = 0; i < 4; i++) await app.flush();
    assert.deepEqual(r.calls, ['ensure'], 'no ensure on the new server until the reader opens it');
    // Edge: the head's switch moves it to the left.
    app.document.querySelectorAll<HTMLButtonElement>('.scratch-head .segmented button')[1]!.click();
    await app.flush();
    assert.equal(app.window.__scratch.edge, 'left');
    assert.ok(app.document.querySelector('.scratch.left'));
    assert.equal(app.document.querySelector('.scratch .side-handle')!.getAttribute('aria-orientation'), 'vertical', 'the left panel resizes on X');
  } finally { await app.close(); }
});
