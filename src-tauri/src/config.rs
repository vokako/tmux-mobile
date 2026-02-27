use serde::Deserialize;
use std::path::PathBuf;

#[derive(Deserialize, Default)]
struct FileConfig {
    host: Option<String>,
    port: Option<u16>,
    token: Option<String>,
}

pub struct Config {
    pub host: String,
    pub port: u16,
    pub token: String,
}

fn config_path() -> PathBuf {
    dirs_next().join("config.toml")
}

fn dirs_next() -> PathBuf {
    if let Some(home) = std::env::var_os("HOME") {
        PathBuf::from(home).join(".config").join("tmux-mobile")
    } else {
        PathBuf::from(".config").join("tmux-mobile")
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
                // Auto-generate and persist
                let t = uuid::Uuid::new_v4().to_string();
                let _ = save_token(&t);
                t
            });

        Config {
            host: std::env::var("HOST").ok().or(file_cfg.host).unwrap_or("0.0.0.0".into()),
            port: std::env::var("PORT").ok().and_then(|p| p.parse().ok()).or(file_cfg.port).unwrap_or(9876),
            token,
        }
    }
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
    std::fs::write(&path, content)
}
