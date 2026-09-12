import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

test('Files and Terminal both adopt feedback and retire their private toast paint (#167)', async () => {
  for (const file of ['../files/Files.svelte', '../terminal/Terminal.svelte']) {
    const text = await readFile(new URL(file, import.meta.url), 'utf8');
    assert.match(text, /import OperationFeedback from '\.\.\/ui\/OperationFeedback\.svelte'/u, file);
    assert.match(text, /<OperationFeedback\b/u, file);
    assert.doesNotMatch(text, /\.(?:copy-toast|download-toast|toast)\s*\{|@keyframes toast-fade/u, file);
  }
});
