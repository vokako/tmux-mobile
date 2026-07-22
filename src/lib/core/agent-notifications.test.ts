import test from 'node:test';
import assert from 'node:assert/strict';

(globalThis as any).$state = (value: unknown) => value;
(globalThis as any).window = { addEventListener() {} };

const { agentNotifications, otherSessionHasNotification, sessionHasNotification, terminalNotificationForWindow, notificationForWindow, otherTerminalSessionHasNotification } = await import('./agent-notifications.svelte.ts');
const { teamState } = await import('./team.svelte.ts');

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

test('Terminal queries retain data but suppress Team sessions', () => {
  // Was a source-regex assertion (terminal-team-notifications.test.js);
  // now tests the actual behavior. Team classification is gated on the
  // server having the bus — flip it on for the test.
  teamState.available = true;
  agentNotifications.unread = [
    { session: 'tmm-team-demo-abc123', window: 1 },
    { session: 'work', window: 2 },
  ];

  // Team-session dots are suppressed in Terminal chrome…
  assert.equal(terminalNotificationForWindow('tmm-team-demo-abc123', 1), null);
  // …but the underlying data is retained (the Team tab still consumes it).
  assert.ok(notificationForWindow('tmm-team-demo-abc123', 1));
  // Regular sessions pass through.
  assert.ok(terminalNotificationForWindow('work', 2));

  // Cross-session summary: Team sessions are ignored as SOURCES of dots,
  // and viewing FROM a Team session suppresses the summary entirely
  // (Terminal chrome shows no Team-related dots in either direction).
  assert.equal(otherTerminalSessionHasNotification('work'), false);
  assert.equal(otherTerminalSessionHasNotification('tmm-team-demo-abc123'), false);

  teamState.available = false;
  agentNotifications.unread = [];
});
