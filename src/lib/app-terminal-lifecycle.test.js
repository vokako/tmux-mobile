import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('../App.svelte', import.meta.url), 'utf8');

test('Terminal navigation and page layer exist without an active target', () => {
  assert.match(source, /const t = \['sessions', 'terminal'\]/u);
  assert.match(
    source,
    /<button tabindex="-1" class:active=\{page === 'terminal'[\s\S]*?\{t\('terminal'\)\}[\s\S]*?<\/button>/u,
  );
  assert.match(
    source,
    /<div class="page-layer" class:hidden=\{page !== 'terminal'\}>\s*\{#if terminalTarget\}/u,
  );
  assert.match(source, /\{:else\}\s*<div class="terminal-empty">/u);
});
