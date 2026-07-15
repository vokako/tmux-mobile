import test from 'node:test';
import assert from 'node:assert/strict';

globalThis.$state = value => value;
globalThis.window = { addEventListener() {} };

const { agentNotifications, sessionHasNotification } = await import('./agent-notifications.svelte.js');

test('session summary excludes the active window', () => {
  agentNotifications.unread = [
    { session: 'work', window: 2 },
    { session: 'other', window: 1 },
  ];

  assert.equal(sessionHasNotification('work'), true);
  assert.equal(sessionHasNotification('work', '2'), false);

  agentNotifications.unread.push({ session: 'work', window: 3 });
  assert.equal(sessionHasNotification('work', 2), true);
});
