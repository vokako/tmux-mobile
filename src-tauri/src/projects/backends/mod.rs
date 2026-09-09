//! Per-backend knowledge for the agents-v2 spawn path (projects/spawn.rs).
//!
//! `shared` holds the backend-neutral launch helpers that used to live in
//! `team::backends_shared` (board #100): per-backend MCP rendering, the
//! launch-script pattern (the 2KB tty lesson) and startup-prompt confirmation.
//! Moved here so the spawn path owns its dependencies and the desktop Team
//! system can be deleted whole (docs/todo.md §A).

pub(crate) mod claude;
pub(crate) mod codex;
pub(crate) mod grok;
pub(crate) mod kiro;
pub(crate) mod omp;
pub(crate) mod shared;

/// The closed set of agent CLIs this app can spawn (board #101/#127). An enum
/// rather than a trait object on purpose: the set is closed, and every
/// per-concern method below is an EXHAUSTIVE match with one-line arms into
/// the backend's own file — so adding a backend is adding a variant, and the
/// compiler's error list is exactly the file the new backend must fill
/// (2026-09-07: adding omp missed registry_save; this makes that class of
/// omission a compile error). The `backend` field stays a STRING at rest
/// (state.db, recipes, the wire); `parse` runs once at each boundary
/// (tenet 8: validate at the door).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Backend {
    Kiro,
    Claude,
    Codex,
    Grok,
    Omp,
}

impl Backend {
    /// Every backend, in the one canonical order (seeds, pickers, caps).
    pub const ALL: [Backend; 5] =
        [Backend::Kiro, Backend::Claude, Backend::Codex, Backend::Grok, Backend::Omp];

    /// `ALL` as the string names — kept literally beside it so a `const` can
    /// borrow it (`SPAWNABLE_BACKENDS`); the roundtrip test pins the two in
    /// sync.
    pub const NAMES: [&'static str; 5] = ["kiro", "claude", "codex", "grok", "omp"];

    /// The documented fallback where an absent backend means kiro (the
    /// registry dispatch's old inline `unwrap_or("kiro")`).
    pub const DEFAULT: Backend = Backend::Kiro;

    pub fn parse(name: &str) -> Option<Backend> {
        match name.trim() {
            "kiro" => Some(Backend::Kiro),
            "claude" => Some(Backend::Claude),
            "codex" => Some(Backend::Codex),
            "grok" => Some(Backend::Grok),
            "omp" => Some(Backend::Omp),
            _ => None,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Backend::Kiro => "kiro",
            Backend::Claude => "claude",
            Backend::Codex => "codex",
            Backend::Grok => "grok",
            Backend::Omp => "omp",
        }
    }

    /// The reasoning-effort levels this backend's CLI accepts — a FIXED enum
    /// per backend, measured per CLI, never guessed (each list's provenance
    /// is on the backend file).
    pub fn effort_values(self) -> &'static [&'static str] {
        match self {
            Backend::Kiro => kiro::effort_values(),
            Backend::Claude => claude::effort_values(),
            Backend::Codex => codex::effort_values(),
            Backend::Grok => grok::effort_values(),
            Backend::Omp => omp::effort_values(),
        }
    }

    /// Read the CLI's own status furniture from a pane capture — each
    /// backend paints its own dialect, and reading one CLI's screen with
    /// another's grammar yields confident nonsense (agent-status.md).
    /// `agent` anchors kiro's positional status line; the other dialects
    /// ignore it.
    pub fn sniff(self, pane: &str, agent: &str) -> crate::projects::vitals::Vitals {
        match self {
            Backend::Kiro => kiro::sniff_kiro(pane, agent),
            Backend::Claude => claude::sniff_claude(pane),
            Backend::Codex => codex::sniff_codex(pane),
            Backend::Grok => grok::sniff_grok(pane),
            Backend::Omp => omp::sniff_omp(pane),
        }
    }

    /// Ask the backend's own CLI for its model ids. `None` = no authoritative
    /// list exists (claude/codex/omp aliases) or the CLI is unavailable —
    /// validation then degrades to accept-all (models.rs module doc).
    pub fn models_fetch(self) -> Option<Vec<String>> {
        match self {
            Backend::Kiro => kiro::models_fetch(),
            Backend::Claude => claude::models_fetch(),
            Backend::Codex => codex::models_fetch(),
            Backend::Grok => grok::models_fetch(),
            Backend::Omp => omp::models_fetch(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Backend;

    #[test]
    fn parse_and_name_roundtrip_and_names_mirror_all() {
        for (i, b) in Backend::ALL.into_iter().enumerate() {
            assert_eq!(Backend::parse(b.name()), Some(b));
            assert_eq!(Backend::NAMES[i], b.name(), "NAMES stays in ALL's order");
        }
        assert_eq!(Backend::parse(" kiro "), Some(Backend::Kiro), "boundary input is trimmed");
        assert_eq!(Backend::parse("kimi"), None);
        assert_eq!(Backend::DEFAULT.name(), "kiro");
    }
}
