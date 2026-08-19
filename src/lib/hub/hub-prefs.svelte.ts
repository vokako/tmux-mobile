// Hub display preferences — the chat feed's detail level (owner ask:
// configurable in Settings, reachable from the Hub itself).
// Three levels, each a superset of the last:
//   chat   — messages only (what people SAID)
//   status — + status declarations and lifecycle notifications
//   tools  — + individual tool calls ("Edit src/lib.rs")
// Delivery receipts and undelivered-line reports are NOT levelled: they are
// about a message the user sent, so feedBlocks() surfaces them at every level.
const FEED_LEVEL_KEY = 'tmux_hub_feed_level';
const LEAD_KEY = 'tmux_hub_lead';
const SEEN_KEY = 'tmux_hub_seen';
// Which project's conversation was open. "Where I left off" is a project, not
// just a tab: reopening the app on somebody else's chat is the same jolt as
// landing on the wrong tab (owner, 2026-08-19).
const PROJECT_KEY = 'tmux_hub_project';
export type FeedLevel = 'chat' | 'status' | 'tools';

const stored = localStorage.getItem(FEED_LEVEL_KEY);
const valid = (v: string | null): v is FeedLevel => v === 'chat' || v === 'status' || v === 'tools';

const readMap = <T,>(key: string): Record<string, T> => {
  try {
    const raw = JSON.parse(localStorage.getItem(key) ?? '{}');
    return raw && typeof raw === 'object' ? raw : {};
  } catch { return {}; }
};

const state = $state({
  // Tools are the default now that a run of them folds into one collapsible
  // row: the reason to hide them was the wall of one-liners, not the content.
  feedLevel: (valid(stored) ? stored : 'tools') as FeedLevel,
  // Per project (tmux session): who the composer addresses by default. Survives
  // reloads because "who am I talking to" is part of where the user left off.
  leads: readMap<string>(LEAD_KEY),
  // Per project: the newest message timestamp the user has actually seen.
  seen: readMap<number>(SEEN_KEY),
  // The project whose conversation was open, restored if it still exists.
  project: localStorage.getItem(PROJECT_KEY) ?? '',
});

export const hubPrefs = {
  get feedLevel() { return state.feedLevel; },
  setFeedLevel(v: FeedLevel) {
    state.feedLevel = v;
    localStorage.setItem(FEED_LEVEL_KEY, v);
  },
  /** Cycle chat → status → tools → chat, for the Hub's own compact control. */
  cycleFeedLevel() {
    const order: FeedLevel[] = ['chat', 'status', 'tools'];
    this.setFeedLevel(order[(order.indexOf(state.feedLevel) + 1) % order.length]!);
  },
  /** The conversation that was open, '' when none was ever chosen. The caller
   * verifies it still exists — a project can be deleted between two visits. */
  get project() { return state.project; },
  setProject(session: string) {
    state.project = session;
    if (session) localStorage.setItem(PROJECT_KEY, session);
    else localStorage.removeItem(PROJECT_KEY);
  },
  /** Follow a project onto its new tmux session name: the per-project prefs are
   * keyed by that name, so a rename would otherwise silently drop the room's
   * lead and its read marker. */
  renameSession(from: string, to: string) {
    if (!from || !to || from === to) return;
    for (const map of [state.leads, state.seen] as Record<string, unknown>[]) {
      if (from in map) {
        map[to] = map[from];
        delete map[from];
      }
    }
    localStorage.setItem(LEAD_KEY, JSON.stringify(state.leads));
    localStorage.setItem(SEEN_KEY, JSON.stringify(state.seen));
    if (state.project === from) this.setProject(to);
  },
  /** The remembered default recipient for a project, '' when none. */
  lead(session: string) { return state.leads[session] ?? ''; },
  setLead(session: string, name: string) {
    if (name) state.leads[session] = name;
    else delete state.leads[session];
    localStorage.setItem(LEAD_KEY, JSON.stringify(state.leads));
  },
  /** Newest message timestamp (ms) the user has looked at, per project. Drives
   * the "an agent replied" dot, so it has to survive a reload like the rest of
   * "where I left off". */
  seen(session: string) { return state.seen[session] ?? 0; },
  setSeen(session: string, ts: number) {
    state.seen[session] = ts;
    localStorage.setItem(SEEN_KEY, JSON.stringify(state.seen));
  },
};
