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
  'files/Files.svelte': {
    browsed: /resetFilesMemory\(\)/u,       // parked cwds keyed by session name
    settles: /suspendDownloads/u,           // awaited and emptied by the suspend
    cancels: /suspendDownloads/u,
    inFlight: /suspendDownloads/u,
    webSeq: /./u,                            // a counter for browser row ids, not data
  },
  'files/downloads.svelte.ts': { rows: /forgetDownloadRows\(\)/u },
  'core/agents.ts': { served: /setServedBackends\(null\)/u, servedListeners: /setServedBackends\(null\)/u },
  'ui/hover.svelte.ts': { shown: /hoverCard\.hide\(\)/u, hiddenAt: /hoverCard\.hide\(\)/u },
  // Re-read from the parked keys after pointTo (comeUp), not reset here.
  'hub/hub-prefs.svelte.ts': { state: /./u },
};
const GLOBAL: Record<string, Record<string, string>> = {
  'core/markdown.ts': { cache: 'rendered HTML keyed by the message text itself' },
  'hub/hub.ts': { squashed: 'a WeakMap on message objects; the tree that holds them is gone' },
  'hub/reveal.ts': { inFlight: 'a WeakMap on DOM nodes' },
  'app/leave-guards.ts': { guards: 'instances unregister on unmount' },
  'app/shortcuts.svelte.ts': { state: 'a preference' },
  'app/terminal-prefs.svelte.ts': { state: 'a preference' },
  'app/layout.svelte.ts': { mode: 'a preference' },
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
    const decl = /^(?:export )?(?:let|const) (\w+)\b[^=\n]*= (?:\$state\b|new (?:Map|Set|WeakMap|LruCache)\b|null;|0;)/gmu;
    for (const m of text.matchAll(decl)) {
      const name = m[1]!;
      if (/^[A-Z_0-9]+$/u.test(name)) continue; // constants
      if (rel === 'core/ws.ts') continue;       // the transport: cleared by disconnect()
      found.push(`${rel}:${name}`);
      const per = PER_SERVER[rel]?.[name];
      const glob = GLOBAL[rel]?.[name];
      assert.ok(per || glob, `${rel}: module-level "${name}" must be reset by a switch or listed as GLOBAL`);
      if (per && rel !== 'hub/hub-prefs.svelte.ts' && !per.test(reset) && !/suspendDownloads/u.test(per.source)) {
        assert.fail(`${rel}:${name} is per-server but resetServerMemory does not reset it`);
      }
    }
  }
  assert.ok(found.includes('files/Files.svelte:browsed'), 'the scan sees module scripts');
  assert.match(app, /resetMemory: resetServerMemory,/u, 'the switch runs the reset');
  assert.match(app, /comeUp: \(target\) => \{\s*hubPrefs\.reloadServerState\(\);/u, 'hub prefs re-read the target’s parked keys');
});
