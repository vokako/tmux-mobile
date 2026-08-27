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
//! The owner's rules ARE the design, in code not prose (2026-08-22 original,
//! tightened 2026-08-26: "针对同一个 error，只发送一次 … 发送后等一会儿，通过
//! hooks 查看之前发送的指令是否正确生效"):
//! 1. An INCIDENT is one error INSTANCE, continuously visible. Within it, ONE
//!    `continue` is the intent; retries exist only for a send the hooks never
//!    saw land. The record lives exactly as long as ITS error stays on the
//!    screen — the tick that no longer sees any error drops the record, and a
//!    DIFFERENT error appearing (a new `request_id` — the resumed turn died
//!    again, owner 2026-08-26: "有可能还会出现二次报错并停止…就需要再次发送
//!    指令") replaces the record with a fresh incident and a full budget,
//!    because "an error is visible" cannot distinguish the second failure
//!    from the first one's still-painted text. The signature is the set of
//!    request_ids read off the visible error lines (hit count as fallback
//!    when an id is not on screen).
//! 2. A send is VERIFIED through the hooks, not the screen: the error text
//!    stays painted long after the agent moved on, so "error still visible"
//!    means nothing. Before any retry the tracker asks whether a turn fact
//!    (accepted prompt, turn end, tool call — `telemetry::turn_fact_since`)
//!    arrived after the send; if one did the incident is CONFIRMED and goes
//!    silent. A tool call observed by telemetry confirms it directly too —
//!    and must never erase the record, because an erased record plus the
//!    still-painted error re-opened the incident every tick and typed
//!    `continue` into a working agent (the owner's repeat-send report).
//! 3. Unverified retries back off EXPONENTIALLY (`BASE_BACKOFF_SECS * 2^(n-1)`)
//!    and run out after `MAX_ATTEMPTS`, with one `warn` event — an error that
//!    survives four spaced, never-landing sends is not transient, and an
//!    unattended loop typing into a broken pane is worse than silence.
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
/// Second kiro shape (owner, 2026-08-26): `The model you've selected is
/// temporarily unavailable. Please use '/model' to select a different model
/// and try again. (request_id: …)`. No "unexpected error" header — the whole
/// first sentence IS the header, and it is specific enough on its own: it
/// self-contains the transient reason ("temporarily unavailable") and kiro
/// hard-wraps its continuation lines, so requiring a second marker on the
/// same logical line would miss the real error on any normal-width pane.
const MODEL_UNAVAILABLE: &str = "themodelyouveselectedistemporarilyunavailable";

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
    error_signature(text).is_some()
}

/// The IDENTITY of what is on the screen: `None` when no transient error is
/// visible, else `"<hits>|<sorted request_ids>"`. Two ticks that see the SAME
/// painted error produce the same signature; a SECOND error (the resumed turn
/// died again) changes it — new hit, new request_id — which is what re-arms a
/// confirmed incident. Ids are read from the hit line and its next few lines,
/// because kiro hard-wraps and the `(request_id: …)` tail often lands on its
/// own continuation line.
pub fn error_signature(text: &str) -> Option<String> {
    let lines: Vec<&str> = {
        let mut v: Vec<&str> = text.lines().rev().take(40).collect();
        v.reverse();
        v
    };
    let mut hits = 0u32;
    let mut ids: Vec<String> = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let c = canonical(line);
        // A chat line QUOTING the error (delivered into the pane, or echoed
        // back in the conversation) is not the agent having one.
        if c.contains("tmmchat") {
            continue;
        }
        let hit = (c.contains(HEADER) && TRANSIENT.iter().any(|m| c.contains(m)))
            || c.contains(MODEL_UNAVAILABLE);
        if !hit {
            continue;
        }
        hits += 1;
        for l in lines[i..lines.len().min(i + 4)].iter() {
            if canonical(l).contains("tmmchat") {
                continue; // a quote's id must not pollute the signature
            }
            if let Some(id) = extract_request_id(l) {
                ids.push(id);
                break;
            }
        }
    }
    if hits == 0 {
        return None;
    }
    ids.sort();
    ids.dedup();
    Some(format!("{}|{}", hits, ids.join(",")))
}

/// `… (request_id: 30010907-33c4-…)` → the id token.
fn extract_request_id(line: &str) -> Option<String> {
    let idx = line.find("request_id")?;
    let rest = &line[idx + "request_id".len()..];
    let id: String = rest
        .chars()
        .skip_while(|c| !c.is_ascii_alphanumeric())
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
        .collect();
    (!id.is_empty()).then_some(id)
}

#[derive(Default)]
struct Rec {
    /// Which error instance this incident is about (`error_signature`). A
    /// tick whose visible signature differs replaces the whole record: the
    /// SECOND error is a new incident with a full budget.
    sig: String,
    attempts: u32,
    /// Unix seconds before which no further attempt may run.
    next_at: u64,
    /// When the last `continue` was typed — the hook check measures from here.
    sent_at: u64,
    /// Hooks showed a turn fact after `sent_at`: the incident is OVER, even
    /// though the error text is still painted on the screen. Nothing more is
    /// sent until the error leaves the screen and a fresh one appears.
    confirmed: bool,
    /// The give-up warning is emitted once per incident, not once per tick.
    gave_up_reported: bool,
}

/// What the tracker decided for one (window, tick) with the error visible.
#[derive(Debug, PartialEq)]
pub enum Decision {
    /// Type `continue` now; this is attempt `n` of `MAX_ATTEMPTS`.
    Send { attempt: u32 },
    /// A sent attempt has not been verified yet and its backoff has not
    /// elapsed — do nothing this tick.
    Wait,
    /// The hooks JUST verified the last send (a turn fact arrived after it).
    /// `true` exactly once, for the log line.
    Confirmed { first: bool },
    /// The budget is spent with nothing verified. `true` exactly once.
    GiveUp { first: bool },
}

/// Pure bookkeeping, `now` and the hook check injected so every path is
/// testable without tmux or real hooks.
#[derive(Default)]
pub struct Tracker {
    map: HashMap<(String, usize), Rec>,
}

impl Tracker {
    /// The error with signature `sig` is visible in (session, window) at
    /// `now` — what do we do? `verified(sent_at)` answers "did any turn fact
    /// arrive at/after that time" (see `telemetry::turn_fact_since`); it is
    /// consulted before any retry, so a `continue` that WORKED is never
    /// followed by another one just because the error text is still painted
    /// (owner, 2026-08-26). A DIFFERENT signature than the record's replaces
    /// it wholesale — confirmed, mid-backoff or given-up alike: whatever the
    /// old incident's state, a new error instance is a new incident (owner,
    /// 2026-08-26: "有可能还会出现二次报错并停止…就需要再次发送指令").
    pub fn decide(
        &mut self,
        session: &str,
        window: usize,
        now: u64,
        sig: &str,
        verified: impl FnOnce(u64) -> bool,
    ) -> Decision {
        let key = (session.to_string(), window);
        if self.map.get(&key).is_some_and(|r| r.sig != sig) {
            self.map.remove(&key);
        }
        let rec = self.map.entry(key).or_default();
        rec.sig = sig.to_string();
        if rec.confirmed {
            return Decision::Confirmed { first: false };
        }
        // A send exists: ask the hooks FIRST. Verification outranks both the
        // backoff ladder and the give-up — a 4th attempt that finally landed
        // is a success, not an exhausted budget.
        if rec.attempts > 0 && verified(rec.sent_at) {
            rec.confirmed = true;
            return Decision::Confirmed { first: true };
        }
        if rec.attempts >= MAX_ATTEMPTS {
            let first = !rec.gave_up_reported;
            rec.gave_up_reported = true;
            return Decision::GiveUp { first };
        }
        if now < rec.next_at {
            return Decision::Wait;
        }
        rec.attempts += 1;
        rec.sent_at = now;
        // 30s after the 1st attempt, 60s after the 2nd, 120s after the 3rd.
        rec.next_at = now + BASE_BACKOFF_SECS * (1 << (rec.attempts - 1));
        Decision::Send { attempt: rec.attempts }
    }

    /// A tool call from the window CONFIRMS an open incident — the model
    /// answered, so the `continue` (or something else) worked. It must never
    /// erase the record: the error text is still painted, and a wiped record
    /// made the next tick open a brand-new incident and type `continue` into
    /// the now-working agent — the repeat-send the owner reported
    /// (2026-08-26: "系统会反复发送好几次 auto recovery").
    pub fn confirm(&mut self, session: &str, window: usize) {
        if let Some(rec) = self.map.get_mut(&(session.to_string(), window)) {
            rec.confirmed = true;
        }
    }

    /// The error is no longer on the window's screen: the incident is over,
    /// whatever its state was. The record is dropped so a FRESH error later
    /// starts a fresh incident with a full budget.
    pub fn clear(&mut self, session: &str, window: usize) {
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

/// Called by telemetry on every observed tool call: work is proof the model
/// answered, so an open incident is CONFIRMED (never erased — see
/// `Tracker::confirm` for the repeat-send that erasing caused).
pub fn note_tool_activity(session: &str, window: usize) {
    tracker().lock().unwrap().confirm(session, window);
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
            let Some(sig) = error_signature(&tail) else {
                // The error left the screen: the incident (if any) is over.
                // Dropping the record here is what scopes "one incident" to
                // one CONTINUOUS sighting — a fresh error later starts fresh.
                tracker().lock().unwrap().clear(&project.session, p.window);
                continue;
            };
            let decision = tracker().lock().unwrap().decide(
                &project.session,
                p.window,
                now_secs(),
                &sig,
                |sent_at| super::telemetry::turn_fact_since(&project.session, p.window, sent_at),
            );
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
                Decision::Confirmed { first: true } => {
                    super::telemetry::record_recovery(
                        &project.session,
                        p.window,
                        &format!("auto-continue took effect — {} resumed its turn", p.window_name),
                    );
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
                Decision::Wait
                | Decision::Confirmed { first: false }
                | Decision::GiveUp { first: false } => {}
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

    /// The second shape, exactly as kiro painted it (2026-08-26): its own
    /// hard-wrapped lines, first sentence whole on the first line.
    const MODEL_ERR: &str = "The model you've selected is temporarily unavailable.\n  Please use '/model' to select a different model and try\n  again. (request_id: db05d310-a189-497d-a6f2-b53410863243)";

    #[test]
    fn the_model_unavailable_error_is_detected() {
        assert!(scan_tail(MODEL_ERR));
        assert!(scan_tail(&format!("some output\n{MODEL_ERR}\n\n╭───╮\n│ ❯ │\n╰───╯\n")));
        // capture -J re-joins a soft-wrapped paint into one logical line.
        assert!(scan_tail(&MODEL_ERR.replace('\n', " ")));
    }

    #[test]
    fn quoting_the_model_error_is_not_having_it() {
        // The owner pasted this error into the chat (2026-08-26) — the
        // delivery lands in an agent's pane stamped, wrapping after it.
        let quoted = format!("[tmm chat 2026-08-26 15:42] human: @builder-2 {MODEL_ERR} 遇到这个异常，麻烦你也给我处理一下");
        assert!(!scan_tail(&quoted), "a chat quote must not trigger a retry");
        // The continuation lines alone (stamp scrolled onto the line above)
        // never carry the full header sentence, so they cannot hit either.
        assert!(!scan_tail("Please use '/model' to select a different model and try\n  again. (request_id: x)\n"));
        // Prose that names the reason without the sentence.
        assert!(!scan_tail("the model may be temporarily unavailable, retry later\n"));
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

    /// A hook check that never sees anything land.
    fn silent(_: u64) -> bool {
        false
    }

    #[test]
    fn a_verified_send_is_never_repeated() {
        let mut t = Tracker::default();
        let t0 = 1_000_000;
        assert_eq!(t.decide("s", 1, t0, "1|a", silent), Decision::Send { attempt: 1 });
        // Next tick, error still painted, but a turn fact arrived after the
        // send: confirmed ONCE, then silence for as long as the error shows.
        assert_eq!(
            t.decide("s", 1, t0 + 20, "1|a", |sent| sent == t0),
            Decision::Confirmed { first: true }
        );
        assert_eq!(
            t.decide("s", 1, t0 + 40, "1|a", |_| true),
            Decision::Confirmed { first: false }
        );
        // Even hours later, the painted error must not re-trigger a send.
        assert_eq!(
            t.decide("s", 1, t0 + 10_000, "1|a", |_| true),
            Decision::Confirmed { first: false }
        );
    }

    #[test]
    fn verification_outranks_backoff_and_the_spent_budget() {
        let mut t = Tracker::default();
        let t0 = 1_000_000;
        for i in 0..MAX_ATTEMPTS as u64 {
            t.decide("s", 1, t0 + i * 10_000, "1|a", silent);
        }
        // The budget is spent — but the 4th send finally landed: that is a
        // success, not a give-up.
        assert_eq!(
            t.decide("s", 1, t0 + 40_000, "1|a", |_| true),
            Decision::Confirmed { first: true }
        );
    }

    #[test]
    fn unverified_retries_back_off_exponentially_and_run_out() {
        let mut t = Tracker::default();
        let t0 = 1_000_000;
        // Attempt 1 is immediate.
        assert_eq!(t.decide("s", 1, t0, "1|a", silent), Decision::Send { attempt: 1 });
        // Still inside the 30s backoff: wait.
        assert_eq!(t.decide("s", 1, t0 + 29, "1|a", silent), Decision::Wait);
        assert_eq!(t.decide("s", 1, t0 + 30, "1|a", silent), Decision::Send { attempt: 2 });
        // The ladder doubles: 60s after attempt 2, 120s after attempt 3.
        assert_eq!(t.decide("s", 1, t0 + 89, "1|a", silent), Decision::Wait);
        assert_eq!(t.decide("s", 1, t0 + 90, "1|a", silent), Decision::Send { attempt: 3 });
        assert_eq!(t.decide("s", 1, t0 + 209, "1|a", silent), Decision::Wait);
        assert_eq!(t.decide("s", 1, t0 + 210, "1|a", silent), Decision::Send { attempt: 4 });
        // The budget is spent with nothing verified — warned ONCE, then silence.
        assert_eq!(t.decide("s", 1, t0 + 10_000, "1|a", silent), Decision::GiveUp { first: true });
        assert_eq!(t.decide("s", 1, t0 + 20_000, "1|a", silent), Decision::GiveUp { first: false });
    }

    #[test]
    fn a_tool_call_confirms_the_incident_without_erasing_it() {
        let mut t = Tracker::default();
        let t0 = 1_000_000;
        assert_eq!(t.decide("s", 1, t0, "1|a", silent), Decision::Send { attempt: 1 });
        // telemetry::record_tool → note_tool_activity → confirm. The record
        // SURVIVES: erasing it here re-opened the incident on the next tick
        // (the error is still painted) and re-sent `continue` into a working
        // agent — the owner's repeat-send report (2026-08-26).
        t.confirm("s", 1);
        assert_eq!(
            t.decide("s", 1, t0 + 60, "1|a", silent),
            Decision::Confirmed { first: false }
        );
        // A tool call with no open incident is a no-op, not a new record.
        t.confirm("s", 2);
        assert_eq!(t.decide("s", 2, t0, "1|a", silent), Decision::Send { attempt: 1 });
    }

    #[test]
    fn a_vanished_error_ends_the_incident_and_a_fresh_one_starts_over() {
        let mut t = Tracker::default();
        let t0 = 1_000_000;
        for i in 0..MAX_ATTEMPTS as u64 {
            t.decide("s", 1, t0 + i * 10_000, "1|a", silent);
        }
        assert_eq!(t.decide("s", 1, t0 + 99_000, "1|a", silent), Decision::GiveUp { first: true });
        // The screen moved on (check_once calls clear when scan_tail misses):
        // whatever the incident's state, it is over.
        t.clear("s", 1);
        // A NEW error later opens a fresh incident with a full budget.
        assert_eq!(t.decide("s", 1, t0 + 100_000, "1|a", silent), Decision::Send { attempt: 1 });
    }

    #[test]
    fn a_second_error_reopens_a_confirmed_incident() {
        // Owner, 2026-08-26: the continue LANDED (hooks saw the turn), the
        // agent ran — and died again. The first error is still painted, so
        // "error visible" never went false and clear() never ran; only the
        // NEW request_id says this is a new failure.
        let mut t = Tracker::default();
        let t0 = 1_000_000;
        assert_eq!(t.decide("s", 1, t0, "1|a", silent), Decision::Send { attempt: 1 });
        assert_eq!(
            t.decide("s", 1, t0 + 20, "1|a", |_| true),
            Decision::Confirmed { first: true }
        );
        // Same painted error, hours later: still silent.
        assert_eq!(
            t.decide("s", 1, t0 + 5_000, "1|a", |_| true),
            Decision::Confirmed { first: false }
        );
        // The resumed turn dies again — both errors on screen: new signature,
        // fresh incident, immediate send with a full budget.
        assert_eq!(
            t.decide("s", 1, t0 + 6_000, "2|a,b", silent),
            Decision::Send { attempt: 1 }
        );
        // And the new incident verifies independently.
        assert_eq!(
            t.decide("s", 1, t0 + 6_020, "2|a,b", |sent| sent == t0 + 6_000),
            Decision::Confirmed { first: true }
        );
    }

    #[test]
    fn a_second_error_also_resets_a_spent_or_waiting_budget() {
        let mut t = Tracker::default();
        let t0 = 1_000_000;
        // Mid-backoff: a new instance replaces the wait.
        assert_eq!(t.decide("s", 1, t0, "1|a", silent), Decision::Send { attempt: 1 });
        assert_eq!(t.decide("s", 1, t0 + 10, "1|a", silent), Decision::Wait);
        assert_eq!(t.decide("s", 1, t0 + 20, "1|b", silent), Decision::Send { attempt: 1 });
        // Spent budget: a new instance starts a fresh one.
        let mut t = Tracker::default();
        for i in 0..MAX_ATTEMPTS as u64 {
            t.decide("s", 1, t0 + i * 10_000, "1|a", silent);
        }
        assert_eq!(t.decide("s", 1, t0 + 90_000, "1|a", silent), Decision::GiveUp { first: true });
        assert_eq!(t.decide("s", 1, t0 + 91_000, "1|c", silent), Decision::Send { attempt: 1 });
    }

    #[test]
    fn signatures_identify_error_instances() {
        // One error, id on the hit line (capture -J joined it).
        assert_eq!(error_signature(REAL), Some("1|30010907-33c4-483a-b024-2ed61321e233".into()));
        // Hard-wrapped shape: the id sits on a continuation line below the hit.
        assert_eq!(
            error_signature(MODEL_ERR),
            Some("1|db05d310-a189-497d-a6f2-b53410863243".into())
        );
        // Two errors visible at once → both ids, either order of appearance.
        let both = format!("{REAL}\nsome output\n{MODEL_ERR}\n");
        assert_eq!(
            error_signature(&both),
            Some("2|30010907-33c4-483a-b024-2ed61321e233,db05d310-a189-497d-a6f2-b53410863243".into())
        );
        // No error, no signature (scan_tail's contract).
        assert_eq!(error_signature("all quiet\n"), None);
        // A quoted error contributes neither a hit nor an id.
        let quoted = format!("[tmm chat 2026-08-26 15:42] human: @b {MODEL_ERR}");
        assert_eq!(error_signature(&quoted.replace('\n', " ")), None);
    }

    #[test]
    fn dead_windows_are_forgotten() {
        let mut t = Tracker::default();
        t.decide("s", 1, 1_000, "1|a", silent);
        t.decide("s", 7, 1_000, "1|a", silent);
        t.decide("other", 1, 1_000, "1|a", silent);
        t.retain_windows("s", &[7]);
        // Window 1 was dropped: a new agent at the same index starts fresh.
        assert_eq!(t.decide("s", 1, 1_001, "1|a", silent), Decision::Send { attempt: 1 });
        // Window 7 and the other session were untouched.
        assert_eq!(t.decide("s", 7, 1_001, "1|a", silent), Decision::Wait);
        assert_eq!(t.decide("other", 1, 1_001, "1|a", silent), Decision::Wait);
    }
}
