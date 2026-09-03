import test from 'node:test';
import assert from 'node:assert/strict';
import { directoryLoadState, leaveDecision, cwdFollowStep } from './file-view-state.ts';

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

test('only an EDITED editor asks before leaving; every other view just goes', () => {
  assert.equal(leaveDecision({ view: 'edit', edited: true }), 'ask');
  assert.equal(leaveDecision({ view: 'edit', edited: false }), 'go');
  assert.equal(leaveDecision({ view: 'preview', edited: false }), 'go');
  // `edited` is derived from view === 'edit' in Files; a stale flag on another
  // view must still not block navigation.
  assert.equal(leaveDecision({ view: 'list', edited: true }), 'go');
});

test('the cwd follow is disarmed when it asks — a cancelled follow is skipped, not queued', () => {
  const editing = { view: 'edit', edited: true };
  // The real cwd moved while the user was editing: the move must ask…
  const step = cwdFollowStep('/proj/b', '/proj/a', editing);
  assert.equal(step.move, 'ask');
  // …and the new cwd is recorded BEFORE the answer, so when the user cancels
  // and the effect re-runs (tab shown again, session prop re-fires) the same
  // cwd is a no-op instead of a second dialog.
  assert.equal(step.lastSourceDir, '/proj/b');
  assert.deepEqual(cwdFollowStep('/proj/b', step.lastSourceDir, editing), { lastSourceDir: '/proj/b', move: 'none' });
  // A LATER move to a different cwd asks again.
  assert.equal(cwdFollowStep('/proj/c', step.lastSourceDir, editing).move, 'ask');
});

test('the cwd follow moves at once when nothing is at stake', () => {
  assert.deepEqual(cwdFollowStep('/home/u', '', { view: 'list', edited: false }), { lastSourceDir: '/home/u', move: 'go' });
  assert.deepEqual(cwdFollowStep('', '/x', { view: 'list', edited: false }), { lastSourceDir: '/x', move: 'none' }, 'an empty report is not a move');
  assert.deepEqual(cwdFollowStep('/x', '/x', { view: 'edit', edited: true }), { lastSourceDir: '/x', move: 'none' }, 'unchanged cwd never asks');
});
