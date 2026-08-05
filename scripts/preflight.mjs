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
//    machine-local gradle build symlink at THIS checkout's output. A global
//    gradle init script (~/.gradle/init.d/00-build-dir.gradle.kts) redirects
//    build/ to ~/.cache/builds/gradle-builds/<dirname>-<md5(abs path)[:12]>/,
//    so the slug is a pure function of the gradle root path and we can compute
//    the right answer instead of guessing.
//
//    Two ways the symlink goes wrong, and the second one is the nasty one:
//    - DANGLING (cache pruned, or the path canonicalizes differently under the
//      zfs-boost overlay): the documented APK path 404s even though the build
//      SUCCEEDED. Loud, easy to spot.
//    - STALE BUT VALID: it points at another checkout's build dir — the same
//      repo cloned at a second path hashes to a different slug. Then a green
//      build silently serves that other tree's older APK, with a plausible
//      timestamp and everything. Hit for real on 2026-08-05: this checkout's
//      link had pointed at /Users/clawd/work/project/260226_tmux_mobile's
//      output since Jul 22, so a fresh build "produced" a day-old APK.
//    Healing only the dangling case is therefore not enough, and picking the
//    newest APK across all android-* dirs is exactly how you land on the wrong
//    checkout. Compute, compare, re-point.
//
// Zero dependencies; runs everywhere node does.
import { execSync } from 'node:child_process';
import { existsSync, readlinkSync, unlinkSync, symlinkSync, readdirSync, statSync, lstatSync } from 'node:fs';
import { basename, dirname, join, resolve } from 'node:path';
import { createHash } from 'node:crypto';
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

/** Where the gradle init script puts `:app`'s build dir for this checkout.
 *  Mirrors 00-build-dir.gradle.kts exactly: slug = "<dirname>-<md5(abs path of
 *  the GRADLE ROOT)[:12]>", and the `:app` subproject lands in `app/`. The root
 *  here is src-tauri/gen/android, NOT the repo root. */
function expectedAppBuildDir(androidRoot) {
  const hash = createHash('md5').update(androidRoot).digest('hex').slice(0, 12);
  const slug = `${basename(androidRoot)}-${hash}`;
  return join(process.env.HOME || '', '.cache/builds/gradle-builds', slug, 'app');
}

/** Fallback for when the computed slug has no APK — the overlay case where
 *  gradle saw a different absolute path than we do. Newest APK wins, which is a
 *  guess: it can pick another checkout, so it is only ever a last resort. */
function newestAppBuildDir(apkRel) {
  const cache = join(process.env.HOME || '', '.cache/builds/gradle-builds');
  try {
    return readdirSync(cache)
      .filter(d => d.startsWith('android-'))
      .map(d => join(cache, d, 'app'))
      .filter(d => existsSync(join(d, apkRel)))
      .sort((a, b) => statSync(join(b, apkRel)).mtimeMs - statSync(join(a, apkRel)).mtimeMs)[0] || null;
  } catch {
    return null;
  }
}

if (mode === 'android-apk') {
  const androidRoot = join(root, 'src-tauri/gen/android');
  const link = join(androidRoot, 'app/build');
  const apkRel = 'outputs/apk/universal/release/app-universal-release.apk';
  try {
    const info = (() => { try { return lstatSync(link); } catch { return null; } })();
    // A real directory means this machine opted out of the redirect
    // (.gradle-local-build) — gradle really did build in-tree. Not ours to touch.
    if (!info || info.isSymbolicLink()) {
      const want = expectedAppBuildDir(androidRoot);
      const have = info ? readlinkSync(link).replace(/\/+$/, '') : null;
      const target = existsSync(join(want, apkRel)) ? want : newestAppBuildDir(apkRel);
      if (target && target !== have) {
        const why = have === null ? 'missing'
          : existsSync(have) ? 'STALE — it pointed at another checkout, so this build would have served ITS older APK'
          : 'dangling';
        if (info) unlinkSync(link);
        symlinkSync(target + '/', link);
        console.error(`🔗 gradle build symlink re-pointed (${why})`);
        if (have) console.error(`   was:  ${have}`);
        console.error(`   now:  ${target}`);
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
