import test from 'node:test';
import assert from 'node:assert/strict';
import { detectPlatform, hasTauriInternals, tauriReady } from './platform.ts';

const ANDROID_UA = 'Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36 Chrome/124.0 Mobile Safari/537.36';
const MAC_UA = 'Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 Safari/605.1.15';

test('a browser is neither Tauri nor Android-app, whatever the user agent says', () => {
  // A phone's Chrome opening the PWA has an Android UA and no bridge: it must
  // take the browser paths (service worker, window.open), not the app's.
  assert.deepEqual(detectPlatform({}, { userAgent: ANDROID_UA }), { isAndroid: false, isTauri: false, isTauriDesktop: false });
  assert.deepEqual(detectPlatform({}, { userAgent: MAC_UA }), { isAndroid: false, isTauri: false, isTauriDesktop: false });
  assert.deepEqual(detectPlatform(null, null), { isAndroid: false, isTauri: false, isTauriDesktop: false });
});

test('the Android shell is Tauri AND Android, never desktop', () => {
  // Rule 3's "isAndroid before isTauri": Android IS a Tauri shell, so a bare
  // isTauri branch runs on the phone too. The flags make that explicit —
  // isAndroid implies isTauri, and excludes isTauriDesktop.
  const f = detectPlatform({ __TAURI_INTERNALS__: {} }, { userAgent: ANDROID_UA });
  assert.deepEqual(f, { isAndroid: true, isTauri: true, isTauriDesktop: false });
});

test('the desktop shell is Tauri and desktop, not Android', () => {
  const f = detectPlatform({ __TAURI_INTERNALS__: {} }, { userAgent: MAC_UA });
  assert.deepEqual(f, { isAndroid: false, isTauri: true, isTauriDesktop: true });
});

test('either marker global counts as the bridge', () => {
  // `withGlobalTauri` exposes __TAURI__; the IPC layer always sets
  // __TAURI_INTERNALS__. Either alone means "inside the shell".
  assert.equal(hasTauriInternals({ __TAURI__: {} }), true);
  assert.equal(hasTauriInternals({ __TAURI_INTERNALS__: {} }), true);
  assert.equal(hasTauriInternals({}), false);
  assert.equal(hasTauriInternals(null), false);
  assert.equal(hasTauriInternals(undefined), false);
});

test('tauriReady is a settled promise callers can always await', async () => {
  assert.ok(tauriReady instanceof Promise);
  assert.equal(await tauriReady, undefined);
});
