import test from 'node:test';
import assert from 'node:assert/strict';
import { createPersistedList } from './persisted-list.ts';

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
}

test('load fetches once and mirrors to onChange', async () => {
  let fetches = 0;
  let mirrored = null;
  const list = createPersistedList({
    fetch: async () => { fetches++; return ['a']; },
    persist: async () => {},
    onChange: (v) => { mirrored = v; },
  });
  await Promise.all([list.load(), list.load()]); // single-flight
  assert.equal(fetches, 1);
  assert.deepEqual(mirrored, ['a']);
  assert.equal(list.loaded, true);
});

test('rule 1: mutation before first load fetches, merges, then persists', async () => {
  const persisted: string[][] = [];
  const list = createPersistedList<string>({
    fetch: async () => ['server-item'],
    persist: async (v) => { persisted.push(v); },
    onChange: () => {},
  });
  const ok = await list.mutate(items => [...items, 'new']);
  assert.equal(ok, true);
  assert.deepEqual(list.items, ['server-item', 'new']);
  assert.deepEqual(persisted, [['server-item', 'new']]);
});

test('rule 1: failed first fetch skips the persist entirely', async () => {
  let persistCalls = 0;
  const list = createPersistedList({
    fetch: async () => { throw new Error('offline'); },
    persist: async () => { persistCalls++; },
    onChange: () => {},
  });
  const ok = await list.mutate(items => [...items, 'x']);
  assert.equal(ok, false);
  assert.equal(persistCalls, 0);
  assert.deepEqual(list.items, []);
});

test('rule 2: an in-flight refresh must not clobber a newer local mutation', async () => {
  const first = deferred<string[]>();
  const second = deferred<string[]>();
  let call = 0;
  const list = createPersistedList<string>({
    fetch: () => (++call === 1 ? first.promise : second.promise),
    persist: async () => {},
    onChange: () => {},
  });

  // First load completes normally.
  const initial = list.load();
  first.resolve(['a']);
  await initial;
  assert.deepEqual(list.items, ['a']);

  // A refresh starts (e.g. tab re-opened), and while its fetch is in
  // flight the user mutates locally. The mutation bumps the generation,
  // so the stale response must be discarded.
  const refresh = list.load();
  await list.mutate(items => [...items, 'local']);
  second.resolve(['stale-server']);
  await refresh;

  assert.deepEqual(list.items, ['a', 'local']);
});

test('persist failures are swallowed and do not roll back local state', async () => {
  const list = createPersistedList<string>({
    fetch: async () => [],
    persist: async () => { throw new Error('write failed'); },
    onChange: () => {},
  });
  const ok = await list.mutate(items => [...items, 'kept']);
  assert.equal(ok, true);
  await new Promise(r => setTimeout(r, 0)); // let the rejected persist settle
  assert.deepEqual(list.items, ['kept']);
});
