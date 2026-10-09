import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { chmod, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises';
import { existsSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';

// Board #323 (reviewer P1-5): the smoke's EXIT trap must clean up only what
// THIS run owns, manage its foreground gateway, and fail when cleanup
// fails. Run the real script against a fake user manager (systemctl,
// journalctl, ss) and a fake tmm in a scratch HOME — never the real one.
const SCRIPT = join(import.meta.dirname, 'gateway-smoke.sh');

async function world(mode: string) {
  const root = await mkdtemp(join(tmpdir(), 'gwsmoke-'));
  const bin = join(root, 'bin');
  const home = join(root, 'home');
  await mkdir(bin, { recursive: true });
  await mkdir(home, { recursive: true });
  const log = join(root, 'calls.log');
  const fg = join(root, 'fg.pid');
  const fake = (body: string) => `#!/bin/sh\necho "$(basename "$0") $*" >> '${log}'\n${body}\n`;
  await writeFile(join(bin, 'systemctl'), fake('case "$*" in *" cat "*) exit 1 ;; *" show "*) echo 0 ;; *" is-active "*) exit 3 ;; esac; exit 0'));
  await writeFile(join(bin, 'journalctl'), fake('exit 0'));
  await writeFile(join(bin, 'ss'), fake('exit 0'));
  // The fake tmm: the unit lives where the script looks; "ours" = it names
  // this run's XDG_CONFIG_HOME.
  await writeFile(join(root, 'tmm'), fake(`
unit="$HOME/.config/systemd/user/$TMM_GATEWAY_SERVICE"
case "$1 $2" in
  "gateway start")
    echo $$ > '${fg}'
    ${mode === 'fg-hang' ? '' : `port=$(sed -n 's/^port = //p' "$XDG_CONFIG_HOME/tmux-mobile/config.toml"); echo "listening on ws://127.0.0.1:$port"`}
    exec sleep 30 ;;
  "gateway install")
    mkdir -p "$(dirname "$unit")"
    ${mode === 'competing'
      ? `printf 'Environment="XDG_CONFIG_HOME=/someone/else"\\n' > "$unit"; echo "not this tmm" >&2; exit 1`
      : `printf 'ExecStart="%s" gateway start --service\\nEnvironment="XDG_CONFIG_HOME=%s"\\n' "$0" "$XDG_CONFIG_HOME" > "$unit"`} ;;
  "gateway uninstall")
    grep -q "XDG_CONFIG_HOME=$XDG_CONFIG_HOME\\"" "$unit" || { echo "not this tmm; refusing" >&2; exit 1; }
    ${mode === 'stop-fails' ? 'echo "could not stop: bus error" >&2; exit 1' : 'rm -f "$unit"'} ;;
esac`));
  for (const f of ['systemctl', 'journalctl', 'ss']) await chmod(join(bin, f), 0o755);
  await chmod(join(root, 'tmm'), 0o755);
  let last = '';
  const spawned = () => spawnSync('bash', [SCRIPT, join(root, 'tmm'), '19999'], {
    env: { HOME: home, PATH: `${bin}:/usr/bin:/bin`, XDG_RUNTIME_DIR: root, DBUS_SESSION_BUS_ADDRESS: 'unix:path=/nonexistent' },
    encoding: 'utf8',
    timeout: 30_000,
  });
  const run = () => { const r = spawned(); last = r.stderr ?? ''; return r; };
  // Everything the script said it kept, removed on every exit path.
  const dispose = async () => {
    for (const m of last.matchAll(/(?:keeping|evidence kept in) (\/tmp\/gw323-smoke[\w.-]+)/g)) await rm(m[1], { recursive: true, force: true });
    await rm(root, { recursive: true, force: true });
  };
  return { root, home, log, fg, run, dispose, calls: async () => (existsSync(log) ? await readFile(log, 'utf8') : '') };
}

const alive = (pid: number) => { try { process.kill(pid, 0); return true; } catch { return false; } };
const scratchKept = (out: string) => /keeping (\/tmp\/gw323-smoke\.\w+)/.exec(out)?.[1];

test('a competing install of the name is refused and the other unit is never stopped', async () => {
  const w = await world('competing');
  try {
    const r = w.run();
    assert.notEqual(r.status, 0, r.stdout + r.stderr);
    const calls = await w.calls();
    assert.doesNotMatch(calls, /systemctl .*(disable|stop|kill)/, calls);
    assert.match(calls, /tmm gateway uninstall/, 'cleanup goes through the identity check');
    assert.match(r.stderr, /CLEANUP FAILED: not this tmm/);
    const kept = scratchKept(r.stderr);
    assert.ok(kept && existsSync(kept), 'the scratch root is kept while a unit remains');
  } finally { await w.dispose(); }
});

test('a failure in the foreground phase stops the foreground gateway', async () => {
  const w = await world('fg-hang');
  try {
    const r = w.run();
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /did not use config.toml's port/);
    const pid = Number(await readFile(w.fg, 'utf8'));
    assert.ok(pid > 0 && !alive(pid), `the foreground gateway ${pid} is gone`);
    assert.doesNotMatch(await w.calls(), /tmm gateway (install|uninstall)/, 'nothing was installed, nothing to remove');
  } finally { await w.dispose(); }
});

test('a cleanup stop that fails fails the run and keeps the evidence and the scratch root', async () => {
  const w = await world('stop-fails');
  try {
    const r = w.run();
    assert.notEqual(r.status, 0);
    assert.match(r.stderr, /the service is not on the scratch config's port/, 'the first failure');
    assert.match(r.stderr, /CLEANUP FAILED: could not stop/);
    assert.match(r.stderr, /evidence kept in/);
    const kept = scratchKept(r.stderr);
    assert.ok(kept && existsSync(join(kept, 'tmm')), 'the exe a still-installed unit runs is not deleted');
  } finally { await w.dispose(); }
});
