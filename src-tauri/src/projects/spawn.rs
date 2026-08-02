//! Spawn a registry agent into a project: materialize its ISOLATED home from
//! the registry definition, open a tmux window in the project's session, and
//! launch the backend CLI wired to `tmm`.
//!
//! Two agents-v2 principles land here (docs/exec-plans/agents-v2.md §1):
//!
//! - **Home isolation** (principle 5): everything the agent runs with —
//!   persona, skills, MCP servers, hooks — is rendered into
//!   `<workspace>/.tmm/agents/<name>/` and selected via the backend's home
//!   env var (`KIRO_HOME` / `CODEX_HOME`) or config flags (claude). The
//!   user's global CLI config never leaks in, so the same registry definition
//!   behaves identically in every project.
//! - **CLI-only substrate** (principle 2): NO team MCP server, NO heartbeat
//!   machinery. The agent talks through `tmm` (one paragraph in its system
//!   prompt) and we observe it through the notify/telemetry hooks that are
//!   part of the rendered home.
//!
//! The per-backend rendering deliberately mirrors `team/backends.rs` (same
//! file formats, same flags, same 2KB-launch-line lesson via
//! `write_launch_script`) minus the team plumbing. Team stays untouched and
//! becomes legacy in Phase C.

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::team::backends_shared as shared;
use crate::tmux;

use super::store::RegAgent;

/// Max agents per project for `spawn` — a lead that needs more than this is
/// fanning out instead of thinking (and each window burns real tokens).
pub const SPAWN_CAP: usize = 4;

pub struct SpawnRequest<'a> {
    pub session: &'a str,
    /// Registry definition name to spawn.
    pub agent: &'a str,
    /// Opening brief posted into the hub chat and injected into the prompt.
    pub brief: &'a str,
    /// Who asked (agent name from $TMM_AGENT, or empty = the human).
    pub by: &'a str,
}

/// Spawn `agent` into `session`. Returns `{ window_name, pane }`.
pub fn spawn(req: &SpawnRequest) -> Result<Value, String> {
    let def = super::registry_get(req.agent)?
        .ok_or_else(|| format!("no agent named '{}' in the registry", req.agent))?;

    // can_hire gate: when an AGENT asks, its own registry def must allow
    // hiring. A human caller (empty `by`) is always allowed.
    if !req.by.is_empty() {
        if let Some(caller) = super::registry_get(req.by)? {
            if !caller.can_hire {
                return Err(format!("agent '{}' is not allowed to spawn (can_hire=false)", req.by));
            }
        }
        // A caller not in the registry (adopted window) counts as the human
        // driving that window — allowed.
    }

    let project = super::project_for_session(req.session)?
        .ok_or_else(|| format!("no project for session '{}'", req.session))?;
    let workspace = project.path.clone();

    // The cap counts existing agent windows (any backend), not shells.
    let panes = tmux::list_panes(req.session).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let agent_windows = panes
        .iter()
        .filter(|p| seen.insert(p.window))
        .filter(|p| super::agents::detect(&format!("{} {} {}", p.current_command, p.pane_title, p.window_name)).is_some())
        .count();
    if agent_windows >= SPAWN_CAP {
        return Err(format!("project already has {agent_windows} agents (cap {SPAWN_CAP}) — finish or close one first"));
    }

    // Window name = agent name, uniquified if taken (lead, lead-2, …). The
    // window name is the agent's identity for telemetry and tmm status.
    let taken: std::collections::HashSet<&str> = panes.iter().map(|p| p.window_name.as_str()).collect();
    let window_name = if taken.contains(def.name.as_str()) {
        (2..10)
            .map(|i| format!("{}-{}", def.name, i))
            .find(|c| !taken.contains(c.as_str()))
            .ok_or("too many windows with this agent's name")?
    } else {
        def.name.clone()
    };

    let home = agent_home(&workspace, &window_name);
    std::fs::create_dir_all(&home).map_err(|e| format!("create agent home: {e}"))?;
    ensure_gitignore(&workspace);

    let system_prompt = build_prompt(&def, &window_name, req.session, req.brief);
    let skills = resolve_skill_refs(&def, &home);

    let prepared = match def.backend.as_str() {
        "kiro" => render_kiro(&def, &window_name, &home, &system_prompt, &skills)?,
        "claude" => render_claude(&def, &window_name, &home, &system_prompt, &skills)?,
        "codex" => render_codex(&def, &window_name, &home, &system_prompt, &skills)?,
        other => return Err(format!("unknown backend '{other}'")),
    };

    // Env every spawned agent gets: its identity for tmm.
    let mut env = prepared.env;
    env.push(("TMM_PROJECT".into(), req.session.to_string()));
    env.push(("TMM_AGENT".into(), window_name.clone()));
    // tmm sits next to the server binary; make sure the pane can find it.
    if let Some(dir) = tmm_dir() {
        env.push(("PATH".into(), format!("{}:{}", dir.display(), std::env::var("PATH").unwrap_or_default())));
    }

    tmux::ensure_session(req.session, &workspace)?;
    let pane = tmux::new_named_window(req.session, &window_name, &workspace)?;
    std::thread::sleep(std::time::Duration::from_millis(800));

    let prefix = env
        .iter()
        .map(|(k, v)| format!("{}={}", k, shared::shell_quote(v)))
        .collect::<Vec<_>>()
        .join(" ");
    let full = format!("{} {}", prefix, prepared.cmd);
    // NEVER send the full line via send-keys — see team/launch.rs: tty shims
    // swallow bursts ≳2KB. Source a script instead.
    let script = shared::write_launch_script(&home, &window_name, &full)?;
    tmux::send_command(&pane, &format!(". {}", shared::shell_quote(&script.to_string_lossy())))?;
    if let Some(confirmation) = prepared.confirmation {
        shared::confirm_startup_prompt(pane.clone(), confirmation);
    }

    Ok(json!({ "window_name": window_name, "pane": pane, "backend": def.backend }))
}

/// The initial user message: an agent CLI boots into an interactive prompt
/// and does nothing until spoken to — the brief in the system prompt is
/// context, this line is the starter pistol (same trick as team_kick).
const KICK: &str = "Start now: read your instructions and task brief, then begin working. When the task is complete, run `tmm done \"summary\"`.";

struct Rendered {
    env: Vec<(String, String)>,
    cmd: String,
    confirmation: Option<shared::StartupConfirmation>,
}

fn agent_home(workspace: &str, name: &str) -> PathBuf {
    Path::new(workspace).join(".tmm").join("agents").join(name)
}

/// `.tmm/` self-gitignores (same convention as team runtime homes).
fn ensure_gitignore(workspace: &str) {
    let dir = Path::new(workspace).join(".tmm");
    let gi = dir.join(".gitignore");
    if dir.is_dir() && !gi.exists() {
        let _ = std::fs::write(gi, "*\n");
    }
}

/// Persona + tmm usage + brief. The tmm paragraph is the ENTIRE integration —
/// that is the point of the CLI-only substrate.
fn build_prompt(def: &RegAgent, name: &str, session: &str, brief: &str) -> String {
    let mut s = String::new();
    if !def.system.trim().is_empty() {
        s += def.system.trim();
        s += "\n\n";
    }
    s += &format!(
        "You are agent \"{name}\" in project \"{session}\" (a tmux session managed by tmux-mobile).\n\
         Coordinate through the `tmm` CLI:\n\
         - `tmm send \"@name message\"` — talk in the project chat (@name to address someone, use @human for the operator)\n\
         - `tmm log --limit 30` — read recent chat; `tmm agent list` — who is here and their state\n\
         - `tmm status working|waiting|blocked \"note\"` — declare what you are doing when it changes\n\
         - `tmm done \"summary\"` — REQUIRED when you finish the briefed task\n\
         You can also manage the workspace itself when the task calls for it:\n\
         - `tmm spawn <registry-name> --brief \"...\"` — bring in a teammate (see `tmm registry list`)\n\
         - `tmm project create|up|down|archive` — set up or tear down whole projects\n\
         - `tmm registry save --name .. --backend .. --system \"..\"` — define NEW kinds of agents, then spawn them\n\
         Rules: report results through `tmm send`/`tmm done`, not just terminal output. \
         If tmm fails (server down), keep working — it is telemetry, never a blocker. \
         Run `tmm --help` for the full command list."
    );
    if !brief.trim().is_empty() {
        s += &format!("\n\nYour task, briefed by {}:\n{}", if brief.is_empty() { "the operator" } else { "your teammate" }, brief.trim());
    }
    s
}

/// Skills: each entry is either a central asset NAME (reg_skills → its ref)
/// or a raw ref (local dir / github url) — central names win, raw refs keep
/// working, nothing migrates.
fn resolve_skill_refs(def: &RegAgent, home: &Path) -> Vec<crate::team::skills::ResolvedSkill> {
    let entries: Vec<String> = serde_json::from_str(&def.skills).unwrap_or_default();
    let central: std::collections::HashMap<String, String> = super::with_registry_skills();
    let refs: Vec<String> = entries
        .into_iter()
        .map(|e| central.get(&e).cloned().unwrap_or(e))
        .collect();
    crate::team::skills::resolve_skills(&refs, &home.to_string_lossy())
}

/// MCP: an array entry that is a STRING names a central server (reg_mcp);
/// an inline object is used as-is.
fn mcp_defs(def: &RegAgent) -> Vec<shared::McpDef> {
    let entries: Vec<serde_json::Value> = serde_json::from_str(&def.mcp).unwrap_or_default();
    let central = super::with_registry_mcp();
    entries
        .into_iter()
        .filter_map(|e| match e {
            serde_json::Value::String(name) => {
                let def_json = central.get(&name)?;
                let mut parsed: shared::McpDef = serde_json::from_str(def_json).ok()?;
                if parsed.name.is_empty() {
                    parsed.name = name;
                }
                Some(parsed)
            }
            obj => serde_json::from_value(obj).ok(),
        })
        .collect()
}

fn render_kiro(
    def: &RegAgent, name: &str, home: &Path, system_prompt: &str,
    skills: &[crate::team::skills::ResolvedSkill],
) -> Result<Rendered, String> {
    std::fs::create_dir_all(home.join("agents")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(home.join("settings")).map_err(|e| e.to_string())?;
    std::fs::write(
        home.join("settings").join("cli.json"),
        serde_json::to_string_pretty(&json!({ "chat.disableTrustAllConfirmation": true })).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    notifications.ensure_helper()?;
    let notify = notifications.helper_command("kiro");

    let resources: Vec<String> = skills
        .iter()
        .map(|sk| format!("skill://{}/SKILL.md", sk.dir.to_string_lossy()))
        .collect();
    let mut mcp_servers = json!({});
    for m in &mcp_defs(def) {
        if !m.name.is_empty() {
            mcp_servers.as_object_mut().unwrap().insert(m.name.clone(), shared::kiro_mcp_value(m));
        }
    }
    let conf = json!({
        "name": name,
        "description": format!("{} (registry agent)", def.name),
        "prompt": system_prompt,
        "tools": ["*"],
        "allowedTools": ["*"],
        "resources": resources,
        "mcpServers": mcp_servers,
        "hooks": {
            // The notify helper feeds notifications AND telemetry (tool events
            // are recognized by hook_event_name and routed to telemetry only).
            "preToolUse": [ { "matcher": "*", "command": notify.clone() } ],
            "postToolUse": [ { "matcher": "*", "command": notify.clone() } ],
            "stop": [ { "command": notify } ]
        },
    });
    std::fs::write(home.join("agents").join(format!("{name}.json")), serde_json::to_string_pretty(&conf).unwrap())
        .map_err(|e| e.to_string())?;

    let model = if def.model.is_empty() { "claude-sonnet-4.6" } else { &def.model };
    Ok(Rendered {
        env: vec![("KIRO_HOME".into(), home.to_string_lossy().to_string())],
        cmd: format!(
            "command kiro-cli chat --agent {} --model {} --trust-all-tools {}",
            shared::shell_quote(name),
            shared::shell_quote(model),
            shared::shell_quote(KICK),
        ),
        confirmation: None,
    })
}

fn render_claude(
    def: &RegAgent, _name: &str, home: &Path, system_prompt: &str,
    skills: &[crate::team::skills::ResolvedSkill],
) -> Result<Rendered, String> {
    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    notifications.ensure_helper()?;
    let notify = notifications.helper_command("claude");

    let mut mcp_servers = json!({});
    for m in &mcp_defs(def) {
        if !m.name.is_empty() {
            mcp_servers.as_object_mut().unwrap().insert(m.name.clone(), shared::claude_mcp_value(m));
        }
    }
    let mcpfile = home.join("mcp.json");
    std::fs::write(&mcpfile, serde_json::to_string_pretty(&json!({ "mcpServers": mcp_servers })).unwrap())
        .map_err(|e| e.to_string())?;
    let settingsfile = home.join("settings.json");
    std::fs::write(
        &settingsfile,
        serde_json::to_string_pretty(&json!({
            "skipDangerousModePermissionPrompt": true,
            "hooks": {
                "PreToolUse":  [ { "matcher": "*", "hooks": [ { "type": "command", "command": notify.clone() } ] } ],
                "PostToolUse": [ { "matcher": "*", "hooks": [ { "type": "command", "command": notify.clone() } ] } ],
                "Notification": [ { "matcher": "permission_prompt|idle_prompt|agent_needs_input|agent_completed", "hooks": [ { "type": "command", "command": notify.clone() } ] } ],
                "Stop": [ { "hooks": [ { "type": "command", "command": notify.clone() } ] } ],
                "StopFailure": [ { "hooks": [ { "type": "command", "command": notify } ] } ]
            }
        }))
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;

    // Claude has no native skill mechanism — inject the compact index.
    let full_prompt = if skills.is_empty() {
        system_prompt.to_string()
    } else {
        format!("{}\n\n{}", system_prompt, crate::team::skills::skills_index_text(skills))
    };
    let model = if def.model.is_empty() { "sonnet" } else { &def.model };
    Ok(Rendered {
        env: Vec::new(),
        cmd: format!(
            "command claude --mcp-config {} --strict-mcp-config --settings {} --model {} --dangerously-skip-permissions --append-system-prompt {} {}",
            shared::shell_quote(&mcpfile.to_string_lossy()),
            shared::shell_quote(&settingsfile.to_string_lossy()),
            shared::shell_quote(model),
            shared::shell_quote(&full_prompt),
            shared::shell_quote(KICK),
        ),
        confirmation: Some(shared::StartupConfirmation {
            markers: shared::CLAUDE_FOLDER_TRUST_MARKERS.to_vec(),
            ready_markers: vec!["bypass permissions on"],
            timeout: std::time::Duration::from_secs(120),
        }),
    })
}

fn render_codex(
    def: &RegAgent, _name: &str, home: &Path, system_prompt: &str,
    skills: &[crate::team::skills::ResolvedSkill],
) -> Result<Rendered, String> {
    let codex_home = home.join("codex");
    std::fs::create_dir_all(&codex_home).map_err(|e| e.to_string())?;
    shared::inherit_codex_system_files(&codex_home)?;
    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    notifications.ensure_helper()?;
    let notify = notifications.helper_command("codex");

    let mut config_args: Vec<String> = Vec::new();
    for m in &mcp_defs(def) {
        if !m.name.is_empty() {
            config_args.extend(shared::codex_mcp_overrides(m));
        }
    }
    let full_prompt = if skills.is_empty() {
        system_prompt.to_string()
    } else {
        format!("{}\n\n{}", system_prompt, crate::team::skills::skills_index_text(skills))
    };
    config_args.push(shared::codex_config_override("developer_instructions", Value::String(full_prompt)));
    std::fs::write(
        codex_home.join("hooks.json"),
        serde_json::to_vec_pretty(&json!({
            "hooks": {
                "PreToolUse":  [ { "matcher": "*", "hooks": [ { "type": "command", "command": notify.clone() } ] } ],
                "PostToolUse": [ { "matcher": "*", "hooks": [ { "type": "command", "command": notify.clone() } ] } ],
                "PermissionRequest": [ { "hooks": [ { "type": "command", "command": notify.clone() } ] } ],
                "Stop": [ { "hooks": [ { "type": "command", "command": notify } ] } ]
            }
        }))
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;

    if !def.model.is_empty() {
        config_args.push(format!("--model {}", shared::shell_quote(&def.model)));
    }
    config_args.push("--dangerously-bypass-approvals-and-sandbox".into());
    config_args.push("--dangerously-bypass-hook-trust".into());
    config_args.push(shared::shell_quote(KICK));
    Ok(Rendered {
        env: vec![("CODEX_HOME".into(), codex_home.to_string_lossy().to_string())],
        cmd: format!("command codex {}", config_args.join(" ")),
        confirmation: Some(shared::StartupConfirmation {
            markers: shared::CODEX_FOLDER_TRUST_MARKERS.to_vec(),
            ready_markers: vec!["Starting MCP servers", "OpenAI Codex"],
            timeout: std::time::Duration::from_secs(120),
        }),
    })
}

fn tmm_dir() -> Option<PathBuf> {
    std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn def(backend: &str) -> RegAgent {
        // Central-asset resolution in mcp_defs/resolve_skill_refs touches the
        // process-global store — it must NEVER be the user's real state.db.
        super::super::tests::use_test_store();
        RegAgent {
            name: "tester".into(),
            backend: backend.into(),
            model: String::new(),
            system: "Persona text.".into(),
            skills: "[]".into(),
            mcp: r#"[{"name":"files","command":"mcp-files","args":["--root","/tmp"]}]"#.into(),
            can_hire: false,
        }
    }

    #[test]
    fn kiro_home_is_isolated_and_wired_to_tmm() {
        let dir = std::env::temp_dir().join(format!("tmm-spawn-kiro-{}", uuid::Uuid::new_v4()));
        let d = def("kiro");
        let r = render_kiro(&d, "tester", &dir, &build_prompt(&d, "tester", "proj", "fix the bug"), &[]).unwrap();
        assert!(r.env.iter().any(|(k, v)| k == "KIRO_HOME" && v.contains("tmm-spawn-kiro")), "home must be the isolated dir");
        let conf: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("agents/tester.json")).unwrap()).unwrap();
        let prompt = conf.get("prompt").and_then(|p| p.as_str()).unwrap();
        assert!(prompt.contains("tmm send"), "the tmm paragraph IS the integration");
        assert!(prompt.contains("fix the bug"), "brief must reach the prompt");
        assert!(conf.get("mcpServers").and_then(|m| m.get("files")).is_some(), "registry MCP def must materialize");
        // Tool hooks feed telemetry.
        assert!(conf.get("hooks").and_then(|h| h.get("preToolUse")).is_some());
        assert!(!r.cmd.contains("@team"), "no team plumbing in registry agents");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn claude_and_codex_render_without_team_plumbing() {
        for backend in ["claude", "codex"] {
            let dir = std::env::temp_dir().join(format!("tmm-spawn-{backend}-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let d = def(backend);
            let prompt = build_prompt(&d, "tester", "proj", "");
            let r = match backend {
                "claude" => render_claude(&d, "tester", &dir, &prompt, &[]).unwrap(),
                _ => render_codex(&d, "tester", &dir, &prompt, &[]).unwrap(),
            };
            assert!(!r.cmd.contains("x-room"), "no team room headers");
            assert!(r.cmd.contains("files") || dir.join("mcp.json").exists(), "registry MCP present");
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    #[test]
    fn central_mcp_and_skill_names_resolve_at_spawn() {
        super::super::tests::use_test_store();
        // Define central assets, then reference them by NAME from an agent.
        super::super::mcp_save(&serde_json::json!({
            "name": "central-files",
            "def": "{\"command\":\"mcp-files\",\"args\":[\"--root\",\"/tmp\"]}"
        })).unwrap();
        // Skills are imported (files copied into the managed store) — build
        // a real local source so the import path runs for real.
        let src = std::env::temp_dir().join(format!("tmm-central-skill-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(src.join("SKILL.md"), "---\nname: central-skill\n---\nbody").unwrap();
        super::super::skill_save(&serde_json::json!({
            "name": "central-skill",
            "source": src.to_string_lossy()
        })).unwrap();
        std::fs::remove_dir_all(&src).ok();
        let mut d = def("kiro");
        d.mcp = r#"["central-files", {"name":"inline","command":"inline-cmd"}]"#.into();
        let resolved = mcp_defs(&d);
        let names: Vec<&str> = resolved.iter().map(|m| m.name.as_str()).collect();
        assert_eq!(resolved.len(), 2, "name entry + inline entry both resolve; got {names:?}");
        assert!(resolved.iter().any(|m| m.name == "central-files" && m.command.as_deref() == Some("mcp-files")),
            "string entry resolves through reg_mcp");
        assert!(resolved.iter().any(|m| m.name == "inline" && m.command.as_deref() == Some("inline-cmd")),
            "inline object keeps working");
        // Unknown names drop silently rather than breaking the spawn.
        d.mcp = r#"["no-such-server"]"#.into();
        assert!(mcp_defs(&d).is_empty());
    }

    #[test]
    fn prompt_carries_identity_project_and_rules() {
        let d = def("kiro");
        let p = build_prompt(&d, "rev-2", "blog", "review the branch");
        assert!(p.starts_with("Persona text."));
        assert!(p.contains("agent \"rev-2\" in project \"blog\""));
        assert!(p.contains("tmm done"));
        assert!(p.contains("review the branch"));
    }
}
