//! Launching one agent into its tmux window: backend config dispatch, the
//! inline kick prompt, and startup-prompt auto-confirmation (permissions /
//! folder-trust dialogs). Split from team.rs 2026-07-22 — content unchanged.

use std::path::Path;
use std::time::Duration;

use serde_json::Value;

use crate::tmux;

use super::backends::{prepare_claude, prepare_codex, prepare_kiro, Extras};
use crate::projects::backends::shared::{
    agent_launch_path, confirm_startup_prompt, shell_quote, write_launch_script, McpDef,
    StartupConfirmation,
};
use crate::projects::skills::resolve_skills;
use super::workspace::Paths;
use super::TeamConfig;

/// What a backend `prepare_*` returns: (env vars, launch command, post-launch
/// confirmation). Aliased to keep the per-backend signatures readable.
pub(super) type Prepared = (Vec<(String, String)>, String, Option<StartupConfirmation>);

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

    let (mut env, cmd, startup_confirmation) = match backend {
        "kiro" => prepare_kiro(name, role, goal, team_prompt, cfg, room, paths, model, &extras)?,
        "claude" => prepare_claude(name, role, goal, team_prompt, cfg, room, paths, model, &extras)?,
        "codex" => prepare_codex(name, role, goal, team_prompt, cfg, room, paths, model, &extras)?,
        other => return Err(format!("unknown backend: {}", other)),
    };
    let inherited_path = env
        .iter()
        .rev()
        .find(|(key, _)| key == "PATH")
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
    env.retain(|(key, _)| key != "PATH");
    let colocated = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    env.push((
        "PATH".into(),
        agent_launch_path(colocated.as_deref(), &inherited_path),
    ));

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
    // NEVER send the full launch line via send-keys: claude/codex inline the
    // multi-KB system prompt on the command line, and terminal-integration
    // shims that proxy the pane's tty (kiro-cli-term / figterm renames the
    // shell to `zsh (kiro-cli-term)`) silently SWALLOW input bursts ≳2 KB —
    // the window is left at a bare prompt with nothing in scrollback, the
    // supervisor "adopts" that empty window as a live agent, and the team
    // shows one working Kiro (short launch line) next to dead Claude/Codex.
    // Reproduced at exactly ≥2000 bytes on 2026-07-23; `zsh -f` (no user rc)
    // takes 6 KB fine. Writing the command to a script and sourcing it keeps
    // the typed line ~60 bytes regardless of prompt size, immune to any rc
    // shim and to tty canonical-mode limits (MAX_CANON 1024) during shell
    // startup races.
    let script = write_launch_script(&paths.home, name, &full)?;
    tmux::send_command(&pane, &format!(". {}", shell_quote(&script.to_string_lossy())))?;

    if let Some(confirmation) = startup_confirmation {
        confirm_startup_prompt(pane.clone(), confirmation);
    }
    Ok(pane)
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
            codex_profile: String::new(),
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
