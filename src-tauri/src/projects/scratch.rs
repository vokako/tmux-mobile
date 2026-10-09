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
//! no agent and installs no hooks; it is never relaunched by `project up`
//! because it is not a project. Sessions and Terminal list it like any tmux
//! session — we do not hide tmux.

use crate::tmux;

pub const SCRATCH_SESSION: &str = "tmm-scratch";
const MARK: &str = "@tmm-scratch";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ownership {
    Ours,
    Absent,
    Taken(String),
}

/// The pure verdict over the three facts (tested without tmux).
pub(crate) fn verdict(exists: bool, marked: bool, project: Option<&str>) -> Ownership {
    if let Some(name) = project {
        return Ownership::Taken(format!("the name '{SCRATCH_SESSION}' belongs to project '{name}'"));
    }
    match (exists, marked) {
        (false, _) => Ownership::Absent,
        (true, true) => Ownership::Ours,
        (true, false) => Ownership::Taken(format!("a tmux session named '{SCRATCH_SESSION}' already exists and is not the scratch terminal")),
    }
}

fn declared_by() -> Result<Option<String>, String> {
    Ok(super::project_for_session(SCRATCH_SESSION)?.map(|p| p.name))
}

/// THE predicate.
pub fn owned() -> Result<Ownership, String> {
    let exists = tmux::session_exists(SCRATCH_SESSION);
    let marked = exists && tmux::session_option(SCRATCH_SESSION, MARK).as_deref() == Some("1");
    Ok(verdict(exists, marked, declared_by()?.as_deref()))
}

/// True when `session` is our scratch session — the guard auto-adopt and
/// the other project paths share.
pub fn is_scratch(session: &str) -> bool {
    session == SCRATCH_SESSION && matches!(owned(), Ok(Ownership::Ours))
}

/// Ensure the scratch session exists and answer `{session, target}`: the
/// concrete `session:window.pane` of its first window's first pane (base
/// index honoured), the target the Terminal subscribes, types and resizes
/// against. A concurrent ensure that created it first is the only create
/// failure accepted, and only once `owned` says it is ours.
pub fn ensure() -> Result<serde_json::Value, String> {
    match owned()? {
        Ownership::Taken(why) => return Err(why),
        Ownership::Ours => {}
        Ownership::Absent => {
            if let Err(e) = tmux::new_session(SCRATCH_SESSION, None, None) {
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
                tmux::set_session_option(SCRATCH_SESSION, MARK, "1")?;
            }
        }
    }
    Ok(serde_json::json!({ "session": SCRATCH_SESSION, "target": first_pane()? }))
}

fn first_pane() -> Result<String, String> {
    let panes = tmux::list_panes(SCRATCH_SESSION)?;
    let first = panes
        .iter()
        .min_by_key(|p| (p.window, p.pane))
        .ok_or_else(|| "the scratch session has no pane".to_string())?;
    Ok(format!("{}:{}.{}", SCRATCH_SESSION, first.window, first.pane))
}

/// Kill the scratch session — only when it is ours.
pub fn kill() -> Result<serde_json::Value, String> {
    match owned()? {
        Ownership::Ours => {
            tmux::kill_session(SCRATCH_SESSION)?;
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

    /// Real tmux (test:rust runs with one): create once, idempotent, a
    /// concrete pane target, kill only ours, and a plain session of the same
    /// name is refused and left alone.
    #[test]
    fn ensure_creates_once_marks_and_answers_a_pane_target() {
        crate::projects::tests::use_test_store();
        let _ = tmux::kill_session(SCRATCH_SESSION);
        let a = ensure().expect("created");
        let target = a["target"].as_str().unwrap().to_string();
        assert!(target.starts_with("tmm-scratch:") && target.contains('.'), "a session:window.pane target: {target}");
        assert_eq!(owned().unwrap(), Ownership::Ours);
        assert!(is_scratch(SCRATCH_SESSION));
        // Auto-adopt (every other session becomes a project) skips it.
        let adopted = crate::projects::projects::auto_adopt_with(&[(SCRATCH_SESSION.to_string(), 0)], 10_000).unwrap();
        assert!(adopted.is_empty(), "the scratch session is never adopted: {adopted:?}");
        assert!(crate::projects::project_for_session(SCRATCH_SESSION).unwrap().is_none());
        let b = ensure().expect("idempotent");
        assert_eq!(b["target"], a["target"]);
        assert_eq!(kill().unwrap()["killed"], true);
        assert_eq!(owned().unwrap(), Ownership::Absent);
        // A plain session the user made with that name.
        tmux::new_session(SCRATCH_SESSION, None, None).unwrap();
        assert!(matches!(owned().unwrap(), Ownership::Taken(_)));
        assert!(ensure().is_err(), "never taken over");
        assert!(kill().is_err(), "never killed");
        assert!(tmux::session_exists(SCRATCH_SESSION), "left alone");
        assert!(!is_scratch(SCRATCH_SESSION), "so it is adoptable like any session");
        tmux::kill_session(SCRATCH_SESSION).unwrap();
        // A PROJECT that declares the name holds it even while it is down.
        let dir = std::env::temp_dir().join(format!("scratch-proj-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = crate::projects::create(dir.to_str().unwrap(), Some("scratch pad"), Some(SCRATCH_SESSION), None).unwrap();
        assert_eq!(p["session"], SCRATCH_SESSION);
        assert!(!tmux::session_exists(SCRATCH_SESSION), "declared, not up");
        assert!(matches!(owned().unwrap(), Ownership::Taken(_)), "a down project still owns its name");
        assert!(ensure().is_err(), "ensure refuses instead of creating over a declaration");
        assert!(!tmux::session_exists(SCRATCH_SESSION));
        crate::projects::delete(p["id"].as_str().unwrap()).ok();
    }
}
