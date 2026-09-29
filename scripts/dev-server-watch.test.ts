import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { chmod, mkdir, mkdtemp, readFile, rm, stat, utimes, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

// Board #282: the watcher builds `tmm` with the server, and a rebuilt tmm
// restarts nothing. Run for real against a copy of the script in a scratch
// tree, with a fake `cargo` that writes whichever binaries it was asked for
// (their bytes carry a version the test bumps) and a fake server that logs
// each start.
const FAKE_CARGO = `#!/bin/sh
case " $* " in *" --bins "*) bins="server tmm" ;; *" --bin server "*) bins="server" ;; *) exit 2 ;; esac
mkdir -p src-tauri/target/release
for b in $bins; do
  v=$(cat "$STATE/$b.v")
  if [ "$b" = server ]; then
    printf '#!/bin/sh\\n# server v%s\\necho up >> "$STATE/starts"\\nexec sleep 600\\n' "$v" > src-tauri/target/release/server
    chmod +x src-tauri/target/release/server
  else
    printf 'tmm v%s\\n' "$v" > src-tauri/target/release/tmm
  fi
done
`;

type Watcher = {
  root: string;
  state: string;
  starts: () => Promise<number>;
  tmm: () => Promise<string>;
  edit: () => Promise<void>;
  until: (what: string, ok: () => Promise<boolean>, ms?: number) => Promise<void>;
};

// One scratch tree and one watcher process, cleaned up on EVERY exit path
// (validator / orchestrator 14:12): the tree is made inside the cleanup
// scope, the child's exit and spawn error are registered at spawn — before
// any wait — so a watcher that dies early fails the wait at once instead of
// hanging on an event already missed, and cleanup kills only a child still
// running. The watcher's process GROUP is killed, so its fake server goes too.
async function withWatcher(script: string, body: (w: Watcher) => Promise<void>) {
  const root = await mkdtemp(join(tmpdir(), 'tmm-watch-'));
  let watch: ReturnType<typeof spawn> | undefined;
  let exited: Promise<unknown> = Promise.resolve();
  let dead: string | undefined;
  try {
    const state = join(root, 'state');
    const src = join(root, 'src-tauri', 'src', 'a.rs');
    await Promise.all(['scripts', 'bin', 'home', 'state', 'src-tauri/src'].map((d) => mkdir(join(root, d), { recursive: true })));
    await writeFile(join(root, 'scripts', 'dev-server-watch.sh'), script);
    await writeFile(join(root, 'bin', 'cargo'), FAKE_CARGO);
    await chmod(join(root, 'bin', 'cargo'), 0o755);
    await writeFile(join(root, 'src-tauri', 'Cargo.toml'), '');
    await writeFile(src, '');
    await writeFile(join(state, 'server.v'), '1');
    await writeFile(join(state, 'tmm.v'), '1');

    watch = spawn('bash', [join(root, 'scripts', 'dev-server-watch.sh')], {
      env: { ...process.env, PATH: `${join(root, 'bin')}:${process.env.PATH}`, HOME: join(root, 'home'), STATE: state },
      stdio: 'ignore',
      detached: true,
    });
    exited = Promise.race([
      once(watch, 'exit').then(([code, signal]) => (dead = `watcher exited (code ${code}, signal ${signal})`)),
      once(watch, 'error').then(([e]) => (dead = `watcher failed to start: ${e}`)),
    ]);

    let bumps = 0;
    await body({
      root,
      state,
      starts: async () => (await readFile(join(state, 'starts'), 'utf8').catch(() => '')).split('\n').filter(Boolean).length,
      tmm: () => readFile(join(root, 'src-tauri/target/release/tmm'), 'utf8').catch(() => ''),
      // A source edit, with an mtime the watcher cannot miss.
      edit: async () => {
        bumps += 1;
        const t = Date.now() / 1000 + bumps * 10;
        await utimes(src, t, t);
      },
      until: async (what, ok, ms = 20_000) => {
        const end = Date.now() + ms;
        while (Date.now() < end) {
          if (await ok()) return;
          if (dead) assert.fail(`${dead} while waiting for ${what}`);
          await new Promise((r) => setTimeout(r, 200));
        }
        assert.fail(`timed out waiting for ${what}`);
      },
    });
  } finally {
    if (watch?.pid !== undefined && watch.exitCode === null && watch.signalCode === null) {
      try {
        process.kill(-watch.pid, 'SIGTERM');
      } catch {
        // already gone
      }
      await exited;
    }
    if (watch?.pid !== undefined) {
      try {
        process.kill(-watch.pid, 'SIGKILL'); // the fake server, if its group outlived the watcher
      } catch {
        // already gone
      }
    }
    await rm(root, { recursive: true, force: true });
  }
}

const SCRIPT = readFile(new URL('./dev-server-watch.sh', import.meta.url), 'utf8');

test('the watcher ships tmm with the server and restarts the server only for its own bytes', { timeout: 60_000 }, async () => {
  await withWatcher(await SCRIPT, async (w) => {
    await w.until('the first build and start', async () => (await w.starts()) === 1 && (await w.tmm()) === 'tmm v1\n');

    // Only tmm changed: it is rebuilt in place, the server keeps running.
    await writeFile(join(w.state, 'tmm.v'), '2');
    await w.edit();
    await w.until('tmm v2', async () => (await w.tmm()) === 'tmm v2\n');
    await new Promise((r) => setTimeout(r, 4_000));
    assert.equal(await w.starts(), 1, 'a rebuilt tmm must not restart the server');

    // The server's own bytes changed: restarted once, and tmm is still built.
    await writeFile(join(w.state, 'server.v'), '2');
    await writeFile(join(w.state, 'tmm.v'), '3');
    await w.edit();
    await w.until('the server restart', async () => (await w.starts()) === 2);
    assert.equal(await w.tmm(), 'tmm v3\n');
  });
});

// The harness itself (validator 14:10): a watcher that dies before its first
// build — a syntax error, say — fails the wait promptly, and leaves nothing.
test('a watcher that exits before its first build fails promptly and leaves no scratch tree', { timeout: 30_000 }, async () => {
  let root = '';
  const started = Date.now();
  await assert.rejects(
    withWatcher('#!/usr/bin/env bash\nif then\n', async (w) => {
      root = w.root;
      await w.until('the first build and start', async () => (await w.starts()) === 1);
    }),
    /watcher exited \(code 2/,
  );
  assert.ok(Date.now() - started < 5_000, `bounded: ${Date.now() - started} ms`);
  await assert.rejects(stat(root), { code: 'ENOENT' }, 'the scratch tree is removed');
});
