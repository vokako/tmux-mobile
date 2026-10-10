// Maintenance pin for the in-place switch (board 315): every module-level
// store under src/lib is either reset by App's resetServerMemory (it holds
// something of the server being left) or listed here as GLOBAL with the
// reason it cannot hold a server's data. A new module cache must pick a side.
// The behaviour proof is server-switch.test.ts; this only keeps the list honest.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';

const LIB = new URL('../', import.meta.url);
const app = await readFile(new URL('../../App.svelte', import.meta.url), 'utf8');
const reset = app.match(/function resetServerMemory\(\) \{[\s\S]*?\n  \}/u)?.[0] ?? '';

/** file → store name → how a switch handles it. */
const PER_SERVER: Record<string, Record<string, RegExp>> = {
  // Since #335 ②a-4 all of it — parked cwds keyed by session name, the
  // one-writer claim per part, each attempt's abort handle and settle, and the
  // browser row counter — is an INSTANCE of createFilesMemory(). Same
  // obligation: the switch resets the positions and awaits the attempts.
  'files/Files.svelte': { memory: /resetFilesMemory\(\)/u },
  // Since #335 ②a-4 the rows are an INSTANCE of createDownloads(), so what
  // the scan sees at module level is the instance. Same obligation while the
  // app holds one of them: ②b replaces "reset it on a switch" with "drop that
  // server's instance".
  'files/downloads.svelte.ts': { downloadStore: /forgetDownloadRows\(\)/u },
  // Since #335 ②a-4 the list and its listeners are an INSTANCE of
  // createBackendCatalog(); the frozen fallback lists stayed module-level
  // because they describe what this BUILD ships, not what a server serves.
  'core/agents.ts': { backendCatalog: /setServedBackends\(null\)/u },
  'ui/hover.svelte.ts': { shown: /hoverCard\.hide\(\)/u, hiddenAt: /hoverCard\.hide\(\)/u },
  // Re-read from the parked keys after pointTo (comeUp), not reset here.
  // Since #335 ②a-4 it is an INSTANCE of createHubPrefs(storage); ②b builds
  // each runtime's on that server's scoped store.
  'hub/hub-prefs.svelte.ts': { hubPrefs: /./u },   // the per-SERVER half
  // The notification centre (board #322): its list is a parked key too,
  // re-read by centre.reload() beside hubPrefs on comeUp.
  'hub/notify-centre.svelte.ts': { state: /./u, jumps: /./u },
};
const GLOBAL: Record<string, Record<string, string>> = {
  'core/markdown.ts': { cache: 'rendered HTML keyed by the message text itself' },
  'hub/hub.ts': { squashed: 'a WeakMap on message objects; the tree that holds them is gone' },
  'hub/reveal.ts': { inFlight: 'a WeakMap on DOM nodes' },
  'app/leave-guards.ts': { guards: 'instances unregister on unmount' },
  'app/shortcuts.svelte.ts': { state: 'a preference' },
  // The person's and the window's Hub preferences (feed level, tool-row cap,
  // sidebar collapse). ONE owner however many servers are on screen, which is
  // the point: a shared storage key is not shared state, and two live
  // instances must not disagree about the same human's app (#335 ②a-4).
  'hub/hub-prefs.svelte.ts': { appPrefs: 'the person\'s preferences, shared by every server' },
  'app/terminal-prefs.svelte.ts': { state: 'a preference' },
  'app/layout.svelte.ts': { mode: 'a preference' },
  'app/server-mode.svelte.ts': { current: 'THIS computer\'s own server, not the connected one', started: 'one subscription per page' },
  'core/i18n.svelte.ts': { i18n: 'a preference' },
  'test/ssr.ts': { harness: 'test-only' },
};

async function* walk(dir: URL): AsyncGenerator<URL> {
  for (const e of await readdir(dir, { withFileTypes: true })) {
    const u = new URL(e.name + (e.isDirectory() ? '/' : ''), dir);
    if (e.isDirectory()) yield* walk(u);
    else if (/\.(ts|svelte)$/u.test(e.name) && !/\.test\.|fixture/u.test(e.name)) yield u;
  }
}

test('every module-level store is reset on a switch or named global (board 315)', async () => {
  const found: string[] = [];
  for await (const file of walk(LIB)) {
    const rel = decodeURIComponent(file.pathname.slice(LIB.pathname.length));
    let text = await readFile(file, 'utf8');
    if (rel.endsWith('.svelte')) text = (/<script module>([\s\S]*?)<\/script>/u.exec(text)?.[1] ?? '').replace(/^  /gmu, '');
    // Column 0 only: a module's own top level (a module script is dedented).
    // `= create<Something>()` is in the list because board #335 ②a turns
    // module stores into factories: a per-server store's state moves inside
    // its factory, where a column-0 scan cannot see it, so the INSTANCE has to
    // pick a side instead. Without this a conversion would quietly empty the
    // inventory this file exists to keep.
    const decl = /^(?:export )?(?:let|const) (\w+)\b[^=\n]*= (?:\$state\b|new (?:Map|Set|WeakMap|LruCache)\b|create[A-Z]\w*\(|null;|0;)/gmu;
    for (const m of text.matchAll(decl)) {
      const name = m[1]!;
      if (/^[A-Z_0-9]+$/u.test(name)) continue; // constants
      // The transport layer (board #335 ①). A connection's mutable state —
      // socket, pending, listeners, refcounts, ciphers, timers — lives on the
      // object `createConnection()` returns and is released by that object's
      // own disconnect()/dispose(), so it cannot be a module cache this list
      // is about. What stays module-level in ws.ts is the DEVICE reachability
      // memory (probeFailedAt), global by design: see
      // websocket-client.md § The transport is an object, not a module.
      if (/^core\/(ws|connection|ws-api|connection-registry)\.ts$/u.test(rel)) continue;
      found.push(`${rel}:${name}`);
      const per = PER_SERVER[rel]?.[name];
      const glob = GLOBAL[rel]?.[name];
      assert.ok(per || glob, `${rel}: module-level "${name}" must be reset by a switch or listed as GLOBAL`);
      if (per && rel !== 'hub/hub-prefs.svelte.ts' && rel !== 'hub/notify-centre.svelte.ts' && !per.test(reset) && !/suspendDownloads/u.test(per.source)) {
        assert.fail(`${rel}:${name} is per-server but resetServerMemory does not reset it`);
      }
    }
  }
  for (const instance of [
    'files/downloads.svelte.ts:downloadStore', 'core/agents.ts:backendCatalog',
    'files/Files.svelte:memory', 'hub/hub-prefs.svelte.ts:hubPrefs',
  ]) {
    assert.ok(found.includes(instance), `the scan sees the factory instance ${instance}`);
  }
  assert.match(app, /resetMemory: resetServerMemory,/u, 'the switch runs the reset');
  assert.match(app, /comeUp: \(target\) => \{\s*hubPrefs\.reloadServerState\(\);\s*centre\.reload\(\);/u, 'hub prefs and the centre re-read the target’s parked keys');
});
