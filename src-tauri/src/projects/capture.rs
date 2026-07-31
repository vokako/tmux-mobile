//! tmux → declaration. The reverse direction of `reconcile`.
//!
//! Nobody hand-writes a project: we watch the sessions we already know about
//! and fold what we see back into the declaration, which is therefore always
//! the LAST OBSERVED state — that is what makes "close it and reopen it later"
//! and "survive a reboot" work, with no history to keep.
//!
//! One rule keeps the declaration honest (exec plan §3): a window must survive
//! `SETTLE_SECS` before it is worth restoring, so the window you opened to grep
//! something and closed again does not come back on every future `up`. A window
//! that disappears simply leaves the declaration.

use super::agents;
use super::store::{Slot, SlotKind};
use crate::tmux::{self, TmuxPane};

/// How long a window must exist before it enters the declaration.
pub const SETTLE_SECS: u64 = 120;

/// What a live window looks like right now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Observed {
    /// tmux window index, used to ask the notification hub which conversation
    /// this window's agent is in.
    pub window: usize,
    pub window_name: String,
    /// Relative to the project path; empty means the project root.
    pub cwd: String,
    pub kind: SlotKind,
    pub command: Option<String>,
    /// The agent's own conversation id, when a hook has reported one.
    pub agent_session_id: Option<String>,
}

/// Where the agent conversation ids come from. Implemented by
/// `AgentNotificationHub` (the hooks already carry `session_id`); the tests
/// pass a stub. A trait keeps `projects` from naming the notification types.
pub trait AgentSessions {
    fn agent_session_for(&self, session: &str, window: usize) -> Option<String>;
}

/// No hub available (tests, or a capture that does not care).
pub struct NoSessions;
impl AgentSessions for NoSessions {
    fn agent_session_for(&self, _session: &str, _window: usize) -> Option<String> {
        None
    }
}

/// Result of folding observation into the declaration.
pub struct Merge {
    pub slots: Vec<Slot>,
    /// Something changed, so the declaration must be written back.
    pub dirty: bool,
}

/// Read the live windows of `session` as project-relative observations.
///
/// One entry per window, taken from its ACTIVE pane: a window is a workspace
/// slot, and the active pane is what the user considers "that window".
pub fn observe(session: &str, project_path: &str, sessions: &dyn AgentSessions) -> Result<Vec<Observed>, String> {
    let panes = tmux::list_panes(session)?;
    let mut out: Vec<(usize, Observed)> = Vec::new();
    for pane in &panes {
        if !pane.active && panes.iter().any(|p| p.window == pane.window && p.active) {
            continue;
        }
        if out.iter().any(|(w, _)| *w == pane.window) {
            continue;
        }
        let mut observed = observed_from(pane, project_path);
        if observed.kind == SlotKind::Agent {
            observed.agent_session_id = sessions.agent_session_for(session, pane.window);
        }
        out.push((pane.window, observed));
    }
    out.sort_by_key(|(w, _)| *w);
    Ok(out.into_iter().map(|(_, o)| o).collect())
}

fn observed_from(pane: &TmuxPane, project_path: &str) -> Observed {
    // Shallow → deep, matching the client's detector: an early match is what
    // the user launched, a late one is a subprocess.
    let text = format!("{} {} {}", pane.current_command, pane.pane_title, pane.child_cmd);
    match agents::detect(&text) {
        Some(agent) => Observed {
            window: pane.window,
            window_name: pane.window_name.clone(),
            cwd: relative_cwd(&pane.current_path, project_path),
            kind: SlotKind::Agent,
            command: Some(agent.backend.to_string()),
            agent_session_id: None,
        },
        None => Observed {
            window: pane.window,
            window_name: pane.window_name.clone(),
            cwd: relative_cwd(&pane.current_path, project_path),
            kind: SlotKind::Shell,
            // Observed only — never replayed by `up` (decision 5).
            command: (!pane.child_cmd.is_empty()).then(|| pane.child_cmd.clone()),
            agent_session_id: None,
        },
    }
}

/// `cwd` expressed relative to the project path, so moving the workspace keeps
/// the declaration valid. Paths outside the project stay absolute.
pub fn relative_cwd(cwd: &str, project_path: &str) -> String {
    if cwd.is_empty() || cwd == project_path {
        return String::new();
    }
    let prefix = format!("{}/", project_path.trim_end_matches('/'));
    match cwd.strip_prefix(&prefix) {
        Some(rest) => rest.to_string(),
        None => cwd.to_string(),
    }
}

/// Fold `observed` into `existing`, applying the settle and removal rules.
///
/// Pure on purpose: every rule that decides what gets remembered is testable
/// without a tmux server.
pub fn merge(existing: &[Slot], observed: &[Observed], now: u64, settle_secs: u64) -> Merge {
    let mut slots = Vec::with_capacity(observed.len());
    let mut dirty = false;

    // A window's identity in the declaration IS its name (`up` matches by
    // name), so two live windows with the same name are indistinguishable —
    // and writing both would violate UNIQUE(project_id, window_name). Keep the
    // first observation of each name; the projection honestly recreates ONE
    // window of that name.
    let mut seen_names = std::collections::HashSet::new();
    let observed: Vec<&Observed> = observed
        .iter()
        .filter(|o| seen_names.insert(o.window_name.clone()))
        .collect();

    for (ord, obs) in observed.iter().enumerate() {
        let ord = ord as i64;
        match existing.iter().find(|s| s.window_name == obs.window_name) {
            Some(prev) => {
                // Sticky: a hook only reports the conversation id now and then,
                // so an observation without one must not erase what we know.
                let agent_session_id = obs
                    .agent_session_id
                    .clone()
                    .or_else(|| prev.agent_session_id.clone())
                    .filter(|_| obs.kind == SlotKind::Agent);
                let changed = prev.ord != ord
                    || prev.cwd != obs.cwd
                    || prev.kind != obs.kind
                    || prev.command != obs.command
                    || prev.agent_session_id != agent_session_id;
                let settles_now = prev.settled_at.is_none() && now - prev.first_seen_at >= settle_secs;
                if changed || settles_now {
                    dirty = true;
                }
                slots.push(Slot {
                    id: prev.id,
                    ord,
                    window_name: obs.window_name.clone(),
                    cwd: obs.cwd.clone(),
                    kind: obs.kind,
                    command: obs.command.clone(),
                    auto_run: obs.kind == SlotKind::Agent,
                    agent_session_id,
                    first_seen_at: prev.first_seen_at,
                    settled_at: prev.settled_at.or(settles_now.then_some(now)),
                });
            }
            None => {
                dirty = true;
                // A brand-new window is remembered so its age survives a server
                // restart, but it is not restorable until it settles.
                slots.push(Slot {
                    id: None,
                    ord,
                    window_name: obs.window_name.clone(),
                    cwd: obs.cwd.clone(),
                    kind: obs.kind,
                    command: obs.command.clone(),
                    auto_run: obs.kind == SlotKind::Agent,
                    agent_session_id: obs.agent_session_id.clone(),
                    first_seen_at: now,
                    settled_at: None,
                });
            }
        }
    }

    for prev in existing {
        if !observed.iter().any(|o| o.window_name == prev.window_name) {
            // A window that is gone simply leaves the declaration: it is no
            // longer part of the workspace, so `up` must not recreate it.
            dirty = true;
        }
    }

    Merge { slots, dirty }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn obs(name: &str, cwd: &str) -> Observed {
        Observed {
            window: 0,
            window_name: name.into(),
            cwd: cwd.into(),
            kind: SlotKind::Shell,
            command: None,
            agent_session_id: None,
        }
    }

    fn slot(name: &str, ord: i64, first_seen: u64, settled: Option<u64>) -> Slot {
        Slot {
            id: Some(ord + 1),
            ord,
            window_name: name.into(),
            cwd: String::new(),
            kind: SlotKind::Shell,
            command: None,
            auto_run: false,
            agent_session_id: None,
            first_seen_at: first_seen,
            settled_at: settled,
        }
    }

    fn agent_obs(name: &str, id: Option<&str>) -> Observed {
        Observed {
            window: 1,
            window_name: name.into(),
            cwd: String::new(),
            kind: SlotKind::Agent,
            command: Some("kiro".into()),
            agent_session_id: id.map(str::to_string),
        }
    }

    #[test]
    fn a_learned_conversation_id_is_recorded_and_then_kept() {
        let first = merge(&[], &[agent_obs("kiro", Some("conv-1"))], 1_000, 120);
        assert_eq!(first.slots[0].agent_session_id.as_deref(), Some("conv-1"));
        assert!(first.dirty);

        // Hooks only report now and then: a quiet cycle must not erase it.
        let quiet = merge(&first.slots, &[agent_obs("kiro", None)], 1_200, 120);
        assert_eq!(quiet.slots[0].agent_session_id.as_deref(), Some("conv-1"));

        // A different conversation in the same window replaces it.
        let moved_on = merge(&quiet.slots, &[agent_obs("kiro", Some("conv-2"))], 1_400, 120);
        assert_eq!(moved_on.slots[0].agent_session_id.as_deref(), Some("conv-2"));
        assert!(moved_on.dirty, "the new id has to reach the database");

        // The window stopped being an agent: the stale conversation goes.
        let now_shell = merge(&moved_on.slots, &[obs("kiro", "")], 1_600, 120);
        assert_eq!(now_shell.slots[0].agent_session_id, None);
    }

    #[test]
    fn a_new_window_is_remembered_but_not_restorable_yet() {
        let m = merge(&[], &[obs("shell", "")], 1_000, 120);
        assert_eq!(m.slots.len(), 1);
        assert_eq!(m.slots[0].first_seen_at, 1_000);
        assert!(!m.slots[0].is_settled(), "not restorable yet");
        assert!(m.dirty, "must be persisted so first_seen_at survives a restart");
    }

    #[test]
    fn a_window_settles_once_it_has_survived_the_threshold() {
        let existing = vec![slot("shell", 0, 1_000, None)];
        let early = merge(&existing, &[obs("shell", "")], 1_100, 120);
        assert!(!early.slots[0].is_settled());
        assert!(!early.dirty, "nothing to write yet");

        let late = merge(&existing, &[obs("shell", "")], 1_120, 120);
        assert_eq!(late.slots[0].settled_at, Some(1_120));
        assert!(late.dirty);
    }

    #[test]
    fn a_window_that_is_gone_leaves_the_declaration() {
        for existing in [
            vec![slot("shell", 0, 1_000, None)],          // never settled
            vec![slot("shell", 0, 1_000, Some(1_120))],   // was restorable
        ] {
            let gone = merge(&existing, &[], 1_300, 120);
            assert!(gone.slots.is_empty(), "up must not recreate a window you closed");
            assert!(gone.dirty);
        }
    }

    #[test]
    fn reordering_and_moving_cwd_are_written_back() {
        let existing = vec![slot("a", 0, 1_000, Some(1_120)), slot("b", 1, 1_000, Some(1_120))];
        let same = merge(&existing, &[obs("a", ""), obs("b", "")], 1_400, 120);
        assert!(!same.dirty, "an unchanged capture writes nothing");

        let swapped = merge(&existing, &[obs("b", ""), obs("a", "")], 1_400, 120);
        assert!(swapped.dirty);
        assert_eq!(swapped.slots[0].window_name, "b");
        assert_eq!(swapped.slots[0].ord, 0);

        let moved = merge(&existing, &[obs("a", "src"), obs("b", "")], 1_400, 120);
        assert!(moved.dirty);
        assert_eq!(moved.slots[0].cwd, "src");
    }

    #[test]
    fn becoming_an_agent_window_flips_auto_run_and_keeps_first_seen() {
        let existing = vec![slot("work", 0, 1_000, Some(1_120))];
        let observed = vec![Observed {
            window: 0,
            window_name: "work".into(),
            cwd: String::new(),
            kind: SlotKind::Agent,
            command: Some("kiro".into()),
            agent_session_id: None,
        }];
        let m = merge(&existing, &observed, 1_500, 120);
        assert!(m.dirty);
        assert_eq!(m.slots[0].kind, SlotKind::Agent);
        assert!(m.slots[0].auto_run, "agents are relaunched, shells are not");
        assert_eq!(m.slots[0].first_seen_at, 1_000, "age is not reset by a change");
        assert_eq!(m.slots[0].settled_at, Some(1_120));
    }

    #[test]
    fn cwd_is_stored_relative_to_the_project() {
        assert_eq!(relative_cwd("/w/app", "/w/app"), "");
        assert_eq!(relative_cwd("/w/app/src/api", "/w/app"), "src/api");
        assert_eq!(relative_cwd("/w/app/src", "/w/app/"), "src");
        assert_eq!(relative_cwd("/elsewhere", "/w/app"), "/elsewhere");
        assert_eq!(relative_cwd("", "/w/app"), "");
    }

    #[test]
    fn duplicate_window_names_collapse_to_one_slot() {
        // Two live windows named "zsh" are indistinguishable to a by-name
        // declaration; writing both violated UNIQUE(project_id, window_name)
        // and made every capture tick fail for the whole project.
        let observed = vec![obs("zsh", ""), obs("zsh", "/elsewhere")];
        let m = merge(&[], &observed, 100, SETTLE_SECS);
        assert_eq!(m.slots.len(), 1);
        assert_eq!(m.slots[0].window_name, "zsh");
    }
}
