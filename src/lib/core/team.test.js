import test from 'node:test';
import assert from 'node:assert/strict';

globalThis.$state = value => value;

const { teamState, isTeamSession, teamRoomOf, teamSessionOf, teamLabel, TEAM_PREFIX } =
  await import('./team.svelte.ts');

test('room/session mapping round-trips', () => {
  assert.equal(teamSessionOf('demo-abc123'), `${TEAM_PREFIX}demo-abc123`);
  assert.equal(teamRoomOf(teamSessionOf('demo-abc123')), 'demo-abc123');
});

test('classification is gated on the server actually having the bus', () => {
  teamState.available = false;
  assert.equal(isTeamSession('tmm-team-demo-abc123'), false); // busless server → ordinary session
  teamState.available = true;
  assert.equal(isTeamSession('tmm-team-demo-abc123'), true);
  assert.equal(isTeamSession('regular-session'), false);
  assert.equal(isTeamSession(''), false);
  assert.equal(isTeamSession(null), false);
  teamState.available = false;
});

test('teamLabel strips only a trailing 6-hex slug', () => {
  assert.equal(teamLabel('tmm-team-demo-abc123'), 'demo');
  assert.equal(teamLabel('tmm-team-my-app-0f3e2d'), 'my-app');
  // No slug → name kept whole (legacy rooms).
  assert.equal(teamLabel('tmm-team-main'), 'main');
  // Only exactly-6 hex chars count as a slug.
  assert.equal(teamLabel('tmm-team-demo-abcde'), 'demo-abcde');
  assert.equal(teamLabel('tmm-team-demo-abcdefg'), 'demo-abcdefg');
});
