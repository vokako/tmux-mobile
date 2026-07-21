import {
  addAgentNotificationListener,
  agentNotificationsList,
  agentNotificationsMarkRead,
  removeAgentNotificationListener,
} from './ws.ts';
import { isTeamSession } from './team.svelte.ts';

export interface AgentNotification {
  session: string;
  window: number | string;
  [key: string]: unknown;
}

export const agentNotifications = $state<{ unread: AgentNotification[] }>({ unread: [] });

function applySnapshot(snapshot: { unread?: AgentNotification[] } | null | undefined) {
  agentNotifications.unread = Array.isArray(snapshot?.unread) ? snapshot.unread : [];
}

function onNotification(snapshot: { unread?: AgentNotification[] }) {
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

export function notificationForWindow(session: string, window: number | string): AgentNotification | null {
  return agentNotifications.unread.find(item => item.session === session && Number(item.window) === Number(window)) || null;
}

export function sessionHasNotification(session: string, excludedWindow: number | string | null = null): boolean {
  return agentNotifications.unread.some(item => (
    item.session === session
    && (excludedWindow == null || Number(item.window) !== Number(excludedWindow))
  ));
}

export function otherSessionHasNotification(session: string): boolean {
  return agentNotifications.unread.some(item => item.session !== session);

}

/** Terminal chrome suppresses Team dots without dropping their persisted data. */
export function terminalNotificationForWindow(session: string, window: number | string): AgentNotification | null {
  return isTeamSession(session) ? null : notificationForWindow(session, window);
}

export function otherTerminalSessionHasNotification(session: string): boolean {
  if (isTeamSession(session)) return false;
  return agentNotifications.unread.some(item => (
    item.session !== session && !isTeamSession(item.session)
  ));
}

export async function markWindowRead(session: string, window: number | string | null | undefined) {
  if (!session || window == null || !notificationForWindow(session, window)) return;
  try { applySnapshot(await agentNotificationsMarkRead(session, Number(window))); } catch {}
}
