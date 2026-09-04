//! The app-wide agent instructions — `<config>/AGENTS.md`.
//!
//! A managed agent runs from an ISOLATED home (agents-overview.md), so the
//! user's own global memory files (`~/.claude/CLAUDE.md`, `~/.codex/AGENTS.md`,
//! kiro's global config) are deliberately NOT read: nothing leaks in, nothing
//! leaks out. The owner noticed the consequence (2026-09-04: "我发现现在我启动
//! 的 kiro claude 啊 这些好像都没有这种全局的系统提示词了") and asked for the
//! replacement to be a setting of THIS software: one file, edited in the app
//! or with any editor, prepended to every managed agent's system prompt
//! regardless of backend. It is the first block `spawn::build_prompt` writes —
//! before the agent's own persona, because a house rule outranks a role.
//!
//! It is a FILE, not a database row, because it is markdown the human writes
//! and wants to `cat`, diff and back up like a CLAUDE.md. Read at SPAWN time:
//! a change reaches every agent started after it; a running agent keeps the
//! prompt it was started with (its launch recipe is replayed verbatim), so
//! `tmm agent restart` is how an existing one picks the new text up.

use std::path::PathBuf;

/// Where the file lives — beside `config.toml`, `prefs.json` and `skills/`.
pub fn path() -> PathBuf {
    crate::config::config_dir().join("AGENTS.md")
}

/// A ceiling, because the text rides on claude's `--append-system-prompt`
/// argument and codex's `developer_instructions` override: a novel there is a
/// launch line that no longer fits a tmux `send-keys`. 24 KB is ~6k tokens.
pub const MAX_BYTES: usize = 24 * 1024;

/// The current text, trimmed; empty when the file is absent or blank.
pub fn read() -> String {
    std::fs::read_to_string(path()).map(|s| s.trim().to_string()).unwrap_or_default()
}

/// Replace the text. Empty (after trim) DELETES the file, so "cleared" and
/// "never written" are the same state and `read()` needs no second truth.
pub fn write(text: &str) -> Result<(), String> {
    let trimmed = text.trim();
    if trimmed.len() > MAX_BYTES {
        return Err(format!("global instructions are {} bytes; the limit is {MAX_BYTES}", trimmed.len()));
    }
    let p = path();
    if trimmed.is_empty() {
        match std::fs::remove_file(&p) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(format!("remove {}: {e}", p.display())),
        }
    } else {
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        std::fs::write(&p, format!("{trimmed}\n")).map_err(|e| format!("write {}: {e}", p.display()))
    }
}

/// Prepend the global block to a prompt body (pure, tested). The block is
/// separated by a blank line and carries no header of its own: the file IS
/// the header — it is the human's markdown, shown as written.
pub fn compose(global: &str, body: &str) -> String {
    let g = global.trim();
    if g.is_empty() {
        return body.to_string();
    }
    format!("{g}\n\n{body}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compose_prepends_the_global_block_or_leaves_the_body_alone() {
        assert_eq!(compose("", "You are dev."), "You are dev.");
        assert_eq!(compose("  \n", "You are dev."), "You are dev.");
        assert_eq!(compose("# House rules\nBe terse.\n", "You are dev."), "# House rules\nBe terse.\n\nYou are dev.");
    }

    #[test]
    fn write_rejects_a_novel() {
        let big = "x".repeat(MAX_BYTES + 1);
        let err = write_to(&std::env::temp_dir().join(format!("tmm-gp-{}", uuid::Uuid::new_v4())), &big).unwrap_err();
        assert!(err.contains("limit"), "{err}");
    }

    #[test]
    fn empty_write_removes_the_file_so_cleared_equals_absent() {
        let p = std::env::temp_dir().join(format!("tmm-gp-{}", uuid::Uuid::new_v4())).join("AGENTS.md");
        write_to(&p, "Be kind.").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "Be kind.\n");
        write_to(&p, "   ").unwrap();
        assert!(!p.exists());
        write_to(&p, "").unwrap(); // idempotent on an absent file
    }

    /// The same body as `write`, at an explicit path (the public one is bound
    /// to the config dir, which a test must not touch).
    fn write_to(p: &std::path::Path, text: &str) -> Result<(), String> {
        let trimmed = text.trim();
        if trimmed.len() > MAX_BYTES {
            return Err(format!("global instructions are {} bytes; the limit is {MAX_BYTES}", trimmed.len()));
        }
        if trimmed.is_empty() {
            return match std::fs::remove_file(p) {
                Ok(()) => Ok(()),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
                Err(e) => Err(e.to_string()),
            };
        }
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        std::fs::write(p, format!("{trimmed}\n")).map_err(|e| e.to_string())
    }
}
