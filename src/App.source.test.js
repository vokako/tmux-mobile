// Source-contract tests for App.svelte (see docs/conventions/testing.md):
// component wiring that node can't execute is pinned by matching the source.
// If one of these fails after an intentional change, update the assertion —
// the point is that the change must be INTENTIONAL.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./App.svelte', import.meta.url), 'utf8');

test('every authenticated connection path refreshes agent notifications', () => {
  const reconnectSuccess = source.match(/function onReconnectSuccess[\s\S]*?\n  \}/u)?.[0] || '';
  const manualSuccess = source.match(/function onConnected[\s\S]*?\n  \}/u)?.[0] || '';
  const optimizedConnection = source.match(/async function optimizeConnection[\s\S]*?\n  \}/u)?.[0] || '';
  const automaticConnection = source.match(/\$effect\(\(\) => \{\n    if \(autoConnectAttempted[\s\S]*?\n  \}\);/u)?.[0] || '';

  assert.match(reconnectSuccess, /syncAgentNotifications\(\)/u);
  assert.match(manualSuccess, /syncAgentNotifications\(\)/u);
  assert.match(optimizedConnection, /syncAgentNotifications\(\)/u);
  assert.match(automaticConnection, /syncAgentNotifications\(\)/u);
});

test('Terminal navigation and page layer exist without an active target', () => {
  assert.match(source, /const t = \['sessions', 'terminal'\]/u);
  assert.match(
    source,
    /<button tabindex="-1" class:active=\{page === 'terminal'[\s\S]*?\{t\('terminal'\)\}[\s\S]*?<\/button>/u,
  );
  assert.match(
    source,
    /<div class="page-layer" class:hidden=\{page !== 'terminal'\}>\s*\{#if terminalTarget\}/u,
  );
  assert.match(source, /\{:else\}\s*<div class="terminal-empty">/u);
});
