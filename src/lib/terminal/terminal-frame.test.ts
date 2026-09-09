import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import type { Terminal as Xterm } from '@xterm/xterm';
import { writeTerminalFrame } from './terminal-frame.ts';

const require = createRequire(import.meta.url);
const { Terminal } = require('@xterm/xterm') as typeof import('@xterm/xterm');
const snapshot = ['history-0', 'history-1', 'screen-0', 'screen-1', 'screen-2'];

function lines(term: Xterm): string[] {
  const buffer = term.buffer.active;
  return Array.from({ length: buffer.length }, (_, i) => buffer.getLine(i)!.translateToString(true));
}

function draw(term: Xterm, content = snapshot, cursorRow = term.rows): Promise<void> {
  const buffer = term.buffer.active;
  const atBottom = buffer.viewportY >= buffer.baseY;
  const previous = buffer.viewportY;
  const body = content.map(line => line + '\x1b[0m\x1b[K').join('\n')
    + `\x1b[0m\x1b[0J\x1b[${cursorRow};1H`;
  return new Promise(resolve => writeTerminalFrame(term, body, () => {
    if (atBottom) term.scrollToBottom();
    else term.scrollToLine(Math.min(previous, term.buffer.active.baseY));
    resolve();
  }));
}

test('a queued rewrite leaves the existing scrollback intact until parsing begins (#109)', async t => {
  const term = new Terminal({ rows: 3, cols: 20, scrollback: 10, convertEol: true });
  t.after(() => term.dispose());
  await draw(term);
  const before = lines(term);
  const writing = draw(term);
  const whileQueued = lines(term);
  await writing;
  assert.deepEqual(whileQueued, before, 'no synchronous clear between the two paints');
  assert.deepEqual(lines(term), before, 'the authoritative snapshot restores the same history');
});

test('rewriting while reading history is not a user return to tail (#109)', async t => {
  const term = new Terminal({ rows: 3, cols: 20, scrollback: 10, convertEol: true });
  t.after(() => term.dispose());
  await draw(term);
  term.scrollToLine(1);
  let atBottom = false;
  let hasNewContent = true;
  let replayRequests = 0;
  // Terminal.svelte's onScroll semantics: false tail events erase the news
  // indicator and enqueue another full rewrite, producing a redraw loop.
  term.onScroll(() => {
    const buffer = term.buffer.active;
    const wasAtBottom = atBottom;
    atBottom = buffer.viewportY >= buffer.baseY;
    if (!wasAtBottom && atBottom) replayRequests++;
    if (atBottom) hasNewContent = false;
  });
  await draw(term);
  assert.equal(replayRequests, 0, 'a snapshot must not queue itself again');
  assert.equal(hasNewContent, true, 'the reader has not returned to new output');
  assert.equal(term.buffer.active.viewportY, 1);
  assert.deepEqual(lines(term), snapshot);
});

test('replaying a complete snapshot is idempotent, not append-only', async t => {
  const term = new Terminal({ rows: 3, cols: 20, scrollback: 10, convertEol: true });
  t.after(() => term.dispose());
  for (let i = 0; i < 4; i++) await draw(term);
  assert.equal(term.buffer.active.baseY, 2);
  assert.deepEqual(lines(term), snapshot, 'history is present once, not once per frame');
});

test('queued frames erase in stream order, even when both were queued on an empty buffer', async t => {
  const term = new Terminal({ rows: 3, cols: 20, scrollback: 10, convertEol: true });
  t.after(() => term.dispose());
  const newer = snapshot.map(line => `new-${line}`);
  const first = draw(term);
  const second = draw(term, newer);
  await Promise.all([first, second]);
  assert.deepEqual(lines(term), newer);
  assert.equal(term.buffer.active.baseY, 2);
});

test('resize/replay keeps a bounded authoritative history, cursor and blank trailing rows', async t => {
  const term = new Terminal({ rows: 3, cols: 20, scrollback: 5, convertEol: true });
  t.after(() => term.dispose());
  const history = Array.from({ length: 10 }, (_, i) => `line-${i}`);
  await draw(term, history);
  assert.deepEqual(lines(term), history.slice(-8), '500 in the app is the same bounded xterm contract');
  term.resize(20, 4);
  await draw(term, [...history, ''], 3);
  assert.equal(term.buffer.active.baseY, 5);
  assert.equal(term.buffer.active.cursorY, 2);
  assert.deepEqual(lines(term), [...history.slice(-8), '']);
  await draw(term, ['prompt', '', '', ''], 1);
  assert.equal(term.buffer.active.baseY, 0, 'no fabricated history beyond what this snapshot supplies');
  assert.equal(term.buffer.active.cursorY, 0);
  assert.deepEqual(lines(term), ['prompt', '', '', '']);
});
