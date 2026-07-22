//! Workspace identity and on-disk layout: tmux-safe slugs, per-team
//! runtime dirs, agent home preparation, and the team system prompt file.
//! Split from team.rs 2026-07-22 — content unchanged.

use std::path::PathBuf;

use super::{HEARTBEAT_SH, KEEPALIVE_SH};

/// Per-team config home under `<workspace>/.tmm/`. Lives inside the project but
/// is self-gitignored (`.tmm/.gitignore` = `*`). Each backend's config files and
/// hooks live here; prompts are passed inline.
pub(super) struct Paths {
    /// Agents' working directory (the user's project) — agents run `-c` here.
    pub(super) workspace: PathBuf,
    /// Our private config root: `.tmm/` for legacy rooms, otherwise
    /// `<workspace>/.tmm/teams/<team-id>/`.
    pub(super) home: PathBuf,
    pub(super) kiro: PathBuf,
    pub(super) claude: PathBuf,
    pub(super) codex: PathBuf,
    pub(super) keepalive: PathBuf,
    pub(super) heartbeat: PathBuf,
}

impl Paths {
    pub(super) fn new(workspace: &str, room: &str) -> Self {
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
pub(super) fn prepare_home(p: &Paths) -> std::io::Result<()> {
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
pub(super) fn prepare_kiro_home(p: &Paths) -> std::io::Result<()> {
    let legacy = p.workspace.join(".tmm").join("kiro-home");
    if p.home == p.workspace.join(".tmm") && legacy.exists() && !p.kiro.exists() {
        std::fs::rename(&legacy, &p.kiro)?;
    }
    std::fs::create_dir_all(&p.kiro)
}

#[cfg(test)]
mod tests {
    use super::*;

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
