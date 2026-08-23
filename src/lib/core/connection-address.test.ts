import assert from 'node:assert/strict';
import test from 'node:test';
import { defaultConnectionAddress, STANDALONE_WS_DEFAULT } from './connection-address.ts';

test('browser development uses the page host and one /ws entry point', () => {
  assert.equal(
    defaultConnectionAddress({ protocol: 'http:', host: 'devbox:5173' }, true),
    'ws://devbox:5173/ws',
  );
  assert.equal(
    defaultConnectionAddress({ protocol: 'https:', host: 'devbox.example' }, true),
    'wss://devbox.example/ws',
  );
});

test('standalone and non-browser defaults remain compatible', () => {
  assert.equal(defaultConnectionAddress({ protocol: 'file:', host: '' }, true), STANDALONE_WS_DEFAULT);
  assert.equal(defaultConnectionAddress({ protocol: 'http:', host: 'devbox:5173' }, false), STANDALONE_WS_DEFAULT);
});
