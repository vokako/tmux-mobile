import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Sessions.svelte', import.meta.url), 'utf8');

test('Sessions imports the notification state used by its template', () => {
  assert.match(
    source,
    /import\s*\{[^}]*agentNotifications[^}]*sessionHasNotification[^}]*\}\s*from\s*['"]\.\.\/core\/agent-notifications\.svelte\.ts['"]/s,
  );
});
