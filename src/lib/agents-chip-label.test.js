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
