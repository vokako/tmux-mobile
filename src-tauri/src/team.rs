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
const KICK: &str = "你已接入 team 群聊（协作规则见 AGENTS.md）。直接调用 wait 等待消息；被点名就用 post 回复发起人，没你的事就继续 wait；不要主动停止。";

/// One member of the default team (role / goal / backstory / backend / manage).
struct Member {
    name: &'static str,
    backend: &'static str,
    role: &'static str,
    goal: &'static str,
    backstory: &'static str,
    manage: bool,
}

/// The built-in team — same roster as the original `team.yaml`.
const DEFAULT_TEAM: &[Member] = &[
    Member {
        name: "manager",
        backend: "kiro",
        role: "经理",
        goal: "把人类给的目标拆成清晰的小任务，分派给 worker；完成后向人类汇报结果。",
        backstory: "你统筹全局，不亲自写实现，只做拆解、分派与收口。",
        manage: true,
    },
    Member {
        name: "worker",
        backend: "kiro",
        role: "执行者",
        goal: "领取分派给你的任务并完成，把结果简洁地回复给 manager。",
        backstory: "你专注把单个任务做扎实，做完就汇报。",
        manage: false,
    },
    Member {
        name: "reviewer",
        backend: "kiro",
        role: "评审",
        goal: "检查 worker 的产出，批准或指出一处需要改进的地方。",
        backstory: "你严谨、建设性，只关注质量与正确性。",
        manage: false,
    },
];

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
pub fn start(bridge: Arc<dyn TeamBridge>, cfg: TeamConfig, room: String, workspace: String) {
    tokio::spawn(async move {
        let session = format!("tmm-team-{}", room);
        let paths = Paths::new(&workspace, &room);
        if let Err(e) = prepare_home(&paths) {
            eprintln!("⚠️  team: failed to prepare config home: {}", e);
            return;
        }
        seed_default_team(&*bridge, &room, &cfg);
        println!("🜂 team: room={} workspace={} session={}", room, workspace, session);
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

/// Write the shared brief + keepalive hook into our private per-team home (NOT
/// the user's workspace).
fn prepare_home(p: &Paths) -> std::io::Result<()> {
    std::fs::create_dir_all(&p.kiro_home)?;
    std::fs::write(&p.brief, AGENTS_MD)?;
    std::fs::write(&p.keepalive, KEEPALIVE_SH)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p.keepalive, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

/// Register the built-in team as employees, unless a team already exists (so a
/// restart doesn't duplicate-seed; seed_employee also rejects taken names).
fn seed_default_team(bridge: &dyn TeamBridge, room: &str, cfg: &TeamConfig) {
    let existing = bridge.employee_specs(room);
    if !existing.is_empty() {
        return; // a team is already seeded (this run or a previous one)
    }
    for m in DEFAULT_TEAM {
        let spec = serde_json::json!({
            "role": m.role,
            "goal": m.goal,
            "backstory": m.backstory,
            "backend": m.backend,
            "manage": m.manage,
            "model": if m.backend == "kiro" { Value::String(cfg.model.clone()) } else { Value::Null },
        });
        if let Err(e) = bridge.seed_employee(room, m.name, &spec) {
            eprintln!("⚠️  team: seed '{}' failed: {}", m.name, e);
        }
    }
    println!("🜂 team: seeded default roster (manager · worker · reviewer); launching…");
}

/// Reconcile the desired roster into real agent windows, forever (until the
/// process exits). `launched` maps a name → its pane id (or None if adopted).
///
/// Idempotency (the dup-window fix): before launching, we check tmux for an
/// EXISTING window named after the agent in this session. If one is there
/// (server restarted, agent already running), we adopt it instead of opening a
/// second. The previous in-memory-only tracking re-launched every agent on
/// restart, piling up duplicate manager/worker/reviewer windows.
async fn reconcile_loop(bridge: Arc<dyn TeamBridge>, cfg: TeamConfig, room: String, session: String, paths: Paths) {
    let mut launched: HashMap<String, Option<String>> = HashMap::new();
    let mut launched_any = false;
    loop {
        // Stop the loop once the team is closed. close_team kills the session,
        // so: if we have already launched ≥1 agent and the session no longer
        // exists, the team was closed — exit cleanly.
        if launched_any && !tmux::session_exists(&session) {
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
            match launch_agent(name, spec, &cfg, &room, &session, &paths) {
                Ok(pane) => {
                    println!("🜂 team: launched '{}' in window {}", name, pane);
                    launched.insert(name.clone(), Some(pane));
                    launched_any = true;
                }
                Err(e) => eprintln!("⚠️  team: launch '{}' failed: {}", name, e),
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

enum PostKey {
    Enter,
    Text(String),
}

/// What a backend `prepare_*` returns: (env vars, launch command, post-launch
/// scripted keys). Aliased to keep the per-backend signatures readable.
type Prepared = (Vec<(String, String)>, String, Vec<PostKey>);

fn role_line(role: &str, goal: &str) -> String {
    let g = goal.replace('\n', " ");
    format!("你是「{}」。{} 请用中文、消息简短。", role, g.trim()).trim().to_string()
}

/// KICK plus a pointer to the team brief. Kiro injects the brief via `resources`
/// so it just gets KICK; claude/codex have no such mechanism here (we don't
/// write into the user's workspace), so we tell them to read the brief by path.
fn kick_with_brief(paths: &Paths) -> String {
    format!("{} 先读团队协作手册：{}。", KICK, paths.brief.to_string_lossy())
}

fn full_prompt(role: &str, goal: &str, backstory: &str) -> String {
    format!(
        "你是「{}」。\n目标：{}\n背景：{}\n你和其他 agent、以及一位人类，在共享的『team 群聊』里协作（通过 @team 工具）。请始终用中文交流，消息保持简短。",
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
        fn start_team(&self, _workspace: &str) -> Value { serde_json::json!({ "started": false }) }
        fn close_team(&self, _room: &str) -> bool { false }
        fn teams(&self) -> Value { serde_json::json!({ "teams": [] }) }
        fn default_workspace(&self) -> String { "/tmp/ws".into() }
        fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
            tokio::sync::broadcast::channel(1).1
        }
    }

    fn cfg() -> TeamConfig {
        TeamConfig { url: "http://127.0.0.1:8787".into(), model: "claude-sonnet-4.6".into() }
    }

    #[test]
    fn seed_default_team_seeds_three_with_one_manager() {
        let b = RecordingBridge { seeded: Mutex::new(vec![]), existing: vec![] };
        seed_default_team(&b, "myroom", &cfg());
        let seeded = b.seeded.lock().unwrap();
        assert_eq!(seeded.len(), 3, "manager + worker + reviewer");
        let names: Vec<&str> = seeded.iter().map(|(n, _)| n.as_str()).collect();
        assert!(names.contains(&"manager") && names.contains(&"worker") && names.contains(&"reviewer"));
        let managers = seeded.iter().filter(|(_, s)| s["manage"] == true).count();
        assert_eq!(managers, 1);
        // kiro agents carry a model; the spec must include it.
        assert_eq!(seeded[0].1["model"], "claude-sonnet-4.6");
    }

    #[test]
    fn seed_default_team_skips_when_team_already_present() {
        let b = RecordingBridge {
            seeded: Mutex::new(vec![]),
            existing: vec![("manager".into(), Value::Null, "active".into())],
        };
        seed_default_team(&b, "myroom", &cfg());
        assert!(b.seeded.lock().unwrap().is_empty(), "must not re-seed an existing team");
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
    fn default_team_has_one_manager() {
        let managers = DEFAULT_TEAM.iter().filter(|m| m.manage).count();
        assert_eq!(managers, 1, "exactly one manager in the default team");
    }
}


