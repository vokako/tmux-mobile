// Source-contract tests for Terminal.svelte (see docs/conventions/testing.md).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Terminal.svelte', import.meta.url), 'utf8');

test('Terminal chrome uses only Team-filtered notification queries', () => {
  assert.match(source, /attention=\{otherTerminalSessionHasNotification\(session\)\}/u);
  assert.match(
    source,
    /\{@const notice = terminalNotificationForWindow\(w\.session, w\.window\)\}/u,
  );
  // The unfiltered query must never appear in Terminal chrome — Team dots
  // would leak into the window switcher.
  assert.doesNotMatch(source, /\bnotificationForWindow\(/u);
});
