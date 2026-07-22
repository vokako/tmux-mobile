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
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

// Shared hooks, embedded so a packaged .app has no external file dependency.
// Written to the self-gitignored Team runtime directory at startup.
const KEEPALIVE_SH: &str = include_str!("../../../team/hooks/keepalive.sh");
const HEARTBEAT_SH: &str = include_str!("../../../team/hooks/heartbeat.sh");

const RECONCILE_INTERVAL: Duration = Duration::from_secs(3);
/// Kiro 2.12 caps a configured MCP timeout at ten minutes. Keep every backend
/// on that shared boundary so the server can leave a full minute for delivery.
const TEAM_MCP_TOOL_TIMEOUT_MS: u64 = 600_000;
/// Self-heal threshold: no `wait` and no heartbeat for this long ⇒ the agent is
/// wedged; the supervisor nudges its window (Esc + reconnect re-prompt). Well
/// above the bus's 90s `unreachable` mark so we only ever auto-restart an agent
/// that has been silent for a genuinely long time, not one merely between tools.
const RECOVERY_STALE_MS: i64 = 1_800_000; // 30 minutes
/// A dead parked `wait` is distinguishable from real work: the bus exposes an
/// idle/online row as `stalled` once its 15-second refresh stops for 90 seconds,
/// while a silent working/thinking row remains `hardworking` for 30 minutes.
/// Recover the former promptly instead of making a human message wait 30 min.
const WAIT_RECOVERY_STALE_MS: i64 = 90_000;
/// Avoid repeatedly interrupting a pane if reconnect itself cannot make
/// progress. A successful `wait` refreshes last_seen and naturally rearms this.
const RECOVERY_COOLDOWN_MS: i64 = 5 * 60 * 1000;
/// Allow one 15-second wait heartbeat after a backend restart before deciding
/// that an adopted idle call is still attached to the dead daemon.
const RESTART_RECOVERY_GRACE: Duration = Duration::from_secs(20);
const RESTART_TRUST_CHECK_DELAY: Duration = Duration::from_secs(2);
/// Idle-sleep threshold: when EVERY non-offline agent has been parked in `wait`
/// (status=`idle`) for this long, the supervisor sends Esc to each pane to
/// cancel the in-flight `wait` MCP call. The agent's CLI falls back to its
/// shell prompt, so an empty wait never completes and consumes another turn.
/// Any new message in the room (typically the human resuming) wakes the team
/// back up via the standard reconnect nudge. Set to 0 to disable.
const IDLE_SLEEP_MS: i64 = agora::mcp::MCP_WAIT_MAX_MS as i64 - 60_000;

// ─── Team templates (named rosters under <config>/tmux-mobile/teams/) ──────
// A template is a JSON file `teams/<name>.json` = { "agents": [ {name, backend,
// role, goal, model, manage}, … ] }. The user edits these from the
// app (Templates panel); `start_team` seeds the chosen template into the room.
// The built-in default is written to teams/default.json on first run so there
// is always something to edit.

/// Default model placeholder substituted in when a kiro agent leaves model empty.
pub const BUILTIN_TEMPLATE: &str = include_str!("../../../team/templates/default/team.yaml");

/// A ready-made software-development roster (tech-lead / product / architect /
/// coder / reviewer / tester), seeded alongside the default so it appears in
/// the app's template picker out of the box. The whole collaboration workflow
/// lives in each agent's `goal` (role isolation) — team-brief.md stays a
/// role-agnostic, workflow-free communication contract.
pub const SOFTWARE_DEV_TEMPLATE: &str = include_str!("../../../team/templates/software-dev/team.yaml");

/// A financial-research roster modeled on Dexter (virattt/dexter): a research
/// director plus fundamentals / market+sentiment / valuation(DCF) / memo /
/// reviewer analysts. Dexter's single-agent skills (DCF, investment memo, X
/// sentiment) become specialist roles; its data discipline (figures carry
/// sources, the deliverable is a file, chat is a scannable header) and its
/// educational-only / not-investment-advice posture are baked into the goals.
pub const FINANCIAL_RESEARCH_TEMPLATE: &str =
    include_str!("../../../team/templates/financial-research/team.yaml");

/// A deep-research roster: a director who decomposes the question, two parallel
/// researchers, a synthesist, and a skeptic — every claim sourced, output to
/// report.md.
pub const DEEP_RESEARCH_TEMPLATE: &str = include_str!("../../../team/templates/deep-research/team.yaml");

/// A content-studio roster (editor-in-chief / researcher / writer / copy editor)
/// for shipping a publish-ready article or docs in a shared house style.
pub const CONTENT_STUDIO_TEMPLATE: &str = include_str!("../../../team/templates/content-studio/team.yaml");

/// A data-analysis roster (lead / data engineer / analyst / reporter) that
/// answers a question from data with reproducible work and honest caveats.
pub const DATA_ANALYSIS_TEMPLATE: &str = include_str!("../../../team/templates/data-analysis/team.yaml");

/// A lean mixed-backend engineering roster: Kiro coordinates requirements and
/// delivery, Claude designs and reviews, and Codex implements and verifies.
pub const MIXED_ENGINEERING_TEMPLATE: &str =
    include_str!("../../../team/templates/mixed-engineering/team.yaml");

/// Built-in templates seeded into teams/ on first run: (file stem, contents).
const BUILTIN_TEMPLATES: &[(&str, &str)] = &[
    ("default", BUILTIN_TEMPLATE),
    ("software-dev", SOFTWARE_DEV_TEMPLATE),
    ("financial-research", FINANCIAL_RESEARCH_TEMPLATE),
    ("deep-research", DEEP_RESEARCH_TEMPLATE),
    ("content-studio", CONTENT_STUDIO_TEMPLATE),
    ("data-analysis", DATA_ANALYSIS_TEMPLATE),
    ("mixed-engineering", MIXED_ENGINEERING_TEMPLATE),
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

/// Per-team config home under `<workspace>/.tmm/`. Lives inside the project but
/// is self-gitignored (`.tmm/.gitignore` = `*`). Each backend's config files and
/// hooks live here; prompts are passed inline.
struct Paths {
    /// Agents' working directory (the user's project) — agents run `-c` here.
    workspace: PathBuf,
    /// Our private config root: `.tmm/` for legacy rooms, otherwise
    /// `<workspace>/.tmm/teams/<team-id>/`.
    home: PathBuf,
    kiro: PathBuf,
    claude: PathBuf,
    codex: PathBuf,
    keepalive: PathBuf,
    heartbeat: PathBuf,
}

impl Paths {
    fn new(workspace: &str, room: &str) -> Self {
        let home = team_runtime_dir(workspace, room);
        Paths {
            workspace: PathBuf::from(workspace),
            home: home.clone(),
            kiro: home.join("kiro"),
            claude: home.join("claude"),
            codex: home.join("codex"),
            keepalive: home.join("keepalive.sh"),
            heartbeat: home.join("heartbeat.sh"),
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

/// Sanitize a workspace path into a tmux-safe slug. The slug = sanitized
/// basename + 6-char hash of the full canonical path. This guarantees uniqueness
/// even when two workspaces share a basename (e.g. `/a/demo` vs `/b/demo`).
/// tmux session names can't contain ':' or '.'.
pub fn workspace_slug(workspace: &str) -> String {
    use sha2::{Sha256, Digest};
    let canonical = std::fs::canonicalize(workspace)
        .unwrap_or_else(|_| std::path::PathBuf::from(workspace));
    let base = canonical.file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("root");
    let mut name: String = base
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '-' || c == '_' { c } else { '-' })
        .collect();
    name.make_ascii_lowercase();
    let name = name.trim_matches('-');
    let name = if name.is_empty() { "root" } else { &name[..name.len().min(24)] };
    // 6 hex chars of SHA-256 of full path → 16M buckets, collision-free in practice.
    let hash = format!("{:x}", Sha256::digest(canonical.to_string_lossy().as_bytes()));
    format!("{}-{}", name, &hash[..6])
}

/// Stable identity for one Team instance. A workspace may run several
/// templates concurrently; the pair, rather than the workspace alone, is the
/// durable identity used by SQLite, tmux, runtime files, and history.
pub fn team_slug(workspace: &str, template: &str) -> String {
    use sha2::{Digest, Sha256};
    let canonical = std::fs::canonicalize(workspace)
        .unwrap_or_else(|_| PathBuf::from(workspace));
    let workspace_name = canonical
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("root");
    let workspace_name = slug_component(workspace_name, 20, "root");
    let template_name = slug_component(template.trim(), 16, "default");
    let mut hasher = Sha256::new();
    hasher.update(canonical.to_string_lossy().as_bytes());
    hasher.update([0]);
    hasher.update(template.trim().as_bytes());
    let hash = format!("{:x}", hasher.finalize());
    format!("{}-{}-{}", workspace_name, template_name, &hash[..8])
}

fn slug_component(value: &str, max_len: usize, fallback: &str) -> String {
    let mut value: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    value.truncate(max_len);
    let value = value.trim_matches('-');
    if value.is_empty() { fallback.to_string() } else { value.to_string() }
}

pub fn same_workspace(left: &str, right: &str) -> bool {
    let normalize = |value: &str| {
        std::fs::canonicalize(value).unwrap_or_else(|_| PathBuf::from(value))
    };
    normalize(left) == normalize(right)
}

/// Runtime root shared by every stateful surface of one Team. Workspace-only
/// room IDs are the pre-instance-ID format and retain the root `.tmm` layout so
/// a recovered live CLI is never moved underneath its process.
pub fn team_runtime_dir(workspace: &str, room: &str) -> PathBuf {
    let root = PathBuf::from(workspace).join(".tmm");
    if room == workspace_slug(workspace) {
        root
    } else {
        root.join("teams").join(runtime_segment(room))
    }
}

fn runtime_segment(room: &str) -> String {
    use sha2::{Digest, Sha256};
    if !room.is_empty()
        && room != "."
        && room != ".."
        && room
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'))
    {
        room.to_string()
    } else {
        let hash = format!("{:x}", Sha256::digest(room.as_bytes()));
        format!("team-{}", &hash[..12])
    }
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

/// Write hooks into our private per-team home (`<workspace>/.tmm/`).
/// The agent prompt is now fully inline (no external brief file).
fn prepare_home(p: &Paths) -> std::io::Result<()> {
    let tmm_dir = p.workspace.join(".tmm");
    std::fs::create_dir_all(&tmm_dir)?;
    // Self-gitignore: `.tmm/.gitignore` = `*`
    let gi = tmm_dir.join(".gitignore");
    if !gi.exists() {
        std::fs::write(&gi, "*\n")?;
    }
    std::fs::create_dir_all(&p.home)?;
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

/// Adopt the pre-0.6 Kiro home only when no canonical home exists. This runs
/// immediately before a new Kiro launch, never while merely adopting a pane
/// that may still have `KIRO_HOME` pointed at the legacy directory.
fn prepare_kiro_home(p: &Paths) -> std::io::Result<()> {
    let legacy = p.workspace.join(".tmm").join("kiro-home");
    if p.home == p.workspace.join(".tmm") && legacy.exists() && !p.kiro.exists() {
        std::fs::rename(&legacy, &p.kiro)?;
    }
    std::fs::create_dir_all(&p.kiro)
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

/// Reconcile the desired roster into real agent windows, forever (until the
/// process exits). `launched` maps a name → its pane id (or None if adopted).
///
/// Idempotency (the dup-window fix): before launching, we check tmux for an
/// EXISTING window named after the agent in this session. If one is there
/// (server restarted, agent already running), we adopt it instead of opening a
/// second. The previous in-memory-only tracking re-launched every agent on
/// restart, piling up duplicate manager/worker/reviewer windows. Reconnecting
/// adopted agents after a *server* restart is handled separately, once, by
/// `nudge_adopted_agents` (called from recovery) — not here, because the loop's
/// presence check can't tell a healthy agent from one hung on a dead socket.
async fn reconcile_loop(bridge: Arc<dyn TeamBridge>, cfg: TeamConfig, room: String, session: String, paths: Paths) {
    const MAX_LAUNCH_FAILURES: u32 = 3; // give up relaunching an agent after this
    let mut launched: HashMap<String, Option<String>> = HashMap::new();
    let mut fail_count: HashMap<String, u32> = HashMap::new();
    let mut last_nudge: HashMap<String, i64> = HashMap::new(); // self-heal cooldown
    let mut launched_any = false;
    let mut sleep_state = SleepState::default();
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
        // ── Sleep / wake ─────────────────────────────────────────────────
        // Once every non-offline agent has been parked in `wait` (status =
        // "idle") for IDLE_SLEEP_MS, send Esc to each pane to cancel the
        // in-flight wait. The CLI returns to its shell prompt and stops
        // burning a fresh LLM turn every 50s. Any new bus message — typically
        // the human resuming — wakes the team back up via `nudge_pane`. Skip
        // the existing self-heal while slept: this silence is intentional, not
        // a wedged agent.
        let now = now_ms();
        let online_idle = sleep_state.is_online_idle(&roster);
        let latest_seq = latest_room_seq(&*bridge, &room);
        match sleep_state.step(now, online_idle, latest_seq, IDLE_SLEEP_MS) {
            SleepAction::Sleep => {
                println!(
                    "🜂 team: room '{}' all-idle ≥ {}s — sending Esc to {} agents to sleep",
                    room,
                    IDLE_SLEEP_MS / 1000,
                    employees.iter().filter(|(_, _, s)| s != "disabled").count(),
                );
                for (name, _, state) in &employees {
                    if state == "disabled" { continue; }
                    if let Some(pane) = tmux::find_window_by_name(&session, name) {
                        let _ = tmux::send_keys(&pane, "Escape", false);
                    }
                }
            }
            SleepAction::Wake => {
                println!("🜂 team: room '{}' new message during sleep — waking agents", room);
                for (name, _, state) in &employees {
                    if state == "disabled" { continue; }
                    // Clear the sleep label immediately for a responsive UI; the
                    // agent's own fresh `wait` will refine it to idle/thinking.
                    let _ = bridge.set_agent_status(&room, name, "idle");
                    if let Some(pane) = tmux::find_window_by_name(&session, name) {
                        tokio::spawn(async move { nudge_pane(&pane).await; });
                    }
                }
            }
            SleepAction::None => {}
        }

        // Re-assert "sleeping" every tick while slept. Our Esc takes ~1-2s to
        // actually cancel the agent's in-flight `wait`; in that window the wait
        // loop parks at least once more and writes "idle" (refreshing last_seen),
        // clobbering the status we just set. Setting it only once on the Sleep
        // tick therefore loses the race — the stale "idle" then ages into
        // "stalled" (red) after 90s, which is exactly the bug we saw. Re-stamping
        // it on every 3s tick wins: by the next tick the wait is truly stopped,
        // nothing else writes the row, and apply_presence never ages "sleeping"
        // itself, so the label sticks. Idempotent + cheap (one UPDATE per agent).
        if sleep_state.slept {
            for (name, _, state) in &employees {
                if state == "disabled" { continue; }
                let _ = bridge.set_agent_status(&room, name, "sleeping");
            }
        }

        // ── Self-heal backstop ────────────────────────────────────────────
        // A parked wait that stops its 15-second refresh is exposed by the bus
        // as `stalled` after 90 seconds. Recover that case promptly: a dead MCP
        // transport can otherwise leave persisted human messages unread for
        // the old 30-minute generic threshold. A working/thinking agent instead
        // becomes `hardworking` at 90 seconds and is still left alone until the
        // 30-minute backstop. Skipped while slept: that silence is intentional.
        if !sleep_state.slept {
            for (name, (status, last_seen)) in &roster {
                if !should_recover_agent(status, *last_seen, now, last_nudge.get(name).copied()) {
                    continue;
                }
                if let Some(pane) = tmux::find_window_by_name(&session, name) {
                    println!(
                        "🜂 team: agent '{}' {} for {}s — self-heal nudging {}",
                        name,
                        status,
                        (now - last_seen) / 1000,
                        pane
                    );
                    tokio::spawn(async move { nudge_pane(&pane).await; });
                    last_nudge.insert(name.clone(), now);
                }
            }
        }

        tokio::time::sleep(RECONCILE_INTERVAL).await;
    }
}

fn should_recover_agent(status: &str, last_seen: i64, now: i64, last_nudge: Option<i64>) -> bool {
    if status == "offline" || status == "sleeping" {
        return false;
    }
    if now - last_nudge.unwrap_or(0) < RECOVERY_COOLDOWN_MS {
        return false;
    }
    let stale_for = now - last_seen;
    if status == "stalled" {
        stale_for >= WAIT_RECOVERY_STALE_MS
    } else {
        stale_for >= RECOVERY_STALE_MS
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

/// Latest message sequence number in `room`, or 0 if the bus is empty / the
/// room hasn't logged anything yet. Used as the anchor we compare against
/// while slept: any seq strictly greater than the anchor means a new message
/// has arrived (the human resuming, almost always) and the team should wake.
fn latest_room_seq(bridge: &dyn TeamBridge, room: &str) -> i64 {
    bridge
        .history(room, 1)
        .get("messages")
        .and_then(|v| v.as_array())
        .and_then(|arr| arr.last())
        .and_then(|m| m.get("seq"))
        .and_then(|v| v.as_i64())
        .unwrap_or(0)
}

/// What `SleepState::step` decided this tick. The reconcile loop translates
/// each action into pane keystrokes (Esc to sleep, the standard reconnect
/// nudge to wake); separating decision from effect keeps the state machine
/// trivial to unit-test without tmux.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SleepAction {
    /// Nothing to do this tick.
    None,
    /// All agents have been idle past the threshold — send Esc to every pane.
    Sleep,
    /// We're slept and a new bus message just arrived — re-prompt every pane
    /// back into `wait` (the existing `nudge_pane` primitive).
    Wake,
}

/// State for the idle-sleep / new-message-wake state machine that lives inside
/// `reconcile_loop`. Pulled into its own type so the decision logic can be
/// covered by plain unit tests (no tmux, no bus). Once `slept` is set, only a
/// new bus message clears it: status changes after sleep are EXPECTED (Esc'd
/// agents have no live `wait`, so they age to "stalled" within ~90s) and must
/// not be treated as a wake signal, otherwise we would oscillate.
#[derive(Debug, Default, Clone, Copy)]
struct SleepState {
    /// First tick on which all online agents looked idle, or `None` if any of
    /// them has been doing real work since.
    idle_since: Option<i64>,
    /// True between the Esc-everyone tick and the wake tick.
    slept: bool,
    /// Latest bus seq we saw at the moment we went to sleep. Wake when the
    /// real latest is strictly greater than this.
    sleep_anchor_seq: i64,
}

impl SleepState {
    /// Advance one tick. `online_idle` is "every non-offline agent has status
    /// `idle`" (caller computes from the roster); `latest_seq` is the latest
    /// bus sequence; `threshold_ms` is `IDLE_SLEEP_MS` (test injection).
    fn step(&mut self, now: i64, online_idle: bool, latest_seq: i64, threshold_ms: i64) -> SleepAction {
        if self.slept {
            // Once slept, ONLY a new message wakes us. (Status drift to
            // "stalled" while the wait MCP call is cancelled is expected.)
            if latest_seq > self.sleep_anchor_seq {
                self.slept = false;
                self.idle_since = None;
                self.sleep_anchor_seq = latest_seq;
                return SleepAction::Wake;
            }
            return SleepAction::None;
        }
        if !online_idle {
            self.idle_since = None;
            return SleepAction::None;
        }
        // Every online agent is idle. Start the clock if we haven't already,
        // then trip Sleep once the threshold is met.
        let started = *self.idle_since.get_or_insert(now);
        if threshold_ms > 0 && now - started >= threshold_ms {
            self.slept = true;
            self.sleep_anchor_seq = latest_seq;
            return SleepAction::Sleep;
        }
        SleepAction::None
    }

    /// Convenience: derive `online_idle` from the supervisor's roster snapshot
    /// (`roster_liveness` output). Empty room or no online agent ⇒ false (so
    /// we never sleep a team that hasn't even come online yet).
    fn is_online_idle(&self, roster: &HashMap<String, (String, i64)>) -> bool {
        let mut saw_online = false;
        for (_, (status, _)) in roster {
            if status == "offline" { continue; }
            saw_online = true;
            if status != "idle" { return false; }
        }
        saw_online
    }
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

/// After a *server* restart, reconnect adopted agents whose idle `wait` is
/// still attached to the dead daemon.
///
/// A recovered agent's MCP client lost its connection to the old (now dead)
/// daemon and is hung inside a `wait` tool call. Verified with kiro-cli 2.7.0:
/// the client neither times out nor retries on its own — but it reconnects fine
/// once the dead call is cancelled and a new turn starts. Working agents must
/// not receive Escape: recovery snapshots presence, allows one normal heartbeat
/// interval, and nudges only idle-like agents whose `last_seen` did not advance.
///
/// Runs in a spawned task: it sleeps between keystrokes (TUI needs a beat to
/// settle) and we must not block the recovery path.
pub fn nudge_adopted_agents(
    bridge: Arc<dyn TeamBridge>,
    room: String,
    windows: Vec<(String, String)>,
) {
    let before = roster_liveness(&*bridge, &room);
    tokio::spawn(async move {
        // Trust prompts do not join the roster, so handle them independently
        // while the presence grace period is still running.
        tokio::time::sleep(RESTART_TRUST_CHECK_DELAY).await;
        for (name, pane) in &windows {
            if name != "zsh" {
                confirm_folder_trust_if_visible(pane);
            }
        }

        tokio::time::sleep(RESTART_RECOVERY_GRACE - RESTART_TRUST_CHECK_DELAY).await;
        for (name, pane) in windows {
            if name == "zsh" {
                continue; // the session's initial shell, not an agent
            }
            if confirm_folder_trust_if_visible(&pane) {
                continue;
            }
            let prior = before.get(&name).map(|(status, seen)| (status.as_str(), *seen));
            // Read immediately before acting so a heartbeat that arrives while
            // an earlier pane is being handled also protects this pane.
            let current_roster = roster_liveness(&*bridge, &room);
            let current = current_roster.get(&name).map(|(status, seen)| (status.as_str(), *seen));
            if !should_reconnect_adopted_agent(prior, current) {
                println!("🜂 team: adopted agent '{}' is active or recovered; leaving {} untouched", name, pane);
                continue;
            }
            println!("🜂 team: adopted agent '{}' has an unchanged idle wait; reconnecting {}", name, pane);
            nudge_pane(&pane).await;
        }
    });
}

fn should_reconnect_adopted_agent(
    before: Option<(&str, i64)>,
    after: Option<(&str, i64)>,
) -> bool {
    let Some((before_status, before_seen)) = before else { return false };
    let Some((after_status, after_seen)) = after else { return false };
    let idle_like = |status: &str| matches!(status, "idle" | "online" | "stalled");
    idle_like(before_status) && idle_like(after_status) && after_seen <= before_seen
}

/// Keep recovery neutral: the shared Team contract tells an idle agent to
/// return to `wait`, while an agent with pending context resumes that work.
const RECONNECT_NUDGE: &str = "Continue.";

fn confirm_folder_trust_if_visible(pane: &str) -> bool {
    if let Ok(content) = tmux::capture_pane_plain(pane, Some(80)) {
        if folder_trust_prompt_visible(&content) {
            println!("🜂 team: confirming folder trust in recovered pane {}", pane);
            let _ = tmux::send_keys(pane, "Enter", false);
            return true;
        }
    }
    false
}

/// Press Esc (cancel any stuck in-flight call → back to the prompt), then send
/// the reconnect re-prompt and submit it. Shared by restart-recovery and the
/// supervisor's liveness self-heal. Sleeps between keystrokes (the TUI needs a
/// beat to settle), so callers run it inside a spawned task.
async fn nudge_pane(pane: &str) {
    if confirm_folder_trust_if_visible(pane) {
        return;
    }
    let _ = tmux::send_keys(pane, "Escape", false);
    tokio::time::sleep(Duration::from_millis(500)).await;
    let _ = tmux::send_keys(pane, RECONNECT_NUDGE, true);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let _ = tmux::send_keys(pane, "Enter", false);
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupConfirmation {
    markers: Vec<&'static str>,
    ready_markers: Vec<&'static str>,
    timeout: Duration,
}

/// What a backend `prepare_*` returns: (env vars, launch command, post-launch
/// confirmation). Aliased to keep the per-backend signatures readable.
type Prepared = (Vec<(String, String)>, String, Option<StartupConfirmation>);

const CLAUDE_FOLDER_TRUST_MARKERS: &[&str] = &[
    "Accessing workspace:",
    "Yes, I trust this folder",
    "Enter to confirm",
];

const CODEX_FOLDER_TRUST_MARKERS: &[&str] = &[
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
#[derive(Default)]
struct Extras {
    env: Vec<(String, String)>,
    mcp: Vec<McpDef>,
    skills: Vec<ResolvedSkill>,
}

fn env_reference(value: &str) -> Option<&str> {
    let name = value
        .strip_prefix("${")
        .and_then(|s| s.strip_suffix('}'))
        .or_else(|| value.strip_prefix('$'))?;
    let mut chars = name.chars();
    let first = chars.next()?;
    if (first == '_' || first.is_ascii_alphabetic())
        && chars.all(|c| c == '_' || c.is_ascii_alphanumeric())
    {
        Some(name)
    } else {
        None
    }
}

fn header_env_reference(value: &str) -> Option<(&str, bool)> {
    if let Some(name) = value.strip_prefix("Bearer ").and_then(env_reference) {
        Some((name, true))
    } else {
        env_reference(value).map(|name| (name, false))
    }
}

fn interpolated_headers(headers: &std::collections::BTreeMap<String, String>) -> Value {
    let values: std::collections::BTreeMap<String, String> = headers
        .iter()
        .map(|(key, value)| {
            let value = match header_env_reference(value) {
                Some((name, true)) => format!("Bearer ${{{name}}}"),
                Some((name, false)) => format!("${{{name}}}"),
                None => value.clone(),
            };
            (key.clone(), value)
        })
        .collect();
    serde_json::to_value(values).unwrap_or(Value::Null)
}

/// kiro mcpServers entry: remote = `{url,headers}`, local = `{command,args,env}`.
fn kiro_mcp_value(m: &McpDef) -> Value {
    if let Some(url) = &m.url {
        let mut o = serde_json::json!({ "url": url });
        if !m.headers.is_empty() {
            o["headers"] = interpolated_headers(&m.headers);
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

fn codex_key_segment(value: &str) -> String {
    if !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        value.to_string()
    } else {
        serde_json::to_string(value).unwrap()
    }
}

fn codex_config_override(key: &str, value: Value) -> String {
    let assignment = format!("{}={}", key, serde_json::to_string(&value).unwrap());
    format!("-c {}", shell_quote(&assignment))
}

/// Codex CLI overrides for one extra MCP server. Team keeps the system
/// config.toml intact and layers room-specific MCP settings at launch.
fn codex_mcp_overrides(m: &McpDef) -> Vec<String> {
    let name = codex_key_segment(&m.name);
    let prefix = format!("mcp_servers.{}", name);
    let mut args = Vec::new();
    if let Some(url) = &m.url {
        args.push(codex_config_override(&format!("{}.url", prefix), Value::String(url.clone())));
        args.push(codex_config_override(&format!("{}.enabled", prefix), Value::Bool(true)));
        args.push(codex_config_override(
            &format!("{}.experimental_use_rmcp_client", prefix),
            Value::Bool(true),
        ));
        for (key, value) in &m.headers {
            match header_env_reference(value) {
                Some((name, true)) if key.eq_ignore_ascii_case("authorization") => {
                    args.push(codex_config_override(
                        &format!("{}.bearer_token_env_var", prefix),
                        Value::String(name.to_string()),
                    ));
                }
                Some((name, false)) => {
                    args.push(codex_config_override(
                        &format!("{}.env_http_headers.{}", prefix, codex_key_segment(key)),
                        Value::String(name.to_string()),
                    ));
                }
                _ => {
                    args.push(codex_config_override(
                        &format!("{}.http_headers.{}", prefix, codex_key_segment(key)),
                        Value::String(value.clone()),
                    ));
                }
            }
        }
    } else if let Some(cmd) = &m.command {
        args.push(codex_config_override(
            &format!("{}.command", prefix),
            Value::String(cmd.clone()),
        ));
        if !m.args.is_empty() {
            args.push(codex_config_override(
                &format!("{}.args", prefix),
                serde_json::to_value(&m.args).unwrap(),
            ));
        }
        for (key, value) in &m.env {
            args.push(codex_config_override(
                &format!("{}.env.{}", prefix, codex_key_segment(key)),
                Value::String(value.clone()),
            ));
        }
    }
    args
}

fn codex_team_mcp_overrides(m: &McpDef) -> Vec<String> {
    let mut args = codex_mcp_overrides(m);
    args.push(codex_config_override(
        "mcp_servers.team.tool_timeout_sec",
        Value::from(team_mcp_tool_timeout_ms() / 1000),
    ));
    args
}

fn team_mcp_tool_timeout_ms() -> u64 {
    TEAM_MCP_TOOL_TIMEOUT_MS
}

/// A compact system-level skills index for backends without a native skill
/// mechanism (claude/codex). Kiro instead gets `skill://` resources.
fn skills_index_text(skills: &[ResolvedSkill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut s = String::from("Skills available — read the named SKILL.md before a matching task:");
    for sk in skills {
        s += &format!(" [{}] {} (at {}/SKILL.md);", sk.name, sk.description, sk.dir.display());
    }
    s
}

fn build_cli_system_prompt(
    role: &str,
    goal: &str,
    team_prompt: &str,
    cfg: &TeamConfig,
    skills: &[ResolvedSkill],
) -> String {
    let mut prompt = build_agent_prompt(role, goal, team_prompt, cfg);
    let skills = skills_index_text(skills);
    if !skills.is_empty() {
        prompt.push_str("\n\n<skills-system-prompt>\n");
        prompt.push_str(&skills);
        prompt.push_str("\n</skills-system-prompt>");
    }
    prompt
}

fn skills_cache_dir() -> PathBuf {
    crate::config::config_dir().join("skills-cache")
}

fn system_codex_home() -> PathBuf {
    std::env::var_os("CODEX_HOME")
        .map(PathBuf::from)
        .or_else(|| dirs::home_dir().map(|home| home.join(".codex")))
        .unwrap_or_else(|| PathBuf::from(".codex"))
}

/// Keep Team runtime state isolated while sharing the system Codex provider and
/// login. Links follow config/token refreshes without copying credentials.
fn inherit_codex_system_files(home: &Path) -> Result<(), String> {
    inherit_codex_system_files_from(home, &system_codex_home())
}

fn link_codex_system_file(
    home: &Path,
    system_home: &Path,
    filename: &str,
    replace_team_owned: bool,
) -> Result<(), String> {
    let source = system_home.join(filename);
    if !source.is_file() {
        if replace_team_owned {
            let target = home.join(filename);
            match std::fs::symlink_metadata(&target) {
                Ok(metadata) if metadata.file_type().is_file() || metadata.file_type().is_symlink() => {
                    std::fs::remove_file(target).map_err(|e| e.to_string())?;
                }
                Ok(_) => return Err(format!("refusing to replace Codex path: {}", target.display())),
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => return Err(error.to_string()),
            }
        }
        return Ok(());
    }
    let source = std::fs::canonicalize(source).map_err(|e| e.to_string())?;

    std::fs::create_dir_all(home).map_err(|e| e.to_string())?;
    let target = home.join(filename);
    match std::fs::symlink_metadata(&target) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                && std::fs::read_link(&target).is_ok_and(|path| path == source)
            {
                return Ok(());
            }
            if replace_team_owned && (metadata.file_type().is_file() || metadata.file_type().is_symlink()) {
                std::fs::remove_file(&target).map_err(|e| e.to_string())?;
            } else {
                return Err(format!(
                    "refusing to replace existing Codex path: {}",
                    target.display()
                ));
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }

    symlink_file(&source, &target).map_err(|e| {
        format!(
            "failed to inherit Codex system file from {}: {}",
            source.display(),
            e
        )
    })
}

fn inherit_codex_system_files_from(home: &Path, system_home: &Path) -> Result<(), String> {
    // config.toml in the private home was Team-generated before MCP settings
    // moved to CLI overrides, so it is the one path Team may replace.
    link_codex_system_file(home, system_home, "config.toml", true)?;
    link_codex_system_file(home, system_home, ".env", false)?;
    link_codex_system_file(home, system_home, "auth.json", false)
}

fn codex_developer_instructions(home: &Path, team_instructions: &str) -> Result<String, String> {
    let path = home.join("config.toml");
    let existing = match std::fs::read_to_string(&path) {
        Ok(content) => {
            let config: toml::Value = toml::from_str(&content)
                .map_err(|e| format!("failed to parse {}: {}", path.display(), e))?;
            match config.get("developer_instructions") {
                Some(value) => Some(
                    value
                        .as_str()
                        .ok_or_else(|| {
                            format!(
                                "{} developer_instructions must be a string",
                                path.display()
                            )
                        })?
                        .trim()
                        .to_string(),
                ),
                None => None,
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => None,
        Err(error) => return Err(format!("failed to read {}: {}", path.display(), error)),
    };
    let team = format!(
        "<tmux-mobile-team-instructions>\n{}\n</tmux-mobile-team-instructions>",
        team_instructions.trim()
    );
    Ok(match existing.filter(|value| !value.is_empty()) {
        Some(existing) => format!("{}\n\n{}", existing, team),
        None => team,
    })
}

#[cfg(unix)]
fn symlink_file(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(source, target)
}

#[cfg(windows)]
fn symlink_file(source: &Path, target: &Path) -> std::io::Result<()> {
    std::os::windows::fs::symlink_file(source, target)
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
    name: &str, role: &str, goal: &str, team_prompt: &str,
    cfg: &TeamConfig, room: &str, paths: &Paths, model: Option<&str>, extras: &Extras,
) -> Result<Prepared, String> {
    prepare_kiro_home(paths).map_err(|e| e.to_string())?;
    let home = &paths.kiro;
    std::fs::create_dir_all(home.join("agents")).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(home.join("settings")).map_err(|e| e.to_string())?;
    std::fs::write(
        home.join("settings").join("cli.json"),
        serde_json::to_string_pretty(&serde_json::json!({ "chat.disableTrustAllConfirmation": true })).unwrap(),
    )
    .map_err(|e| e.to_string())?;

    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    notifications.ensure_helper()?;
    let notify = notifications.helper_command("kiro");
    let tools = vec!["*".to_string(), "@team".to_string()];

    // Skills are loaded as native skill:// resources.
    let resources: Vec<String> = extras.skills.iter()
        .map(|sk| format!("skill://{}/SKILL.md", sk.dir.to_string_lossy()))
        .collect();
    // The team MCP server plus any extra per-agent servers from the team.yaml.
    let mut mcp_servers = serde_json::json!({
        "team": {
            "url": format!("{}/mcp", cfg.url),
            "timeout": team_mcp_tool_timeout_ms(),
            "headers": { "x-agent": name, "x-room": room }
        }
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
        "prompt": build_agent_prompt(role, goal, team_prompt, cfg),
        "tools": tools,
        "allowedTools": ["*"],
        "resources": resources,
        "mcpServers": mcp_servers,
        "hooks": {
            "preToolUse": [ { "matcher": "*", "command": heartbeat_command(&paths.heartbeat, "pre") } ],
            "postToolUse": [ { "matcher": "*", "command": heartbeat_command(&paths.heartbeat, "post") } ],
            "userPromptSubmit": [ { "command": heartbeat_command(&paths.heartbeat, "pulse") } ],
            "stop": [
                { "command": bash_script_command(&paths.keepalive) },
                { "command": notify },
                { "command": heartbeat_command(&paths.heartbeat, "post") }
            ]
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
        shell_quote(name),
        shell_quote(m),
        shell_quote(&cfg.team_kick)
    );
    Ok((env, cmd, None))
}

// ---- Claude Code ----
#[allow(clippy::too_many_arguments)]
fn prepare_claude(
    name: &str, role: &str, goal: &str, team_prompt: &str,
    cfg: &TeamConfig, room: &str, paths: &Paths, model: Option<&str>, extras: &Extras,
) -> Result<Prepared, String> {
    let d = &paths.claude;
    std::fs::create_dir_all(d).map_err(|e| e.to_string())?;
    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    notifications.ensure_helper()?;
    let notify = notifications.helper_command("claude");
    let mcpfile = d.join(format!("{}.mcp.json", name));
    let mut mcp_servers = serde_json::json!({
        "team": {
            "type": "http",
            "url": format!("{}/mcp", cfg.url),
            "timeout": team_mcp_tool_timeout_ms(),
            "headers": { "x-agent": name, "x-room": room }
        }
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
            "skipDangerousModePermissionPrompt": true,
            "hooks": {
                "PreToolUse": [ { "matcher": "*", "hooks": [ { "type": "command", "command": heartbeat_command(&paths.heartbeat, "pre") } ] } ],
                "PostToolUse": [ { "matcher": "*", "hooks": [ { "type": "command", "command": heartbeat_command(&paths.heartbeat, "post") } ] } ],
                "PostToolUseFailure": [ { "matcher": "*", "hooks": [ { "type": "command", "command": heartbeat_command(&paths.heartbeat, "post") } ] } ],
                "UserPromptSubmit": [ { "hooks": [ { "type": "command", "command": heartbeat_command(&paths.heartbeat, "pulse") } ] } ],
                "Notification": [ { "matcher": "permission_prompt|idle_prompt|agent_needs_input|agent_completed", "hooks": [ { "type": "command", "command": notify } ] } ],
                "Stop": [ { "hooks": [
                    { "type": "command", "command": bash_script_command(&paths.keepalive) },
                    { "type": "command", "command": notify },
                    { "type": "command", "command": heartbeat_command(&paths.heartbeat, "post") }
                ] } ],
                "StopFailure": [ { "hooks": [ { "type": "command", "command": notify } ] } ]
            }
        }))
        .unwrap(),
    )
    .map_err(|e| e.to_string())?;

    let m = model.unwrap_or("sonnet");
    let system_prompt = build_cli_system_prompt(role, goal, team_prompt, cfg, &extras.skills);
    let cmd = format!(
        "claude --mcp-config {} --strict-mcp-config --settings {} --model {} --dangerously-skip-permissions --append-system-prompt {} {}",
        shell_quote(&mcpfile.to_string_lossy()),
        shell_quote(&settingsfile.to_string_lossy()),
        shell_quote(m),
        shell_quote(&system_prompt),
        shell_quote(&cfg.team_kick)
    )
    .trim_end()
    .to_string();
    // Tool permissions are bypassed by the CLI flag. Workspace trust is a
    // separate first-use dialog with no public skip flag, so confirm it only
    // after all stable prompt markers are visible.
    let confirmation = StartupConfirmation {
        markers: CLAUDE_FOLDER_TRUST_MARKERS.to_vec(),
        ready_markers: vec!["bypass permissions on"],
        timeout: Duration::from_secs(120),
    };
    let mut env = hb_env(name, room, cfg);
    env.extend(extras.env.iter().cloned());
    env.push(("MCP_TOOL_TIMEOUT".to_string(), team_mcp_tool_timeout_ms().to_string()));
    env.push(("CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT".to_string(), "0".to_string()));
    Ok((env, cmd, Some(confirmation)))
}

// ---- Codex ----
#[allow(clippy::too_many_arguments)]
fn prepare_codex(
    name: &str, role: &str, goal: &str, team_prompt: &str, cfg: &TeamConfig, room: &str, paths: &Paths, model: Option<&str>, extras: &Extras,
) -> Result<Prepared, String> {
    let home = paths.codex.join(name);
    std::fs::create_dir_all(&home).map_err(|e| e.to_string())?;
    inherit_codex_system_files(&home)?;
    let notifications = crate::agent_notifications::AgentNotificationHub::load();
    notifications.ensure_helper()?;
    let notify = notifications.helper_command("codex");
    let team_mcp = McpDef {
        name: "team".to_string(),
        url: Some(format!("{}/mcp", cfg.url)),
        headers: [
            ("x-agent".to_string(), name.to_string()),
            ("x-room".to_string(), room.to_string()),
        ]
        .into_iter()
        .collect(),
        ..McpDef::default()
    };
    let mut config_args = codex_team_mcp_overrides(&team_mcp);
    for m in &extras.mcp {
        if !m.name.is_empty() {
            config_args.extend(codex_mcp_overrides(m));
        }
    }
    let system_prompt = build_cli_system_prompt(role, goal, team_prompt, cfg, &extras.skills);
    config_args.push(codex_config_override(
        "developer_instructions",
        Value::String(codex_developer_instructions(&home, &system_prompt)?),
    ));
    std::fs::write(
        home.join("hooks.json"),
        serde_json::to_vec_pretty(&codex_hooks_value(&paths.keepalive, &paths.heartbeat, &notify)).unwrap(),
    ).map_err(|e| e.to_string())?;
    let mut env = vec![("CODEX_HOME".to_string(), home.to_string_lossy().to_string())];
    env.extend(hb_env(name, room, cfg));
    env.extend(extras.env.iter().cloned());
    if let Some(value) = model {
        config_args.push(format!("--model {}", shell_quote(value)));
    }
    config_args.push("--dangerously-bypass-approvals-and-sandbox".to_string());
    config_args.push("--dangerously-bypass-hook-trust".to_string());
    config_args.push(shell_quote(&cfg.team_kick));
    let cmd = format!("codex {}", config_args.join(" "));
    let confirmation = StartupConfirmation {
        markers: CODEX_FOLDER_TRUST_MARKERS.to_vec(),
        ready_markers: vec!["Starting MCP servers"],
        timeout: Duration::from_secs(120),
    };
    Ok((env, cmd, Some(confirmation)))
}

fn codex_hooks_value(keepalive: &Path, heartbeat: &Path, notify: &str) -> Value {
    serde_json::json!({
        "hooks": {
            "PreToolUse": [ { "matcher": "*", "hooks": [
                { "type": "command", "command": heartbeat_command(heartbeat, "pre") }
            ] } ],
            "PermissionRequest": [ { "hooks": [
                { "type": "command", "command": notify }
            ] } ],
            "PostToolUse": [ { "matcher": "*", "hooks": [
                { "type": "command", "command": heartbeat_command(heartbeat, "post") }
            ] } ],
            "UserPromptSubmit": [ { "hooks": [
                { "type": "command", "command": heartbeat_command(heartbeat, "pulse") }
            ] } ],
            "Stop": [ { "hooks": [
                { "type": "command", "command": bash_script_command(keepalive) },
                { "type": "command", "command": notify },
                { "type": "command", "command": heartbeat_command(heartbeat, "post") }
            ] } ]
        }
    })
}

fn bash_script_command(path: &Path) -> String {
    format!("/bin/bash {}", shell_quote(&path.to_string_lossy()))
}

fn heartbeat_command(path: &Path, mode: &str) -> String {
    format!("{} {}", bash_script_command(path), mode)
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

    // ── SleepState ──────────────────────────────────────────────────────
    // The state machine that mediates "all idle long enough → Esc the team"
    // and "new message arrived → wake them". Pure: no tmux, no bus.

    fn idle_roster() -> HashMap<String, (String, i64)> {
        let mut r = HashMap::new();
        r.insert("alice".into(), ("idle".into(), 0));
        r.insert("bob".into(), ("idle".into(), 0));
        r
    }

    #[test]
    fn sleep_state_does_nothing_on_an_empty_room() {
        // No agents at all (a freshly-started team that hasn't booted yet)
        // must not be considered idle — there's no one to sleep.
        let mut s = SleepState::default();
        let empty: HashMap<String, (String, i64)> = HashMap::new();
        assert!(!s.is_online_idle(&empty));
        assert_eq!(s.step(0, false, 0, 60_000), SleepAction::None);
        assert!(!s.slept);
    }

    #[test]
    fn sleep_state_skips_if_any_agent_is_busy() {
        // The whole point: ONE working agent cancels sleep for everyone.
        let mut s = SleepState::default();
        let mut r = idle_roster();
        r.insert("worker".into(), ("working".into(), 0));
        assert!(!s.is_online_idle(&r));
        assert_eq!(s.step(0, false, 0, 60_000), SleepAction::None);
        assert!(s.idle_since.is_none());
    }

    #[test]
    fn sleep_state_offline_agents_dont_block_sleep() {
        // Offline agents are deliberately gone — they don't count toward the
        // "is anyone busy?" check. The remaining online agents alone decide.
        let s = SleepState::default();
        let mut r = idle_roster();
        r.insert("ghost".into(), ("offline".into(), 0));
        assert!(s.is_online_idle(&r));
    }

    #[test]
    fn sleep_state_arms_then_fires_at_threshold() {
        // First idle tick arms the timer; later ticks below threshold = no
        // action; once we cross threshold = Sleep + state flips.
        let mut s = SleepState::default();
        assert_eq!(s.step(1_000, true, 0, 60_000), SleepAction::None);
        assert_eq!(s.idle_since, Some(1_000));
        assert!(!s.slept);

        assert_eq!(s.step(30_000, true, 0, 60_000), SleepAction::None);
        assert!(!s.slept);

        assert_eq!(s.step(61_000, true, 5, 60_000), SleepAction::Sleep);
        assert!(s.slept);
        assert_eq!(s.sleep_anchor_seq, 5);
    }

    #[test]
    fn sleep_state_threshold_zero_disables_sleep() {
        // 0 means "feature off". We still arm idle_since (cheap), but never
        // trip Sleep no matter how long the team has been idle.
        let mut s = SleepState::default();
        assert_eq!(s.step(0, true, 0, 0), SleepAction::None);
        assert_eq!(s.step(86_400_000, true, 0, 0), SleepAction::None); // 1 day
        assert!(!s.slept);
    }

    #[test]
    fn idle_sleep_precedes_the_first_empty_wait_result() {
        assert_eq!(IDLE_SLEEP_MS, 8 * 60 * 1000);
        assert_eq!(
            agora::mcp::MCP_WAIT_MAX_MS as i64 - IDLE_SLEEP_MS,
            60_000
        );
    }

    #[test]
    fn sleep_state_resets_arming_when_an_agent_starts_working() {
        // If the team started looking idle, then someone actually picks work
        // back up before the threshold, we drop the timer entirely.
        let mut s = SleepState::default();
        s.step(1_000, true, 0, 60_000); // arm
        assert_eq!(s.idle_since, Some(1_000));

        s.step(2_000, false, 0, 60_000); // someone working
        assert!(s.idle_since.is_none());

        // And re-arming starts fresh, not from the original idle_since.
        s.step(3_000, true, 0, 60_000);
        assert_eq!(s.idle_since, Some(3_000));
    }

    #[test]
    fn sleep_state_wakes_only_on_a_new_message_seq() {
        // While slept, the rule is intentionally narrow: a strictly newer bus
        // seq than the anchor wakes. Stale roster (status drifting from idle
        // to stalled because the wait MCP call is dead) MUST NOT wake — that
        // would oscillate forever.
        let mut s = SleepState::default();
        // Force slept state at anchor seq=10.
        s.step(0, true, 10, 60_000);
        s.step(60_001, true, 10, 60_000); // → Sleep
        assert!(s.slept);
        assert_eq!(s.sleep_anchor_seq, 10);

        // Same seq, status drifts to stalled (online_idle now false). No wake.
        assert_eq!(s.step(60_002, false, 10, 60_000), SleepAction::None);
        assert!(s.slept);

        // New message: seq jumps to 11. Wake.
        assert_eq!(s.step(60_003, false, 11, 60_000), SleepAction::Wake);
        assert!(!s.slept);
        assert_eq!(s.sleep_anchor_seq, 11);
        assert!(s.idle_since.is_none());
    }

    #[test]
    fn sleep_state_can_re_sleep_after_a_wake_round_trip() {
        // Sleep → wake → idle again → sleep again. Exercises the full cycle.
        let mut s = SleepState::default();
        s.step(0, true, 0, 60_000);
        assert_eq!(s.step(60_000, true, 0, 60_000), SleepAction::Sleep);

        // Human posts (seq 1) → wake.
        assert_eq!(s.step(60_100, false, 1, 60_000), SleepAction::Wake);
        assert!(!s.slept);

        // Team handles it, returns to idle, threshold passes again → sleep.
        s.step(60_200, true, 1, 60_000); // re-arm
        assert_eq!(s.step(120_300, true, 2, 60_000), SleepAction::Sleep);
        assert_eq!(s.sleep_anchor_seq, 2);
    }

    #[test]
    fn recovery_nudges_dead_wait_without_interrupting_long_work() {
        let now = 2_000_000;

        assert!(should_recover_agent(
            "stalled",
            now - WAIT_RECOVERY_STALE_MS,
            now,
            None
        ));
        assert!(!should_recover_agent(
            "hardworking",
            now - WAIT_RECOVERY_STALE_MS,
            now,
            None
        ));
        assert!(should_recover_agent(
            "hardworking",
            now - RECOVERY_STALE_MS,
            now,
            None
        ));
    }

    #[test]
    fn recovery_respects_sleep_offline_and_nudge_cooldown() {
        let now = 2_000_000;
        let stale = now - RECOVERY_STALE_MS;

        assert!(!should_recover_agent("sleeping", stale, now, None));
        assert!(!should_recover_agent("offline", stale, now, None));
        assert!(!should_recover_agent(
            "stalled",
            stale,
            now,
            Some(now - RECOVERY_COOLDOWN_MS + 1)
        ));
        assert!(should_recover_agent(
            "stalled",
            stale,
            now,
            Some(now - RECOVERY_COOLDOWN_MS)
        ));
    }

    #[test]
    fn restart_recovery_nudges_only_an_unchanged_idle_wait() {
        assert_eq!(RECONNECT_NUDGE, "Continue.");
        assert!(should_reconnect_adopted_agent(
            Some(("idle", 1_000)),
            Some(("idle", 1_000)),
        ));
        assert!(should_reconnect_adopted_agent(
            Some(("online", 1_000)),
            Some(("stalled", 1_000)),
        ));

        assert!(!should_reconnect_adopted_agent(
            Some(("idle", 1_000)),
            Some(("idle", 1_001)),
        ));
        assert!(!should_reconnect_adopted_agent(
            Some(("idle", 1_000)),
            Some(("working", 1_000)),
        ));
    }

    #[test]
    fn restart_recovery_never_interrupts_work_sleep_or_unknown_agents() {
        for status in ["thinking", "working", "hardworking", "sleeping", "offline"] {
            assert!(!should_reconnect_adopted_agent(
                Some((status, 1_000)),
                Some((status, 1_000)),
            ), "{status} must not be interrupted");
        }
        assert!(!should_reconnect_adopted_agent(None, Some(("idle", 1_000))));
        assert!(!should_reconnect_adopted_agent(Some(("idle", 1_000)), None));
    }

    // ── existing tests follow ──

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
    fn codex_system_files_link_config_env_and_auth_idempotently() {
        let root = std::env::temp_dir().join(format!("teamtest-codex-system-{}", uuid::Uuid::new_v4()));
        let system_home = root.join("system");
        let agent_home = root.join("agent");
        std::fs::create_dir_all(&system_home).unwrap();
        std::fs::write(system_home.join("config.toml"), "model_provider = \"custom\"").unwrap();
        std::fs::write(system_home.join(".env"), "PROVIDER_TOKEN=secret").unwrap();
        std::fs::write(system_home.join("auth.json"), "{}").unwrap();

        inherit_codex_system_files_from(&agent_home, &system_home).unwrap();
        inherit_codex_system_files_from(&agent_home, &system_home).unwrap();

        for filename in ["config.toml", ".env", "auth.json"] {
            let target = agent_home.join(filename);
            assert!(std::fs::symlink_metadata(&target)
                .unwrap()
                .file_type()
                .is_symlink());
        }
        assert_eq!(
            std::fs::read_to_string(agent_home.join("config.toml")).unwrap(),
            "model_provider = \"custom\""
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_system_files_missing_source_removes_only_team_config() {
        let root = std::env::temp_dir().join(format!("teamtest-codex-no-auth-{}", uuid::Uuid::new_v4()));
        let agent_home = root.join("agent");
        std::fs::create_dir_all(&agent_home).unwrap();
        std::fs::write(agent_home.join("config.toml"), "[mcp_servers.team]").unwrap();

        inherit_codex_system_files_from(&agent_home, &root.join("system")).unwrap();

        assert!(!agent_home.join("config.toml").exists());
        assert!(!agent_home.join(".env").exists());
        assert!(!agent_home.join("auth.json").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_team_instructions_follow_existing_user_instructions() {
        let root = std::env::temp_dir().join(format!(
            "teamtest-codex-instructions-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("config.toml"),
            "developer_instructions = \"Keep the user's convention.\"\n",
        )
        .unwrap();

        let merged = codex_developer_instructions(&root, "Team contract.").unwrap();

        assert!(merged.starts_with("Keep the user's convention."));
        assert!(merged.contains(
            "<tmux-mobile-team-instructions>\nTeam contract.\n</tmux-mobile-team-instructions>"
        ));
        assert!(
            merged.find("Keep the user's convention.").unwrap()
                < merged.find("Team contract.").unwrap()
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_auth_does_not_replace_an_existing_private_file() {
        let root = std::env::temp_dir().join(format!("teamtest-codex-existing-auth-{}", uuid::Uuid::new_v4()));
        let system_home = root.join("system");
        let agent_home = root.join("agent");
        std::fs::create_dir_all(&system_home).unwrap();
        std::fs::create_dir_all(&agent_home).unwrap();
        std::fs::write(system_home.join("auth.json"), "system").unwrap();
        std::fs::write(agent_home.join("auth.json"), "private").unwrap();

        let error = inherit_codex_system_files_from(&agent_home, &system_home).unwrap_err();

        assert!(error.contains("refusing to replace"));
        assert_eq!(std::fs::read_to_string(agent_home.join("auth.json")).unwrap(), "private");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn codex_stop_hooks_keep_wait_loop_alive_and_notify() {
        let hooks = codex_hooks_value(
            Path::new("/team/keepalive.sh"),
            Path::new("/team/heartbeat.sh"),
            "notify codex",
        );
        let stop = hooks["hooks"]["Stop"][0]["hooks"].as_array().unwrap();

        assert_eq!(stop.len(), 3);
        assert_eq!(stop[0]["command"], "/bin/bash /team/keepalive.sh");
        assert_eq!(stop[1]["command"], "notify codex");
        assert_eq!(stop[2]["command"], "/bin/bash /team/heartbeat.sh post");
        assert_eq!(
            hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            "/bin/bash /team/heartbeat.sh pre"
        );
        assert_eq!(
            hooks["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            "/bin/bash /team/heartbeat.sh post"
        );
    }

    #[test]
    fn backend_launch_permissions_and_startup_confirmations() {
        let root = std::env::temp_dir().join(format!(
            "teamtest-backend-permissions-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let paths = Paths::new(root.to_str().unwrap(), "backend-permissions");
        prepare_home(&paths).unwrap();
        let extras = Extras::default();
        let cfg = cfg();

        let (kiro_env, kiro_cmd, kiro_confirmation) = prepare_kiro(
            "lead",
            "lead",
            "coordinate",
            "",
            &cfg,
            "room",
            &paths,
            None,
            &extras,
        )
        .unwrap();
        assert!(kiro_cmd.contains("--trust-all-tools"));
        assert!(kiro_cmd.contains("--model claude-sonnet-4.6"));
        assert!(kiro_confirmation.is_none());
        let kiro_settings: Value = serde_json::from_slice(
            &std::fs::read(paths.kiro.join("settings/cli.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            kiro_settings["chat.disableTrustAllConfirmation"],
            Value::Bool(true)
        );
        let kiro_agent: Value = serde_json::from_slice(
            &std::fs::read(paths.kiro.join("agents/lead.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            kiro_agent["hooks"]["stop"][0]["command"],
            format!("/bin/bash {}", paths.keepalive.display())
        );
        assert_eq!(
            kiro_agent["hooks"]["stop"][2]["command"],
            format!("/bin/bash {} post", paths.heartbeat.display())
        );
        assert_eq!(
            kiro_agent["hooks"]["preToolUse"][0]["command"],
            format!("/bin/bash {} pre", paths.heartbeat.display())
        );
        assert_eq!(
            kiro_agent["hooks"]["postToolUse"][0]["command"],
            format!("/bin/bash {} post", paths.heartbeat.display())
        );
        assert_eq!(
            kiro_agent["hooks"]["userPromptSubmit"][0]["command"],
            format!("/bin/bash {} pulse", paths.heartbeat.display())
        );
        assert_eq!(
            kiro_agent["mcpServers"]["team"]["timeout"],
            TEAM_MCP_TOOL_TIMEOUT_MS
        );
        assert!(kiro_agent["prompt"]
            .as_str()
            .unwrap()
            .contains("<role-system-prompt>"));
        assert!(!kiro_env.iter().any(|(key, _)| key == "MCP_TOOL_TIMEOUT"));

        let (claude_env, claude_cmd, claude_confirmation) = prepare_claude(
            "planner",
            "planner",
            "plan",
            "",
            &cfg,
            "room",
            &paths,
            None,
            &extras,
        )
        .unwrap();
        assert!(claude_cmd.contains("--dangerously-skip-permissions"));
        assert!(claude_cmd.contains("--model sonnet"));
        assert!(claude_cmd.contains("--append-system-prompt"));
        assert!(claude_cmd.contains("<role-system-prompt>"));
        assert!(
            claude_cmd.ends_with(" kick"),
            "the positional startup message contains only the kick: {claude_cmd}"
        );
        let claude_settings: Value = serde_json::from_slice(
            &std::fs::read(paths.claude.join("planner.settings.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            claude_settings["skipDangerousModePermissionPrompt"],
            Value::Bool(true)
        );
        assert_eq!(
            claude_settings["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
            format!("/bin/bash {} pulse", paths.heartbeat.display())
        );
        assert_eq!(
            claude_settings["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            format!("/bin/bash {} pre", paths.heartbeat.display())
        );
        assert_eq!(
            claude_settings["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            format!("/bin/bash {} post", paths.heartbeat.display())
        );
        assert_eq!(
            claude_settings["hooks"]["PostToolUseFailure"][0]["hooks"][0]["command"],
            format!("/bin/bash {} post", paths.heartbeat.display())
        );
        assert_eq!(
            claude_settings["hooks"]["Stop"][0]["hooks"][2]["command"],
            format!("/bin/bash {} post", paths.heartbeat.display())
        );
        let claude_mcp: Value = serde_json::from_slice(
            &std::fs::read(paths.claude.join("planner.mcp.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            claude_mcp["mcpServers"]["team"]["timeout"],
            TEAM_MCP_TOOL_TIMEOUT_MS
        );
        assert!(claude_env.contains(&(
            "MCP_TOOL_TIMEOUT".to_string(),
            TEAM_MCP_TOOL_TIMEOUT_MS.to_string()
        )));
        assert!(claude_env.contains(&(
            "CLAUDE_CODE_MCP_TOOL_IDLE_TIMEOUT".to_string(),
            "0".to_string()
        )));
        let confirmation = claude_confirmation.unwrap();
        assert_eq!(confirmation.timeout, Duration::from_secs(120));
        assert!(startup_prompt_visible(
            "Accessing workspace:\nYes, I trust this folder\nEnter to confirm",
            &confirmation
        ));
        assert!(!startup_prompt_visible(
            "Claude is ready for input",
            &confirmation
        ));
        assert!(startup_already_ready(
            "bypass permissions on (shift+tab to cycle)",
            &confirmation
        ));
        assert!(folder_trust_prompt_visible(
            "Accessing workspace:\nYes, I trust this folder\nEnter to confirm"
        ));

        let (_, codex_cmd, codex_confirmation) = prepare_codex(
            "builder",
            "builder",
            "build",
            "",
            &cfg,
            "room",
            &paths,
            None,
            &extras,
        )
        .unwrap();
        assert!(codex_cmd.contains("--dangerously-bypass-approvals-and-sandbox"));
        assert!(codex_cmd.contains("--dangerously-bypass-hook-trust"));
        assert!(codex_cmd.contains("developer_instructions="));
        assert!(codex_cmd.contains("<tmux-mobile-team-instructions>"));
        assert!(codex_cmd.contains("<role-system-prompt>"));
        assert!(
            codex_cmd.ends_with(" kick"),
            "the positional startup message contains only the kick: {codex_cmd}"
        );
        assert!(!codex_cmd.contains("--model"));
        let confirmation = codex_confirmation.unwrap();
        assert!(startup_prompt_visible(
            "Do you trust the contents of this directory?\n1. Yes, continue\nPress enter to continue",
            &confirmation
        ));
        assert!(!startup_prompt_visible(
            "Do you trust the contents of this directory?\n2. No, quit",
            &confirmation
        ));
        assert!(folder_trust_prompt_visible(
            "Do you trust the contents of this directory?\n1. Yes, continue\nPress enter to continue"
        ));
        assert!(!folder_trust_prompt_visible(
            "A tool needs permission.\n1. Yes, continue\nPress enter to continue"
        ));
        let codex_hooks: Value = serde_json::from_slice(
            &std::fs::read(paths.codex.join("builder/hooks.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(
            codex_hooks["hooks"]["PreToolUse"][0]["hooks"][0]["command"],
            format!("/bin/bash {} pre", paths.heartbeat.display())
        );
        assert_eq!(
            codex_hooks["hooks"]["PostToolUse"][0]["hooks"][0]["command"],
            format!("/bin/bash {} post", paths.heartbeat.display())
        );
        assert_eq!(
            codex_hooks["hooks"]["UserPromptSubmit"][0]["hooks"][0]["command"],
            format!("/bin/bash {} pulse", paths.heartbeat.display())
        );

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn backend_model_selection_is_forwarded() {
        let root = std::env::temp_dir().join(format!(
            "teamtest-backend-models-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let paths = Paths::new(root.to_str().unwrap(), "backend-models");
        prepare_home(&paths).unwrap();
        let extras = Extras::default();
        let cfg = cfg();
        let model = "provider/model name;not-a-command";
        let expected = format!("--model {}", shell_quote(model));

        let (_, kiro_cmd, _) = prepare_kiro(
            "lead", "lead", "coordinate", "", &cfg, "room", &paths, Some(model), &extras,
        )
        .unwrap();
        let (_, claude_cmd, _) = prepare_claude(
            "planner", "planner", "plan", "", &cfg, "room", &paths, Some(model), &extras,
        )
        .unwrap();
        let (_, codex_cmd, _) = prepare_codex(
            "builder", "builder", "build", "", &cfg, "room", &paths, Some(model), &extras,
        )
        .unwrap();

        assert!(kiro_cmd.contains(&expected), "{kiro_cmd}");
        assert!(claude_cmd.contains(&expected), "{claude_cmd}");
        assert!(codex_cmd.contains(&expected), "{codex_cmd}");

        let _ = std::fs::remove_dir_all(root);
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
            headers: [
                ("Authorization".to_string(), "Bearer $API_TOKEN".to_string()),
                ("X-Features".to_string(), "${FEATURES}".to_string()),
                ("X-Static".to_string(), "literal".to_string()),
            ]
            .into_iter()
            .collect(),
            ..Default::default()
        };
        // kiro remote omits an explicit type; claude tags it http.
        assert!(kiro_mcp_value(&remote).get("type").is_none());
        assert_eq!(claude_mcp_value(&remote)["type"], "http");
        assert_eq!(kiro_mcp_value(&remote)["url"], "https://x/mcp");
        assert_eq!(
            kiro_mcp_value(&remote)["headers"]["Authorization"],
            "Bearer ${API_TOKEN}"
        );
        assert_eq!(
            claude_mcp_value(&remote)["headers"]["X-Features"],
            "${FEATURES}"
        );
        assert_eq!(kiro_mcp_value(&remote)["headers"]["X-Static"], "literal");

        let remote_overrides = codex_mcp_overrides(&remote).join(" ");
        assert!(remote_overrides.contains("mcp_servers.gh.bearer_token_env_var"));
        assert!(remote_overrides.contains("API_TOKEN"));
        assert!(remote_overrides.contains("mcp_servers.gh.env_http_headers.X-Features"));
        assert!(remote_overrides.contains("FEATURES"));
        assert!(remote_overrides.contains("mcp_servers.gh.http_headers.X-Static"));
        assert!(!remote_overrides.contains("Bearer $API_TOKEN"));

        let local = McpDef { name: "pg".into(), command: Some("mcp-pg".into()), args: vec!["--stdio".into()], ..Default::default() };
        let overrides = codex_mcp_overrides(&local).join(" ");
        assert!(overrides.contains("mcp_servers.pg.command"));
        assert!(overrides.contains("mcp-pg"));
        assert!(overrides.contains("mcp_servers.pg.args"));
        assert!(overrides.contains("--stdio"));
    }

    #[test]
    fn team_tool_timeout_exceeds_coalesced_wait_budget() {
        let team = McpDef {
            name: "team".into(),
            url: Some("http://127.0.0.1:8787/mcp".into()),
            ..Default::default()
        };
        let overrides = codex_team_mcp_overrides(&team).join(" ");
        let timeout_ms = team_mcp_tool_timeout_ms();
        let timeout_secs = timeout_ms / 1000;

        assert!(timeout_ms > agora::mcp::MCP_WAIT_MAX_MS);
        assert_eq!(timeout_ms - agora::mcp::MCP_WAIT_MAX_MS, 60_000);
        assert!(overrides.contains(&format!(
            "mcp_servers.team.tool_timeout_sec={timeout_secs}"
        )));
        assert_eq!(timeout_ms, TEAM_MCP_TOOL_TIMEOUT_MS);
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
        fn set_agent_status(&self, _room: &str, _agent: &str, _status: &str) -> Result<(), String> { Ok(()) }
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
        TeamConfig { url: "http://127.0.0.1:8787".into(), model: "claude-sonnet-4.6".into(), system_prompt: String::new(), team_rules: String::new(), team_kick: "kick".into() }
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
        // Workflow lives in each role's goal (team-brief.md is contract-only), so
        // every agent must carry a substantive goal.
        assert!(
            agents.iter().all(|a| a["goal"].as_str().map(|g| g.len() > 80).unwrap_or(false)),
            "each role's goal must carry its slice of the workflow"
        );
        assert!(agents.iter().all(|a| a["model"] == ""), "models use the server default");

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
    fn builtin_templates_are_seeded() {
        // Isolate config dir so we don't touch the real ~/.config.
        let dir = std::env::temp_dir().join(format!("teamtest-tpl-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::env::set_var("XDG_CONFIG_HOME", &dir);

        ensure_templates_seeded();
        let mut names = list_templates();
        names.sort();
        for expected in [
            "default",
            "software-dev",
            "financial-research",
            "deep-research",
            "content-studio",
            "data-analysis",
            "mixed-engineering",
        ] {
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
    fn mixed_engineering_template_uses_all_backends_with_explicit_handoffs() {
        let v: Value = serde_yml::from_str(MIXED_ENGINEERING_TEMPLATE).unwrap();
        let agents = v["agents"].as_array().unwrap();
        assert_eq!(agents.len(), 3, "keep the mixed team lean");

        for backend in ["kiro", "claude", "codex"] {
            assert_eq!(
                agents.iter().filter(|agent| agent["backend"] == backend).count(),
                1,
                "expected exactly one {backend} agent"
            );
        }
        let agent = |name: &str| agents.iter().find(|agent| agent["name"] == name).unwrap();
        assert_eq!(agent("lead")["backend"], "kiro");
        assert_eq!(agent("architect")["backend"], "claude");
        assert_eq!(agent("builder")["backend"], "codex");
        assert!(agents.iter().all(|agent| agent["model"] == ""));
        assert!(agents.iter().all(|agent| agent["manage"] == false));

        let prompt = v["prompt"].as_str().unwrap();
        for handoff in ["@lead", "@architect", "@builder", "verification", "review"] {
            assert!(prompt.contains(handoff), "missing workflow handoff: {handoff}");
        }
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
    fn workspace_slug_is_tmux_safe_with_hash() {
        let s = workspace_slug("/Users/clawd/work/My Project");
        assert!(s.starts_with("my-project-"), "got {s}");
        assert_eq!(s.len(), "my-project-".len() + 6, "basename + 6-char hash: {s}");

        // Different paths, same basename → different slugs.
        let a = workspace_slug("/a/demo");
        let b = workspace_slug("/b/demo");
        assert_ne!(a, b, "same basename must get different hashes: {a} vs {b}");

        // No ':' or '.' (illegal in tmux session names).
        let s = workspace_slug("/a/b.c:d");
        assert!(!s.contains(':') && !s.contains('.'), "got {s}");

        // Empty / root paths.
        let r = workspace_slug("/");
        assert!(r.starts_with("root-"), "got {r}");
    }

    #[test]
    fn team_slug_is_stable_and_separates_templates_in_one_workspace() {
        let workspace = "/Users/clawd/work/My Project";
        let default = team_slug(workspace, "default");
        let triad = team_slug(workspace, "triad");

        assert_eq!(default, team_slug(workspace, "default"));
        assert_ne!(default, triad);
        assert!(default.contains("my-project-default-"), "{default}");
        assert!(triad.contains("my-project-triad-"), "{triad}");
        assert!(default
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')));
    }

    #[test]
    fn team_runtime_dirs_are_isolated_with_legacy_compatibility() {
        let workspace = "/tmp/shared-project";
        let legacy_room = workspace_slug(workspace);
        let first = team_slug(workspace, "default");
        let second = team_slug(workspace, "triad");

        assert_eq!(
            team_runtime_dir(workspace, &legacy_room),
            PathBuf::from(workspace).join(".tmm")
        );
        assert_ne!(
            team_runtime_dir(workspace, &first),
            team_runtime_dir(workspace, &second)
        );
        assert_eq!(
            team_runtime_dir(workspace, &first),
            PathBuf::from(workspace).join(".tmm/teams").join(first)
        );
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

    #[test]
    fn cli_skill_index_is_part_of_the_system_prompt() {
        let cfg = TeamConfig {
            url: String::new(),
            model: String::new(),
            system_prompt: String::new(),
            team_rules: "Shared rule.".into(),
            team_kick: "kick".into(),
        };
        let skills = vec![ResolvedSkill {
            name: "review".into(),
            dir: PathBuf::from("/skills/review"),
            description: "Review changes".into(),
        }];

        let prompt = build_cli_system_prompt("reviewer", "Review.", "", &cfg, &skills);

        assert!(prompt.contains("<skills-system-prompt>"));
        assert!(prompt.contains("[review] Review changes (at /skills/review/SKILL.md)"));
        assert!(!prompt.contains("kick"));
    }

    #[test]
    fn prepare_home_creates_gitignore() {
        let dir = std::env::temp_dir().join(format!("teamtest-home-{}", std::process::id()));
        let ws = dir.join("proj");
        std::fs::create_dir_all(&ws).unwrap();
        let paths = Paths::new(ws.to_str().unwrap(), "team-a");
        prepare_home(&paths).unwrap();
        let gi = ws.join(".tmm").join(".gitignore");
        assert!(gi.exists());
        assert_eq!(std::fs::read_to_string(&gi).unwrap(), "*\n");
        assert!(paths.keepalive.exists());
        assert!(paths.heartbeat.exists());
        assert!(!paths.kiro.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_kiro_home_migrates_legacy_state() {
        let dir = std::env::temp_dir().join(format!(
            "teamtest-kiro-home-migration-{}",
            uuid::Uuid::new_v4()
        ));
        let ws = dir.join("proj");
        let legacy = ws.join(".tmm").join("kiro-home");
        std::fs::create_dir_all(legacy.join("state")).unwrap();
        std::fs::write(legacy.join("state/session.json"), "preserved").unwrap();
        let room = workspace_slug(ws.to_str().unwrap());
        let paths = Paths::new(ws.to_str().unwrap(), &room);

        prepare_kiro_home(&paths).unwrap();

        assert!(!legacy.exists());
        assert_eq!(
            std::fs::read_to_string(paths.kiro.join("state/session.json")).unwrap(),
            "preserved"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prepare_kiro_home_never_overwrites_canonical_state() {
        let dir = std::env::temp_dir().join(format!(
            "teamtest-kiro-home-existing-{}",
            uuid::Uuid::new_v4()
        ));
        let ws = dir.join("proj");
        let legacy = ws.join(".tmm").join("kiro-home");
        let room = workspace_slug(ws.to_str().unwrap());
        let paths = Paths::new(ws.to_str().unwrap(), &room);
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::create_dir_all(&paths.kiro).unwrap();
        std::fs::write(legacy.join("state"), "legacy").unwrap();
        std::fs::write(paths.kiro.join("state"), "canonical").unwrap();

        prepare_kiro_home(&paths).unwrap();

        assert_eq!(
            std::fs::read_to_string(paths.kiro.join("state")).unwrap(),
            "canonical"
        );
        assert_eq!(
            std::fs::read_to_string(legacy.join("state")).unwrap(),
            "legacy"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn new_team_home_does_not_adopt_legacy_kiro_state() {
        let dir = std::env::temp_dir().join(format!(
            "teamtest-kiro-instance-isolation-{}",
            uuid::Uuid::new_v4()
        ));
        let ws = dir.join("proj");
        let legacy = ws.join(".tmm/kiro-home");
        std::fs::create_dir_all(&legacy).unwrap();
        std::fs::write(legacy.join("state"), "another team").unwrap();
        let room = team_slug(ws.to_str().unwrap(), "triad");
        let paths = Paths::new(ws.to_str().unwrap(), &room);

        prepare_kiro_home(&paths).unwrap();

        assert_eq!(
            std::fs::read_to_string(legacy.join("state")).unwrap(),
            "another team"
        );
        assert!(paths.kiro.is_dir());
        assert!(!paths.kiro.join("state").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
