//! In-process team supervisor (desktop only).
//!
//! This is the Rust port of the standalone `team/` Python launcher, moved
//! inside the app so the user never starts a separate process: the desktop
//! server itself seeds the team into the team bus and reconciles the desired
//! roster into real agent windows in tmux. The agent CLIs (kiro/claude/codex)
//! still run as their own processes in tmux panes — that is intrinsic, and it
//! is exactly what lets the Team tab preview an agent's live execution state.
//!
//! Flow (mirrors team's supervisor, but native):
//!   1. `seed_default_team` registers the built-in team (manager/worker/
//!      reviewer) as employees on the bus, if no team is present yet.
//!   2. a reconcile loop polls the bus's employee roster: a `requested`/`active`
//!      employee not yet launched gets its backend config written + a named
//!      tmux window opened; a `disabled` one has its window killed. The same
//!      path serves the initial team and any runtime `hire`/`fire`.
//!
//! The bus is reached through [`TeamBridge`] (JSON-only) so this module stays
//! decoupled from team's concrete types, like the rest of server.rs.

use crate::server::TeamBridge;
use crate::tmux;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;

mod templates;
use templates::team_dir;
mod workspace;
pub use workspace::{workspace_slug, team_slug, same_workspace, team_runtime_dir, read_system_prompt, save_system_prompt};
use workspace::{Paths, prepare_home};
mod skills;
use skills::resolve_skills;
mod backends;
#[cfg(test)]
mod test_util;
use backends::{prepare_kiro, prepare_claude, prepare_codex, shell_quote, McpDef, Extras};
mod reconcile;
pub use reconcile::nudge_adopted_agents;
use reconcile::reconcile_loop;
pub use templates::{
    ensure_templates_seeded, list_templates, read_team_def, read_template, read_all_templates,
    save_template, delete_template,
};

// Shared hooks, embedded so a packaged .app has no external file dependency.
// Written to the self-gitignored Team runtime directory at startup.
const KEEPALIVE_SH: &str = include_str!("../../../team/hooks/keepalive.sh");
const HEARTBEAT_SH: &str = include_str!("../../../team/hooks/heartbeat.sh");

/// Kiro 2.12 caps a configured MCP timeout at ten minutes. Keep every backend
/// on that shared boundary so the server can leave a full minute for delivery.
const TEAM_MCP_TOOL_TIMEOUT_MS: u64 = 600_000;
/// Server-level config the supervisor needs (bus URL + default model). The
/// per-run session + workspace are passed to `start`.
#[derive(Clone)]
pub struct TeamConfig {
    /// Bus URL the agents connect to over HTTP MCP (the in-process daemon).
    pub url: String,
    /// Default model for kiro-backed agents.
    pub model: String,
    /// User-editable rules shared by every team, refreshed at team start.
    pub system_prompt: String,
    /// Shared collaboration rules prepended to every agent's inline prompt.
    pub team_rules: String,
    /// The kick message that connects an agent to the bus loop.
    pub team_kick: String,
}

/// Start the team for `workspace`: seed the selected roster and spawn the
/// reconcile loop, launching agents into a per-Team tmux session. The
/// agents' working directory is `workspace` (the user's project); runtime hooks
/// live under the Team's self-gitignored runtime home, and prompts are passed inline.
/// Best-effort — any failure is logged, never fatal.
pub fn start(bridge: Arc<dyn TeamBridge>, mut cfg: TeamConfig, room: String, workspace: String, template: String) {
    cfg.system_prompt = read_system_prompt();
    tokio::spawn(async move {
        let session = format!("tmm-team-{}", room);
        let paths = Paths::new(&workspace, &room);
        let tpl = if template.trim().is_empty() { "default".to_string() } else { template };
        if let Err(e) = prepare_home(&paths) {
            eprintln!("⚠️  team: failed to prepare config home: {}", e);
            return;
        }
        seed_template(&*bridge, &room, &tpl, &cfg);
        println!("🜂 team: room={} workspace={} template={} session={}", room, workspace, tpl, session);
        reconcile_loop(bridge, cfg, room, session, paths).await;
    });
}

/// Seed the chosen `template`'s roster as employees, unless the room already has
/// a team (so a restart doesn't duplicate-seed; seed_employee also rejects taken
/// names). Each agent's `spec` is the template entry; an empty `model` on a kiro
/// agent falls back to the server default.
fn seed_template(bridge: &dyn TeamBridge, room: &str, template: &str, cfg: &TeamConfig) {
    let existing = bridge.employee_specs(room);
    if !existing.is_empty() {
        return; // already seeded (this run or a recovered one)
    }
    let def = read_team_def(template);
    let agents = def.get("agents").and_then(|a| a.as_array()).cloned().unwrap_or_default();
    if agents.is_empty() {
        eprintln!("⚠️  team: template '{}' empty/missing; nothing to seed", template);
        return;
    }
    // Team-wide config applies to every agent. env merges (agent overrides);
    // mcp/skills are prepended so per-agent entries come last (agent wins on a
    // same-named MCP server when the launcher builds the server map).
    let team_env = def.get("env").cloned().unwrap_or_else(|| serde_json::json!({}));
    let team_mcp = def.get("mcp").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let team_skills = def.get("skills").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let team_prompt = def.get("prompt").and_then(|v| v.as_str()).unwrap_or("");
    let mut names = Vec::new();
    for a in &agents {
        let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
        if name.is_empty() { continue; }
        let backend = a.get("backend").and_then(|v| v.as_str()).unwrap_or("kiro");
        // Empty model on a kiro agent → server default; other backends keep null.
        let model = a
            .get("model")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty());
        let model = match (backend, model) {
            (_, Some(m)) => Value::String(m.to_string()),
            ("kiro", None) => Value::String(cfg.model.clone()),
            _ => Value::Null,
        };
        let env = merge_env(&team_env, a.get("env"));
        let agent_mcp = a.get("mcp").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let agent_skills = a.get("skills").and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let spec = serde_json::json!({
            "role": a.get("role").and_then(|v| v.as_str()).unwrap_or(name),
            "goal": a.get("goal").and_then(|v| v.as_str()).unwrap_or(""),
            "backend": backend,
            "manage": a.get("manage").and_then(|v| v.as_bool()).unwrap_or(false),
            "model": model,
            "env": env,
            "mcp": merge_list(&team_mcp, &agent_mcp),
            "skills": merge_list(&team_skills, &agent_skills),
            // The team folder is where local (relative) skills resolve from.
            "team_dir": team_dir(template).to_string_lossy(),
            "team_prompt": team_prompt,
        });
        if let Err(e) = bridge.seed_employee(room, name, &spec) {
            eprintln!("⚠️  team: seed '{}' failed: {}", name, e);
        } else {
            names.push(name.to_string());
        }
    }
    println!("🜂 team: seeded '{}' ({}); launching…", template, names.join(" · "));
}

/// Concatenate a team-wide list with a per-agent list (team first), de-duping
/// string entries (skills) and keeping objects as-is (mcp — the launcher builds
/// a name-keyed map, so a later per-agent server overrides a same-named team one).
fn merge_list(team: &[Value], agent: &[Value]) -> Value {
    let mut out: Vec<Value> = Vec::new();
    for v in team.iter().chain(agent.iter()) {
        if let Some(s) = v.as_str() {
            if out.iter().any(|x| x.as_str() == Some(s)) { continue; }
        }
        out.push(v.clone());
    }
    Value::Array(out)
}

/// Merge a team-wide env object with a per-agent env object (agent wins) into a
/// flat JSON object. Either side may be absent/non-object.
fn merge_env(team_env: &Value, agent_env: Option<&Value>) -> Value {
    let mut out = serde_json::Map::new();
    if let Some(o) = team_env.as_object() {
        for (k, v) in o { out.insert(k.clone(), v.clone()); }
    }
    if let Some(o) = agent_env.and_then(|v| v.as_object()) {
        for (k, v) in o { out.insert(k.clone(), v.clone()); }
    }
    Value::Object(out)
}

/// Write the backend config for `name` and open a named tmux window running it.
/// Returns the new pane id. Blocking tmux/fs work runs on the caller (the
/// reconcile loop is its own task and the cadence is 3 s, so this is fine).
fn launch_agent(name: &str, spec: &Value, cfg: &TeamConfig, room: &str, session: &str, paths: &Paths) -> Result<String, String> {
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
    markers: Vec<&'static str>,
    ready_markers: Vec<&'static str>,
    timeout: Duration,
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

fn startup_prompt_visible(content: &str, confirmation: &StartupConfirmation) -> bool {
    prompt_markers_visible(content, &confirmation.markers)
}

fn folder_trust_prompt_visible(content: &str) -> bool {
    prompt_markers_visible(content, CLAUDE_FOLDER_TRUST_MARKERS)
        || prompt_markers_visible(content, CODEX_FOLDER_TRUST_MARKERS)
}

fn startup_already_ready(content: &str, confirmation: &StartupConfirmation) -> bool {
    confirmation
        .ready_markers
        .iter()
        .any(|marker| content.contains(marker))
}

/// Confirm a known first-use dialog without serializing the supervisor's launch
/// loop. No key is sent when the workspace is already trusted or the UI differs.
fn confirm_startup_prompt(pane: String, confirmation: StartupConfirmation) {
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
fn build_agent_prompt(role: &str, goal: &str, team_prompt: &str, cfg: &TeamConfig) -> String {
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
    use super::test_util::{cfg, RecordingBridge};
    use std::sync::Mutex;

    // ── SleepState ──────────────────────────────────────────────────────
    // The state machine that mediates "all idle long enough → Esc the team"
    // and "new message arrived → wake them". Pure: no tmux, no bus.

    #[test]
    fn merge_env_agent_overrides_team() {
        let team = serde_json::json!({ "A": "1", "B": "team" });
        let agent = serde_json::json!({ "B": "agent", "C": "3" });
        let m = merge_env(&team, Some(&agent));
        assert_eq!(m["A"], "1");
        assert_eq!(m["B"], "agent", "agent env wins");
        assert_eq!(m["C"], "3");
    }

    #[test]
    fn seed_template_skips_when_team_already_present() {
        let b = RecordingBridge {
            seeded: Mutex::new(vec![]),
            existing: vec![("manager".into(), Value::Null, "active".into())],
        };
        seed_template(&b, "myroom", "default", &cfg());
        assert!(b.seeded.lock().unwrap().is_empty(), "must not re-seed an existing team");
    }

    #[test]
    fn seed_template_normalizes_models_and_kiro_default() {
        let dir = std::env::temp_dir().join(format!("teamtest-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);

        let agents = serde_json::json!([
            { "name": "k", "backend": "kiro", "role": "k", "goal": "do things well", "model": "   ", "manage": false },
            { "name": "c", "backend": "claude", "role": "c", "goal": "do things well", "model": " sonnet ", "manage": false },
            { "name": "x", "backend": "codex", "role": "x", "goal": "do things well", "model": " gpt-test ", "manage": false }
        ]);
        save_template("blankmodel", &agents).unwrap();
        let b = RecordingBridge { seeded: Mutex::new(vec![]), existing: vec![] };
        seed_template(&b, "myroom", "blankmodel", &cfg());
        let seeded = b.seeded.lock().unwrap();
        let model = |name: &str| {
            seeded
                .iter()
                .find(|(agent, _)| agent == name)
                .map(|(_, spec)| spec["model"].clone())
                .unwrap()
        };
        assert_eq!(model("k"), "claude-sonnet-4.6");
        assert_eq!(model("c"), "sonnet");
        assert_eq!(model("x"), "gpt-test");

        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn team_wide_config_folds_into_each_agent() {
        let dir = std::env::temp_dir().join(format!("teamtest-tw-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);

        let def = serde_json::json!({
            "env": { "TEAM": "1" },
            "mcp": [{ "name": "context7", "url": "https://mcp.context7.com/mcp" }],
            "skills": ["./skills/shared"],
            "prompt": "Team charter for all.",
            "agents": [{
                "name": "w", "backend": "kiro", "role": "w", "goal": "do things well",
                "env": { "A": "2" }, "mcp": [{ "name": "pg", "command": "x" }]
            }],
        });
        save_template("tw", &def).unwrap();
        // prompt round-trips into the saved YAML.
        assert_eq!(read_team_def("tw")["prompt"], "Team charter for all.");

        let b = RecordingBridge { seeded: Mutex::new(vec![]), existing: vec![] };
        seed_template(&b, "room", "tw", &cfg());
        let seeded = b.seeded.lock().unwrap();
        let (_, s) = seeded.first().expect("seeded one agent");
        assert_eq!(s["env"]["TEAM"], "1", "team env reaches the agent");
        assert_eq!(s["env"]["A"], "2", "agent env preserved");
        let mcp_names: Vec<&str> = s["mcp"].as_array().unwrap().iter().filter_map(|m| m["name"].as_str()).collect();
        assert!(mcp_names.contains(&"context7"), "team mcp folded in: {mcp_names:?}");
        assert!(mcp_names.contains(&"pg"), "agent mcp kept: {mcp_names:?}");
        assert!(
            s["skills"].as_array().unwrap().iter().any(|x| x.as_str() == Some("./skills/shared")),
            "team skill folded in"
        );

        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }

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

