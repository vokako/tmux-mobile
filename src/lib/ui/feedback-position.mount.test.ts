import assert from 'node:assert/strict';
import test from 'node:test';
import { compileMount } from '../test/mount.ts';

test('feedback placement tracks locally, waits for measurement and disposes its observers/listener (#167)', async context => {
  const compiled = await compileMount(new URL('./FeedbackPosition.test.svelte', import.meta.url), []);
  let hide!: () => void;
  let disconnects = 0, adds = 0, removes = 0;
  const app = await compiled.mount(context, {
    props: { ready: (fn: () => void) => hide = fn }, modules: [],
    setup(window) {
      window.ResizeObserver = class { observe() {} unobserve() {} disconnect() { disconnects++; } };
      const add = window.HTMLElement.prototype.addEventListener;
      const remove = window.HTMLElement.prototype.removeEventListener;
      window.HTMLElement.prototype.addEventListener = function (...args: Parameters<typeof add>) {
        if (args[0] === 'scroll' && this.classList.contains('scrollport')) adds++;
        return add.apply(this, args);
      };
      window.HTMLElement.prototype.removeEventListener = function (...args: Parameters<typeof remove>) {
        if (args[0] === 'scroll' && this.classList.contains('scrollport')) removes++;
        return remove.apply(this, args);
      };
    },
  });
  try {
    assert.equal(app.document.querySelector('.pop-layer')?.classList.contains('ready'), false,
      'jsdom supplies no geometry; the box must not pretend to be measured');
    assert.equal(app.document.querySelector('.pop-layer')?.hasAttribute('inert'), true,
      'an unplaced Close must not remain a keyboard target');
    assert.ok(adds > 0);
    hide(); await app.flush();
    assert.equal(app.document.querySelector('.pop-layer'), null);
    assert.equal(removes, adds);
    assert.ok(disconnects > 0);
  } finally { await app.close(); }
});
