//! Typing a chat line into an agent's pane — the ONE path every delivery
//! takes (`@name` mentions, board notices, `[reply]` returns, typed briefs).
//!
//! Board #257 (owner, 2026-09-28: "kiro 有堆积未处理消息时，可以合并投递，保持
//! 效率高效"): a queue-mode agent that is mid-turn used to receive every line
//! as its own queued prompt — its own turn, its own full reply — so a burst of
//! 20 lines was 20 turns, and on 2026-09-27 builder's median delivery lag was
//! ~19.5 min (temp/stall-analysis.md). Now, while such a window's turn is open
//! (`telemetry::turn_busy`, derived from turn edges), a line is HELD: a row in
//! the one deliveries table, persisted, not typed. At the turn's end the held
//! lines are typed as ONE prompt — each line intact with its stamp, in order —
//! and the #249 echo match settles them all from that one submission.
//! Delivery is still typing into a pane (non-negotiable 12), later and
//! combined. An idle target, a steer-mode agent and every backend without a
//! queue/steer switch are typed at once, as before.
//!
//! Every decision and every typing for a window runs under one per-window
//! lock, so a line is never typed twice and a stop that lands between "is it
//! busy?" and "hold it" still finds the row.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use super::telemetry;

/// The most characters one combined prompt carries (board #257). Held lines
/// are packed whole, oldest first, until the next would pass it; the rest
/// stay held for the next turn's end, so an overflow becomes the fewest
/// prompts and later arrivals still join them. A single line longer than
/// this is typed alone, never cut: the echo match needs the whole line.
/// Measured on kiro-cli 2.22.1 (see hub-composer.md).
pub const FLUSH_MAX_CHARS: usize = 32 * 1024;

/// How long a line may be held before it is typed even though the window
/// still reads busy (board #257, measured on kiro-cli 2.22.1): an Escape
/// pressed IN the pane cancels the turn and fires no hook, so the window
/// keeps reading `running` until its next turn — and holding every line for
/// a turn that already ended would leave the agent deaf for good. Past this
/// age the held lines are typed at the next trigger anyway; a busy
/// queue-mode CLI queues them itself (what it did before #257), still as one
/// combined prompt, and an idle one starts the turn that corrects the state.
/// So a long turn gets at most one combined prompt per this interval.
pub const HOLD_MAX_SECS: u64 = 300;

/// What separates two held lines in the combined prompt: a blank line, the
/// same break a team context block already sits behind.
const SEPARATOR: &str = "\n\n";

fn window_lock(session: &str, window: &str) -> Arc<Mutex<()>> {
    static LOCKS: OnceLock<Mutex<HashMap<String, Arc<Mutex<()>>>>> = OnceLock::new();
    let mut map = LOCKS.get_or_init(|| Mutex::new(HashMap::new())).lock().unwrap();
    if map.len() > 256 {
        map.retain(|_, lock| Arc::strong_count(lock) > 1);
    }
    map.entry(format!("{session}:{window}")).or_default().clone()
}

/// Does this window hold lines while busy? Read off its launch recipe — the
/// mode the RUNNING process was started with (a registry edit takes effect
/// at the next restart, which rewrites the recipe). Only backends with a
/// queue/steer switch (kiro, codex) take part; a recipe from before the
/// field reads `queue`, the v25 default.
pub fn coalesces(session: &str, window: &str) -> bool {
    let Some(ws) = super::project_for_session(session).ok().flatten().map(|p| p.path) else { return false };
    let Some(home) = super::agents::home_dir(&ws, window) else { return false };
    let Ok(text) = std::fs::read_to_string(home.join("launch.json")) else { return false };
    let Ok(recipe) = serde_json::from_str::<serde_json::Value>(&text) else { return false };
    let switches = recipe["backend"]
        .as_str()
        .and_then(crate::backends::Backend::parse)
        .is_some_and(|b| b.switches_input_mode());
    switches && recipe["input_mode"].as_str() != Some("steer")
}

/// The live pane of a managed agent window, as a tmux target — `None` for a
/// dead window, a shell, or an agent this app did not start.
pub fn agent_target(session: &str, window: &str) -> Option<String> {
    let ws = super::project_for_session(session).ok().flatten().map(|p| p.path);
    let panes = crate::tmux::list_panes(session).ok()?;
    let p = panes.iter().find(|p| p.active && p.window_name == window)?;
    let is_agent = super::agents::detect_pane(ws.as_deref(), p).is_some();
    (is_agent && super::is_managed_in(ws.as_deref(), window)).then(|| format!("{}:{}.{}", session, p.window, p.pane))
}

/// Deliver one line to one managed agent window whose pane is `target`.
/// True when it was typed or held; false when the pane refused it (said in
/// the feed as `undelivered`, board #250).
pub fn deliver(session: &str, window: &str, target: &str, line: &str, msg_id: &str) -> bool {
    deliver_as(session, window, target, line, msg_id, coalesces(session, window))
}

fn deliver_as(session: &str, window: &str, target: &str, line: &str, msg_id: &str, coalesce: bool) -> bool {
    let lock = window_lock(session, window);
    let _guard = lock.lock().unwrap();
    let held = telemetry::held_rows(session, window);
    if coalesce && telemetry::turn_busy(session, window) {
        telemetry::record_held(session, window, line, msg_id);
        if overdue(&held) {
            flush_locked(session, window, target);
        }
        return true;
    }
    if held.is_empty() {
        return type_now(session, window, target, line, msg_id);
    }
    // Idle, but earlier lines are still held (a copy-mode refusal, a
    // restart): this one goes BEHIND them, never ahead.
    telemetry::record_held(session, window, line, msg_id);
    flush_locked(session, window, target);
    true
}

/// Has the oldest held line waited past `HOLD_MAX_SECS`?
fn overdue(held: &[super::store::DeliveryRow]) -> bool {
    held.first().is_some_and(|r| super::now().saturating_sub(r.ts) >= HOLD_MAX_SECS)
}

/// The pre-#257 path: type the line, then record the promise its echo will
/// settle; a refusal is said at once and records nothing (#250).
fn type_now(session: &str, window: &str, target: &str, line: &str, msg_id: &str) -> bool {
    match type_text(target, line) {
        Ok(()) => {
            telemetry::record_delivery(session, window, line, msg_id);
            // A line just landed in this pane: sniff its vitals once the TUI
            // has repainted (delayed + throttled inside).
            super::vitals::sniff_window_soon(session, window);
            true
        }
        Err(e) => {
            telemetry::record_undelivered(session, window, line, e.trim());
            false
        }
    }
}

/// Type the window's held lines if its turn is over. Triggers: the turn's
/// end edge (the stop hook), the next delivery, and the feed read — never a
/// timer. A busy window keeps them.
pub fn flush(session: &str, window: &str) {
    if telemetry::held_rows(session, window).is_empty() {
        return;
    }
    let Some(target) = agent_target(session, window) else { return };
    flush_at(session, window, &target);
}

/// `flush` for every window of a session that holds lines (the feed read:
/// it also picks up what a restart left held for an agent that went idle).
pub fn flush_idle(session: &str) {
    for window in telemetry::held_windows(session) {
        flush(session, &window);
    }
}

fn flush_at(session: &str, window: &str, target: &str) {
    let lock = window_lock(session, window);
    let _guard = lock.lock().unwrap();
    if telemetry::turn_busy(session, window) && !overdue(&telemetry::held_rows(session, window)) {
        return;
    }
    flush_locked(session, window, target);
}

/// Type the first combined prompt of the held lines. The rows are marked
/// typed BEFORE the keys go in (an echo racing the Enter must find them) and
/// put back on a refusal: a pane in copy mode keeps them held, reported once,
/// retried at the next trigger (board #257 decision 2 — the sender was
/// already told the line is queued).
fn flush_locked(session: &str, window: &str, target: &str) {
    let rows = telemetry::held_rows(session, window);
    let take = first_prompt(&rows.iter().map(|r| r.line.as_str()).collect::<Vec<_>>(), FLUSH_MAX_CHARS);
    if take == 0 {
        return;
    }
    let batch = &rows[..take];
    let ids: Vec<i64> = batch.iter().map(|r| r.id).collect();
    let text = batch.iter().map(|r| r.line.as_str()).collect::<Vec<_>>().join(SEPARATOR);
    telemetry::release_held(&ids);
    match type_text(target, &text) {
        Ok(()) => super::vitals::sniff_window_soon(session, window),
        Err(e) => {
            telemetry::rehold(&ids);
            let reason = if e.trim() == crate::tmux::PANE_IN_MODE { "pane in copy mode".to_string() } else { e.trim().to_string() };
            telemetry::record_held_blocked(session, window, &rows, &reason);
        }
    }
}

/// An interrupt ends the window's turn and DROPS its held lines (board #257
/// decision 1), under the window's lock so a delivery cannot flush them in
/// between. `end` records the turn's end (`telemetry::record_interrupt`).
pub fn interrupt(session: &str, window: &str, end: impl FnOnce()) {
    let lock = window_lock(session, window);
    let _guard = lock.lock().unwrap();
    end();
    telemetry::drop_held(session, window);
}

/// How many of `lines` (oldest first) the next combined prompt carries:
/// whole lines while the total, separators included, stays within `max`;
/// always at least one. Pure.
fn first_prompt(lines: &[&str], max: usize) -> usize {
    let mut chars = 0;
    for (i, line) in lines.iter().enumerate() {
        let cost = line.chars().count() + if i == 0 { 0 } else { SEPARATOR.len() };
        if i > 0 && chars + cost > max {
            return i;
        }
        chars += cost;
    }
    lines.len()
}

#[cfg(not(test))]
fn type_text(target: &str, text: &str) -> Result<(), String> {
    crate::tmux::send_command(target, text)
}

#[cfg(test)]
thread_local! {
    /// Test seam: when `Some`, typing is recorded here instead of reaching
    /// tmux, and `REFUSE` decides the outcome.
    static TYPED: std::cell::RefCell<Option<Vec<String>>> = const { std::cell::RefCell::new(None) };
    static REFUSE: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
fn type_text(target: &str, text: &str) -> Result<(), String> {
    let faked = TYPED.with(|t| t.borrow().is_some());
    if !faked {
        return crate::tmux::send_command(target, text);
    }
    if let Some(e) = REFUSE.with(|r| r.borrow().clone()) {
        return Err(e);
    }
    TYPED.with(|t| t.borrow_mut().as_mut().unwrap().push(text.to_string()));
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::projects::telemetry::{record_notification, record_prompt};

    fn fake() {
        TYPED.with(|t| *t.borrow_mut() = Some(Vec::new()));
        REFUSE.with(|r| *r.borrow_mut() = None);
    }
    fn typed() -> Vec<String> {
        TYPED.with(|t| t.borrow().clone().unwrap_or_default())
    }
    fn refuse(e: Option<&str>) {
        REFUSE.with(|r| *r.borrow_mut() = e.map(str::to_string));
    }
    fn pending(session: &str, window: &str) -> usize {
        crate::projects::with_store(|s| s.pending_deliveries(session, Some(window))).unwrap().len()
    }
    fn held(session: &str, window: &str) -> usize {
        telemetry::held_rows(session, window).len()
    }
    fn warns(session: &str) -> Vec<String> {
        telemetry::recent_events(session, 0).into_iter().filter(|e| e.kind == "warn").map(|e| e.text).collect()
    }
    fn busy(session: &str, window: &str) {
        record_prompt(session, window, "[tmm chat 2026-09-28 03:00] human: @dev a long task");
        assert!(telemetry::turn_busy(session, window));
    }
    fn end(session: &str, window: &str) {
        record_notification(session, window, "completed", crate::projects::now());
    }
    fn setup(tag: &str) -> String {
        crate::projects::tests::use_test_store();
        fake();
        format!("coalesce-{tag}-{}", uuid::Uuid::new_v4())
    }

    /// The owner's case: a burst of 10 to a busy queue-mode agent is ONE
    /// prompt at the turn's end, all 10 rows settled by its one echo.
    #[test]
    fn a_burst_to_a_busy_queue_agent_is_one_prompt_at_turn_end() {
        let s = setup("burst");
        busy(&s, "dev");
        let lines: Vec<String> = (0..10)
            .map(|i| format!("[tmm chat 2026-09-28 03:0{i}] {}: @dev item {i}\nsecond line {i}", if i % 2 == 0 { "lead" } else { "validator" }))
            .collect();
        for (i, line) in lines.iter().enumerate() {
            assert!(deliver_as(&s, "dev", "t", line, &format!("m{i}"), true));
        }
        assert!(typed().is_empty(), "nothing is typed while the turn is open");
        assert_eq!((held(&s, "dev"), pending(&s, "dev")), (10, 0));
        assert!(warns(&s).is_empty());

        end(&s, "dev");
        flush_at(&s, "dev", "t");
        let prompts = typed();
        assert_eq!(prompts.len(), 1, "one prompt");
        assert_eq!(prompts[0], lines.join("\n\n"), "every line intact, in order");
        assert_eq!((held(&s, "dev"), pending(&s, "dev")), (0, 10));

        // The agent submits it (newlines glued, the tmux extended-keys shape):
        // the #249 matcher settles all ten from the one echo.
        assert!(record_prompt(&s, "dev", &prompts[0].replace('\n', "")));
        assert_eq!(pending(&s, "dev"), 0);
        let prompt = telemetry::recent_events(&s, 0).into_iter().rfind(|e| e.kind == "prompt").unwrap();
        assert_eq!(prompt.deliveries.len(), 10, "the echo names every row it settled");
        assert_eq!(prompt.deliveries.iter().map(|d| d.msg.clone()).collect::<Vec<_>>(), (0..10).map(|i| format!("m{i}")).collect::<Vec<_>>());
    }

    #[test]
    fn an_idle_target_a_steer_agent_and_a_non_switching_backend_are_typed_at_once() {
        let s = setup("direct");
        assert!(deliver_as(&s, "dev", "t", "[tmm chat 03:00] lead: @dev idle", "", true));
        assert_eq!(typed().len(), 1, "idle: typed now");
        busy(&s, "dev");
        assert!(deliver_as(&s, "dev", "t", "[tmm chat 03:01] lead: @dev steer", "", false));
        assert_eq!(typed().len(), 2, "not coalescing (steer / claude): typed now, busy or not");
        assert_eq!((held(&s, "dev"), pending(&s, "dev")), (0, 2));
        // A stop with nothing held types nothing.
        end(&s, "dev");
        flush_at(&s, "dev", "t");
        assert_eq!(typed().len(), 2);
    }

    /// A line for an idle agent that still has held lines goes behind them.
    #[test]
    fn a_new_line_joins_lines_still_held_in_order() {
        let s = setup("join");
        busy(&s, "dev");
        deliver_as(&s, "dev", "t", "[tmm chat 03:00] lead: @dev one", "", true);
        end(&s, "dev");
        deliver_as(&s, "dev", "t", "[tmm chat 03:01] lead: @dev two", "", true);
        assert_eq!(typed(), vec!["[tmm chat 03:00] lead: @dev one\n\n[tmm chat 03:01] lead: @dev two".to_string()]);
        assert_eq!((held(&s, "dev"), pending(&s, "dev")), (0, 2));
    }

    /// Decision 1: an interrupt drops the held lines, one warn each, no retry.
    #[test]
    fn an_interrupt_drops_held_lines_with_one_warn_each() {
        let s = setup("intr");
        busy(&s, "dev");
        deliver_as(&s, "dev", "t", "[tmm chat 03:00] lead: @dev a", "m1", true);
        deliver_as(&s, "dev", "t", "[tmm chat 03:00] lead: @dev b", "m2", true);
        interrupt(&s, "dev", || telemetry::record_interrupt(&s, "dev"));
        assert_eq!((held(&s, "dev"), pending(&s, "dev")), (0, 0));
        let w = warns(&s);
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(w.iter().all(|t| t.starts_with("undelivered (interrupted): ")), "{w:?}");
        let rows = telemetry::recent_events(&s, 0);
        assert!(rows.iter().filter(|e| e.kind == "warn").all(|e| e.deliveries.is_empty()), "no row id: nothing to retract");
        flush_at(&s, "dev", "t");
        assert!(typed().is_empty(), "nothing retried");
    }

    /// Decision 2: copy mode at the flush keeps the lines held, says so ONCE,
    /// and the next trigger types them.
    #[test]
    fn copy_mode_at_flush_keeps_lines_held_warns_once_and_retries() {
        let s = setup("copy");
        busy(&s, "dev");
        deliver_as(&s, "dev", "t", "[tmm chat 03:00] lead: @dev a", "", true);
        deliver_as(&s, "dev", "t", "[tmm chat 03:00] lead: @dev b", "", true);
        end(&s, "dev");
        refuse(Some(crate::tmux::PANE_IN_MODE));
        flush_at(&s, "dev", "t");
        flush_at(&s, "dev", "t");
        assert_eq!((held(&s, "dev"), pending(&s, "dev")), (2, 0), "still owed, not typed");
        assert_eq!(warns(&s), vec!["held (pane in copy mode): 2 lines will be typed when it clears".to_string()]);
        // A line arriving meanwhile queues behind them, still one warn.
        deliver_as(&s, "dev", "t", "[tmm chat 03:01] lead: @dev c", "", true);
        assert_eq!(held(&s, "dev"), 3);
        assert_eq!(warns(&s).len(), 2, "the new row gets its own single report");
        refuse(None);
        flush_at(&s, "dev", "t");
        assert_eq!(typed().len(), 1);
        assert_eq!((held(&s, "dev"), pending(&s, "dev")), (0, 3));
        // Released rows start fresh: their ack clock and report are the
        // ordinary delivery's (the held-time report does not suppress it).
        assert!(crate::projects::with_store(|st| st.pending_deliveries(&s, Some("dev"))).unwrap().iter().all(|r| !r.warned));
    }

    /// A restart keeps held lines (they are rows): an idle agent gets them
    /// at the next trigger, a running one at its turn's end.
    #[test]
    fn held_lines_survive_a_restart() {
        let s = setup("restart");
        busy(&s, "dev");
        deliver_as(&s, "dev", "t", "[tmm chat 03:00] lead: @dev a", "", true);
        telemetry::forget_process_state(&s);
        // The prompt row is not persisted in unit tests: after the "restart"
        // the window has no facts, so it reads idle and the next trigger types.
        flush_at(&s, "dev", "t");
        assert_eq!(typed().len(), 1);

        let s = setup("restart-busy");
        crate::projects::with_store(|st| st.insert_activity(&s, "dev", crate::projects::now() * 1000, "prompt", "long turn", "", "app", "", "")).unwrap();
        crate::projects::with_store(|st| st.insert_held_delivery(&s, "dev", "[tmm chat 03:00] lead: @dev a", crate::projects::now(), "")).unwrap();
        telemetry::forget_process_state(&s);
        flush_at(&s, "dev", "t");
        assert!(typed().is_empty(), "the replayed turn is open: still held");
        end(&s, "dev");
        flush_at(&s, "dev", "t");
        assert_eq!(typed().len(), 1);
    }

    /// kiro-cli 2.22.1 measured: an Escape typed IN the pane cancels the turn
    /// and fires no hook, so the window reads busy until its next turn. A
    /// line held past HOLD_MAX_SECS is typed at the next trigger anyway — the
    /// agent is never left deaf.
    #[test]
    fn a_line_held_too_long_is_typed_although_the_window_reads_busy() {
        let s = setup("overdue");
        busy(&s, "dev");
        deliver_as(&s, "dev", "t", "[tmm chat 03:00] lead: @dev a", "", true);
        flush_at(&s, "dev", "t");
        assert!(typed().is_empty(), "fresh: held");
        crate::projects::with_store(|st| st.exec_test_sql(&format!(
            "UPDATE deliveries SET ts = ts - {} WHERE session = '{s}'", HOLD_MAX_SECS
        ))).unwrap();
        // The feed-read trigger types it, busy or not.
        flush_at(&s, "dev", "t");
        assert_eq!(typed(), vec!["[tmm chat 03:00] lead: @dev a".to_string()]);
        // So does the next delivery, which joins the overdue line.
        deliver_as(&s, "dev", "t", "[tmm chat 03:10] lead: @dev b", "", true);
        crate::projects::with_store(|st| st.exec_test_sql(&format!(
            "UPDATE deliveries SET ts = ts - {} WHERE session = '{s}' AND held = 1", HOLD_MAX_SECS
        ))).unwrap();
        deliver_as(&s, "dev", "t", "[tmm chat 03:11] lead: @dev c", "", true);
        assert_eq!(typed().len(), 2);
        assert_eq!(typed()[1], "[tmm chat 03:10] lead: @dev b\n\n[tmm chat 03:11] lead: @dev c");
        assert_eq!((held(&s, "dev"), pending(&s, "dev")), (0, 3));
    }

    /// The bound: whole lines, oldest first, the rest wait for the next end;
    /// an oversized line goes alone and is never cut.
    #[test]
    fn a_flush_packs_whole_lines_up_to_the_cap() {
        assert_eq!(first_prompt(&[], 10), 0);
        assert_eq!(first_prompt(&["aaaa", "bbbb", "cccc"], 10), 2, "4 + 2 + 4 = 10 fits, the third would not");
        assert_eq!(first_prompt(&["aaaa", "bbbb", "cccc"], 9), 1);
        assert_eq!(first_prompt(&["a".repeat(50).as_str(), "b"], 10), 1, "an oversized line alone");

        let s = setup("cap");
        busy(&s, "dev");
        let big = "x".repeat(FLUSH_MAX_CHARS / 2);
        for i in 0..3 {
            deliver_as(&s, "dev", "t", &format!("[tmm chat 03:0{i}] lead: @dev {big}"), "", true);
        }
        end(&s, "dev");
        flush_at(&s, "dev", "t");
        assert_eq!(typed().len(), 1, "one prompt per turn end");
        assert_eq!((held(&s, "dev"), pending(&s, "dev")), (2, 1), "the overflow waits for the next end");
        assert!(typed()[0].chars().count() <= FLUSH_MAX_CHARS + 64);
    }
}
