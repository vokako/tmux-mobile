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
    /// `tmm done [summary]`: (summary, ts). Also ENDS the turn.
    done: Option<(String, u64)>,
    /// Turn START: the `userPromptSubmit` hook. The agent accepted a prompt, so
    /// a turn is open from here until an end arrives.
    prompt: Option<u64>,
    /// Turn END: ("completed" | "failed", ts) from the stop / StopFailure hook.
    /// Separate from `ask` because they are different questions — "is a turn
    /// running" vs "is it blocked on me" — and one Option cannot answer both
    /// once a permission prompt overwrites a stop.
    end: Option<(String, u64)>,
    /// The agent is blocked on the human: permission_required | input_required.
    ask: Option<(String, u64)>,
    /// Last hook tool event: (activity line, ts). Work observed inside a turn.
    tool: Option<(String, u64)>,
    /// A line this app typed into the pane that has not been echoed back by a
    /// `userPromptSubmit` hook yet: (line, ts). Cleared by the echo (delivery
    /// confirmed) or by the sweep (delivery unconfirmed).
    pending: Option<(String, u64)>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentStatus {
    /// Derived state, one of exactly four: `running` (a turn is open — the
    /// agent accepted a prompt and has not stopped), `waiting` (blocked on the
    /// human), `idle` (no turn open) or `failed`.
    pub state: String,
    /// Human line explaining the state (explicit note, what it asked for, the
    /// last observed tool call, or a `tmm done` summary).
    pub detail: String,
    /// When the state began — the turn's start for `running`, so a client can
    /// render "running 2m14s" without keeping its own clock.
    pub since: u64,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Event timestamps are MILLISECONDS, and they have to be real ones. They were
/// `now() * 1000`, so every event inside the same second carried an identical
/// timestamp while chat messages carried true millis — the client's sort then
/// had nothing to order them by and a turn's tool calls could land after the
/// reply they produced.
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
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
    /// `tool` events only: the tool's NAME, kept apart from its argument so the
    /// client can render the scannable half differently. `text` is the argument.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub tool: String,
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
    push_full(session, window, kind, text, String::new(), String::new());
}

fn push_event_via(session: &str, window: usize, kind: &str, text: String, via: String) {
    push_full(session, window, kind, text, String::new(), via);
}

fn push_full(session: &str, window: usize, kind: &str, text: String, tool: String, via: String) {
    let mut map = events().lock().unwrap();
    let q = map.entry(session.to_string()).or_default();
    q.push_back(ActivityEvent { ts: now_ms(), window, kind: kind.into(), text, tool, via });
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

/// `tmm done [summary]` — completion declared by the agent. Ends the turn.
pub fn record_done(session: &str, window: usize, summary: &str) {
    let (summary, ts) = (summary.to_string(), now());
    with_rec(session, window, |r| {
        r.done = Some((summary, ts));
        r.explicit = None; // done supersedes any earlier declaration
    });
}

/// A hook notification consumed by the AgentNotificationHub. Two different
/// facts arrive here and they are stored apart: a stop ENDS the turn, a
/// permission/input prompt means the agent is blocked on the human while the
/// turn stays open.
pub fn record_notification(session: &str, window: usize, kind: &str, ts: u64) {
    push_event(session, window, "notif", kind.to_string());
    let kind = kind.to_string();
    with_rec(session, window, |r| match kind.as_str() {
        "permission_required" | "input_required" => r.ask = Some((kind, ts)),
        _ => {
            r.end = Some((kind, ts));
            r.ask = None; // a finished turn cannot still be asking
        }
    });
}

/// A hook tool event (isolated-home agents only, Phase B+): `("Edit",
/// "foo.rs")`. The event keeps the two parts apart for rendering; the status
/// record keeps the joined line, which is what "working — Edit foo.rs" shows.
pub fn record_tool(session: &str, window: usize, tool: &str, detail: &str) {
    push_full(session, window, "tool", detail.to_string(), tool.to_string(), String::new());
    let line = if detail.is_empty() { tool.to_string() } else { format!("{tool} {detail}") };
    let ts = now();
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
    let ts = now();
    with_rec(session, window, |r| {
        if let Some((line, _)) = r.pending.clone() {
            if prompt.contains(line.as_str()) || line.contains(prompt) {
                r.pending = None;
                acked = true;
            }
        }
        // A turn just opened. This is the ONE honest "it started working"
        // signal: pane activity cannot be it, because an agent TUI repaints its
        // prompt (spinner, status line, cursor) long after it finished.
        r.prompt = Some(ts);
        r.explicit = None; // a new turn supersedes the last turn's words
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
/// window_activity for the window — used ONLY when the window has produced no
/// hook facts at all. Pure given the record + clock.
pub fn derive(session: &str, window: usize, activity_ts: u64) -> AgentStatus {
    let rec = store()
        .lock()
        .unwrap()
        .get(&(session.to_string(), window))
        .cloned()
        .unwrap_or_default();
    derive_from(&rec, activity_ts, now())
}

/// The state machine, in one place. A turn is a bracket: `userPromptSubmit`
/// opens it, `stop` / `tmm done` closes it, tool calls happen inside it, and a
/// permission prompt suspends it. So the rule is simply *which boundary is the
/// most recent fact*, and the four states fall out of that:
///
/// | newest fact                    | state   | since        |
/// |--------------------------------|---------|--------------|
/// | a failed stop                  | failed  | the stop     |
/// | an explicit `tmm status`       | that    | the claim    |
/// | a turn end (stop / done)       | idle    | the end      |
/// | an ask (permission / input)    | waiting | the ask      |
/// | a turn start (prompt / tool)   | running | the START    |
///
/// `since` for `running` is the turn's start, not the newest event, so "running
/// 2m14s" means the turn has been open that long.
///
/// Pane activity is NOT a work signal for a window with hooks. It used to be,
/// and that was the bug: an agent TUI repaints after replying (spinner, status
/// line, blinking cursor), so `window_activity` was always newer than the stop
/// and every finished agent read as "working" forever. Windows with no hook
/// coverage at all (codex today, anything hand-started) still fall back to it,
/// because for them the alternative is no signal at all.
fn derive_from(rec: &Rec, activity_ts: u64, now: u64) -> AgentStatus {
    let (end_kind, end_ts) = rec.end.clone().unwrap_or_default();
    let done_ts = rec.done.as_ref().map(|(_, t)| *t).unwrap_or(0);
    let ask_ts = rec.ask.as_ref().map(|(_, t)| *t).unwrap_or(0);
    let tool_ts = rec.tool.as_ref().map(|(_, t)| *t).unwrap_or(0);
    let prompt_ts = rec.prompt.unwrap_or(0);

    let turn_start = prompt_ts.max(tool_ts);
    let turn_end = end_ts.max(done_ts);
    let newest = turn_start.max(turn_end).max(ask_ts);

    // No hook has ever spoken for this window: fall back to pane activity,
    // which is all a hookless backend gives us.
    if newest == 0 && rec.explicit.is_none() {
        let state = if now.saturating_sub(activity_ts) < ACTIVE_SECS { "running" } else { "idle" };
        return AgentStatus { state: state.into(), detail: String::new(), since: activity_ts };
    }

    // A failed stop is the one distress signal we can observe. It stands until
    // a new turn starts.
    if end_kind == "failed" && end_ts >= turn_start && end_ts >= ask_ts {
        return AgentStatus { state: "failed".into(), detail: "failed".into(), since: end_ts };
    }

    // The agent's own words. What they are good for is the part we CANNOT
    // observe: "blocked on a credential", "waiting for the API spec". A claim of
    // `working` adds nothing — the turn bracket already knows a turn is open —
    // so it contributes its note and nothing else. That keeps one class of lie
    // out of the system: an agent cannot declare itself busy while its stop hook
    // says the turn is over.
    if let Some((state, note, ts)) = &rec.explicit {
        let claims_block = matches!(state.as_str(), "waiting" | "blocked");
        if claims_block && now.saturating_sub(*ts) < EXPLICIT_TTL_SECS && *ts >= newest {
            return AgentStatus { state: "waiting".into(), detail: note.clone(), since: *ts };
        }
    }

    // A turn that ended is rest, not distress: direct agents fire stop after
    // every exchange and never call `tmm done`.
    if turn_end >= turn_start && turn_end >= ask_ts {
        let summary = rec
            .done
            .as_ref()
            .filter(|(_, t)| *t == turn_end)
            .map(|(s, _)| s.clone())
            .unwrap_or_default();
        return AgentStatus { state: "idle".into(), detail: summary, since: turn_end };
    }

    // Blocked on the human, and nothing has happened since.
    if ask_ts > tool_ts && ask_ts >= prompt_ts {
        let kind = rec.ask.as_ref().map(|(k, _)| k.clone()).unwrap_or_default();
        return AgentStatus { state: "waiting".into(), detail: kind, since: ask_ts };
    }

    // A turn is open. `since` is when it opened; the detail is the agent's own
    // note if it left one this turn, else the last thing we saw it do.
    let note = rec
        .explicit
        .as_ref()
        .filter(|(_, note, ts)| !note.is_empty() && *ts >= prompt_ts)
        .map(|(_, note, _)| note.clone());
    let detail = note.unwrap_or_else(|| {
        rec.tool
            .as_ref()
            .filter(|(_, t)| *t >= prompt_ts)
            .map(|(l, _)| l.clone())
            .unwrap_or_default()
    });
    let since = if prompt_ts > 0 { prompt_ts } else { tool_ts };
    AgentStatus { state: "running".into(), detail, since }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> Rec {
        Rec::default()
    }

    /// The bug this machine replaced: an agent that had just answered kept
    /// reading "working" forever, because its TUI repaints the prompt after
    /// every reply and pane activity was treated as work.
    #[test]
    fn a_finished_turn_is_idle_no_matter_how_busy_the_pane_looks() {
        let mut r = rec();
        r.prompt = Some(1000);
        r.tool = Some(("Edit src/lib.rs".into(), 1010));
        r.end = Some(("completed".into(), 1020));
        // Pane activity 500s of it, all newer than the stop — a repainting TUI.
        let s = derive_from(&r, 1500, 1520);
        assert_eq!(s.state, "idle", "a repainting prompt is not work");
        assert_eq!(s.since, 1020, "idle since the turn ended");
    }

    #[test]
    fn a_turn_is_running_from_its_prompt_until_it_stops() {
        let mut r = rec();
        r.prompt = Some(1000);
        let s = derive_from(&r, 0, 1300);
        assert_eq!(s.state, "running");
        assert_eq!(s.since, 1000, "since = the turn's start, so elapsed is the TURN's age");
        // A long think with no tool calls stays running: only an end ends it.
        assert_eq!(derive_from(&r, 0, 1000 + 3600).state, "running");
        // Tools inside the turn keep it running and describe it.
        r.tool = Some(("Edit a.rs".into(), 1200));
        let s = derive_from(&r, 0, 1300);
        assert_eq!((s.state.as_str(), s.detail.as_str(), s.since), ("running", "Edit a.rs", 1000));
    }

    #[test]
    fn tools_without_a_prompt_still_mean_running() {
        // Claude/codex may deliver tool events without a turn-start hook.
        let mut r = rec();
        r.tool = Some(("Bash npm test".into(), 1000));
        let s = derive_from(&r, 0, 1005);
        assert_eq!((s.state.as_str(), s.since), ("running", 1000));
    }

    #[test]
    fn an_ask_suspends_the_turn_and_an_answer_resumes_it() {
        let mut r = rec();
        r.prompt = Some(1000);
        r.ask = Some(("permission_required".into(), 1100));
        let s = derive_from(&r, 0, 9000);
        assert_eq!(s.state, "waiting", "blocked on the human, however long it waits");
        assert_eq!(s.detail, "permission_required");
        // A tool call after the ask means it got answered.
        r.tool = Some(("Bash rm -rf build".into(), 1200));
        assert_eq!(derive_from(&r, 0, 1250).state, "running");
        // And a stop after the ask ends the turn, ask or no ask.
        r.tool = None;
        r.end = Some(("completed".into(), 1300));
        assert_eq!(derive_from(&r, 0, 1350).state, "idle");
    }

    #[test]
    fn done_is_idle_and_carries_its_summary() {
        let mut r = rec();
        r.prompt = Some(1000);
        r.done = Some(("PR 已提交".into(), 1100));
        let s = derive_from(&r, 0, 2000);
        assert_eq!((s.state.as_str(), s.detail.as_str(), s.since), ("idle", "PR 已提交", 1100));
        // A NEW turn after done is running again, and does not keep the summary.
        r.prompt = Some(1200);
        let s = derive_from(&r, 0, 2000);
        assert_eq!((s.state.as_str(), s.detail.as_str(), s.since), ("running", "", 1200));
    }

    #[test]
    fn a_failed_stop_is_the_one_distress_signal() {
        let mut r = rec();
        r.prompt = Some(900);
        r.end = Some(("failed".into(), 1000));
        assert_eq!(derive_from(&r, 5000, 5000).state, "failed", "and pane noise cannot clear it");
        // Only a new turn clears it.
        r.prompt = Some(1100);
        assert_eq!(derive_from(&r, 0, 1200).state, "running");
    }

    /// What an agent says about itself is only trusted where we cannot observe:
    /// a block. A claim of `working` contributes its NOTE and no state, because
    /// the turn bracket already answers "is it running" and a self-declared
    /// state could contradict the hooks.
    #[test]
    fn an_explicit_claim_speaks_only_for_what_we_cannot_observe() {
        let mut r = rec();
        r.prompt = Some(1000);
        // waiting / blocked: unobservable, so the claim stands (and `blocked`
        // reads as waiting — the CLI's vocabulary is wider than the UI's).
        r.explicit = Some(("waiting".into(), "等接口定稿".into(), 1100));
        let s = derive_from(&r, 0, 1200);
        assert_eq!((s.state.as_str(), s.detail.as_str(), s.since), ("waiting", "等接口定稿", 1100));
        r.explicit = Some(("blocked".into(), "no creds".into(), 1100));
        assert_eq!(derive_from(&r, 0, 1200).state, "waiting");
        // A claim of working does NOT set the state; the open turn does, and the
        // note becomes the line the user reads.
        r.explicit = Some(("working".into(), "重写状态机".into(), 1100));
        let s = derive_from(&r, 0, 1200);
        assert_eq!((s.state.as_str(), s.detail.as_str(), s.since), ("running", "重写状态机", 1000),
            "state from the bracket, words from the agent, since = the turn start");
        // And it cannot outlive the turn: a stop is a newer fact.
        r.end = Some(("completed".into(), 1150));
        assert_eq!(derive_from(&r, 0, 1200).state, "idle");
        // A stale block expires, so a crashed agent does not wait forever.
        r.end = None;
        r.explicit = Some(("blocked".into(), "no creds".into(), 1100));
        assert_eq!(derive_from(&r, 0, 1100 + EXPLICIT_TTL_SECS + 1).state, "running",
            "the open turn still explains it");
        r.prompt = None;
        assert_eq!(derive_from(&r, 0, 1100 + EXPLICIT_TTL_SECS + 1).state, "idle");
    }

    #[test]
    fn a_window_with_no_hooks_at_all_falls_back_to_pane_activity() {
        let s = derive_from(&rec(), 1000, 1005);
        assert_eq!(s.state, "running", "a hookless backend has nothing else");
        assert_eq!(derive_from(&rec(), 1000, 1000 + ACTIVE_SECS + 1).state, "idle");
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
        record_status("tsess", 2, "waiting", "");
        retain_windows("tsess", &[2]);
        let s1 = derive("tsess", 1, 0);
        assert_eq!(s1.state, "idle", "dropped window's record must be gone");
        let s2 = derive("tsess", 2, 0);
        assert_eq!(s2.state, "waiting", "the surviving record still answers");
        retain_windows("tsess", &[]);
    }
}
