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
export type FeedLevel = 'chat' | 'status' | 'tools';

const stored = localStorage.getItem(FEED_LEVEL_KEY);
const valid = (v: string | null): v is FeedLevel => v === 'chat' || v === 'status' || v === 'tools';

const readLeads = (): Record<string, string> => {
  try {
    const raw = JSON.parse(localStorage.getItem(LEAD_KEY) ?? '{}');
    return raw && typeof raw === 'object' ? raw : {};
  } catch { return {}; }
};

const state = $state({
  // Tools are the default now that a run of them folds into one collapsible
  // row: the reason to hide them was the wall of one-liners, not the content.
  feedLevel: (valid(stored) ? stored : 'tools') as FeedLevel,
  // Per project (tmux session): who the composer addresses by default. Survives
  // reloads because "who am I talking to" is part of where the user left off.
  leads: readLeads() as Record<string, string>,
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
  /** The remembered default recipient for a project, '' when none. */
  lead(session: string) { return state.leads[session] ?? ''; },
  setLead(session: string, name: string) {
    if (name) state.leads[session] = name;
    else delete state.leads[session];
    localStorage.setItem(LEAD_KEY, JSON.stringify(state.leads));
  },
};
