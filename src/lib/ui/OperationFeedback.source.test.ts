import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./OperationFeedback.svelte', import.meta.url), 'utf8');

test('operation feedback is presentation over the shared chrome and control owners (#167)', () => {
  assert.match(source, /class="operation-feedback menu-surface"/u);
  assert.match(source, /<CommandButton variant="icon" icon="x"/u);
  assert.doesNotMatch(source, /setTimeout|setInterval|core\/ws|window\.|document\.|1500/u);
  assert.match(source, /prefers-reduced-motion: reduce/u);
  assert.match(source, /class:config-error=/u);
  assert.doesNotMatch(source, /#[0-9a-fA-F]{3,8}\b|rgba?\(/u, 'shared tokens own the palette');
});
