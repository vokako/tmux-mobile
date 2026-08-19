// Source-contract test for destructive actions (see docs/conventions/testing.md).
//
// The 2026-08-19 audit ("所有的 close delete 之类的按钮都检查一下二次确认") found
// FOUR different answers to "are you sure?" in one app: a modal (the Hub), a
// button you click twice within 3s (Files, Sessions, Team, TeamTemplates,
// Projects), the browser's own confirm() — an OS dialog in the middle of our UI,
// the same seam a native <select> opens — and nothing at all (deleting an agent
// definition, a skill, an MCP server, a downloaded file).
//
// One shape now: src/lib/ui/ConfirmDialog.svelte. These tests keep the two
// retired patterns from coming back, because both look perfectly reasonable
// while you are writing them.
import test from 'node:test';
import assert from 'node:assert/strict';
import { readdir, readFile } from 'node:fs/promises';

const SRC = new URL('../../', import.meta.url);

async function* walk(dir: URL): AsyncGenerator<URL> {
  for (const entry of await readdir(dir, { withFileTypes: true })) {
    const child = new URL(entry.name + (entry.isDirectory() ? '/' : ''), dir);
    if (entry.isDirectory()) yield* walk(child);
    else if (entry.name.endsWith('.svelte')) yield child;
  }
}

const files: { rel: string; text: string }[] = [];
for await (const f of walk(SRC)) {
  files.push({ rel: f.href.slice(SRC.href.length), text: await readFile(f, 'utf8') });
}

/** Code only: these files DISCUSS the retired patterns in comments, and a test
 * that cannot tell a mention from a call is a test nobody can satisfy. */
const code = (text: string) =>
  text
    .replace(/<!--[\s\S]*?-->/gu, '')      // markup comments
    .replace(/\/\*[\s\S]*?\*\//gu, '')      // block comments
    .replace(/^\s*\/\/.*$/gmu, '')          // line comments
    .replace(/on[Cc]onfirm|confirmLabel|ConfirmDialog/gu, '');   // our own names

test('no component calls the browser confirm/alert — that is an OS dialog', () => {
  const offenders = files
    .filter(({ text }) => /(?:^|[^.\w])(?:window\.)?(?:confirm|alert)\s*\(/mu.test(code(text)))
    .map(({ rel }) => rel);
  assert.deepEqual(offenders, [], 'use ConfirmDialog instead');
});

test('no component re-grows tap-to-confirm', () => {
  // The pattern was: a `confirm…` flag armed for 3 seconds, with the SAME button
  // meaning "arm" and then "do it". It says nothing about what is lost, and on a
  // phone the second tap is easy to hit by accident.
  const offenders = files
    .filter(({ text }) => /class:confirm=/u.test(text))
    .map(({ rel }) => rel);
  assert.deepEqual(offenders, [], 'a destructive verb opens ConfirmDialog');
});

test('every component with a destructive verb has a confirmation', async () => {
  // A page that can delete/kill/close something must import the dialog. Keyed on
  // the RPC verbs rather than on button labels, because the label is a
  // translation and the call is the thing that actually destroys.
  const destructive = /\b(fsDelete|killSession|killWindow|registryDelete|skillsDelete|mcpDelete|projectDelete|projectDown|projectArchive|hubAgentRemove|hubAgentStop|teamCloseTeam|delete_download)\b/u;
  const offenders = files
    .filter(({ text }) => destructive.test(text) && !text.includes('ConfirmDialog'))
    .map(({ rel }) => rel);
  assert.deepEqual(offenders, [], 'these destroy something without confirming it');
});
