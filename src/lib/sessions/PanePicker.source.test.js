// Source-contract tests for PanePicker.svelte (see docs/conventions/testing.md).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./PanePicker.svelte', import.meta.url), 'utf8');

test('pane picker suppresses Team summary and window dots', () => {
  assert.match(
    source,
    /\{#if !team && s\.name !== currentSession && sessionHasNotification\(s\.name\)\}/u,
  );
  assert.match(
    source,
    /\{@const notice = terminalNotificationForWindow\(p\.session, p\.window\)\}/u,
  );
});
