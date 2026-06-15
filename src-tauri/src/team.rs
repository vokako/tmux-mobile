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
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

// Shared team brief + keepalive hook, embedded so a packaged .app has no
// external file dependency. Written to the team work dir at startup.
const AGENTS_MD: &str = include_str!("../../team/AGENTS.md");
const KEEPALIVE_SH: &str = include_str!("../../team/hooks/keepalive.sh");

const RECONCILE_INTERVAL: Duration = Duration::from_secs(3);
const KICK: &str = "You are connected to the team group chat (collaboration rules are in AGENTS.md). Call `wait` to receive messages; when someone @mentions you, reply with `post`; otherwise keep calling `wait`. Never stop on your own — always end your turn with `wait`.";

// ─── Team templates (named rosters under <config>/tmux-mobile/teams/) ──────
// A template is a JSON file `teams/<name>.json` = { "agents": [ {name, backend,
// role, goal, backstory, model, manage}, … ] }. The user edits these from the
// app (Templates panel); `start_team` seeds the chosen template into the room.
// The built-in default is written to teams/default.json on first run so there
// is always something to edit.

/// Default model placeholder substituted in when a kiro agent leaves model empty.
pub const BUILTIN_TEMPLATE: &str = include_str!("../../team/templates/default.json");

/// The teams/ template directory.
fn templates_dir() -> PathBuf {
    crate::config::config_dir().join("teams")
}

fn template_path(name: &str) -> PathBuf {
    // Sanitize to a bare file stem so a template name can't escape the dir.
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    let safe = if safe.trim_matches('-').is_empty() { "default".to_string() } else { safe };
    templates_dir().join(format!("{}.json", safe))
}

/// Ensure the teams/ dir exists and holds at least the built-in default.
pub fn ensure_templates_seeded() {
    let dir = templates_dir();
    let _ = std::fs::create_dir_all(&dir);
    let def = dir.join("default.json");
    if !def.exists() {
        let _ = std::fs::write(&def, BUILTIN_TEMPLATE);
    }
}

/// List available template names (file stems in teams/).
pub fn list_templates() -> Vec<String> {
    ensure_templates_seeded();
    let mut names: Vec<String> = std::fs::read_dir(templates_dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some("json") {
                        p.file_stem().and_then(|s| s.to_str()).map(|s| s.to_string())
                    } else {
                        None
                    }
                })
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

/// Read a template's agent list (the `agents` array), or empty if missing/bad.
pub fn read_template(name: &str) -> Vec<Value> {
    std::fs::read_to_string(template_path(name))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v.get("agents").and_then(|a| a.as_array()).cloned())
        .unwrap_or_default()
}

/// Read every template as `{ name, agents }` for the editor panel.
pub fn read_all_templates() -> Vec<Value> {
    list_templates()
        .into_iter()
        .map(|name| serde_json::json!({ "name": name, "agents": read_template(&name) }))
        .collect()
}

/// Write a template (overwrites). `agents` is the raw array of member objects.
pub fn save_template(name: &str, agents: &Value) -> Result<(), String> {
    ensure_templates_seeded();
    let body = serde_json::json!({ "agents": agents });
    let s = serde_json::to_string_pretty(&body).map_err(|e| e.to_string())?;
    std::fs::write(template_path(name), s).map_err(|e| e.to_string())
}

/// Delete a template (the built-in default is protected).
pub fn delete_template(name: &str) -> Result<(), String> {
    if name == "default" {
        return Err("the default template cannot be deleted".into());
    }
    std::fs::remove_file(template_path(name)).map_err(|e| e.to_string())
}

/// Per-run config homes under `~/.config/tmux-mobile/team/<slug>/`. NOTE: this is
/// where each backend's *config* + the shared brief live — NOT the agents'
/// working directory. Agents `cd` into the user's chosen `workspace` (their
/// real project); we never write our brief into that project. Kiro loads the
/// brief via an absolute `resources` path; claude/codex get it pointed to in
/// their kick message.
struct Paths {
    /// Agents' working directory (the user's project) — agents run `-c` here.
    workspace: PathBuf,
    /// Our private per-team config root (NOT inside the user's project).
    kiro_home: PathBuf,
    claude: PathBuf,
    codex: PathBuf,
    keepalive: PathBuf,
    brief: PathBuf,
}

impl Paths {
    /// `workspace` = the agents' working dir; `slug` = its sanitized basename,
    /// used to namespace our config home so multiple teams coexist.
    fn new(workspace: &str, slug: &str) -> Self {
        let home = crate::config::config_dir().join("team").join(slug);
        Paths {
            workspace: PathBuf::from(workspace),
            kiro_home: home.join("kiro-home"),
            claude: home.join("claude"),
            codex: home.join("codex"),
            keepalive: home.join("keepalive.sh"),
            brief: home.join("AGENTS.md"),
        }
    }
}

/// Server-level config the supervisor needs (bus URL + default model). The
/// per-run session + workspace are passed to `start`.
#[derive(Clone)]
pub struct TeamConfig {
    /// Bus URL the agents connect to over HTTP MCP (the in-process daemon).
    pub url: String,
    /// Default model for kiro-backed agents.
    pub model: String,
}

/// Start the team for `workspace`: seed the default roster and spawn the
/// reconcile loop, launching agents into a per-workspace tmux session. The
/// agents' working directory is `workspace` (the user's project); our config +
/// brief live in a private per-team home, never written into the project.
/// Best-effort — any failure is logged, never fatal.
pub fn start(bridge: Arc<dyn TeamBridge>, cfg: TeamConfig, room: String, workspace: String, template: String) {
    tokio::spawn(async move {
        let session = format!("tmm-team-{}", room);
        let paths = Paths::new(&workspace, &room);
        if let Err(e) = prepare_home(&paths) {
            eprintln!("⚠️  team: failed to prepare config home: {}", e);
            return;
        }
        let tpl = if template.trim().is_empty() { "default".to_string() } else { template };
        seed_template(&*bridge, &room, &tpl, &cfg);
        println!("🜂 team: room={} workspace={} template={} session={}", room, workspace, tpl, session);
        reconcile_loop(bridge, cfg, room, session, paths).await;
    });
}

/// Sanitize a workspace path into a tmux-safe slug from its basename. tmux
/// session names can't contain ':' or '.'; keep it short and predictable so
/// `tmm-team-<slug>` is easy to recognize and parse.
pub fn workspace_slug(workspace: &str) -> String {
    let base = std::path::Path::new(workspace)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("root");
    let mut slug: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    slug.make_ascii_lowercase();
    let slug = slug.trim_matches('-').to_string();
    if slug.is_empty() { "root".to_string() } else { slug.chars().take(32).collect() }
}

// ─── Global system prompt (shared across every team + agent) ──────────────
// A single editable text at <config>/tmux-mobile/system_prompt.md, prepended to
// the brief that EVERY agent reads at startup. Use it for project-wide
// conventions, tone, language preference, etc. — instructions that should apply
// regardless of team or role. Empty by default (no-op).

fn system_prompt_path() -> PathBuf {
    crate::config::config_dir().join("system_prompt.md")
}

/// Read the global system prompt (empty string if unset).
pub fn read_system_prompt() -> String {
    std::fs::read_to_string(system_prompt_path()).unwrap_or_default()
}

/// Write the global system prompt (creates the file; empty clears it).
pub fn save_system_prompt(text: &str) -> Result<(), String> {
    let _ = std::fs::create_dir_all(crate::config::config_dir());
    std::fs::write(system_prompt_path(), text).map_err(|e| e.to_string())
}

/// Write the shared brief + keepalive hook into our private per-team home (NOT
/// the user's workspace). The global system prompt is prepended to the brief so
/// every agent — across all teams — sees it first.
fn prepare_home(p: &Paths) -> std::io::Result<()> {
    std::fs::create_dir_all(&p.kiro_home)?;
    let sys = read_system_prompt();
    let brief = if sys.trim().is_empty() {
        AGENTS_MD.to_string()
    } else {
        format!("{}\n\n---\n\n{}", sys.trim(), AGENTS_MD)
    };
    std::fs::write(&p.brief, brief)?;
    std::fs::write(&p.keepalive, KEEPALIVE_SH)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p.keepalive, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
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
    let agents = read_template(template);
    if agents.is_empty() {
        eprintln!("⚠️  team: template '{}' empty/missing; nothing to seed", template);
        return;
    }
    let mut names = Vec::new();
    for a in &agents {
        let name = a.get("name").and_then(|v| v.as_str()).unwrap_or("").trim();
        if name.is_empty() { continue; }
        let backend = a.get("backend").and_then(|v| v.as_str()).unwrap_or("kiro");
        // Empty model on a kiro agent → server default; other backends keep null.
        let model = a.get("model").and_then(|v| v.as_str()).filter(|s| !s.is_empty());
        let model = match (backend, model) {
            (_, Some(m)) => Value::String(m.to_string()),
            ("kiro", None) => Value::String(cfg.model.clone()),
            _ => Value::Null,
        };
        let spec = serde_json::json!({
            "role": a.get("role").and_then(|v| v.as_str()).unwrap_or(name),
            "goal": a.get("goal").and_then(|v| v.as_str()).unwrap_or(""),
            "backstory": a.get("backstory").and_then(|v| v.as_str()).unwrap_or(""),
            "backend": backend,
            "manage": a.get("manage").and_then(|v| v.as_bool()).unwrap_or(false),
            "model": model,
        });
        if let Err(e) = bridge.seed_employee(room, name, &spec) {
            eprintln!("⚠️  team: seed '{}' failed: {}", name, e);
        } else {
            names.push(name.to_string());
        }
    }
    println!("🜂 team: seeded '{}' ({}); launching…", template, names.join(" · "));
}

/// Reconcile the desired roster into real agent windows, forever (until the
/// process exits). `launched` maps a name → its pane id (or None if adopted).
///
/// Idempotency (the dup-window fix): before launching, we check tmux for an
/// EXISTING window named after the agent in this session. If one is there
/// (server restarted, agent already running), we adopt it instead of opening a
/// second. The previous in-memory-only tracking re-launched every agent on
/// restart, piling up duplicate manager/worker/reviewer windows. Reconnecting
/// adopted agents after a *server* restart is handled separately, once, by
/// `nudge_session_agents` (called from recovery) — not here, because the loop's
/// presence check can't tell a healthy agent from one hung on a dead socket.
async fn reconcile_loop(bridge: Arc<dyn TeamBridge>, cfg: TeamConfig, room: String, session: String, paths: Paths) {
    const MAX_LAUNCH_FAILURES: u32 = 3; // give up relaunching an agent after this
    let mut launched: HashMap<String, Option<String>> = HashMap::new();
    let mut fail_count: HashMap<String, u32> = HashMap::new();
    let mut launched_any = false;
    loop {
        // Stop the loop once the team is closed. close_team removes the room
        // from the registry AND kills the session — exit on either signal (the
        // room check also covers a team closed before any agent launched).
        if !bridge.room_exists(&room) || (launched_any && !tmux::session_exists(&session)) {
            println!("🜂 team: room '{}' closed; supervisor exiting", room);
            return;
        }
        let employees = bridge.employee_specs(&room);
        let roster = roster_status(&*bridge, &room);
        for (name, spec, state) in &employees {
            if state == "disabled" {
                // Kill any window we launched OR an orphan window with this name.
                if let Some(Some(pane)) = launched.get(name) {
                    let _ = tmux::kill_window(pane);
                } else if let Some(pane) = tmux::find_window_by_name(&session, name) {
                    let _ = tmux::kill_window(&pane);
                }
                launched.insert(name.clone(), None);
                continue;
            }
            if launched.contains_key(name) {
                continue;
            }
            // Already online OR a window already exists for it → adopt, don't
            // relaunch (survives server restarts without duplicating windows).
            let online = roster.get(name).map(|s| s != "offline").unwrap_or(false);
            if online {
                launched.insert(name.clone(), None);
                continue;
            }
            if let Some(pane) = tmux::find_window_by_name(&session, name) {
                println!("🜂 team: adopted existing window for '{}' ({})", name, pane);
                launched.insert(name.clone(), Some(pane));
                launched_any = true;
                continue;
            }
            // Give up after repeated failures (e.g. backend CLI not installed)
            // instead of retrying — and churning windows — every tick forever.
            if fail_count.get(name).copied().unwrap_or(0) >= MAX_LAUNCH_FAILURES {
                continue;
            }
            match launch_agent(name, spec, &cfg, &room, &session, &paths) {
                Ok(pane) => {
                    println!("🜂 team: launched '{}' in window {}", name, pane);
                    launched.insert(name.clone(), Some(pane));
                    fail_count.remove(name);
                    launched_any = true;
                }
                Err(e) => {
                    let n = fail_count.entry(name.clone()).or_insert(0);
                    *n += 1;
                    eprintln!("⚠️  team: launch '{}' failed ({}/{}): {}", name, n, MAX_LAUNCH_FAILURES, e);
                }
            }
        }
        tokio::time::sleep(RECONCILE_INTERVAL).await;
    }
}

fn roster_status(bridge: &dyn TeamBridge, room: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Some(arr) = bridge.roster(room).get("roster").and_then(|v| v.as_array()) {
        for a in arr {
            if let (Some(n), Some(s)) = (a.get("name").and_then(|v| v.as_str()), a.get("status").and_then(|v| v.as_str())) {
                out.insert(n.to_string(), s.to_string());
            }
        }
    }
    out
}

/// Write the backend config for `name` and open a named tmux window running it.
/// Returns the new pane id. Blocking tmux/fs work runs on the caller (the
/// reconcile loop is its own task and the cadence is 3 s, so this is fine).
fn launch_agent(name: &str, spec: &Value, cfg: &TeamConfig, room: &str, session: &str, paths: &Paths) -> Result<String, String> {
    let backend = spec.get("backend").and_then(|v| v.as_str()).unwrap_or("kiro");
    let role = spec.get("role").and_then(|v| v.as_str()).unwrap_or(name);
    let goal = spec.get("goal").and_then(|v| v.as_str()).unwrap_or("");
    let backstory = spec.get("backstory").and_then(|v| v.as_str()).unwrap_or("");
    let manage = spec.get("manage").and_then(|v| v.as_bool()).unwrap_or(false);
    let model = spec.get("model").and_then(|v| v.as_str());

    let (env, cmd, post_keys) = match backend {
        "kiro" => prepare_kiro(name, role, goal, backstory, manage, cfg, room, paths, model)?,
        "claude" => prepare_claude(name, role, goal, manage, cfg, room, paths, model)?,
        "codex" => prepare_codex(name, role, goal, manage, cfg, room, paths)?,
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

    // Post-launch scripted steps (e.g. Claude's folder-trust accept + kick).
    for step in post_keys {
        std::thread::sleep(Duration::from_secs(4));
        match step {
            PostKey::Enter => { let _ = tmux::send_keys(&pane, "Enter", false); }
            PostKey::Text(t) => { let _ = tmux::send_keys(&pane, &t, true); }
        }
    }
    Ok(pane)
}

/// After a *server* restart, nudge every agent window in `session` back online.
///
/// A recovered agent's MCP client lost its connection to the old (now dead)
/// daemon and is hung inside a `wait` tool call. Verified with kiro-cli 2.7.0:
/// the client neither times out nor retries on its own — but it reconnects fine
/// once the dead call is cancelled and a new turn starts. So for each agent
/// window we press Escape to cancel the in-flight call (returning the TUI to its
/// prompt), then send a short re-prompt that makes it call `wait` again, which
/// re-establishes the connection. Harmless if an agent happened to be healthy:
/// it just restarts its wait loop. This is done ONCE from recovery rather than
/// in the reconcile loop, whose presence check can't distinguish a healthy agent
/// from one hung on a dead socket (a just-restarted agent still looks "online"
/// for ~30 s until its presence TTL lapses).
///
/// Runs in a spawned task: it sleeps between keystrokes (TUI needs a beat to
/// settle) and we must not block the recovery path.
pub fn nudge_session_agents(session: String) {
    const NUDGE: &str =
        "Reconnect to the team chat: call `wait` now, and keep calling it to stay in the conversation.";
    tokio::spawn(async move {
        // Give a freshly-restarted daemon a moment to be listening before we
        // ask agents to reconnect.
        tokio::time::sleep(Duration::from_secs(2)).await;
        for (name, pane) in tmux::list_named_windows(&session) {
            if name == "zsh" {
                continue; // the session's initial shell, not an agent
            }
            println!("🜂 team: nudging adopted agent '{}' ({}) to reconnect", name, pane);
            let _ = tmux::send_keys(&pane, "Escape", false);
            tokio::time::sleep(Duration::from_millis(500)).await;
            let _ = tmux::send_keys(&pane, NUDGE, true);
            tokio::time::sleep(Duration::from_millis(200)).await;
            let _ = tmux::send_keys(&pane, "Enter", false);
        }
    });
}

enum PostKey {
    Enter,
    Text(String),
}

/// What a backend `prepare_*` returns: (env vars, launch command, post-launch
/// scripted keys). Aliased to keep the per-backend signatures readable.
type Prepared = (Vec<(String, String)>, String, Vec<PostKey>);

fn role_line(role: &str, goal: &str) -> String {
    let g = goal.replace('\n', " ");
    format!("You are the {}. {} Keep messages short.", role, g.trim()).trim().to_string()
}

/// KICK plus a pointer to the team brief. Kiro injects the brief via `resources`
/// so it just gets KICK; claude/codex have no such mechanism here (we don't
/// write into the user's workspace), so we tell them to read the brief by path.
fn kick_with_brief(paths: &Paths) -> String {
    format!("{} First read the team playbook: {}", KICK, paths.brief.to_string_lossy())
}

fn full_prompt(role: &str, goal: &str, backstory: &str) -> String {
    format!(
        "You are the {}.\nGoal: {}\nBackground: {}\nYou collaborate with other agents and a human operator in a shared team group chat (via the @team tools). Keep messages short.",
        role, goal.trim(), backstory.trim()
    )
}

const WORKER_TOOLS: &[&str] = &["post", "wait", "list_agents", "history"];

// ---- Kiro ----
#[allow(clippy::too_many_arguments)] // agent config genuinely needs all of these
fn prepare_kiro(
    name: &str, role: &str, goal: &str, backstory: &str, manage: bool,
    cfg: &TeamConfig, room: &str, paths: &Paths, model: Option<&str>,
) -> Result<Prepared, String> {
    let home = &paths.kiro_home;
    std::fs::create_dir_all(home.join("agents")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(home.join("settings")).map_err(|e| e.to_string())?;
    std::fs::write(
        home.join("settings").join("cli.json"),
        serde_json::to_string_pretty(&serde_json::json!({ "chat.disableTrustAllConfirmation": true })).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    let team: Vec<String> = if manage {
        vec!["@team".to_string()]
    } else {
        WORKER_TOOLS.iter().map(|t| format!("@team/{}", t)).collect()
    };
    let mut tools = vec!["*".to_string()];
    tools.extend(team.clone());
    let mut allowed = vec!["@builtin".to_string()];
    allowed.extend(team);

    // Brief lives in our private home (NOT the user's workspace); kiro loads it
    // by absolute path via `resources`.
    let conf = serde_json::json!({
        "name": name,
        "description": format!("{} on the team bus", role),
        "prompt": full_prompt(role, goal, backstory),
        "tools": tools,
        "allowedTools": allowed,
        "resources": [format!("file://{}", paths.brief.to_string_lossy())],
        "mcpServers": { "team": { "url": format!("{}/mcp", cfg.url), "headers": { "x-agent": name, "x-room": room } } },
        "hooks": { "stop": [ { "command": paths.keepalive.to_string_lossy() } ] },
    });
    std::fs::write(
        home.join("agents").join(format!("{}.json", name)),
        serde_json::to_string_pretty(&conf).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    let m = model.unwrap_or("claude-sonnet-4.6");
    let env = vec![("KIRO_HOME".to_string(), home.to_string_lossy().to_string())];
    let cmd = format!(
        "kiro-cli chat --agent {} --model {} --trust-all-tools {}",
        name, m, shell_quote(KICK)
    );
    Ok((env, cmd, vec![]))
}

// ---- Claude Code ----
#[allow(clippy::too_many_arguments)]
fn prepare_claude(
    name: &str, role: &str, goal: &str, manage: bool,
    cfg: &TeamConfig, room: &str, paths: &Paths, model: Option<&str>,
) -> Result<Prepared, String> {
    let d = &paths.claude;
    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    let mcpfile = d.join(format!("{}.mcp.json", name));
    std::fs::write(
        &mcpfile,
        serde_json::to_string_pretty(&serde_json::json!({
            "mcpServers": { "team": { "type": "http", "url": format!("{}/mcp", cfg.url), "headers": { "x-agent": name, "x-room": room } } }
        }))
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let settingsfile = d.join(format!("{}.settings.json", name));
    std::fs::write(
        &settingsfile,
        serde_json::to_string_pretty(&serde_json::json!({
            "hooks": { "Stop": [ { "hooks": [ { "type": "command", "command": paths.keepalive.to_string_lossy() } ] } ] }
        }))
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;

    let m = model.unwrap_or("sonnet");
    // MCP tool names are mcp__<server>__<tool>; our server is named "team".
    let disallow = if manage { "" } else { "--disallowedTools mcp__team__hire mcp__team__fire " };
    let cmd = format!(
        "claude --mcp-config {} --strict-mcp-config --settings {} --model {} --dangerously-skip-permissions {}",
        shell_quote(&mcpfile.to_string_lossy()),
        shell_quote(&settingsfile.to_string_lossy()),
        m,
        disallow
    )
    .trim_end()
    .to_string();
    let first_msg = format!("{} {}", role_line(role, goal), kick_with_brief(paths));
    // Start interactive; then accept the folder-trust dialog, type the kick, submit.
    let post = vec![PostKey::Enter, PostKey::Text(first_msg), PostKey::Enter];
    Ok((vec![], cmd, post))
}

// ---- Codex ----
#[allow(clippy::too_many_arguments)]
fn prepare_codex(
    name: &str, role: &str, goal: &str, manage: bool, cfg: &TeamConfig, room: &str, paths: &Paths,
) -> Result<Prepared, String> {
    let home = paths.codex.join(name);
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let gating = if manage { "" } else { "disabled_tools = [\"hire\", \"fire\"]\n" };
    let config = format!(
        "[mcp_servers.team]\nurl = \"{}/mcp\"\nenabled = true\nexperimental_use_rmcp_client = true\n{}\n[mcp_servers.team.http_headers]\n\"x-agent\" = \"{}\"\n\"x-room\" = \"{}\"\n",
        cfg.url, gating, name, room
    );
    std::fs::write(home.join("config.toml"), config).map_err(|e| e.to_string())?;
    let env = vec![("CODEX_HOME".to_string(), home.to_string_lossy().to_string())];
    let first_msg = format!("{} {}", role_line(role, goal), kick_with_brief(paths));
    let cmd = format!("codex --dangerously-bypass-approvals-and-sandbox {}", shell_quote(&first_msg));
    Ok((env, cmd, vec![]))
}

/// Single-quote a string for the shell (the agent launch line is sent to a
/// tmux pane's shell). Wraps in '…' and escapes embedded single quotes.
fn shell_quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'/' | b'=' | b':')) {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Records seed_employee calls so we can assert the default roster.
    struct RecordingBridge {
        seeded: Mutex<Vec<(String, Value)>>,
        existing: Vec<(String, Value, String)>,
    }
    impl TeamBridge for RecordingBridge {
        fn history(&self, _room: &str, _l: i64) -> Value { serde_json::json!({}) }
        fn roster(&self, _room: &str) -> Value { serde_json::json!({ "roster": [] }) }
        fn post(&self, _room: &str, _f: &str, _b: &str, _r: bool) -> Result<Value, String> { Ok(Value::Null) }
        fn employees(&self, _room: &str) -> Value { serde_json::json!({}) }
        fn seed_employee(&self, _room: &str, name: &str, spec: &Value) -> Result<(), String> {
            self.seeded.lock().unwrap().push((name.to_string(), spec.clone()));
            Ok(())
        }
        fn employee_specs(&self, _room: &str) -> Vec<(String, Value, String)> { self.existing.clone() }
        fn room_exists(&self, _room: &str) -> bool { true }
        fn start_team(&self, _workspace: &str, _template: &str) -> Value { serde_json::json!({ "started": false }) }
        fn close_team(&self, _room: &str) -> bool { false }
        fn teams(&self) -> Value { serde_json::json!({ "teams": [] }) }
        fn templates(&self) -> Value { serde_json::json!({ "templates": [] }) }
        fn save_template(&self, _name: &str, _agents: &Value) -> Result<(), String> { Ok(()) }
        fn delete_template(&self, _name: &str) -> Result<(), String> { Ok(()) }
        fn system_prompt(&self) -> String { String::new() }
        fn save_system_prompt(&self, _text: &str) -> Result<(), String> { Ok(()) }
        fn default_workspace(&self) -> String { "/tmp/ws".into() }
        fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
            tokio::sync::broadcast::channel(1).1
        }
    }

    fn cfg() -> TeamConfig {
        TeamConfig { url: "http://127.0.0.1:8787".into(), model: "claude-sonnet-4.6".into() }
    }

    #[test]
    fn builtin_default_template_has_three_agents_one_manager() {
        // Parse the embedded built-in template (what teams/default.json seeds).
        let v: Value = serde_json::from_str(BUILTIN_TEMPLATE).unwrap();
        let agents = v["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 3, "manager + worker + reviewer");
        let names: Vec<&str> = agents.iter().filter_map(|a| a["name"].as_str()).collect();
        assert!(names.contains(&"manager") && names.contains(&"worker") && names.contains(&"reviewer"));
        let managers = agents.iter().filter(|a| a["manage"] == true).count();
        assert_eq!(managers, 1, "exactly one manager");
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
    fn seed_template_kiro_empty_model_falls_back_to_default() {
        // seed_template reads from disk; ensure the built-in default exists.
        ensure_templates_seeded();
        let b = RecordingBridge { seeded: Mutex::new(vec![]), existing: vec![] };
        seed_template(&b, "myroom", "default", &cfg());
        let seeded = b.seeded.lock().unwrap();
        assert!(!seeded.is_empty(), "default template should seed agents");
        // kiro agents with empty model inherit the server default.
        assert!(seeded.iter().all(|(_, s)| s["model"] == "claude-sonnet-4.6"));
    }

    #[test]
    fn workspace_slug_is_tmux_safe_basename() {
        assert_eq!(workspace_slug("/Users/clawd/work/My Project"), "my-project");
        assert_eq!(workspace_slug("/Users/clawd/work/260226_tmux_mobile"), "260226_tmux_mobile");
        assert_eq!(workspace_slug("/"), "root");
        assert_eq!(workspace_slug(""), "root");
        // No ':' or '.' (illegal in tmux session names).
        let s = workspace_slug("/a/b.c:d");
        assert!(!s.contains(':') && !s.contains('.'), "got {s}");
    }

    #[test]
    fn shell_quote_plain_passthrough() {
        assert_eq!(shell_quote("kiro-cli"), "kiro-cli");
        assert_eq!(shell_quote("a/b_c.d"), "a/b_c.d");
    }

    #[test]
    fn shell_quote_wraps_spaces_and_unicode() {
        assert_eq!(shell_quote("hello world"), "'hello world'");
        assert_eq!(shell_quote("你是「经理」"), "'你是「经理」'");
    }

    #[test]
    fn shell_quote_escapes_single_quote() {
        assert_eq!(shell_quote("it's"), r"'it'\''s'");
    }

    #[test]
    fn role_line_is_single_line() {
        let r = role_line("worker", "do\nthings\nwell");
        assert!(!r.contains('\n'), "role line must be single-line: {}", r);
    }

    #[test]
    fn system_prompt_roundtrip_and_prepend() {
        // Isolate config dir so we don't touch the real ~/.config.
        let dir = std::env::temp_dir().join(format!("teamtest-sys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);

        // Empty by default.
        assert_eq!(read_system_prompt(), "");
        // Save + read back.
        save_system_prompt("Respond in English. Be terse.").unwrap();
        assert_eq!(read_system_prompt(), "Respond in English. Be terse.");

        // prepare_home prepends it to the embedded brief.
        let paths = Paths::new("/tmp/proj", "proj");
        prepare_home(&paths).unwrap();
        let brief = std::fs::read_to_string(&paths.brief).unwrap();
        assert!(brief.starts_with("Respond in English. Be terse."), "system prompt must lead the brief");
        assert!(brief.contains("collaboration playbook") || brief.contains("AGENTS.md") || brief.contains("team group chat"),
            "brief body must still be present");

        // Clearing it leaves the brief as just the built-in.
        save_system_prompt("").unwrap();
        prepare_home(&paths).unwrap();
        let brief2 = std::fs::read_to_string(&paths.brief).unwrap();
        assert!(!brief2.starts_with("Respond in English"));

        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }
}


