import test from 'node:test';
import assert from 'node:assert/strict';
import { configFingerprint, configPayload, configValid, type ConfigDraft, type AgentDraft } from './config-draft.ts';

const agent: AgentDraft = { name: 'alpha', backend: 'codex', model: '', system: 'Original',
  skillSel: ['missing'], mcpSel: ['remote'], mcpExtra: [{ expanded: true, nested: { token: 'fixture' } }] };

test('agent payload preserves unknown references and inline MCP values', () => {
  const value = configPayload({ kind: 'agent', value: agent }) as { skills: string; mcp: string };
  assert.deepEqual(JSON.parse(value.skills), ['missing']);
  assert.deepEqual(JSON.parse(value.mcp), ['remote', ...agent.mcpExtra]);
});

test('team disclosure state is not a draft change, but nested MCP expanded is data', () => {
  const draft: ConfigDraft = { kind: 'team', value: {
    name: 'squad', description: '', members: [{ name: 'dev', base: '', role: '', expanded: true, agent: structuredClone(agent) }],
  } };
  const original = configFingerprint(draft);
  draft.value.members[0]!.expanded = false;
  assert.equal(configFingerprint(draft), original);
  draft.value.members[0]!.agent!.mcpExtra = [{ expanded: false }];
  assert.notEqual(configFingerprint(draft), original);
});

test('inherited team members never serialize bare-only configuration', () => {
  const draft: ConfigDraft = { kind: 'team', value: { name: 'squad', description: '', members: [
    { name: 'dev', base: 'alpha', role: 'review', model: 'model', effort: 'low', expanded: true, agent },
    { name: 'unused', base: 'unused', team: 'other', role: 'brief', expanded: false, agent },
  ] } };
  const members = JSON.parse((configPayload(draft) as { members: string }).members);
  assert.equal(members[0].agent, null);
  assert.equal(members[0].model, 'model');
  assert.equal(members[1].name, '');
  assert.equal(members[1].base, '');
  assert.equal(members[1].agent, null);
});

test('MCP JSON must parse; invalid text is still dirty and valid formatting is not', () => {
  const a: ConfigDraft = { kind: 'mcp', value: { name: 'test', defText: '{"command":"x"}' } };
  const original = configFingerprint(a);
  a.value.defText = '{\n  "command": "x"\n}';
  assert.equal(configFingerprint(a), original);
  a.value.defText = '{';
  assert.equal(configValid(a, false), false);
  assert.notEqual(configFingerprint(a), original);
});

test('skill imports allow an unnamed source but built-ins are not editable', () => {
  const draft: ConfigDraft = { kind: 'skill', value: { name: '', source: '/fixture', description: '' } };
  assert.equal(configValid(draft, true), true);
  assert.equal(configValid(draft, false), false);
  draft.value.name = 'builtin'; draft.value.source = 'builtin';
  assert.equal(configValid(draft, false), false);
});

test('global instructions cannot overwrite from an unfinished/failed read or oversized text', () => {
  const draft: ConfigDraft = { kind: 'global', value: { text: 'x', loading: true, max_bytes: 2 } };
  assert.equal(configValid(draft, false), false);
  draft.value.loading = false; draft.value.failed = true;
  assert.equal(configValid(draft, false), false);
  draft.value.failed = false;
  assert.equal(configValid(draft, false), true);
  draft.value.text = 'long';
  assert.equal(configValid(draft, false), false);
});
