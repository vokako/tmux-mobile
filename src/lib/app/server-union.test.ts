// One list over two servers (board #335 ②a-5), against the real ordering
// rules — the projects list's own `sortRows`/`projectUpdatedMs`, not a
// restatement of them. Nothing here touches a component: ②a's a5 is the
// projection and the ref outputs, and ②b wires the consumers.
import test from 'node:test';
import assert from 'node:assert/strict';
import { contributors, unionRows, type ServerSlice } from './server-union.ts';
import { issueRef, projectRef, refKey, sessionRef } from './refs.ts';
import { projectUpdatedMs, sortRows, type Project, type ProjectRow } from '../projects/projects.ts';

const project = (over: Partial<Project> & { id: string; session: string }): Project => ({
  name: over.session, path: `/srv/${over.session}`, adopted: false, autostart: false,
  created_at: 0, archived: false, ...over,
});
const row = (id: string, session: string, over: Partial<Project> & { live?: boolean } = {}): ProjectRow => {
  const { live = false, ...rest } = over;
  return { project: project({ id, session, ...rest }), slots: [], live };
};

const slice = <T,>(serverId: string, order: number, items: T[], over: Partial<ServerSlice<T>> = {}): ServerSlice<T> =>
  ({ serverId, name: serverId.toUpperCase(), order, ok: true, items, ...over });

/** The projects list's own rule, composed from its own functions. */
const projectSpec = (talk: Record<string, number> = {}) => ({
  ref: (serverId: string, r: ProjectRow) => projectRef(serverId, r.project.id),
  rank: (r: ProjectRow) => [r.live ? 1 : 0, projectUpdatedMs(r, talk)],
});

test('one server contributing is that server s list, untagged', () => {
  // The Single-mode shape: same order, no tag column, nothing to explain.
  const rows = [row('p1', 'alpha', { live: true }), row('p2', 'beta')];
  const union = unionRows([slice('a', 0, sortRows(rows))], projectSpec());
  assert.deepEqual(union.rows.map((r) => r.item.project.id), ['p1', 'p2']);
  assert.deepEqual(union.rows.map((r) => r.tag), [null, null], 'nothing to tell apart');
  assert.deepEqual(union.missing, []);
});

test('two servers interleave by the projects list s own rule', () => {
  // Open first, then by activity, newest first — and across servers, not
  // server by server. The rank comes from projects.ts's own functions.
  const talk = { 'proj:a-chat': 3000, 'proj:b-chat': 5000 };
  const a = [row('a1', 'a-open', { live: true }), row('a2', 'a-chat')];
  const b = [row('b1', 'b-open', { live: true }), row('b2', 'b-chat')];
  const union = unionRows(
    [slice('a', 0, sortRows(a, talk)), slice('b', 1, sortRows(b, talk))],
    projectSpec(talk),
  );
  assert.deepEqual(union.rows.map((r) => `${r.serverId}:${r.item.project.id}`),
    ['a:a1', 'b:b1', 'b:b2', 'a:a2'],
    'both open rows first, then the busier chat, whichever server it is on');
});

test('equal rows fall in saved-server order, stably', () => {
  // Two servers whose rows rank identically must not flicker with whichever
  // answered first; the registry order decides, every render.
  const a = [row('a1', 'same'), row('a2', 'same-too')];
  const b = [row('b1', 'same'), row('b2', 'same-too')];
  const first = unionRows([slice('b', 1, b), slice('a', 0, a)], projectSpec());
  const second = unionRows([slice('a', 0, a), slice('b', 1, b)], projectSpec());
  const ids = (u: typeof first) => u.rows.map((r) => `${r.serverId}:${r.item.project.id}`);
  assert.deepEqual(ids(first), ids(second), 'the slice argument order changes nothing');
  assert.deepEqual(ids(first), ['a:a1', 'a:a2', 'b:b1', 'b:b2']);
});

test('one server failing leaves the other s list whole', () => {
  // The failure a shorter list would hide. B is asked and fails: A's rows all
  // stay, and B is named so the view can say so.
  const a = [row('a1', 'alpha', { live: true })];
  const union = unionRows(
    [slice('a', 0, a), slice('b', 1, [] as ProjectRow[], { ok: false })],
    projectSpec(),
  );
  assert.deepEqual(union.rows.map((r) => r.item.project.id), ['a1']);
  assert.deepEqual(union.missing, ['b']);
  assert.deepEqual(union.rows.map((r) => r.tag), [null],
    'and with only one server answering there is still nothing to tag');
});

test('a server that has not answered yet is simply absent', () => {
  // Independent arrival: the list is complete for the servers that have
  // answered, and grows when the next one does.
  const a = [row('a1', 'alpha')];
  const b = [row('b1', 'beta')];
  const early = unionRows([slice('a', 0, a)], projectSpec());
  assert.deepEqual(early.rows.map((r) => r.serverId), ['a']);
  assert.deepEqual(early.missing, [], 'not answered yet is not a failure');
  const later = unionRows([slice('a', 0, a), slice('b', 1, b)], projectSpec());
  assert.deepEqual(later.rows.map((r) => r.serverId), ['a', 'b']);
});

test('same-named objects on two servers are different rows with different keys', () => {
  // The collision the whole phase exists for: a project called `app` on both
  // machines. Two rows, two keys, two refs — and each ref names its own
  // server, which is where ②b's row action gets its runtime.
  const a = [row('p1', 'app')];
  const b = [row('p1', 'app')];                       // same project id, too
  const union = unionRows([slice('a', 0, a), slice('b', 1, b)], projectSpec());
  assert.equal(union.rows.length, 2);
  const [first, second] = union.rows;
  assert.notEqual(first!.key, second!.key, 'the {#each} keys cannot collide');
  assert.equal(first!.key, refKey(projectRef('a', 'p1')));
  assert.equal(second!.ref.serverId, 'b');
  assert.deepEqual(union.rows.map((r) => r.tag), ['A', 'B'], 'and both rows say which server');
});

test('the tag appears only when more than one server contributes', () => {
  const a = [row('a1', 'alpha')];
  const b = [row('b1', 'beta')];
  // An empty but live slice still counts: its server is on screen, so the
  // list does not grow a tag the moment that server's first row arrives.
  const withEmpty = unionRows([slice('a', 0, a), slice('b', 1, [] as ProjectRow[])], projectSpec());
  assert.deepEqual(withEmpty.rows.map((r) => r.tag), ['A']);
  const both = unionRows([slice('a', 0, a), slice('b', 1, b)], projectSpec());
  assert.deepEqual(both.rows.map((r) => r.tag), ['A', 'B']);
});

test('without a rank each server s given order is kept', () => {
  // A list whose order IS the server's answer — tmux's session order. The
  // union must not reorder it, only interleave by server.
  const union = unionRows(
    [slice('a', 0, ['s2', 's1']), slice('b', 1, ['s9', 's3'])],
    { ref: (serverId, name: string) => sessionRef(serverId, name) },
  );
  assert.deepEqual(union.rows.map((r) => `${r.serverId}:${r.item}`),
    ['a:s2', 'a:s1', 'b:s9', 'b:s3']);
});

test('it projects any ref kind, with session-local ids kept apart', () => {
  // Board issue #3 of `app` on A is not #3 of `app` on B — the id is
  // session-local, so only the ref keeps them apart.
  const union = unionRows(
    [slice('a', 0, [3, 7]), slice('b', 1, [3])],
    { ref: (serverId, id: number) => issueRef(serverId, 'app', id) },
  );
  assert.equal(new Set(union.rows.map((r) => r.key)).size, 3, 'three distinct issues');
  assert.deepEqual(union.rows.map((r) => `${r.serverId}#${r.item}`), ['a#3', 'a#7', 'b#3']);
});

test('contributors are listed in saved-server order, failures included', () => {
  // What a per-server header or an "unreachable" notice reads.
  const slices = [
    slice('b', 1, [] as ProjectRow[], { ok: false }),
    slice('a', 0, [row('a1', 'alpha')]),
  ];
  assert.deepEqual(contributors(slices).map((s) => `${s.serverId}:${s.ok}`), ['a:true', 'b:false']);
});
