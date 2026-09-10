import test from 'node:test';
import assert from 'node:assert/strict';
import { compileMount } from '../test/mount.ts';

const compiled = compileMount(new URL('./CommandButton.svelte', import.meta.url), []);

test('commands keep their name and reject activation while disabled or pending', async context => {
  const fixture = await compiled;
  for (const state of [{}, { disabled: true }, { pending: true }]) {
    let calls = 0;
    const app = await fixture.mount(context, {
      props: { label: 'Save', icon: 'check', variant: 'primary', ...state, onclick: () => { calls++; } },
      modules: [],
    });
    try {
      const button = app.document.querySelector('button')!;
      assert.equal(button.getAttribute('aria-label'), 'Save');
      assert.equal(button.textContent?.trim(), 'Save');
      assert.equal(button.getAttribute('type'), 'button');
      assert.equal(button.getAttribute('aria-busy'), state.pending ? 'true' : null);
      button.click();
      button.dispatchEvent(new app.window.MouseEvent('click', { bubbles: true }));
      assert.equal(calls, state.disabled || state.pending ? 0 : 2);
      assert.equal(button.classList.contains('pending'), !!state.pending);
    } finally { await app.close(); }
  }
});

test('icon commands expose a name without a competing native title', async context => {
  const app = await (await compiled).mount(context, {
    props: { label: 'Refresh', icon: 'refresh', variant: 'icon' }, modules: [],
  });
  try {
    const button = app.document.querySelector('button')!;
    assert.equal(button.getAttribute('aria-label'), 'Refresh');
    assert.equal(button.getAttribute('title'), null);
    assert.equal(button.textContent?.trim(), '');
    assert.ok(button.querySelector('svg'));
  } finally { await app.close(); }
});
