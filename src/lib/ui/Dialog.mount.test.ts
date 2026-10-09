// The shared dialog shell (board #317). The confirmation's own contract is
// tested through ConfirmDialog, which is where the words and the commands
// live; these cover what the SHELL owns for every dialog and what no single
// consumer exercised before it existed: focus in and back, the IME-safe
// Escape, the Tab wrap over fields as well as buttons, and the modal stack.
import test from 'node:test';
import assert from 'node:assert/strict';
import type { TestContext } from 'node:test';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./Dialog.test.svelte', import.meta.url), []);

type Controls = { open: () => void; close: () => void; autofocus: () => void; raiseModal: () => void };

async function host(context: TestContext, oncancel: () => void = () => {}) {
  let controls!: Controls;
  const app = await (await compiled).mount(context, {
    props: { ready: (value: Controls) => controls = value, oncancel },
    modules: [],
  });
  return { app, controls: () => controls };
}

test('opening parks focus on the first control and closing gives it back to the opener', async context => {
  const { app, controls } = await host(context);
  try {
    const opener = app.document.querySelector<HTMLButtonElement>('.opener')!;
    opener.focus();
    controls().open(); await app.flush();
    assert.equal(app.document.activeElement, app.document.querySelector('.field'),
      'the shell focuses the first thing the dialog offers — for a confirmation that is Cancel');
    controls().close(); await app.flush();
    assert.equal(app.document.activeElement, opener, 'and hands focus back to what opened it');
  } finally { await app.close(); }
});

test('content that claims focus itself keeps it', async context => {
  // ConnectFields autofocuses the address field; the shell must not move
  // focus to "the first control" over the top of that.
  const { app, controls } = await host(context);
  try {
    controls().autofocus(); await app.flush();
    assert.ok(app.document.querySelector('.field')!.hasAttribute('autofocus'));
    assert.equal(app.document.activeElement, app.document.querySelector('.field'));
  } finally { await app.close(); }
});

test('Tab wraps over fields AND buttons, inside the dialog only', async context => {
  const { app, controls } = await host(context);
  try {
    controls().open(); await app.flush();
    const field = app.document.querySelector('.field');
    const act = app.document.querySelector('.act');
    const tab = (shift = false) => app.window.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Tab', shiftKey: shift, cancelable: true }));
    assert.equal(app.document.activeElement, field);
    tab(); assert.equal(app.document.activeElement, act);
    tab(); assert.equal(app.document.activeElement, field, 'the cycle never reaches the opener behind the scrim');
    tab(true); assert.equal(app.document.activeElement, act, 'and runs backwards the same way');
  } finally { await app.close(); }
});

test('Escape cancels — but never while an IME is composing', async context => {
  let cancelled = 0;
  const { app, controls } = await host(context, () => { cancelled++; });
  try {
    controls().open(); await app.flush();
    app.window.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', isComposing: true, cancelable: true }));
    assert.equal(cancelled, 0, 'that Escape belongs to the candidate window, not to the dialog');
    app.window.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', cancelable: true }));
    assert.equal(cancelled, 1);
  } finally { await app.close(); }
});

test('the scrim cancels, and only the TOP modal answers the keyboard', async context => {
  let cancelled = 0;
  const { app, controls } = await host(context, () => { cancelled++; });
  try {
    controls().open(); await app.flush();
    const dialog = app.document.querySelector<HTMLElement>('[role=dialog][aria-label="Edit thing"]')!;
    assert.equal(dialog.getAttribute('aria-modal'), 'true');
    assert.ok(!dialog.classList.contains('sheet'), 'desktop is the centred card');
    controls().raiseModal(); await app.flush();
    app.window.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', cancelable: true }));
    assert.equal(cancelled, 0, 'a dialog opened over this one owns Escape');
    app.document.querySelector<HTMLElement>('.dlg-backdrop')!.click();
    assert.equal(cancelled, 1, 'the scrim is still a cancel');
  } finally { await app.close(); }
});
