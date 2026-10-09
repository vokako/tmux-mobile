// What the Files pages remember about one server, and that two servers
// remember separately (board #335 ②a-4).
//
// These rules used to live in Files.svelte's `<script module>`, where only a
// mount test could reach them. As a factory they are directly testable, and
// `Files.mount.test.ts` passing unchanged is the evidence the component still
// behaves the same.
import test from 'node:test';
import assert from 'node:assert/strict';
import { createFilesMemory } from './files-memory.ts';

test('two servers park the same session name separately', () => {
  // The collision this exists for: a project called `app` on both machines.
  // Keying by session name alone made returning to one server show the
  // other's directory.
  const a = createFilesMemory();
  const b = createFilesMemory();
  a.park('app', { cwd: '/srv/a/app', sourceDir: '/srv/a' });
  b.park('app', { cwd: '/srv/b/app', sourceDir: '/srv/b' });
  assert.deepEqual(a.position('app'), { cwd: '/srv/a/app', sourceDir: '/srv/a' });
  assert.deepEqual(b.position('app'), { cwd: '/srv/b/app', sourceDir: '/srv/b' });
  assert.equal(a.position('other'), undefined, 'a first visit inherits nothing');

  // Dropping one server's positions leaves the other's.
  a.reset();
  assert.equal(a.position('app'), undefined);
  assert.deepEqual(b.position('app'), { cwd: '/srv/b/app', sourceDir: '/srv/b' });
});

test('the one-writer rule holds across instances of one server, not across two', () => {
  // Board #305: the page and the drawer write the same part folder, so one
  // part has one writer. On two servers the same path yields the same part id
  // only if the machine id is the same — and when it is not, the two attempts
  // are different files and must not block each other.
  const a = createFilesMemory();
  const b = createFilesMemory();
  const page = {};
  const drawer = {};
  a.claim('part-1', { owner: page, adopt() {} });

  assert.equal(a.writerOf('part-1')?.owner, page, 'the drawer sees the page s claim');
  assert.equal(b.writerOf('part-1'), undefined, 'the other server has no claim on it');
  b.claim('part-1', { owner: drawer, adopt() {} });
  assert.equal(a.writerOf('part-1')?.owner, page, 'and claiming there did not take A s');

  a.release('part-1');
  assert.equal(a.writerOf('part-1'), undefined);
  assert.equal(b.writerOf('part-1')?.owner, drawer, 'releasing A s left B s');
});

test('a suspend stops this server s attempts and waits for them', () => {
  // The rule a switch depends on: every attempt is aborted with the switch's
  // reason, and the promise resolves only once each has SETTLED, so no retry
  // or re-sign of the old server's chain goes out after the socket moves.
  const a = createFilesMemory();
  const reasons: unknown[] = [];
  const settle: (() => void)[] = [];
  for (const id of ['r1', 'r2']) {
    const control = new AbortController();
    control.signal.addEventListener('abort', () => reasons.push(control.signal.reason));
    a.track(id, control, new Promise<void>((resolve) => settle.push(() => resolve())));
  }
  let done = false;
  const suspending = a.suspend('switching').then(() => { done = true; });

  assert.deepEqual(reasons, ['switching', 'switching'], 'both were aborted with the switch s reason');
  return Promise.resolve().then(() => {
    assert.equal(done, false, 'and it is still waiting for them to settle');
    settle[0]!();
    return Promise.resolve();
  }).then(() => {
    assert.equal(done, false, 'one settled is not all settled');
    settle[1]!();
    return suspending;
  }).then(() => {
    assert.equal(done, true);
  });
});

test('a suspend on one server does not touch the other s attempts', () => {
  const a = createFilesMemory();
  const b = createFilesMemory();
  const aborted: string[] = [];
  const track = (m: ReturnType<typeof createFilesMemory>, id: string) => {
    const control = new AbortController();
    control.signal.addEventListener('abort', () => aborted.push(id));
    m.track(id, control, Promise.resolve());
    return control;
  };
  track(a, 'a-row');
  const bControl = track(b, 'b-row');

  return a.suspend('switching').then(() => {
    assert.deepEqual(aborted, ['a-row'], 'B s transfer kept running');
    assert.equal(bControl.signal.aborted, false);
  });
});

test('an untracked attempt is not aborted twice, and a cancel is per row', () => {
  const a = createFilesMemory();
  const aborted: string[] = [];
  for (const id of ['r1', 'r2']) {
    const control = new AbortController();
    control.signal.addEventListener('abort', () => aborted.push(id));
    a.track(id, control, Promise.resolve());
  }
  a.abort('r1');
  assert.deepEqual(aborted, ['r1'], 'Cancel stops one row');
  a.untrack('r1');
  return a.suspend('switching').then(() => {
    assert.deepEqual(aborted, ['r1', 'r2'], 'the finished row is not aborted again');
    a.abort('gone');                 // an unknown row is a no-op, not a throw
  });
});

test('browser row ids are unique within a record', () => {
  // They need to be unique in the record that holds them and nowhere else:
  // two servers' rows live in two records.
  const a = createFilesMemory();
  const b = createFilesMemory();
  const ids = [a.nextWebRowId(), a.nextWebRowId(), a.nextWebRowId()];
  assert.equal(new Set(ids).size, 3, `${ids.join(' ')}`);
  assert.equal(b.nextWebRowId(), ids[0], 'and each record counts for itself');
});
