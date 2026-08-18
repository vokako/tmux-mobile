//! Declaration → tmux. The reverse direction of `capture`.
//!
//! `up` is idempotent by construction: it matches windows BY NAME and only
//! creates what is missing. It never renames, reorders or restarts a window
//! that is already there, because the session it is reconciling may be the one
//! you are typing in right now.

use super::agents;
use super::store::{Project, Slot, SlotKind};
use crate::tmux;

#[derive(Debug, Clone, serde::Serialize)]
pub struct SlotResult {
    pub window_name: String,
    /// `created` | `existing` | `skipped` | `failed`
    pub status: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UpReport {
    pub session: String,
    pub created_session: bool,
    pub slots: Vec<SlotResult>,
}

/// Bring a project up: ensure its session exists and every settled slot has a
/// window. Unsettled slots are skipped — they have not earned a place in the
/// restored workspace yet (see `capture::SETTLE_SECS`).
pub fn up(project: &Project, slots: &[Slot]) -> Result<UpReport, String> {
    let existed = tmux::session_exists(&project.session);
    let mut results = Vec::new();
    let restorable: Vec<&Slot> = slots.iter().filter(|s| s.is_settled()).collect();

    if !existed {
        tmux::ensure_session(&project.session, &project.path)?;
        // A fresh session already owns one window, named after the shell. Give
        // it to the first slot instead of leaving a stray window behind.
        if let Some(first) = restorable.first() {
            let target = format!("{}:^", project.session);
            tmux::rename_window(&target, &first.window_name)?;
            results.push(start_in_existing(project, first, &target));
        }
    }

    for slot in restorable
        .iter()
        .skip(if existed { 0 } else { 1 })
        .copied()
    {
        results.push(create_or_keep(project, slot));
    }

    for slot in slots.iter().filter(|s| !s.is_settled()) {
        results.push(SlotResult {
            window_name: slot.window_name.clone(),
            status: "skipped",
            error: None,
        });
    }

    Ok(UpReport {
        session: project.session.clone(),
        created_session: !existed,
        slots: results,
    })
}

fn create_or_keep(project: &Project, slot: &Slot) -> SlotResult {
    if tmux::find_window_by_name(&project.session, &slot.window_name).is_some() {
        return SlotResult {
            window_name: slot.window_name.clone(),
            status: "existing",
            error: None,
        };
    }
            let cwd = absolute_cwd(&project.path, &slot.cwd);
    // A managed agent's hooks are re-materialized before it starts: the config
    // on disk was written by whatever version spawned it, and hooks are how we
    // observe the agent at all.
    if slot.kind == SlotKind::Agent {
        super::spawn::refresh_hooks(&project.path, &slot.window_name);
    }
    match tmux::new_named_window(&project.session, &slot.window_name, &cwd) {
        Ok(pane) => {
            let mut result = SlotResult {
                window_name: slot.window_name.clone(),
                status: "created",
                error: None,
            };
            if let Err(e) = run_slot_command(project, slot, &pane) {
                result.error = Some(e);
            }
            result
        }
        Err(e) => SlotResult {
            window_name: slot.window_name.clone(),
            status: "failed",
            error: Some(e),
        },
    }
}

fn start_in_existing(project: &Project, slot: &Slot, target: &str) -> SlotResult {
    let cwd = absolute_cwd(&project.path, &slot.cwd);
    let mut result = SlotResult {
        window_name: slot.window_name.clone(),
        status: "created",
        error: None,
    };
    if !slot.cwd.is_empty() {
        if let Err(e) = tmux::send_command(target, &format!("cd {}", shell_quote(&cwd))) {
            result.error = Some(e);
            return result;
        }
    }
    if let Err(e) = run_slot_command(project, slot, target) {
        result.error = Some(e);
    }
    result
}

/// Only agent slots and explicitly declared commands are replayed. Restoring a
/// workspace must not re-execute whatever the user happened to be running last
/// time (decision 5 in the exec plan). An agent goes back into the conversation
/// it was in, not to a blank prompt — see `agents::launch_line`.
pub(super) fn slot_command(slot: &Slot) -> Option<String> {
    slot_command_in(slot, None)
}

/// `project_path` enables the managed-agent path: a spawned agent restarts
/// with its FULL identity (isolated home env + --agent + resume) via the
/// launch recipe `spawn` persisted. Without it the generic backend launch
/// line runs the user-space config, whose kiro hooks never fire (measured) —
/// the restarted agent kept answering but went observably deaf: no tool rows,
/// no delivery receipts, every message "unconfirmed" (owner report 2026-08-18).
pub(super) fn slot_command_in(slot: &Slot, project_path: Option<&str>) -> Option<String> {
    if !slot.auto_run {
        return None;
    }
    let command = match slot.kind {
        SlotKind::Agent => {
            let recipe = project_path.and_then(|p| {
                super::spawn::relaunch_line(p, &slot.window_name, slot.agent_session_id.as_deref())
            });
            match recipe {
                Some(line) => line,
                None => slot
                    .command
                    .as_deref()
                    .and_then(|backend| agents::launch_line(backend, slot.agent_session_id.as_deref()))?,
            }
        }
        SlotKind::Shell => slot.command.clone()?,
    };
    (!command.trim().is_empty()).then_some(command)
}

fn run_slot_command(project: &Project, slot: &Slot, target: &str) -> Result<(), String> {
    match slot_command_in(slot, Some(&project.path)) {
        // Agents get their tmm identity: the project is the session, the agent
        // name is the window. Resumed agents keep the user's own config (no
        // isolated home — that is spawn's job), but tmm works in their window
        // and telemetry can attribute their declarations.
        Some(cmd) => {
            let prefixed = if slot.kind == SlotKind::Agent {
                format!(
                    "TMM_PROJECT={} TMM_AGENT={} {}",
                    shell_quote(&project.session),
                    shell_quote(&slot.window_name),
                    cmd
                )
            } else {
                cmd
            };
            tmux::send_command(target, &prefixed)
        }
        None => Ok(()),
    }
}

/// Take a project down: kill the session, keep the declaration.
pub fn down(project: &Project) -> Result<(), String> {
    if !tmux::session_exists(&project.session) {
        return Ok(());
    }
    tmux::kill_session(&project.session)
}

pub fn absolute_cwd(project_path: &str, slot_cwd: &str) -> String {
    if slot_cwd.is_empty() {
        return project_path.to_string();
    }
    if slot_cwd.starts_with('/') {
        return slot_cwd.to_string();
    }
    format!("{}/{}", project_path.trim_end_matches('/'), slot_cwd)
}

fn shell_quote(s: &str) -> String {
    if !s.is_empty()
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/'))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slot(name: &str, cwd: &str, settled: bool) -> Slot {
        Slot {
            id: None,
            ord: 0,
            window_name: name.into(),
            cwd: cwd.into(),
            kind: SlotKind::Shell,
            command: None,
            auto_run: false,
            agent_session_id: None,
            first_seen_at: 1_000,
            settled_at: settled.then_some(1_200),
        }
    }

    fn project(name: &str, path: &str) -> Project {
        Project {
            id: name.into(),
            name: name.into(),
            path: path.into(),
            icon: None,
            session: name.into(),
            adopted: false,
            autostart: false,
            created_at: 1_000,
            last_up_at: None,
            last_seen_at: None,
            archived: false,
        }
    }

    #[test]
    fn slot_cwd_resolves_against_the_project_root() {
        assert_eq!(absolute_cwd("/w/app", ""), "/w/app");
        assert_eq!(absolute_cwd("/w/app", "src"), "/w/app/src");
        assert_eq!(absolute_cwd("/w/app/", "src"), "/w/app/src");
        assert_eq!(absolute_cwd("/w/app", "/other"), "/other");
    }

    #[test]
    fn quoting_survives_spaces_and_quotes() {
        assert_eq!(shell_quote("/w/app"), "/w/app");
        assert_eq!(shell_quote("/w/my app"), "'/w/my app'");
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn a_restored_agent_window_resumes_its_conversation() {
        let mut agent = slot("kiro", "", true);
        agent.kind = SlotKind::Agent;
        agent.command = Some("kiro".into());
        agent.auto_run = true;

        // No id learned yet: resume this directory's newest conversation.
        assert_eq!(slot_command(&agent).as_deref(), Some("kiro-cli chat --resume"));

        // With the id the hooks reported, go back to that exact one — a
        // directory can hold several conversations (real case: two kiro
        // sessions in this repo), so "most recent" is a guess and the id is not.
        agent.agent_session_id = Some("aa816dcf-6615-4e41-86bb-1c9ec1c2c6a1".into());
        assert_eq!(
            slot_command(&agent).as_deref(),
            Some("kiro-cli chat --resume-id aa816dcf-6615-4e41-86bb-1c9ec1c2c6a1")
        );
    }

    #[test]
    fn an_observed_shell_process_is_never_replayed() {
        let mut shell = slot("dev", "", true);
        shell.command = Some("npm run dev".into());
        assert_eq!(slot_command(&shell), None, "auto_run is off for shells");

        // Even a stale conversation id cannot make a shell slot run anything.
        shell.agent_session_id = Some("conv-1".into());
        assert_eq!(slot_command(&shell), None);
    }

    #[test]
    fn up_and_down_reconcile_a_real_session_idempotently() {
        let path = std::env::temp_dir().join("tmm-proj-recon");
        std::fs::create_dir_all(&path).unwrap();
        let p = project("tmm-test-recon", path.to_str().unwrap());
        let _ = tmux::kill_session(&p.session);

        let slots = vec![
            slot("editor", "", true),
            slot("logs", "sub", true),
            slot("scratch", "", false),
        ];
        std::fs::create_dir_all(path.join("sub")).unwrap();

        let first = up(&p, &slots).unwrap();
        assert!(first.created_session);
        let created: Vec<_> = first.slots.iter().filter(|s| s.status == "created").collect();
        assert_eq!(created.len(), 2, "both settled slots got a window");
        assert!(
            first.slots.iter().any(|s| s.window_name == "scratch" && s.status == "skipped"),
            "an unsettled slot is not restored"
        );
        let windows: Vec<String> = tmux::list_named_windows(&p.session)
            .into_iter()
            .map(|(n, _)| n)
            .collect();
        assert_eq!(windows.len(), 2, "no stray shell window left over: {windows:?}");
        assert!(windows.contains(&"editor".to_string()));
        assert!(windows.contains(&"logs".to_string()));

        let second = up(&p, &slots).unwrap();
        assert!(!second.created_session);
        assert!(
            second.slots.iter().filter(|s| s.status == "existing").count() == 2,
            "second up must change nothing: {:?}",
            second.slots
        );
        assert_eq!(tmux::list_named_windows(&p.session).len(), 2);

        down(&p).unwrap();
        assert!(!tmux::session_exists(&p.session));
        down(&p).unwrap_or_else(|e| panic!("down must be idempotent: {e}"));
        let _ = std::fs::remove_dir_all(&path);
    }
}
