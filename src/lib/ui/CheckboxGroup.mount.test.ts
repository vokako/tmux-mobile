import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./CheckboxGroup.svelte', import.meta.url), []);
const options = [{ value: 'a', label: 'Alpha' }, { value: 'b', label: 'Beta' }];

test('checkbox choices preserve unlisted values and report the changed membership only', async context => {
  const updates: string[][] = [];
  const app = await (await compiled).mount(context, {
    props: { label: 'Skills', options, value: ['unlisted', 'a'], onchange: (next: string[]) => updates.push(Array.from(next)) },
    modules: [],
  });
  try {
    assert.equal(app.document.querySelector('legend')?.textContent, 'Skills');
    const [a, b] = app.document.querySelectorAll<HTMLInputElement>('input[type=checkbox]');
    assert.equal(a!.checked, true);
    assert.equal(b!.checked, false);
    a!.click();
    b!.click();
    assert.deepEqual(updates, [['unlisted'], ['unlisted', 'a', 'b']]);
    assert.equal(a!.checked, true, 'uncommitted native changes reset to the caller value');
    assert.equal(b!.checked, false);
    assert.equal(a!.labels?.[0]?.textContent?.trim(), 'Alpha');
  } finally { await app.close(); }
});

test('the disabled group blocks both native and manually dispatched changes', async context => {
  const app = await (await compiled).mount(context, {
    props: { label: 'Skills', options, disabled: true, onchange: () => assert.fail('disabled choice') }, modules: [],
  });
  try {
    const input = app.document.querySelector('input')!;
    assert.equal(input.matches(':disabled'), true);
    input.click();
    input.dispatchEvent(new app.window.Event('change', { bubbles: true }));
  } finally { await app.close(); }
});
