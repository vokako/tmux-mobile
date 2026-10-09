#!/usr/bin/env node
// `npm run build:mac` (board #323): the desktop app WITH the release `tmm`
// inside it, at TmuxMobile.app/Contents/MacOS/tmm.
//
// Tauri's `bundle.externalBin` copies `binaries/tmm-<target triple>` into
// the bundle as `tmm`. The sidecar is only needed here, so it lives in
// `tauri.bundle.conf.json`, merged by `--config`: `tauri dev` and other
// builds never look for it. The `tmm` built is the gateway's (`embed-ui`,
// no GUI), so `tmm gateway` serves the web UI from inside the app.
//
// Only the HOST triple is built and bundled. A `--target` for another
// architecture is refused rather than shipping a sidecar of the wrong one.
import { execFileSync } from 'node:child_process';
import { copyFileSync, chmodSync, mkdirSync } from 'node:fs';
import { join, dirname } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = join(dirname(fileURLToPath(import.meta.url)), '..');
const tauri = join(root, 'src-tauri');
const run = (cmd, args) => execFileSync(cmd, args, { cwd: root, stdio: 'inherit' });

export function hostTriple(rustcVV) {
  const m = /^host: (\S+)$/mu.exec(rustcVV);
  if (!m) throw new Error('rustc -vV printed no host triple');
  return m[1];
}

export function refuseForeignTarget(args, host) {
  const i = args.findIndex((a) => a === '--target' || a.startsWith('--target='));
  if (i < 0) return;
  const t = args[i].includes('=') ? args[i].split('=')[1] : args[i + 1];
  if (t && t !== host) throw new Error(`build:mac bundles the host's tmm (${host}); --target ${t} is not supported`);
}

if (process.argv[1] === fileURLToPath(import.meta.url)) {
  const extra = process.argv.slice(2);
  const host = hostTriple(execFileSync('rustc', ['-vV'], { encoding: 'utf8' }));
  refuseForeignTarget(extra, host);
  run('npx', ['vite', 'build']);
  run('cargo', ['build', '--manifest-path', join(tauri, 'Cargo.toml'), '--release', '--no-default-features', '--features', 'embed-ui', '--bin', 'tmm']);
  mkdirSync(join(tauri, 'binaries'), { recursive: true });
  const side = join(tauri, 'binaries', `tmm-${host}`);
  copyFileSync(join(tauri, 'target', 'release', 'tmm'), side);
  chmodSync(side, 0o755);
  run('npx', ['tauri', 'build', '--config', join(tauri, 'tauri.bundle.conf.json'), ...extra]);
}
