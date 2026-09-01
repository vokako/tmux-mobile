// Source-contract tests for PanePicker.svelte (see docs/conventions/testing.md).
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./PanePicker.svelte', import.meta.url), 'utf8');

test('the retired unread-notification dots stay retired (2026-09-01)', () => {
  assert.doesNotMatch(source, /agent-notifications\.svelte/u);
  assert.doesNotMatch(source, /picker-attention|sessionHasNotification|NotificationForWindow/u);
});
