import test from 'node:test';
import assert from 'node:assert/strict';
import { directoryLoadState } from './file-view-state.js';

const preview = {
  view: 'preview',
  currentFile: { path: '/tmp/readme.md', content: '# kept' },
};

test('reconnect refresh preserves the active file preview', () => {
  const next = directoryLoadState(preview, 'refresh');

  assert.equal(next.view, 'preview');
  assert.equal(next.currentFile, preview.currentFile);
});

test('directory navigation closes the active file preview', () => {
  assert.deepEqual(directoryLoadState(preview, 'navigate'), {
    view: 'list',
    currentFile: null,
  });
});
