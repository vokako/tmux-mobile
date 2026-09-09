import test from 'node:test';
import assert from 'node:assert/strict';
import { createRequire } from 'node:module';
import { createTerminalResponseFilter } from './terminal-responses.ts';

const responses = [
  '\x1b[?62;22;52c', '\x1b[?1;2c', '\x1b[>0;276;0c', '\x1b[=1;2c',
  '\x1b[0c', '\x1b[12;34R', '\x1b[0n',
];

test('complete DA and DSR replies are filtered', () => {
  const filter = createTerminalResponseFilter();
  for (const response of responses) assert.equal(filter.push(response), '');
});

test('DA and DSR replies split at every boundary do not reach input (#108)', () => {
  for (const response of responses) {
    for (let split = 1; split < response.length; split++) {
      const filter = createTerminalResponseFilter();
      assert.equal(filter.push(response.slice(0, split)), '', `prefix ${JSON.stringify(response)} at ${split}`);
      assert.equal(filter.push(response.slice(split)), '', `suffix ${JSON.stringify(response)} at ${split}`);
      assert.equal(filter.push('typed'), 'typed', 'no stale carry after a complete reply');
    }
    const filter = createTerminalResponseFilter();
    assert.equal([...response].map(char => filter.push(char)).join(''), '', 'one byte per payload');
  }
});

test('ordinary input and complete non-response keys are forwarded immediately', () => {
  const filter = createTerminalResponseFilter();
  for (const input of [
    'abc', '\u4f60\u597d', '?62;22;52c', '\x03', '\r', '\x7f',
    '\x1b[A', '\x1b[1;5D', '\x1b[15~', '\x1bOP', '\x1bx',
    '\x1b[200~paste\x1b[201~', '\x1b[<0;12;3M',
  ]) assert.equal(filter.push(input), input, JSON.stringify(input));
});

test('responses embedded in other data are removed without dropping surrounding input', () => {
  const filter = createTerminalResponseFilter();
  assert.equal(filter.push(`before${responses[0]}middle${responses[1]}after`), 'beforemiddleafter');
  assert.equal(filter.push('text\x1b[?62;'), 'text');
  assert.equal(filter.push('22;52cnext'), 'next');
});

test('incomplete non-response keys are released unchanged once disambiguated', () => {
  const filter = createTerminalResponseFilter();
  assert.equal(filter.push('\x1b'), '');
  assert.equal(filter.push('x'), '\x1bx');
  assert.equal(filter.push('\x1b[1;'), '');
  assert.equal(filter.push('5D'), '\x1b[1;5D');
  assert.equal(filter.push('\x1b[?12;'), '');
  assert.equal(filter.push('oops'), '\x1b[?12;oops');
  assert.equal(filter.push('\x1b[12;\x1b[?1;2c'), '\x1b[12;', 'a new ESC starts its own candidate');
});

test('carry belongs to one terminal and reset drops it at the pane boundary', () => {
  const first = createTerminalResponseFilter();
  const second = createTerminalResponseFilter();
  assert.equal(first.push('\x1b[?62;22'), '');
  assert.equal(second.push(';52c'), ';52c');
  first.reset();
  assert.equal(first.push('new pane'), 'new pane');
  assert.equal(first.push('\x1b[?1;2c'), '');
});

test('paste is literal and does not consume a response fragment waiting on the data channel', () => {
  const filter = createTerminalResponseFilter();
  assert.equal(filter.push('\x1b[?62;22'), '');
  assert.equal(filter.push('\x1b[?1;2c pasted', true), '\x1b[?1;2c pasted');
  assert.equal(filter.push(';52c'), '');
});

test('unbounded numeric input cannot grow the carry buffer forever', () => {
  const filter = createTerminalResponseFilter();
  const input = '\x1b[' + '1'.repeat(4096);
  const forwarded = [...input].map(char => filter.push(char)).join('');
  assert.ok(forwarded.length > 0, 'an overlong candidate must be released');
  assert.equal(forwarded + filter.push('x'), input + 'x', 'overflow is lossless');
});

// The published browser package also runs its parser without open()/a DOM.
// Use its public APIs, not a mock event emitter or private core service.
const require = createRequire(import.meta.url);
const { Terminal } = require('@xterm/xterm') as typeof import('@xterm/xterm');

test('real xterm 6 emits whole query replies; injected input splits traverse its real onData path', async t => {
  const term = new Terminal();
  t.after(() => term.dispose());
  const filter = createTerminalResponseFilter();
  const chunks: string[] = [];
  const forwarded: string[] = [];
  term.onData(data => {
    chunks.push(data);
    const input = filter.push(data);
    if (input) forwarded.push(input);
  });
  // Splitting the QUERY across writes does not split xterm's RESPONSE.
  for (const chunk of ['\x1b[', 'c', '\x1b[>', 'c', '\x1b[5', 'n', '\x1b[6n']) {
    await new Promise<void>(resolve => term.write(chunk, resolve));
  }
  assert.deepEqual(chunks, ['\x1b[?1;2c', '\x1b[>0;276;0c', '\x1b[0n', '\x1b[1;1R']);
  assert.deepEqual(forwarded, []);
  chunks.length = 0;
  term.input('\x1b[?62;22', false);
  term.input(';52c', false);
  assert.deepEqual(chunks, ['\x1b[?62;22', ';52c'], 'synthetic split, real event path');
  assert.deepEqual(forwarded, []);
  term.input('\x1b[A');
  term.input('typed');
  assert.deepEqual(forwarded, ['\x1b[A', 'typed'], 'the same channel still sends user input');
});
