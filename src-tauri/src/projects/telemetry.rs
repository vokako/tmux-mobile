//! Agent telemetry: the passive half of the v2 dual channel.
//!
//! What an agent SAYS goes through the `tmm` CLI (hub_post / hub_status /
//! hub_done). What we OBSERVE arrives here: hook notifications (it stopped, it
//! wants permission) and tmux window activity. Status is DERIVED from those
//! facts at read time — an agent never fills in a form, and a backend with
//! poor hook coverage (codex) degrades to pane-activity granularity instead
//! of lying. A finished turn (Stop) is REST, not distress: "stuck" detection
//! by stop-without-done was a Team-supervisor-era rule and mislabeled every
//! long-idle direct agent — the only distress we can honestly observe is a
//! failed stop. See docs/exec-plans/agents-v2.md §4.1/§4.3.
//!
//! The store is a process-global map keyed by (session, window index) — the
//! same granularity as a project slot and a hook notification. Records are
//! small and bounded by the number of live windows; entries for windows that
//! no longer exist are dropped opportunistically on write.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Tool activity within this window means "working".
const ACTIVE_SECS: u64 = 30;
/// An explicit `tmm status` declaration expires after this long so a crashed
/// agent cannot stay "working" forever on its own last words.
const EXPLICIT_TTL_SECS: u64 = 30 * 60;

#[derive(Debug, Clone, Default)]
struct Rec {
    /// Explicit declaration via `tmm status <state> [note]`: (state, note, ts).
    explicit: Option<(String, String, u64)>,
    /// `tmm done [summary]`: (summary, ts).
    done: Option<(String, u64)>,
    /// Last hook notification: (kind, ts). Kinds are the normalized ones from
    /// agent_notifications (completed / failed / permission_required /
    /// input_required).
    notif: Option<(String, u64)>,
    /// Last hook tool event (pre/postToolUse from an isolated-home agent):
    /// (activity line, ts).
    tool: Option<(String, u64)>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentStatus {
    /// Derived state: working | waiting | idle | failed.
    pub state: String,
    /// Human line explaining the state (explicit note, notification kind, or
    /// the last observed tool activity).
    pub detail: String,
    /// Timestamp of the fact the state was derived from.
    pub since: u64,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

fn store() -> &'static Mutex<HashMap<(String, usize), Rec>> {
    static STORE: OnceLock<Mutex<HashMap<(String, usize), Rec>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_rec(session: &str, window: usize, f: impl FnOnce(&mut Rec)) {
    let mut map = store().lock().unwrap();
    f(map.entry((session.to_string(), window)).or_default());
}

/// `tmm status <state> [note]` — explicit declaration by the agent.
pub fn record_status(session: &str, window: usize, state: &str, note: &str) {
    let (state, note, ts) = (state.to_string(), note.to_string(), now());
    with_rec(session, window, |r| r.explicit = Some((state, note, ts)));
}

/// `tmm done [summary]` — completion declared by the agent.
pub fn record_done(session: &str, window: usize, summary: &str) {
    let (summary, ts) = (summary.to_string(), now());
    with_rec(session, window, |r| {
        r.done = Some((summary, ts));
        r.explicit = None; // done supersedes any earlier declaration
    });
}

/// A hook notification consumed by the AgentNotificationHub (it stopped, it
/// wants input…). Called from the hub's inbox consumer; must stay cheap.
pub fn record_notification(session: &str, window: usize, kind: &str, ts: u64) {
    let kind = kind.to_string();
    with_rec(session, window, |r| r.notif = Some((kind, ts)));
}

/// A hook tool event (isolated-home agents only, Phase B+): "Edit foo.rs".
pub fn record_tool(session: &str, window: usize, line: &str) {
    let (line, ts) = (line.to_string(), now());
    with_rec(session, window, |r| r.tool = Some((line, ts)));
}

/// Drop records for windows that no longer exist (called opportunistically
/// with the live window set whenever someone lists a session's agents).
pub fn retain_windows(session: &str, live: &[usize]) {
    let mut map = store().lock().unwrap();
    map.retain(|(s, w), _| s != session || live.contains(w));
}

/// Derive the current status for (session, window). `activity_ts` is tmux's
/// window_activity for the window; it is the fallback signal for backends
/// without tool hooks. Pure given the record + clock, so the rules read as a
/// table (§4.3): latest fact wins, ties broken by specificity.
pub fn derive(session: &str, window: usize, activity_ts: u64) -> AgentStatus {
    let rec = store()
        .lock()
        .unwrap()
        .get(&(session.to_string(), window))
        .cloned()
        .unwrap_or_default();
    derive_from(&rec, activity_ts, now())
}

fn derive_from(rec: &Rec, activity_ts: u64, now: u64) -> AgentStatus {
    let done_ts = rec.done.as_ref().map(|(_, t)| *t).unwrap_or(0);
    let notif_ts = rec.notif.as_ref().map(|(_, t)| *t).unwrap_or(0);
    let tool_ts = rec.tool.as_ref().map(|(_, t)| *t).unwrap_or(0);

    // Fresh tool activity is the strongest working signal — it is an observed
    // fact, newer facts first.
    if now.saturating_sub(tool_ts) < ACTIVE_SECS && tool_ts >= notif_ts && tool_ts >= done_ts {
        let line = rec.tool.as_ref().map(|(l, _)| l.clone()).unwrap_or_default();
        return AgentStatus { state: "working".into(), detail: line, since: tool_ts };
    }

    // An explicit declaration wins while fresh and not superseded by a newer
    // stop/done fact.
    if let Some((state, note, ts)) = &rec.explicit {
        if now.saturating_sub(*ts) < EXPLICIT_TTL_SECS && *ts >= notif_ts && *ts >= done_ts {
            return AgentStatus { state: state.clone(), detail: note.clone(), since: *ts };
        }
    }

    // done → idle, unless something happened after it.
    if done_ts > 0 && done_ts >= notif_ts {
        let summary = rec.done.as_ref().map(|(s, _)| s.clone()).unwrap_or_default();
        // New pane activity after done means the agent picked up new work.
        if activity_ts > done_ts && now.saturating_sub(activity_ts) < ACTIVE_SECS {
            return AgentStatus { state: "working".into(), detail: String::new(), since: activity_ts };
        }
        return AgentStatus { state: "idle".into(), detail: summary, since: done_ts };
    }

    // Hook notification is the freshest fact. Newer pane activity resolves
    // any of them: a prompt that got answered (or an agent that moved on)
    // shows as working, and self-corrects back if the activity was noise.
    if let Some((kind, ts)) = &rec.notif {
        if activity_ts > *ts && now.saturating_sub(activity_ts) < ACTIVE_SECS {
            return AgentStatus { state: "working".into(), detail: String::new(), since: activity_ts };
        }
        match kind.as_str() {
            "permission_required" | "input_required" => {
                return AgentStatus { state: "waiting".into(), detail: kind.clone(), since: *ts };
            }
            "failed" => {
                // The one genuine distress signal we can observe: the agent's
                // stop hook reported failure.
                return AgentStatus { state: "failed".into(), detail: kind.clone(), since: *ts };
            }
            _ => {
                // completed (Stop) = the agent finished a TURN. That is REST,
                // not distress: direct agents fire Stop after every exchange
                // and never call `tmm done`, so treating a quiet stop as
                // "stuck" branded every long-idle window as broken (owner
                // report, 2026-08-01). "It stopped without done" carries no
                // alarm on its own — the old rule came from the Team
                // supervisor world where done was contractual.
                return AgentStatus { state: "idle".into(), detail: String::new(), since: *ts };
            }
        }
    }

    // No facts at all: fall back to pane activity.
    if now.saturating_sub(activity_ts) < ACTIVE_SECS {
        return AgentStatus { state: "working".into(), detail: String::new(), since: activity_ts };
    }
    AgentStatus { state: "idle".into(), detail: String::new(), since: activity_ts }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> Rec {
        Rec::default()
    }

    #[test]
    fn fresh_tool_activity_means_working_with_the_activity_line() {
        let mut r = rec();
        r.tool = Some(("Edit src/lib.rs".into(), 1000));
        let s = derive_from(&r, 0, 1010);
        assert_eq!(s.state, "working");
        assert_eq!(s.detail, "Edit src/lib.rs");
    }

    #[test]
    fn explicit_declaration_wins_while_fresh() {
        let mut r = rec();
        r.explicit = Some(("waiting".into(), "等接口定稿".into(), 1000));
        let s = derive_from(&r, 0, 1100);
        assert_eq!(s.state, "waiting");
        assert_eq!(s.detail, "等接口定稿");
    }

    #[test]
    fn explicit_declaration_expires() {
        let mut r = rec();
        r.explicit = Some(("working".into(), String::new(), 1000));
        let s = derive_from(&r, 0, 1000 + EXPLICIT_TTL_SECS + 1);
        assert_eq!(s.state, "idle", "a crashed agent must not stay working on its own last words");
    }

    #[test]
    fn done_means_idle_and_supersedes_an_earlier_stop() {
        let mut r = rec();
        r.notif = Some(("completed".into(), 1000));
        r.done = Some(("PR 已提交".into(), 1005));
        let s = derive_from(&r, 0, 2000);
        assert_eq!(s.state, "idle");
        assert_eq!(s.detail, "PR 已提交");
    }

    #[test]
    fn a_stop_is_rest_not_distress_no_matter_how_old() {
        // Direct agents fire Stop after every exchange and never call
        // `tmm done`; a long-idle window must read idle, not stuck.
        let mut r = rec();
        r.notif = Some(("completed".into(), 1000));
        let hours_later = derive_from(&r, 1000, 1000 + 6 * 3600);
        assert_eq!(hours_later.state, "idle");
    }

    #[test]
    fn a_failed_stop_is_the_one_distress_signal() {
        let mut r = rec();
        r.notif = Some(("failed".into(), 1000));
        assert_eq!(derive_from(&r, 1000, 5000).state, "failed");
    }

    #[test]
    fn permission_prompt_is_waiting_regardless_of_age() {
        let mut r = rec();
        r.notif = Some(("permission_required".into(), 1000));
        let s = derive_from(&r, 1000, 1000 + 36000);
        assert_eq!(s.state, "waiting");
    }

    #[test]
    fn fresh_activity_after_a_notification_resolves_it() {
        // The prompt got answered in the terminal (or the agent moved on):
        // pane activity newer than the notification wins.
        let mut r = rec();
        r.notif = Some(("permission_required".into(), 1000));
        let s = derive_from(&r, 2000, 2010);
        assert_eq!(s.state, "working");
    }

    #[test]
    fn pane_activity_after_done_means_new_work() {
        let mut r = rec();
        r.done = Some((String::new(), 1000));
        let s = derive_from(&r, 1100, 1110);
        assert_eq!(s.state, "working");
    }

    #[test]
    fn no_facts_and_quiet_pane_is_idle() {
        let s = derive_from(&rec(), 100, 10_000);
        assert_eq!(s.state, "idle");
    }

    #[test]
    fn store_roundtrip_and_window_retention() {
        record_status("tsess", 1, "waiting", "note");
        record_status("tsess", 2, "working", "");
        retain_windows("tsess", &[2]);
        let s1 = derive("tsess", 1, 0);
        assert_eq!(s1.state, "idle", "dropped window's record must be gone");
        let s2 = derive("tsess", 2, 0);
        assert_eq!(s2.state, "working");
        retain_windows("tsess", &[]);
    }
}
