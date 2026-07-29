//! tmux → declaration. The reverse direction of `reconcile`.
//!
//! Nobody hand-writes a project: we watch the sessions we already know about
//! and fold what we see back into the declaration. Two rules keep the
//! declaration honest (exec plan §3):
//!
//! * a window must survive `SETTLE_SECS` before it is worth restoring — the
//!   window you opened to grep something and closed again must not come back
//!   on every future `up`;
//! * a window that disappeared is dropped from the declaration, but the
//!   topology it belonged to stays in `snapshots`, so "give me back yesterday's
//!   layout" is still answerable.

use super::agents;
use super::store::{Slot, SlotKind};
use crate::tmux::{self, TmuxPane};

/// How long a window must exist before it enters the declaration.
pub const SETTLE_SECS: u64 = 120;
/// Topology snapshots kept per project.
pub const SNAPSHOT_KEEP: usize = 20;

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
    /// The window set / cwd / agent changed — worth a snapshot.
    pub topology_changed: bool,
    /// Something changed at all (including a slot merely settling), so the
    /// declaration must be written back.
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
    let mut topology_changed = false;
    let mut dirty = false;

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
                let moved_or_changed = prev.ord != ord
                    || prev.cwd != obs.cwd
                    || prev.kind != obs.kind
                    || prev.command != obs.command;
                let learned_session = prev.agent_session_id != agent_session_id;
                let settles_now = prev.settled_at.is_none() && now - prev.first_seen_at >= settle_secs;
                if moved_or_changed {
                    topology_changed = true;
                }
                if moved_or_changed || settles_now || learned_session {
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
                // A brand-new window is remembered but not yet restorable, so
                // it is not a topology change either — it becomes one when it
                // settles.
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
            dirty = true;
            // Losing a settled window changes the topology; losing one that
            // never settled is the throwaway window we promised to ignore.
            if prev.is_settled() {
                topology_changed = true;
            }
        }
    }

    Merge { slots, topology_changed, dirty }
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
    fn a_new_window_is_remembered_unsettled_and_is_not_a_topology_change() {
        let m = merge(&[], &[obs("shell", "")], 1_000, 120);
        assert_eq!(m.slots.len(), 1);
        assert_eq!(m.slots[0].first_seen_at, 1_000);
        assert!(!m.slots[0].is_settled(), "not restorable yet");
        assert!(m.dirty, "must be persisted so first_seen_at survives a restart");
        assert!(!m.topology_changed, "no snapshot for a window that may vanish");
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
        assert!(!late.topology_changed, "settling alone is not a new topology");
    }

    #[test]
    fn a_throwaway_window_leaves_no_trace_but_a_settled_one_is_a_change() {
        let unsettled = vec![slot("shell", 0, 1_000, None)];
        let gone = merge(&unsettled, &[], 1_200, 120);
        assert!(gone.slots.is_empty());
        assert!(gone.dirty);
        assert!(!gone.topology_changed, "the grep window must not spam snapshots");

        let settled = vec![slot("shell", 0, 1_000, Some(1_120))];
        let lost = merge(&settled, &[], 1_300, 120);
        assert!(lost.topology_changed, "losing a real window is history worth keeping");
    }

    #[test]
    fn reordering_and_moving_cwd_are_topology_changes() {
        let existing = vec![slot("a", 0, 1_000, Some(1_120)), slot("b", 1, 1_000, Some(1_120))];
        let same = merge(&existing, &[obs("a", ""), obs("b", "")], 1_400, 120);
        assert!(!same.dirty, "an unchanged capture writes nothing");

        let swapped = merge(&existing, &[obs("b", ""), obs("a", "")], 1_400, 120);
        assert!(swapped.topology_changed);
        assert_eq!(swapped.slots[0].window_name, "b");
        assert_eq!(swapped.slots[0].ord, 0);

        let moved = merge(&existing, &[obs("a", "src"), obs("b", "")], 1_400, 120);
        assert!(moved.topology_changed);
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
        assert!(m.topology_changed);
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
}
