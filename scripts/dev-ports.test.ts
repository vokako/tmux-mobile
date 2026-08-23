import assert from 'node:assert/strict';
import test from 'node:test';
import {
  DEV_SERVER_PORT_ENV,
  devServerPort,
  devServerTargets,
} from './dev-ports.mjs';

test('dev server defaults to the loopback 9899 target', () => {
  assert.equal(devServerPort({}), 9899);
  assert.deepEqual(devServerTargets({}), {
    ws: 'ws://127.0.0.1:9899',
    http: 'http://127.0.0.1:9899',
  });
});

test('the dedicated internal-port setting wins over the server PORT fallback', () => {
  const env = { PORT: '19000', [DEV_SERVER_PORT_ENV]: '19001' };
  assert.equal(devServerPort(env), 19001);
  assert.equal(devServerTargets(env).ws, 'ws://127.0.0.1:19001');
});

test('invalid internal ports fail before either dev service starts', () => {
  assert.throws(() => devServerPort({ [DEV_SERVER_PORT_ENV]: 'not-a-port' }), /invalid internal dev server port/);
  assert.throws(() => devServerPort({ PORT: '70000' }), /invalid internal dev server port/);
});
