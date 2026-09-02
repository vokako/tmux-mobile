//! Launching one agent into its tmux window: backend config dispatch, the
//! inline kick prompt, and startup-prompt auto-confirmation (permissions /
//! folder-trust dialogs). Split from team.rs 2026-07-22 — content unchanged.

use std::path::{Path, PathBuf};
use std::time::Duration;

use serde_json::Value;

/// Build the PATH used by managed agent launch scripts. The server is often
/// supervised with a deliberately small service PATH, while user-installed
/// CLIs (`claude`, `uvx`, cargo tools) live under the standard per-user bin
/// directories. A launch recipe must be self-sufficient: inheriting only the
/// server PATH made a configured Claude agent open a shell and fail with
/// `command not found: claude`.
fn agent_launch_path_from(home: Option<&Path>, prepend: Option<&Path>, base: &str) -> String {
    let mut parts: Vec<PathBuf> = Vec::new();
    if let Some(p) = prepend.filter(|p| !p.as_os_str().is_empty()) {
        parts.push(p.to_path_buf());
    }
    if let Some(home) = home {
        parts.push(home.join(".local/bin"));
        parts.push(home.join("bin"));
        parts.push(home.join(".cargo/bin"));
    }
    parts.extend(std::env::split_paths(base));
    let mut seen = std::collections::HashSet::new();
    parts.retain(|p| seen.insert(p.as_os_str().to_os_string()));
    std::env::join_paths(parts)
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned()
}

pub(crate) fn agent_launch_path(prepend: Option<&Path>, base: &str) -> String {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    agent_launch_path_from(home.as_deref(), prepend, base)
}

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

/// Write the full launch command to `<team home>/launch-<name>.sh`. The team
/// home is self-gitignored, and the script carries the same data as the
/// backend config files beside it (env values from team.yaml included), so
/// this adds no new exposure. Overwritten on every (re)launch.
pub(crate) fn write_launch_script(home: &std::path::Path, name: &str, full_cmd: &str) -> Result<std::path::PathBuf, String> {
    let path = home.join(format!("launch-{}.sh", name));
    std::fs::write(&path, format!("# tmux-mobile team launcher (regenerated on every launch)\n{}\n", full_cmd))
        .map_err(|e| format!("write {}: {}", path.display(), e))?;
    Ok(path)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct StartupConfirmation {
    pub(crate) markers: Vec<&'static str>,
    pub(crate) ready_markers: Vec<&'static str>,
    /// Named tmux keys used to accept the detected prompt. Claude 2.1.258
    /// defaults its folder-trust cursor to "No, exit", so Enter alone exits;
    /// Codex still defaults to the affirmative row.
    pub(crate) accept_keys: Vec<&'static str>,
    pub(crate) timeout: Duration,
}

/// What a backend `prepare_*` returns: (env vars, launch command, post-launch
/// confirmation). Aliased to keep the per-backend signatures readable.
pub(super) type Prepared = (Vec<(String, String)>, String, Option<StartupConfirmation>);

pub(crate) const CLAUDE_FOLDER_TRUST_MARKERS: &[&str] = &[
    "Accessing workspace:",
    "Yes, I trust this folder",
    "Enter to confirm",
];

pub(crate) const CODEX_FOLDER_TRUST_MARKERS: &[&str] = &[
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
pub(crate) fn confirm_startup_prompt(pane: String, confirmation: StartupConfirmation) {
    std::thread::spawn(move || {
        let deadline = std::time::Instant::now() + confirmation.timeout;
        while std::time::Instant::now() < deadline {
            if let Ok(content) = tmux::capture_pane_plain(&pane, Some(80)) {
                if startup_prompt_visible(&content, &confirmation) {
                    println!("🜂 team: confirming folder trust in new pane {}", pane);
                    for key in &confirmation.accept_keys {
                        let _ = tmux::send_keys(&pane, key, false);
                        std::thread::sleep(Duration::from_millis(100));
                    }
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
    fn managed_agent_path_includes_user_cli_bins_even_with_a_service_path() {
        let home = Path::new("/home/tester");
        let path = agent_launch_path_from(
            Some(home),
            Some(Path::new("/opt/tmm/bin")),
            "/usr/bin:/home/tester/.local/bin:/usr/bin",
        );
        let parts: Vec<_> = std::env::split_paths(&path).collect();
        assert_eq!(parts[0], PathBuf::from("/opt/tmm/bin"));
        assert_eq!(parts[1], PathBuf::from("/home/tester/.local/bin"));
        assert!(parts.contains(&PathBuf::from("/home/tester/bin")));
        assert!(parts.contains(&PathBuf::from("/home/tester/.cargo/bin")));
        assert_eq!(
            parts.iter().filter(|p| *p == &PathBuf::from("/usr/bin")).count(),
            1
        );
        assert_eq!(
            parts
                .iter()
                .filter(|p| *p == &PathBuf::from("/home/tester/.local/bin"))
                .count(),
            1
        );
    }

    #[test]
    fn launch_script_keeps_typed_line_short() {
        // The typed `. '<script>'` line must stay tiny no matter how large the
        // inline system prompt grows — kiro-cli-term swallows ≥2 KB bursts.
        let dir = std::env::temp_dir().join(format!("tmm-launch-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let huge_cmd = format!("TEAM_AGENT='x' claude --append-system-prompt '{}' 'kick'", "p".repeat(8000));
        let script = write_launch_script(&dir, "planner", &huge_cmd).unwrap();
        let written = std::fs::read_to_string(&script).unwrap();
        assert!(written.contains(&huge_cmd));
        assert!(script.file_name().unwrap().to_string_lossy() == "launch-planner.sh");
        let typed = format!(". {}", shell_quote(&script.to_string_lossy()));
        assert!(typed.len() < 200, "typed line must stay far below the ~2KB swallow threshold, got {}", typed.len());
        // Relaunch overwrites, not appends.
        let script2 = write_launch_script(&dir, "planner", "echo v2").unwrap();
        let w2 = std::fs::read_to_string(&script2).unwrap();
        assert!(w2.contains("echo v2") && !w2.contains("claude"));
        std::fs::remove_dir_all(&dir).ok();
    }

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
