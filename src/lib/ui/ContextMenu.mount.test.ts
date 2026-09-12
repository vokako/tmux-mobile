import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./ContextMenu.svelte', import.meta.url), []);

test('rect menus ignore sibling output scroll but close when their trigger moves (#180)', async context => {
  const fixture = await compileMount(new URL('./ContextMenu.test.svelte', import.meta.url), []);
  for (const rect of [true, false]) {
    const app = await fixture.mount(context, { props: { rect }, modules: [] });
    try {
      app.document.querySelector<HTMLButtonElement>('.source-scroll button')!.click();
      await app.flush();
      app.document.querySelector('.ctx')!.dispatchEvent(new app.window.Event('scroll'));
      await app.flush();
      assert.ok(app.document.querySelector('.ctx'), 'own scroll stays inside the menu');
      app.document.querySelector('.sibling-scroll')!.dispatchEvent(new app.window.Event('scroll'));
      await app.flush();
      assert.equal(!!app.document.querySelector('.ctx'), rect, 'pointer anchors retain broad outside-scroll dismissal');
      if (rect) {
        app.document.querySelector('.source-scroll')!.dispatchEvent(new app.window.Event('scroll'));
        await app.flush();
        assert.equal(app.document.querySelector('.ctx'), null, 'an ancestor scroll still dismisses');
      }
    } finally { await app.close(); }
  }
});

test('controlled menu states are announced and disabled items cannot activate (#164)', async context => {
  let calls = 0;
  const app = await (await compiled).mount(context, {
    props: { at: { x: 10, y: 10 }, items: [
      { label: 'Hidden files', icon: 'folder', checked: false, onselect: () => {} },
      { label: 'Bookmark', checked: true, onselect: () => {} },
      { label: 'Delete', disabled: true, onselect: () => { calls++; } },
    ] }, modules: [],
  });
  try {
    const buttons = [...app.document.querySelectorAll<HTMLButtonElement>('.ctx button')];
    for (const button of buttons) {
      assert.equal(button.type, 'button', 'a menu action never submits an enclosing form (#165)');
      assert.ok(button.querySelector('.menu-icon'), 'one icon column even on rows without an icon (#165)');
    }
    assert.equal(buttons[0]?.getAttribute('role'), 'menuitemcheckbox');
    assert.equal(buttons[0]?.getAttribute('aria-checked'), 'false');
    assert.equal(buttons[1]?.getAttribute('aria-checked'), 'true');
    assert.equal(buttons[2]?.getAttribute('role'), 'menuitem');
    app.document.querySelector('.ctx')!.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true }));
    await app.flush();
    assert.equal(app.document.querySelector('.ctx')?.getAttribute('aria-activedescendant'), buttons[0]?.id);
    assert.ok(buttons[0]?.id, 'the focused menu announces its keyboard cursor');
    buttons[2]!.dispatchEvent(new app.window.MouseEvent('click', { bubbles: true }));
    assert.equal(calls, 0, 'a queued/programmatic click cannot bypass disabled');
  } finally { await app.close(); }
});

test('opening a context menu gives its keys local focus and Escape returns it (#164)', async context => {
  const fixture = await compileMount(new URL('./ContextMenu.test.svelte', import.meta.url), []);
  const app = await fixture.mount(context, { modules: [] });
  try {
    const opener = app.document.querySelector('button')!;
    opener.focus(); opener.click(); await app.flush();
    assert.equal(app.document.activeElement, app.document.querySelector('.ctx'),
      'keys must originate in the menu, not a previously focused pane behind it');
    app.document.activeElement!.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Escape', bubbles: true, cancelable: true }));
    await app.flush();
    assert.equal(app.document.querySelector('.ctx'), null);
    assert.equal(app.document.activeElement, opener);
  } finally { await app.close(); }
});

test('menu ArrowUp starts at the last enabled item and Home/End use the same boundary (#165)', async context => {
  const chosen: string[] = [];
  const app = await (await compiled).mount(context, {
    props: { at: { x: 10, y: 10 }, items: [
      { label: 'First', onselect: () => chosen.push('first') },
      { label: 'Disabled', disabled: true, onselect: () => assert.fail('disabled') },
      { label: 'Last', onselect: () => chosen.push('last') },
    ] }, modules: [],
  });
  try {
    const menu = app.document.querySelector('.ctx')!;
    const buttons = [...menu.querySelectorAll<HTMLButtonElement>('button')];
    const key = async (value: string) => {
      menu.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: value, bubbles: true, cancelable: true }));
      await app.flush();
    };
    await key('ArrowUp');
    assert.equal(menu.getAttribute('aria-activedescendant'), buttons[2]!.id);
    await key('Home');
    assert.equal(menu.getAttribute('aria-activedescendant'), buttons[0]!.id);
    await key('End');
    assert.equal(menu.getAttribute('aria-activedescendant'), buttons[2]!.id);
    await key('Enter');
    assert.deepEqual(chosen, ['last']);
    assert.ok(buttons.every(button => button.tabIndex === -1), 'one menu tab stop, not one per row');
  } finally { await app.close(); }
});

test('Tab closes the menu without stealing the next focus, and foreign fields retain their keys (#165)', async context => {
  const fixture = await compileMount(new URL('./ContextMenu.test.svelte', import.meta.url), []);
  const app = await fixture.mount(context, { modules: [] });
  try {
    const opener = app.document.querySelector('button')!;
    const next = app.document.querySelector('input')!;
    opener.focus(); opener.click(); await app.flush();
    app.document.activeElement!.dispatchEvent(new app.window.KeyboardEvent('keydown', { key: 'Tab', bubbles: true }));
    next.focus(); // Native Tab movement belongs to Chromium coverage.
    await app.flush();
    assert.equal(app.document.querySelector('.ctx'), null);
    assert.equal(app.document.activeElement, next);
    opener.click(); await app.flush();
    next.focus();
    const key = new app.window.KeyboardEvent('keydown', { key: 'ArrowDown', bubbles: true, cancelable: true });
    next.dispatchEvent(key); await app.flush();
    assert.equal(key.defaultPrevented, false);
    assert.equal(app.document.querySelector('.ctx')?.getAttribute('aria-activedescendant'), null);
  } finally { await app.close(); }
});
