import assert from 'node:assert/strict';
import test from 'node:test';
import {
  externalWebUrlFromAnchor,
  handleExternalLinkClick,
  installExternalLinkHandler,
  isExternalWebUrl,
  openExternalUrl,
} from './external-links.ts';

// Deliberately-partial test doubles. One cast helper per shape keeps the
// intent visible: these mocks implement exactly what the code under test
// touches, nothing more.
const asAnchor = (a: { getAttribute: () => string; href: string }) => a as unknown as HTMLAnchorElement;
const asWindow = (w: object) => w as unknown as Window;
const asEvent = (e: object) => e as unknown as MouseEvent;

test('recognizes only absolute HTTP links as external web URLs', () => {
  assert.equal(isExternalWebUrl('https://example.com/path'), true);
  assert.equal(isExternalWebUrl('HTTP://example.com'), true);
  assert.equal(isExternalWebUrl('/relative/path'), false);
  assert.equal(isExternalWebUrl('javascript:alert(1)'), false);
});

test('uses the browser-resolved URL for relative Markdown links', () => {
  const anchor = asAnchor({
    getAttribute: () => '../guide',
    href: 'http://localhost:5173/guide',
  });
  assert.equal(externalWebUrlFromAnchor(anchor), 'http://localhost:5173/guide');
});

test('does not externalize non-web protocols', () => {
  const anchor = asAnchor({
    getAttribute: () => '#section',
    href: 'tauri://localhost/current#section',
  });
  assert.equal(externalWebUrlFromAnchor(anchor), null);
});

test('Tauri links use the system opener without window.open fallback', async () => {
  const calls: unknown[] = [];
  const windowRef = asWindow({
    __TAURI_INTERNALS__: {},
    open() { calls.push('window.open'); },
  });

  await openExternalUrl('https://example.com', {
    windowRef,
    loadTauriOpener: async () => ({
      async openUrl(url: string) { calls.push(['openUrl', url]); },
    }),
  });

  assert.deepEqual(calls, [['openUrl', 'https://example.com']]);
});

test('browser links open a separate noopener browsing context', async () => {
  const calls: unknown[] = [];
  const child: { opener: unknown } = { opener: {} };
  const windowRef = asWindow({
    open(...args: unknown[]) {
      calls.push(args);
      return child;
    },
  });

  await openExternalUrl('http://example.com', { windowRef });

  assert.deepEqual(calls, [['http://example.com', '_blank', 'noopener,noreferrer']]);
  assert.equal(child.opener, null);
});

test('delegated link clicks are prevented before opening externally', async () => {
  let prevented = false;
  const opened: string[] = [];
  const event = asEvent({
    target: {
      closest: () => ({
        getAttribute: () => 'https://example.com/docs',
        href: 'https://example.com/docs',
      }),
    },
    preventDefault() { prevented = true; },
  });

  const handled = await handleExternalLinkClick(event, {
    windowRef: asWindow({
      open(url: string) {
        opened.push(url);
        return null;
      },
    }),
  });

  assert.equal(handled, true);
  assert.equal(prevented, true);
  assert.deepEqual(opened, ['https://example.com/docs']);
});

test('middle clicks open externally while right auxiliary clicks are ignored', async () => {
  const opened: string[] = [];
  const event = (button: number) => asEvent({
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
    windowRef: asWindow({
      open(url: string) {
        opened.push(url);
        return null;
      },
    }),
  };

  assert.equal(await handleExternalLinkClick(event(1), runtime), true);
  assert.equal(await handleExternalLinkClick(event(2), runtime), false);
  assert.deepEqual(opened, ['https://example.com']);
});

test('delegated handlers subscribe and unsubscribe click plus auxclick', () => {
  type ListenerRecord = [string, unknown, boolean];
  const added: ListenerRecord[] = [];
  const removed: ListenerRecord[] = [];
  const root = {
    addEventListener(type: string, handler: unknown, capture: boolean) {
      added.push([type, handler, capture]);
    },
    removeEventListener(type: string, handler: unknown, capture: boolean) {
      removed.push([type, handler, capture]);
    },
  } as unknown as HTMLElement;

  const dispose = installExternalLinkHandler(root);
  dispose();

  assert.deepEqual(added.map(([type, , capture]) => [type, capture]), [
    ['click', true],
    ['auxclick', true],
  ]);
  assert.deepEqual(removed, added);
});
