import test from 'node:test';
import assert from 'node:assert/strict';

globalThis.$state = value => value;
globalThis.window = { addEventListener() {} };

const { agentNotifications, otherSessionHasNotification, sessionHasNotification } = await import('./agent-notifications.svelte.js');

test('session and cross-session summaries stay distinct', () => {
  agentNotifications.unread = [
    { session: 'work', window: 2 },
    { session: 'other', window: 1 },
  ];

  assert.equal(sessionHasNotification('work'), true);
  assert.equal(sessionHasNotification('work', '2'), false);
  assert.equal(otherSessionHasNotification('work'), true);
  assert.equal(otherSessionHasNotification('other'), true);

  agentNotifications.unread = [{ session: 'work', window: 3 }];
  assert.equal(sessionHasNotification('work', 2), true);
  assert.equal(otherSessionHasNotification('work'), false);
});
