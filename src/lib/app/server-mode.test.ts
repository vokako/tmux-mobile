import test from 'node:test';
import assert from 'node:assert/strict';
import { localServerLine } from './server-mode.ts';

const t = (k: string) => k;
test('the This-computer line names the local start mode, never the connection (#323)', () => {
  assert.equal(localServerLine(null, t), null, 'not the desktop app');
  assert.deepEqual(localServerLine({ mode: 'gateway', url: 'ws://127.0.0.1:19977' }, t), { label: 'localServer', value: 'localServerGateway · ws://127.0.0.1:19977' });
  assert.deepEqual(localServerLine({ mode: 'embedded', url: 'ws://127.0.0.1:9899' }, t), { label: 'localServer', value: 'localServerEmbedded · ws://127.0.0.1:9899' });
  assert.equal(localServerLine({ mode: 'starting', url: 'u' }, t)!.tone, 'warn');
  assert.deepEqual(localServerLine({ mode: 'occupied', url: 'u', reason: 'u is another machine\u2019s gateway' }, t), { label: 'localServer', value: 'localServerOccupied · u is another machine\u2019s gateway', tone: 'warn' });
  assert.equal(localServerLine({ mode: 'failed', url: 'u', reason: 'Address in use' }, t)!.tone, 'danger');
  assert.equal(localServerLine({ mode: 'later', url: 'u' }, t), null, 'an unknown mode says nothing');
});
