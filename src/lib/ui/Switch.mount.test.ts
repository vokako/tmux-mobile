import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./Switch.svelte', import.meta.url), []);

test('the switch publishes a boolean intent while the caller remains the state owner', async context => {
  const values: boolean[] = [];
  const app = await (await compiled).mount(context, {
    props: { label: 'Permission', checked: true, onchange: (value: boolean) => values.push(value) }, modules: [],
  });
  try {
    const button = app.document.querySelector('button')!;
    assert.equal(button.getAttribute('role'), 'switch');
    assert.equal(button.getAttribute('aria-label'), 'Permission');
    assert.equal(button.getAttribute('aria-checked'), 'true');
    button.click();
    await app.flush();
    assert.deepEqual(values, [false]);
    assert.equal(button.getAttribute('aria-checked'), 'true', 'no duplicate committed state inside the control');
  } finally { await app.close(); }
});

test('a disabled switch cannot emit a change', async context => {
  const app = await (await compiled).mount(context, {
    props: { label: 'Permission', disabled: true, onchange: () => assert.fail('disabled switch') }, modules: [],
  });
  try {
    const button = app.document.querySelector('button')!;
    button.click();
    button.dispatchEvent(new app.window.MouseEvent('click', { bubbles: true }));
  } finally { await app.close(); }
});
