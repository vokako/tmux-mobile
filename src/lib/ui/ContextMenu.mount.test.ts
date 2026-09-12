import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./ContextMenu.svelte', import.meta.url), []);

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
