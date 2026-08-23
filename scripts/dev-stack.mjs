#!/usr/bin/env node
import { spawn, spawnSync } from 'node:child_process';
import { dirname, join, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';
import { DEV_SERVER_PORT_ENV, devServerPort } from './dev-ports.mjs';

export const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const STOP_TIMEOUT_MS = 5000;

/** Commands are kept explicit so neither child re-runs a partial preflight. */
export function serviceSpecs(root = repoRoot, env = process.env) {
  const port = String(devServerPort(env));
  return [
    {
      name: 'server',
      command: 'bash',
      args: [join(root, 'scripts/dev-server-watch.sh')],
      env: { HOST: '127.0.0.1', PORT: port, TLS_CERT: '', TLS_KEY: '' },
    },
    {
      name: 'web',
      command: join(root, 'node_modules/.bin/vite'),
      args: ['dev'],
      env: { [DEV_SERVER_PORT_ENV]: port },
    },
  ];
}

/** Each service owns a process group, so Cargo/rustc and Vite children stop too. */
export function stopProcessTree(child, signal = 'SIGTERM') {
  try {
    if (process.platform !== 'win32' && Number.isInteger(child.pid)) {
      process.kill(-child.pid, signal);
    } else {
      child.kill(signal);
    }
  } catch (error) {
    if (error?.code !== 'ESRCH') throw error;
  }
}

export function processTreeAlive(child) {
  if (process.platform !== 'win32' && Number.isInteger(child.pid)) {
    try {
      process.kill(-child.pid, 0);
      return true;
    } catch (error) {
      if (error?.code === 'ESRCH') return false;
      if (error?.code === 'EPERM') return true;
      throw error;
    }
  }
  return child.exitCode === null && child.signalCode === null;
}

async function waitForProcessTrees(children, allExited, timeoutMs) {
  let leadersExited = false;
  void allExited.then(() => { leadersExited = true; });
  const deadline = Date.now() + timeoutMs;

  while (true) {
    if (leadersExited && children.every(({ child }) => !processTreeAlive(child))) return true;
    const remaining = deadline - Date.now();
    if (remaining <= 0) return false;
    await new Promise((resolveWait) => setTimeout(resolveWait, Math.min(25, remaining)));
  }
}

function runPreflight(root) {
  const result = spawnSync(process.execPath, [join(root, 'scripts/preflight.mjs'), 'dev'], {
    cwd: root,
    env: process.env,
    stdio: 'inherit',
  });
  if (result.error) {
    console.error(`[dev] preflight failed: ${result.error.message}`);
    return 1;
  }
  return result.status ?? 1;
}

export async function runDevStack({
  root = repoRoot,
  spawnProcess = spawn,
  preflight = runPreflight,
  stopProcess = stopProcessTree,
  stopTimeoutMs = STOP_TIMEOUT_MS,
  signalSource = process,
} = {}) {
  const preflightStatus = preflight(root);
  if (preflightStatus !== 0) return preflightStatus;

  console.log('[dev] starting web (Vite HMR) + server (Rust rebuild/restart watcher)');

  const children = [];
  let resolveSignal;
  let requested = null;
  let shutdownStarted = false;
  let forceStopped = false;
  const requestedSignal = new Promise((resolveRequested) => { resolveSignal = resolveRequested; });
  const requestStop = (signal) => {
    if (shutdownStarted || requested) {
      forceStopped = true;
      console.error('\n[dev] received another signal; killing service process groups');
      for (const { child } of children) stopProcess(child, 'SIGKILL');
      return;
    }
    requested = signal;
    resolveSignal(signal);
  };
  const onSigint = () => requestStop('SIGINT');
  const onSigterm = () => requestStop('SIGTERM');
  signalSource.on('SIGINT', onSigint);
  signalSource.on('SIGTERM', onSigterm);

  const exits = [];
  for (const spec of serviceSpecs(root)) {
    const child = spawnProcess(spec.command, spec.args, {
      cwd: root,
      env: { ...process.env, ...spec.env },
      stdio: 'inherit',
      // On Unix this makes a new process group without unref'ing the child.
      detached: process.platform !== 'win32',
    });
    children.push({ ...spec, child });
    exits.push(new Promise((resolveExit) => {
      let settled = false;
      const finish = (result) => {
        if (settled) return;
        settled = true;
        resolveExit({ name: spec.name, ...result });
      };
      child.once('error', (error) => finish({ code: 1, signal: null, error }));
      child.once('exit', (code, signal) => finish({ code, signal, error: null }));
    }));
  }

  const first = await Promise.race([
    ...exits.map((promise) => promise.then((result) => ({ type: 'exit', result }))),
    requestedSignal.then((signal) => ({ type: 'signal', signal })),
  ]);

  shutdownStarted = true;
  if (first.type === 'exit') {
    const detail = first.result.error?.message || first.result.signal || `exit ${first.result.code}`;
    console.error(`[dev] ${first.result.name} stopped (${detail}); stopping the dev stack`);
  } else {
    console.log(`\n[dev] received ${first.signal}; stopping the dev stack`);
  }

  if (!forceStopped) {
    for (const { child } of children) stopProcess(child, 'SIGTERM');
  }

  const allExited = Promise.all(exits);
  const stopped = await waitForProcessTrees(children, allExited, stopTimeoutMs);

  if (!stopped) {
    console.error(`[dev] services did not stop within ${stopTimeoutMs}ms; killing process groups`);
    for (const { child } of children) stopProcess(child, 'SIGKILL');
  }
  await allExited;

  signalSource.removeListener('SIGINT', onSigint);
  signalSource.removeListener('SIGTERM', onSigterm);

  if (first.type === 'signal') return first.signal === 'SIGINT' ? 130 : 143;
  return first.result.code && first.result.code !== 0 ? first.result.code : 1;
}

const isMain = process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url);
if (isMain) process.exitCode = await runDevStack();
