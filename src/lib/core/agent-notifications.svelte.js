import {
  addAgentNotificationListener,
  agentNotificationsList,
  agentNotificationsMarkRead,
  removeAgentNotificationListener,
} from './ws.ts';
import { isTeamSession } from './team.svelte.js';

export const agentNotifications = $state({ unread: [] });

function applySnapshot(snapshot) {
  agentNotifications.unread = Array.isArray(snapshot?.unread) ? snapshot.unread : [];
}

function onNotification(snapshot) {
  applySnapshot(snapshot);
}

let listening = false;

export async function syncAgentNotifications() {
  if (!listening) {
    addAgentNotificationListener(onNotification);
    listening = true;
  }
  try { applySnapshot(await agentNotificationsList()); } catch {}
}

export function stopAgentNotifications() {
  if (listening) removeAgentNotificationListener(onNotification);
  listening = false;
  agentNotifications.unread = [];
}

export function notificationForWindow(session, window) {
  return agentNotifications.unread.find(item => item.session === session && Number(item.window) === Number(window)) || null;
}

export function sessionHasNotification(session, excludedWindow = null) {
  return agentNotifications.unread.some(item => (
    item.session === session
    && (excludedWindow == null || Number(item.window) !== Number(excludedWindow))
  ));
}

export function otherSessionHasNotification(session) {
  return agentNotifications.unread.some(item => item.session !== session);
}

/** Terminal chrome suppresses Team dots without dropping their persisted data. */
export function terminalNotificationForWindow(session, window) {
  return isTeamSession(session) ? null : notificationForWindow(session, window);
}

export function otherTerminalSessionHasNotification(session) {
  if (isTeamSession(session)) return false;
  return agentNotifications.unread.some(item => (
    item.session !== session && !isTeamSession(item.session)
  ));
}

export async function markWindowRead(session, window) {
  if (!session || window == null || !notificationForWindow(session, window)) return;
  try { applySnapshot(await agentNotificationsMarkRead(session, Number(window))); } catch {}
}
