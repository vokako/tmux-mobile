#!/usr/bin/env node
// Preflight for dev/build commands. Two jobs:
//
// 1. `preflight dev`: fail FAST if the dev ports are already taken, and say
//    exactly who owns them. Without this, a second `tauri dev` half-starts:
//    vite dies on the port conflict AFTER nuking node_modules/.vite (the
//    dep-optimizer cache) — every open client then 504s on the next reload
//    (the "blank white page") — and the Tauri app process survives as an
//    orphan holding no ports but wasting a build.
//
// 2. `preflight android-apk`: after `tauri android build`, re-point the
//    machine-local gradle build symlink if it dangles. On this machine a
//    global gradle init script redirects build/ to
//    ~/.cache/builds/gradle-builds/<name-md5(path)>/ — when the project
//    path canonicalizes differently (zfs-boost overlay) or the cache is
//    pruned, the old symlink dangles and the documented APK path 404s even
//    though the build SUCCEEDED.
//
// Zero dependencies; runs everywhere node does.
import { execSync } from 'node:child_process';
import { existsSync, readlinkSync, unlinkSync, symlinkSync, readdirSync, statSync, lstatSync } from 'node:fs';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const mode = process.argv[2] || 'dev';

function whoOwnsPort(port) {
  try {
    const out = execSync(`lsof -nP -iTCP:${port} -sTCP:LISTEN`, { encoding: 'utf8', stdio: ['ignore', 'pipe', 'ignore'] });
    const line = out.trim().split('\n')[1];
    if (!line) return null;
    const [cmd, pid] = line.split(/\s+/);
    return { cmd, pid };
  } catch {
    return null; // lsof exits non-zero when nothing listens — that's the good case
  }
}

if (mode === 'dev') {
  const conflicts = [];
  for (const [port, what] of [[5173, 'vite dev server'], [9899, 'tmux-mobile WS server']]) {
    const owner = whoOwnsPort(port);
    if (owner) conflicts.push({ port, what, ...owner });
  }
  if (conflicts.length) {
    console.error('\n✋ dev stack already running — refusing to half-start a second one.\n');
    for (const c of conflicts) {
      console.error(`   port ${c.port} (${c.what}) is held by ${c.cmd} (pid ${c.pid})`);
    }
    console.error('\n   Reuse the running instance, or stop it first:');
    console.error(`   kill ${conflicts.map(c => c.pid).join(' ')}\n`);
    console.error('   (Starting a second instance corrupts the vite dep cache and');
    console.error('   leaves an orphaned Tauri process — see scripts/preflight.mjs.)\n');
    process.exit(1);
  }
  process.exit(0);
}

if (mode === 'android-apk') {
  const link = join(root, 'src-tauri/gen/android/app/build');
  const apkRel = 'outputs/apk/universal/release/app-universal-release.apk';
  try {
    // Not a symlink (or missing) → nothing to heal on this machine.
    const isLink = (() => { try { return lstatSync(link).isSymbolicLink(); } catch { return false; } })();
    if (isLink && !existsSync(readlinkSync(link))) {
      // Dangling: find the newest gradle-builds android dir that has our APK.
      const cache = join(process.env.HOME || '', '.cache/builds/gradle-builds');
      const candidates = readdirSync(cache)
        .filter(d => d.startsWith('android-'))
        .map(d => join(cache, d, 'app'))
        .filter(d => existsSync(join(d, apkRel)))
        .sort((a, b) => statSync(join(b, apkRel)).mtimeMs - statSync(join(a, apkRel)).mtimeMs);
      if (candidates.length) {
        unlinkSync(link);
        symlinkSync(candidates[0] + '/', link);
        console.error(`🔗 healed dangling gradle build symlink → ${candidates[0]}`);
      }
    }
    const apk = join(link, apkRel);
    if (existsSync(apk)) {
      console.error(`📦 APK: ${apk}`);
      process.exit(0);
    }
    console.error(`⚠️  build reported success but no APK at ${apk} — check the gradle build dir redirect (scripts/preflight.mjs).`);
    process.exit(1);
  } catch (e) {
    console.error(`⚠️  android-apk postflight failed: ${e.message}`);
    process.exit(1);
  }
}

console.error(`unknown preflight mode: ${mode}`);
process.exit(1);
