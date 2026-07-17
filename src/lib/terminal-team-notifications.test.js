import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const notifications = await readFile(
  new URL('./agent-notifications.svelte.js', import.meta.url),
  'utf8',
);
const terminal = await readFile(new URL('./Terminal.svelte', import.meta.url), 'utf8');
const picker = await readFile(new URL('./PanePicker.svelte', import.meta.url), 'utf8');

test('Terminal notification queries retain data but suppress Team sessions', () => {
  assert.match(
    notifications,
    /terminalNotificationForWindow\(session, window\)[\s\S]*?isTeamSession\(session\) \? null : notificationForWindow\(session, window\)/u,
  );
  assert.match(
    notifications,
    /otherTerminalSessionHasNotification\(session\)[\s\S]*?if \(isTeamSession\(session\)\) return false;[\s\S]*?!isTeamSession\(item\.session\)/u,
  );
});

test('Terminal chrome uses only Team-filtered notification queries', () => {
  assert.match(terminal, /attention=\{otherTerminalSessionHasNotification\(session\)\}/u);
  assert.match(
    terminal,
    /\{@const notice = terminalNotificationForWindow\(w\.session, w\.window\)\}/u,
  );
  assert.doesNotMatch(terminal, /\bnotificationForWindow\(/u);
});

test('pane picker suppresses Team summary and window dots', () => {
  assert.match(
    picker,
    /\{#if !team && s\.name !== currentSession && sessionHasNotification\(s\.name\)\}/u,
  );
  assert.match(
    picker,
    /\{@const notice = terminalNotificationForWindow\(p\.session, p\.window\)\}/u,
  );
});
