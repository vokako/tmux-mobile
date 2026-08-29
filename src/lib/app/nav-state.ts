// Which tab a reload lands on. Framework-free so the rule is testable: it is a
// two-line decision that used to be wrong in a way nobody notices until they
// reload on the tab they were reading (owner, 2026-08-19: "每次切换或者刷新都会变").

/** The tabs a saved state may name. A stale key from an older build — or a
 * retired tab, like the Team one the Hub replaced — must not strand the app on
 * a page that no longer renders. */
export const PAGES = ['terminal', 'hub', 'files', 'board', 'sessions', 'agents', 'team', 'prefs', 'settings'] as const;
export type Page = (typeof PAGES)[number];

/** The default when there is nothing to restore: a phone opens on the terminal
 * (it is the reason the app exists on a phone), a desktop on the Hub. */
export function defaultPage(isTouchDevice: boolean): Page {
  return isTouchDevice ? 'terminal' : 'hub';
}

/**
 * The tab to restore. `saved` is whatever was in localStorage, so it is
 * untrusted: anything that is not a known page falls back to the default.
 *
 * `settings` is deliberately restorable — it is a real page (the connect card
 * lives there), and a user who left it open is not lost.
 */
export function restorePage(saved: unknown, isTouchDevice: boolean): Page {
  return typeof saved === 'string' && (PAGES as readonly string[]).includes(saved)
    ? (saved as Page)
    : defaultPage(isTouchDevice);
}

/**
 * Whether the agent configuration is a CATEGORY OF SETTINGS rather than a page
 * of its own — true on touch devices (owner, 2026-08-29: "手机上的 Agent 设置
 * 页面应该归到 settings 里边的一个子页面，不用单独在底下一行展示了，现在看着
 * 有点多底下的标签").
 *
 * One definition, because four places have to agree or the page becomes
 * unreachable in one of them: the bottom tab bar (no icon), the swipe order (not
 * a stop), the Settings category list (a row), and the Hub's "configure agent"
 * jump. The desktop rail is untouched — there Agents is a page with its own
 * draggable icon.
 */
export function agentsLivesInSettings(isTouchDevice: boolean): boolean {
  return isTouchDevice;
}

/** A Settings category that a saved page name resolves into, or null. */
export type SettingsCategory = 'agents';

/**
 * Restoring navigation, which is not always restoring a PAGE: on touch,
 * `agents` names a page that has no way in and no way out — no tab icon, no
 * swipe stop — so a saved `agents` (an older build, or the same profile opened
 * on a phone) must come back as Settings opened at its Agents category rather
 * than as a page layer nobody can leave.
 */
export function restoreNav(
  saved: unknown,
  isTouchDevice: boolean,
): { page: Page; settingsTab: SettingsCategory | null } {
  if (saved === 'agents' && agentsLivesInSettings(isTouchDevice)) {
    return { page: 'prefs', settingsTab: 'agents' };
  }
  return { page: restorePage(saved, isTouchDevice), settingsTab: null };
}

/**
 * Retarget a `session:window.pane` reference after its session was renamed.
 *
 * A project rename renames the tmux session, so every saved target that names the
 * old one stops resolving — including the pane currently on screen. Only an exact
 * session-name prefix is rewritten: `old:1.0` moves, `older:1.0` does not, and a
 * target for some other session is returned untouched.
 */
export function retarget(target: string, from: string, to: string): string {
  if (!target || !from || !to || from === to) return target;
  return target.startsWith(`${from}:`) ? `${to}:${target.slice(from.length + 1)}` : target;
}
