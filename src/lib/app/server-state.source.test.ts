// Maintenance pins for the per-server storage layout (board #335 ②a-3).
// Behaviour proofs are server-state.test.ts; these keep two facts that a
// reviewer cannot see by calling the modules:
//
//   - the `<key>::<id>` layout is spelled in ONE function. Phase ② adds a
//     second reader of these slots beside `servers.ts`'s park/point, and two
//     places building the string by hand is how they come to disagree.
//   - nothing in ②a is wired into production. The orchestrator's revised scope
//     ships the capability and leaves every live read and write where it was,
//     because enabling the fold while the legacy switch still owns the live
//     keys loses a draft rather than improving anything.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { glob } from 'node:fs/promises';

const read = (name: string) => readFile(new URL(`./${name}`, import.meta.url), 'utf8');
/** Code only: a rule about imports must not trip over a comment naming one. */
const code = (text: string) => text
  .replace(/\/\*[\s\S]*?\*\//gu, '')
  .split('\n')
  .filter(line => !/^\s*(\/\/|\*)/u.test(line))
  .join('\n');

test('one function spells the per-server key layout', async () => {
  const layout = code(await read('server-state-migration.ts'));
  // residentKey is that function. It is the only template literal in the
  // module that joins a key to a server id.
  assert.match(layout, /export function residentKey/u);
  const store = code(await read('server-store.ts'));
  assert.match(store, /residentKey\(/u, 'the store asks for the layout rather than rebuilding it');
  assert.doesNotMatch(store, /`\$\{[^}]*\}::/u, 'and never spells `::` itself');
  // servers.ts keeps its own `parked()` until ②b retires park/point; what must
  // not happen is a THIRD speller appearing in the new modules.
  for (const name of ['server-store.ts', 'server-runtime.ts', 'server-fleet.ts', 'refs.ts']) {
    assert.doesNotMatch(code(await read(name)), /::\$\{/u, `${name} builds a parked key by hand`);
  }
});

test('the store keeps the storage layout and none of the semantics', async () => {
  const src = code(await read('server-store.ts'));
  // What a SeenMark is, that an empty draft removes its row, how stepsRows
  // clamps: all of that stays in hub-prefs and notify-centre. A copy here
  // would be a second definition that drifts.
  for (const leak of ['SeenMark', 'draftUpdate', 'clampStepsRows', 'JSON.parse', 'JSON.stringify']) {
    assert.ok(!src.includes(leak), `${leak} in server-store.ts — the store is a Storage view, not a second store`);
  }
});

test('nothing in ②a is wired into production', async () => {
  // The revised scope: the capability ships, the switch-over is ②b. If any
  // existing module imported one of these, Single mode would have changed and
  // the review would be looking at the wrong diff.
  const added = ['refs.ts', 'server-runtime.ts', 'server-fleet.ts', 'server-store.ts',
    'server-state-migration.ts', 'server-context.ts'];
  const importers: string[] = [];
  for await (const file of glob('src/**/*.{ts,svelte}')) {
    if (/\.(test|fixture)\.(ts|svelte)$/u.test(file) || /\.test\.[a-z]+\.svelte$/u.test(file)) continue;
    if (added.some((a) => file.endsWith(`/app/${a}`))) continue;
    const text = await readFile(file, 'utf8');
    for (const a of added) {
      const base = a.replace(/\.ts$/u, '');
      if (new RegExp(`from '[^']*${base}\\.ts'`, 'u').test(text)) importers.push(`${file} → ${a}`);
    }
  }
  // The only allowed edges are among the five new modules themselves.
  assert.deepEqual(importers, [],
    `②a is capability only; these consumers must wait for ②b:\n  ${importers.join('\n  ')}`);
});
