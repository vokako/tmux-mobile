use serde::Deserialize;
use serde_json;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
struct FileConfig {
    host: Option<String>,
    port: Option<u16>,
    token: Option<String>,
    tmux_socket: Option<String>,
    tls_cert: Option<String>,
    tls_key: Option<String>,
    scrollback: Option<usize>,
    // Seconds to wait after the last client on a resized tmux window
    // disconnects before restoring that window to auto-size. 0 = restore
    // immediately (legacy behavior). Default 600 (10 min) so short
    // reconnects (backgrounded app, network blip) skip the reflow cycle.
    disconnect_grace_secs: Option<u64>,
    // Team multi-agent bus (desktop only). `team_bind` is where the in-process
    // MCP daemon + dashboard listen for external coding agents; the phone
    // reaches the same room through the tmux-mobile WS server, so this address
    // only matters for the agents themselves. The bus always runs in-process.
    // `team_*` / `agora_*` aliases are accepted for configs written by the
    // pre-rebrand builds (the bus library is still upstream "agora").
    #[serde(alias = "crew_bind", alias = "agora_bind")]
    team_bind: Option<String>,
    #[serde(alias = "crew_db", alias = "agora_db")]
    team_db: Option<String>,
    #[serde(alias = "crew_room", alias = "agora_room")]
    team_room: Option<String>,
    // Default model for kiro-backed team agents.
    #[serde(alias = "crew_model", alias = "agora_model")]
    team_model: Option<String>,
    // Codex config profile for codex-backed team agents (codex --profile).
    team_codex_profile: Option<String>,
    // Shared rules prepended to every team agent's brief (the "how we
    // collaborate" contract). Overrides the built-in default if set.
    team_rules: Option<String>,
    // The kick message sent to each agent at startup (connects them to the bus).
    team_kick: Option<String>,
}

pub struct Config {
    pub host: String,
    pub port: u16,
    pub token: String,
    pub machine_id: String,
    pub tmux_socket: Option<String>,
    pub tls_cert: Option<String>,
    pub tls_key: Option<String>,
    pub scrollback: usize,
    pub disconnect_grace_secs: u64,
    pub team_bind: String,
    pub team_db: String,
    pub team_room: String,
    pub team_model: String,
    pub team_codex_profile: String,
    pub team_rules: String,
    pub team_kick: String,
}

fn config_path() -> PathBuf {
    dirs_next().join("config.toml")
}

/// The tmux-mobile config directory (`~/.config/tmux-mobile`). Public so the
/// team supervisor can place its per-agent working files alongside the rest of
/// the app's state.
pub fn config_dir() -> PathBuf {
    dirs_next()
}

fn dirs_next() -> PathBuf {
    // Follow the XDG Base Directory convention: $XDG_CONFIG_HOME if set,
    // else ~/.config. App state lives under the `tmux-mobile/` subdir.
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
        .unwrap_or_else(|| PathBuf::from(".config"));
    base.join("tmux-mobile")
}

impl Config {
    /// Load config: file < env vars. Auto-generates token if missing everywhere.
    pub fn load() -> Self {
        let file_cfg = std::fs::read_to_string(config_path())
            .ok()
            .and_then(|s| toml::from_str::<FileConfig>(&s).ok())
            .unwrap_or_default();

        let token = std::env::var("TOKEN")
            .ok()
            .or(file_cfg.token)
            .unwrap_or_else(|| {
                let t = uuid::Uuid::new_v4().to_string();
                let _ = save_token(&t);
                t
            });

        Config {
            host: std::env::var("HOST")
                .ok()
                .or(file_cfg.host)
                .unwrap_or("0.0.0.0".into()),
            port: std::env::var("PORT")
                .ok()
                .and_then(|p| p.parse().ok())
                .or(file_cfg.port)
                .unwrap_or(9899),
            token,
            machine_id: load_or_create_machine_id(),
            tmux_socket: std::env::var("TMUX_SOCKET").ok().or(file_cfg.tmux_socket),
            tls_cert: std::env::var("TLS_CERT").ok().or(file_cfg.tls_cert),
            tls_key: std::env::var("TLS_KEY").ok().or(file_cfg.tls_key),
            scrollback: std::env::var("SCROLLBACK")
                .ok()
                .and_then(|s| s.parse().ok())
                .or(file_cfg.scrollback)
                .unwrap_or(500),
            disconnect_grace_secs: std::env::var("DISCONNECT_GRACE_SECS")
                .ok()
                .and_then(|s| s.parse().ok())
                .or(file_cfg.disconnect_grace_secs)
                .unwrap_or(600),
            team_bind: std::env::var("TEAM_BIND")
                .ok()
                .or_else(|| std::env::var("CREW_BIND").ok())
                .or_else(|| std::env::var("AGORA_BIND").ok())
                .or(file_cfg.team_bind)
                .unwrap_or_else(|| "127.0.0.1:8787".into()),
            team_db: std::env::var("TEAM_DB")
                .ok()
                .or_else(|| std::env::var("CREW_DB").ok())
                .or_else(|| std::env::var("AGORA_DB").ok())
                .or(file_cfg.team_db)
                .unwrap_or_else(|| dirs_next().join("team.db").to_string_lossy().into_owned()),
            team_room: std::env::var("TEAM_ROOM")
                .ok()
                .or_else(|| std::env::var("CREW_ROOM").ok())
                .or_else(|| std::env::var("AGORA_ROOM").ok())
                .or(file_cfg.team_room)
                .unwrap_or_else(|| "main".into()),
            team_model: std::env::var("TEAM_MODEL")
                .ok()
                .or_else(|| std::env::var("CREW_MODEL").ok())
                .or_else(|| std::env::var("AGORA_MODEL").ok())
                .or(file_cfg.team_model)
                .unwrap_or_else(|| "claude-sonnet-4.6".into()),
            team_codex_profile: std::env::var("TEAM_CODEX_PROFILE")
                .ok()
                .or(file_cfg.team_codex_profile)
                .unwrap_or_default(),
            team_rules: std::env::var("TEAM_RULES")
                .ok()
                .or(file_cfg.team_rules)
                .unwrap_or_else(|| {
                    let rules = DEFAULT_TEAM_RULES.to_string();
                    // Seed into config.toml so the user can see and edit it.
                    let _ = append_config_field("team_rules", &rules);
                    rules
                }),
            team_kick: std::env::var("TEAM_KICK")
                .ok()
                .or(file_cfg.team_kick)
                .unwrap_or_else(|| {
                    let kick = DEFAULT_TEAM_KICK.to_string();
                    // Keep lifecycle behavior visible and operator-editable too.
                    let _ = append_config_field("team_kick", &kick);
                    kick
                }),
        }
    }
}

fn load_or_create_machine_id() -> String {
    let path = dirs_next().join("machine_id");
    if let Ok(id) = std::fs::read_to_string(&path) {
        let id = id.trim().to_string();
        if !id.is_empty() { return id; }
    }
    let id = uuid::Uuid::new_v4().to_string();
    let _ = std::fs::create_dir_all(dirs_next());
    let _ = std::fs::write(&path, &id);
    id
}

fn save_token(token: &str) -> std::io::Result<()> {
    let dir = dirs_next();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("config.toml");
    // Read existing or start fresh
    let mut content = std::fs::read_to_string(&path).unwrap_or_default();
    if content.contains("token") {
        return Ok(());
    }
    if !content.is_empty() && !content.ends_with('\n') {
        content.push('\n');
    }
    content.push_str(&format!("token = \"{}\"\n", token));
    std::fs::write(&path, content)?;
    // The token is a session-wide secret; any local user who can read
    // this file can impersonate the owner. Tighten to 0600 so cohabiting
    // accounts (shared Macs, multi-user Linux) can't trivially pick it up.
    // We only set perms when we're the writer — we do NOT tighten existing
    // files to avoid surprising users who intentionally relaxed them.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    }
    Ok(())
}

/// Tighten permissions on existing config.toml if we can. Safe to call on
/// every startup: no-op on non-unix; no-op if file absent; clamps to 0600
/// otherwise. This is a belt-and-braces measure for configs written by
/// older versions (pre-hardening) or by a manual edit that widened perms.
#[cfg(unix)]
pub fn harden_config_perms() {
    harden_path_0600(&config_path());
}
#[cfg(not(unix))]
pub fn harden_config_perms() {}

/// Clamp `path`'s mode to 0600 if any group/other bits are set. No-op on
/// non-unix. Extracted so it can be unit-tested without touching
/// `~/.config/tmux-mobile`.
#[cfg(unix)]
fn harden_path_0600(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    if let Ok(meta) = std::fs::metadata(path) {
        let mode = meta.permissions().mode() & 0o777;
        if mode & 0o077 != 0 {
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
    }
}
#[cfg(not(unix))]
#[allow(dead_code)]
fn harden_path_0600(_path: &std::path::Path) {}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::sync::atomic::{AtomicUsize, Ordering};

    static TEST_CTR: AtomicUsize = AtomicUsize::new(0);

    fn mkfile(mode: u32) -> std::path::PathBuf {
        let n = TEST_CTR.fetch_add(1, Ordering::Relaxed);
        let p = std::env::temp_dir()
            .join(format!("tmux_mobile_cfg_test_{}_{}", std::process::id(), n));
        std::fs::write(&p, b"token = \"x\"\n").unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(mode)).unwrap();
        p
    }

    #[test]
    fn harden_tightens_0644_to_0600() {
        let p = mkfile(0o644);
        harden_path_0600(&p);
        let m = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(m, 0o600, "expected 0600, got {:o}", m);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn harden_leaves_0600_alone() {
        let p = mkfile(0o600);
        harden_path_0600(&p);
        let m = std::fs::metadata(&p).unwrap().permissions().mode() & 0o777;
        assert_eq!(m, 0o600);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn harden_on_missing_file_is_noop() {
        let p = std::env::temp_dir().join("tmux_mobile_cfg_test_nonexistent");
        let _ = std::fs::remove_file(&p);
        harden_path_0600(&p); // must not panic
    }
}

/// Bookmarks: read/write ~/.config/tmux-mobile/bookmarks.json
fn bookmarks_path() -> std::path::PathBuf {
    dirs_next().join("bookmarks.json")
}

pub fn get_bookmarks() -> Vec<String> {
    std::fs::read_to_string(bookmarks_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<String>>(&s).ok())
        .unwrap_or_default()
}

pub fn save_bookmarks(bookmarks: &[String]) -> Result<(), String> {
    let dir = dirs_next();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string(bookmarks).map_err(|e| e.to_string())?;
    std::fs::write(bookmarks_path(), json).map_err(|e| e.to_string())
}

/// User preferences: synced key-value store at ~/.config/tmux-mobile/prefs.json
fn prefs_path() -> std::path::PathBuf {
    dirs_next().join("prefs.json")
}

pub fn get_prefs() -> serde_json::Value {
    std::fs::read_to_string(prefs_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or(serde_json::json!({}))
}

pub fn set_prefs(key: &str, value: serde_json::Value) -> Result<(), String> {
    let dir = dirs_next();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut prefs: serde_json::Map<String, serde_json::Value> = std::fs::read_to_string(prefs_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    prefs.insert(key.to_string(), value);
    let json = serde_json::to_string_pretty(&prefs).map_err(|e| e.to_string())?;
    std::fs::write(prefs_path(), json).map_err(|e| e.to_string())
}

/// Tauri command: return config for frontend auto-fill
pub fn get_config_json() -> serde_json::Value {
    let cfg = Config::load();
    serde_json::json!({
        "host": cfg.host,
        "port": cfg.port,
        "token": cfg.token,
        "tmux_socket": cfg.tmux_socket,
    })
}

/// Per-session "last opened in tmux-mobile" timestamp store.
/// Persisted to ~/.config/tmux-mobile/session_usage.json as a name → unix-seconds map.
/// Used by the Sessions page to sort recently-used sessions to the top.
fn session_usage_path() -> std::path::PathBuf {
    dirs_next().join("session_usage.json")
}

pub fn get_session_usage() -> std::collections::HashMap<String, u64> {
    std::fs::read_to_string(session_usage_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn touch_session(name: &str) -> Result<(), String> {
    let dir = dirs_next();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let mut map = get_session_usage();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    map.insert(name.to_string(), now);
    let json = serde_json::to_string(&map).map_err(|e| e.to_string())?;
    std::fs::write(session_usage_path(), json).map_err(|e| e.to_string())
}

/// Default collaboration rules seeded into config.toml on first run.
const DEFAULT_TEAM_RULES: &str = "\
# Team contract

## Message ownership
- End each active turn with `wait`.
- Reply to anything directly @-addressed to you and address the asker in the reply.
- An unaddressed human message is actionable only for the default recipient named by the selected team template. Everyone else treats it as context.
- Otherwise act only on an explicit assignment or a handoff defined by the template.
- Acknowledge each new assignment once before starting; then post only substantive progress, blockers, or results.

## Communication and artifacts
- Keep coordination messages concise.
- Conversational answers may be delivered directly in chat.
- For work that creates durable artifacts or decisions, store them in appropriate workspace files and point to them from chat.
- Do not create files merely to answer a conversational question.
- Before editing shared files, announce ownership to avoid overlap.

## Quality and safety
- Match verification to the task and report the evidence actually gathered.
- Diagnose root cause before changing a failing system.
- Never place credentials in prompts, config, commands, files, or logs; use environment-variable references.
- Stay within the assigned scope and report uncertainty or blockers honestly.";

const DEFAULT_TEAM_KICK: &str = "\
Join the Team conversation now by calling `wait`. Follow the shared team rules and your assigned role.";

/// Append a TOML field to config.toml (used to seed defaults on first run).
fn append_config_field(key: &str, value: &str) -> std::io::Result<()> {
    let dir = dirs_next();
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("config.toml");
    let mut content = std::fs::read_to_string(&path).unwrap_or_default();
    if content.contains(key) { return Ok(()); }
    if !content.is_empty() && !content.ends_with('\n') { content.push('\n'); }
    // Multi-line string: use TOML triple-quoted literal.
    content.push_str(&format!("{} = '''\n{}'''\n", key, value));
    std::fs::write(&path, content)
}
