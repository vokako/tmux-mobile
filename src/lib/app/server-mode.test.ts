import test from 'node:test';
import assert from 'node:assert/strict';
import { localServerLine, newerMode } from './server-mode.ts';

const t = (k: string) => k;
test('the This-computer line names the local start mode, never the connection (#323)', () => {
  assert.equal(localServerLine(null, t), null, 'not the desktop app');
  assert.deepEqual(localServerLine({ mode: 'gateway', url: 'ws://127.0.0.1:19977' }, t), { label: 'localServer', value: 'localServerGateway · ws://127.0.0.1:19977' });
  assert.deepEqual(localServerLine({ mode: 'embedded', url: 'ws://127.0.0.1:9899' }, t), { label: 'localServer', value: 'localServerEmbedded · ws://127.0.0.1:9899' });
  assert.deepEqual(localServerLine({ mode: 'starting', url: 'u' }, t), { label: 'localServer', value: 'localServerStarting · u' }, 'starting is achromatic');
  assert.deepEqual(localServerLine({ mode: 'occupied', url: 'u', reason: 'u is another machine\u2019s gateway' }, t), { label: 'localServer', value: 'localServerOccupied · u is another machine\u2019s gateway', tone: 'warn' });
  assert.equal(localServerLine({ mode: 'failed', url: 'u', reason: 'Address in use' }, t)!.tone, 'danger');
  assert.equal(localServerLine({ mode: 'later', url: 'u' }, t), null, 'an unknown mode says nothing');
});

test('newerMode keeps only a strictly newer seq, whatever the arrival order (#323 review)', () => {
  const at = (mode: string, seq: number) => ({ mode, url: 'u', gen: 1, seq });
  // Normal order.
  let cur = newerMode(null, at('starting', 1));
  cur = newerMode(cur, at('embedded', 2));
  assert.equal(cur!.mode, 'embedded');
  // failed published (3) before an older embedded (2) arrives.
  cur = newerMode(newerMode(cur, at('failed', 3)), at('embedded', 2));
  assert.equal(cur!.mode, 'failed', 'an old event never revives');
  // The initial read lands late and is older than an event.
  assert.equal(newerMode(at('failed', 3), at('starting', 1))!.mode, 'failed', 'an old read never rolls back');
  // The read is newer than an event that arrived first.
  assert.equal(newerMode(at('starting', 1), at('failed', 3))!.mode, 'failed', 'a newer read is not dropped');
  assert.equal(newerMode(at('failed', 3), at('failed', 3))!.seq, 3, 'equal is not newer');
  assert.equal(newerMode(at('starting', 1), { mode: 'embedded', url: 'u' })!.mode, 'starting', 'no seq is never newer');
});
