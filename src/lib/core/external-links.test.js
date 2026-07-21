import assert from 'node:assert/strict';
import test from 'node:test';
import {
  externalWebUrlFromAnchor,
  handleExternalLinkClick,
  installExternalLinkHandler,
  isExternalWebUrl,
  openExternalUrl,
} from './external-links.ts';

test('recognizes only absolute HTTP links as external web URLs', () => {
  assert.equal(isExternalWebUrl('https://example.com/path'), true);
  assert.equal(isExternalWebUrl('HTTP://example.com'), true);
  assert.equal(isExternalWebUrl('/relative/path'), false);
  assert.equal(isExternalWebUrl('javascript:alert(1)'), false);
});

test('uses the browser-resolved URL for relative Markdown links', () => {
  const anchor = {
    getAttribute: () => '../guide',
    href: 'http://localhost:5173/guide',
  };
  assert.equal(externalWebUrlFromAnchor(anchor), 'http://localhost:5173/guide');
});

test('does not externalize non-web protocols', () => {
  const anchor = {
    getAttribute: () => '#section',
    href: 'tauri://localhost/current#section',
  };
  assert.equal(externalWebUrlFromAnchor(anchor), null);
});

test('Tauri links use the system opener without window.open fallback', async () => {
  const calls = [];
  const windowRef = {
    __TAURI_INTERNALS__: {},
    open() { calls.push('window.open'); },
  };

  await openExternalUrl('https://example.com', {
    windowRef,
    loadTauriOpener: async () => ({
      openUrl(url) { calls.push(['openUrl', url]); },
    }),
  });

  assert.deepEqual(calls, [['openUrl', 'https://example.com']]);
});

test('browser links open a separate noopener browsing context', async () => {
  const calls = [];
  const child = { opener: {} };
  const windowRef = {
    open(...args) {
      calls.push(args);
      return child;
    },
  };

  await openExternalUrl('http://example.com', { windowRef });

  assert.deepEqual(calls, [['http://example.com', '_blank', 'noopener,noreferrer']]);
  assert.equal(child.opener, null);
});

test('delegated link clicks are prevented before opening externally', async () => {
  let prevented = false;
  const opened = [];
  const event = {
    target: {
      closest: () => ({
        getAttribute: () => 'https://example.com/docs',
        href: 'https://example.com/docs',
      }),
    },
    preventDefault() { prevented = true; },
  };

  const handled = await handleExternalLinkClick(event, {
    windowRef: {
      open(url) {
        opened.push(url);
        return null;
      },
    },
  });

  assert.equal(handled, true);
  assert.equal(prevented, true);
  assert.deepEqual(opened, ['https://example.com/docs']);
});

test('middle clicks open externally while right auxiliary clicks are ignored', async () => {
  const opened = [];
  const event = (button) => ({
    type: 'auxclick',
    button,
    target: {
      closest: () => ({
        getAttribute: () => 'https://example.com',
        href: 'https://example.com',
      }),
    },
    preventDefault() {},
  });
  const runtime = {
    windowRef: {
      open(url) {
        opened.push(url);
        return null;
      },
    },
  };

  assert.equal(await handleExternalLinkClick(event(1), runtime), true);
  assert.equal(await handleExternalLinkClick(event(2), runtime), false);
  assert.deepEqual(opened, ['https://example.com']);
});

test('delegated handlers subscribe and unsubscribe click plus auxclick', () => {
  const added = [];
  const removed = [];
  const root = {
    addEventListener(type, handler, capture) {
      added.push([type, handler, capture]);
    },
    removeEventListener(type, handler, capture) {
      removed.push([type, handler, capture]);
    },
  };

  const dispose = installExternalLinkHandler(root);
  dispose();

  assert.deepEqual(added.map(([type, , capture]) => [type, capture]), [
    ['click', true],
    ['auxclick', true],
  ]);
  assert.deepEqual(removed, added);
});
