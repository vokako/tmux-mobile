import assert from 'node:assert/strict';
import { spawn } from 'node:child_process';
import { EventEmitter, once } from 'node:events';
import { mkdtemp, readFile, rm } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import test from 'node:test';
import { serviceSpecs, runDevStack, stopProcessTree } from './dev-stack.mjs';

class FakeChild extends EventEmitter {
  exitCode: number | null = null;
  signalCode: NodeJS.Signals | null = null;
  killedWith: NodeJS.Signals[] = [];

  finish(code: number | null, signal: NodeJS.Signals | null = null) {
    this.exitCode = code;
    this.signalCode = signal;
    this.emit('exit', code, signal);
  }

  kill(signal: NodeJS.Signals) {
    this.killedWith.push(signal);
    queueMicrotask(() => this.finish(null, signal));
    return true;
  }
}

class StubbornChild extends FakeChild {
  override kill(signal: NodeJS.Signals) {
    this.killedWith.push(signal);
    if (signal === 'SIGKILL') queueMicrotask(() => this.finish(null, signal));
    return true;
  }
}


function fakeStack(children: FakeChild[]) {
  let index = 0;
  return runDevStack({
    root: '/workspace',
    preflight: () => 0,
    spawnProcess: () => children[index++],
  });
}

test('dev stack uses the Rust watcher and the local Vite binary', async () => {
  const specs = serviceSpecs('/workspace');
  assert.deepEqual(specs, [
    {
      name: 'server',
      command: 'bash',
      args: ['/workspace/scripts/dev-server-watch.sh'],
    },
    {
      name: 'web',
      command: '/workspace/node_modules/.bin/vite',
      args: ['dev'],
    },
  ]);

  const pkg = JSON.parse(await readFile(new URL('../package.json', import.meta.url), 'utf8'));
  assert.equal(pkg.scripts['dev:all'], 'node scripts/dev-stack.mjs');
  assert.match(pkg.scripts.dev, /preflight\.mjs web/);
  assert.match(pkg.scripts['dev:server'], /preflight\.mjs server/);
  assert.equal(pkg.scripts.predev, undefined);
});

test('when either service exits the supervisor stops its sibling', async () => {
  const children = [new FakeChild(), new FakeChild()];
  const result = fakeStack(children);

  queueMicrotask(() => children[0].finish(7));

  assert.equal(await result, 7);
  assert.deepEqual(children[1].killedWith, ['SIGTERM']);
});

test('an unexpected clean service exit still fails the stopped stack', async () => {
  const children = [new FakeChild(), new FakeChild()];
  const result = fakeStack(children);

  queueMicrotask(() => children[0].finish(0));

  assert.equal(await result, 1);
  assert.deepEqual(children[1].killedWith, ['SIGTERM']);
});


test('TERM timeout escalates to KILL for stubborn service groups', async () => {
  const children = [new StubbornChild(), new StubbornChild()];
  let index = 0;
  const result = runDevStack({
    root: '/workspace',
    preflight: () => 0,
    spawnProcess: () => children[index++],
    stopTimeoutMs: 10,
  });

  queueMicrotask(() => children[0].finish(7));

  assert.equal(await result, 7);
  assert.deepEqual(children[0].killedWith, ['SIGTERM', 'SIGKILL']);
  assert.deepEqual(children[1].killedWith, ['SIGTERM', 'SIGKILL']);
});

test('a second Ctrl-C immediately kills stubborn service groups', async () => {
  const children = [new StubbornChild(), new StubbornChild()];
  const signals = new EventEmitter();
  let index = 0;
  const result = runDevStack({
    root: '/workspace',
    preflight: () => 0,
    spawnProcess: () => children[index++],
    stopTimeoutMs: 10_000,
    signalSource: signals,
  });

  queueMicrotask(() => signals.emit('SIGINT'));
  setTimeout(() => signals.emit('SIGINT'), 10);

  assert.equal(await result, 130);
  assert.deepEqual(children[0].killedWith, ['SIGTERM', 'SIGKILL']);
  assert.deepEqual(children[1].killedWith, ['SIGTERM', 'SIGKILL']);
});

test('timeout kills descendants even after their process-group leader exits', {
  skip: process.platform === 'win32' ? 'Unix process groups are required by the Bash watcher' : false,
}, async (t) => {
  const dir = await mkdtemp(join(tmpdir(), 'dev-stack-orphan-test-'));
  const pidFiles = [join(dir, 'child-0.pid'), join(dir, 'child-1.pid')];
  const launched: ReturnType<typeof spawn>[] = [];
  const signals = new EventEmitter();
  let index = 0;

  t.after(async () => {
    for (const child of launched) stopProcessTree(child, 'SIGKILL');
    await rm(dir, { recursive: true, force: true });
  });

  const result = runDevStack({
    root: '/workspace',
    preflight: () => 0,
    stopTimeoutMs: 50,
    signalSource: signals,
    spawnProcess: (_command: string, _args: readonly string[], options: Parameters<typeof spawn>[2]) => {
      const pidFile = pidFiles[index++];
      const child = spawn('bash', [
        '-c',
        `trap 'exit 0' TERM INT; bash -c 'trap "" TERM INT; echo $$ > "$PID_FILE"; while :; do sleep 1; done' & wait`,
      ], {
        ...options,
        cwd: process.cwd(),
        env: { ...process.env, PID_FILE: pidFile },
        stdio: 'ignore',
      });
      launched.push(child);
      return child;
    },
  });

  for (const pidFile of pidFiles) {
    for (let i = 0; i < 100; i += 1) {
      try {
        if (Number.parseInt(await readFile(pidFile, 'utf8'), 10) > 0) break;
      } catch {
        await new Promise((resolve) => setTimeout(resolve, 10));
      }
    }
  }

  signals.emit('SIGINT');
  assert.equal(await result, 130);

  for (const pidFile of pidFiles) {
    const descendantPid = Number.parseInt(await readFile(pidFile, 'utf8'), 10);
    assert.throws(
      () => process.kill(descendantPid, 0),
      (error: unknown) => error instanceof Error && 'code' in error && error.code === 'ESRCH',
      `descendant ${descendantPid} survived after its leader exited`,
    );
  }
});
test('process-tree stop reaches a watcher foreground child', {
  skip: process.platform === 'win32' ? 'Unix process groups are required by the Bash watcher' : false,
}, async (t) => {
  const dir = await mkdtemp(join(tmpdir(), 'dev-stack-test-'));
  const pidFile = join(dir, 'child.pid');
  const watcher = spawn('bash', [
    '-c',
    'trap "exit 0" INT TERM; sleep 60 & echo $! > "$PID_FILE"; wait',
  ], {
    detached: true,
    env: { ...process.env, PID_FILE: pidFile },
    stdio: 'ignore',
  });

  t.after(async () => {
    stopProcessTree(watcher, 'SIGKILL');
    await rm(dir, { recursive: true, force: true });
  });

  let foregroundPid = 0;
  for (let i = 0; i < 100; i += 1) {
    try {
      foregroundPid = Number.parseInt(await readFile(pidFile, 'utf8'), 10);
      if (foregroundPid) break;
    } catch {
      await new Promise((resolve) => setTimeout(resolve, 10));
    }
  }
  assert.ok(foregroundPid > 0, 'foreground child pid was not recorded');

  stopProcessTree(watcher, 'SIGTERM');
  await Promise.race([
    once(watcher, 'exit'),
    new Promise((_, reject) => setTimeout(() => reject(new Error('watcher did not stop')), 2000)),
  ]);

  let childAlive = true;
  for (let i = 0; i < 100; i += 1) {
    try {
      process.kill(foregroundPid, 0);
      await new Promise((resolve) => setTimeout(resolve, 10));
    } catch (error) {
      if (error instanceof Error && 'code' in error && error.code === 'ESRCH') {
        childAlive = false;
        break;
      }
      throw error;
    }
  }
  assert.equal(childAlive, false, 'foreground child survived the process-group stop');
});
