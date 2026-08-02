// Hub display preferences — the chat feed's detail level (owner ask:
// configurable in Settings). Three levels, each a superset of the last:
//   chat   — messages only (what people SAID)
//   status — + status declarations and lifecycle notifications
//   tools  — + individual tool calls ("Edit src/lib.rs")
const FEED_LEVEL_KEY = 'tmux_hub_feed_level';
export type FeedLevel = 'chat' | 'status' | 'tools';

const stored = localStorage.getItem(FEED_LEVEL_KEY);
const valid = (v: string | null): v is FeedLevel => v === 'chat' || v === 'status' || v === 'tools';

const state = $state({
  feedLevel: (valid(stored) ? stored : 'status') as FeedLevel,
});

export const hubPrefs = {
  get feedLevel() { return state.feedLevel; },
  setFeedLevel(v: FeedLevel) {
    state.feedLevel = v;
    localStorage.setItem(FEED_LEVEL_KEY, v);
  },
};
