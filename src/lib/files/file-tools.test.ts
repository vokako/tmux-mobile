import test from 'node:test';
import assert from 'node:assert/strict';
import { entryToolActions, visibleToolCount } from './file-tools.ts';

test('toolbar overflow is measured and reserves a slot only when necessary (#164)', () => {
  assert.equal(visibleToolCount(9, 378, 44, 2), 7);
  assert.equal(visibleToolCount(9, 348, 44, 2), 6);
  assert.equal(visibleToolCount(9, 284, 28, 4), 9, 'exact fit has no More button');
  assert.equal(visibleToolCount(9, 283, 28, 4), 7, 'one pixel short needs a More slot');
  assert.equal(visibleToolCount(3, 1000, 44, 2), 3);
  assert.equal(visibleToolCount(9, 44, 44, 2), 0, 'a tiny toolbar retains the More target');
  assert.equal(visibleToolCount(9, 0, 44, 2), 9, 'unmeasured/SSR keeps all named actions');
});

test('entry menus and inline tools share callbacks and the captured entry (#164)', () => {
  const calls: unknown[] = [];
  const entry = { path: '/a/notes.md', name: 'notes.md', type: 'file' };
  const actions = entryToolActions(entry, key => key, {
    open: target => calls.push(['open', target.path]), copy: path => calls.push(['copy', path]),
    download: path => calls.push(['download', path]), rename: target => calls.push(['rename', target.name]),
    remove: path => calls.push(['remove', path]),
  });
  assert.deepEqual(actions.map(a => a.key), ['open', 'copy', 'download', 'rename', 'delete']);
  assert.deepEqual(actions.filter(a => a.inline).map(a => a.key), ['download', 'rename', 'delete']);
  entry.path = '/different'; entry.name = 'different';
  for (const action of actions) action.run();
  assert.deepEqual(calls, [
    ['open', '/a/notes.md'], ['copy', '/a/notes.md'], ['download', '/a/notes.md'],
    ['rename', 'notes.md'], ['remove', '/a/notes.md'],
  ]);
  assert.equal(actions.at(-1)?.danger, true, 'destruction is last, with its semantic tone');
});

test('folders and broken links do not offer file download (#164)', () => {
  const handlers = { open() {}, copy() {}, download() {}, rename() {}, remove() {} };
  for (const type of ['dir', 'broken']) {
    const actions = entryToolActions({ path: '/x', name: 'x', type }, key => key, handlers);
    assert.deepEqual(actions.filter(a => a.inline).map(a => a.key), ['rename', 'delete']);
    assert.equal(actions[0]?.disabled, type === 'broken');
  }
});
