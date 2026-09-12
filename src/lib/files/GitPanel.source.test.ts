import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./GitPanel.svelte', import.meta.url), 'utf8');

test('GitPanel uses the shared feedback contract below its existing header (#167)', () => {
  // Outcome state and paint must not drift back to a private timed string.
  assert.match(source, /import OperationFeedback from '\.\.\/ui\/OperationFeedback\.svelte'/u);
  assert.match(source, /createFeedbackLifetime<FeedbackValue>/u);
  assert.doesNotMatch(source, /pushResult|flashTimer|function flash|git-push-result/u);
  assert.match(source, /<\/div>\s*\{#if feedback\}\s*<div class="git-feedback">/u);
  assert.match(source, /ondismiss=\{feedback\.kind === 'error' \? feedbackLifetime\.clear : undefined\}/u);
  assert.match(source, /\.git-feedback \{ flex: none; \}/u, 'caller owns placement only');
});
