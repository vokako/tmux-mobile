// Shared team-session identity: the ONE place that knows team tmux sessions
// are named `tmm-team-<room>` (room = workspace slug `<basename>-<6hex>`, see
// src-tauri/src/team.rs workspace_slug). Sessions, PanePicker, and Team all
// import from here so the prefix/slug scheme can't drift between surfaces.
//
// Availability lives here too (same pattern as layout.svelte.js): the server
// probe result is global app state, and classification must be consistent
// everywhere — a session is only "team" when the server actually has the bus;
// otherwise tmm-team-* names fall back to ordinary sessions on every surface.

export const TEAM_PREFIX = 'tmm-team-';

const state = $state({
  available: false, // server has the team bus (App's probeTeam decides)
  probed: false,    // false until the first definitive probe answer
});

export const teamState = state;

/** True when `name` is a team session AND the server has the team bus. */
export function isTeamSession(name) {
  return state.available && !!name && name.startsWith(TEAM_PREFIX);
}

/** Room id for a team session name (inverse of teamSessionOf). */
export function teamRoomOf(name) {
  return name.slice(TEAM_PREFIX.length);
}

/** tmux session name for a room id (inverse of teamRoomOf). */
export function teamSessionOf(room) {
  return TEAM_PREFIX + room;
}

/** Human label for a team session: the workspace basename, without the
 *  disambiguating "-<6hex>" suffix workspace_slug appends for tmux safety.
 *  Collisions (two workspaces with the same basename) display alike — the
 *  full room stays in the row's title attribute. */
export function teamLabel(name) {
  return teamRoomOf(name).replace(/-[0-9a-f]{6}$/, '');
}
