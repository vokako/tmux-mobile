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
    // The kiro-cli agent engine managed kiro agents launch with: "v2" (the
    // CLI's default, ours too) or "v3" (kiro-cli 2.22.1's KAS engine). Read
    // at spawn and at every refresh; an agent takes it at its next restart
    // (board #207).
    kiro_engine: Option<String>,
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
    pub kiro_engine: String,
}

/// The kiro engine door, side-effect free (no token/machine-id seeding — the
/// backend reads it at spawn/refresh time): `KIRO_ENGINE` env, else
/// `kiro_engine` in config.toml, else "v2". Anything but v2/v3 is v2 with a
/// note on stderr.
pub fn kiro_engine() -> String {
    let raw = std::env::var("KIRO_ENGINE")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .or_else(|| {
            std::fs::read_to_string(config_path())
                .ok()
                .and_then(|t| toml::from_str::<FileConfig>(&t).ok())
                .and_then(|c| c.kiro_engine)
        })
        .unwrap_or_default();
    normalize_engine(&raw)
}

pub fn normalize_engine(raw: &str) -> String {
    match raw.trim() {
        "" | "v2" => "v2".into(),
        "v3" => "v3".into(),
        other => {
            eprintln!("config: kiro_engine {other:?} is not v2|v3 — using v2");
            "v2".into()
        }
    }
}

fn config_path() -> PathBuf {
    dirs_next().join("config.toml")
}

/// The tmux-mobile config directory (`~/.config/tmux-mobile`).
pub fn config_dir() -> PathBuf {
    dirs_next()
}

#[cfg(not(test))]
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

/// The unit-test process never sees the operator's config directory. Every
/// `config_dir()` consumer (config.toml, AGENTS.md, the hooks helper,
/// skills-cache, the default state.db) resolves to ONE empty temp directory
/// created once per test process — so a test cannot forget to isolate
/// itself, and the file defaults (`kiro_engine` = v2, no AGENTS.md, …) are
/// what every test measures. Per-test opt-in had already failed once (the
/// spawn tests once pointed the whole process at the real state.db, see
/// `projects::tests::use_test_store`), and it failed again when the live
/// `kiro_engine = "v3"` turned three v2-default assertions red on this host
/// (board #216, 2026-09-20). Tests that WRITE config files use their own
/// subdirectory under this one, not its root.
#[cfg(test)]
fn dirs_next() -> PathBuf {
    static DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("tmm-config-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create the test config dir");
        dir
    })
    .clone()
}

fn optional_env_override(value: Option<String>, fallback: Option<String>) -> Option<String> {
    match value {
        Some(value) if value.trim().is_empty() => None,
        Some(value) => Some(value),
        None => fallback,
    }
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
            tls_cert: optional_env_override(std::env::var("TLS_CERT").ok(), file_cfg.tls_cert),
            tls_key: optional_env_override(std::env::var("TLS_KEY").ok(), file_cfg.tls_key),
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
            kiro_engine: normalize_engine(
                &std::env::var("KIRO_ENGINE").ok().or(file_cfg.kiro_engine).unwrap_or_default(),
            ),
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

    #[test]
    fn empty_optional_env_value_disables_file_fallback() {
        let fallback = Some("configured.pem".to_string());
        assert_eq!(
            optional_env_override(Some(String::new()), fallback.clone()),
            None
        );
        assert_eq!(optional_env_override(None, fallback.clone()), fallback);
        assert_eq!(
            optional_env_override(Some("override.pem".to_string()), None),
            Some("override.pem".to_string()),
        );
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


#[cfg(test)]
mod engine_tests {
    use super::normalize_engine;
    /// Board #207: the kiro engine door has two positions; anything else is the default.
    #[test]
    fn engine_door_is_v2_or_v3() {
        assert_eq!(normalize_engine(""), "v2");
        assert_eq!(normalize_engine("v2"), "v2");
        assert_eq!(normalize_engine(" v3 "), "v3");
        assert_eq!(normalize_engine("v1"), "v2", "v1 is not offered: v2");
        assert_eq!(normalize_engine("kas"), "v2");
    }
}
