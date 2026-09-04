import test from 'node:test';
import assert from 'node:assert/strict';
import { gitStatusMeaning } from './git-status.ts';

const id = (k: string) => k.replace(/^git/u, '').toLowerCase();

test('porcelain codes read as words, index then work tree', () => {
  assert.equal(gitStatusMeaning('??', id), 'untracked');
  assert.equal(gitStatusMeaning('!!', id), 'ignored');
  assert.equal(gitStatusMeaning('M ', id), 'modified, staged');
  assert.equal(gitStatusMeaning(' M', id), 'modified, unstaged');
  assert.equal(gitStatusMeaning('MM', id), 'modified, staged · modified, unstaged');
  assert.equal(gitStatusMeaning('A ', id), 'added, staged');
  assert.equal(gitStatusMeaning(' D', id), 'deleted, unstaged');
  assert.equal(gitStatusMeaning('R ', id), 'renamed, staged');
  assert.equal(gitStatusMeaning('UU', id), 'unmerged, staged · unmerged, unstaged');
});

test('an unknown letter or an empty code never reads blank', () => {
  assert.equal(gitStatusMeaning('X ', id), 'X, staged');
  assert.equal(gitStatusMeaning('  ', id), '  ');
});
