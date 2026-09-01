// Source pins for the system-vitals corner (board #56). These hold the
// component to its contract with the OTHER territories: transport is
// injected, the tempo is low and floored, and failure never blanks a reading.
import { test } from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';

const source = readFileSync(
  fileURLToPath(new URL('./SystemStatus.svelte', import.meta.url)),
  'utf8',
);

test('transport is INJECTED — the component never imports the socket or App (board #56)', () => {
  // The lead's territory split: #55/#57 own App/ws. This component must stay
  // mountable with a stub `load` in a test and integrable with two lines.
  assert.ok(!/core\/ws|from ['"].*\/App|hub\//u.test(source),
    'no ws.ts / App / hub imports — `load` is a prop');
  assert.match(source, /load,/u, 'the load callback is a prop');
});

test('the poll is low-frequency, floored, and stops while hidden', () => {
  // The server computes CPU% over the poll interval itself — a hot loop
  // would be cost AND noise, so the clamp must survive refactors.
  assert.match(source, /Math\.max\(SYS_POLL_MIN_MS, interval\)/u,
    'the interval prop is clamped to the floor');
  assert.match(source, /if \(!visible\) return;/u,
    'hidden → no timer (the hidden-terminal lesson in miniature)');
  assert.match(source, /return \(\) => clearInterval\(timer\);/u,
    'the effect cleans its own timer up');
});

test('fail-soft: a failed load keeps the last reading; nothing renders before the first', () => {
  // "I could not ask" is not "there is nothing" (the roster lesson): only a
  // truthy answer overwrites, and the catch swallows without clearing.
  assert.match(source, /if \(r\) status = r;/u, 'null/failed answers never clear the reading');
  assert.ok(!/catch[\s\S]{0,80}status\s*=/u.test(source), 'the catch never writes status');
  // The verdict rule: the markup is gated on having parts to say.
  assert.match(source, /\{#if parts\.length\}/u, 'renders nothing until a first reading');
});

test('the corner wears tokens, not raw type', () => {
  assert.ok(!/font-size:\s*\d/u.test(source), 'no raw px font-size (tokens contract)');
  assert.match(source, /var\(--fs-micro\)/u, 'micro chrome step');
});
