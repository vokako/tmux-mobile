export const TEAM_ACTIVE_ROOM_KEY = 'tmux_team_active_room';

export function readStoredActiveRoom(storage?: Storage | null): string {
  try {
    const target = storage ?? globalThis.localStorage;
    return target?.getItem(TEAM_ACTIVE_ROOM_KEY) || '';
  } catch {
    return '';
  }
}

export function writeStoredActiveRoom(room: string, storage?: Storage | null): void {
  try {
    const target = storage ?? globalThis.localStorage;
    if (!target) return;
    if (room) target.setItem(TEAM_ACTIVE_ROOM_KEY, room);
    else target.removeItem(TEAM_ACTIVE_ROOM_KEY);
  } catch {
    // Storage can be unavailable in private or restricted webviews.
  }
}

type TeamLike = { room?: string } & Record<string, unknown>;

export function pickActiveRoom(teams: TeamLike[] | null | undefined, currentRoom: string, storedRoom: string): string {
  const rooms = new Set((teams || []).map(team => team?.room).filter(Boolean));
  if (rooms.has(currentRoom)) return currentRoom;
  if (rooms.has(storedRoom)) return storedRoom;
  return teams?.find(team => team?.room)?.room || '';
}

/** Trailing slashes off, so `/a/b/` and `/a/b` are the same folder. */
function normPath(p: string): string {
  return p.replace(/\/+$/u, '') || p;
}

/**
 * What the switcher CALLS a team. A room id is `<basename>-<template>-<6hex>`
 * — stable, unique, and unreadable — while the Hub and Projects pages show
 * the project's NAME. The team knows its canonical workspace; the project
 * whose path is that folder names it. Without a matching project the
 * workspace's basename is the name (the same word `teamLabel` gives a team
 * SESSION), and only a team with no workspace at all falls back to the room
 * with its hash suffix dropped. The raw id stays available as a tooltip.
 */
export function teamDisplayName(
  team: { room?: string; workspace?: string } | null | undefined,
  projects: ReadonlyArray<{ name: string; path: string }> = [],
): string {
  if (!team) return '';
  const ws = team.workspace ? normPath(team.workspace) : '';
  if (ws) {
    const hit = projects.find((p) => p.path && normPath(p.path) === ws);
    if (hit?.name) return hit.name;
    const base = ws.slice(ws.lastIndexOf('/') + 1);
    if (base) return base;
  }
  return (team.room || '').replace(/-[0-9a-f]{6}$/u, '');
}
