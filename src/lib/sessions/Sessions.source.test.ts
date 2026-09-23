import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Sessions.svelte', import.meta.url), 'utf8');

test('confirmation Back delegates locally without owning browser history (#167, 2026-09-12)', () => {
  // App owns native Back; the nested caller exposes only its local contract.
  assert.doesNotMatch(source, /\b(?:window|document)\.addEventListener\(['"](?:popstate|keydown)['"]/u);
});

test('kill confirmations explicitly distinguish process icons from deletion (#167, 2026-09-12)', () => {
  // Glyph choice belongs to the caller; danger alone cannot mean trash.
  assert.match(source, /confirmIcon=\{pendingKill\?\.kind === 'window' \? 'x' : 'stop'\}/u);
  assert.match(source, /confirmKillSessionAction'\), icon: 'stop'/u);
  assert.match(source, /confirmKillWindowAction'\), icon: 'x'/u);
});

test('the retired unread-notification dots stay retired (2026-09-01)', () => {
  assert.doesNotMatch(source, /agent-notifications\.svelte/u);
  assert.doesNotMatch(source, /attention-dot|sessionHasNotification/u);
});

test('the sidebar creates a project with the shared row, not its own button', () => {
  // ui-unification.md: every sidebar's create affordance is `.side-row.add`
  // (Chat's projects, Agents' defs/skills/MCP). The Terminal sidebar used a
  // full-width bordered button in a bottom bar, which is what made the two
  // sidebars read as different apps even though the dialog behind them is the
  // same one (owner, 2026-08-19).
  assert.match(source, /\{#if !chips && trackedReady && !hasProjects\}\s*<button class="side-row add"[\s\S]*?projectNew/u,
    'the foot row remains only while there is no list to head (owner, 2026-09-23: the command moved to the Projects head)');
  assert.match(source, /oncreate=\{\(\) => showNew = true\}/u, 'the Projects head opens the same dialog');
  // The page dialect keeps its button — but only there.
  assert.match(source, /\{#if chips\}\s*<button class="new-btn"/u);
});

test('sidebar section headers speak the shared .side-h dialect', () => {
  // Two header styles in ONE column (accent-bold group headers above dense
  // mono PROJECTS) is the drift the shared vocabulary exists to stop. The
  // fix is to WEAR the class, not to restate its properties — restating
  // them is how the tracking ended up at 1.05px next to Chat's 1.4px.
  // (One header since board #100 deleted the Team group; the dialect rule
  // is per header, not per count.)
  const labels = source.match(/<div class="group-label"[^>]*>/gu) ?? [];
  assert.ok(labels.length >= 1, 'the group header exists');
  for (const l of labels) assert.match(l, /class:side-h=\{!chips\}/u);
  const dense = /\.sessions\.sidebar-mode \.group-label \{([\s\S]*?)\}/u.exec(source)?.[1] ?? '';
  assert.doesNotMatch(dense, /font-family|font-size|letter-spacing/u, 'type comes from app.css');
  // A flat list gets a header too, so no group of rows is unlabelled.
  assert.match(source, /\{#if !chips && filtered\.length > 0\}\s*<div class="group-label"/u);
});
