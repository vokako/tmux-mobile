import test from 'node:test';
import assert from 'node:assert/strict';
import { paneAgent, paneChipLabel } from './agents.js';

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
