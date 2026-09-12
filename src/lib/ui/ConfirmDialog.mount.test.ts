import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./ConfirmDialog.svelte', import.meta.url), []);

test('confirmation focuses Cancel and keeps keyboard traversal inside the open dialog', async context => {
  let cancelled = 0;
  const app = await (await compiled).mount(context, {
    props: { open: true, title: 'Remove item', confirmLabel: 'Remove', cancelLabel: 'Keep',
      oncancel: () => { cancelled++; } }, modules: [],
  });
  try {
    const [cancel, confirm] = app.document.querySelectorAll<HTMLButtonElement>('.dlg-actions button');
    assert.equal(app.document.activeElement, cancel);
    app.window.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Tab', cancelable: true }));
    assert.equal(app.document.activeElement, confirm);
    app.window.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Tab', cancelable: true }));
    assert.equal(app.document.activeElement, cancel);
    app.window.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', cancelable: true }));
    assert.equal(cancelled, 1);
  } finally { await app.close(); }
});

test('a busy confirmation cannot cancel or submit twice and keeps its operation label', async context => {
  const app = await (await compiled).mount(context, {
    props: { open: true, title: 'Remove item', confirmLabel: 'Remove', cancelLabel: 'Keep', busy: true,
      oncancel: () => assert.fail('pending cancel'), onconfirm: () => assert.fail('duplicate operation') }, modules: [],
  });
  try {
    const buttons = app.document.querySelectorAll<HTMLButtonElement>('.dlg-actions button');
    assert.ok([...buttons].every(button => button.disabled));
    assert.equal(buttons[1]!.textContent?.trim(), 'Remove');
    for (const button of buttons) button.click();
    app.document.querySelector<HTMLElement>('.dlg-backdrop')!.click();
    app.window.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', cancelable: true }));
    assert.equal(app.document.querySelector('[role=alertdialog]')?.getAttribute('aria-busy'), 'true');
  } finally { await app.close(); }
});

test('danger is severity, not a trash-icon classifier (#167)', async context => {
  const app = await (await compiled).mount(context, {
    props: { open: true, title: 'Stop agent alpha?', confirmLabel: 'Stop', confirmIcon: 'stop', danger: true },
    modules: [],
  });
  try {
    const confirm = app.document.querySelector<HTMLButtonElement>('.dlg-actions button:last-child')!;
    assert.ok(confirm.classList.contains('danger'));
    assert.ok(confirm.querySelector('svg rect[x="6"][width="12"]'), 'the caller supplies the process-stop glyph');
    assert.equal(confirm.textContent?.trim(), 'Stop');
  } finally { await app.close(); }
});

test('a failed operation is visible inside the confirmation and stays retryable (#167)', async context => {
  let retries = 0;
  const app = await (await compiled).mount(context, {
    props: { open: true, title: 'Delete report?', confirmLabel: 'Delete', confirmIcon: 'trash',
      error: 'Permission denied', onconfirm: () => retries++ },
    modules: [],
  });
  try {
    const dialog = app.document.querySelector('[role=alertdialog]')!;
    const error = dialog.querySelector('[role=alert]');
    assert.equal(error?.textContent, 'Permission denied');
    assert.equal(dialog.getAttribute('aria-describedby'), error?.id);
    const confirm = dialog.querySelector<HTMLButtonElement>('.dlg-actions button:last-child')!;
    assert.equal(confirm.disabled, false);
    confirm.click();
    assert.equal(retries, 1);
  } finally { await app.close(); }
});

test('disabling the clicked command parks focus inside the modal without replacing its return target (#167)', async context => {
  const fixture = await compileMount(new URL('./ConfirmDialog.test.svelte', import.meta.url), []);
  let controls!: { fail: () => void; close: () => void };
  const app = await fixture.mount(context, {
    props: { ready: (value: typeof controls) => controls = value },
    setup(window) {
      const origin = window.document.createElement('button');
      origin.id = 'origin';
      window.document.body.append(origin);
      origin.focus();
    },
    modules: [],
  });
  try {
    const dialog = app.document.querySelector<HTMLElement>('[role=alertdialog]')!;
    const confirm = dialog.querySelector<HTMLButtonElement>('.dlg-actions button:last-child')!;
    confirm.focus(); confirm.click(); await app.flush();
    assert.equal(app.document.activeElement, dialog, 'a disabled command cannot retain usable focus');
    controls.fail(); await app.flush();
    assert.ok(dialog.contains(app.document.activeElement), 'failure keeps keyboard ownership in the dialog');
    controls.close(); await app.flush();
    assert.equal(app.document.activeElement?.id, 'origin', 'busy transitions do not replace the original focus target');
  } finally { await app.close(); }
});
