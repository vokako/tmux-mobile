import assert from 'node:assert/strict';
import test from 'node:test';
import { walkFeedGap } from './hub-history.ts';
import { mergeMessages } from './hub.ts';

const message = (seq: number) => ({ id: `m${seq}`, seq, ts: seq * 10, body: `body ${seq}` });
type Message = ReturnType<typeof message>;
const request = { session: 'original', floorTs: 30, cursor: 8 };
const page = (seqs: number[], oldest_seq: number, has_more = true) => ({
  messages: seqs.map(message), oldest_seq, has_more,
});

test('the gap walk merges pages in read order and reuses the existing id dedupe', async () => {
  const pages = new Map([[8, page([6, 7], 6)], [6, page([4, 5], 4)], [4, page([2, 3], 2)]]);
  const trace: string[] = [];
  let feed = [message(3), message(7)];
  const existing = feed[1];
  const completed = await walkFeedGap(request, {
    readPage: async (session, cursor) => {
      trace.push(`read ${session} ${cursor}`);
      assert.ok(pages.has(cursor));
      return pages.get(cursor);
    },
    stillCurrent: (session) => { trace.push(`check ${session}`); return true; },
    mergePage: (messages) => {
      trace.push(`merge ${messages.map((m) => m.seq).join(',')}`);
      feed = mergeMessages(feed, messages);
    },
  });
  assert.equal(completed, true);
  assert.deepEqual(feed.map((m) => m.seq), [3, 4, 5, 6, 7]);
  assert.equal(feed.at(-1), existing, 'a page overlap keeps the existing message object');
  assert.deepEqual(trace, [
    'read original 8', 'check original', 'merge 6,7',
    'read original 6', 'check original', 'merge 4,5',
    'read original 4', 'check original',
  ]);
});

test('reaching the captured timestamp keeps only newer rows and stops early', async () => {
  let reads = 0;
  const merged: Message[][] = [];
  assert.equal(await walkFeedGap(request, {
    readPage: async () => { reads++; return page([2, 3, 4], 2); },
    stillCurrent: () => true,
    mergePage: (messages) => { merged.push(messages); },
  }), true);
  assert.equal(reads, 1);
  assert.deepEqual(merged, [[message(4)]]);
});

test('empty or absent pages stop without a merge, even with a remaining cursor', async () => {
  for (const empty of [null, undefined, {}, page([], 7)]) {
    let reads = 0;
    assert.equal(await walkFeedGap<Message>(request, {
      readPage: async () => { reads++; return empty; },
      stillCurrent: () => true,
      mergePage: () => assert.fail('an empty page must not merge'),
    }), true);
    assert.equal(reads, 1);
  }
});

test('no older page or no next cursor ends after merging the new rows', async () => {
  for (const last of [page([7], 7, false), { messages: [message(7)], has_more: true }]) {
    let reads = 0;
    const merged: Message[][] = [];
    assert.equal(await walkFeedGap(request, {
      readPage: async () => { reads++; return last; },
      stillCurrent: () => true,
      mergePage: (messages) => { merged.push(messages); },
    }), true);
    assert.equal(reads, 1);
    assert.deepEqual(merged, [[message(7)]]);
  }
});

test('a missing starting cursor performs no I/O', async () => {
  for (const cursor of [0, null, undefined]) {
    assert.equal(await walkFeedGap({ ...request, cursor }, {
      readPage: async () => assert.fail('no cursor to read'),
      stillCurrent: () => assert.fail('no awaited page to validate'),
      mergePage: () => assert.fail('no page to merge'),
    }), true);
  }
});

test('the walk stops at exactly 50 pages even if the server repeats a cursor', async () => {
  let reads = 0;
  let merges = 0;
  assert.equal(await walkFeedGap(request, {
    readPage: async (session, cursor) => {
      assert.equal(session, request.session);
      assert.equal(cursor, request.cursor);
      reads++;
      assert.ok(reads <= 51, 'a missing bound must not hang the test');
      return page([7], 8);
    },
    stillCurrent: () => true,
    mergePage: () => { merges++; },
  }), true);
  assert.equal(reads, 50);
  assert.equal(merges, 50);
});

test('a room switch at any page await discards that response and stops the walk', async () => {
  for (const stopAt of [1, 2, 3]) {
    let selected = request.session;
    const reads: Array<{ session: string; cursor: number }> = [];
    const replies: Array<(value: ReturnType<typeof page>) => void> = [];
    const merged: Message[][] = [];
    const walk = walkFeedGap<Message>(request, {
      readPage: (session, cursor) => {
        reads.push({ session, cursor });
        return new Promise((resolve) => { replies.push(resolve); });
      },
      stillCurrent: (session) => session === selected,
      mergePage: (messages) => { merged.push(messages); },
    });
    for (let i = 0; i < stopAt; i++) {
      assert.equal(reads.length, i + 1);
      if (i + 1 === stopAt) selected = 'other';
      replies[i]!(page([7 - i], 7 - i, i < 2));
      await Promise.resolve();
    }
    assert.equal(await walk, false);
    assert.deepEqual(reads, Array.from({ length: stopAt }, (_, i) => ({
      session: 'original', cursor: 8 - i,
    })));
    assert.deepEqual(merged, Array.from({ length: stopAt - 1 }, (_, i) => [message(7 - i)]));
  }
});

test('read failures propagate without undoing pages already merged', async () => {
  const failure = new Error('read failed');
  const merged: Message[][] = [];
  let reads = 0;
  await assert.rejects(walkFeedGap(request, {
    readPage: async () => {
      if (++reads === 2) throw failure;
      return page([7], 7);
    },
    stillCurrent: () => true,
    mergePage: (messages) => { merged.push(messages); },
  }), failure);
  assert.equal(reads, 2);
  assert.deepEqual(merged, [[message(7)]]);
});
