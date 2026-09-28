import assert from 'node:assert/strict';
import test from 'node:test';
import { groupActions, groupScope, runGroup } from './group-actions.ts';

const agents = [
  { name: 'lead', managed: true, team: 'squad', state: 'running' },
  { name: 'dev', managed: true, team: 'squad/sub', state: 'idle' },
  { name: 'solo', managed: true, team: null, state: 'waiting' },
  { name: 'other', managed: true, team: 'ops', state: 'idle' },
  { name: 'shell', managed: false, team: null, state: 'shell' },
];
const stopped = ['qa', 'loner', 'ops2'];
const stoppedTeams = { qa: 'squad/sub', ops2: 'ops' };
const verbs = (target: string, a = agents, s = stopped) =>
  groupActions(groupScope(target, a, s, stoppedTeams)).map((x) => `${x.verb}:${x.names.join(',')}:${x.confirm ? 'ask' : 'go'}`);

test('All covers every managed agent and every stopped identity (#258)', () => {
  assert.deepEqual(verbs('all'), [
    'interrupt:lead,solo:go',
    'restart:lead,dev,solo,other:ask',
    'start:qa,loner,ops2:go',
    'stop:lead,dev,solo,other:ask',
  ]);
});

test('a team covers only its members, nested sub-teams included, stopped ones by their recipe team', () => {
  assert.deepEqual(verbs('team:squad'), [
    'interrupt:lead:go',
    'restart:lead,dev:ask',
    'start:qa:go',
    'stop:lead,dev:ask',
  ]);
  assert.deepEqual(verbs('team:ops'), ['restart:other:go', 'start:ops2:go', 'stop:other:ask']);
});

test('a restart asks only when it would cut a turn; a stop always asks', () => {
  const idle = agents.map((a) => ({ ...a, state: 'idle' }));
  assert.deepEqual(verbs('all', idle, []), ['restart:lead,dev,solo,other:go', 'stop:lead,dev,solo,other:ask']);
});

test('a verb with nobody to act on is not offered', () => {
  assert.deepEqual(verbs('all', [], ['qa']), ['start:qa:go'], 'all stopped: only Start stopped');
  assert.deepEqual(verbs('team:none', agents, stopped), [], 'an unknown team has no verbs');
  const acts = groupActions(groupScope('all', agents, []));
  assert.ok(!acts.some((a) => (a.verb as string) === 'remove'), 'remove is never a group verb');
  assert.equal(acts.find((a) => a.verb === 'stop')?.danger, true);
});

test('runGroup tries every member once and reports only the failures, in order', async () => {
  const calls: string[] = [];
  const failed = await runGroup(['a', 'b', 'c', 'd'], async (name) => {
    calls.push(name);
    if (name === 'b' || name === 'd') throw new Error(`no ${name}`);
  });
  assert.deepEqual(failed, ['b', 'd']);
  assert.deepEqual(calls.sort(), ['a', 'b', 'c', 'd'], 'a failure does not stop the others; nothing is retried');
  assert.deepEqual(await runGroup([], async () => {}), []);
});
