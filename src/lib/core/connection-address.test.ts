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


// Board #323: a page the gateway served defaults to that gateway, same origin.
test('a gateway-hosted page connects to its own origin, ws or wss by its protocol', () => {
  assert.equal(defaultConnectionAddress({ protocol: 'http:', host: '192.168.1.5:19899' }, false, true), 'ws://192.168.1.5:19899');
  assert.equal(defaultConnectionAddress({ protocol: 'https:', host: '[::1]:9899' }, false, true), 'wss://[::1]:9899');
  assert.equal(defaultConnectionAddress({ protocol: 'http:', host: 'devbox:5173' }, true, true), 'ws://devbox:5173', 'the marker wins over the Vite proxy rule');
  assert.equal(defaultConnectionAddress({ protocol: 'tauri:', host: 'localhost' }, false, true), STANDALONE_WS_DEFAULT, 'only an http(s) page');
});

test('the marker is a meta tag, read from the document', async () => {
  const { hostedByGateway } = await import('./connection-address.ts');
  assert.equal(hostedByGateway({ querySelector: (s) => (s === 'meta[name="tmm-gateway"]' ? {} : null) }), true);
  assert.equal(hostedByGateway({ querySelector: () => null }), false);
  assert.equal(hostedByGateway(undefined), false);
});

test('local autofill fills only what nothing saved and nobody changed, field by field', async () => {
  const { localAutofill } = await import('./connection-address.ts');
  const cfg = { url: 'wss://[::1]:9900', token: 'tok', tmux_socket: '/s' };
  const blank = { address: 'ws://127.0.0.1:9899', token: '', socket: '' };
  const none = { address: null, token: null, socket: null };
  assert.deepEqual(localAutofill(none, blank, blank, cfg), { address: 'wss://[::1]:9900', token: 'tok', socket: '/s' }, 'a fresh app takes the local_url');
  assert.deepEqual(
    localAutofill({ address: 'ws://phone-server:9899', token: null, socket: null }, blank, blank, cfg),
    { token: 'tok', socket: '/s' },
    'a saved address with NO token keeps its address',
  );
  const typed = { ...blank, address: 'ws://typing-meanwhile' };
  assert.deepEqual(localAutofill(none, blank, typed, cfg), { token: 'tok', socket: '/s' }, 'an address typed while the config loaded is kept');
  assert.deepEqual(localAutofill({ address: 'a', token: 't', socket: 's' }, blank, blank, cfg), {}, 'everything saved: nothing filled');
  assert.deepEqual(localAutofill(none, blank, blank, {}), {}, 'nothing to fill from');
});
