import test from 'node:test';
import assert from 'node:assert/strict';
import { readFileSync } from 'node:fs';
// @ts-expect-error a plain .mjs script
import { hostTriple, refuseForeignTarget } from './build-mac.mjs';

test('build:mac bundles the host tmm and refuses another architecture (#323)', () => {
  assert.equal(hostTriple('rustc 1.90.0\nhost: aarch64-apple-darwin\nrelease: 1.90.0\n'), 'aarch64-apple-darwin');
  assert.throws(() => hostTriple('rustc 1.90.0\n'));
  refuseForeignTarget([], 'aarch64-apple-darwin');
  refuseForeignTarget(['--target', 'aarch64-apple-darwin'], 'aarch64-apple-darwin');
  assert.throws(() => refuseForeignTarget(['--target', 'x86_64-apple-darwin'], 'aarch64-apple-darwin'), /not supported/u);
  assert.throws(() => refuseForeignTarget(['--target=universal-apple-darwin'], 'aarch64-apple-darwin'), /not supported/u);
  const pkg = JSON.parse(readFileSync(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(pkg.scripts['build:mac'], 'node scripts/build-mac.mjs');
  const side = JSON.parse(readFileSync(new URL('../src-tauri/tauri.bundle.conf.json', import.meta.url), 'utf8'));
  assert.deepEqual(side.bundle.externalBin, ['binaries/tmm'], 'Tauri copies binaries/tmm-<triple> to Contents/MacOS/tmm');
  const main = JSON.parse(readFileSync(new URL('../src-tauri/tauri.conf.json', import.meta.url), 'utf8'));
  assert.equal(main.bundle.externalBin, undefined, 'dev and other builds never need the sidecar');
});
