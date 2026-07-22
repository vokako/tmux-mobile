//! The supervisor loop: keeps every rostered agent's tmux window alive,
//! adopts survivors after a server restart, nudges stuck agents (self-heal
//! backstop), and puts a fully idle team to sleep to save turns.
//! Split from team.rs 2026-07-22 — content unchanged.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use crate::server::TeamBridge;
use crate::tmux;

use super::{folder_trust_prompt_visible, launch_agent};
use super::workspace::Paths;
use super::TeamConfig;

const RECONCILE_INTERVAL: Duration = Duration::from_secs(3);

/// Self-heal threshold: no `wait` and no heartbeat for this long ⇒ the agent is
/// wedged; the supervisor nudges its window (Esc + reconnect re-prompt). Well
/// above the bus's 90s `unreachable` mark so we only ever auto-restart an agent
/// that has been silent for a genuinely long time, not one merely between tools.
const RECOVERY_STALE_MS: i64 = 1_800_000; // 30 minutes
/// A dead parked `wait` is distinguishable from real work: the bus exposes an
/// idle/online row as `stalled` once its 15-second refresh stops for 90 seconds,
/// while a silent working/thinking row remains `hardworking` for 30 minutes.
/// Recover the former promptly instead of making a human message wait 30 min.
const WAIT_RECOVERY_STALE_MS: i64 = 90_000;
/// Avoid repeatedly interrupting a pane if reconnect itself cannot make
/// progress. A successful `wait` refreshes last_seen and naturally rearms this.
const RECOVERY_COOLDOWN_MS: i64 = 5 * 60 * 1000;
/// Allow one 15-second wait heartbeat after a backend restart before deciding
/// that an adopted idle call is still attached to the dead daemon.
const RESTART_RECOVERY_GRACE: Duration = Duration::from_secs(20);
const RESTART_TRUST_CHECK_DELAY: Duration = Duration::from_secs(2);
/// Idle-sleep threshold: when EVERY non-offline agent has been parked in `wait`
/// (status=`idle`) for this long, the supervisor sends Esc to each pane to
/// cancel the in-flight `wait` MCP call. The agent's CLI falls back to its
/// shell prompt, so an empty wait never completes and consumes another turn.
/// Any new message in the room (typically the human resuming) wakes the team
/// back up via the standard reconnect nudge. Set to 0 to disable.
const IDLE_SLEEP_MS: i64 = agora::mcp::MCP_WAIT_MAX_MS as i64 - 60_000;

/// Reconcile the desired roster into real agent windows, forever (until the
/// process exits). `launched` maps a name → its pane id (or None if adopted).
///
/// Idempotency (the dup-window fix): before launching, we check tmux for an
/// EXISTING window named after the agent in this session. If one is there
/// (server restarted, agent already running), we adopt it instead of opening a
/// second. The previous in-memory-only tracking re-launched every agent on
/// restart, piling up duplicate manager/worker/reviewer windows. Reconnecting
/// adopted agents after a *server* restart is handled separately, once, by
/// `nudge_adopted_agents` (called from recovery) — not here, because the loop's
/// presence check can't tell a healthy agent from one hung on a dead socket.
pub(super) async fn reconcile_loop(bridge: Arc<dyn TeamBridge>, cfg: TeamConfig, room: String, session: String, paths: Paths) {
    const MAX_LAUNCH_FAILURES: u32 = 3; // give up relaunching an agent after this
    let mut launched: HashMap<String, Option<String>> = HashMap::new();
    let mut fail_count: HashMap<String, u32> = HashMap::new();
    let mut last_nudge: HashMap<String, i64> = HashMap::new(); // self-heal cooldown
    let mut launched_any = false;
    let mut sleep_state = SleepState::default();
    loop {
        // Stop the loop once the team is closed. close_team removes the room
        // from the registry AND kills the session — exit on either signal (the
        // room check also covers a team closed before any agent launched).
        if !bridge.room_exists(&room) || (launched_any && !tmux::session_exists(&session)) {
            println!("🜂 team: room '{}' closed; supervisor exiting", room);
            return;
        }
        let employees = bridge.employee_specs(&room);
        let roster = roster_liveness(&*bridge, &room);
        for (name, spec, state) in &employees {
            if state == "disabled" {
                // Kill any window we launched OR an orphan window with this name.
                if let Some(Some(pane)) = launched.get(name) {
                    let _ = tmux::kill_window(pane);
                } else if let Some(pane) = tmux::find_window_by_name(&session, name) {
                    let _ = tmux::kill_window(&pane);
                }
                launched.insert(name.clone(), None);
                continue;
            }
            if launched.contains_key(name) {
                continue;
            }
            // Already online OR a window already exists for it → adopt, don't
            // relaunch (survives server restarts without duplicating windows).
            let online = roster.get(name).map(|(s, _)| s != "offline").unwrap_or(false);
            if online {
                launched.insert(name.clone(), None);
                continue;
            }
            if let Some(pane) = tmux::find_window_by_name(&session, name) {
                println!("🜂 team: adopted existing window for '{}' ({})", name, pane);
                launched.insert(name.clone(), Some(pane));
                launched_any = true;
                continue;
            }
            // Give up after repeated failures (e.g. backend CLI not installed)
            // instead of retrying — and churning windows — every tick forever.
            if fail_count.get(name).copied().unwrap_or(0) >= MAX_LAUNCH_FAILURES {
                continue;
            }
            match launch_agent(name, spec, &cfg, &room, &session, &paths) {
                Ok(pane) => {
                    println!("🜂 team: launched '{}' in window {}", name, pane);
                    launched.insert(name.clone(), Some(pane));
                    fail_count.remove(name);
                    launched_any = true;
                }
                Err(e) => {
                    let n = fail_count.entry(name.clone()).or_insert(0);
                    *n += 1;
                    eprintln!("⚠️  team: launch '{}' failed ({}/{}): {}", name, n, MAX_LAUNCH_FAILURES, e);
                }
            }
        }
        // ── Sleep / wake ─────────────────────────────────────────────────
        // Once every non-offline agent has been parked in `wait` (status =
        // "idle") for IDLE_SLEEP_MS, send Esc to each pane to cancel the
        // in-flight wait. The CLI returns to its shell prompt and stops
        // burning a fresh LLM turn every 50s. Any new bus message — typically
        // the human resuming — wakes the team back up via `nudge_pane`. Skip
        // the existing self-heal while slept: this silence is intentional, not
        // a wedged agent.
        let now = now_ms();
        let online_idle = sleep_state.is_online_idle(&roster);
        let latest_seq = latest_room_seq(&*bridge, &room);
        match sleep_state.step(now, online_idle, latest_seq, IDLE_SLEEP_MS) {
            SleepAction::Sleep => {
                println!(
                    "🜂 team: room '{}' all-idle ≥ {}s — sending Esc to {} agents to sleep",
                    room,
                    IDLE_SLEEP_MS / 1000,
                    employees.iter().filter(|(_, _, s)| s != "disabled").count(),
                );
                for (name, _, state) in &employees {
                    if state == "disabled" { continue; }
                    if let Some(pane) = tmux::find_window_by_name(&session, name) {
                        let _ = tmux::send_keys(&pane, "Escape", false);
                    }
                }
            }
            SleepAction::Wake => {
                println!("🜂 team: room '{}' new message during sleep — waking agents", room);
                for (name, _, state) in &employees {
                    if state == "disabled" { continue; }
                    // Clear the sleep label immediately for a responsive UI; the
                    // agent's own fresh `wait` will refine it to idle/thinking.
                    let _ = bridge.set_agent_status(&room, name, "idle");
                    if let Some(pane) = tmux::find_window_by_name(&session, name) {
                        tokio::spawn(async move { nudge_pane(&pane).await; });
                    }
                }
            }
            SleepAction::None => {}
        }

        // Re-assert "sleeping" every tick while slept. Our Esc takes ~1-2s to
        // actually cancel the agent's in-flight `wait`; in that window the wait
        // loop parks at least once more and writes "idle" (refreshing last_seen),
        // clobbering the status we just set. Setting it only once on the Sleep
        // tick therefore loses the race — the stale "idle" then ages into
        // "stalled" (red) after 90s, which is exactly the bug we saw. Re-stamping
        // it on every 3s tick wins: by the next tick the wait is truly stopped,
        // nothing else writes the row, and apply_presence never ages "sleeping"
        // itself, so the label sticks. Idempotent + cheap (one UPDATE per agent).
        if sleep_state.slept {
            for (name, _, state) in &employees {
                if state == "disabled" { continue; }
                let _ = bridge.set_agent_status(&room, name, "sleeping");
            }
        }

        // ── Self-heal backstop ────────────────────────────────────────────
        // A parked wait that stops its 15-second refresh is exposed by the bus
        // as `stalled` after 90 seconds. Recover that case promptly: a dead MCP
        // transport can otherwise leave persisted human messages unread for
        // the old 30-minute generic threshold. A working/thinking agent instead
        // becomes `hardworking` at 90 seconds and is still left alone until the
        // 30-minute backstop. Skipped while slept: that silence is intentional.
        if !sleep_state.slept {
            for (name, (status, last_seen)) in &roster {
                if !should_recover_agent(status, *last_seen, now, last_nudge.get(name).copied()) {
                    continue;
                }
                if let Some(pane) = tmux::find_window_by_name(&session, name) {
                    println!(
                        "🜂 team: agent '{}' {} for {}s — self-heal nudging {}",
                        name,
                        status,
                        (now - last_seen) / 1000,
                        pane
                    );
                    tokio::spawn(async move { nudge_pane(&pane).await; });
                    last_nudge.insert(name.clone(), now);
                }
            }
        }

        tokio::time::sleep(RECONCILE_INTERVAL).await;
    }
}

fn should_recover_agent(status: &str, last_seen: i64, now: i64, last_nudge: Option<i64>) -> bool {
    if status == "offline" || status == "sleeping" {
        return false;
    }
    if now - last_nudge.unwrap_or(0) < RECOVERY_COOLDOWN_MS {
        return false;
    }
    let stale_for = now - last_seen;
    if status == "stalled" {
        stale_for >= WAIT_RECOVERY_STALE_MS
    } else {
        stale_for >= RECOVERY_STALE_MS
    }
}

fn roster_liveness(bridge: &dyn TeamBridge, room: &str) -> HashMap<String, (String, i64)> {
    let mut out = HashMap::new();
    if let Some(arr) = bridge.roster(room).get("roster").and_then(|v| v.as_array()) {
        for a in arr {
            let name = a.get("name").and_then(|v| v.as_str());
            let status = a.get("status").and_then(|v| v.as_str());
            let last_seen = a.get("last_seen").and_then(|v| v.as_i64()).unwrap_or(0);
            if let (Some(n), Some(s)) = (name, status) {
                out.insert(n.to_string(), (s.to_string(), last_seen));
            }
        }
    }
    out
}

/// Latest message sequence number in `room`, or 0 if the bus is empty / the
/// room hasn't logged anything yet. Used as the anchor we compare against
/// while slept: any seq strictly greater than the anchor means a new message
/// has arrived (the human resuming, almost always) and the team should wake.
fn latest_room_seq(bridge: &dyn TeamBridge, room: &str) -> i64 {
    bridge
        .history(room, 1)
        .get("messages")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.last())
        .and_then(|m| m.get("seq"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

/// What `SleepState::step` decided this tick. The reconcile loop translates
/// each action into pane keystrokes (Esc to sleep, the standard reconnect
/// nudge to wake); separating decision from effect keeps the state machine
/// trivial to unit-test without tmux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SleepAction {
    /// Nothing to do this tick.
    None,
    /// All agents have been idle past the threshold — send Esc to every pane.
    Sleep,
    /// We're slept and a new bus message just arrived — re-prompt every pane
    /// back into `wait` (the existing `nudge_pane` primitive).
    Wake,
}

/// State for the idle-sleep / new-message-wake state machine that lives inside
/// `reconcile_loop`. Pulled into its own type so the decision logic can be
/// covered by plain unit tests (no tmux, no bus). Once `slept` is set, only a
/// new bus message clears it: status changes after sleep are EXPECTED (Esc'd
/// agents have no live `wait`, so they age to "stalled" within ~90s) and must
/// not be treated as a wake signal, otherwise we would oscillate.
#[derive(Debug, Default, Clone, Copy)]
struct SleepState {
    /// First tick on which all online agents looked idle, or `None` if any of
    /// them has been doing real work since.
    idle_since: Option<i64>,
    /// True between the Esc-everyone tick and the wake tick.
    slept: bool,
    /// Latest bus seq we saw at the moment we went to sleep. Wake when the
    /// real latest is strictly greater than this.
    sleep_anchor_seq: i64,
}

impl SleepState {
    /// Advance one tick. `online_idle` is "every non-offline agent has status
    /// `idle`" (caller computes from the roster); `latest_seq` is the latest
    /// bus sequence; `threshold_ms` is `IDLE_SLEEP_MS` (test injection).
    fn step(&mut self, now: i64, online_idle: bool, latest_seq: i64, threshold_ms: i64) -> SleepAction {
        if self.slept {
            // Once slept, ONLY a new message wakes us. (Status drift to
            // "stalled" while the wait MCP call is cancelled is expected.)
            if latest_seq > self.sleep_anchor_seq {
                self.slept = false;
                self.idle_since = None;
                self.sleep_anchor_seq = latest_seq;
                return SleepAction::Wake;
            }
            return SleepAction::None;
        }
        if !online_idle {
            self.idle_since = None;
            return SleepAction::None;
        }
        // Every online agent is idle. Start the clock if we haven't already,
        // then trip Sleep once the threshold is met.
        let started = *self.idle_since.get_or_insert(now);
        if threshold_ms > 0 && now - started >= threshold_ms {
            self.slept = true;
            self.sleep_anchor_seq = latest_seq;
            return SleepAction::Sleep;
        }
        SleepAction::None
    }

    /// Convenience: derive `online_idle` from the supervisor's roster snapshot
    /// (`roster_liveness` output). Empty room or no online agent ⇒ false (so
    /// we never sleep a team that hasn't even come online yet).
    fn is_online_idle(&self, roster: &HashMap<String, (String, i64)>) -> bool {
        let mut saw_online = false;
        for (_, (status, _)) in roster {
            if status == "offline" { continue; }
            saw_online = true;
            if status != "idle" { return false; }
        }
        saw_online
    }
}

/// Wall-clock millis since the epoch — same basis as the bus's `last_seen`, so
/// staleness math in the self-heal backstop lines up across the crate boundary.
pub(super) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// After a *server* restart, reconnect adopted agents whose idle `wait` is
/// still attached to the dead daemon.
///
/// A recovered agent's MCP client lost its connection to the old (now dead)
/// daemon and is hung inside a `wait` tool call. Verified with kiro-cli 2.7.0:
/// the client neither times out nor retries on its own — but it reconnects fine
/// once the dead call is cancelled and a new turn starts. Working agents must
/// not receive Escape: recovery snapshots presence, allows one normal heartbeat
/// interval, and nudges only idle-like agents whose `last_seen` did not advance.
///
/// Runs in a spawned task: it sleeps between keystrokes (TUI needs a beat to
/// settle) and we must not block the recovery path.
pub fn nudge_adopted_agents(
    bridge: Arc<dyn TeamBridge>,
    room: String,
    windows: Vec<(String, String)>,
) {
    let before = roster_liveness(&*bridge, &room);
    tokio::spawn(async move {
        // Trust prompts do not join the roster, so handle them independently
        // while the presence grace period is still running.
        tokio::time::sleep(RESTART_TRUST_CHECK_DELAY).await;
        for (name, pane) in &windows {
            if name != "zsh" {
                confirm_folder_trust_if_visible(pane);
            }
        }

        tokio::time::sleep(RESTART_RECOVERY_GRACE - RESTART_TRUST_CHECK_DELAY).await;
        for (name, pane) in windows {
            if name == "zsh" {
                continue; // the session's initial shell, not an agent
            }
            if confirm_folder_trust_if_visible(&pane) {
                continue;
            }
            let prior = before.get(&name).map(|(status, seen)| (status.as_str(), *seen));
            // Read immediately before acting so a heartbeat that arrives while
            // an earlier pane is being handled also protects this pane.
            let current_roster = roster_liveness(&*bridge, &room);
            let current = current_roster.get(&name).map(|(status, seen)| (status.as_str(), *seen));
            if !should_reconnect_adopted_agent(prior, current) {
                println!("🜂 team: adopted agent '{}' is active or recovered; leaving {} untouched", name, pane);
                continue;
            }
            println!("🜂 team: adopted agent '{}' has an unchanged idle wait; reconnecting {}", name, pane);
            nudge_pane(&pane).await;
        }
    });
}

fn should_reconnect_adopted_agent(
    before: Option<(&str, i64)>,
    after: Option<(&str, i64)>,
) -> bool {
    let Some((before_status, before_seen)) = before else { return false };
    let Some((after_status, after_seen)) = after else { return false };
    let idle_like = |status: &str| matches!(status, "idle" | "online" | "stalled");
    idle_like(before_status) && idle_like(after_status) && after_seen <= before_seen
}

/// Keep recovery neutral: the shared Team contract tells an idle agent to
/// return to `wait`, while an agent with pending context resumes that work.
const RECONNECT_NUDGE: &str = "Continue.";

fn confirm_folder_trust_if_visible(pane: &str) -> bool {
    if let Ok(content) = tmux::capture_pane_plain(pane, Some(80)) {
        if folder_trust_prompt_visible(&content) {
            println!("🜂 team: confirming folder trust in recovered pane {}", pane);
            let _ = tmux::send_keys(pane, "Enter", false);
            return true;
        }
    }
    false
}

/// Press Esc (cancel any stuck in-flight call → back to the prompt), then send
/// the reconnect re-prompt and submit it. Shared by restart-recovery and the
/// supervisor's liveness self-heal. Sleeps between keystrokes (the TUI needs a
/// beat to settle), so callers run it inside a spawned task.
async fn nudge_pane(pane: &str) {
    if confirm_folder_trust_if_visible(pane) {
        return;
    }
    let _ = tmux::send_keys(pane, "Escape", false);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = tmux::send_keys(pane, RECONNECT_NUDGE, true);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = tmux::send_keys(pane, "Enter", false);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idle_roster() -> HashMap<String, (String, i64)> {
        let mut r = HashMap::new();
        r.insert("alice".into(), ("idle".into(), 0));
        r.insert("bob".into(), ("idle".into(), 0));
        r
    }

    #[test]
    fn sleep_state_does_nothing_on_an_empty_room() {
        // No agents at all (a freshly-started team that hasn't booted yet)
        // must not be considered idle — there's no one to sleep.
        let mut s = SleepState::default();
        let empty: HashMap<String, (String, i64)> = HashMap::new();
        assert!(!s.is_online_idle(&empty));
        assert_eq!(s.step(0, false, 0, 60_000), SleepAction::None);
        assert!(!s.slept);
    }

    #[test]
    fn sleep_state_skips_if_any_agent_is_busy() {
        // The whole point: ONE working agent cancels sleep for everyone.
        let mut s = SleepState::default();
        let mut r = idle_roster();
        r.insert("worker".into(), ("working".into(), 0));
        assert!(!s.is_online_idle(&r));
        assert_eq!(s.step(0, false, 0, 60_000), SleepAction::None);
        assert!(s.idle_since.is_none());
    }

    #[test]
    fn sleep_state_offline_agents_dont_block_sleep() {
        // Offline agents are deliberately gone — they don't count toward the
        // "is anyone busy?" check. The remaining online agents alone decide.
        let s = SleepState::default();
        let mut r = idle_roster();
        r.insert("ghost".into(), ("offline".into(), 0));
        assert!(s.is_online_idle(&r));
    }

    #[test]
    fn sleep_state_arms_then_fires_at_threshold() {
        // First idle tick arms the timer; later ticks below threshold = no
        // action; once we cross threshold = Sleep + state flips.
        let mut s = SleepState::default();
        assert_eq!(s.step(1_000, true, 0, 60_000), SleepAction::None);
        assert_eq!(s.idle_since, Some(1_000));
        assert!(!s.slept);

        assert_eq!(s.step(30_000, true, 0, 60_000), SleepAction::None);
        assert!(!s.slept);

        assert_eq!(s.step(61_000, true, 5, 60_000), SleepAction::Sleep);
        assert!(s.slept);
        assert_eq!(s.sleep_anchor_seq, 5);
    }

    #[test]
    fn sleep_state_threshold_zero_disables_sleep() {
        // 0 means "feature off". We still arm idle_since (cheap), but never
        // trip Sleep no matter how long the team has been idle.
        let mut s = SleepState::default();
        assert_eq!(s.step(0, true, 0, 0), SleepAction::None);
        assert_eq!(s.step(86_400_000, true, 0, 0), SleepAction::None); // 1 day
        assert!(!s.slept);
    }

    #[test]
    fn idle_sleep_precedes_the_first_empty_wait_result() {
        assert_eq!(IDLE_SLEEP_MS, 8 * 60 * 1000);
        assert_eq!(
            agora::mcp::MCP_WAIT_MAX_MS as i64 - IDLE_SLEEP_MS,
            60_000
        );
    }

    #[test]
    fn sleep_state_resets_arming_when_an_agent_starts_working() {
        // If the team started looking idle, then someone actually picks work
        // back up before the threshold, we drop the timer entirely.
        let mut s = SleepState::default();
        s.step(1_000, true, 0, 60_000); // arm
        assert_eq!(s.idle_since, Some(1_000));

        s.step(2_000, false, 0, 60_000); // someone working
        assert!(s.idle_since.is_none());

        // And re-arming starts fresh, not from the original idle_since.
        s.step(3_000, true, 0, 60_000);
        assert_eq!(s.idle_since, Some(3_000));
    }

    #[test]
    fn sleep_state_wakes_only_on_a_new_message_seq() {
        // While slept, the rule is intentionally narrow: a strictly newer bus
        // seq than the anchor wakes. Stale roster (status drifting from idle
        // to stalled because the wait MCP call is dead) MUST NOT wake — that
        // would oscillate forever.
        let mut s = SleepState::default();
        // Force slept state at anchor seq=10.
        s.step(0, true, 10, 60_000);
        s.step(60_001, true, 10, 60_000); // → Sleep
        assert!(s.slept);
        assert_eq!(s.sleep_anchor_seq, 10);

        // Same seq, status drifts to stalled (online_idle now false). No wake.
        assert_eq!(s.step(60_002, false, 10, 60_000), SleepAction::None);
        assert!(s.slept);

        // New message: seq jumps to 11. Wake.
        assert_eq!(s.step(60_003, false, 11, 60_000), SleepAction::Wake);
        assert!(!s.slept);
        assert_eq!(s.sleep_anchor_seq, 11);
        assert!(s.idle_since.is_none());
    }

    #[test]
    fn sleep_state_can_re_sleep_after_a_wake_round_trip() {
        // Sleep → wake → idle again → sleep again. Exercises the full cycle.
        let mut s = SleepState::default();
        s.step(0, true, 0, 60_000);
        assert_eq!(s.step(60_000, true, 0, 60_000), SleepAction::Sleep);

        // Human posts (seq 1) → wake.
        assert_eq!(s.step(60_100, false, 1, 60_000), SleepAction::Wake);
        assert!(!s.slept);

        // Team handles it, returns to idle, threshold passes again → sleep.
        s.step(60_200, true, 1, 60_000); // re-arm
        assert_eq!(s.step(120_300, true, 2, 60_000), SleepAction::Sleep);
        assert_eq!(s.sleep_anchor_seq, 2);
    }

    #[test]
    fn recovery_nudges_dead_wait_without_interrupting_long_work() {
        let now = 2_000_000;

        assert!(should_recover_agent(
            "stalled",
            now - WAIT_RECOVERY_STALE_MS,
            now,
            None
        ));
        assert!(!should_recover_agent(
            "hardworking",
            now - WAIT_RECOVERY_STALE_MS,
            now,
            None
        ));
        assert!(should_recover_agent(
            "hardworking",
            now - RECOVERY_STALE_MS,
            now,
            None
        ));
    }

    #[test]
    fn recovery_respects_sleep_offline_and_nudge_cooldown() {
        let now = 2_000_000;
        let stale = now - RECOVERY_STALE_MS;

        assert!(!should_recover_agent("sleeping", stale, now, None));
        assert!(!should_recover_agent("offline", stale, now, None));
        assert!(!should_recover_agent(
            "stalled",
            stale,
            now,
            Some(now - RECOVERY_COOLDOWN_MS + 1)
        ));
        assert!(should_recover_agent(
            "stalled",
            stale,
            now,
            Some(now - RECOVERY_COOLDOWN_MS)
        ));
    }

    #[test]
    fn restart_recovery_nudges_only_an_unchanged_idle_wait() {
        assert_eq!(RECONNECT_NUDGE, "Continue.");
        assert!(should_reconnect_adopted_agent(
            Some(("idle", 1_000)),
            Some(("idle", 1_000)),
        ));
        assert!(should_reconnect_adopted_agent(
            Some(("online", 1_000)),
            Some(("stalled", 1_000)),
        ));

        assert!(!should_reconnect_adopted_agent(
            Some(("idle", 1_000)),
            Some(("idle", 1_001)),
        ));
        assert!(!should_reconnect_adopted_agent(
            Some(("idle", 1_000)),
            Some(("working", 1_000)),
        ));
    }

    #[test]
    fn restart_recovery_never_interrupts_work_sleep_or_unknown_agents() {
        for status in ["thinking", "working", "hardworking", "sleeping", "offline"] {
            assert!(!should_reconnect_adopted_agent(
                Some((status, 1_000)),
                Some((status, 1_000)),
            ), "{status} must not be interrupted");
        }
        assert!(!should_reconnect_adopted_agent(None, Some(("idle", 1_000))));
        assert!(!should_reconnect_adopted_agent(Some(("idle", 1_000)), None));
    }

    // ── existing tests follow ──
}

