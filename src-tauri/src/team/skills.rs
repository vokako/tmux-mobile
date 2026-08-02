//! Skill resolution for team agents: local/team-bundled skill dirs and
//! GitHub-referenced skills (sparse-cloned into a shared cache), plus the
//! compact skills index injected into CLI system prompts.
//! Split from team.rs 2026-07-22 — content unchanged.

use std::path::PathBuf;

use serde_json::Value;

pub(crate) struct ResolvedSkill {
    pub(crate) name: String,
    pub(crate) dir: PathBuf,
    pub(crate) description: String,
}

/// A compact system-level skills index for backends without a native skill
/// mechanism (claude/codex). Kiro instead gets `skill://` resources.
pub(crate) fn skills_index_text(skills: &[ResolvedSkill]) -> String {
    if skills.is_empty() {
        return String::new();
    }
    let mut s = String::from("Skills available — read the named SKILL.md before a matching task:");
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
pub(crate) fn resolve_skills(refs: &[String], team_dir: &str) -> Vec<ResolvedSkill> {
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

/// Drop the clone cache for a GitHub skill URL so the next resolve re-fetches
/// the remote's current state. Used by the registry's skill refresh — the
/// cache is keyed owner/repo/ref and otherwise lives forever.
pub(crate) fn invalidate_git_cache(url: &str) {
    if let Ok((owner, repo, gitref, _)) = parse_github(url) {
        let _ = std::fs::remove_dir_all(skills_cache_dir().join(&owner).join(&repo).join(&gitref));
    }
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
