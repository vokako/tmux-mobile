import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../../App.svelte', import.meta.url), 'utf8');

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
