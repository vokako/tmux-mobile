import test from 'node:test';
import assert from 'node:assert/strict';
import { LruCache } from './lru.ts';

test('LruCache evicts ONE least-recently-used entry past its bound, never the whole set', () => {
  const c = new LruCache<string, number>(3);
  c.set('a', 1).set('b', 2).set('c', 3);
  assert.equal(c.size, 3);
  c.set('d', 4);
  assert.equal(c.size, 3, 'bounded');
  assert.deepEqual([...c.keys()], ['b', 'c', 'd'], 'the oldest went, the rest stayed');
});

test('a hit refreshes the entry, so a hot key survives a cold stream', () => {
  const c = new LruCache<string, number>(2);
  c.set('hot', 1).set('x', 2);
  assert.equal(c.get('hot'), 1);
  c.set('y', 3);
  assert.deepEqual([...c.keys()], ['hot', 'y'], 'x was the least recently used, not hot');
  // Re-setting an existing key updates and refreshes without growing.
  c.set('hot', 10);
  assert.equal(c.size, 2);
  assert.equal(c.get('hot'), 10);
  assert.deepEqual([...c.keys()], ['y', 'hot']);
});

test('misses, deletes and a falsy value behave like a Map', () => {
  const c = new LruCache<string, string>(2);
  assert.equal(c.get('nope'), undefined);
  assert.equal(c.has('nope'), false);
  c.set('empty', '');
  assert.equal(c.get('empty'), '', 'an empty string is a stored value, not a miss');
  assert.equal(c.delete('empty'), true);
  assert.equal(c.size, 0);
  assert.throws(() => new LruCache(0), RangeError);
});
