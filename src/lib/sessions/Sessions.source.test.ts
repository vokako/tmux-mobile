import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

const source = await readFile(new URL('./Sessions.svelte', import.meta.url), 'utf8');

test('Sessions imports the notification state used by its template', () => {
  assert.match(
    source,
    /import\s*\{[^}]*agentNotifications[^}]*sessionHasNotification[^}]*\}\s*from\s*['"]\.\.\/core\/agent-notifications\.svelte\.ts['"]/s,
  );
});

test('the sidebar creates a project with the shared row, not its own button', () => {
  // ui-unification.md: every sidebar's create affordance is `.side-row.add`
  // (Chat's projects, Agents' defs/skills/MCP). The Terminal sidebar used a
  // full-width bordered button in a bottom bar, which is what made the two
  // sidebars read as different apps even though the dialog behind them is the
  // same one (owner, 2026-08-19).
  assert.match(source, /\{#if !chips\}\s*<button class="side-row add"[\s\S]*?projectNew/u);
  // The page dialect keeps its button — but only there.
  assert.match(source, /\{#if chips\}\s*<button class="new-btn"/u);
});

test('sidebar section headers speak the shared .side-h dialect', () => {
  // Two header styles in ONE column (accent-bold TEAMS/SESSIONS above
  // dense mono PROJECTS) is the drift the shared vocabulary exists to stop.
  // The fix is to WEAR the class, not to restate its properties — restating
  // them is how the tracking ended up at 1.05px next to Chat's 1.4px.
  const labels = source.match(/<div class="group-label"[^>]*>/gu) ?? [];
  assert.ok(labels.length >= 2, 'both group headers exist');
  for (const l of labels) assert.match(l, /class:side-h=\{!chips\}/u);
  const dense = /\.sessions\.sidebar-mode \.group-label \{([\s\S]*?)\}/u.exec(source)?.[1] ?? '';
  assert.doesNotMatch(dense, /font-family|font-size|letter-spacing/u, 'type comes from app.css');
  // A flat list gets a header too, so no group of rows is unlabelled.
  assert.match(source, /\{#if !chips && filtered\.length > 0\}\s*<div class="group-label"/u);
});
