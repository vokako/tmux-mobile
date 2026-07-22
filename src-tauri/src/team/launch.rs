//! Launching one agent into its tmux window: backend config dispatch, the
//! inline kick prompt, and startup-prompt auto-confirmation (permissions /
//! folder-trust dialogs). Split from team.rs 2026-07-22 — content unchanged.

use std::time::Duration;

use serde_json::Value;

use crate::tmux;

use super::backends::{prepare_claude, prepare_codex, prepare_kiro, shell_quote, Extras, McpDef};
use super::skills::resolve_skills;
use super::workspace::Paths;
use super::TeamConfig;

/// Write the backend config for `name` and open a named tmux window running it.
/// Returns the new pane id. Blocking tmux/fs work runs on the caller (the
/// reconcile loop is its own task and the cadence is 3 s, so this is fine).
pub(super) fn launch_agent(name: &str, spec: &Value, cfg: &TeamConfig, room: &str, session: &str, paths: &Paths) -> Result<String, String> {
    let backend = spec.get("backend").and_then(|v| v.as_str()).unwrap_or("kiro");
    let role = spec.get("role").and_then(|v| v.as_str()).unwrap_or(name);
    let goal = spec.get("goal").and_then(|v| v.as_str()).unwrap_or("");
    let model = spec
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let team_prompt = spec.get("team_prompt").and_then(|v| v.as_str()).unwrap_or("");

    // Per-agent extras (env / extra MCP servers / skills) from the team.yaml.
    let env: Vec<(String, String)> = spec
        .get("env")
        .and_then(|v| v.as_object())
        .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default();
    let mcp: Vec<McpDef> = spec
        .get("mcp")
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    let skill_refs: Vec<String> = spec
        .get("skills")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|s| s.as_str().map(String::from)).collect())
        .unwrap_or_default();
    let team_dir_ref = spec.get("team_dir").and_then(|v| v.as_str()).unwrap_or("");
    let skills = resolve_skills(&skill_refs, team_dir_ref);
    let extras = Extras { env, mcp, skills };

    let (env, cmd, startup_confirmation) = match backend {
        "kiro" => prepare_kiro(name, role, goal, team_prompt, cfg, room, paths, model, &extras)?,
        "claude" => prepare_claude(name, role, goal, team_prompt, cfg, room, paths, model, &extras)?,
        "codex" => prepare_codex(name, role, goal, team_prompt, cfg, room, paths, model, &extras)?,
        other => return Err(format!("unknown backend: {}", other)),
    };

    let ws = paths.workspace.to_string_lossy().to_string();
    tmux::ensure_session(session, &ws)?;
    let pane = tmux::new_named_window(session, name, &ws)?;

    // Give the new shell a beat to initialize before sending the launch line.
    std::thread::sleep(Duration::from_millis(800));
    let prefix = env
        .iter()
        .map(|(k, v)| format!("{}={}", k, shell_quote(v)))
        .collect::<Vec<_>>()
        .join(" ");
    let full = if prefix.is_empty() { cmd.clone() } else { format!("{} {}", prefix, cmd) };
    tmux::send_command(&pane, &full)?;

    if let Some(confirmation) = startup_confirmation {
        confirm_startup_prompt(pane.clone(), confirmation);
    }
    Ok(pane)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct StartupConfirmation {
    pub(super) markers: Vec<&'static str>,
    pub(super) ready_markers: Vec<&'static str>,
    pub(super) timeout: Duration,
}

/// What a backend `prepare_*` returns: (env vars, launch command, post-launch
/// confirmation). Aliased to keep the per-backend signatures readable.
pub(super) type Prepared = (Vec<(String, String)>, String, Option<StartupConfirmation>);

pub(super) const CLAUDE_FOLDER_TRUST_MARKERS: &[&str] = &[
    "Accessing workspace:",
    "Yes, I trust this folder",
    "Enter to confirm",
];

pub(super) const CODEX_FOLDER_TRUST_MARKERS: &[&str] = &[
    "Do you trust the contents of this directory?",
    "1. Yes, continue",
    "Press enter to continue",
];

fn prompt_markers_visible(content: &str, markers: &[&str]) -> bool {
    markers.iter().all(|marker| content.contains(marker))
}

pub(super) fn startup_prompt_visible(content: &str, confirmation: &StartupConfirmation) -> bool {
    prompt_markers_visible(content, &confirmation.markers)
}

pub(super) fn folder_trust_prompt_visible(content: &str) -> bool {
    prompt_markers_visible(content, CLAUDE_FOLDER_TRUST_MARKERS)
        || prompt_markers_visible(content, CODEX_FOLDER_TRUST_MARKERS)
}

pub(super) fn startup_already_ready(content: &str, confirmation: &StartupConfirmation) -> bool {
    confirmation
        .ready_markers
        .iter()
        .any(|marker| content.contains(marker))
}

/// Confirm a known first-use dialog without serializing the supervisor's launch
/// loop. No key is sent when the workspace is already trusted or the UI differs.
pub(super) fn confirm_startup_prompt(pane: String, confirmation: StartupConfirmation) {
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + confirmation.timeout;
        while std::time::Instant::now() < deadline {
            if let Ok(content) = tmux::capture_pane_plain(&pane, Some(80)) {
                if startup_prompt_visible(&content, &confirmation) {
                    println!("🜂 team: confirming folder trust in new pane {}", pane);
                    let _ = tmux::send_keys(&pane, "Enter", false);
                    return;
                }
                if startup_already_ready(&content, &confirmation) {
                    return;
                }
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

/// Build the complete agent system prompt with XML-structured layers.
/// - `<team-system-prompt>`: global rules (from config) + team-specific prompt
/// - `<role-system-prompt>`: this agent's role + goal
pub(super) fn build_agent_prompt(role: &str, goal: &str, team_prompt: &str, cfg: &TeamConfig) -> String {
    let mut team_section = String::new();
    if !cfg.system_prompt.trim().is_empty() {
        team_section.push_str(cfg.system_prompt.trim());
    }
    if !team_section.is_empty() {
        team_section.push_str("\n\n");
    }
    if !cfg.team_rules.trim().is_empty() {
        team_section.push_str(cfg.team_rules.trim());
    }
    if !team_prompt.trim().is_empty() {
        if !team_section.is_empty() { team_section.push_str("\n\n---\n\n"); }
        team_section.push_str(team_prompt.trim());
    }
    format!(
        "<team-system-prompt>\n{}\n</team-system-prompt>\n\n<role-system-prompt>\nYou are the {}.\nGoal: {}\n</role-system-prompt>",
        team_section, role, goal.trim()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_agent_prompt_structure() {
        let cfg = TeamConfig {
            url: String::new(), model: String::new(),
            system_prompt: "Global rule.".into(),
            team_rules: "Rule one.\nRule two.".into(),
            team_kick: "kick".into(),
        };
        let p = build_agent_prompt("architect", "Design the system.", "Blog style.", &cfg);
        assert!(p.contains("<team-system-prompt>"));
        assert!(p.contains("</team-system-prompt>"));
        assert!(p.contains("<role-system-prompt>"));
        assert!(p.starts_with("<team-system-prompt>\nGlobal rule."));
        assert!(p.contains("Rule one."));
        assert!(!p.contains("read_history"));
        assert!(!p.contains("Team runtime"));
        assert!(!p.contains("Unaddressed messages are context."));
        assert!(!p.contains(".tmm/team-history.jsonl"));
        assert!(p.contains("Blog style."));
        assert!(p.contains("architect"));
        assert!(p.contains("Design the system."));
    }
}
