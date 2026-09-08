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
/// Agent windows per project. 4 until 2026-09-02; raised for NESTED teams
/// (owner: the dev squad includes the four-reviewer board), so 8 = a squad of
/// four plus its review board. Still counts every agent-looking window.
pub const SPAWN_CAP: usize = 8;

pub struct SpawnRequest<'a> {
    pub session: &'a str,
    /// Registry definition name to spawn (ignored when `def` is given).
    pub agent: &'a str,
    /// Opening brief posted into the hub chat and injected into the prompt.
    pub brief: &'a str,
    /// Who asked (agent name from $TMM_AGENT, or empty = the human).
    pub by: &'a str,
    /// A ready-made definition instead of a registry lookup — how a TEAM
    /// member spawns (board #74): its base def with the role block appended.
    pub def: Option<RegAgent>,
    /// The exact window name to use; the caller has already uniquified it.
    /// `None` = the def's name, uniquified here.
    pub window_name: Option<String>,
    /// The team this spawn belongs to; recorded in the launch recipe so the
    /// Hub can group the cards.
    pub team: Option<String>,
    /// Restart fallback for a managed home whose slot is not currently
    /// restorable. Normal hires start fresh; Start/Restart resumes this
    /// isolated home's newest cwd conversation.
    pub resume: bool,
}

impl Default for SpawnRequest<'_> {
    fn default() -> Self {
        SpawnRequest {
            session: "", agent: "", brief: "", by: "", def: None,
            window_name: None, team: None, resume: false,
        }
    }
}

/// Spawn `agent` into `session`. Returns `{ window_name, pane }`.
pub fn spawn(req: &SpawnRequest) -> Result<Value, String> {
    let def = match &req.def {
        Some(d) => d.clone(),
        None => super::registry_get(req.agent)?
            .ok_or_else(|| format!("no agent named '{}' in the registry", req.agent))?,
    };
    // A model the backend does not know is not a runtime hiccup: kiro answers
    // the first turn with "not available, use /model" and the agent is alive but
    // mute — no reply, no auto-post, nothing for the app to notice. Registry
    // defs saved before validation existed can still carry one, so refuse here
    // too rather than open a window that cannot work.
    super::models::validate(&def.backend, &def.model)?;
    super::models::validate_effort(&def.backend, &def.effort)?;

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
        .filter(|p| super::agents::detect_pane(Some(workspace.as_str()), p).is_some())
        .count();
    if agent_windows >= SPAWN_CAP {
        return Err(format!("project already has {agent_windows} agents (cap {SPAWN_CAP}) — finish or close one first"));
    }

    // Window name = agent name, uniquified if taken (lead, lead-2, …). The
    // Window name is the agent's identity for telemetry and tmm messages.
    let taken: std::collections::HashSet<&str> = panes.iter().map(|p| p.window_name.as_str()).collect();
    let window_name = match &req.window_name {
        Some(w) => w.clone(),
        None => uniquify(&def.name, &taken)?,
    };
    // A def saved before names were validated, or an explicit window name
    // from a caller, still has to be a plain word here: it is about to be a
    // directory under `.tmm/agents/` and a tmux window name.
    super::agents::valid_name(&window_name)?;

    let m = materialize(&def, &window_name, req.session, &workspace, req.brief, req.by, req.team.as_deref())?;
    let home = m.home;
    let env = m.env;

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
    // Restart fallback is different from a new hire: its isolated home already
    // owns the conversation, so resume recent even when the slot/exact id raced
    // the capture tick.
    let launch_cmd = launch_command(&m.cmd, &def.backend, req.brief, req.by, req.resume);
    let full = format!("{} {}", prefix, launch_cmd);
    // NEVER send the full line via send-keys — see team/launch.rs: tty shims
    // swallow bursts ≳2KB. Source a script instead.
    let script = shared::write_launch_script(&home, &window_name, &full)?;
    tmux::send_command(&pane, &format!(". {}", shared::shell_quote(&script.to_string_lossy())))?;
    if let Some(confirmation) = m.confirmation {
        shared::confirm_startup_prompt(pane.clone(), confirmation);
    }

    Ok(json!({ "window_name": window_name, "pane": pane, "backend": def.backend }))
}

/// Everything an agent IS on disk: its isolated home, rendered backend
/// config, launch command, environment, and the recipe that replays them.
struct Materialized {
    home: PathBuf,
    cmd: String,
    env: Vec<(String, String)>,
    confirmation: Option<shared::StartupConfirmation>,
}

/// Materialize (or RE-materialize) an agent's home from its definition:
/// prompt, backend config, skills, MCP seed, hooks, and the launch recipe —
/// everything but the tmux window. `spawn` calls it before opening the
/// window; `refresh_agent` calls it again at restart, so a replayed recipe
/// launches the CURRENT definition and app-wide instructions instead of the
/// spawn-time snapshot.
fn materialize(
    def: &RegAgent,
    window_name: &str,
    session: &str,
    workspace: &str,
    brief: &str,
    by: &str,
    team: Option<&str>,
) -> Result<Materialized, String> {
    let home = agent_home(workspace, window_name);
    std::fs::create_dir_all(&home).map_err(|e| format!("create agent home: {e}"))?;
    ensure_gitignore(workspace);

    // The software-wide instructions (`<config>/AGENTS.md`) lead every prompt,
    // on every backend — read now, so a spawn always carries the current text.
    let system_prompt = build_prompt(def, window_name, session, brief, by, &super::global_prompt::read());
    let skills = resolve_skill_refs(def, &home);
    // MCP is ONE door now (owner, 2026-08-28): registry defs seed the shared
    // workspace config that `tmm mcp` reads per call — never a backend's
    // native config, which loads once at CLI start and made every server
    // change a restart.
    let mcp_config = seed_mcp_config(Path::new(workspace), &mcp_defs(def))?;

    let prepared = match def.backend.as_str() {
        "kiro" => render_kiro(def, window_name, &home, &system_prompt, &skills)?,
        "claude" => render_claude(def, window_name, &home, Path::new(workspace), &system_prompt, &skills)?,
        "codex" => render_codex(def, window_name, &home, &system_prompt, &skills)?,
        "grok" => render_grok(def, window_name, &home, &system_prompt, &skills)?,
        "omp" => render_omp(def, window_name, &home, &system_prompt, &skills)?,
        other => return Err(format!("unknown backend '{other}'")),
    };

    // Env every spawned agent gets: its identity for tmm.
    let mut env = prepared.env;
    env.push(("TMM_PROJECT".into(), session.to_string()));
    env.push(("TMM_AGENT".into(), window_name.to_string()));
    // The MCP config is findable from ANY cwd, not just under the workspace.
    env.push(("TMM_MCP_CONFIG".into(), mcp_config.to_string_lossy().to_string()));
    // tmm sits next to the server binary, while user-installed backends and
    // MCP runners commonly sit in ~/.local/bin or ~/.cargo/bin. The server is
    // often supervised with a minimal PATH, so make the recipe self-sufficient
    // instead of relying on the interactive shell to repair it later.
    let inherited_path = env
        .iter()
        .rev()
        .find(|(key, _)| key == "PATH")
        .map(|(_, value)| value.clone())
        .unwrap_or_else(|| std::env::var("PATH").unwrap_or_default());
    env.retain(|(key, _)| key != "PATH");
    env.push((
        "PATH".into(),
        shared::agent_launch_path(tmm_dir().as_deref(), &inherited_path),
    ));

    // Record before acting: the recipe is what makes this window OURS on
    // restart (`detect_managed` reads the backend off it, `relaunch_line`
    // replays it). Written here, before the window exists, so a write failure
    // is a spawn failure with nothing left running — not a live agent that
    // restarts deaf on the generic launch path.
    write_launch_recipe(&home, &def.backend, &env, &prepared.cmd, by, team)?;
    Ok(Materialized { home, cmd: prepared.cmd, env, confirmation: prepared.confirmation })
}

/// Re-materialize an existing managed agent from its CURRENT definition and
/// the current app-wide AGENTS.md, preserving the recipe's spawner and team.
/// This is what makes `restart` mean "come back up to date": the recipe replay
/// itself is verbatim, so without this an agent restarted forever with its
/// spawn-time prompt (global_prompt.rs promised otherwise — owner noticed,
/// 2026-09-08). `false` when the window is not one we can refresh — no recipe
/// on disk, or a name that resolves to no registry def (a uniquified teammate
/// like `lead-2`, a team-role synthetic) — those keep replaying their
/// spawn-time snapshot, exactly as before.
pub fn refresh_agent(project_path: &str, session: &str, window_name: &str) -> bool {
    let Ok(Some(def)) = super::registry_get(window_name) else { return false };
    let home = agent_home(project_path, window_name);
    let Ok(recipe) = std::fs::read_to_string(home.join("launch.json")) else { return false };
    let recipe: Value = match serde_json::from_str(&recipe) {
        Ok(v) => v,
        Err(_) => return false,
    };
    // A def edited into an invalid model must not brick the restart: keep the
    // old materials rather than write a config the backend will refuse.
    if super::models::validate(&def.backend, &def.model).is_err()
        || super::models::validate_effort(&def.backend, &def.effort).is_err()
    {
        return false;
    }
    let by = recipe.get("spawned_by").and_then(|v| v.as_str()).unwrap_or("");
    let team = recipe.get("team").and_then(|v| v.as_str()).filter(|t| !t.is_empty());
    materialize(&def, window_name, session, project_path, "", by, team).is_ok()
}

/// Window name = agent name, uniquified if taken (lead, lead-2, …). The window
/// name is the agent's identity for telemetry and tmm messages.
fn uniquify(name: &str, taken: &std::collections::HashSet<&str>) -> Result<String, String> {
    if !taken.contains(name) {
        return Ok(name.to_string());
    }
    (2..10)
        .map(|i| format!("{name}-{i}"))
        .find(|c| !taken.contains(c.as_str()))
        .ok_or_else(|| "too many windows with this agent's name".to_string())
}

/// Spawn every member of a configured TEAM into `session` (board #74). Names
/// are uniquified up front so the roster block each member receives names the
/// windows that will actually exist; members spawn in order; one failure does
/// not stop the others and is reported in `errors`. Returns
/// `{ team, spawned: [{name, window_name, pane}], errors: [..] }`.
pub fn spawn_team(session: &str, team_name: &str, brief: &str, by: &str) -> Result<Value, String> {
    let team = super::team_get(team_name)?
        .ok_or_else(|| format!("no team named '{team_name}'"))?;
    // Nested teams flatten here (board #74 follow-up): every leaf remembers
    // the path it came in on, which is what the recipe records and the
    // roster groups on.
    let flat = super::teams::expand(&team, &|n| super::team_get(n), SPAWN_CAP)?;
    let project = super::project_for_session(session)?
        .ok_or_else(|| format!("no project for session '{session}'"))?;
    let panes = tmux::list_panes(session).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    let existing = panes
        .iter()
        .filter(|p| seen.insert(p.window))
        .filter(|p| super::agents::detect_pane(Some(project.path.as_str()), p).is_some())
        .count();
    if existing + flat.len() > SPAWN_CAP {
        return Err(format!(
            "team '{team_name}' expands to {} agents and the project already has {existing} (cap {SPAWN_CAP}) — stop some first",
            flat.len()
        ));
    }
    // Final names first, so every member's prompt can name its teammates.
    let mut taken: std::collections::HashSet<String> = panes.iter().map(|p| p.window_name.clone()).collect();
    let mut roster: Vec<super::teams::RosterEntry> = Vec::new();
    for f in &flat {
        let borrowed: std::collections::HashSet<&str> = taken.iter().map(String::as_str).collect();
        let w = uniquify(f.member.name.trim(), &borrowed)?;
        taken.insert(w.clone());
        roster.push((w, f.member.role.clone(), f.path.clone()));
    }
    let mut spawned = Vec::new();
    let mut errors = Vec::new();
    for (f, (w, _, path)) in flat.iter().zip(roster.iter()) {
        let m = &f.member;
        let base = if m.base.trim().is_empty() { None } else { super::registry_get(m.base.trim())? };
        let result = super::teams::effective_def(f, base.as_ref(), w, &roster).and_then(|def| {
            spawn(&SpawnRequest {
                session,
                agent: w,
                brief,
                by,
                def: Some(def),
                window_name: Some(w.clone()),
                team: Some(path.clone()),
                resume: false,
            })
        });
        match result {
            Ok(r) => spawned.push(json!({ "name": m.name, "window_name": w, "team": path, "pane": r.get("pane").cloned().unwrap_or(Value::Null), "backend": r.get("backend").cloned().unwrap_or(Value::Null) })),
            Err(e) => errors.push(json!({ "name": m.name, "team": path, "error": e })),
        }
    }
    Ok(json!({ "team": team.name, "spawned": spawned, "errors": errors }))
}

/// Persist how this agent is STARTED, so a restart can replay the full
/// identity. Without it, the resume path fell back to the plain backend
/// launch line ("kiro-cli chat --resume-id …" — no KIRO_HOME, no --agent),
/// which runs the USER-SPACE config whose hooks never fire (measured,
/// kiro-cli 2.16.2): the restarted agent went observably deaf — no tool rows,
/// no auto-post, every delivery "unconfirmed" (owner report, 2026-08-18).
/// The kick is NOT part of the recipe: it belongs to the first launch only;
/// a restart resumes a conversation instead.
fn write_launch_recipe(home: &Path, backend: &str, env: &[(String, String)], cmd: &str, by: &str, team: Option<&str>) -> Result<(), String> {
    // `cmd` is the identity command with NO first prompt appended (spawn adds
    // that separately), so the recipe stores it verbatim. It used to strip a
    // trailing quoted argument to remove the kick — a guess that would have
    // eaten a legitimate quoted flag the day a backend ended with one.
    // `spawned_by` records who created the slot; the first stamped brief carries
    // the live reply edge. Empty = the human.
    // `team` (board #74): the configured team this window was spawned as part
    // of, so the Hub can group the cards; absent for a solo spawn.
    let recipe = json!({
        "backend": backend,
        "env": env.iter().map(|(k, v)| json!([k, v])).collect::<Vec<_>>(),
        "cmd": cmd.trim_end(),
        "spawned_by": by,
        "team": team.unwrap_or(""),
    });
    std::fs::write(
        home.join("launch.json"),
        serde_json::to_string_pretty(&recipe).unwrap(),
    )
    .map_err(|e| format!("write launch recipe {}: {e}", home.join("launch.json").display()))
}

/// Add the backend's exact or safe-recent resume dialect to one persisted
/// identity command. The caller separately replays the recipe environment.
fn resume_command(cmd: &str, backend: &str, session_id: Option<&str>) -> String {
    let exact = session_id.filter(|s| !s.is_empty());
    match (backend, exact) {
        ("kiro", Some(id)) => format!("{cmd} --resume-id {}", shared::shell_quote(id)),
        ("claude" | "grok", Some(id)) => {
            format!("{cmd} --resume {}", shared::shell_quote(id))
        }
        // codex's resume is a SUBCOMMAND, so it splices in after the binary
        // instead of appending: `codex resume <id> <flags>`. The managed
        // CODEX_HOME belongs to this one agent and `--last` is cwd-filtered,
        // so the recent fallback cannot cross into another project/session.
        ("codex", id) => match cmd.strip_prefix("command codex ") {
            Some(rest) => {
                let which = id
                    .map(shared::shell_quote)
                    .unwrap_or_else(|| "--last".to_string());
                format!("command codex resume {which} {rest}")
            }
            // An unexpected recipe shape: relaunch without resume rather than
            // guess at where the subcommand goes.
            None => cmd.to_string(),
        },
        // The other three managed homes are isolated too; their native recent
        // flags are cwd-scoped. This closes the window before the 20s capture
        // tick has persisted an exact session id.
        ("kiro", None) => format!("{cmd} --resume"),
        ("claude" | "grok", None) => format!("{cmd} --continue"),
        // omp: `--resume <id>` takes an id prefix; `--continue` follows the
        // cwd-scoped breadcrumb (sessions live per encoded cwd under the
        // isolated PI_CODING_AGENT_DIR, so it cannot cross projects).
        ("omp", Some(id)) => format!("{cmd} --resume {}", shared::shell_quote(id)),
        ("omp", None) => format!("{cmd} --continue"),
        _ => cmd.to_string(),
    }
}

/// Replay the persisted identity (env + backend command), preferring an exact
/// conversation id and otherwise the isolated home's newest cwd conversation.
pub fn relaunch_line(project_path: &str, window_name: &str, session_id: Option<&str>) -> Option<String> {
    let home = agent_home(project_path, window_name);
    let recipe: Value = serde_json::from_str(&std::fs::read_to_string(home.join("launch.json")).ok()?).ok()?;
    let cmd = recipe.get("cmd")?.as_str()?;
    let backend = recipe.get("backend").and_then(|b| b.as_str()).unwrap_or("");
    let cmd = resume_command(cmd, backend, session_id);
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

fn first_prompt(brief: &str, by: &str) -> Option<String> {
    let brief = brief.trim();
    if brief.is_empty() {
        return None;
    }
    let sender = if by.trim().is_empty() { "human" } else { by.trim() };
    Some(format!(
        "[tmm chat {}] {sender}: {brief}",
        chrono::Local::now().format("%Y-%m-%d %H:%M")
    ))
}

fn launch_command(identity_cmd: &str, backend: &str, brief: &str, by: &str, resume: bool) -> String {
    let identity_cmd = if resume {
        resume_command(identity_cmd, backend, None)
    } else {
        identity_cmd.to_string()
    };
    match first_prompt(brief, by) {
        Some(p) => format!("{} {}", identity_cmd, shared::shell_quote(&p)),
        None => identity_cmd,
    }
}

#[cfg(test)]
mod relaunch_tests {
    use super::*;

    #[test]
    fn restart_spawn_resumes_but_a_new_hire_starts_fresh() {
        let cmd = "command claude --settings /tmp/settings.json";
        assert_eq!(launch_command(cmd, "claude", "", "", false), cmd);
        assert_eq!(
            launch_command(cmd, "claude", "", "", true),
            "command claude --settings /tmp/settings.json --continue"
        );
        assert_eq!(
            launch_command("command codex -c a=b", "codex", "", "", true),
            "command codex resume --last -c a=b"
        );
    }

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
            "", None).unwrap();
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
            "",
        );
        // No recorded conversation yet: the managed home is isolated to this
        // one agent, so relaunch the newest conversation instead of opening a
        // blank prompt while the 20s capture tick catches up.
        let recent = relaunch_line(ws.to_str().unwrap(), "lead", None).unwrap();
        assert!(recent.ends_with("--resume"), "managed kiro resumes recent: {recent}");
        // A window with no recipe (hand-started) stays None — the caller falls
        // back to the generic backend line.
        assert!(relaunch_line(ws.to_str().unwrap(), "byhand", None).is_none());

        for (name, backend, cmd, flag) in [
            ("cl", "claude", "command claude --settings /tmp/settings.json", "--continue"),
            ("gx", "grok", "command grok --always-approve --agent gx", "--continue"),
        ] {
            let ahome = agent_home(ws.to_str().unwrap(), name);
            std::fs::create_dir_all(&ahome).unwrap();
            write_launch_recipe(&ahome, backend, &[], cmd, "", None).unwrap();
            let exact = relaunch_line(ws.to_str().unwrap(), name, Some("conv-1")).unwrap();
            assert!(exact.ends_with("--resume conv-1"), "{backend} exact resume: {exact}");
            let recent = relaunch_line(ws.to_str().unwrap(), name, None).unwrap();
            assert!(recent.ends_with(flag), "{backend} recent resume: {recent}");
        }

        // codex resumes via a SUBCOMMAND, so the id splices in after the
        // binary instead of appending (verified on codex-cli 0.148.0:
        // `codex resume <id>` accepts the recipe's own flags). Appending
        // would hand `resume` to the interactive CLI as a prompt.
        let chome = agent_home(ws.to_str().unwrap(), "cx");
        std::fs::create_dir_all(&chome).unwrap();
        write_launch_recipe(
            &chome,
            "codex",
            &[("CODEX_HOME".to_string(), chome.to_string_lossy().to_string())],
            "command codex -c a=b --dangerously-bypass-approvals-and-sandbox",
            "", None).unwrap();
        let cx = relaunch_line(ws.to_str().unwrap(), "cx", Some("01a0-abc")).unwrap();
        assert!(
            cx.contains("command codex resume 01a0-abc -c a=b --dangerously-bypass-approvals-and-sandbox"),
            "subcommand splice: {cx}"
        );
        let cx_recent = relaunch_line(ws.to_str().unwrap(), "cx", None).unwrap();
        assert!(
            cx_recent.contains("command codex resume --last -c a=b --dangerously-bypass-approvals-and-sandbox"),
            "isolated CODEX_HOME can safely resume its last cwd session: {cx_recent}"
        );
        std::fs::remove_dir_all(&ws).ok();
    }
}

struct Rendered {
    env: Vec<(String, String)>,
    cmd: String,
    confirmation: Option<shared::StartupConfirmation>,
}

/// The isolated home for `name`. Callers reach this only with a name that
/// passed `agents::valid_name` (spawn checks the window name up front, and
/// every other name comes off a slot or a tmux window that `is_managed_in`
/// already accepted), so a rejection here is a programming error, not a
/// user-facing one — it falls back to a path that cannot exist rather than
/// escape the agents directory.
fn agent_home(workspace: &str, name: &str) -> PathBuf {
    super::agents::home_dir(workspace, name).unwrap_or_else(|| {
        Path::new(workspace).join(".tmm").join("agents").join("__invalid-name__")
    })
}

/// `.tmm/` self-gitignores (same convention as team runtime homes).
fn ensure_gitignore(workspace: &str) {
    let dir = Path::new(workspace).join(".tmm");
    let gi = dir.join(".gitignore");
    if dir.is_dir() && !gi.exists() {
        let _ = std::fs::write(gi, "*\n");
    }
}

/// Persona + the complete tmm collaboration flow. The brief is a real first
/// user message, never duplicated into this replayed system prompt.
fn build_prompt(def: &RegAgent, name: &str, session: &str, _brief: &str, _by: &str, global: &str) -> String {
    let mut s = String::new();
    // House rules before the role: the app-wide AGENTS.md (global_prompt.rs)
    // is the first block, then the agent's own persona.
    if !global.trim().is_empty() {
        s += global.trim();
        s += "\n\n";
    }
    if !def.system.trim().is_empty() {
        s += def.system.trim();
        s += "\n\n";
    }
    s += &format!(
        "You are agent \"{name}\" in project \"{session}\" (a tmux session managed by tmux-mobile).\n\
         \n\
         tmm collaboration flow:\n\
         - Messages arrive in your pane as `[tmm chat YYYY-MM-DD HH:MM] <sender>: <text>`. Messages received while you work are queued and delivered after the current turn.\n\
         - Hooks automatically record your normal final response in the project room. If another agent initiated the turn, the result is also delivered back to that agent. Do not repeat it with `tmm send`.\n\
         - Use `tmm send \"@name message\"` only to start a new question, notification, or handoff. One message may address several names; `@all` reaches every agent and `@human` reaches the operator. An ordinary send without a recipient is rejected.\n\
         - Use `tmm send \"current progress\" --status` for optional ambient progress. It records in the room without interrupting anyone or suppressing your final response.\n\
         - Use `tmm log --limit 50` for recent messages, `tmm log --grep <text> [--global]` to search history, and `tmm agent list` for participants. Read a queued backlog in full before replying once; when context is unclear, check the log or ask the relevant person.\n\
         - For a board issue, run `tmm board take <id>`, record progress with `tmm board note <id> \"...\"`, and hand off completed work with `tmm board move <id> review`.\n\
         - Delegate independent parallel work with `tmm spawn <agent> --brief \"...\"` or `tmm spawn --team <team> --brief \"...\"`. State the objective, inputs, outputs, and completion criteria; results return automatically.\n\
         - With no message, wait. If tmm is temporarily unavailable, continue local work. Use `tmm --help` for the remaining commands."
    );
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

/// Seed `<ws>/.tmm/mcp.json` from registry defs — MERGE, never clobber: the
/// file is the AGENT's to edit (owner, 2026-08-28: "agent 自己写一个 mcp 配置
/// 配件"), so an existing entry always wins over the registry's copy and
/// unknown entries are kept. The standard shape (claude_mcp_value) is what
/// the MCP Inspector CLI reads. Returns the config path.
pub(crate) fn seed_mcp_config(
    workspace: &Path,
    defs: &[shared::McpDef],
) -> Result<std::path::PathBuf, String> {
    let dir = workspace.join(".tmm");
    std::fs::create_dir_all(&dir).map_err(|e| format!("create .tmm: {e}"))?;
    let path = dir.join("mcp.json");
    let mut root: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_else(|| json!({}));
    if !root.is_object() {
        root = json!({});
    }
    if !root.get("mcpServers").map(|v| v.is_object()).unwrap_or(false) {
        root["mcpServers"] = json!({});
    }
    let servers = root["mcpServers"].as_object_mut().unwrap();
    let mut changed = false;
    for m in defs {
        if !m.name.is_empty() && !servers.contains_key(&m.name) {
            servers.insert(m.name.clone(), shared::claude_mcp_value(m));
            changed = true;
        }
    }
    if changed || !path.is_file() {
        std::fs::write(&path, serde_json::to_string_pretty(&root).unwrap())
            .map_err(|e| format!("write mcp.json: {e}"))?;
    }
    Ok(path)
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
        // Turn start — resets the same-turn dedup flag and carries the
        // submitted prompt (the delivery receipt). Shipping Stop WITHOUT this
        // made the flag sticky: the first `tmm send` killed the auto-post for
        // every later turn of that window, and lines typed by
        // `deliver_mentions` were never acked (hollow ring forever). Kiro and
        // grok had it; claude and codex did not (owner, 2026-08-22: 对齐).
        "UserPromptSubmit": [ { "hooks": [ { "type": "command", "command": notify } ] } ],
        "Notification": [ { "matcher": "permission_prompt|agent_needs_input|agent_completed", "hooks": [ { "type": "command", "command": notify } ] } ],
        "Stop": [ { "hooks": [ { "type": "command", "command": notify } ] } ],
        "StopFailure": [ { "hooks": [ { "type": "command", "command": notify } ] } ]
    })
}

fn codex_hooks(notify: &str) -> Value {
    json!({
        "PreToolUse":  [ { "matcher": "*", "hooks": [ { "type": "command", "command": notify } ] } ],
        "PostToolUse": [ { "matcher": "*", "hooks": [ { "type": "command", "command": notify } ] } ],
        // Same turn-start contract as claude's (measured, codex-cli 0.148.0:
        // payload {hook_event_name:"UserPromptSubmit", prompt, session_id} on
        // hook stdin). Codex has NO StopFailure event (binary strings checked),
        // so `failed` cannot be derived for it.
        "UserPromptSubmit": [ { "hooks": [ { "type": "command", "command": notify } ] } ],
        "PermissionRequest": [ { "hooks": [ { "type": "command", "command": notify } ] } ],
        "Stop": [ { "hooks": [ { "type": "command", "command": notify } ] } ]
    })
}

/// grok 1.0.5 hook schema (its own docs, `~/.grok/docs/user-guide/10-hooks.md`,
/// verified live 2026-08-21: an isolated `GROK_HOME/hooks/*.json` loads as an
/// always-trusted "global" hook and fires). Payload keys are camelCase
/// (`hookEventName`, `toolName`, `sessionId`, `lastAssistantMessage`), event
/// VALUES snake_case (`user_prompt_submit`, `stop`). A `stop` fires once with
/// `reason: "end_turn"` for the turn AND once at session end (`"shutdown"`) —
/// the normalizer filters on the reason. An omitted matcher matches everything.
fn grok_hooks(notify: &str) -> Value {
    json!({
        "UserPromptSubmit": [ { "hooks": [ { "type": "command", "command": notify } ] } ],
        "PreToolUse":  [ { "hooks": [ { "type": "command", "command": notify } ] } ],
        "PostToolUse": [ { "hooks": [ { "type": "command", "command": notify } ] } ],
        "Stop": [ { "hooks": [ { "type": "command", "command": notify } ] } ],
        "StopFailure": [ { "hooks": [ { "type": "command", "command": notify } ] } ]
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
    let Some(home) = super::agents::home_dir(project_path, window_name) else { return false };
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
        // Channel drift is config drift too: old homes receive missing global
        // Bedrock keys without overwriting an explicit per-agent value.
        changed |= ensure_claude_env(&claude);
        // statusLine is app-owned observation plumbing, like hooks: every
        // managed Claude must paint the exact row the sniffer understands.
        changed |= ensure_claude_status_line(&claude);
        // Workspace trust is app-owned for managed agents: the user explicitly
        // spawned this isolated home into this project. Pre-seeding Claude's
        // documented project key avoids an interactive prompt nobody may be
        // watching; the keypress confirmer remains only as a legacy fallback.
        changed |= ensure_claude_state(&home, Path::new(project_path)).unwrap_or(false);
    }
    let codex = home.join("codex").join("hooks.json");
    if codex.is_file() {
        changed |= patch_hooks(&codex, codex_hooks(&notifications.helper_command("codex")));
    }
    let grok = home.join("hooks").join("tmux-mobile.json");
    if grok.is_file() {
        changed |= patch_hooks(&grok, grok_hooks(&notifications.helper_command("grok")));
    }
    // omp's hook is a generated FILE, not a key in someone else's config —
    // rewrite it whole when this build's text differs (helper path moves,
    // new events), same ownership rule as patch_hooks.
    let omp_ext = home.join("extensions").join("tmm-telemetry.ts");
    if omp_ext.is_file() {
        let fresh = omp_telemetry_extension(&notifications.helper_command("omp"));
        if std::fs::read_to_string(&omp_ext).ok().as_deref() != Some(fresh.as_str()) {
            changed |= std::fs::write(&omp_ext, fresh).is_ok();
        }
    }
    // Agents spawned before launch recipes existed can still be restarted with
    // full identity: for kiro the recipe is reconstructible from the isolated
    // home itself (env = KIRO_HOME, cmd = --agent <name>).
    if kiro.is_file() && !home.join("launch.json").exists() {
        // Best effort: a backfill that cannot be written changes nothing, and
        // the restart then takes the generic launch path it took before.
        changed |= write_launch_recipe(
            &home,
            "kiro",
            &[("KIRO_HOME".to_string(), home.to_string_lossy().to_string())],
            &format!(
                "command kiro-cli chat --agent {} --trust-all-tools kick",
                shared::shell_quote(window_name),
            ),
            // A backfilled recipe cannot know who spawned the agent — the
            // provenance simply does not exist for pre-recipe spawns.
            "", None)
        .is_ok();
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
        // MCP tool schemas are DEFERRED into a compact list and loaded on
        // demand via kiro's own tool_search (owner, 2026-08-28: "给 kiro 的
        // mcp 工具开启 toolsearch"). Thresholds 0/0 = defer whenever any MCP
        // tools are present, which is the progressive behavior asked for.
        ("toolSearch.enabled", json!(true)),
        ("toolSearch.minPct", json!(0)),
        ("toolSearch.minTokens", json!(0)),
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

/// ` --effort <level>` for the backends whose CLI takes the flag (kiro,
/// claude, grok — measured; codex takes a config override instead), or the
/// empty string. The value was validated against `models::effort_values` at
/// save time, so a typo cannot reach a launch line. Empty = backend default,
/// same contract as the model.
fn effort_flag(def: &RegAgent) -> String {
    let effort = def.effort.trim();
    if effort.is_empty() {
        String::new()
    } else {
        format!(" --effort {}", shared::shell_quote(effort))
    }
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
        // Native MCP again (owner, 2026-08-28: "mcp 工具还是用原生的方式调用
        // 吧") — the context cost is handled by toolSearch instead
        // (kiro_cli_settings enables it): schemas are DEFERRED into a compact
        // list and loaded on demand via kiro's own tool_search. The `tmm mcp`
        // CLI stays available as a SKILL, never taught in the prompt.
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
            "command kiro-cli chat --agent {} --trust-all-tools{}",
            shared::shell_quote(name),
            effort_flag(def),
        ),
        confirmation: None,
    })
}

fn claude_trust_key(workspace: &Path) -> PathBuf {
    let start = std::fs::canonicalize(workspace).unwrap_or_else(|_| workspace.to_path_buf());
    start
        .ancestors()
        .find(|path| path.join(".git").exists())
        .unwrap_or(&start)
        .to_path_buf()
}

/// Materialize onboarding + workspace trust in an isolated managed Claude home.
/// Claude's official permissions docs name this exact persisted shape. Merge,
/// never replace: `.claude.json` also owns session history, usage and UI state.
fn ensure_claude_state(home: &Path, workspace: &Path) -> Result<bool, String> {
    let path = home.join(".claude.json");
    let mut root = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let obj = root.as_object_mut().expect("filtered to object");
    let mut changed = !path.is_file();
    if obj.get("hasCompletedOnboarding") != Some(&json!(true)) {
        obj.insert("hasCompletedOnboarding".into(), json!(true));
        changed = true;
    }
    if !obj.contains_key("theme") {
        obj.insert("theme".into(), json!("dark"));
        changed = true;
    }
    if !obj.get("projects").is_some_and(Value::is_object) {
        obj.insert("projects".into(), json!({}));
        changed = true;
    }
    let trust_key = claude_trust_key(workspace).to_string_lossy().into_owned();
    let projects = obj.get_mut("projects").and_then(Value::as_object_mut).unwrap();
    let entry = projects.entry(trust_key).or_insert_with(|| json!({}));
    if !entry.is_object() {
        *entry = json!({});
        changed = true;
    }
    let project = entry.as_object_mut().unwrap();
    if project.get("hasTrustDialogAccepted") != Some(&json!(true)) {
        project.insert("hasTrustDialogAccepted".into(), json!(true));
        changed = true;
    }
    if changed {
        let text = format!("{}\n", serde_json::to_string_pretty(&root).unwrap());
        std::fs::write(&path, text).map_err(|e| format!("write {}: {e}", path.display()))?;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).map_err(|e| e.to_string())?.permissions().mode() & 0o777;
        if mode != 0o600 {
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| format!("chmod {}: {e}", path.display()))?;
            changed = true;
        }
    }
    Ok(changed)
}

fn render_claude(
    def: &RegAgent, _name: &str, home: &Path, workspace: &Path,
    system_prompt: &str, skills: &[crate::team::skills::ResolvedSkill],
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
    // The isolated home is the agent's CLAUDE_CONFIG_DIR (claude's KIRO_HOME:
    // history, session state and .claude.json live here, so a managed agent
    // never leaks into the user's ~/.claude). That relocation also means the
    // USER's settings layer is no longer read — so the channel config is
    // INHERITED: the `env` block of ~/.claude/settings.json (the Bedrock
    // switch: CLAUDE_CODE_USE_BEDROCK/AWS_REGION/ANTHROPIC_MODEL…) is copied
    // into the isolated settings.json, grok's "auth carries, prefs do not"
    // pattern in claude's dialect (owner, 2026-08-22: "都用bedrock渠道…复用
    // 我们全局定义的配置 但是自己管理好类似kirohome这种"). Plugins and
    // marketplaces deliberately do NOT carry.
    let settingsfile = home.join("settings.json");
    std::fs::write(
        &settingsfile,
        serde_json::to_string_pretty(&json!({
            "env": shared::claude_user_env(),
            "statusLine": shared::claude_status_line_config(),
            "skipDangerousModePermissionPrompt": true,
            "hooks": claude_hooks(&notify)
        }))
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    // A fresh CLAUDE_CONFIG_DIR otherwise parks at the theme/onboarding and
    // workspace-trust dialogs before the TUI. Claude documents the persisted
    // trust shape (`projects[repo_root].hasTrustDialogAccepted = true`); this
    // home exists only because the user explicitly spawned a managed agent in
    // this workspace, so materialize that decision before launching.
    ensure_claude_state(home, workspace)?;

    // Claude has no native skill mechanism — inject the compact index.
    let full_prompt = if skills.is_empty() {
        system_prompt.to_string()
    } else {
        format!("{}\n\n{}", system_prompt, crate::team::skills::skills_index_text(skills))
    };
    // The prompt is a FILE, like every other backend now (owner, 2026-09-08:
    // "类似的 Claude omp grok 是不是也是这种文件形式的，保证更加稳定"):
    // claude reads the user-memory `CLAUDE.md` from CLAUDE_CONFIG_DIR — this
    // isolated home — verified live (claude 2.1.239 quoted a marker from a
    // relocated dir's CLAUDE.md and obeyed its instruction). The old
    // `--append-system-prompt <6 KB literal>` was the last launch line bigger
    // than a send-keys burst; every start path is a short line now.
    std::fs::write(home.join("CLAUDE.md"), &full_prompt).map_err(|e| e.to_string())?;
    // An empty model means the BACKEND default — with Bedrock that is the
    // inherited env's ANTHROPIC_MODEL, so no `--model` is passed (the old
    // hardcoded `sonnet` alias overrode the env and does not resolve on
    // Bedrock). A configured model rides `--model`, which wins over env.
    let model_arg = if def.model.trim().is_empty() {
        String::new()
    } else {
        format!(" --model {}", shared::shell_quote(def.model.trim()))
    };
    Ok(Rendered {
        env: vec![("CLAUDE_CONFIG_DIR".into(), home.to_string_lossy().to_string())],
        cmd: format!(
            "command claude --mcp-config {} --strict-mcp-config --settings {}{}{} --dangerously-skip-permissions",
            shared::shell_quote(&mcpfile.to_string_lossy()),
            shared::shell_quote(&settingsfile.to_string_lossy()),
            model_arg,
            effort_flag(def),
        ),
        confirmation: Some(shared::StartupConfirmation {
            markers: shared::CLAUDE_FOLDER_TRUST_MARKERS.to_vec(),
            ready_markers: vec!["bypass permissions on"],
            accept_keys: vec!["Down", "Enter"],
            timeout: std::time::Duration::from_secs(120),
        }),
    })
}

/// Merge missing provider-channel keys into a managed Claude settings object.
/// Existing values are per-agent overrides and win; newly introduced global
/// keys (for example ANTHROPIC_DEFAULT_HAIKU_MODEL replacing the deprecated
/// small-fast key) still reach old homes on their next start.
fn merge_missing_claude_env(conf: &mut Value, inherited: &Value) -> bool {
    let Some(root) = conf.as_object_mut() else { return false };
    let Some(source) = inherited.as_object().filter(|env| !env.is_empty()) else { return false };
    if !root.get("env").is_some_and(Value::is_object) {
        root.insert("env".into(), Value::Object(source.clone()));
        return true;
    }
    let target = root.get_mut("env").and_then(Value::as_object_mut).unwrap();
    let mut changed = false;
    for (key, value) in source {
        if !target.contains_key(key) {
            target.insert(key.clone(), value.clone());
            changed = true;
        }
    }
    changed
}

/// Backfill a managed claude settings.json with missing inherited channel keys
/// (see `backends_shared::claude_user_env`). Fail-soft: refresh must never
/// block a start. Returns true when the file changed.
fn ensure_claude_env(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    let Ok(mut conf) = serde_json::from_str::<Value>(&text) else { return false };
    let inherited = shared::claude_user_env();
    if !merge_missing_claude_env(&mut conf, &inherited) {
        return false;
    }
    std::fs::write(path, serde_json::to_string_pretty(&conf).unwrap()).is_ok()
}

fn ensure_claude_status_line(path: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(path) else { return false };
    let Ok(mut conf) = serde_json::from_str::<Value>(&text) else { return false };
    let Some(root) = conf.as_object_mut() else { return false };
    let canonical = shared::claude_status_line_config();
    if root.get("statusLine") == Some(&canonical) {
        return false;
    }
    root.insert("statusLine".into(), canonical);
    std::fs::write(path, serde_json::to_string_pretty(&conf).unwrap()).is_ok()
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
    // The prompt is a FILE, not a launch argument (owner, 2026-09-08: "能类似
    // 通过 kiro 那样一个文件注入进去吗…这样更优雅一些"). codex reads the
    // global `AGENTS.md` from CODEX_HOME on every start, and this home is
    // ISOLATED — ours to write, never the user's (config.toml here is a
    // symlink into the user's real home; AGENTS.md is deliberately not
    // inherited). The old `-c developer_instructions=…` override put ~6 KB on
    // the launch line, which spawn survives (it sources a script) but the
    // restart replay typed into the pane — and a tty burst ≳2KB is exactly
    // what send-keys mangles (team/launch.rs). A restart also re-materializes
    // this file (refresh_agent), so the text stays current.
    std::fs::write(codex_home.join("AGENTS.md"), &full_prompt).map_err(|e| e.to_string())?;
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
    // Effort is a codex CONFIG key (`model_reasoning_effort`), so it rides a
    // `-c` override like the rest of codex's identity — the recipe replays it.
    if !def.effort.trim().is_empty() {
        config_args.push(shared::codex_config_override(
            "model_reasoning_effort",
            Value::String(def.effort.trim().to_string()),
        ));
    }
    config_args.push("--dangerously-bypass-approvals-and-sandbox".into());
    config_args.push("--dangerously-bypass-hook-trust".into());
    Ok(Rendered {
        env: vec![("CODEX_HOME".into(), codex_home.to_string_lossy().to_string())],
        cmd: format!("command codex {}", config_args.join(" ")),
        confirmation: Some(shared::StartupConfirmation {
            markers: shared::CODEX_FOLDER_TRUST_MARKERS.to_vec(),
            ready_markers: vec!["Starting MCP servers", "OpenAI Codex"],
            accept_keys: vec!["Enter"],
            timeout: std::time::Duration::from_secs(120),
        }),
    })
}

fn render_grok(
    def: &RegAgent, name: &str, home: &Path, system_prompt: &str,
    skills: &[crate::team::skills::ResolvedSkill],
) -> Result<Rendered, String> {
    std::fs::create_dir_all(home.join("agents")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(home.join("hooks")).map_err(|e| e.to_string())?;
    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    notifications.ensure_helper()?;
    let notify = notifications.helper_command("grok");

    // Telemetry hooks: `<GROK_HOME>/hooks/*.json` is that home's "global"
    // scope, always trusted — no folder-trust dance (verified live, grok
    // 1.0.5: loaded by `grok inspect`, fired on a real turn).
    std::fs::write(
        home.join("hooks").join("tmux-mobile.json"),
        serde_json::to_string_pretty(&json!({ "hooks": grok_hooks(&notify) })).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    // config.toml: folder trust off (the workspace is the user's own project,
    // spawned deliberately; an untrusted folder would gate project rules and
    // sit the TUI at a prompt nobody sees), MCP servers, and the USER's model
    // catalog carried over — grok auth is HOME-scoped (`auth.json` + custom
    // [model.*] entries whose keys ride env vars), so an isolated home without
    // the catalog is a logged-out agent (measured: "You are not authenticated").
    std::fs::write(home.join("config.toml"), grok_config_toml(&mcp_defs(def)))
        .map_err(|e| e.to_string())?;
    let user_auth = grok_user_home().join("auth.json");
    if user_auth.is_file() {
        let _ = std::fs::copy(&user_auth, home.join("auth.json"));
    }

    // The agent definition: kiro's pattern in grok's dialect — YAML
    // frontmatter + the system prompt as the body, selected via `--agent`.
    // The MODEL lives here, not on the launch line (same lesson as kiro:
    // verified that a frontmatter `model:` is honored, and it survives every
    // start path because they all pass --agent). Skills have no isolated-home
    // mechanism we control, so the compact index rides the prompt like claude.
    let full_prompt = if skills.is_empty() {
        system_prompt.to_string()
    } else {
        format!("{}\n\n{}", system_prompt, crate::team::skills::skills_index_text(skills))
    };
    let mut fm = format!("---\nname: {name}\ndescription: {} (registry agent)\n", def.name);
    let model = def.model.trim();
    if !model.is_empty() {
        fm.push_str(&format!("model: {model}\n"));
    }
    fm.push_str("---\n\n");
    std::fs::write(home.join("agents").join(format!("{name}.md")), format!("{fm}{full_prompt}"))
        .map_err(|e| e.to_string())?;

    Ok(Rendered {
        env: vec![("GROK_HOME".into(), home.to_string_lossy().to_string())],
        cmd: format!(
            "command grok --always-approve --agent {}{}",
            shared::shell_quote(name),
            effort_flag(def),
        ),
        confirmation: None,
    })
}

/// Where the user's own grok lives. Only read, never written.
fn grok_user_home() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".grok")
}

/// omp (oh-my-pi) 18.x. The whole agent state — auth store (`agent.db`),
/// `config.yml`, `mcp.json`, sessions, extensions — lives in ONE directory
/// that `PI_CODING_AGENT_DIR` relocates (its settings docs: "the global
/// config.yml, the auth store (agent.db), and everything else under the
/// agent directory move with it"; verified live 2026-09-07, omp 18.0.6: a
/// fresh dir answers prompts and grows its own agent.db). The isolated home
/// IS that directory:
///
/// * auth carry: the user's `~/.omp/agent/agent.db` is the credential store
///   (`auth_credentials` table), so it is copied in — grok's auth.json
///   lesson; env-keyed providers (Bedrock bearer tokens) need nothing.
/// * the MODEL lives in `<home>/config.yml` under `modelRoles.default`
///   (identity in config, never the launch line); EFFORT rides `--thinking`,
///   omp's own knob, enumerated in `models::effort_values`.
/// * the system prompt is a FILE handed to `--append-system-prompt` —
///   APPEND, deliberately not `--system-prompt`: omp's builtin prompt
///   teaches its own tool harness (hashline edits, LSP, subagents), and
///   replacing it would lobotomize the tools. (Verified live: a file path
///   is read and its text reaches the prompt.)
/// * telemetry: omp auto-loads TS extensions from `<agent-dir>/extensions/`
///   (its extension-loading docs; the path honors PI_CODING_AGENT_DIR). The
///   generated hook forwards turn edges and tool events to the tmm notify
///   helper in claude's payload dialect, so the normalizer needs no new
///   shapes — only the `omp` backend arm.
fn render_omp(
    def: &RegAgent, _name: &str, home: &Path, system_prompt: &str,
    skills: &[crate::team::skills::ResolvedSkill],
) -> Result<Rendered, String> {
    std::fs::create_dir_all(home.join("extensions")).map_err(|e| e.to_string())?;
    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    notifications.ensure_helper()?;
    std::fs::write(
        home.join("extensions").join("tmm-telemetry.ts"),
        omp_telemetry_extension(&notifications.helper_command("omp")),
    )
    .map_err(|e| e.to_string())?;

    // Auth carry: agent.db holds `auth_credentials` — an isolated home
    // without it is a logged-out agent for OAuth-keyed providers. Best
    // effort, like grok's auth.json copy.
    let user_db = omp_user_agent_dir().join("agent.db");
    if user_db.is_file() {
        let _ = std::fs::copy(&user_db, home.join("agent.db"));
    }
    // Model-catalog carry (grok's config.toml lesson, in omp's dialect): the
    // user's `models.yml` declares the custom providers/models the bundled
    // catalog lacks — on this host the Bedrock trio (fable-5-1/opus/gpt-5.6)
    // under `bedrock-extra` — and a registry def may name exactly such a
    // model. An isolated home without the catalog cannot resolve it, so the
    // file carries; UI prefs and hooks deliberately do NOT (that is what
    // isolation is for).
    let user_models = omp_user_agent_dir().join("models.yml");
    if user_models.is_file() {
        let _ = std::fs::copy(&user_models, home.join("models.yml"));
    }

    // config.yml: the model is identity, so it lives in the config the owner
    // can read, not on the launch line (kiro's lesson). Empty = omp default.
    let model = def.model.trim();
    if !model.is_empty() {
        std::fs::write(home.join("config.yml"), format!("modelRoles:\n  default: {model}\n"))
            .map_err(|e| e.to_string())?;
    }

    // mcp.json in this home's user scope — claude's local-stdio/http shape
    // is omp's too (its mcp-config docs name `~/.omp/agent/mcp.json`).
    let mut servers = serde_json::Map::new();
    for m in &mcp_defs(def) {
        if !m.name.is_empty() {
            servers.insert(m.name.clone(), shared::claude_mcp_value(m));
        }
    }
    if !servers.is_empty() {
        std::fs::write(
            home.join("mcp.json"),
            serde_json::to_string_pretty(&json!({ "mcpServers": servers })).unwrap(),
        )
        .map_err(|e| e.to_string())?;
    }

    // The prompt file --append-system-prompt reads. Skills have no isolated-
    // home mechanism we control, so the compact index rides the prompt, like
    // grok and claude.
    let full_prompt = if skills.is_empty() {
        system_prompt.to_string()
    } else {
        format!("{}\n\n{}", system_prompt, crate::team::skills::skills_index_text(skills))
    };
    let prompt_path = home.join("system-prompt.md");
    std::fs::write(&prompt_path, full_prompt).map_err(|e| e.to_string())?;

    let effort = def.effort.trim();
    let thinking = if effort.is_empty() {
        String::new()
    } else {
        format!(" --thinking {}", shared::shell_quote(effort))
    };
    Ok(Rendered {
        env: vec![("PI_CODING_AGENT_DIR".into(), home.to_string_lossy().to_string())],
        cmd: format!(
            "command omp --auto-approve --append-system-prompt {}{}",
            shared::shell_quote(&prompt_path.to_string_lossy()),
            thinking,
        ),
        confirmation: None,
    })
}

/// Where the user's own omp agent state lives. Only read, never written.
fn omp_user_agent_dir() -> PathBuf {
    std::env::var_os("HOME").map(PathBuf::from).unwrap_or_default().join(".omp").join("agent")
}

/// The telemetry extension a managed omp home carries: a hook factory (omp's
/// documented extension shape — default export receiving the `pi` API) that
/// forwards turn edges and tool events to the notify helper on stdin, in
/// claude's payload dialect. Every handler is wrapped in try/catch and the
/// child ignores errors: telemetry must never break the agent. TMUX_PANE
/// reaches the helper because the child inherits omp's env, which inherits
/// the pane's.
fn omp_telemetry_extension(notify: &str) -> String {
    // The helper command is a shell line (quoted path + backend + marker
    // comment); embed it as a JSON string literal, which is also a valid TS
    // string literal.
    let cmd = serde_json::to_string(notify).unwrap();
    format!(
        r#"// tmux-mobile telemetry hook — auto-generated, rewritten on every start
// (refresh_hooks). Forwards turn edges and tool events to the tmm notify
// helper, which is a no-op outside a tmux pane. Do not edit.
import {{ spawn }} from "node:child_process";

const NOTIFY = {cmd};

function send(payload: Record<string, unknown>): void {{
  try {{
    const child = spawn("/bin/sh", ["-c", NOTIFY], {{ stdio: ["pipe", "ignore", "ignore"] }});
    child.on("error", () => {{}});
    child.stdin?.write(JSON.stringify(payload));
    child.stdin?.end();
  }} catch {{}}
}}

function sessionId(ctx: any): string {{
  try {{ return String(ctx?.sessionManager?.getSessionId?.() ?? ""); }} catch {{ return ""; }}
}}

function textOf(content: unknown): string {{
  if (typeof content === "string") return content;
  if (Array.isArray(content)) {{
    return content.map((c: any) => (typeof c?.text === "string" ? c.text : "")).join("");
  }}
  return "";
}}

export default function hook(pi: any): void {{
  // agent_start fires once per user prompt, but the prompt itself is not in
  // the session branch yet at that moment (measured, omp 18.0.6: the branch
  // tail is still model/thinking entries). The FIRST context event after it
  // carries the messages bound for the model — the newest user message there
  // is the submitted prompt, the delivery receipt tmm acks against.
  let awaitingPrompt = false;
  pi.on("agent_start", async (_event: any, _ctx: any) => {{
    awaitingPrompt = true;
  }});
  pi.on("context", async (event: any, ctx: any) => {{
    if (!awaitingPrompt) return;
    awaitingPrompt = false;
    let prompt = "";
    try {{
      const messages = event?.messages ?? [];
      for (let i = messages.length - 1; i >= 0; i--) {{
        const m = messages[i] as any;
        if (m?.role === "user") {{ prompt = textOf(m.content); break; }}
      }}
    }} catch {{}}
    send({{ hook_event_name: "UserPromptSubmit", prompt, session_id: sessionId(ctx) }});
  }});
  // agent_end with willContinue is an auto-continuation, not a settle.
  pi.on("agent_end", async (event: any, ctx: any) => {{
    if (event?.willContinue) return;
    awaitingPrompt = false;
    let reply = "";
    try {{
      const messages = event?.messages ?? [];
      for (let i = messages.length - 1; i >= 0; i--) {{
        const m = messages[i] as any;
        if (m?.role === "assistant") {{ reply = textOf(m.content); break; }}
      }}
    }} catch {{}}
    send({{ hook_event_name: "Stop", last_assistant_message: reply, session_id: sessionId(ctx) }});
  }});
  pi.on("tool_call", async (event: any, ctx: any) => {{
    send({{ hook_event_name: "PreToolUse", tool_name: event?.toolName ?? "tool", tool_input: event?.input ?? {{}}, session_id: sessionId(ctx) }});
  }});
  pi.on("tool_result", async (event: any, ctx: any) => {{
    send({{ hook_event_name: "PostToolUse", tool_name: event?.toolName ?? "tool", session_id: sessionId(ctx) }});
  }});
}}
"#
    )
}


/// The isolated home's config.toml: folder-trust off, the user's model
/// catalog (`[models]` + `[model.*]` — the auth-bearing half of grok config;
/// hooks/MCP/UI prefs deliberately do NOT carry, that is what isolation is
/// for), and the registry MCP servers in grok's `[mcp_servers.<name>]` shape.
fn grok_config_toml(mcps: &[shared::McpDef]) -> String {
    let user = std::fs::read_to_string(grok_user_home().join("config.toml")).ok();
    grok_config_toml_from(mcps, user.as_deref())
}

/// The pure half, so the catalog carry is testable. TRAP, already paid for
/// once: toml 1.x parses a DOCUMENT via `toml::Table` — `Value::from_str`
/// parses a single value and fails on any real config with "expected nothing",
/// which silently dropped the whole catalog and left every spawned grok at a
/// login screen (caught live, 2026-08-21).
fn grok_config_toml_from(mcps: &[shared::McpDef], user_config: Option<&str>) -> String {
    let mut root = toml::value::Table::new();
    let mut trust = toml::value::Table::new();
    trust.insert("enabled".into(), toml::Value::Boolean(false));
    root.insert("folder_trust".into(), toml::Value::Table(trust));
    if let Some(user) = user_config.and_then(|t| t.parse::<toml::Table>().ok()) {
        for key in ["models", "model"] {
            if let Some(v) = user.get(key) {
                root.insert(key.into(), v.clone());
            }
        }
    }
    let mut servers = toml::value::Table::new();
    for m in mcps {
        let Some(cmd) = m.command.as_deref().filter(|c| !c.is_empty()) else { continue };
        if m.name.is_empty() {
            continue;
        }
        let mut t = toml::value::Table::new();
        t.insert("command".into(), toml::Value::String(cmd.to_string()));
        if !m.args.is_empty() {
            t.insert(
                "args".into(),
                toml::Value::Array(m.args.iter().map(|a| toml::Value::String(a.clone())).collect()),
            );
        }
        if !m.env.is_empty() {
            let mut env = toml::value::Table::new();
            for (k, v) in &m.env {
                env.insert(k.clone(), toml::Value::String(v.clone()));
            }
            t.insert("env".into(), toml::Value::Table(env));
        }
        servers.insert(m.name.clone(), toml::Value::Table(t));
    }
    if !servers.is_empty() {
        root.insert("mcp_servers".into(), toml::Value::Table(servers));
    }
    let body = toml::to_string(&root).unwrap_or_default();
    format!("# Written by tmux-mobile — regenerated at every spawn.\n{body}")
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
            effort: String::new(),
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
        let r = render_kiro(&d, "tester", &dir, &build_prompt(&d, "tester", "proj", "fix the bug", "lead", ""), &[]).unwrap();
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
        assert!(!prompt.contains("fix the bug"), "brief is delivered as the first user message");
        // The prompt must NOT carry a date: it is replayed every time the window
        // is restored, so a baked-in "today" becomes a lie. The KICK carries it.
        let year = chrono::Local::now().format("%Y").to_string();
        assert!(!prompt.contains(&year), "no wall-clock date in a replayed prompt");
        // NOTHING is sent to an agent that was spawned without a brief: an
        // invented first prompt is a message the user never wrote, and it made
        // agents reason about nothing (owner, 2026-08-18).
        assert!(first_prompt("", "").is_none(), "no brief, no prompt");
        assert!(first_prompt("   ", "").is_none(), "whitespace is not a brief");
        // A brief IS something to consume: delivered as the first message,
        // stamped like every later one.
        let p = first_prompt("fix the flaky test", "lead").unwrap();
        assert!(p.starts_with(&format!("[tmm chat {year}-")), "a delivered brief is stamped: {p}");
        assert!(p.contains("] lead: "), "the reply edge names the briefer: {p}");
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
        // MCP schemas defer into kiro's own tool_search — always (0/0), so a
        // big tool set never floods the context (owner, 2026-08-28).
        assert_eq!(cli.get("toolSearch.enabled").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(cli.get("toolSearch.minPct").and_then(|v| v.as_u64()), Some(0));
        assert_eq!(cli.get("toolSearch.minTokens").and_then(|v| v.as_u64()), Some(0));
        // A restart must replay the FULL identity, not the bare backend line:
        // the user-space config's hooks never fire (measured), so losing
        // KIRO_HOME/--agent makes a restarted agent observably deaf.
        write_launch_recipe(&dir, "kiro", &r.env, &r.cmd, "", None).unwrap();
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

    #[test]
    fn refresh_hooks_backfills_claude_status_line() {
        let ws = std::env::temp_dir().join(format!("tmm-cc-status-{}", uuid::Uuid::new_v4()));
        let home = ws.join(".tmm/agents/cc");
        std::fs::create_dir_all(&home).unwrap();
        let settings = home.join("settings.json");
        std::fs::write(&settings, serde_json::to_string_pretty(&json!({
            "env": { "ANTHROPIC_MODEL": "agent-override" },
            "hooks": {},
            "theme": "dark"
        })).unwrap()).unwrap();

        assert!(refresh_hooks(&ws.to_string_lossy(), "cc"));
        let read = || -> Value {
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap()
        };
        let after = read();
        assert_eq!(after["statusLine"], shared::claude_status_line_config());
        assert_eq!(after["env"]["ANTHROPIC_MODEL"], "agent-override");
        assert_eq!(after["theme"], "dark");
        assert!(!refresh_hooks(&ws.to_string_lossy(), "cc"), "canonical home is a no-op");
        std::fs::remove_dir_all(&ws).ok();
    }

    /// The grok backend, aligned with kiro (owner, 2026-08-21): isolated
    /// GROK_HOME, identity via `--agent`, MODEL in the definition not on the
    /// line, telemetry hooks in the home's always-trusted hooks dir, MCP +
    /// folder-trust-off in config.toml. All shapes verified live on grok 1.0.5
    /// (an isolated home loaded the agent + hooks and answered a real turn).
    #[test]
    fn grok_home_is_isolated_and_wired_to_tmm() {
        let dir = std::env::temp_dir().join(format!("tmm-spawn-grok-{}", uuid::Uuid::new_v4()));
        let mut d = def("grok");
        d.model = "grok-4.6".into();
        let r = render_grok(&d, "tester", &dir, &build_prompt(&d, "tester", "proj", "fix the bug", "lead", ""), &[]).unwrap();
        assert!(r.env.iter().any(|(k, v)| k == "GROK_HOME" && v.contains("tmm-spawn-grok")), "home must be the isolated dir");
        assert!(r.cmd.contains("--agent tester"), "identity via --agent: {}", r.cmd);
        assert!(r.cmd.contains("--always-approve"), "no interactive permission prompts: {}", r.cmd);
        assert!(!r.cmd.contains("--model"), "the model lives in the definition, not the line: {}", r.cmd);

        let agent_md = std::fs::read_to_string(dir.join("agents/tester.md")).unwrap();
        assert!(agent_md.starts_with("---\nname: tester\n"), "frontmatter first: {agent_md}");
        assert!(agent_md.contains("model: grok-4.6"), "model pinned in frontmatter");
        assert!(agent_md.contains("tmm send"), "the tmm paragraph IS the integration");
        assert!(!agent_md.contains("fix the bug"), "brief is delivered as the first user message");

        let hooks: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("hooks/tmux-mobile.json")).unwrap()).unwrap();
        let h = hooks.get("hooks").unwrap();
        for ev in ["UserPromptSubmit", "PreToolUse", "PostToolUse", "Stop", "StopFailure"] {
            assert!(h.get(ev).is_some(), "grok hook set must carry {ev}");
        }

        let cfg = std::fs::read_to_string(dir.join("config.toml")).unwrap();
        assert!(cfg.contains("[folder_trust]") && cfg.contains("enabled = false"),
            "trust gate off so the TUI never parks at a prompt nobody sees: {cfg}");
        assert!(cfg.contains("[mcp_servers.files]") && cfg.contains("mcp-files"),
            "registry MCP def must materialize in grok's dialect: {cfg}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn omp_home_is_isolated_and_wired_to_tmm() {
        let dir = std::env::temp_dir().join(format!("tmm-spawn-omp-{}", uuid::Uuid::new_v4()));
        let mut d = def("omp");
        d.model = "anthropic/claude-opus-4-6".into();
        d.effort = "high".into();
        let r = render_omp(&d, "tester", &dir, &build_prompt(&d, "tester", "proj", "fix the bug", "lead", ""), &[]).unwrap();
        assert!(
            r.env.iter().any(|(k, v)| k == "PI_CODING_AGENT_DIR" && v.contains("tmm-spawn-omp")),
            "the agent dir must be the isolated home"
        );
        assert!(r.cmd.contains("--auto-approve"), "no interactive approval prompts: {}", r.cmd);
        assert!(r.cmd.contains("--append-system-prompt"), "prompt rides a file, APPENDED: {}", r.cmd);
        assert!(r.cmd.contains("--thinking high"), "effort is omp's own knob: {}", r.cmd);
        assert!(!r.cmd.contains("--model"), "the model lives in config.yml, not the line: {}", r.cmd);
        assert!(!r.cmd.contains("--system-prompt "), "never REPLACE omp's builtin prompt: {}", r.cmd);

        let prompt = std::fs::read_to_string(dir.join("system-prompt.md")).unwrap();
        assert!(prompt.contains("tmm send"), "the tmm paragraph IS the integration");
        assert!(!prompt.contains("fix the bug"), "brief is delivered as the first user message");

        let cfg = std::fs::read_to_string(dir.join("config.yml")).unwrap();
        assert!(cfg.contains("default: anthropic/claude-opus-4-6"), "model pinned in modelRoles: {cfg}");

        let mcp: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("mcp.json")).unwrap()).unwrap();
        assert!(mcp["mcpServers"]["files"]["command"].as_str() == Some("mcp-files"),
            "registry MCP def must materialize in omp's mcp.json: {mcp}");

        let ext = std::fs::read_to_string(dir.join("extensions/tmm-telemetry.ts")).unwrap();
        for needle in ["export default function hook", "UserPromptSubmit", "Stop", "PreToolUse", "PostToolUse", "willContinue"] {
            assert!(ext.contains(needle), "telemetry extension must carry {needle}");
        }
        assert!(ext.contains(" omp #"), "the helper is called with the omp backend tag");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// No model, no effort: the config stays absent (omp default) and the
    /// launch line carries no --thinking.
    #[test]
    fn omp_defaults_leave_config_and_thinking_off() {
        let dir = std::env::temp_dir().join(format!("tmm-spawn-omp-def-{}", uuid::Uuid::new_v4()));
        let d = def("omp");
        let r = render_omp(&d, "t2", &dir, "prompt", &[]).unwrap();
        assert!(!r.cmd.contains("--thinking"), "{}", r.cmd);
        assert!(!dir.join("config.yml").exists(), "empty model writes no config.yml");
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The model catalog is what makes an ISOLATED grok home able to answer at
    /// all: grok auth is home-scoped, and custom [model.*] entries carry the
    /// api key wiring (env_key). The first cut parsed the user config with
    /// `toml::Value::from_str`, which parses a single VALUE — every real
    /// config failed with "expected nothing" and the catalog silently
    /// vanished, leaving the spawned agent at a login screen (live, 2026-08-21).
    #[test]
    fn grok_config_carries_the_user_model_catalog() {
        let user = r#"
[models]
default = "bedrock-x"

[model.bedrock-x]
model = "us.xai.grok-4.6"
base_url = "https://example.com/v1"
env_key = "SOME_TOKEN_VAR"

[ui]
yolo = false

[[hooks.PreToolUse]]
matcher = "Bash"
hooks = [ { type = "command", command = "/opt/guard.sh" } ]
"#;
        let cfg = grok_config_toml_from(&[], Some(user));
        assert!(cfg.contains("[models]") && cfg.contains("default = \"bedrock-x\""), "catalog default: {cfg}");
        assert!(cfg.contains("[model.bedrock-x]") && cfg.contains("env_key"), "custom model with key wiring: {cfg}");
        // Isolation is the point: user hooks/UI prefs must NOT leak in.
        assert!(!cfg.contains("guard.sh") && !cfg.contains("[ui]"), "only the catalog carries: {cfg}");
        assert!(cfg.contains("enabled = false"), "folder trust off");
        // No user config at all still renders a valid file.
        let bare = grok_config_toml_from(&[], None);
        assert!(bare.contains("[folder_trust]"));
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
            "", None).unwrap();

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
            "", None).unwrap();

        refresh_hooks(&ws.to_string_lossy(), "dev");
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&cfg).unwrap()).unwrap();
        assert!(after.get("model").is_none(), "a rejected id must not reach the config: {after}");
        let recipe: Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("launch.json")).unwrap()).unwrap();
        assert!(!recipe["cmd"].as_str().unwrap().contains("--model"), "the line is cleaned up either way");
        std::fs::remove_dir_all(&ws).ok();
    }

    /// The workspace mcp.json is the AGENT's file: the registry seeds missing
    /// servers, an existing entry always wins (an agent-edited command must
    /// survive every respawn), and unknown entries are kept.
    #[test]
    fn mcp_config_seeds_and_never_clobbers() {
        let ws = std::env::temp_dir().join(format!("tmm-mcpseed-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&ws).unwrap();
        let mk = |name: &str, cmd: &str| -> shared::McpDef {
            serde_json::from_value(json!({ "name": name, "command": cmd })).unwrap()
        };
        // First spawn seeds.
        let path = seed_mcp_config(&ws, &[mk("files", "mcp-files")]).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["files"]["command"], json!("mcp-files"));
        // The agent edits the entry and adds its own server.
        std::fs::write(&path, serde_json::to_string(&json!({ "mcpServers": {
            "files": { "command": "my-forked-files" },
            "mine":  { "command": "hand-added" },
        }})).unwrap()).unwrap();
        // A later spawn adds the new def but touches NOTHING the agent wrote.
        seed_mcp_config(&ws, &[mk("files", "mcp-files"), mk("web", "mcp-web")]).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["mcpServers"]["files"]["command"], json!("my-forked-files"), "agent edit survives");
        assert_eq!(v["mcpServers"]["mine"]["command"], json!("hand-added"), "agent's own server kept");
        assert_eq!(v["mcpServers"]["web"]["command"], json!("mcp-web"), "new def seeded");
        std::fs::remove_dir_all(&ws).ok();
    }

    #[test]
    fn claude_channel_backfill_adds_new_keys_without_stomping_overrides() {
        let mut managed = json!({
            "env": {
                "CLAUDE_CODE_USE_BEDROCK": "1",
                "ANTHROPIC_MODEL": "agent-specific-model"
            },
            "hooks": {}
        });
        let global = json!({
            "CLAUDE_CODE_USE_BEDROCK": "1",
            "ANTHROPIC_MODEL": "global-model",
            "ANTHROPIC_DEFAULT_HAIKU_MODEL": "global-haiku"
        });
        assert!(merge_missing_claude_env(&mut managed, &global));
        assert_eq!(managed["env"]["ANTHROPIC_MODEL"], "agent-specific-model");
        assert_eq!(managed["env"]["ANTHROPIC_DEFAULT_HAIKU_MODEL"], "global-haiku");
        assert!(!merge_missing_claude_env(&mut managed, &global), "second refresh is a no-op");
    }

    #[test]
    fn claude_state_pretrusts_the_git_root_without_clobbering_session_data() {
        let root = std::env::temp_dir().join(format!("tmm-cc-trust-{}", uuid::Uuid::new_v4()));
        let workspace = root.join("repo/subdir");
        let home = root.join("home");
        std::fs::create_dir_all(root.join("repo/.git")).unwrap();
        std::fs::create_dir_all(&workspace).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let state = home.join(".claude.json");
        std::fs::write(&state, r#"{"userID":"keep","projects":{"/other":{"lastSessionId":"abc"}}}"#).unwrap();

        assert!(ensure_claude_state(&home, &workspace).unwrap());
        let after: Value = serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
        let repo = std::fs::canonicalize(root.join("repo")).unwrap().to_string_lossy().into_owned();
        assert_eq!(after["projects"][repo]["hasTrustDialogAccepted"], json!(true));
        assert_eq!(after["projects"]["/other"]["lastSessionId"], "abc");
        assert_eq!(after["userID"], "keep");
        assert_eq!(after["hasCompletedOnboarding"], json!(true));
        assert!(!ensure_claude_state(&home, &workspace).unwrap(), "canonical state is a no-op");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&state).unwrap().permissions().mode() & 0o777, 0o600);
        }
        std::fs::remove_dir_all(&root).ok();
    }

    #[test]
    fn claude_and_codex_render_without_team_plumbing() {
        for backend in ["claude", "codex"] {
            let dir = std::env::temp_dir().join(format!("tmm-spawn-{backend}-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(&dir).unwrap();
            let d = def(backend);
            let prompt = build_prompt(&d, "tester", "proj", "", "", "");
            let r = match backend {
                "claude" => render_claude(&d, "tester", &dir, &dir, &prompt, &[]).unwrap(),
                _ => render_codex(&d, "tester", &dir, &prompt, &[]).unwrap(),
            };
            assert!(!r.cmd.contains("x-room"), "no team room headers");
            assert!(r.cmd.contains("files") || dir.join("mcp.json").exists(), "registry MCP present");
            // The prompt does NOT teach `tmm mcp` — the CLI door is a SKILL
            // an agent opts into, never the native path (owner, 2026-08-28).
            assert!(!prompt.contains("tmm mcp"), "prompt must not teach the MCP CLI");
            if backend == "codex" {
                // The prompt is a FILE in the isolated CODEX_HOME (owner,
                // 2026-09-08), not a launch argument: the ~6 KB
                // developer_instructions override made the recipe line bigger
                // than a tty can take in one send-keys burst (≳2KB), which is
                // exactly how the restart replay mangled it. The launch line
                // must stay under that budget.
                let agents_md = std::fs::read_to_string(dir.join("codex").join("AGENTS.md")).unwrap();
                assert_eq!(agents_md, prompt, "the whole prompt lands in AGENTS.md");
                assert!(!r.cmd.contains("developer_instructions"), "{}", r.cmd);
                assert!(r.cmd.len() < 2000, "launch line must fit one send-keys burst: {} bytes", r.cmd.len());
            }
            if backend == "claude" {
                // The isolated home is claude's KIRO_HOME (measured on claude
                // 2.1.239: CLAUDE_CONFIG_DIR relocates state AND the user
                // settings layer, so the channel env is inherited into the
                // isolated settings.json instead).
                assert!(
                    r.env.iter().any(|(k, v)| k == "CLAUDE_CONFIG_DIR" && v == &dir.to_string_lossy()),
                    "isolated config dir: {:?}", r.env
                );
                let settings: Value = serde_json::from_str(
                    &std::fs::read_to_string(dir.join("settings.json")).unwrap()
                ).unwrap();
                assert!(settings.get("env").is_some_and(Value::is_object), "inherited channel env");
                assert_eq!(settings["statusLine"], shared::claude_status_line_config());
                assert_eq!(settings["skipDangerousModePermissionPrompt"], json!(true));
                // A fresh config dir parks at the theme onboarding without this.
                let state: Value = serde_json::from_str(
                    &std::fs::read_to_string(dir.join(".claude.json")).unwrap()
                ).unwrap();
                assert_eq!(state["hasCompletedOnboarding"], json!(true));
                let trust_key = std::fs::canonicalize(&dir).unwrap().to_string_lossy().into_owned();
                assert_eq!(state["projects"][trust_key]["hasTrustDialogAccepted"], json!(true));
                // def() has no model → the BACKEND default (the inherited
                // env's ANTHROPIC_MODEL) decides; the old `--model sonnet`
                // alias overrode it and does not resolve on Bedrock.
                assert!(!r.cmd.contains("--model"), "{}", r.cmd);
                // The prompt is a FILE (owner, 2026-09-08): CLAUDE.md in the
                // isolated CLAUDE_CONFIG_DIR — verified live that a relocated
                // dir's CLAUDE.md is read and obeyed. The launch line was the
                // last one bigger than a send-keys burst; keep it under.
                let claude_md = std::fs::read_to_string(dir.join("CLAUDE.md")).unwrap();
                assert_eq!(claude_md, prompt, "the whole prompt lands in CLAUDE.md");
                assert!(!r.cmd.contains("--append-system-prompt"), "{}", r.cmd);
                assert!(r.cmd.len() < 2000, "launch line must fit one send-keys burst: {} bytes", r.cmd.len());
            }
            std::fs::remove_dir_all(&dir).ok();
        }
    }

    /// Effort rides each backend's own knob (owner, 2026-08-22: "agent配置里
    /// 应该有thinking effort的配置选项"): a `--effort` flag for kiro/claude/
    /// grok (measured on each CLI), a `-c model_reasoning_effort=…` config
    /// override for codex. Empty = the backend default, nothing on the line.
    #[test]
    fn effort_reaches_each_backend_in_its_own_dialect() {
        let dir = std::env::temp_dir().join(format!("tmm-effort-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut d = def("kiro");
        d.effort = "high".into();
        let prompt = build_prompt(&d, "t", "p", "", "", "");
        assert!(render_kiro(&d, "t", &dir, &prompt, &[]).unwrap().cmd.ends_with("--effort high"));
        d.backend = "claude".into();
        assert!(render_claude(&d, "t", &dir, &dir, &prompt, &[]).unwrap().cmd.contains(" --effort high "));
        d.backend = "grok".into();
        assert!(render_grok(&d, "t", &dir, &prompt, &[]).unwrap().cmd.ends_with("--effort high"));
        d.backend = "codex".into();
        let cx = render_codex(&d, "t", &dir, &prompt, &[]).unwrap().cmd;
        assert!(cx.contains("model_reasoning_effort=\\\"high\\\"") || cx.contains("model_reasoning_effort=\"high\""), "{cx}");
        // Empty effort leaves every line clean.
        d.effort = String::new();
        d.backend = "kiro".into();
        assert!(!render_kiro(&d, "t", &dir, &prompt, &[]).unwrap().cmd.contains("--effort"));
        // Validation is a fixed enum per backend; empty always passes.
        assert!(super::super::models::validate_effort("kiro", "xhigh").is_ok());
        assert!(super::super::models::validate_effort("codex", "minimal").is_ok());
        assert!(super::super::models::validate_effort("grok", "max").is_err(), "grok has no max");
        assert!(super::super::models::validate_effort("claude", "ultra").is_err());
        assert!(super::super::models::validate_effort("claude", "").is_ok());
        std::fs::remove_dir_all(&dir).ok();
    }

    /// The turn-start hook is the ONLY reset of the same-turn dedup flag and
    /// the only carrier of the submitted prompt (the delivery receipt), so it
    /// must be registered in EVERY backend's hook set. Claude and codex
    /// shipped without it: their agents lost the stop-hook auto-post forever
    /// after their first `tmm send`, and every delivered line stayed
    /// "unconfirmed" (owner, 2026-08-22: 特性对齐). Codex payload measured on
    /// codex-cli 0.148.0; claude's documented schema is the same family.
    #[test]
    fn every_backend_hook_set_registers_the_turn_start_hook() {
        assert!(kiro_hooks("n")["userPromptSubmit"].is_array(), "kiro");
        assert!(claude_hooks("n")["UserPromptSubmit"].is_array(), "claude");
        assert!(codex_hooks("n")["UserPromptSubmit"].is_array(), "codex");
        assert!(grok_hooks("n")["UserPromptSubmit"].is_array(), "grok");
        // And every set still ends turns: a stop hook.
        assert!(kiro_hooks("n")["stop"].is_array());
        for f in [claude_hooks, codex_hooks, grok_hooks] {
            assert!(f("n")["Stop"].is_array());
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
    fn global_instructions_lead_the_prompt_on_every_backend() {
        let d = def("kiro");
        let p = build_prompt(&d, "b", "proj", "", "", "# House rules\nAnswer in Chinese.\n");
        assert!(p.starts_with("# House rules\nAnswer in Chinese.\n\nPersona text."), "global block first, then the persona: {p}");
        assert!(p.contains("agent \"b\" in project \"proj\""));
        // Absent → the prompt is exactly what it was before the feature.
        assert_eq!(build_prompt(&d, "b", "proj", "", "", "  "), build_prompt(&d, "b", "proj", "", "", ""));
        assert!(build_prompt(&d, "b", "proj", "", "", "").starts_with("Persona text."));
    }

    #[test]
    fn prompt_carries_identity_project_and_rules() {
        let d = def("kiro");
        let p = build_prompt(&d, "rev-2", "blog", "review the branch", "lead", "");
        assert!(p.starts_with("Persona text."));
        assert!(p.contains("agent \"rev-2\" in project \"blog\""));
        assert!(!p.contains("tmm done"));
        assert!(!p.contains("tmm status"));
        assert!(!p.contains("review the branch"), "brief is a real first message: {p}");
        assert!(p.contains("[tmm chat YYYY-MM-DD HH:MM]"), "prompt explains message stamps: {p}");
        assert!(p.contains("final response"), "explains automatic replies: {p}");
        assert!(p.contains("delivered back to that agent"), "explains the reply edge: {p}");
        assert!(p.contains("--status"), "explains ambient progress: {p}");
        assert!(p.contains("@human"), "names the operator address: {p}");
        assert!(p.contains("tmm log --limit 50"), "points at the history: {p}");
        assert!(p.contains("Read a queued backlog in full"), "consolidates backlog answers: {p}");
        assert!(p.contains("tmm board move <id> review"), "explains board handoff: {p}");
    }

    #[test]
    fn first_prompt_names_the_briefer_and_recipe_keeps_provenance() {
        let d = def("kiro");
        let p = build_prompt(&d, "b", "proj", "fix it", "lead", "");
        assert!(!p.contains("fix it"), "brief is not duplicated into the system prompt");
        let first = first_prompt("fix it", "lead").unwrap();
        assert!(first.contains("] lead: fix it"), "the hook can recover the reply edge: {first}");

        // The recipe retains who created the slot as provenance across refresh.
        let ws = std::env::temp_dir().join(format!("tmm-spawnedby-{}", std::process::id()));
        let home = ws.join(".tmm").join("agents").join("b");
        std::fs::create_dir_all(&home).unwrap();
        write_launch_recipe(&home, "kiro", &[], "command kiro-cli chat --agent b", "lead", None).unwrap();
        let recipe: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(home.join("launch.json")).unwrap()).unwrap();
        assert_eq!(recipe["spawned_by"], "lead");
        std::fs::remove_dir_all(&ws).ok();
    }

    /// Restart means "come back up to date": the recipe replay is verbatim, so
    /// `refresh_agent` re-materializes the home from the CURRENT def first
    /// (owner, 2026-09-08: restarting an agent did NOT pick up new prompt
    /// text — global_prompt.rs promised it would). Spawner provenance survives,
    /// while the original brief does not (the conversation is resumed).
    #[test]
    fn refresh_agent_rematerializes_from_the_current_def_and_keeps_the_spawner() {
        super::super::tests::use_test_store();
        let ws = std::env::temp_dir().join(format!("tmm-refresh-{}", uuid::Uuid::new_v4()));
        let ws_str = ws.to_string_lossy().to_string();
        let name = "rfagent";
        let save = |persona: &str| {
            super::super::registry_save(&json!({
                "name": name, "backend": "kiro", "model": "", "effort": "",
                "system": persona, "skills": "[]", "mcp": "[]", "can_hire": false
            }))
            .unwrap()
        };
        save("Old persona.");
        let def = super::super::registry_get(name).unwrap().expect("saved def");
        materialize(&def, name, "proj", &ws_str, "fix the login bug", "lead", None).unwrap();
        let cfg = agent_home(&ws_str, name).join("agents").join(format!("{name}.json"));
        let read = |p: &Path| -> Value { serde_json::from_str(&std::fs::read_to_string(p).unwrap()).unwrap() };
        assert!(read(&cfg)["prompt"].as_str().unwrap().contains("Old persona."));

        // The def evolves after the spawn; restart must launch THIS text.
        save("New persona.");
        assert!(refresh_agent(&ws_str, "proj", name), "a registry-named agent refreshes");
        let prompt = read(&cfg)["prompt"].as_str().unwrap().to_string();
        assert!(prompt.contains("New persona."), "current def text: {prompt}");
        assert!(!prompt.contains("Old persona."));
        assert!(!prompt.contains("fix the login bug"), "the spawn brief is history, not part of a refresh");
        let recipe = read(&agent_home(&ws_str, name).join("launch.json"));
        assert_eq!(recipe["spawned_by"], "lead", "spawner provenance survives");

        // No def / no recipe = no refresh — those windows replay verbatim.
        assert!(!refresh_agent(&ws_str, "proj", "no-such-def"));
        std::fs::remove_dir_all(agent_home(&ws_str, name)).unwrap();
        assert!(!refresh_agent(&ws_str, "proj", name), "a home without a recipe is not ours to rewrite");
        super::super::registry_delete(name).ok();
        std::fs::remove_dir_all(&ws).ok();
    }
}
