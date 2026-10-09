// Board #324: the scratch panel's lifecycle — lazy ensure on open, focus in
// and back, close hides (the session lives on), Kill ends it after a
// confirmation, and a server switch drops every pending completion and never
// ensures again by itself.
// Board #326: opening converges on a LIVE session from every state the panel
// knows about — an ended one included, which is what left the owner opening
// the panel on a bare "Session ended" line — while `ready` still trusts the
// pane it has until the subscription says otherwise.
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
  default: (anchor: Comment, props: { target: string; visible: boolean; onPaneExit: (t: string) => void }) => {
    const el = anchor.ownerDocument.createElement('div');
    el.className = 'xterm-wrap';
    el.dataset.target = props.target;
    el.innerHTML = '<textarea class="xterm-helper-textarea"></textarea>';
    anchor.before(el);
    // The real Terminal calls this from its pane_closed listener, which it
    // keeps registered while HIDDEN (rule 7) — the route #326 is about.
    (anchor.ownerDocument.defaultView as unknown as { __paneExit: (t: string) => void }).__paneExit = props.onPaneExit;
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

test('Escape closes from the head, Kill asks first then ends the session, and opening an ended panel brings a live one back', { timeout: 60000 }, async (context) => {
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
    // Nothing recreates it in the background: the panel is closed and the
    // session stays dead until the READER asks for it again.
    for (let i = 0; i < 6; i++) await app.flush();
    assert.deepEqual(r.calls, ['ensure', 'kill'], 'no background recreate');
    // And that ask is opening the panel (#326): it ensures from `ended` too,
    // so the reader gets a prompt instead of the bare "Session ended" line.
    app.window.__scratch.open = true;
    await until(app, () => r.calls.length === 3);
    assert.deepEqual(r.calls, ['ensure', 'kill', 'ensure'], 'opening an ended panel ensures');
    await until(app, () => !!app.document.querySelector('.scratch .xterm-wrap'));
    assert.equal(app.document.querySelector<HTMLElement>('.scratch .xterm-wrap')!.dataset.target, 'tmm-scratch:1.1');
    assert.equal(app.document.querySelector('.scratch-state'), null, 'no ended state left on screen');
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
    // Edge: the head's two icons move it to the right (#326).
    const edges = app.document.querySelectorAll<HTMLButtonElement>('.scratch-head .segmented button');
    assert.deepEqual([...edges].map((b) => b.getAttribute('aria-label')), ['Bottom', 'Right'],
      'icons, with their words as the accessible names');
    assert.ok([...edges].every((b) => !b.textContent?.trim()), 'no Bottom/Right text in the head');
    edges[1]!.click();
    await app.flush();
    assert.equal(app.window.__scratch.edge, 'right');
    assert.ok(app.document.querySelector('.scratch.right'));
    assert.equal(app.document.querySelector('.scratch .side-handle')!.getAttribute('aria-orientation'), 'vertical', 'the side-docked panel resizes on X');
  } finally { await app.close(); }
});

test('a refusal is a stable error: no automatic retry, Open again asks once (#324 review)', { timeout: 60000 }, async (context) => {
  const r = rpc({ scratchSession: async () => { r.calls.push('ensure'); throw new Error("a tmux session named 'tmm-scratch' already exists and is not the scratch terminal"); } });
  const app = await mount(context, r.mod);
  try {
    app.window.__scratch.open = true;
    await until(app, () => !!app.document.querySelector('.scratch-err'));
    for (let i = 0; i < 12; i++) await app.flush();
    assert.deepEqual(r.calls, ['ensure'], 'the error does not re-ensure');
    assert.match(app.document.querySelector('.scratch-err')!.textContent!, /already exists/u, 'the reason stays on screen');
    app.document.querySelector<HTMLButtonElement>('.scratch-state button')!.click();
    for (let i = 0; i < 8; i++) await app.flush();
    assert.deepEqual(r.calls, ['ensure', 'ensure'], 'one explicit retry, one call');
  } finally { await app.close(); }
});

test('the server-keyed tree unmounting first still drops a pending completion (#324 review, real teardown order)', { timeout: 60000 }, async (context) => {
  let killed: ((v: unknown) => void) | null = null;
  const r = rpc({ scratchKill: () => { r.calls.push('kill'); return new Promise((res) => { killed = res; }); } });
  const app = await mount(context, r.mod);
  try {
    app.window.__scratch.open = true;
    await until(app, () => !!app.document.querySelector('.scratch .xterm-wrap'));
    app.document.querySelector<HTMLButtonElement>('.scratch-head [aria-label="Kill scratch session"]')!.click();
    await until(app, () => !![...app.document.querySelectorAll('button')].find((b) => b.textContent?.includes('Kill scratch session') && !b.closest('.scratch-head')));
    [...app.document.querySelectorAll<HTMLButtonElement>('button')].find((b) => b.textContent?.includes('Kill scratch session') && !b.closest('.scratch-head'))!.click();
    await until(app, () => !!killed);
    // The switch tears the keyed tree down BEFORE `live` ever goes false, and
    // the reader opens the new server's panel.
    app.window.__scratch.bump();
    await app.flush();
    app.window.__scratch.open = true;
    await until(app, () => !!app.document.querySelector('.scratch .xterm-wrap'));
    killed!({ killed: true });
    for (let i = 0; i < 6; i++) await app.flush();
    assert.equal(app.window.__scratch.open, true, 'the old panel\'s kill completion did not close the new one');
  } finally { await app.close(); }
});

test('a session that ends while the panel is CLOSED is not what the reader opens into (#326)', { timeout: 60000 }, async (context) => {
  // The owner's "点开 Terminal 之后，我现在经常看到里面什么都没有": a hidden
  // Terminal stays subscribed, so pane_closed arrives while the panel is
  // closed and leaves phase='ended' with no target. Before #326 the open
  // effect only re-ensured from idle|error, so the next open showed the bare
  // "Session ended" line instead of a prompt.
  const r = rpc();
  const app = await mount(context, r.mod);
  try {
    app.window.__scratch.open = true;
    await until(app, () => !!app.document.querySelector('.scratch .xterm-wrap'));
    app.window.__scratch.open = false;
    for (let i = 0; i < 4; i++) await app.flush();
    // The shell exits / the session is killed from outside: the subscription
    // reports it to a panel nobody is looking at.
    (app.window as unknown as { __paneExit: (t: string) => void }).__paneExit('tmm-scratch:1.1');
    for (let i = 0; i < 4; i++) await app.flush();
    // (The Terminal stub's node is not reactive, so the panel's own state
    // block is what says the pane is gone.)
    assert.match(app.document.querySelector('.scratch-state')!.textContent!, /Session ended/u,
      'the panel drops the dead pane while nobody is looking');
    assert.deepEqual(r.calls, ['ensure'], 'and nothing is recreated behind the reader');
    app.window.__scratch.open = true;
    await until(app, () => r.calls.length === 2);
    assert.deepEqual(r.calls, ['ensure', 'ensure'], 'opening ensures from ended');
    await until(app, () => !app.document.querySelector('.scratch-state'));
    assert.equal(app.document.querySelector('.scratch-state'), null, 'a prompt, not a notice');
  } finally { await app.close(); }
});

test('a `ready` panel trusts its pane until the subscription says otherwise — the window, stated (#326 review P2)', { timeout: 60000 }, async (context) => {
  // Honest boundary: if the session is killed BEHIND our back and the panel
  // is reopened before pane_closed arrives, the panel still believes its
  // target and only focuses it — it does not re-ask on every open (board
  // #324's rule, and its retry-loop guard). The subscription closes the
  // window: once pane_closed lands, the state is `ended` and the next open
  // ensures. This is the behaviour, recorded, not a claim that opening is
  // always a round-trip.
  const r = rpc();
  const app = await mount(context, r.mod);
  try {
    app.window.__scratch.open = true;
    await until(app, () => !!app.document.querySelector('.scratch .xterm-wrap'));
    app.window.__scratch.open = false;
    for (let i = 0; i < 4; i++) await app.flush();
    app.window.__scratch.open = true;              // reopened inside the window
    for (let i = 0; i < 8; i++) await app.flush();
    assert.deepEqual(r.calls, ['ensure'], 'no second ensure while the panel believes its pane is live');
    assert.equal(app.document.querySelector<HTMLElement>('.scratch .xterm-wrap')!.dataset.target, 'tmm-scratch:1.1');
    (app.window as unknown as { __paneExit: (t: string) => void }).__paneExit('tmm-scratch:1.1');
    for (let i = 0; i < 4; i++) await app.flush();
    app.window.__scratch.open = false;
    await app.flush();
    app.window.__scratch.open = true;
    await until(app, () => r.calls.length === 2);
    assert.deepEqual(r.calls, ['ensure', 'ensure'], 'the close notice is what makes the next open re-ask');
  } finally { await app.close(); }
});
