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

    write_launch_recipe(&home, &def.backend, &env, &prepared.cmd);

    Ok(json!({ "window_name": window_name, "pane": pane, "backend": def.backend }))
}

/// Persist how this agent is STARTED, so a restart can replay the full
/// identity. Without it, the resume path fell back to the plain backend
/// launch line ("kiro-cli chat --resume-id …" — no KIRO_HOME, no --agent),
/// which runs the USER-SPACE config whose hooks never fire (measured,
/// kiro-cli 2.16.2): the restarted agent went observably deaf — no tool rows,
/// no auto-post, every delivery "unconfirmed" (owner report, 2026-08-18).
/// The kick is NOT part of the recipe: it belongs to the first launch only;
/// a restart resumes a conversation instead.
fn write_launch_recipe(home: &Path, backend: &str, env: &[(String, String)], cmd: &str) {
    // The kick is always the final argument and always shell-quoted (it
    // contains spaces); strip that whole quoted argument, not the last word.
    let t = cmd.trim_end();
    let cmd_sans_kick = if t.ends_with('\'') {
        t[..t.len() - 1].rfind(" '").map(|i| &t[..i]).unwrap_or(t)
    } else {
        t.rsplit_once(' ').map(|(head, _)| head).unwrap_or(t)
    };
    let recipe = json!({
        "backend": backend,
        "env": env.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "cmd": cmd_sans_kick,
    });
    let _ = std::fs::write(
        home.join("launch.json"),
        serde_json::to_string_pretty(&recipe).unwrap(),
    );
}

/// The full relaunch line for a managed agent: recipe env + identity command +
/// the backend's resume flag for the recorded conversation. `None` when this
/// window has no recipe (a hand-started window, or a pre-recipe spawn).
pub fn relaunch_line(project_path: &str, window_name: &str, session_id: Option<&str>) -> Option<String> {
    let home = agent_home(project_path, window_name);
    let recipe: Value = serde_json::from_str(&std::fs::read_to_string(home.join("launch.json")).ok()?).ok()?;
    let cmd = recipe.get("cmd")?.as_str()?.to_string();
    let backend = recipe.get("backend").and_then(|b| b.as_str()).unwrap_or("");
    let resume = session_id.filter(|s| !s.is_empty()).and_then(|id| match backend {
        "kiro" => Some(format!("--resume-id {}", shared::shell_quote(id))),
        "claude" => Some(format!("--resume {}", shared::shell_quote(id))),
        _ => None,
    });
    let env = recipe.get("env").and_then(|e| e.as_array()).map(|arr| {
        arr.iter()
            .filter_map(|kv| Some((kv.get(0)?.as_str()?, kv.get(1)?.as_str()?)))
            .map(|(k, v)| format!("{}={}", k, shared::shell_quote(v)))
            .collect::<Vec<_>>()
            .join(" ")
    }).unwrap_or_default();
    let mut line = String::new();
    if !env.is_empty() {
        line.push_str(&env);
        line.push(' ');
    }
    line.push_str(&cmd);
    if let Some(r) = resume {
        line.push(' ');
        line.push_str(&r);
    }
    Some(line)
}

/// The initial prompt: an agent CLI boots into an interactive prompt and does
/// nothing until spoken to, so SOMETHING has to arrive — but it must not be an
/// instruction. That channel is the operator's (it is echoed into the chat as a
/// prompt, and the owner reads it as "the human said this"), and standing
/// instructions belong in the agent's own definition where they are stated once
/// and never re-typed. So the kick is a MARKER, not a sentence: the system
/// prompt (`build_prompt`) tells the agent what a session-start marker means.
const KICK: &str = "(session start)";

/// The kick, stamped with local wall time. An agent's first prompt is the only
/// place it learns what "now" is: its system prompt cannot carry a date (that
/// prompt is reused every time the window is restored, so a baked-in date would
/// be a lie a few days later), and the CLI does not volunteer one. Every LATER
/// message carries its own stamp from `deliver_mentions`.
fn kick_now() -> String {
    format!("[{}] {KICK}", chrono::Local::now().format("%Y-%m-%d %H:%M"))
}

#[cfg(test)]
mod relaunch_tests {
    use super::*;

    #[test]
    fn relaunch_line_replays_env_identity_and_resume() {
        let ws = std::env::temp_dir().join(format!("tmm-relaunch-{}", uuid::Uuid::new_v4()));
        let home = agent_home(ws.to_str().unwrap(), "lead");
        std::fs::create_dir_all(&home).unwrap();
        write_launch_recipe(
            &home,
            "kiro",
            &[("KIRO_HOME".to_string(), home.to_string_lossy().to_string())],
            "command kiro-cli chat --agent lead --model m --trust-all-tools kick",
        );
        let line = relaunch_line(ws.to_str().unwrap(), "lead", Some("id-1")).unwrap();
        assert!(line.starts_with("KIRO_HOME="), "isolated home first: {line}");
        assert!(line.contains("--agent lead"), "identity: {line}");
        assert!(line.ends_with("--resume-id id-1"), "conversation resumes: {line}");
        assert!(!line.contains("kick"), "write_launch_recipe strips the kick: {line}");
        // No recorded conversation → plain identity relaunch, no resume flag.
        let fresh = relaunch_line(ws.to_str().unwrap(), "lead", None).unwrap();
        assert!(!fresh.contains("--resume"), "{fresh}");
        // A window with no recipe (hand-started) stays None — the caller falls
        // back to the generic backend line.
        assert!(relaunch_line(ws.to_str().unwrap(), "byhand", None).is_none());
        std::fs::remove_dir_all(&ws).ok();
    }
}

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
         - `tmm status waiting|blocked \"why\"` — ONLY when you are stuck on something outside your control (a credential, an answer, another agent). Do not announce that you are working: your turn boundaries are observed automatically, so a `working` claim is ignored and only its note is kept\n\
         - `tmm done \"summary\"` — REQUIRED when you finish the briefed task\n\
         You can also manage the workspace itself when the task calls for it:\n\
         - `tmm spawn <registry-name> --brief \"...\"` — bring in a teammate (see `tmm registry list`)\n\
         - `tmm project create|up|down|archive` — set up or tear down whole projects\n\
         - `tmm registry save --name .. --backend .. --system \"..\"` — define NEW kinds of agents, then spawn them\n\
         Your first prompt of a session is a MARKER, not a request: `[<local time>] (session start)`. \
         It is how you learn the current time (a system prompt cannot carry a date — it is replayed every restart), and it means: \
         read the brief above, begin working, and run `tmm done \"summary\"` when the briefed task is complete. \
         Nothing else in that line is an instruction from the operator; every real request arrives as its own message.\n\
         Rules: your final answer each turn is captured automatically and posted to the room — do not repeat it with `tmm send`. \
         Use `tmm send` DURING a long turn for progress a human would want before it ends, and `tmm send \"@name ...\"` to hand work to a teammate (it types into their pane and interrupts them). \
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

/// The hook set for each backend, in ONE place. `render_*` writes it at spawn
/// and `refresh_hooks` rewrites it on every start, so a config on disk can
/// never be older than the app that reads its events. (It was: agents spawned
/// before `userPromptSubmit` existed kept a three-hook config, and since that
/// hook is the only reset of the same-turn dedup flag, their first `tmm send`
/// silently killed the stop-hook auto-post for the rest of the window's life.)
fn kiro_hooks(notify: &str) -> Value {
    json!({
        // The notify helper feeds notifications AND telemetry (tool events are
        // recognized by hook_event_name and routed to telemetry only).
        "preToolUse":  [ { "matcher": "*", "command": notify } ],
        "postToolUse": [ { "matcher": "*", "command": notify } ],
        // Turn start — the ONLY reset of the same-turn dedup flag, and the
        // event that carries the submitted prompt.
        "userPromptSubmit": [ { "command": notify } ],
        "stop": [ { "command": notify } ]
    })
}

fn claude_hooks(notify: &str) -> Value {
    json!({
        "PreToolUse":  [ { "matcher": "*", "hooks": [ { "type": "command", "command": notify } ] } ],
        "PostToolUse": [ { "matcher": "*", "hooks": [ { "type": "command", "command": notify } ] } ],
        "Notification": [ { "matcher": "permission_prompt|idle_prompt|agent_needs_input|agent_completed", "hooks": [ { "type": "command", "command": notify } ] } ],
        "Stop": [ { "hooks": [ { "type": "command", "command": notify } ] } ],
        "StopFailure": [ { "hooks": [ { "type": "command", "command": notify } ] } ]
    })
}

fn codex_hooks(notify: &str) -> Value {
    json!({
        "PreToolUse":  [ { "matcher": "*", "hooks": [ { "type": "command", "command": notify } ] } ],
        "PostToolUse": [ { "matcher": "*", "hooks": [ { "type": "command", "command": notify } ] } ],
        "PermissionRequest": [ { "hooks": [ { "type": "command", "command": notify } ] } ],
        "Stop": [ { "hooks": [ { "type": "command", "command": notify } ] } ]
    })
}

/// Bring a managed agent's hooks up to date with this build, in place. Returns
/// true when a file changed. Only the `hooks` key is touched: the prompt carries
/// the agent's brief, which was given once at spawn and cannot be rebuilt here.
///
/// Called on every start (`hub_agent_restart`, and `reconcile` when a project
/// comes up), so the app owns these configs rather than trusting whatever an
/// older version wrote.
pub fn refresh_hooks(project_path: &str, window_name: &str) -> bool {
    let home = std::path::Path::new(project_path).join(".tmm").join("agents").join(window_name);
    if !home.is_dir() {
        return false; // not a managed agent
    }
    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    if notifications.ensure_helper().is_err() {
        return false;
    }
    let mut changed = false;
    let kiro = home.join("agents").join(format!("{window_name}.json"));
    if kiro.is_file() {
        changed |= patch_hooks(&kiro, kiro_hooks(&notifications.helper_command("kiro")));
    }
    let claude = home.join("settings.json");
    if claude.is_file() {
        changed |= patch_hooks(&claude, claude_hooks(&notifications.helper_command("claude")));
    }
    let codex = home.join("codex").join("hooks.json");
    if codex.is_file() {
        changed |= patch_hooks(&codex, codex_hooks(&notifications.helper_command("codex")));
    }
    // Agents spawned before launch recipes existed can still be restarted with
    // full identity: for kiro the recipe is reconstructible from the isolated
    // home itself (env = KIRO_HOME, cmd = --agent <name>).
    if kiro.is_file() && !home.join("launch.json").exists() {
        write_launch_recipe(
            &home,
            "kiro",
            &[("KIRO_HOME".to_string(), home.to_string_lossy().to_string())],
            &format!(
                "command kiro-cli chat --agent {} --trust-all-tools kick",
                shared::shell_quote(window_name),
            ),
        );
        changed = true;
    }
    changed
}

/// Replace the `hooks` key of a JSON config, leaving everything else alone.
/// A no-op when the value already matches, so starting a project does not
/// rewrite files for nothing.
fn patch_hooks(path: &std::path::Path, hooks: Value) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    let Ok(mut root) = serde_json::from_str::<Value>(&text) else { return false };
    let Some(obj) = root.as_object_mut() else { return false };
    if obj.get("hooks") == Some(&hooks) {
        return false;
    }
    obj.insert("hooks".into(), hooks);
    std::fs::write(path, serde_json::to_string_pretty(&root).unwrap_or(text)).is_ok()
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
        "hooks": kiro_hooks(&notify),
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
            shared::shell_quote(&kick_now()),
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
            "hooks": claude_hooks(&notify)
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
            shared::shell_quote(&kick_now()),
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
            "hooks": codex_hooks(&notify)
        }))
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;

    if !def.model.is_empty() {
        config_args.push(format!("--model {}", shared::shell_quote(&def.model)));
    }
    config_args.push("--dangerously-bypass-approvals-and-sandbox".into());
    config_args.push("--dangerously-bypass-hook-trust".into());
    config_args.push(shared::shell_quote(&kick_now()));
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
        // The prompt must NOT carry a date: it is replayed every time the window
        // is restored, so a baked-in "today" becomes a lie. The KICK carries it.
        let year = chrono::Local::now().format("%Y").to_string();
        assert!(!prompt.contains(&year), "no wall-clock date in a replayed prompt");
        assert!(kick_now().starts_with(&format!("[{year}-")), "the kick tells the agent what now is");
        // The kick is a MARKER, not an instruction: that channel is echoed
        // into the chat as something the operator said, so standing
        // instructions live in the system prompt instead.
        let kick = kick_now();
        for word in ["Start now", "read your", "begin working", "REQUIRED"] {
            assert!(!kick.contains(word), "kick must not instruct ({word}): {kick}");
        }
        assert!(kick.ends_with("(session start)"), "kick is a marker: {kick}");
        assert!(conf.get("mcpServers").and_then(|m| m.get("files")).is_some(), "registry MCP def must materialize");
        // Tool hooks feed telemetry.
        assert!(conf.get("hooks").and_then(|h| h.get("preToolUse")).is_some());
        // A restart must replay the FULL identity, not the bare backend line:
        // the user-space config's hooks never fire (measured), so losing
        // KIRO_HOME/--agent makes a restarted agent observably deaf.
        write_launch_recipe(&dir, "kiro", &r.env, &r.cmd);
        let line = relaunch_line(
            dir.parent().unwrap().parent().unwrap().to_str().unwrap(),
            "tester", Some("abc-123"),
        );
        // agent_home(workspace, name) = <ws>/.tmm/agents/<name>; our temp dir is
        // not that shape, so call the parts directly instead:
        let recipe: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("launch.json")).unwrap()).unwrap();
        assert_eq!(recipe["backend"], "kiro");
        let cmd = recipe["cmd"].as_str().unwrap();
        assert!(cmd.contains("--agent tester"), "identity survives: {cmd}");
        assert!(!cmd.contains("session start"), "the kick is not replayed: {cmd}");
        let _ = line; // shape of the ws path differs in this fixture; covered below
        // Turn start resets the same-turn dedup flag. Without it a managed
        // agent that calls `tmm send` once never auto-posts again.
        let turn = conf.get("hooks").and_then(|h| h.get("userPromptSubmit")).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        assert_eq!(turn.len(), 1, "managed kiro must carry the turn-start hook");
        assert!(
            turn[0].get("command").and_then(|c| c.as_str()).is_some_and(|c| c.contains("tmux-mobile")),
            "turn-start hook must run the notify helper, got {turn:?}"
        );
        assert!(!r.cmd.contains("@team"), "no team plumbing in registry agents");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// A config written by an older build must not stay that way: agents
    /// spawned before `userPromptSubmit` existed kept a three-hook set, and
    /// that hook is the only reset of the same-turn dedup flag — so their first
    /// `tmm send` killed the stop-hook auto-post for good. Every start now
    /// re-materializes the hooks in place, and nothing else.
    #[test]
    fn refresh_hooks_repairs_a_stale_config_without_touching_the_prompt() {
        let ws = std::env::temp_dir().join(format!("tmm-refresh-{}", uuid::Uuid::new_v4()));
        let home = ws.join(".tmm/agents/dev/agents");
        std::fs::create_dir_all(&home).unwrap();
        let cfg = home.join("dev.json");
        // What the old renderer wrote: no turn-start hook, and a brief baked
        // into the prompt that cannot be rebuilt from anywhere.
        std::fs::write(&cfg, serde_json::to_string_pretty(&json!({
            "name": "dev",
            "prompt": "You are dev. Brief: fix the flaky test.",
            "hooks": { "stop": [ { "command": "old-helper kiro" } ] }
        })).unwrap()).unwrap();

        assert!(refresh_hooks(&ws.to_string_lossy(), "dev"), "a stale config is rewritten");
        let after: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        let hooks = after.get("hooks").and_then(|h| h.as_object()).unwrap();
        let mut keys: Vec<&str> = hooks.keys().map(String::as_str).collect();
        keys.sort();
        assert_eq!(keys, ["postToolUse", "preToolUse", "stop", "userPromptSubmit"]);
        assert_eq!(
            after.get("prompt").and_then(|p| p.as_str()),
            Some("You are dev. Brief: fix the flaky test."),
            "the brief survives — only hooks are ours to rewrite"
        );
        // Idempotent: a config already current is not rewritten.
        assert!(!refresh_hooks(&ws.to_string_lossy(), "dev"), "no needless writes");
        // A window with no isolated home is not ours to touch.
        assert!(!refresh_hooks(&ws.to_string_lossy(), "byhand"));
        let _ = std::fs::remove_dir_all(&ws);
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
        // The standing instructions the kick used to carry now live HERE.
        assert!(p.contains("(session start)"), "prompt explains the marker: {p}");
        assert!(p.contains("begin working"), "prompt carries the start instruction: {p}");
        assert!(p.contains("review the branch"));
    }
}
