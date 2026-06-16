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
const HEARTBEAT_SH: &str = include_str!("../../team/hooks/heartbeat.sh");

const RECONCILE_INTERVAL: Duration = Duration::from_secs(3);
/// Self-heal threshold: no `wait` and no heartbeat for this long ⇒ the agent is
/// wedged; the supervisor nudges its window (Esc + reconnect re-prompt). Well
/// above the bus's 90s `unreachable` mark so we only ever auto-restart an agent
/// that has been silent for a genuinely long time, not one merely between tools.
const RECOVERY_STALE_MS: i64 = 1_800_000; // 30 minutes
const KICK: &str = "You are connected to the team group chat (collaboration rules are in AGENTS.md). Call `wait` to receive messages; when someone @mentions you, reply with `post`; otherwise keep calling `wait`. Never stop on your own — always end your turn with `wait`.";

// ─── Team templates (named rosters under <config>/tmux-mobile/teams/) ──────
// A template is a JSON file `teams/<name>.json` = { "agents": [ {name, backend,
// role, goal, model, manage}, … ] }. The user edits these from the
// app (Templates panel); `start_team` seeds the chosen template into the room.
// The built-in default is written to teams/default.json on first run so there
// is always something to edit.

/// Default model placeholder substituted in when a kiro agent leaves model empty.
pub const BUILTIN_TEMPLATE: &str = include_str!("../../team/templates/default/team.yaml");

/// A ready-made software-development roster (tech-lead / product / architect /
/// coder / reviewer / tester), seeded alongside the default so it appears in
/// the app's template picker out of the box. The whole collaboration workflow
/// lives in each agent's `goal` (role isolation) — AGENTS.md stays a
/// role-agnostic, workflow-free communication contract.
pub const SOFTWARE_DEV_TEMPLATE: &str = include_str!("../../team/templates/software-dev/team.yaml");

/// A financial-research roster modeled on Dexter (virattt/dexter): a research
/// director plus fundamentals / market+sentiment / valuation(DCF) / memo /
/// reviewer analysts. Dexter's single-agent skills (DCF, investment memo, X
/// sentiment) become specialist roles; its data discipline (figures carry
/// sources, the deliverable is a file, chat is a scannable header) and its
/// educational-only / not-investment-advice posture are baked into the goals.
pub const FINANCIAL_RESEARCH_TEMPLATE: &str =
    include_str!("../../team/templates/financial-research/team.yaml");

/// A deep-research roster: a director who decomposes the question, two parallel
/// researchers, a synthesist, and a skeptic — every claim sourced, output to
/// report.md.
pub const DEEP_RESEARCH_TEMPLATE: &str = include_str!("../../team/templates/deep-research/team.yaml");

/// A content-studio roster (editor-in-chief / researcher / writer / copy editor)
/// for shipping a publish-ready article or docs in a shared house style.
pub const CONTENT_STUDIO_TEMPLATE: &str = include_str!("../../team/templates/content-studio/team.yaml");

/// A data-analysis roster (lead / data engineer / analyst / reporter) that
/// answers a question from data with reproducible work and honest caveats.
pub const DATA_ANALYSIS_TEMPLATE: &str = include_str!("../../team/templates/data-analysis/team.yaml");

/// Built-in templates seeded into teams/ on first run: (file stem, contents).
const BUILTIN_TEMPLATES: &[(&str, &str)] = &[
    ("default", BUILTIN_TEMPLATE),
    ("software-dev", SOFTWARE_DEV_TEMPLATE),
    ("financial-research", FINANCIAL_RESEARCH_TEMPLATE),
    ("deep-research", DEEP_RESEARCH_TEMPLATE),
    ("content-studio", CONTENT_STUDIO_TEMPLATE),
    ("data-analysis", DATA_ANALYSIS_TEMPLATE),
];

/// The teams/ template directory.
fn templates_dir() -> PathBuf {
    crate::config::config_dir().join("teams")
}

/// Sanitize a template name to a safe single path segment (no escaping the dir).
fn sanitize_name(name: &str) -> String {
    let safe: String = name
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    if safe.trim_matches('-').is_empty() { "default".to_string() } else { safe }
}

/// A template now lives in its OWN folder `teams/<name>/`, holding `team.yaml`
/// (the roster + per-agent env/mcp/skills) and optionally a `skills/` dir of
/// local skills bundled with the team. The folder is the unit so a team can
/// carry its own assets.
fn team_dir(name: &str) -> PathBuf {
    templates_dir().join(sanitize_name(name))
}

fn template_yaml_path(name: &str) -> PathBuf {
    team_dir(name).join("team.yaml")
}

/// One-time migration of the old flat `teams/<name>.json` files into the new
/// `teams/<name>/team.yaml` folder layout. The legacy file is renamed to
/// `<name>.json.bak` (kept, not deleted) so the move is reversible.
fn migrate_legacy_json() {
    let dir = templates_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().and_then(|x| x.to_str()) != Some("json") {
            continue;
        }
        let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else { continue };
        let yaml_path = team_dir(stem).join("team.yaml");
        if yaml_path.exists() {
            continue; // already migrated
        }
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        let Ok(val) = serde_json::from_str::<Value>(&text) else { continue };
        if let Some(parent) = yaml_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(yaml) = serde_yml::to_string(&val) {
            if std::fs::write(&yaml_path, yaml).is_ok() {
                let _ = std::fs::rename(&p, p.with_extension("json.bak"));
                println!("🜂 team: migrated legacy template '{}' → team.yaml", stem);
            }
        }
    }
}

/// Ensure the teams/ dir exists and holds the built-in templates. Migrates any
/// legacy `*.json` first, then seed-once per folder: an existing template is
/// never overwritten, so a user's edits (and custom templates) survive restarts.
pub fn ensure_templates_seeded() {
    let dir = templates_dir();
    let _ = std::fs::create_dir_all(&dir);
    migrate_legacy_json();
    for (name, body) in BUILTIN_TEMPLATES {
        let path = template_yaml_path(name);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&path, body);
        }
    }
}

/// List available template names (folders in teams/ that hold a team.yaml).
pub fn list_templates() -> Vec<String> {
    ensure_templates_seeded();
    let mut names: Vec<String> = std::fs::read_dir(templates_dir())
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter_map(|e| {
                    let p = e.path();
                    if p.is_dir() && p.join("team.yaml").is_file() {
                        p.file_name().and_then(|s| s.to_str()).map(|s| s.to_string())
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

/// Read a template's full definition object (`{ env?, agents }`) from YAML, or
/// `null` if missing/bad.
pub fn read_team_def(name: &str) -> Value {
    std::fs::read_to_string(template_yaml_path(name))
        .ok()
        .and_then(|s| serde_yml::from_str::<Value>(&s).ok())
        .unwrap_or(Value::Null)
}

/// Read a template's agent list (the `agents` array), or empty if missing/bad.
pub fn read_template(name: &str) -> Vec<Value> {
    read_team_def(name)
        .get("agents")
        .and_then(|a| a.as_array())
        .cloned()
        .unwrap_or_default()
}

/// Read every template as `{ name, env, mcp, skills, prompt, agents }` for the
/// editor panel (team-wide fields + the roster).
pub fn read_all_templates() -> Vec<Value> {
    list_templates()
        .into_iter()
        .map(|name| {
            let def = read_team_def(&name);
            serde_json::json!({
                "name": name,
                "env": def.get("env").cloned().unwrap_or(serde_json::json!({})),
                "mcp": def.get("mcp").cloned().unwrap_or(serde_json::json!([])),
                "skills": def.get("skills").cloned().unwrap_or(serde_json::json!([])),
                "prompt": def.get("prompt").cloned().unwrap_or(serde_json::json!("")),
                "agents": def.get("agents").cloned().unwrap_or(serde_json::json!([])),
            })
        })
        .collect()
}

/// Write a template from the full definition object `{ env?, mcp?, skills?,
/// prompt?, agents }`. Empty team-wide fields are pruned so the YAML stays tidy.
pub fn save_template(name: &str, def: &Value) -> Result<(), String> {
    ensure_templates_seeded();
    // Accept either a full def object or a bare agents array (legacy callers).
    let def = if def.is_array() {
        serde_json::json!({ "agents": def })
    } else {
        def.clone()
    };
    let mut out = serde_json::Map::new();
    if let Some(env) = def.get("env").and_then(|v| v.as_object()) {
        if !env.is_empty() { out.insert("env".into(), Value::Object(env.clone())); }
    }
    if let Some(mcp) = def.get("mcp").and_then(|v| v.as_array()) {
        if !mcp.is_empty() { out.insert("mcp".into(), Value::Array(mcp.clone())); }
    }
    if let Some(sk) = def.get("skills").and_then(|v| v.as_array()) {
        if !sk.is_empty() { out.insert("skills".into(), Value::Array(sk.clone())); }
    }
    if let Some(p) = def.get("prompt").and_then(|v| v.as_str()) {
        if !p.trim().is_empty() { out.insert("prompt".into(), Value::String(p.to_string())); }
    }
    out.insert("agents".into(), def.get("agents").cloned().unwrap_or(serde_json::json!([])));
    let yaml = serde_yml::to_string(&Value::Object(out)).map_err(|e| e.to_string())?;
    let path = template_yaml_path(name);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    std::fs::write(path, yaml).map_err(|e| e.to_string())
}

/// Delete a template folder (the built-in default is protected).
pub fn delete_template(name: &str) -> Result<(), String> {
    if name == "default" {
        return Err("the default template cannot be deleted".into());
    }
    std::fs::remove_dir_all(team_dir(name)).map_err(|e| e.to_string())
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
    heartbeat: PathBuf,
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
            heartbeat: home.join("heartbeat.sh"),
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
        let tpl = if template.trim().is_empty() { "default".to_string() } else { template };
        // Team-wide prompt (if any) is appended to every agent's brief.
        let team_prompt = read_team_def(&tpl)
            .get("prompt").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if let Err(e) = prepare_home(&paths, &team_prompt) {
            eprintln!("⚠️  team: failed to prepare config home: {}", e);
            return;
        }
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
fn prepare_home(p: &Paths, team_prompt: &str) -> std::io::Result<()> {
    std::fs::create_dir_all(&p.kiro_home)?;
    // Brief = global system prompt (all teams) → AGENTS.md contract → this team's
    // own prompt (all its agents). Each section only added if non-empty.
    let mut brief = String::new();
    let sys = read_system_prompt();
    if !sys.trim().is_empty() {
        brief.push_str(sys.trim());
        brief.push_str("\n\n---\n\n");
    }
    brief.push_str(AGENTS_MD);
    if !team_prompt.trim().is_empty() {
        brief.push_str("\n\n---\n\n");
        brief.push_str(team_prompt.trim());
    }
    std::fs::write(&p.brief, brief)?;
    std::fs::write(&p.keepalive, KEEPALIVE_SH)?;
    std::fs::write(&p.heartbeat, HEARTBEAT_SH)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p.keepalive, std::fs::Permissions::from_mode(0o755))?;
        std::fs::set_permissions(&p.heartbeat, std::fs::Permissions::from_mode(0o755))?;
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
    let mut last_nudge: HashMap<String, i64> = HashMap::new(); // self-heal cooldown
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
        let roster = roster_liveness(&*bridge, &room);
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
            let online = roster.get(name).map(|(s, _)| s != "offline").unwrap_or(false);
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
        // ── Self-heal backstop ────────────────────────────────────────────
        // An agent we have heard NOTHING from for RECOVERY_STALE_MS — no `wait`
        // (a parked agent touches ~every second), no per-tool heartbeat — is
        // wedged: a dead MCP socket, a crashed loop, or a stop we never caught.
        // Nudge its window once (Esc to cancel the stuck call + a re-prompt to
        // resume `wait`), then cool down for the same window before trying
        // again, so we never spam — and so a genuinely long single tool (which
        // emits no heartbeat until it returns) is interrupted at most rarely.
        let now = now_ms();
        for (name, (status, last_seen)) in &roster {
            if status == "offline" || now - last_seen < RECOVERY_STALE_MS {
                continue;
            }
            if now - last_nudge.get(name).copied().unwrap_or(0) < RECOVERY_STALE_MS {
                continue; // already nudged recently; give it time
            }
            if let Some(pane) = tmux::find_window_by_name(&session, name) {
                println!(
                    "🜂 team: agent '{}' unreachable for {}s — self-heal nudging {}",
                    name, (now - last_seen) / 1000, pane
                );
                tokio::spawn(async move { nudge_pane(&pane).await; });
                last_nudge.insert(name.clone(), now);
            }
        }

        tokio::time::sleep(RECONCILE_INTERVAL).await;
    }
}

fn roster_liveness(bridge: &dyn TeamBridge, room: &str) -> HashMap<String, (String, i64)> {
    let mut out = HashMap::new();
    if let Some(arr) = bridge.roster(room).get("roster").and_then(|v| v.as_array()) {
        for a in arr {
            let name = a.get("name").and_then(|v| v.as_str());
            let status = a.get("status").and_then(|v| v.as_str());
            let last_seen = a.get("last_seen").and_then(|v| v.as_i64()).unwrap_or(0);
            if let (Some(n), Some(s)) = (name, status) {
                out.insert(n.to_string(), (s.to_string(), last_seen));
            }
        }
    }
    out
}

/// Wall-clock millis since the epoch — same basis as the bus's `last_seen`, so
/// staleness math in the self-heal backstop lines up across the crate boundary.
fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Write the backend config for `name` and open a named tmux window running it.
/// Returns the new pane id. Blocking tmux/fs work runs on the caller (the
/// reconcile loop is its own task and the cadence is 3 s, so this is fine).
fn launch_agent(name: &str, spec: &Value, cfg: &TeamConfig, room: &str, session: &str, paths: &Paths) -> Result<String, String> {
    let backend = spec.get("backend").and_then(|v| v.as_str()).unwrap_or("kiro");
    let role = spec.get("role").and_then(|v| v.as_str()).unwrap_or(name);
    let goal = spec.get("goal").and_then(|v| v.as_str()).unwrap_or("");
    let manage = spec.get("manage").and_then(|v| v.as_bool()).unwrap_or(false);
    let model = spec.get("model").and_then(|v| v.as_str());

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

    let (env, cmd, post_keys) = match backend {
        "kiro" => prepare_kiro(name, role, goal, manage, cfg, room, paths, model, &extras)?,
        "claude" => prepare_claude(name, role, goal, manage, cfg, room, paths, model, &extras)?,
        "codex" => prepare_codex(name, role, goal, manage, cfg, room, paths, &extras)?,
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
    tokio::spawn(async move {
        // Give a freshly-restarted daemon a moment to be listening before we
        // ask agents to reconnect.
        tokio::time::sleep(Duration::from_secs(2)).await;
        for (name, pane) in tmux::list_named_windows(&session) {
            if name == "zsh" {
                continue; // the session's initial shell, not an agent
            }
            println!("🜂 team: nudging adopted agent '{}' ({}) to reconnect", name, pane);
            nudge_pane(&pane).await;
        }
    });
}

/// The re-prompt that gets a wedged/stopped agent calling `wait` again.
const RECONNECT_NUDGE: &str =
    "Reconnect to the team chat: call `wait` now, and keep calling it to stay in the conversation.";

/// Press Esc (cancel any stuck in-flight call → back to the prompt), then send
/// the reconnect re-prompt and submit it. Shared by restart-recovery and the
/// supervisor's liveness self-heal. Sleeps between keystrokes (the TUI needs a
/// beat to settle), so callers run it inside a spawned task.
async fn nudge_pane(pane: &str) {
    let _ = tmux::send_keys(pane, "Escape", false);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = tmux::send_keys(pane, RECONNECT_NUDGE, true);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = tmux::send_keys(pane, "Enter", false);
}

enum PostKey {
    Enter,
    Text(String),
}

/// What a backend `prepare_*` returns: (env vars, launch command, post-launch
/// scripted keys). Aliased to keep the per-backend signatures readable.
type Prepared = (Vec<(String, String)>, String, Vec<PostKey>);

/// Single-line agent brief for backends whose prompt is injected as a typed
/// first message (claude/codex), where embedded newlines would submit early.
/// Same content as kiro's `full_prompt` (role + goal), flattened to one line.
fn role_line(role: &str, goal: &str) -> String {
    let g = goal.replace('\n', " ");
    format!("You are the {}. {} Keep messages short.", role, g.trim())
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// KICK plus a pointer to the team brief. Kiro injects the brief via `resources`
/// so it just gets KICK; claude/codex have no such mechanism here (we don't
/// write into the user's workspace), so we tell them to read the brief by path.
fn kick_with_brief(paths: &Paths) -> String {
    format!("{} First read the team playbook: {}", KICK, paths.brief.to_string_lossy())
}

fn full_prompt(role: &str, goal: &str) -> String {
    format!(
        "You are the {}.\nGoal: {}\nYou collaborate with other agents and a human operator in a shared team group chat (via the @team tools). Keep messages short.",
        role, goal.trim()
    )
}

const WORKER_TOOLS: &[&str] = &["post", "wait", "list_agents", "history"];

// ---- Kiro ----
#[allow(clippy::too_many_arguments)] // agent config genuinely needs all of these
/// An extra MCP server attached to an agent (from the team.yaml `mcp:` list).
/// Either a remote HTTP server (`url` [+ `headers`]) or a local stdio server
/// (`command` [+ `args`/`env`]).
#[derive(serde::Deserialize, Default, Clone)]
struct McpDef {
    name: String,
    #[serde(default)]
    url: Option<String>,
    #[serde(default)]
    headers: std::collections::BTreeMap<String, String>,
    #[serde(default)]
    command: Option<String>,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: std::collections::BTreeMap<String, String>,
}

/// A skill resolved to a concrete local directory (containing SKILL.md), ready
/// to wire into a backend.
struct ResolvedSkill {
    name: String,
    dir: PathBuf,
    description: String,
}

/// Per-agent extras threaded from the spec into each backend's launcher.
struct Extras {
    env: Vec<(String, String)>,
    mcp: Vec<McpDef>,
    skills: Vec<ResolvedSkill>,
}

/// kiro mcpServers entry: remote = `{url,headers}`, local = `{command,args,env}`.
fn kiro_mcp_value(m: &McpDef) -> Value {
    if let Some(url) = &m.url {
        let mut o = serde_json::json!({ "url": url });
        if !m.headers.is_empty() {
            o["headers"] = serde_json::to_value(&m.headers).unwrap_or(Value::Null);
        }
        o
    } else if let Some(cmd) = &m.command {
        let mut o = serde_json::json!({ "command": cmd, "args": m.args });
        if !m.env.is_empty() {
            o["env"] = serde_json::to_value(&m.env).unwrap_or(Value::Null);
        }
        o
    } else {
        serde_json::json!({})
    }
}

/// claude mcpServers entry: remote gets explicit `type:"http"`.
fn claude_mcp_value(m: &McpDef) -> Value {
    if let Some(url) = &m.url {
        let mut o = serde_json::json!({ "type": "http", "url": url });
        if !m.headers.is_empty() {
            o["headers"] = serde_json::to_value(&m.headers).unwrap_or(Value::Null);
        }
        o
    } else {
        kiro_mcp_value(m) // local stdio form is identical
    }
}

/// codex `[mcp_servers.<name>]` TOML block for an extra server.
fn codex_mcp_toml(m: &McpDef) -> String {
    let mut s = format!("\n[mcp_servers.{}]\n", m.name);
    if let Some(url) = &m.url {
        s += &format!("url = \"{}\"\nenabled = true\nexperimental_use_rmcp_client = true\n", url);
        if !m.headers.is_empty() {
            s += &format!("\n[mcp_servers.{}.http_headers]\n", m.name);
            for (k, v) in &m.headers {
                s += &format!("\"{}\" = \"{}\"\n", k, v);
            }
        }
    } else if let Some(cmd) = &m.command {
        s += &format!("command = \"{}\"\n", cmd);
        if !m.args.is_empty() {
            let args: Vec<String> = m.args.iter().map(|a| format!("\"{}\"", a)).collect();
            s += &format!("args = [{}]\n", args.join(", "));
        }
        if !m.env.is_empty() {
            s += &format!("\n[mcp_servers.{}.env]\n", m.name);
            for (k, v) in &m.env {
                s += &format!("\"{}\" = \"{}\"\n", k, v);
            }
        }
    }
    s
}

/// A one-line skills index appended to the kick for backends without a native
/// skill mechanism (claude/codex). kiro instead gets `skill://` resources.
fn skills_index_text(skills: &[ResolvedSkill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut s = String::from(" Skills available — read the named SKILL.md before a matching task:");
    for sk in skills {
        s += &format!(" [{}] {} (at {}/SKILL.md);", sk.name, sk.description, sk.dir.display());
    }
    s
}

fn skills_cache_dir() -> PathBuf {
    crate::config::config_dir().join("skills-cache")
}

/// Resolve each skill reference to a local directory. A reference is either a
/// local path (relative to the team folder, or absolute) or a GitHub URL, which
/// is sparse-cloned into a shared cache (reused across teams/agents).
fn resolve_skills(refs: &[String], team_dir: &str) -> Vec<ResolvedSkill> {
    let mut out = Vec::new();
    for r in refs {
        let r = r.trim();
        if r.is_empty() {
            continue;
        }
        let dir = if r.starts_with("http://") || r.starts_with("https://") {
            match fetch_git_skill(r) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("⚠️  team: skill '{}' fetch failed: {}", r, e);
                    continue;
                }
            }
        } else {
            let p = PathBuf::from(r);
            let p = if p.is_absolute() { p } else { PathBuf::from(team_dir).join(r) };
            if p.is_file() {
                p.parent().map(|x| x.to_path_buf()).unwrap_or(p)
            } else {
                p
            }
        };
        if !dir.exists() {
            eprintln!("⚠️  team: skill path not found: {}", dir.display());
            continue;
        }
        let (name, description) = read_skill_meta(&dir);
        out.push(ResolvedSkill { name, dir, description });
    }
    out
}

/// Parse SKILL.md YAML frontmatter for name/description (best-effort).
fn read_skill_meta(dir: &std::path::Path) -> (String, String) {
    let fallback = dir.file_name().and_then(|s| s.to_str()).unwrap_or("skill").to_string();
    let md = std::fs::read_to_string(dir.join("SKILL.md")).unwrap_or_default();
    let mut name = fallback;
    let mut desc = String::new();
    if let Some(rest) = md.strip_prefix("---") {
        if let Some(end) = rest.find("\n---") {
            let fm = rest[..end].trim_start_matches('\n');
            if let Ok(v) = serde_yml::from_str::<Value>(fm) {
                if let Some(n) = v.get("name").and_then(|x| x.as_str()) {
                    name = n.to_string();
                }
                if let Some(d) = v.get("description").and_then(|x| x.as_str()) {
                    desc = d.to_string();
                }
            }
        }
    }
    (name, desc)
}

/// Sparse-clone a GitHub `tree/<ref>/<subpath>` URL (or a bare repo URL) into the
/// shared skills cache and return the skill directory. Cache key = owner/repo/ref;
/// repeated refs to the same repo reuse the clone (sparse-checkout adds subpaths).
fn fetch_git_skill(url: &str) -> Result<PathBuf, String> {
    let (owner, repo, gitref, subpath) = parse_github(url)?;
    let repo_cache = skills_cache_dir().join(&owner).join(&repo).join(&gitref);
    let resolved = if subpath.is_empty() { repo_cache.clone() } else { repo_cache.join(&subpath) };
    // Cache hit: the subpath already materialised.
    if resolved.join("SKILL.md").is_file() || (subpath.is_empty() && resolved.exists()) {
        return Ok(resolved);
    }
    let repo_url = format!("https://github.com/{}/{}", owner, repo);
    if !repo_cache.join(".git").exists() {
        if let Some(p) = repo_cache.parent() {
            std::fs::create_dir_all(p).map_err(|e| e.to_string())?;
        }
        let _ = std::fs::remove_dir_all(&repo_cache);
        let out = std::process::Command::new("git")
            .args(["clone", "--depth", "1", "--filter=blob:none", "--sparse", "--branch", &gitref, &repo_url])
            .arg(&repo_cache)
            .output()
            .map_err(|e| format!("spawn git: {}", e))?;
        if !out.status.success() {
            return Err(format!("git clone: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
    }
    if !subpath.is_empty() {
        let out = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo_cache)
            .args(["sparse-checkout", "set", &subpath])
            .output()
            .map_err(|e| format!("spawn git: {}", e))?;
        if !out.status.success() {
            return Err(format!("git sparse-checkout: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
    }
    Ok(resolved)
}

/// Parse a GitHub URL into (owner, repo, ref, subpath). Supports the `tree/<ref>/
/// <subpath>` form and a bare `owner/repo` (defaults ref=main, no subpath).
fn parse_github(url: &str) -> Result<(String, String, String, String), String> {
    let u = url.trim().trim_end_matches('/');
    let rest = u
        .strip_prefix("https://github.com/")
        .or_else(|| u.strip_prefix("http://github.com/"))
        .ok_or_else(|| format!("only github.com skill URLs are supported: {}", url))?;
    let parts: Vec<&str> = rest.split('/').collect();
    if parts.len() < 2 {
        return Err(format!("expected github.com/owner/repo…: {}", url));
    }
    let owner = parts[0].to_string();
    let repo = parts[1].trim_end_matches(".git").to_string();
    if parts.len() >= 4 && (parts[2] == "tree" || parts[2] == "blob") {
        Ok((owner, repo, parts[3].to_string(), parts[4..].join("/")))
    } else {
        Ok((owner, repo, "main".to_string(), String::new()))
    }
}

/// Env the agent process exports so its `heartbeat.sh` hook can ping the daemon
/// (who am I, which room, where). Injected on EVERY backend's launch line.
fn hb_env(name: &str, room: &str, cfg: &TeamConfig) -> Vec<(String, String)> {
    vec![
        ("TEAM_HB_URL".to_string(), format!("{}/api/heartbeat", cfg.url)),
        ("TEAM_AGENT".to_string(), name.to_string()),
        ("TEAM_ROOM".to_string(), room.to_string()),
    ]
}

fn prepare_kiro(
    name: &str, role: &str, goal: &str, manage: bool,
    cfg: &TeamConfig, room: &str, paths: &Paths, model: Option<&str>, extras: &Extras,
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
    // by absolute path via `resources`. Each resolved skill is added natively as
    // a `skill://` resource (metadata at startup, content on demand).
    let mut resources = vec![format!("file://{}", paths.brief.to_string_lossy())];
    for sk in &extras.skills {
        resources.push(format!("skill://{}/SKILL.md", sk.dir.to_string_lossy()));
    }
    // The team MCP server plus any extra per-agent servers from the team.yaml.
    let mut mcp_servers = serde_json::json!({
        "team": { "url": format!("{}/mcp", cfg.url), "headers": { "x-agent": name, "x-room": room } }
    });
    {
        let obj = mcp_servers.as_object_mut().unwrap();
        for m in &extras.mcp {
            if !m.name.is_empty() {
                obj.insert(m.name.clone(), kiro_mcp_value(m));
            }
        }
    }
    let conf = serde_json::json!({
        "name": name,
        "description": format!("{} on the team bus", role),
        "prompt": full_prompt(role, goal),
        "tools": tools,
        "allowedTools": allowed,
        "resources": resources,
        "mcpServers": mcp_servers,
        "hooks": {
            "postToolUse": [ { "matcher": "*", "command": paths.heartbeat.to_string_lossy() } ],
            "userPromptSubmit": [ { "command": paths.heartbeat.to_string_lossy() } ],
            "stop": [ { "command": paths.keepalive.to_string_lossy() } ]
        },
    });
    std::fs::write(
        home.join("agents").join(format!("{}.json", name)),
        serde_json::to_string_pretty(&conf).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    let m = model.unwrap_or("claude-sonnet-4.6");
    let mut env = vec![("KIRO_HOME".to_string(), home.to_string_lossy().to_string())];
    env.extend(hb_env(name, room, cfg));
    env.extend(extras.env.iter().cloned());
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
    cfg: &TeamConfig, room: &str, paths: &Paths, model: Option<&str>, extras: &Extras,
) -> Result<Prepared, String> {
    let d = &paths.claude;
    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    let mcpfile = d.join(format!("{}.mcp.json", name));
    let mut mcp_servers = serde_json::json!({
        "team": { "type": "http", "url": format!("{}/mcp", cfg.url), "headers": { "x-agent": name, "x-room": room } }
    });
    {
        let obj = mcp_servers.as_object_mut().unwrap();
        for m in &extras.mcp {
            if !m.name.is_empty() {
                obj.insert(m.name.clone(), claude_mcp_value(m));
            }
        }
    }
    std::fs::write(
        &mcpfile,
        serde_json::to_string_pretty(&serde_json::json!({ "mcpServers": mcp_servers })).unwrap(),
    )
    .map_err(|e| e.to_string())?;
    let settingsfile = d.join(format!("{}.settings.json", name));
    std::fs::write(
        &settingsfile,
        serde_json::to_string_pretty(&serde_json::json!({
            "hooks": {
                "PostToolUse": [ { "matcher": "*", "hooks": [ { "type": "command", "command": paths.heartbeat.to_string_lossy() } ] } ],
                "UserPromptSubmit": [ { "hooks": [ { "type": "command", "command": paths.heartbeat.to_string_lossy() } ] } ],
                "Stop": [ { "hooks": [ { "type": "command", "command": paths.keepalive.to_string_lossy() } ] } ]
            }
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
    let first_msg = format!("{} {}{}", role_line(role, goal), kick_with_brief(paths), skills_index_text(&extras.skills));
    // Start interactive; then accept the folder-trust dialog, type the kick, submit.
    let post = vec![PostKey::Enter, PostKey::Text(first_msg), PostKey::Enter];
    let mut env = hb_env(name, room, cfg);
    env.extend(extras.env.iter().cloned());
    Ok((env, cmd, post))
}

// ---- Codex ----
#[allow(clippy::too_many_arguments)]
fn prepare_codex(
    name: &str, role: &str, goal: &str, manage: bool, cfg: &TeamConfig, room: &str, paths: &Paths, extras: &Extras,
) -> Result<Prepared, String> {
    let home = paths.codex.join(name);
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    let gating = if manage { "" } else { "disabled_tools = [\"hire\", \"fire\"]\n" };
    let mut config = format!(
        "[mcp_servers.team]\nurl = \"{}/mcp\"\nenabled = true\nexperimental_use_rmcp_client = true\n{}\n[mcp_servers.team.http_headers]\n\"x-agent\" = \"{}\"\n\"x-room\" = \"{}\"\n",
        cfg.url, gating, name, room
    );
    for m in &extras.mcp {
        if !m.name.is_empty() {
            config.push_str(&codex_mcp_toml(m));
        }
    }
    std::fs::write(home.join("config.toml"), config).map_err(|e| e.to_string())?;
    let mut env = vec![("CODEX_HOME".to_string(), home.to_string_lossy().to_string())];
    env.extend(hb_env(name, room, cfg));
    env.extend(extras.env.iter().cloned());
    let first_msg = format!("{} {}{}", role_line(role, goal), kick_with_brief(paths), skills_index_text(&extras.skills));
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

    #[test]
    fn parse_github_tree_url() {
        let (o, r, gr, sub) = parse_github(
            "https://github.com/anthropics/claude-code/tree/main/plugins/frontend-design/skills/frontend-design",
        )
        .unwrap();
        assert_eq!(o, "anthropics");
        assert_eq!(r, "claude-code");
        assert_eq!(gr, "main");
        assert_eq!(sub, "plugins/frontend-design/skills/frontend-design");
    }

    #[test]
    fn parse_github_bare_repo_defaults_main() {
        let (o, r, gr, sub) = parse_github("https://github.com/owner/repo").unwrap();
        assert_eq!((o.as_str(), r.as_str(), gr.as_str(), sub.as_str()), ("owner", "repo", "main", ""));
        assert!(parse_github("https://gitlab.com/x/y").is_err(), "only github.com supported");
    }

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
    fn mcp_value_remote_and_local_per_backend() {
        let remote = McpDef {
            name: "gh".into(),
            url: Some("https://x/mcp".into()),
            headers: [("Authorization".to_string(), "Bearer t".to_string())].into_iter().collect(),
            ..Default::default()
        };
        // kiro remote omits an explicit type; claude tags it http.
        assert!(kiro_mcp_value(&remote).get("type").is_none());
        assert_eq!(claude_mcp_value(&remote)["type"], "http");
        assert_eq!(kiro_mcp_value(&remote)["url"], "https://x/mcp");

        let local = McpDef { name: "pg".into(), command: Some("mcp-pg".into()), args: vec!["--stdio".into()], ..Default::default() };
        let toml = codex_mcp_toml(&local);
        assert!(toml.contains("[mcp_servers.pg]"));
        assert!(toml.contains("command = \"mcp-pg\""));
        assert!(toml.contains("args = [\"--stdio\"]"));
    }

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
    fn builtin_default_template_is_minimal_manager_worker() {
        // The default template is the minimal demo: a manager + one worker that
        // shows the delegate→report loop and can grow via the manager's hire().
        let v: Value = serde_yml::from_str(BUILTIN_TEMPLATE).unwrap();
        let agents = v["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 2, "minimal demo = manager + worker");
        let names: Vec<&str> = agents.iter().filter_map(|a| a["name"].as_str()).collect();
        assert!(names.contains(&"manager") && names.contains(&"worker"));
        let managers = agents.iter().filter(|a| a["manage"] == true).count();
        assert_eq!(managers, 1, "exactly one manager");
        assert!(agents.iter().all(|a| a["model"] == ""), "models use the server default");
    }

    #[test]
    fn software_dev_template_roster_and_tools() {
        // The software-dev roster is a built-in (teams/software-dev/team.yaml).
        let v: Value = serde_yml::from_str(SOFTWARE_DEV_TEMPLATE).unwrap();
        let agents = v["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 8, "manager+product+architect+frontend+backend+reviewer+tester+devops");
        let names: Vec<&str> = agents.iter().filter_map(|a| a["name"].as_str()).collect();
        for expected in ["manager", "product", "architect", "frontend", "backend", "reviewer", "tester", "devops"] {
            assert!(names.contains(&expected), "missing role '{expected}': {names:?}");
        }
        let managers = agents.iter().filter(|a| a["manage"] == true).count();
        assert_eq!(managers, 0, "hire/fire off: no manage=true agent");
        // Workflow lives in each role's goal (AGENTS.md is contract-only), so
        // every agent must carry a substantive goal.
        assert!(
            agents.iter().all(|a| a["goal"].as_str().map(|g| g.len() > 80).unwrap_or(false)),
            "each role's goal must carry its slice of the workflow"
        );
        assert!(agents.iter().all(|a| a["model"] == "auto"), "all models pinned to auto");

        // Per-agent tools wired via the new schema.
        let agent = |n: &str| agents.iter().find(|a| a["name"] == n).unwrap();
        let mcp_names = |n: &str| -> Vec<String> {
            agent(n)["mcp"].as_array().map(|a| {
                a.iter().filter_map(|m| m["name"].as_str().map(String::from)).collect()
            }).unwrap_or_default()
        };
        for dev in ["architect", "frontend", "backend", "reviewer", "tester", "devops"] {
            assert!(mcp_names(dev).contains(&"context7".to_string()), "{dev} should have context7");
        }
        // architect/backend/reviewer/devops reach the AWS knowledge base.
        for n in ["architect", "backend", "reviewer", "devops"] {
            assert!(mcp_names(n).contains(&"aws-knowledge".to_string()), "{n} has AWS knowledge");
        }
        assert!(mcp_names("tester").contains(&"chrome-devtools".to_string()), "tester has chrome-devtools for e2e");
        let fe_skills = agent("frontend")["skills"].as_array().unwrap();
        assert!(
            fe_skills.iter().any(|s| s.as_str().map(|x| x.contains("frontend-design")).unwrap_or(false)),
            "frontend has the frontend-design skill"
        );
    }

    #[test]
    fn both_builtin_templates_are_seeded() {
        // Isolate config dir so we don't touch the real ~/.config.
        let dir = std::env::temp_dir().join(format!("teamtest-tpl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);

        ensure_templates_seeded();
        let mut names = list_templates();
        names.sort();
        for expected in ["default", "software-dev", "financial-research", "deep-research", "content-studio", "data-analysis"] {
            assert!(names.contains(&expected.to_string()), "{expected} seeded: {names:?}");
        }

        // Each built-in parses and every agent carries a substantive goal.
        for (name, body) in BUILTIN_TEMPLATES {
            let v: Value = serde_yml::from_str(body).unwrap_or_else(|e| panic!("{name} bad yaml: {e}"));
            let agents = v["agents"].as_array().unwrap_or_else(|| panic!("{name} has no agents"));
            assert!(!agents.is_empty(), "{name} empty roster");
            assert!(
                agents.iter().all(|a| a["goal"].as_str().map(|g| g.len() > 80).unwrap_or(false)),
                "{name}: every role needs a substantive goal"
            );
        }

        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn financial_research_template_has_lead_and_default_models() {
        let v: Value = serde_yml::from_str(FINANCIAL_RESEARCH_TEMPLATE).unwrap();
        let agents = v["agents"].as_array().unwrap();
        assert!(agents.len() >= 5, "a multi-analyst research team");
        let names: Vec<&str> = agents.iter().filter_map(|a| a["name"].as_str()).collect();
        for expected in ["lead", "fundamentals", "valuation", "memo", "reviewer"] {
            assert!(names.contains(&expected), "missing role '{expected}': {names:?}");
        }
        let managers = agents.iter().filter(|a| a["manage"] == true).count();
        assert_eq!(managers, 0, "hire/fire off: no manage=true agent");
        assert!(agents.iter().all(|a| a["model"] == ""), "models use the server default");
        assert!(
            agents.iter().all(|a| a["goal"].as_str().map(|g| g.len() > 80).unwrap_or(false)),
            "each role's goal must carry its methodology"
        );
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
        // Built-in templates now pin "auto", so the empty-model fallback is for
        // USER-authored templates that leave model blank. Verify with a custom one.
        let dir = std::env::temp_dir().join(format!("teamtest-model-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);

        let agents = serde_json::json!([
            { "name": "w", "backend": "kiro", "role": "w", "goal": "do things well", "model": "", "manage": false }
        ]);
        save_template("blankmodel", &agents).unwrap();
        let b = RecordingBridge { seeded: Mutex::new(vec![]), existing: vec![] };
        seed_template(&b, "myroom", "blankmodel", &cfg());
        let seeded = b.seeded.lock().unwrap();
        assert!(!seeded.is_empty(), "custom template should seed agents");
        // Empty model on a kiro agent inherits the server default.
        assert!(seeded.iter().all(|(_, s)| s["model"] == "claude-sonnet-4.6"));

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
        // A multi-line goal must flatten to one line (it's typed as a first
        // message to claude/codex, where an embedded \n would submit early).
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
        prepare_home(&paths, "").unwrap();
        let brief = std::fs::read_to_string(&paths.brief).unwrap();
        assert!(brief.starts_with("Respond in English. Be terse."), "system prompt must lead the brief");
        assert!(brief.contains("collaboration playbook") || brief.contains("AGENTS.md") || brief.contains("team group chat"),
            "brief body must still be present");

        // Clearing it leaves the brief as just the built-in.
        save_system_prompt("").unwrap();
        prepare_home(&paths, "").unwrap();
        let brief2 = std::fs::read_to_string(&paths.brief).unwrap();
        assert!(!brief2.starts_with("Respond in English"));

        std::env::remove_var("XDG_CONFIG_HOME");
        let _ = std::fs::remove_dir_all(&dir);
    }
}


