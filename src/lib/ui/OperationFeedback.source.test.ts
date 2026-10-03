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
  assert.match(source, /\.feedback-body \{ min-width: 0; flex: 1 1 auto;/u,
    'intrinsic text width keeps a short notice and Close in one row in a shrink-to-fit popover');
  assert.doesNotMatch(source, /#[0-9a-fA-F]{3,8}\b|rgba?\(/u, 'shared tokens own the palette');
});

test('the progress glyph fills its box, so the spin is centred (board #301)', () => {
  // Icon draws 16px; the phone's --control-icon-size is 17px. A corner-pinned
  // glyph orbited the box centre by 0.5px (measured at 390, dpr 3).
  assert.match(source, /\.feedback-icon \{ display: inline-flex; flex: none; width: var\(--control-icon-size\); height: var\(--control-icon-size\); \}/u);
  assert.match(source, /\.feedback-icon :global\(svg\) \{ width: 100%; height: 100%; \}/u);
});
