import assert from 'node:assert/strict';
import test from 'node:test';
import { groupRoster } from './roster.ts';
import type { RosterGroup } from './roster.ts';

interface Member { name: string; team?: string | null }
const shape = (items: Array<Member | RosterGroup<Member>>): unknown[] => items.map((item) =>
  'items' in item ? { path: item.path, items: shape(item.items) } : item.name);

test('solo agents keep their order and identity, including an empty roster', () => {
  const members = [{ name: 'one', team: null }, { name: 'two', team: '' }];
  const grouped = groupRoster(members);
  assert.deepEqual(groupRoster([]), []);
  assert.deepEqual(shape(grouped), ['one', 'two']);
  assert.equal(grouped[0], members[0]);
  assert.equal(grouped[1], members[1]);
});

test('teams take their first member position without sorting the roster', () => {
  const grouped = groupRoster([
    { name: 'solo' }, { name: 'dev-a', team: 'dev' },
    { name: 'ops-a', team: 'ops' }, { name: 'dev-b', team: 'dev' }, { name: 'last' },
  ]);
  assert.deepEqual(shape(grouped), [
    'solo', { path: 'dev', items: ['dev-a', 'dev-b'] }, { path: 'ops', items: ['ops-a'] }, 'last',
  ]);
});

test('nested teams and direct members preserve first appearance at every level', () => {
  const reviewer = { name: 'review-a', team: 'dev/review' };
  const grouped = groupRoster<Member>([
    reviewer, { name: 'lead', team: 'dev' }, { name: 'solo' },
    { name: 'test-a', team: 'dev/test' }, { name: 'review-b', team: 'dev/review' },
  ]);
  assert.deepEqual(shape(grouped), [
    { path: 'dev', items: [
      { path: 'dev/review', items: ['review-a', 'review-b'] }, 'lead',
      { path: 'dev/test', items: ['test-a'] },
    ] }, 'solo',
  ]);
  const dev = grouped[0] as RosterGroup<Member>;
  const review = dev.items[0] as RosterGroup<Member>;
  assert.equal(review.team, 'review');
  assert.equal(review.items[0], reviewer);
});

test('same leaf names in different paths remain separate groups', () => {
  assert.deepEqual(shape(groupRoster([
    { name: 'a', team: 'dev/review' }, { name: 'b', team: 'ops/review' },
  ])), [
    { path: 'dev', items: [{ path: 'dev/review', items: ['a'] }] },
    { path: 'ops', items: [{ path: 'ops/review', items: ['b'] }] },
  ]);
});
