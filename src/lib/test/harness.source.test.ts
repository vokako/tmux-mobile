// Board #177 (lead, 2026-09-12): two `npm test` runs on one machine — the
// launch checkout and a worktree — made every render/mount case in one of
// them time out at 60 s with "WebSocket server error: Port 24678 is already
// in use". Vite 6's `createServer` in middleware mode opens its OWN http
// server for the HMR websocket on the fixed default 24678 even with
// `server.hmr: false` — that flag stops the updates, `server.ws: false` is
// what stops the socket (dep-*.js: `if (config.server.ws === false)`).
// Concurrent suites are normal here (several agents, one host), so no
// harness may hold a fixed port. The rule lives in ONE helper.
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';
import test from 'node:test';
import { execFileSync } from 'node:child_process';

const root = fileURLToPath(new URL('../../../', import.meta.url));
const listed = (pattern: string) => execFileSync('rg', ['-l', pattern, 'src', 'scripts', '--glob', '*.ts', '--glob', '*.mjs', '--glob', '*.js'], { cwd: root, encoding: 'utf8' })
  .split('\n').filter((f) => f && !f.endsWith('harness.source.test.ts')).sort();

test('the ONE Vite SSR helper opens no socket: ws:false, hmr:false, no port', async () => {
  const ssr = await readFile(new URL('./ssr.ts', import.meta.url), 'utf8');
  assert.match(ssr, /server:\s*\{\s*middlewareMode:\s*true,\s*hmr:\s*false,\s*ws:\s*false\s*\}/u, 'middleware mode, no HMR, and NO websocket server');
  assert.doesNotMatch(ssr, /port\s*:/u, 'no fixed port anywhere in the helper');
  assert.match(ssr, /appType:\s*'custom'/u);
});

test('every render harness goes through the helper; nothing else in the test tree creates a Vite server', () => {
  const creators = listed('createServer\\(');
  assert.deepEqual(creators, ['src/lib/test/ssr.ts'], 'createServer( exists once, in the helper');
  const renderTests = listed('\\.render\\.test\\.ts|ssrLoadModule').filter((f) => f.endsWith('.render.test.ts'));
  assert.ok(renderTests.length >= 7, `render tests found: ${renderTests.length}`);
  for (const f of renderTests) {
    const src = execFileSync('cat', [f], { cwd: root, encoding: 'utf8' });
    assert.match(src, /import \{ ssrServer \} from '[./]*lib\/test\/ssr\.ts'|import \{ ssrServer \} from '\.\.\/test\/ssr\.ts'/u, `${f} uses the helper`);
    assert.doesNotMatch(src, /from 'vite'/u, `${f} does not reach for vite itself`);
  }
});

test('the mount harness builds; it never serves', async () => {
  const mount = await readFile(new URL('./mount.ts', import.meta.url), 'utf8');
  assert.match(mount, /await build\(\{/u);
  assert.doesNotMatch(mount, /createServer|server:\s*\{|hmr|24678/u, 'a build has no listener to collide on');
});
