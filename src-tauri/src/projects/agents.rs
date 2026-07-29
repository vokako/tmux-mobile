//! Which agent CLI is running in a pane, and how to start it again.
//!
//! One table, two uses: detection during capture and the launch line `up`
//! replays. Keeping them together is the point — a detector that recognises
//! "codex" but relaunches something else would silently rebuild the wrong
//! workspace. P2 replaces this table with the real agent definitions on disk
//! (see `docs/exec-plans/projects-and-tasks.md` §5); until then it stays
//! deliberately small.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KnownAgent {
    /// Stable name we store in the slot.
    pub backend: &'static str,
    /// Lowercase substring searched in the pane's command text.
    pub needle: &'static str,
    /// What `up` runs in a fresh window to bring the agent back.
    pub launch: &'static str,
}

/// Kimi must precede Kiro: a Kimi pane's process chain contains its
/// `kiro-web-search` helper, so an array-order rule would paint it as Kiro.
/// Earliest match in the text wins regardless (see `detect`), but keeping the
/// order meaningful documents the trap.
const KNOWN: &[KnownAgent] = &[
    KnownAgent { backend: "kimi", needle: "kimi", launch: "kimi" },
    KnownAgent { backend: "kiro", needle: "kiro", launch: "kiro-cli chat" },
    KnownAgent { backend: "claude", needle: "claude", launch: "claude" },
    KnownAgent { backend: "codex", needle: "codex", launch: "codex" },
    KnownAgent { backend: "openclaw", needle: "openclaw", launch: "openclaw" },
];

/// The agent running in a pane, or `None` for an ordinary shell.
///
/// `text` must be ordered shallow → deep (`pane_current_command`, then the
/// title, then the foreground child's argv) because the EARLIEST match wins:
/// a late match is a subprocess the agent spawned, not what the user launched.
/// Claude Code needs no special case even though its process name is a bare
/// version number — its argv path contains `.../claude/versions/<v>`.
pub fn detect(text: &str) -> Option<&'static KnownAgent> {
    let lower = text.to_lowercase();
    let mut best: Option<(usize, &'static KnownAgent)> = None;
    for agent in KNOWN {
        if let Some(idx) = lower.find(agent.needle) {
            if best.is_none_or(|(prev, _)| idx < prev) {
                best = Some((idx, agent));
            }
        }
    }
    best.map(|(_, a)| a)
}

/// The launch line for a backend name we stored earlier.
pub fn launch_for(backend: &str) -> Option<&'static str> {
    KNOWN.iter().find(|a| a.backend == backend).map(|a| a.launch)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_the_shallowest_agent_not_the_first_listed() {
        // codex spawning a kiro-web-search MCP tool: "kiro" sits deeper.
        let a = detect("node codex /Users/me/.codex/bin kiro-web-search").unwrap();
        assert_eq!(a.backend, "codex");
    }

    #[test]
    fn kimi_wins_over_its_kiro_helper() {
        let a = detect("kimi-code kiro-web-search").unwrap();
        assert_eq!(a.backend, "kimi");
    }

    #[test]
    fn claude_is_found_through_its_version_named_binary_path() {
        let a = detect("2.1.141  /Users/me/.local/share/claude/versions/2.1.141").unwrap();
        assert_eq!(a.backend, "claude");
        assert_eq!(a.launch, "claude");
    }

    #[test]
    fn a_plain_shell_is_not_an_agent() {
        assert!(detect("zsh").is_none());
        assert!(detect("npm run dev").is_none());
        assert!(detect("").is_none());
    }

    #[test]
    fn launch_lines_round_trip_by_backend_name() {
        assert_eq!(launch_for("kiro"), Some("kiro-cli chat"));
        assert_eq!(launch_for("nope"), None);
    }
}
