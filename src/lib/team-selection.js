export const TEAM_ACTIVE_ROOM_KEY = 'tmux_team_active_room';

export function readStoredActiveRoom(storage) {
  try {
    const target = storage ?? globalThis.localStorage;
    return target?.getItem(TEAM_ACTIVE_ROOM_KEY) || '';
  } catch {
    return '';
  }
}

export function writeStoredActiveRoom(room, storage) {
  try {
    const target = storage ?? globalThis.localStorage;
    if (!target) return;
    if (room) target.setItem(TEAM_ACTIVE_ROOM_KEY, room);
    else target.removeItem(TEAM_ACTIVE_ROOM_KEY);
  } catch {
    // Storage can be unavailable in private or restricted webviews.
  }
}

export function pickActiveRoom(teams, currentRoom, storedRoom) {
  const rooms = new Set((teams || []).map(team => team?.room).filter(Boolean));
  if (rooms.has(currentRoom)) return currentRoom;
  if (rooms.has(storedRoom)) return storedRoom;
  return teams?.find(team => team?.room)?.room || '';
}
