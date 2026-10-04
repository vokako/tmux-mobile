import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { readFileSync } from 'node:fs';
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

test('the download glyph: a dashed ring that turns for the whole progress phase, a still arrow (board #307)', () => {
  // Owner 2026-10-04: "圆圈都不动了". With real byte progress the refresh glyph
  // stopped at the first piece; a download now turns its ring until it ends.
  assert.match(source, /const downloadGlyph = \$derived\(value\?\.kind === 'progress' && value\.glyph === 'download'\);/u);
  assert.match(source, /class:spinning=\{value\.kind === 'progress' && percent === null && !downloadGlyph\}/u, 'refresh keeps its rule for every other user');
  assert.match(source, /class:downloading=\{downloadGlyph\}/u, 'no percent condition: the ring turns while progress lasts');
  assert.match(source, /\.spinning, \.downloading :global\(\.dl-ring\) \{ animation: spin 0\.6s linear infinite; \}/u, 'one turn, the existing keyframes and duration');
  assert.match(source, /\.downloading :global\(\.dl-ring\) \{ transform-box: fill-box; transform-origin: center; \}/u, 'the ring turns about its own centre');
  assert.match(source, /prefers-reduced-motion: reduce\) \{ \.spinning, \.downloading :global\(\.dl-ring\) \{ animation: none; \} \}/u);
  const icon = readFileSync(new URL('./Icon.svelte', import.meta.url), 'utf8');
  const glyph = icon.slice(icon.indexOf("name === 'downloading'"), icon.indexOf("name === 'download'}"));
  assert.match(glyph, /<circle class="dl-ring" cx="12" cy="12" r="10" stroke-dasharray=/u, 'a dashed ring centred in the 24 box');
  assert.equal((glyph.match(/class="dl-ring"/gu) ?? []).length, 1, 'only the ring carries the turning class');
});
