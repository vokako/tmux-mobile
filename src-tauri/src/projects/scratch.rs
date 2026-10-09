//! The scratch terminal's session (board #324, owner 2026-10-09: "这个
//! Terminal 只是临时用的，并且跟你的项目没有关系，我就是单纯想在系统上去运行个
//! Terminal").
//!
//! Every tmux session is a project (`auto_adopt_once`), except ONE: the
//! session the desktop's scratch panel opens. What makes a session that one
//! is OWNERSHIP, never the name alone — `owned` is the single predicate every
//! path asks (ensure, kill, auto-adopt):
//!
//! - the session `tmm-scratch` exists (exact target),
//! - it carries our mark, the tmux session option `@tmm-scratch` = `1`, set
//!   when we created it (a native tmux fact: no new store, gone with the
//!   session),
//! - and no project row declares that session (a project that is down still
//!   holds its name).
//!
//! A name held by anything else — a plain session the user made, or a
//! project — is `Taken`: we refuse with the reason and never kill, rename or
//! undeclare it. The session starts in `$HOME` with the default shell, runs
//! no agent and installs no hooks beyond its own keep-alive.
//!
//! Two things make it the owner's "hidden quick terminal" rather than just
//! another session (board #326, owner 2026-10-09: "不需要作为一个真实的
//! project 显示出来，就是一个隐藏的会话就好…退出这个隐藏会话时，也不要关掉"):
//!
//! - **Hidden from every listing.** `hidden_session()` — derived from the one
//!   `owned` predicate, never from the name — is the session the session/pane
//!   listings drop (`server/rpc.rs`). The panel is its only door.
//! - **Exiting the shell does not end it.** tmux's own mechanism, not a
//!   process manager of ours: `remain-on-exit` keeps the pane when its shell
//!   exits, and the session's `pane-died` hook respawns it IN PLACE, so the
//!   `session:window.pane` target the Terminal subscribes stays valid.

use crate::tmux;

pub const SCRATCH_SESSION: &str = "tmm-scratch";
const MARK: &str = "@tmm-scratch";
/// The keep-alive hook's body. Plain `respawn-pane`, with no target and no
/// `-k`:
///
/// - no TARGET, because tmux runs a `pane-died` hook with the pane that died
///   as its target (measured on tmux 3.6a with two panes — only the dead pane
///   was respawned, the live pane kept its shell pid). The explicit
///   `respawn-pane -t "#{pane_id}"` form stores fine and never fires on 3.6a;
/// - no `-k`, so the hook cannot kill a shell either: if `ensure` repaired the
///   pane between the death and the hook, tmux refuses the hook's respawn
///   instead of killing the shell the reader is already typing into
///   (`tmux::respawn_pane`).
const RESPAWN_HOOK: &str = "respawn-pane";

/// The session name every path below uses. Production: `SCRATCH_SESSION`.
/// A test points it at a name its `tmux::Scratch` guard owns, so a test run
/// never touches the user's real scratch session (all cargo runs share one
/// tmux server — testing.md).
#[cfg(not(test))]
pub fn name() -> String {
    SCRATCH_SESSION.to_string()
}
#[cfg(test)]
thread_local! {
    static TEST_NAME: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}
#[cfg(test)]
pub fn name() -> String {
    TEST_NAME.with(|n| n.borrow().clone()).unwrap_or_else(|| SCRATCH_SESSION.to_string())
}
#[cfg(test)]
pub(crate) fn use_test_name(name: &str) {
    TEST_NAME.with(|n| *n.borrow_mut() = Some(name.to_string()));
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ownership {
    Ours,
    Absent,
    Taken(String),
}

/// The pure verdict over the three facts (tested without tmux).
pub(crate) fn verdict(exists: bool, marked: bool, project: Option<&str>) -> Ownership {
    if let Some(name) = project {
        return Ownership::Taken(format!("the name '{}' belongs to project '{name}'", self::name()));
    }
    match (exists, marked) {
        (false, _) => Ownership::Absent,
        (true, true) => Ownership::Ours,
        (true, false) => Ownership::Taken(format!("a tmux session named '{}' already exists and is not the scratch terminal", name())),
    }
}

fn declared_by() -> Result<Option<String>, String> {
    Ok(super::project_for_session(&name())?.map(|p| p.name))
}

/// THE predicate.
pub fn owned() -> Result<Ownership, String> {
    let n = name();
    let exists = tmux::session_exists(&n);
    let marked = exists && tmux::session_option(&n, MARK).as_deref() == Some("1");
    Ok(verdict(exists, marked, declared_by()?.as_deref()))
}

/// True when `session` is our scratch session — the guard auto-adopt and
/// the other project paths share.
pub fn is_scratch(session: &str) -> bool {
    session == name() && matches!(owned(), Ok(Ownership::Ours))
}

/// The session every listing HIDES (board #326) — `Some` only while it is
/// ours. A plain same-name session the user made, or a name a project holds,
/// is listed like anything else, and a failed read hides nothing: the name
/// alone is never enough to drop a row.
pub fn hidden_session() -> Option<String> {
    match owned() {
        Ok(Ownership::Ours) => Some(name()),
        _ => None,
    }
}

/// Exiting the shell must not end the session (owner: "退出这个隐藏会话时，
/// 也不要关掉"). Two native tmux facts, applied to the pane we are about to
/// hand out and re-applied by every `ensure` (declaration is truth: a session
/// created before this existed, or one whose hook was removed, self-heals):
///
/// - `remain-on-exit` (a WINDOW option, hence the pane target) keeps the pane
///   when its shell exits instead of closing the window — and the window is
///   the session's only one, so the session cannot end that way either;
/// - the session's `pane-died` hook respawns that pane in place, keeping its
///   `%id`, its `session:window.pane` target and its start directory
///   (measured, tmux 3.6a: a pane created with `-c /tmp/probe-cwd`, `cd /usr`,
///   then `exit`, came back in `/tmp/probe-cwd` — never `/`; our session is
///   created with `-c $HOME`).
///
/// Both are scoped to this window and this session; no global option or hook
/// is touched.
fn keep_alive(pane: &str) -> Result<(), String> {
    tmux::set_window_option(pane, "remain-on-exit", "on")?;
    tmux::set_hook(&name(), "pane-died", RESPAWN_HOOK)
}

/// Ensure the scratch session exists and answer `{session, target}`: the
/// concrete `session:window.pane` of its first window's first pane (base
/// index honoured), the target the Terminal subscribes, types and resizes
/// against. A concurrent ensure that created it first is the only create
/// failure accepted, and only once `owned` says it is ours.
///
/// The pane handed back is always LIVE: a pane that died before the keep-alive
/// existed (an older session, a hook that was removed, an exit between create
/// and hook) is repaired here. "Only a dead pane" is TMUX's decision, not a
/// check-then-act of ours (#326 review P1): `respawn_pane` carries no `-k`, so
/// tmux refuses it while a shell or a command is still running. That is what
/// makes a repeated ensure safe — our own `pane_live` reading is only a fast
/// path, and it is stale by the time the next line runs, because the
/// `pane-died` hook or another ensure may revive the pane in between. The END
/// STATE is therefore what decides: a refusal whose pane is now live is
/// success (someone else won the race), and only a pane that is still not live
/// is an error.
pub fn ensure() -> Result<serde_json::Value, String> {
    match owned()? {
        Ownership::Taken(why) => return Err(why),
        Ownership::Ours => {}
        Ownership::Absent => {
            if let Err(e) = tmux::new_session(&name(), None, None) {
                let raced = e.contains("already exists") || e.contains("duplicate session");
                if !raced || owned()? != Ownership::Ours {
                    // A raced create that is not ours yet may be the other
                    // ensure between create and mark: give it one beat.
                    std::thread::sleep(std::time::Duration::from_millis(150));
                    if !raced || owned()? != Ownership::Ours {
                        return Err(e);
                    }
                }
            } else {
                tmux::set_session_option(&name(), MARK, "1")?;
            }
        }
    }
    let target = first_pane()?;
    keep_alive(&target)?;
    // Everything from here on names the pane by its tmux `%id`, read once:
    // `session:window.pane` is a POSITION, and a window or pane created beside
    // ours renumbers it, so the check and the repair could otherwise land on
    // two different panes. The id is stable for the pane's whole life — it is
    // what `respawn-pane` keeps — while the client needs the positional target
    // for its subscription, which is what we answer with.
    let id = tmux::pane_format(&target, "#{pane_id}")
        .ok_or_else(|| format!("the scratch terminal's pane {target} cannot be read"))?;
    // Unless the pane is PROVEN live, offer it a shell: an unreadable answer
    // falls on the repair side, which costs nothing now that the repair cannot
    // kill anything.
    if tmux::pane_live(&id) != Some(true) {
        let attempt = tmux::respawn_pane(&id);
        if tmux::pane_live(&id) != Some(true) {
            return Err(attempt.err().unwrap_or_else(||
                format!("the scratch terminal's shell will not start in {target}")));
        }
    }
    // The answer is the position of the pane we actually verified, read from
    // its %id at the END (#326 review): the string `first_pane` gave us is a
    // POSITION, and a pane created or killed beside ours while we worked
    // renumbers it, so handing back the pre-repair string could name a pane
    // this call never checked. Reading it back from the id cannot.
    let target = tmux::pane_format(&id, "#{session_name}:#{window_index}.#{pane_index}")
        .ok_or_else(|| format!("the scratch terminal's pane {id} vanished while it was being prepared"))?;
    Ok(serde_json::json!({ "session": name(), "target": target }))
}

fn first_pane() -> Result<String, String> {
    let n = name();
    let panes = tmux::list_panes(&n)?;
    let first = panes
        .iter()
        .min_by_key(|p| (p.window, p.pane))
        .ok_or_else(|| "the scratch session has no pane".to_string())?;
    Ok(format!("{}:{}.{}", n, first.window, first.pane))
}

/// Kill the scratch session — only when it is ours. The keep-alive cannot
/// fight this: `pane-died` is a SESSION option, so it goes with the session
/// (measured on tmux 3.6a: `kill-session` on a session carrying the hook
/// leaves no session behind, not a respawned one). Explicit Kill stays the
/// one way to end it.
pub fn kill() -> Result<serde_json::Value, String> {
    match owned()? {
        Ownership::Ours => {
            tmux::kill_session(&name())?;
            Ok(serde_json::json!({ "killed": true }))
        }
        Ownership::Absent => Ok(serde_json::json!({ "killed": false })),
        Ownership::Taken(why) => Err(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ownership_is_existence_plus_our_mark_and_no_project() {
        assert_eq!(verdict(false, false, None), Ownership::Absent);
        assert_eq!(verdict(true, true, None), Ownership::Ours);
        assert!(matches!(verdict(true, false, None), Ownership::Taken(_)), "a plain session of that name is not ours");
        // A project that declares the name holds it even while it is down.
        assert!(matches!(verdict(false, false, Some("scratchpad")), Ownership::Taken(_)));
        assert!(matches!(verdict(true, true, Some("scratchpad")), Ownership::Taken(_)), "a declaration wins over a stale mark");
    }

    /// Poll a tmux fact that an asynchronous hook changes. The hook fires in
    /// milliseconds; the budget is for a loaded host, not for the mechanism.
    fn settles(mut done: impl FnMut() -> bool) -> bool {
        for _ in 0..60 {
            if done() {
                return true;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        false
    }
    fn pane_pid(target: &str) -> String {
        tmux::pane_format(target, "#{pane_pid}").unwrap_or_default()
    }
    fn global_hook() -> Option<String> {
        tmux::run_tmux(&["show-options", "-g", "-v", "pane-died"]).ok().map(|s| s.trim().to_string())
    }

    /// The hidden session stays alive across an exit, and `ensure` repairs a
    /// pane that died before the keep-alive existed (board #326, review P1-b).
    /// Under a guard-owned name: the production `tmm-scratch` and the global
    /// tmux options/hooks are never touched.
    #[test]
    fn the_shell_can_exit_without_ending_the_session_and_a_dead_pane_is_repaired() {
        crate::projects::tests::use_test_store();
        let real_before = tmux::session_exists(SCRATCH_SESSION);
        let real_hook_before = tmux::session_hook(SCRATCH_SESSION, "pane-died");
        let global_hook_before = global_hook();
        let mut guard = tmux::Scratch::new("ka");
        let n = guard.session("k");
        use_test_name(&n);
        if tmux::new_session(&guard.session("probe"), None, None).is_err() {
            eprintln!("no tmux server — skipping");
            return;
        }
        let first = ensure().expect("created");
        let target = first["target"].as_str().unwrap().to_string();
        // The keep-alive is tmux's own, scoped to this window and session.
        assert_eq!(tmux::window_option(&target, "remain-on-exit").as_deref(), Some("on"));
        // The body is asserted LITERALLY, not against RESPAWN_HOOK: comparing
        // the constant to itself would pass however it changed, and the two
        // things that must not come back — a `-k` and an explicit target —
        // are exactly a change to this string (#326 review P1).
        assert_eq!(tmux::session_hook(&n, "pane-died").as_deref(), Some("respawn-pane"));
        assert_eq!(RESPAWN_HOOK, "respawn-pane");
        // A repeated ensure re-applies it and never kills a running shell.
        let pid = pane_pid(&target);
        assert!(!pid.is_empty());
        for _ in 0..3 {
            assert_eq!(ensure().unwrap()["target"], target.as_str());
        }
        assert_eq!(pane_pid(&target), pid, "a healthy ensure leaves the shell alone");
        assert_eq!(tmux::pane_live(&target), Some(true));

        // Ctrl-D / `exit`: the pane comes back IN PLACE — same target, so the
        // Terminal's subscription stays valid — and the session survives.
        tmux::send_command(&target, "exit").expect("typed exit");
        assert!(settles(|| pane_pid(&target) != pid && tmux::pane_live(&target) == Some(true)),
            "the pane-died hook respawns the shell in place");
        assert!(tmux::session_exists(&n), "the hidden session does not end with its shell");
        assert_eq!(first_pane().unwrap(), target, "and keeps the same session:window.pane");
        let revived = pane_pid(&target);

        // A pane that died BEFORE the keep-alive existed (an old session, a
        // removed hook): ensure repairs it instead of handing out a dead pane.
        tmux::run_tmux(&["set-hook", "-u", "-t", &n, "pane-died"]).expect("hook removed");
        assert_eq!(tmux::session_hook(&n, "pane-died"), None);
        tmux::send_command(&target, "exit").expect("typed exit");
        assert!(settles(|| tmux::pane_live(&target) == Some(false)), "with no hook the pane stays dead");
        // The review's exact sequence: this path has OBSERVED the pane dead,
        // and another path (the hook, or a concurrent ensure) revives it
        // before the repair runs. The repair must kill nothing.
        assert_eq!(tmux::pane_live(&target), Some(false), "observed dead");
        tmux::respawn_pane(&target).expect("another path revives it first");
        let winner = pane_pid(&target);
        assert_eq!(tmux::pane_live(&target), Some(true));
        assert!(tmux::respawn_pane(&target).is_err(), "the late repair is refused, not applied");
        assert_eq!(pane_pid(&target), winner, "the winner's shell is untouched");
        // ensure over that same state is idempotent: same pane, still alive.
        let after_race = ensure().expect("idempotent over a revived pane");
        assert_eq!(after_race["target"], target.as_str());
        assert_eq!(pane_pid(&target), winner);
        // Back to dead, so the repair path itself is still exercised below.
        // (That last ensure re-installed the hook, as it is meant to.)
        tmux::run_tmux(&["set-hook", "-u", "-t", &n, "pane-died"]).expect("hook removed again");
        tmux::send_command(&target, "exit").expect("typed exit");
        assert!(settles(|| tmux::pane_live(&target) == Some(false)), "dead again, with no hook");
        let repaired = ensure().expect("ensure repairs a dead pane");
        assert_eq!(repaired["target"], target.as_str());
        assert_eq!(tmux::pane_live(&target), Some(true), "the target handed back is a LIVE pane");
        assert_ne!(pane_pid(&target), revived, "a new shell, in the same pane");
        assert_eq!(tmux::session_hook(&n, "pane-died").as_deref(), Some("respawn-pane"), "and the hook is back");

        // #326 review P1: the repair is TMUX's decision, not our
        // check-then-act. A respawn aimed at a pane whose shell is running —
        // which is what a stale `pane_live` reading, the pane-died hook or a
        // concurrent ensure would produce — is REFUSED, and the shell (and
        // anything it started) survives. Without this, the `-k` form would
        // have killed whatever won the race.
        let running = pane_pid(&target);
        tmux::send_command(&target, "sleep 120").expect("typed a command");
        assert!(settles(|| tmux::pane_format(&target, "#{pane_current_command}").as_deref() == Some("sleep")));
        let refused = tmux::respawn_pane(&target);
        assert!(refused.is_err(), "tmux refuses to respawn a pane that is still active: {refused:?}");
        assert_eq!(pane_pid(&target), running, "the shell is untouched");
        assert_eq!(tmux::pane_format(&target, "#{pane_current_command}").as_deref(), Some("sleep"),
            "and so is the command it was running");
        // Which is also why a repeated ensure through this path is a no-op.
        assert_eq!(ensure().unwrap()["target"], target.as_str());
        assert_eq!(pane_pid(&target), running, "ensure did not restart a live shell");
        tmux::send_keys(&target, "C-c", false).ok();

        // The target ensure ANSWERS with is read back from that same %id at
        // the end, so the client can only ever be handed the position of the
        // pane this call verified.
        let answered = ensure().unwrap()["target"].as_str().unwrap().to_string();
        let answered_id = tmux::pane_format(&answered, "#{pane_id}");
        assert_eq!(answered_id, tmux::pane_format(&target, "#{pane_id}"), "the answer names the pane we checked");
        assert_eq!(tmux::pane_live(&answered), Some(true), "and it is live");

        // The check and the repair name the pane by its %id, so a window
        // created beside ours (which renumbers positions) cannot make them
        // land on two different panes.
        let id = tmux::pane_format(&target, "#{pane_id}").expect("a pane id");
        assert!(id.starts_with('%'));
        assert_eq!(tmux::pane_live(&id), tmux::pane_live(&target), "both names, one pane");
        // The three liveness states tmux can report, each distinguished —
        // 'unreadable' is its own answer and never passes for 'live'.
        assert_eq!(tmux::pane_live("tmm-no-such-session-326:9.9"), None, "a pane that is not there is unknown, not live");
        assert_eq!(tmux::pane_live("%999999"), None, "and so is an id that is not there");

        // Explicit Kill is still the one way to end it: the hook is a session
        // option, so it dies with the session instead of rebuilding the pane.
        assert_eq!(kill().unwrap()["killed"], true);
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert!(!tmux::session_exists(&n), "killed, not respawned by its own hook");
        assert_eq!(owned().unwrap(), Ownership::Absent);

        assert_eq!(tmux::session_exists(SCRATCH_SESSION), real_before, "the real scratch session was never touched");
        assert_eq!(tmux::session_hook(SCRATCH_SESSION, "pane-died"), real_hook_before, "nor its hooks");
        assert_eq!(global_hook(), global_hook_before, "and no global hook was set");
    }

    /// Why the repair names the pane by `%id` and not by its position
    /// (#326 review): a POSITION is reused. Measured here on real tmux, so
    /// the rule's reason cannot quietly expire.
    #[test]
    fn a_pane_position_is_reused_but_an_id_is_not() {
        let mut guard = tmux::Scratch::new("id");
        let s = guard.session("i");
        if tmux::new_session(&s, None, None).is_err() {
            eprintln!("no tmux server — skipping");
            return;
        }
        let panes = tmux::list_panes(&s).unwrap();
        let first = panes.iter().min_by_key(|p| (p.window, p.pane)).unwrap();
        let target = format!("{}:{}.{}", s, first.window, first.pane);
        let held = tmux::pane_format(&target, "#{pane_id}").expect("an id");
        // A second pane, then the FIRST one dies: the survivor inherits the
        // index the first one had.
        tmux::run_tmux(&["split-window", "-d", "-t", &target]).expect("split");
        tmux::run_tmux(&["kill-pane", "-t", &held]).expect("kill the first pane");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let now = tmux::pane_format(&target, "#{pane_id}").expect("the position still resolves");
        assert_ne!(now, held, "the position was handed to another pane");
        // Which is the whole point: the id we read answers honestly that ITS
        // pane is gone (so ensure errors), while the stale position answers
        // for a pane we never examined — the one a positional repair would
        // have respawned.
        assert_eq!(tmux::pane_live(&held), None, "our pane is gone, and we can tell");
        assert_eq!(tmux::pane_live(&target), Some(true), "the position reads as a live stranger");
    }

    /// Hidden from the listings only while it is OURS (board #326).
    #[test]
    fn only_an_owned_scratch_session_is_hidden_from_the_listings() {
        crate::projects::tests::use_test_store();
        let mut guard = tmux::Scratch::new("hid");
        let n = guard.session("h");
        use_test_name(&n);
        if tmux::new_session(&guard.session("probe"), None, None).is_err() {
            eprintln!("no tmux server — skipping");
            return;
        }
        assert_eq!(hidden_session(), None, "nothing to hide while it does not exist");
        ensure().expect("created");
        assert_eq!(hidden_session().as_deref(), Some(n.as_str()));
        kill().unwrap();
        // A plain session the user made with that name is listed like any
        // other: the NAME alone never hides a row.
        tmux::new_session(&n, None, None).unwrap();
        assert!(matches!(owned().unwrap(), Ownership::Taken(_)));
        assert_eq!(hidden_session(), None, "a session that is not ours stays visible");
        tmux::kill_session(&n).unwrap();
        // And a name a project declares stays visible too.
        let p = crate::projects::create(&guard.path(), Some("held"), Some(&n), None).unwrap();
        assert_eq!(hidden_session(), None, "a project's session is a project, listed as one");
        crate::projects::delete(p["id"].as_str().unwrap()).ok();
    }

    /// Real tmux (test:rust runs with one), under a guard-owned name — the
    /// production `tmm-scratch` is never touched, so a run cannot end the
    /// user's own scratch shell. Create once, idempotent, a concrete pane
    /// target, kill only ours, a plain same-name session refused and left
    /// alone, a declared-but-down project still Taken, and every project
    /// path meeting it (auto-adopt, adopt, create, rename) keeps one owner.
    #[test]
    fn ensure_creates_once_marks_and_answers_a_pane_target() {
        crate::projects::tests::use_test_store();
        let real_before = tmux::session_exists(SCRATCH_SESSION);
        // Short: a project session name is slugged to 24 characters.
        let mut guard = tmux::Scratch::new("sc");
        let n = guard.session("s");
        use_test_name(&n);
        assert_ne!(name(), SCRATCH_SESSION);
        if tmux::new_session(&guard.session("probe"), None, None).is_err() {
            eprintln!("no tmux server — skipping");
            return;
        }
        let a = ensure().expect("created");
        let target = a["target"].as_str().unwrap().to_string();
        assert_eq!(a["session"], n.as_str());
        assert!(target.starts_with(&format!("{n}:")) && target.contains('.'), "a session:window.pane target: {target}");
        assert_eq!(owned().unwrap(), Ownership::Ours);
        assert!(is_scratch(&n));
        // Auto-adopt and a direct adopt both refuse it.
        let adopted = crate::projects::projects::auto_adopt_with(&[(n.clone(), 0)], 10_000).unwrap();
        assert!(adopted.is_empty(), "never auto-adopted: {adopted:?}");
        assert!(crate::projects::adopt(&n, None).is_err(), "never adopted by hand");
        assert!(crate::projects::project_for_session(&n).unwrap().is_none());
        // A project created with that session name while it is live is
        // suffixed away from it; a rename onto it is refused.
        let dir = guard.path();
        let made = crate::projects::create(&dir, Some("pad"), Some(&n), None).unwrap();
        assert_ne!(made["session"], n.as_str(), "create gives the new project another session name");
        assert!(crate::projects::rename(made["id"].as_str().unwrap(), &n).is_err(), "rename onto it is refused");
        assert_eq!(owned().unwrap(), Ownership::Ours, "still ours after both");
        crate::projects::delete(made["id"].as_str().unwrap()).ok();
        let b = ensure().expect("idempotent");
        assert_eq!(b["target"], a["target"]);
        assert_eq!(kill().unwrap()["killed"], true);
        assert_eq!(owned().unwrap(), Ownership::Absent);
        // A plain session the user made with that name.
        tmux::new_session(&n, None, None).unwrap();
        assert!(matches!(owned().unwrap(), Ownership::Taken(_)));
        assert!(ensure().is_err(), "never taken over");
        assert!(kill().is_err(), "never killed");
        assert!(tmux::session_exists(&n), "left alone");
        assert!(!is_scratch(&n), "so it is adoptable like any session");
        tmux::kill_session(&n).unwrap();
        // A PROJECT that declares the name holds it even while it is down.
        let p = crate::projects::create(&dir, Some("held"), Some(&n), None).unwrap();
        assert_eq!(p["session"], n.as_str());
        assert!(!tmux::session_exists(&n), "declared, not up");
        assert!(matches!(owned().unwrap(), Ownership::Taken(_)), "a down project still owns its name");
        assert!(ensure().is_err(), "ensure refuses instead of creating over a declaration");
        assert!(!tmux::session_exists(&n));
        crate::projects::delete(p["id"].as_str().unwrap()).ok();
        assert_eq!(tmux::session_exists(SCRATCH_SESSION), real_before, "the real scratch session was never touched");
    }
}
