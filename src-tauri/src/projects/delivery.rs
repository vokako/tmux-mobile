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

/// The launch recipe's backend and START mode (board #245): the mode the
/// session began in; a recipe from before the field reads `queue`, the v25
/// default. `None` for a window this app did not start.
fn recipe_mode(session: &str, window: &str) -> Option<(crate::backends::Backend, &'static str)> {
    let ws = super::project_for_session(session).ok().flatten()?.path;
    let home = super::agents::home_dir(&ws, window)?;
    let recipe: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(home.join("launch.json")).ok()?).ok()?;
    let backend = crate::backends::Backend::parse(recipe["backend"].as_str()?)?;
    Some((backend, if recipe["input_mode"].as_str() == Some("steer") { "steer" } else { "queue" }))
}

/// The last mode the SCREEN showed, per window, with the pane it was read
/// on: kiro paints nothing about the mode while idle, so a Ctrl+S whose
/// confirmation has scrolled away is remembered — but only for that pane. A
/// restart kills the window (a new pane id) and the session starts from the
/// recipe again, as kiro itself does.
fn seen_modes() -> &'static Mutex<HashMap<(String, String), (String, &'static str)>> {
    static SEEN: OnceLock<Mutex<HashMap<(String, String), (String, &'static str)>>> = OnceLock::new();
    SEEN.get_or_init(|| Mutex::new(HashMap::new()))
}

/// The queue/steer mode a managed agent's session RUNS now (board #271;
/// orchestrator 03:58): what its screen says (`Backend::live_input_mode`,
/// the one reading, in the backend file), else what the screen said last in
/// this same pane, else the launch recipe's start mode. `pane` is the
/// window's pane id and `screen` a capture of it. `None` for a window this
/// app did not start or a backend without the choice.
pub fn input_mode_seen(session: &str, window: &str, pane: &str, screen: &str) -> Option<&'static str> {
    let (backend, start) = recipe_mode(session, window)?;
    if !backend.switches_input_mode() {
        return None;
    }
    let key = (session.to_string(), window.to_string());
    let mut seen = seen_modes().lock().unwrap();
    if seen.len() > 512 {
        seen.clear();
    }
    if let Some(now) = backend.live_input_mode(screen) {
        seen.insert(key, (pane.to_string(), now));
        return Some(now);
    }
    match seen.get(&key) {
        Some((p, m)) if p == pane => Some(*m),
        _ => {
            seen.remove(&key);
            Some(start)
        }
    }
}

/// The same, capturing the window's visible screen now.
pub fn input_mode(session: &str, window: &str) -> Option<&'static str> {
    let pane = crate::tmux::find_window_by_name(session, window).unwrap_or_default();
    let screen = if pane.is_empty() { String::new() } else { crate::tmux::capture_pane_plain(&pane, Some(0)).unwrap_or_default() };
    input_mode_seen(session, window, &pane, &screen)
}

/// Switch the session to `want` with the CLI's live toggle `key` (board
/// #271), as ONE step under the pane's send lock (validator 04:31): two
/// clients asking for the same mode at once must not both read the old mode
/// and both press, toggling it back. Idempotent — nothing is typed when the
/// pane already runs `want` — and verified: after the one key the pane must
/// show `want` within `VERIFY`, else an error, never a second press.
/// `Ok(true)` = switched, `Ok(false)` = was already there.
pub fn switch_input_mode(session: &str, window: &str, target: &str, want: &'static str, key: &str) -> Result<bool, String> {
    // The pane id keys the remembered reading, as for every other caller.
    let pane = crate::tmux::find_window_by_name(session, window).unwrap_or_default();
    let read = || {
        let screen = crate::tmux::capture_pane_plain(target, Some(0)).unwrap_or_default();
        input_mode_seen(session, window, &pane, &screen)
    };
    switch_with(target, want, read, || crate::tmux::send_keys(target, key, false), SWITCH_VERIFY)
}

const SWITCH_VERIFY: std::time::Duration = std::time::Duration::from_secs(3);

/// The switch's logic with its screen and its key injected (tested).
fn switch_with(
    target: &str,
    want: &'static str,
    read: impl Fn() -> Option<&'static str>,
    press: impl FnOnce() -> Result<(), String>,
    verify: std::time::Duration,
) -> Result<bool, String> {
    crate::tmux::with_pane_send_lock(target, || {
        if read() == Some(want) {
            return Ok(false);
        }
        press()?;
        let deadline = std::time::Instant::now() + verify;
        loop {
            std::thread::sleep(std::time::Duration::from_millis(50));
            if read() == Some(want) {
                return Ok(true);
            }
            if std::time::Instant::now() >= deadline {
                return Err(format!("the pane did not confirm {want} mode"));
            }
        }
    })
}

/// Does this window hold lines while busy (board #257)? Only a session
/// that runs QUEUE does: a line held for a steering kiro would wait for an
/// echo a steered line never produces. Read at typing time from the live
/// mode, so a Ctrl+S pressed in the terminal a moment ago is honoured (#271).
pub fn coalesces(session: &str, window: &str) -> bool {
    input_mode(session, window) == Some("queue")
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
    let busy = coalesce && telemetry::turn_busy(session, window);
    if !busy && held.is_empty() {
        return type_now(session, window, target, line, msg_id);
    }
    // Held — for the busy turn's end, or (idle) BEHIND lines still held from a
    // copy-mode refusal or a restart, never ahead of them. A hold that could
    // not be written is today's behaviour, never a silent loss (orchestrator
    // 04:49): type it now, as an ordinary delivery; if that fails too, the
    // existing `undelivered` warn says so.
    if let Err(e) = telemetry::record_held(session, window, line, msg_id) {
        eprintln!("⚠️  could not hold a delivery for {session}:{window} ({e}); typing it now");
        return type_now(session, window, target, line, msg_id);
    }
    if !busy || overdue(&held) {
        flush_locked(session, window, target);
    }
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

/// The fourth trigger (orchestrator, #257 decision 3): at server start, every
/// window that holds lines and is idle once its turn is RECOVERED is flushed
/// at once, so held lines never wait for a client to read the feed. The
/// recovery is `turn_busy`'s own first step (`recovery_mark` →
/// `recover_open_turns`), per session, before the question is asked; a window
/// still running keeps its lines for its stop.
pub fn flush_on_start() {
    for session in telemetry::held_sessions() {
        flush_idle(&session);
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
    // The batch flips as ONE transaction; if it cannot, nothing is typed and
    // every row stays held for the next trigger (orchestrator 04:49).
    if let Err(e) = telemetry::release_held(&ids) {
        eprintln!("⚠️  could not release held deliveries for {session}:{window} ({e}); kept held");
        return;
    }
    match type_text(target, &text) {
        Ok(()) => super::vitals::sniff_window_soon(session, window),
        Err(e) => {
            if let Err(err) = telemetry::rehold(&ids) {
                eprintln!("⚠️  could not re-hold deliveries for {session}:{window} ({err})");
            }
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

    /// A managed agent's home with a launch recipe, in a project the store
    /// knows — what `recipe_mode` reads.
    fn agent_with_recipe(scratch: &crate::tmux::Scratch, session: &str, window: &str, backend: &str, mode: &str) {
        crate::projects::tests::use_test_store();
        let path = scratch.path();
        if crate::projects::project_for_session(session).unwrap().is_none() { crate::projects::with_store(|s| s.insert_project(&crate::projects::Project {
            id: session.into(), name: session.into(), path: path.clone(), icon: None,
            session: session.into(), adopted: false, autostart: false, created_at: 1,
            last_up_at: None, last_seen_at: None, archived: false, room: String::new(),
        })).unwrap(); }
        let home = std::path::Path::new(&path).join(".tmm").join("agents").join(window);
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(home.join("launch.json"), serde_json::json!({ "backend": backend, "input_mode": mode, "cmd": "x" }).to_string()).unwrap();
    }

    /// Board #271 (validator 04:31): two concurrent switches to the same mode
    /// on one pane press ONE key; the second sees the switched pane and
    /// changes nothing. The fake pane repaints only after a delay, the
    /// window in which both would otherwise read the old mode.
    #[test]
    fn concurrent_switches_to_one_mode_press_one_key() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let target = format!("switch-test-{}", uuid::Uuid::new_v4());
        let presses = Arc::new(AtomicUsize::new(0));
        let mode = Arc::new(Mutex::new("queue"));
        let run = |target: String, presses: Arc<AtomicUsize>, mode: Arc<Mutex<&'static str>>| {
            std::thread::spawn(move || {
                let read = || Some(*mode.lock().unwrap());
                let press = || {
                    presses.fetch_add(1, Ordering::SeqCst);
                    let mode = mode.clone();
                    std::thread::spawn(move || {
                        std::thread::sleep(std::time::Duration::from_millis(120));
                        let mut m = mode.lock().unwrap();
                        *m = if *m == "queue" { "steer" } else { "queue" };
                    });
                    Ok(())
                };
                switch_with(&target, "steer", read, press, std::time::Duration::from_secs(2))
            })
        };
        let a = run(target.clone(), presses.clone(), mode.clone());
        let b = run(target.clone(), presses.clone(), mode.clone());
        let mut results = vec![a.join().unwrap(), b.join().unwrap()];
        results.sort();
        assert_eq!(presses.load(Ordering::SeqCst), 1, "one key for two requests");
        assert_eq!(*mode.lock().unwrap(), "steer", "the pane ends in the asked mode");
        assert_eq!(results, vec![Ok(false), Ok(true)], "one switched, the other found it switched");
    }

    #[test]
    fn a_switch_the_pane_never_confirms_is_an_error_after_one_key() {
        let target = format!("switch-test-{}", uuid::Uuid::new_v4());
        let presses = std::cell::Cell::new(0);
        let r = switch_with(&target, "steer", || Some("queue"), || { presses.set(presses.get() + 1); Ok(()) }, std::time::Duration::from_millis(200));
        assert_eq!(presses.get(), 1, "never a blind second press");
        assert!(r.unwrap_err().contains("did not confirm steer"));
        assert_eq!(switch_with(&target, "queue", || Some("queue"), || panic!("no key when already there"), std::time::Duration::from_millis(10)), Ok(false));
        assert!(switch_with(&target, "steer", || None, || Err("tmux says no".into()), std::time::Duration::from_millis(10)).is_err(),
            "a key tmux refused is an error, not a success");
    }

    /// Board #271: the running mode is the SCREEN's, then what the screen
    /// said last in the same pane, then the recipe's start mode — and #257
    /// holds lines only for a session that runs queue.
    #[test]
    fn the_running_input_mode_follows_the_pane_not_the_recipe() {
        let mut scratch = crate::tmux::Scratch::new("inputmode");
        let s = scratch.session("s");
        agent_with_recipe(&scratch, &s, "dev", "kiro", "queue");
        let idle = "kiro · Claude Opus 5.5 · high · ◔ 1%\n›  ask a question or describe a task ↵\n";
        let steer = "kiro · Claude Opus 5.5\n›  Kiro is working · Type to steer · Ctrl+S to queue\n";
        assert_eq!(input_mode_seen(&s, "dev", "%1", idle), Some("queue"), "nothing on screen: the start mode");
        assert_eq!(input_mode_seen(&s, "dev", "%1", steer), Some("steer"), "the screen outranks the recipe");
        assert_eq!(input_mode_seen(&s, "dev", "%1", idle), Some("steer"), "idle kiro paints nothing: the pane's last reading stands");
        assert_eq!(input_mode_seen(&s, "dev", "%2", idle), Some("queue"), "a new pane (restart) starts from the recipe again");
        assert_eq!(input_mode_seen(&s, "dev", "%2", "● Switched to Steer mode\n"), Some("steer"), "a Ctrl+S confirmation");
        // codex switches only at launch: its screen is never read for a mode.
        agent_with_recipe(&scratch, &s, "cx", "codex", "steer");
        assert_eq!(input_mode_seen(&s, "cx", "%3", "›  Kiro is working · Type to queue · Ctrl+S to steer\n"), Some("steer"));
        // A backend with no choice has no mode.
        agent_with_recipe(&scratch, &s, "cl", "claude", "queue");
        assert_eq!(input_mode_seen(&s, "cl", "%4", steer), None);
        assert_eq!(input_mode_seen(&s, "nobody", "%5", steer), None, "not an agent this app started");
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

    /// Decision 3: the server-start trigger flushes every idle window with
    /// held lines after its turn is recovered, across sessions, and leaves a
    /// recovered running turn alone.
    #[test]
    fn the_server_start_flushes_recovered_idle_windows_only() {
        let s = setup("start-idle");
        let r = setup("start-running");
        crate::projects::with_store(|st| {
            st.insert_held_delivery(&s, "dev", "[tmm chat 03:00] lead: @dev idle one", crate::projects::now(), "")?;
            st.insert_activity(&r, "dev", crate::projects::now() * 1000, "prompt", "long turn", "", "app", "", "")?;
            st.insert_held_delivery(&r, "dev", "[tmm chat 03:00] lead: @dev running one", crate::projects::now(), "")
        })
        .unwrap();
        telemetry::forget_process_state(&s);
        telemetry::forget_process_state(&r);
        assert!(telemetry::held_sessions().contains(&s) && telemetry::held_sessions().contains(&r));
        // flush_on_start's per-window step, with the test's fake pane.
        for session in [&s, &r] {
            for w in telemetry::held_windows(session) {
                flush_at(session, &w, "t");
            }
        }
        assert_eq!(typed(), vec!["[tmm chat 03:00] lead: @dev idle one".to_string()]);
        assert_eq!((held(&s, "dev"), held(&r, "dev")), (0, 1), "the recovered running turn keeps its line");
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

    fn sql(stmt: &str) {
        crate::projects::with_store(|st| st.exec_test_sql(stmt)).unwrap();
    }

    /// Review blocker 1 (real SQLite fault, a TEMP trigger as in #249): a
    /// hold that cannot be written is typed NOW as an ordinary delivery —
    /// never reported queued while stored nowhere; if typing fails too, the
    /// existing `undelivered` warn says so.
    #[test]
    fn a_hold_that_cannot_be_written_is_typed_now() {
        let s = setup("hold-fault");
        busy(&s, "dev");
        sql("CREATE TEMP TRIGGER hold_fault BEFORE INSERT ON deliveries
             WHEN NEW.held = 1 AND NEW.line LIKE '%hold-fault%'
             BEGIN SELECT RAISE(ABORT, 'disk full'); END;");
        let line = "[tmm chat 03:00] lead: @dev hold-fault one";
        assert!(deliver_as(&s, "dev", "t", line, "m1", true));
        assert_eq!(typed(), vec![line.to_string()], "typed at once, today's behaviour");
        assert_eq!((held(&s, "dev"), pending(&s, "dev")), (0, 1), "an ordinary pending delivery its echo settles");

        refuse(Some(crate::tmux::PANE_IN_MODE));
        assert!(!deliver_as(&s, "dev", "t", "[tmm chat 03:01] lead: @dev hold-fault two", "", true));
        assert!(warns(&s).iter().any(|w| w.starts_with("undelivered (pane is in copy mode): ") && w.contains("hold-fault two")),
            "neither stored nor typed is SAID: {:?}", warns(&s));
        sql("DROP TRIGGER hold_fault;");
    }

    /// Review blocker 2: the release is ONE transaction. A failure on the
    /// second row's UPDATE leaves the WHOLE batch held and types nothing;
    /// after recovery the batch is typed exactly once.
    #[test]
    fn a_release_that_fails_mid_batch_types_nothing_and_keeps_all_held() {
        let s = setup("release-fault");
        busy(&s, "dev");
        for tag in ["a", "release-fault-b", "c"] {
            deliver_as(&s, "dev", "t", &format!("[tmm chat 03:00] lead: @dev {tag}"), "", true);
        }
        end(&s, "dev");
        sql("CREATE TEMP TRIGGER release_fault BEFORE UPDATE ON deliveries
             WHEN OLD.line LIKE '%release-fault-b%' AND NEW.held = 0
             BEGIN SELECT RAISE(ABORT, 'locked'); END;");
        flush_at(&s, "dev", "t");
        assert!(typed().is_empty(), "nothing is typed when the release fails");
        assert_eq!((held(&s, "dev"), pending(&s, "dev")), (3, 0), "the first row's UPDATE was rolled back too");
        flush_at(&s, "dev", "t");
        assert!(typed().is_empty(), "still failing: still nothing");
        sql("DROP TRIGGER release_fault;");
        flush_at(&s, "dev", "t");
        assert_eq!(typed().len(), 1, "recovered: one prompt");
        assert!(typed()[0].contains("@dev a") && typed()[0].contains("release-fault-b") && typed()[0].contains("@dev c"));
        assert_eq!((held(&s, "dev"), pending(&s, "dev")), (0, 3));
        flush_at(&s, "dev", "t");
        assert_eq!(typed().len(), 1, "exactly once");
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
