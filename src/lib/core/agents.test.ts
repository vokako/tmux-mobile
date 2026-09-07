import test from 'node:test';
import assert from 'node:assert/strict';
import { paneAgent, paneChipLabel } from './agents.ts';

test('recognized agent panes use icon-only chip labels', () => {
  const panes = [
    { current_command: 'kiro-cli-chat' },
    { current_command: 'node', child_cmd: 'codex' },
    { current_command: '2.1.141' },
  ];
  for (const pane of panes) {
    assert.ok(paneAgent(pane));
    assert.equal(paneChipLabel(pane, '0.0'), '');
  }
});

test('ordinary panes keep their process or fallback label', () => {
  assert.equal(paneChipLabel({ current_command: 'zsh' }, '0.0'), 'zsh');
  assert.equal(paneChipLabel({ window_name: 'logs' }, '0.0'), 'logs');
  assert.equal(paneChipLabel({}, '0.0'), '0.0');
});

// kimi-code spawns a "kiro-web-search" helper — without the Kimi entry the
// /kiro/ match in the child chain painted the pane as Kiro.
test('kimi-code wins over a kiro-* tool in its child chain', () => {
  const pane = { current_command: 'kimi-code', child_cmd: 'uv tool uvx kiro-web-search' };
  assert.equal(paneAgent(pane)?.tag, 'Kimi');
});

// oh-my-pi's CLI is a single `omp` binary, and "omp" is a substring of
// everyday process text (docker-compose, a component build). The entry is
// word-bounded: real omp panes match, compose panes stay ordinary shells.
test('omp is detected as a word, never inside compose', () => {
  assert.equal(paneAgent({ current_command: 'omp' })?.tag, 'OMP');
  assert.equal(paneAgent({ current_command: 'sh', child_cmd: '/home/u/.local/bin/omp --continue' })?.tag, 'OMP');
  assert.equal(paneAgent({ current_command: 'docker-compose' }), null);
  assert.equal(paneAgent({ current_command: 'node', pane_title: 'component-lab' }), null);
});

// The same boundary rule protects every short brand name: a window named
// after the kirocrew project is not a Kiro agent, but the real launch
// spellings (kiro-cli-chat, codex.js) keep their `-`/`.` boundaries.
test('brand needles are word-bounded on every backend', () => {
  assert.equal(paneAgent({ current_command: 'bash', pane_title: 'kirocrew-in-agentcore' }), null);
  assert.equal(paneAgent({ current_command: 'kiro-cli-chat' })?.tag, 'Kiro');
  assert.equal(paneAgent({ current_command: 'node', child_cmd: 'node /usr/lib/node_modules/codex/bin/codex.js' })?.tag, 'Codex');
});
