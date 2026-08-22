//! Auto-recovery from TRANSIENT model errors (owner, 2026-08-22: "自动帮我处理
//! 异常。如果检查到类似的错误，自动帮我发送 continue 指令").
//!
//! A backend occasionally aborts a turn with a load-shedding error — kiro
//! paints `An unexpected error occurred during the response stream …
//! ModelTemporarilyUnavailable … please try again` and then sits at its prompt.
//! The agent is fine, the turn is lost, and until a human types something the
//! window is dead weight. This module watches managed agent panes from the
//! capture tick and types `continue` at one that shows such an error.
//!
//! The owner's three rules ARE the design, in code not prose:
//! 1. Retries back off EXPONENTIALLY (`BASE_BACKOFF_SECS * 2^(n-1)`), because
//!    the error means "overloaded" and hammering an overloaded service is how
//!    you stay overloaded.
//! 2. After `MAX_ATTEMPTS` the window is left alone (one `warn` event says so)
//!    — an error that survives four spaced retries is not transient, and an
//!    unattended loop typing into a broken pane is worse than silence.
//! 3. Any TOOL CALL from that window resets the counter: a tool call is proof
//!    the model answered, so the incident is over and the budget refills.
//!
//! Detection is deliberately narrow. A pane is full of text ABOUT errors —
//! the owner pasted this very error into the chat, which typed it into an
//! agent's pane — so a hit requires the error's own header AND a transient
//! marker on the SAME logical line (capture joins soft-wrapped lines), and a
//! line carrying the `[tmm chat …]` stamp is never a hit (that is somebody
//! QUOTING an error, not having one). A missed real error costs one manual
//! `continue`; a false positive types into a working agent's conversation.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};

/// Retry budget per incident (rule 2).
pub const MAX_ATTEMPTS: u32 = 4;
/// First retry is immediate on detection; attempt n+1 waits
/// `BASE_BACKOFF_SECS * 2^(n-1)` after attempt n (rule 1): 30s, 60s, 120s.
pub const BASE_BACKOFF_SECS: u64 = 30;

/// What is typed into the pane. Plain — an invented stamp would read as a
/// message the user never wrote.
const CONTINUE_LINE: &str = "continue";

/// The error's own header, canonicalized. Everything kiro streams out of a
/// dead turn starts with this sentence.
const HEADER: &str = "anunexpectederroroccurred";
/// Transient markers — the reasons that mean "try again later", canonicalized.
const TRANSIENT: [&str; 3] = [
    "unexpectedlyhighload",
    "modeltemporarilyunavailable",
    "pleasetryagain",
];

/// Lowercase alphanumerics only: the pane wraps the error blob at arbitrary
/// points (and pads with box furniture), so whitespace and punctuation carry
/// no signal.
fn canonical(line: &str) -> String {
    line.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

/// Does this pane tail show a transient model error? Pure, per logical line.
pub fn scan_tail(text: &str) -> bool {
    text.lines().rev().take(40).any(|line| {
        let c = canonical(line);
        // A chat line QUOTING the error (delivered into the pane, or echoed
        // back in the conversation) is not the agent having one.
        if c.contains("tmmchat") {
            return false;
        }
        c.contains(HEADER) && TRANSIENT.iter().any(|m| c.contains(m))
    })
}

#[derive(Default)]
struct Rec {
    attempts: u32,
    /// Unix seconds before which no further attempt may run.
    next_at: u64,
    /// The give-up warning is emitted once per incident, not once per tick.
    gave_up_reported: bool,
}

/// What the tracker decided for one (window, tick) with the error visible.
#[derive(Debug, PartialEq)]
pub enum Decision {
    /// Type `continue` now; this is attempt `n` of `MAX_ATTEMPTS`.
    Send { attempt: u32 },
    /// An attempt is pending its backoff — do nothing this tick.
    Wait,
    /// The budget is spent. `true` exactly once, for the warning.
    GiveUp { first: bool },
}

/// Pure bookkeeping, `now` injected so the backoff ladder is testable.
#[derive(Default)]
pub struct Tracker {
    map: HashMap<(String, usize), Rec>,
}

impl Tracker {
    /// The error is visible in (session, window) at `now` — what do we do?
    pub fn decide(&mut self, session: &str, window: usize, now: u64) -> Decision {
        let rec = self.map.entry((session.to_string(), window)).or_default();
        if rec.attempts >= MAX_ATTEMPTS {
            let first = !rec.gave_up_reported;
            rec.gave_up_reported = true;
            return Decision::GiveUp { first };
        }
        if now < rec.next_at {
            return Decision::Wait;
        }
        rec.attempts += 1;
        // 30s after the 1st attempt, 60s after the 2nd, 120s after the 3rd.
        rec.next_at = now + BASE_BACKOFF_SECS * (1 << (rec.attempts - 1));
        Decision::Send { attempt: rec.attempts }
    }

    /// Rule 3: a tool call from the window is proof the model answered.
    pub fn reset(&mut self, session: &str, window: usize) {
        self.map.remove(&(session.to_string(), window));
    }

    /// Housekeeping: drop records for windows that no longer exist.
    pub fn retain_windows(&mut self, session: &str, live: &[usize]) {
        self.map.retain(|(s, w), _| s != session || live.contains(w));
    }
}

fn tracker() -> &'static Mutex<Tracker> {
    static T: OnceLock<Mutex<Tracker>> = OnceLock::new();
    T.get_or_init(|| Mutex::new(Tracker::default()))
}

/// Called by telemetry on every observed tool call (rule 3).
pub fn note_tool_activity(session: &str, window: usize) {
    tracker().lock().unwrap().reset(session, window);
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// One pass over every managed agent window of every live project, from the
/// capture tick. Reads the VISIBLE screen only (an error deep in scrollback is
/// an error somebody already moved past) and stays fail-soft throughout: this
/// is a convenience, and a convenience that can error a capture tick away is
/// too expensive.
pub fn check_once() {
    let projects = match super::with_store(|s| s.list_projects(false)) {
        Ok(list) => list,
        Err(_) => return,
    };
    for project in &projects {
        if !crate::tmux::session_exists(&project.session) {
            continue;
        }
        let Ok(panes) = crate::tmux::list_panes(&project.session) else { continue };
        let live: Vec<usize> = panes.iter().map(|p| p.window).collect();
        tracker().lock().unwrap().retain_windows(&project.session, &live);
        let mut seen = std::collections::HashSet::new();
        for p in &panes {
            if !seen.insert(p.window) || !p.active {
                continue;
            }
            let text = format!("{} {} {}", p.current_command, p.pane_title, p.window_name);
            if super::agents::detect_managed(Some(project.path.as_str()), &p.window_name, &text).is_none()
                || !super::is_managed_in(Some(project.path.as_str()), &p.window_name)
            {
                continue;
            }
            let target = format!("{}:{}.{}", project.session, p.window, p.pane);
            let Ok(tail) = crate::tmux::capture_pane_plain(&target, Some(0)) else { continue };
            if !scan_tail(&tail) {
                continue;
            }
            let decision = tracker().lock().unwrap().decide(&project.session, p.window, now_secs());
            match decision {
                Decision::Send { attempt } => {
                    if crate::tmux::send_command(&target, CONTINUE_LINE).is_ok() {
                        super::telemetry::record_recovery(
                            &project.session,
                            p.window,
                            &format!(
                                "auto-continue {attempt}/{MAX_ATTEMPTS} — transient model error in {}",
                                p.window_name
                            ),
                        );
                    }
                }
                Decision::GiveUp { first: true } => {
                    super::telemetry::record_recovery(
                        &project.session,
                        p.window,
                        &format!(
                            "auto-continue gave up after {MAX_ATTEMPTS} attempts — {} needs a person",
                            p.window_name
                        ),
                    );
                }
                Decision::Wait | Decision::GiveUp { first: false } => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The owner's real error, as kiro painted it (2026-08-21, wrapped by the
    /// pane; capture -J re-joins it, so the test keeps it on one line).
    const REAL: &str = "An unexpected error occurred during the response stream: CodewhispererChatResponseStream(ServiceError(ServiceError { source: InternalServerError(InternalServerError { message: \"Encountered unexpectedly high load when processing the request, please try again.\", reason: Some(ModelTemporarilyUnavailable), … }) })) (request_id: 30010907-33c4-483a-b024-2ed61321e233)";

    #[test]
    fn the_real_error_is_detected() {
        assert!(scan_tail(REAL));
        assert!(scan_tail(&format!("some output\n{REAL}\n\n╭───╮\n│ ❯ │\n╰───╯\n")));
    }

    #[test]
    fn talking_about_the_error_is_not_having_it() {
        // The owner pasted the error into the chat — that line was TYPED into
        // an agent's pane, stamped like every delivery.
        let quoted = format!("[tmm chat 2026-08-21 17:57] human: @builder {REAL} 自动帮我处理异常");
        assert!(!scan_tail(&quoted), "a chat quote must not trigger a retry");
        // Prose that names the reason without the error's own header.
        assert!(!scan_tail("we should detect ModelTemporarilyUnavailable and unexpectedly high load\n"));
        // The header alone (a non-transient stream error) is not retried.
        assert!(!scan_tail("An unexpected error occurred during the response stream: context overflow\n"));
        assert!(!scan_tail(""));
    }

    #[test]
    fn retries_back_off_exponentially_and_run_out() {
        let mut t = Tracker::default();
        let t0 = 1_000_000;
        // Attempt 1 is immediate.
        assert_eq!(t.decide("s", 1, t0), Decision::Send { attempt: 1 });
        // Still inside the 30s backoff: wait.
        assert_eq!(t.decide("s", 1, t0 + 29), Decision::Wait);
        assert_eq!(t.decide("s", 1, t0 + 30), Decision::Send { attempt: 2 });
        // The ladder doubles: 60s after attempt 2, 120s after attempt 3.
        assert_eq!(t.decide("s", 1, t0 + 89), Decision::Wait);
        assert_eq!(t.decide("s", 1, t0 + 90), Decision::Send { attempt: 3 });
        assert_eq!(t.decide("s", 1, t0 + 209), Decision::Wait);
        assert_eq!(t.decide("s", 1, t0 + 210), Decision::Send { attempt: 4 });
        // Rule 2: the budget is spent — warned ONCE, then silence.
        assert_eq!(t.decide("s", 1, t0 + 10_000), Decision::GiveUp { first: true });
        assert_eq!(t.decide("s", 1, t0 + 20_000), Decision::GiveUp { first: false });
    }

    #[test]
    fn a_tool_call_resets_the_budget() {
        let mut t = Tracker::default();
        for i in 0..MAX_ATTEMPTS as u64 {
            t.decide("s", 1, 1_000 + i * 10_000);
        }
        assert_eq!(t.decide("s", 1, 99_000), Decision::GiveUp { first: true });
        // Rule 3: the model answered something — the incident is over.
        t.reset("s", 1);
        assert_eq!(t.decide("s", 1, 100_000), Decision::Send { attempt: 1 });
        // Windows are independent.
        assert_eq!(t.decide("s", 2, 100_000), Decision::Send { attempt: 1 });
    }

    #[test]
    fn dead_windows_are_forgotten() {
        let mut t = Tracker::default();
        t.decide("s", 1, 1_000);
        t.decide("s", 7, 1_000);
        t.decide("other", 1, 1_000);
        t.retain_windows("s", &[7]);
        // Window 1 was dropped: a new agent at the same index starts fresh.
        assert_eq!(t.decide("s", 1, 1_001), Decision::Send { attempt: 1 });
        // Window 7 and the other session were untouched.
        assert_eq!(t.decide("s", 7, 1_001), Decision::Wait);
        assert_eq!(t.decide("other", 1, 1_001), Decision::Wait);
    }
}
