//! Agent telemetry: the passive half of the v2 dual channel.
//!
//! What an agent SAYS goes through the `tmm` CLI (hub_post / hub_status /
//! hub_done). What we OBSERVE arrives here: hook notifications (it stopped, it
//! wants permission), the prompts it accepted, and tmux window activity. Status
//! is DERIVED from those facts at read time — an agent never fills in a form,
//! and a backend with poor hook coverage (codex) degrades to pane-activity
//! granularity instead of lying. A finished turn (Stop) is REST, not distress:
//! "stuck" detection by stop-without-done was a Team-supervisor-era rule and
//! mislabeled every long-idle direct agent — the only distress we can honestly
//! observe is a failed stop. See docs/exec-plans/agents-v2.md §4.1/§4.3.
//!
//! `userPromptSubmit` carries the INPUT half, which nothing else does: a prompt
//! typed at the keyboard exists nowhere in the room, and a line we typed into a
//! pane is only *sent*, never *confirmed*. Recording both closes the loop —
//! `record_delivery` remembers what we typed, the echo acknowledges it, and
//! `sweep_deliveries` reports the ones that never arrived.
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
/// How long a line we typed into a pane may wait for its `userPromptSubmit`
/// echo before we call the delivery unconfirmed. Typing is `send-keys`, which
/// succeeds as long as the pane exists — it says nothing about whether the CLI
/// accepted the text as a prompt (a busy agent queues it, a shell would have
/// executed it, a crashed pane swallows it). The hook echo is the only
/// end-to-end proof, so an unacked line is reported rather than assumed.
const DELIVERY_ACK_SECS: u64 = 45;
/// Prompt text kept per event. Long enough for a real instruction, short
/// enough that 120 of them stay a cheap in-memory ring.
const MAX_PROMPT_CHARS: usize = 1024;

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
    /// A line this app typed into the pane that has not been echoed back by a
    /// `userPromptSubmit` hook yet: (line, ts). Cleared by the echo (delivery
    /// confirmed) or by the sweep (delivery unconfirmed).
    pending: Option<(String, u64)>,
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

/// The activity FEED: recent observed events per session, newest last. This
/// is telemetry made visible in the chat timeline (owner ask: show tool
/// calls / status changes between the final replies) — an in-memory ring,
/// NOT chat history: it never touches the bus db and dies with the server.
const EVENTS_CAP: usize = 120;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ActivityEvent {
    /// Epoch MILLISECONDS to merge directly with bus message timestamps.
    pub ts: u64,
    pub window: usize,
    /// tool | status | notif | prompt | warn
    pub kind: String,
    pub text: String,
    /// Provenance, `prompt` events only: `app` when the text is the line this
    /// app typed into the pane (so the event doubles as the delivery receipt),
    /// `local` when it was typed at the keyboard. Empty for every other kind.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub via: String,
}

fn events() -> &'static Mutex<HashMap<String, std::collections::VecDeque<ActivityEvent>>> {
    static EVENTS: OnceLock<Mutex<HashMap<String, std::collections::VecDeque<ActivityEvent>>>> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn push_event(session: &str, window: usize, kind: &str, text: String) {
    push_event_via(session, window, kind, text, String::new());
}

fn push_event_via(session: &str, window: usize, kind: &str, text: String, via: String) {
    let mut map = events().lock().unwrap();
    let q = map.entry(session.to_string()).or_default();
    q.push_back(ActivityEvent { ts: now() * 1000, window, kind: kind.into(), text, via });
    while q.len() > EVENTS_CAP {
        q.pop_front();
    }
}

/// Events newer than `since_ts` (ms, exclusive), oldest first.
pub fn recent_events(session: &str, since_ts: u64) -> Vec<ActivityEvent> {
    events()
        .lock()
        .unwrap()
        .get(session)
        .map(|q| q.iter().filter(|e| e.ts > since_ts).cloned().collect())
        .unwrap_or_default()
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
    let text = if note.is_empty() { state.to_string() } else { format!("{state} — {note}") };
    push_event(session, window, "status", text);
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
    push_event(session, window, "notif", kind.to_string());
    let kind = kind.to_string();
    with_rec(session, window, |r| r.notif = Some((kind, ts)));
}

/// A hook tool event (isolated-home agents only, Phase B+): "Edit foo.rs".
pub fn record_tool(session: &str, window: usize, line: &str) {
    push_event(session, window, "tool", line.to_string());
    let (line, ts) = (line.to_string(), now());
    with_rec(session, window, |r| r.tool = Some((line, ts)));
}

/// A line this app typed into an agent's pane (`deliver_mentions`). Held as a
/// pending delivery until the agent's `userPromptSubmit` hook echoes it back.
pub fn record_delivery(session: &str, window: usize, line: &str) {
    let (line, ts) = (line.to_string(), now());
    with_rec(session, window, |r| r.pending = Some((line, ts)));
}

/// The `userPromptSubmit` hook: the agent accepted a prompt. This is BOTH the
/// input half of the transcript (what the agent was asked, which no other
/// channel carries) and the delivery receipt for a line we typed.
///
/// Returns true when it acknowledged a pending delivery. Matching is
/// containment, not equality: the CLI may submit the line with its own
/// decoration, and an agent that is mid-task receives our line appended to
/// whatever it was already typing.
pub fn record_prompt(session: &str, window: usize, prompt: &str) -> bool {
    let text = truncate_chars(prompt, MAX_PROMPT_CHARS);
    let mut acked = false;
    with_rec(session, window, |r| {
        if let Some((line, _)) = r.pending.clone() {
            if prompt.contains(line.as_str()) || line.contains(prompt) {
                r.pending = None;
                acked = true;
            }
        }
    });
    push_event_via(
        session,
        window,
        "prompt",
        text,
        if acked { "app".into() } else { "local".into() },
    );
    acked
}

/// Report deliveries that never came back as a prompt. Called before a client
/// reads the feed, which is exactly when the answer is wanted; a swept line is
/// cleared so the warning is emitted once.
pub fn sweep_deliveries(session: &str) {
    let now = now();
    let stale: Vec<(usize, String)> = {
        let mut map = store().lock().unwrap();
        map.iter_mut()
            .filter(|((s, _), _)| s == session)
            .filter_map(|((_, w), r)| {
                let (line, ts) = r.pending.clone()?;
                if now.saturating_sub(ts) < DELIVERY_ACK_SECS {
                    return None;
                }
                r.pending = None;
                Some((*w, line))
            })
            .collect()
    };
    for (window, line) in stale {
        push_event(session, window, "warn", format!("unconfirmed: {}", truncate_chars(&line, 160)));
    }
}

fn truncate_chars(input: &str, max: usize) -> String {
    let mut out: String = input.chars().take(max).collect();
    if input.chars().count() > max {
        out.push('…');
    }
    out
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
    fn activity_ring_orders_caps_and_filters_by_since() {
        for i in 0..130u64 {
            push_event("ring-test", 1, "tool", format!("evt{i}"));
        }
        let all = recent_events("ring-test", 0);
        assert_eq!(all.len(), EVENTS_CAP, "capped");
        assert_eq!(all.first().unwrap().text, "evt10", "oldest dropped first");
        assert_eq!(all.last().unwrap().text, "evt129");
        // since filter is exclusive on ms timestamps
        let ts = all.last().unwrap().ts;
        assert!(recent_events("ring-test", ts).is_empty());
        assert!(recent_events("other-session", 0).is_empty(), "sessions are isolated");
    }

    #[test]
    fn recorders_feed_the_activity_ring() {
        record_status("feed-test", 2, "waiting", "等接口");
        record_tool("feed-test", 2, "Edit src/lib.rs");
        record_notification("feed-test", 2, "completed", 123);
        let kinds: Vec<String> = recent_events("feed-test", 0).iter().map(|e| e.kind.clone()).collect();
        assert_eq!(kinds, vec!["status", "tool", "notif"]);
        let texts: Vec<String> = recent_events("feed-test", 0).iter().map(|e| e.text.clone()).collect();
        assert_eq!(texts[0], "waiting — 等接口");
    }

    #[test]
    fn a_typed_line_is_acknowledged_by_the_prompt_hook_that_echoes_it() {
        let line = "[tmm chat] human: @dev ship it";
        record_delivery("ack-test", 3, line);
        // The CLI submits our line (possibly with the agent's own leading text
        // when it was mid-typing) — containment, not equality.
        assert!(
            record_prompt("ack-test", 3, &format!("{line}\n")),
            "the echo must acknowledge the pending delivery"
        );
        let evs = recent_events("ack-test", 0);
        assert_eq!(evs.last().unwrap().kind, "prompt");
        assert_eq!(evs.last().unwrap().via, "app", "an acked prompt came from this app");
        // Nothing pending any more, so the sweep stays silent.
        sweep_deliveries("ack-test");
        assert_eq!(recent_events("ack-test", 0).len(), 1, "no warning for a delivered line");
    }

    #[test]
    fn a_prompt_typed_at_the_keyboard_is_recorded_as_local_input() {
        assert!(!record_prompt("local-test", 1, "fix the flaky test"), "nothing was pending");
        let e = recent_events("local-test", 1).into_iter().next().unwrap();
        assert_eq!((e.kind.as_str(), e.via.as_str()), ("prompt", "local"));
        assert_eq!(e.text, "fix the flaky test", "the input half of the transcript");
    }

    #[test]
    fn an_unacknowledged_delivery_is_reported_once() {
        // Backdate the pending line past the ack window.
        record_delivery("sweep-test", 2, "[tmm chat] human: @dev hello");
        with_rec("sweep-test", 2, |r| {
            let (line, ts) = r.pending.clone().unwrap();
            r.pending = Some((line, ts - DELIVERY_ACK_SECS - 1));
        });
        sweep_deliveries("sweep-test");
        let warns: Vec<String> = recent_events("sweep-test", 0)
            .iter()
            .filter(|e| e.kind == "warn")
            .map(|e| e.text.clone())
            .collect();
        assert_eq!(warns.len(), 1, "one report per unacked line");
        assert!(warns[0].contains("hello"), "the report names the line, got {:?}", warns[0]);
        sweep_deliveries("sweep-test");
        assert_eq!(
            recent_events("sweep-test", 0).iter().filter(|e| e.kind == "warn").count(),
            1,
            "sweeping again must not re-report"
        );
    }

    #[test]
    fn prompt_text_is_capped() {
        let long = "x".repeat(MAX_PROMPT_CHARS + 50);
        record_prompt("cap-test", 1, &long);
        let e = recent_events("cap-test", 0).into_iter().next().unwrap();
        assert_eq!(e.text.chars().count(), MAX_PROMPT_CHARS + 1, "capped plus the ellipsis");
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
