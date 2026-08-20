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
    // A model the backend does not know is not a runtime hiccup: kiro answers
    // the first turn with "not available, use /model" and the agent is alive but
    // mute — no reply, no auto-post, nothing for the app to notice. Registry
    // defs saved before validation existed can still carry one, so refuse here
    // too rather than open a window that cannot work.
    super::models::validate(&def.backend, &def.model)?;

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
    // The launch line ends with the first prompt ONLY when a brief gave us
    // something for the agent to act on; otherwise the CLI opens and waits.
    let launch_cmd = match first_prompt(req.brief) {
        Some(p) => format!("{} {}", prepared.cmd, shared::shell_quote(&p)),
        None => prepared.cmd.clone(),
    };
    let full = format!("{} {}", prefix, launch_cmd);
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
    // `cmd` is the identity command with NO first prompt appended (spawn adds
    // that separately), so the recipe stores it verbatim. It used to strip a
    // trailing quoted argument to remove the kick — a guess that would have
    // eaten a legitimate quoted flag the day a backend ended with one.
    let recipe = json!({
        "backend": backend,
        "env": env.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "cmd": cmd.trim_end(),
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

/// The first prompt, if there is one AT ALL. A spawned agent used to receive a
/// synthetic starter (an instruction, then a `(session start)` marker) purely
/// because an interactive CLI sits idle until spoken to. Both were wrong for
/// the same reason: that channel is where the OPERATOR's words arrive, so
/// anything we invent there is a message the user never wrote — and an agent
/// handed a contentless prompt starts reasoning about nothing ("多此一举",
/// owner 2026-08-18). So: no brief, no prompt. The agent waits at its prompt,
/// costing nothing, until a real message arrives via `deliver_mentions` (which
/// stamps its own time — the only reason the marker carried one).
///
/// A brief IS something to consume: `tmm spawn <agent> --brief "…"` is a task
/// assignment from the operator or a teammate, so it is delivered as the first
/// message, stamped like every later one.

fn first_prompt(brief: &str) -> Option<String> {
    let brief = brief.trim();
    if brief.is_empty() {
        return None;
    }
    Some(format!("[{}] {brief}", chrono::Local::now().format("%Y-%m-%d %H:%M")))
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
            "command kiro-cli chat --agent lead --model m --trust-all-tools",
        );
        let line = relaunch_line(ws.to_str().unwrap(), "lead", Some("id-1")).unwrap();
        assert!(line.starts_with("KIRO_HOME="), "isolated home first: {line}");
        assert!(line.contains("--agent lead"), "identity: {line}");
        assert!(line.ends_with("--resume-id id-1"), "conversation resumes: {line}");
        // The recipe stores the identity command VERBATIM: a first prompt is
        // never part of it (spawn appends that separately, only for a brief),
        // so nothing has to be guessed off the end of the line.
        let stored: Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("launch.json")).unwrap()).unwrap();
        assert_eq!(
            stored.get("cmd").and_then(|c| c.as_str()).unwrap(),
            "command kiro-cli chat --agent lead --model m --trust-all-tools",
        );
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
         - `tmm status working \"<what you are doing right now>\"` — KEEP THIS CURRENT. Your turn boundaries are observed automatically, but nobody can see WHAT you are working on unless you say it. Send one when you start the task, again whenever you move to a different part of it, and again if a single step runs long. One short line, no ceremony — it appears in the chat as your current activity, and it is how the operator follows a long task without interrupting you\n\
         - `tmm status waiting|blocked \"why\"` — when you are stuck on something outside your control (a credential, an answer, another agent). This one asks for attention, so keep it for the real thing\n\
         - `tmm send \"@name message\"` — talk in the project chat (@name to address someone, use @human for the operator). This INTERRUPTS the reader, so use it for something that needs a person: a question, a decision, a result. Plain progress belongs in `tmm status`\n\
         - `tmm log --limit 30` — read recent chat; `tmm agent list` — who is here and their state\n\
         - `tmm done \"summary\"` — REQUIRED when you finish the briefed task\n\
         You can also manage the workspace itself when the task calls for it:\n\
         - `tmm spawn <registry-name> --brief \"...\"` — bring in a teammate (see `tmm registry list`)\n\
         - `tmm project create|up|down|archive` — set up or tear down whole projects\n\
         - `tmm registry save --name .. --backend .. --system \"..\"` — define NEW kinds of agents, then spawn them\n\
         When you start with no message waiting, just WAIT at your prompt — nothing is expected of you until someone writes. \
         Every real request arrives as a prompt stamped `[YYYY-MM-DD HH:MM]`, which is also how you learn the current time \
         (this system prompt cannot carry a date: it is replayed on every restart). \
         If a task was briefed to you it appears below — do it when you are asked to start, and run `tmm done \"summary\"` when it is complete.\n\
         Rules: your final answer each turn is captured automatically and posted to the room — do not repeat it with `tmm send`. \
         Keep `tmm status` flowing DURING a long turn so the work is visible before it ends, and use `tmm send \"@name ...\"` to hand work to a teammate (it types into their pane and interrupts them). \
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

/// Bring a managed agent's config up to date with this build, in place. Returns
/// true when a file changed. Two things are ours to rewrite — the `hooks` key
/// and a `--model` an older build left on the launch line; the prompt is not,
/// because it carries the agent's brief, which was given once at spawn and
/// cannot be rebuilt here.
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
        changed |= migrate_launch_model(&home, &kiro);
        // Settings drift is config drift: agents spawned before queue-mode
        // (or before settings existed at all) get the canonical file on their
        // next start, same as hooks.
        changed |= ensure_kiro_settings(&home);
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

/// Move a `--model <id>` an older build put on the launch line into the agent
/// config, where kiro actually honours it, and drop it from the recipe so the
/// two cannot disagree. Exact information, so nothing is guessed: the id is
/// read off the line that was really used.
///
/// Why it matters beyond tidiness: `refresh_hooks` also BACKFILLS recipes for
/// pre-recipe agents, and that backfilled line has no `--model` at all — so an
/// agent restarted through that path silently lost its model. Once the id is in
/// the config it survives every start (`up`, restart, resume) because they all
/// pass `--agent`.
fn migrate_launch_model(home: &Path, config: &Path) -> bool {
    let recipe_path = home.join("launch.json");
    let Ok(text) = std::fs::read_to_string(&recipe_path) else { return false };
    let Ok(mut recipe) = serde_json::from_str::<Value>(&text) else { return false };
    let Some(cmd) = recipe.get("cmd").and_then(Value::as_str).map(str::to_string) else {
        return false;
    };
    // Model ids never contain whitespace, so token splitting is exact here.
    let mut tokens: Vec<&str> = cmd.split_whitespace().collect();
    let Some(at) = tokens.iter().position(|t| *t == "--model") else { return false };
    let model = tokens
        .get(at + 1)
        .map(|m| m.trim_matches('\'').trim_matches('"').to_string())
        .filter(|m| !m.is_empty() && !m.starts_with('-'));
    tokens.drain(at..(at + 2).min(tokens.len()));
    recipe["cmd"] = json!(tokens.join(" "));
    let recipe_written = std::fs::write(
        &recipe_path,
        serde_json::to_string_pretty(&recipe).unwrap_or(text),
    )
    .is_ok();
    // Only fill a config that has no model of its own: a value already there
    // came from a newer spawn (or the user) and outranks the old launch line.
    let Some(model) = model else { return recipe_written };
    // And only if the backend actually accepts it. An id it rejects was never
    // the agent's model — kiro fell back to its default and said so above the
    // splash — so carrying the typo into the config would turn a working agent
    // into a mute one on its next restart. Dropping it preserves what was
    // really running.
    if let Err(e) = crate::projects::models::validate("kiro", &model) {
        eprintln!("projects: dropping the launch line's model for a managed agent — {e}");
        return recipe_written;
    }
    let config_written = std::fs::read_to_string(config)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|mut root| {
            let obj = root.as_object_mut()?;
            if obj.contains_key("model") {
                return None;
            }
            obj.insert("model".into(), json!(model));
            Some(std::fs::write(config, serde_json::to_string_pretty(&root).unwrap()).is_ok())
        })
        .unwrap_or(false);
    recipe_written || config_written
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

/// The CLI settings every managed kiro agent runs with (`<home>/settings/
/// cli.json`, read because the pane launches with `KIRO_HOME=<home>`).
///
/// `chat.defaultInterruptBehavior = "queue"` is an owner decision, 2026-08-20
/// ("所有 Agent 在 kiro 里边发送指令的模式 默认给我设计成 Queue 队列模式吧 不要
/// steer 模式"): a line typed at a BUSY agent waits for the turn to end instead
/// of steering the turn mid-flight — the agent reads it whole, as its own
/// prompt. That is also the contract the delivery pipeline already assumes:
/// `delivery_overdue` pauses the ack clock while a turn is open precisely
/// because kiro "Type to queue"s what we send.
fn kiro_cli_settings() -> Vec<(&'static str, Value)> {
    vec![
        ("chat.disableTrustAllConfirmation", json!(true)),
        ("chat.defaultInterruptBehavior", json!("queue")),
    ]
}

/// Force the canonical CLI settings into a managed kiro home, leaving any
/// other keys alone. Creates the file when it is missing (pre-settings homes),
/// no-op write when everything already matches — the same contract as
/// `patch_hooks`, because the app owns these configs. Returns true on change.
fn ensure_kiro_settings(home: &Path) -> bool {
    let dir = home.join("settings");
    if std::fs::create_dir_all(&dir).is_err() {
        return false;
    }
    let path = dir.join("cli.json");
    let mut root = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let obj = root.as_object_mut().expect("filtered to object above");
    let mut changed = !path.is_file();
    for (key, value) in kiro_cli_settings() {
        if obj.get(key) != Some(&value) {
            obj.insert(key.to_string(), value);
            changed = true;
        }
    }
    changed && std::fs::write(&path, serde_json::to_string_pretty(&root).unwrap()).is_ok()
}

fn render_kiro(
    def: &RegAgent, name: &str, home: &Path, system_prompt: &str,
    skills: &[crate::team::skills::ResolvedSkill],
) -> Result<Rendered, String> {
    std::fs::create_dir_all(home.join("agents")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(home.join("settings")).map_err(|e| e.to_string())?;
    // Fail-loud at spawn (a home without settings/cli.json would re-enable the
    // trust-all confirmation, which nobody is there to answer); refresh_hooks
    // reuses the same canonical list fail-soft via ensure_kiro_settings.
    let settings: serde_json::Map<String, Value> =
        kiro_cli_settings().into_iter().map(|(k, v)| (k.to_string(), v)).collect();
    std::fs::write(
        home.join("settings").join("cli.json"),
        serde_json::to_string_pretty(&Value::Object(settings)).unwrap(),
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
    let mut conf = json!({
        "name": name,
        "description": format!("{} (registry agent)", def.name),
        "prompt": system_prompt,
        "tools": ["*"],
        "allowedTools": ["*"],
        "resources": resources,
        "mcpServers": mcp_servers,
        "hooks": kiro_hooks(&notify),
    });
    // The model belongs to the agent's IDENTITY, not to one launch of it. It
    // used to ride on `--model`, which had two costs: it was invisible in the
    // config the owner reads (`.tmm/agents/<name>/agents/<name>.json`), and
    // kiro-cli's TUI answers an unknown id with a warning above the splash and
    // then runs its DEFAULT model — so a typo'd id was a silent downgrade. In
    // the config, kiro reports it as a real error on the first turn instead,
    // and every later start (resume, restart, `up`) reads the same field.
    // `registry_save` rejects unknown ids up front.
    //
    // An empty model means what the editor's placeholder says — the BACKEND's
    // default — so the key is omitted rather than set to a hardcoded id (the
    // old launch line pinned `claude-sonnet-4.6`, which silently contradicted
    // the UI and would have outlived that model).
    let model = def.model.trim();
    if !model.is_empty() {
        conf["model"] = json!(model);
    }
    std::fs::write(home.join("agents").join(format!("{name}.json")), serde_json::to_string_pretty(&conf).unwrap())
        .map_err(|e| e.to_string())?;

    Ok(Rendered {
        env: vec![("KIRO_HOME".into(), home.to_string_lossy().to_string())],
        cmd: format!(
            "command kiro-cli chat --agent {} --trust-all-tools",
            shared::shell_quote(name),
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
            "command claude --mcp-config {} --strict-mcp-config --settings {} --model {} --dangerously-skip-permissions --append-system-prompt {}",
            shared::shell_quote(&mcpfile.to_string_lossy()),
            shared::shell_quote(&settingsfile.to_string_lossy()),
            shared::shell_quote(model),
            shared::shell_quote(&full_prompt),
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
        let mut d = def("kiro");
        d.model = "claude-haiku-4.5".into();
        let r = render_kiro(&d, "tester", &dir, &build_prompt(&d, "tester", "proj", "fix the bug"), &[]).unwrap();
        assert!(r.env.iter().any(|(k, v)| k == "KIRO_HOME" && v.contains("tmm-spawn-kiro")), "home must be the isolated dir");
        let conf: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("agents/tester.json")).unwrap()).unwrap();
        // The model lives in the CONFIG, not on the launch line: kiro's TUI
        // treats an unknown `--model` as a warning and runs its default, so a
        // flag made a wrong id invisible (owner report, 2026-08-19).
        assert_eq!(conf.get("model").and_then(|m| m.as_str()), Some("claude-haiku-4.5"));
        assert!(!r.cmd.contains("--model"), "no model on the launch line: {}", r.cmd);
        let prompt = conf.get("prompt").and_then(|p| p.as_str()).unwrap();
        assert!(prompt.contains("tmm send"), "the tmm paragraph IS the integration");
        assert!(prompt.contains("fix the bug"), "brief must reach the prompt");
        // The prompt must NOT carry a date: it is replayed every time the window
        // is restored, so a baked-in "today" becomes a lie. The KICK carries it.
        let year = chrono::Local::now().format("%Y").to_string();
        assert!(!prompt.contains(&year), "no wall-clock date in a replayed prompt");
        // NOTHING is sent to an agent that was spawned without a brief: an
        // invented first prompt is a message the user never wrote, and it made
        // agents reason about nothing (owner, 2026-08-18).
        assert!(first_prompt("").is_none(), "no brief, no prompt");
        assert!(first_prompt("   ").is_none(), "whitespace is not a brief");
        // A brief IS something to consume: delivered as the first message,
        // stamped like every later one.
        let p = first_prompt("fix the flaky test").unwrap();
        assert!(p.starts_with(&format!("[{year}-")), "a delivered brief is stamped: {p}");
        assert!(p.ends_with("fix the flaky test"), "the brief is the message: {p}");
        assert!(conf.get("mcpServers").and_then(|m| m.get("files")).is_some(), "registry MCP def must materialize");
        // Tool hooks feed telemetry.
        assert!(conf.get("hooks").and_then(|h| h.get("preToolUse")).is_some());
        // The CLI settings ship with the home: queue mode is the DEFAULT for
        // every managed kiro agent (owner, 2026-08-20 — a line typed at a busy
        // agent waits for the turn to end instead of steering it mid-flight),
        // and the trust-all confirmation stays off.
        let cli: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("settings/cli.json")).unwrap()).unwrap();
        assert_eq!(cli.get("chat.defaultInterruptBehavior").and_then(|v| v.as_str()), Some("queue"));
        assert_eq!(cli.get("chat.disableTrustAllConfirmation").and_then(|v| v.as_bool()), Some(true));
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
        assert!(!cmd.contains("session start"), "no synthetic kick anywhere: {cmd}");
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

    /// Agents spawned before queue mode (or before settings existed at all)
    /// pick it up on their next start: `refresh_hooks` treats settings drift
    /// as config drift. Owner decision, 2026-08-20: every managed kiro agent
    /// runs with `chat.defaultInterruptBehavior = "queue"` — a message typed
    /// at a busy agent is read whole when the turn ends, never steered into
    /// the middle of it.
    #[test]
    fn refresh_hooks_backfills_queue_mode_settings() {
        let ws = std::env::temp_dir().join(format!("tmm-qmode-{}", uuid::Uuid::new_v4()));
        let home = ws.join(".tmm/agents/dev");
        std::fs::create_dir_all(home.join("agents")).unwrap();
        std::fs::write(home.join("agents/dev.json"), serde_json::to_string_pretty(&json!({
            "name": "dev", "prompt": "You are dev.", "hooks": {}
        })).unwrap()).unwrap();
        let cli = home.join("settings/cli.json");

        // Case 1: a pre-settings home has NO cli.json — it is created whole.
        assert!(refresh_hooks(&ws.to_string_lossy(), "dev"));
        let read = || -> serde_json::Value {
            serde_json::from_str(&std::fs::read_to_string(&cli).unwrap()).unwrap()
        };
        assert_eq!(read().get("chat.defaultInterruptBehavior").and_then(|v| v.as_str()), Some("queue"));

        // Case 2: an older file missing the key gains it, and a key the app
        // does not own survives untouched.
        std::fs::write(&cli, serde_json::to_string_pretty(&json!({
            "chat.disableTrustAllConfirmation": true,
            "chat.editMode": "vi"
        })).unwrap()).unwrap();
        assert!(ensure_kiro_settings(&home), "missing key is backfilled");
        let after = read();
        assert_eq!(after.get("chat.defaultInterruptBehavior").and_then(|v| v.as_str()), Some("queue"));
        assert_eq!(after.get("chat.editMode").and_then(|v| v.as_str()), Some("vi"), "foreign keys are not ours to drop");

        // Case 3: already canonical — no write, so starting a project does not
        // churn mtimes.
        assert!(!ensure_kiro_settings(&home), "no needless writes");
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// An empty model means the BACKEND's default — which is what the agent
    /// editor's placeholder promises. The key is omitted rather than pinned to
    /// a hardcoded id (the launch line used to force `claude-sonnet-4.6`).
    #[test]
    fn no_model_configured_leaves_the_backend_default_alone() {
        let dir = std::env::temp_dir().join(format!("tmm-spawn-nomodel-{}", uuid::Uuid::new_v4()));
        let mut d = def("kiro");
        d.model = "   ".into();
        let r = render_kiro(&d, "tester", &dir, "p", &[]).unwrap();
        let conf: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("agents/tester.json")).unwrap()).unwrap();
        assert!(conf.get("model").is_none(), "no key at all, not \"\" (kiro rejects that): {conf}");
        assert!(!r.cmd.contains("--model"));
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Agents spawned by an older build carry their model on the launch line,
    /// where kiro downgrades a wrong id silently — and where the recipe backfill
    /// drops it entirely on restart. Every start moves it into the config once.
    #[test]
    fn a_launch_line_model_migrates_into_the_config() {
        let ws = std::env::temp_dir().join(format!("tmm-modelmig-{}", uuid::Uuid::new_v4()));
        let home = ws.join(".tmm/agents/dev");
        std::fs::create_dir_all(home.join("agents")).unwrap();
        let cfg = home.join("agents").join("dev.json");
        std::fs::write(&cfg, serde_json::to_string_pretty(&json!({
            "name": "dev",
            "prompt": "You are dev.",
            "hooks": {}
        })).unwrap()).unwrap();
        write_launch_recipe(
            &home,
            "kiro",
            &[("KIRO_HOME".to_string(), home.to_string_lossy().to_string())],
            "command kiro-cli chat --agent dev --model claude-haiku-4.5 --trust-all-tools",
        );

        assert!(refresh_hooks(&ws.to_string_lossy(), "dev"), "a stale config is rewritten");
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert_eq!(after.get("model").and_then(|m| m.as_str()), Some("claude-haiku-4.5"));
        let recipe: Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("launch.json")).unwrap()).unwrap();
        let cmd = recipe["cmd"].as_str().unwrap();
        assert!(!cmd.contains("--model"), "the line must not keep a second opinion: {cmd}");
        assert!(cmd.contains("--agent dev") && cmd.contains("--trust-all-tools"), "{cmd}");
        // Idempotent, and a model already in the config outranks the line.
        assert!(!refresh_hooks(&ws.to_string_lossy(), "dev"), "no needless writes");
        std::fs::remove_dir_all(&ws).ok();
    }

    /// The migration must preserve what was really RUNNING, and an id the
    /// backend rejects was never that: kiro fell back to its default. Carrying
    /// such a typo into the config would turn a working agent into a mute one
    /// on its next restart, so it is dropped — the line is still cleaned up.
    #[test]
    fn a_launch_line_model_the_backend_rejects_is_dropped_not_migrated() {
        if super::super::models::list("kiro").is_none() {
            eprintln!("kiro-cli unavailable — nothing can be rejected, skipping");
            return;
        }
        let ws = std::env::temp_dir().join(format!("tmm-modelbad-{}", uuid::Uuid::new_v4()));
        let home = ws.join(".tmm/agents/dev");
        std::fs::create_dir_all(home.join("agents")).unwrap();
        let cfg = home.join("agents").join("dev.json");
        std::fs::write(&cfg, serde_json::to_string_pretty(&json!({ "name": "dev", "hooks": {} })).unwrap()).unwrap();
        write_launch_recipe(
            &home,
            "kiro",
            &[],
            // The owner's real value: one character off `claude-sonnet-4.5`.
            "command kiro-cli chat --agent dev --model claude-sonnet-4-5 --trust-all-tools",
        );

        refresh_hooks(&ws.to_string_lossy(), "dev");
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert!(after.get("model").is_none(), "a rejected id must not reach the config: {after}");
        let recipe: Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("launch.json")).unwrap()).unwrap();
        assert!(!recipe["cmd"].as_str().unwrap().contains("--model"), "the line is cleaned up either way");
        std::fs::remove_dir_all(&ws).ok();
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
        assert!(p.contains("just WAIT at your prompt"), "prompt tells it to idle: {p}");
        assert!(p.contains("[YYYY-MM-DD HH:MM]"), "prompt explains message stamps: {p}");
        assert!(p.contains("review the branch"));
    }
}
