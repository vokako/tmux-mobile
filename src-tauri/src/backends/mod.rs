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

    /// The kind ("completed"/"failed"/"permission_required"/"input_required")
    /// carried by one hook payload, read with this backend's own key spelling
    /// and screening rules — each arm lives on the backend's file (board
    /// #129). Compiled on every target: the mobile shell consumes the same
    /// inbox envelopes.
    pub fn normalize_kind(
        self,
        payload: &serde_json::Map<String, serde_json::Value>,
    ) -> Result<&'static str, String> {
        match self {
            Backend::Kiro => kiro::normalize_kind(payload),
            Backend::Claude => claude::normalize_kind(payload),
            Backend::Codex => codex::normalize_kind(payload),
            Backend::Grok => grok::normalize_kind(payload),
            Backend::Omp => omp::normalize_kind(payload),
        }
    }

    /// True when the payload marks the start of a new user turn (resets the
    /// auto-post dedup flag), in this backend's spelling.
    pub fn is_user_prompt_submit(self, payload: &serde_json::Value) -> bool {
        match self {
            Backend::Kiro => kiro::is_user_prompt_submit(payload),
            Backend::Claude => claude::is_user_prompt_submit(payload),
            Backend::Codex => codex::is_user_prompt_submit(payload),
            Backend::Grok => grok::is_user_prompt_submit(payload),
            Backend::Omp => omp::is_user_prompt_submit(payload),
        }
    }

    /// True for claude's idle reminder — an event that must never read as an
    /// ask (board #75). Only claude has one.
    pub fn is_idle_nudge(self, payload: &serde_json::Value) -> bool {
        match self {
            Backend::Claude => claude::is_idle_nudge(payload),
            _ => false,
        }
    }

    /// The backend's detection/relaunch row for the KNOWN table
    /// (projects/agents.rs assembles it from ALL, board #129).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn known(self) -> crate::projects::agents::KnownAgent {
        match self {
            Backend::Kiro => kiro::known(),
            Backend::Claude => claude::known(),
            Backend::Codex => codex::known(),
            Backend::Grok => grok::known(),
            Backend::Omp => omp::known(),
        }
    }

    /// Render an agent's isolated home from its definition — prompt, backend
    /// config, hooks, launch command (board #128). Each arm is the backend's
    /// own file; `workspace` is read by claude alone (trust pre-seeding).
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    #[allow(clippy::too_many_arguments)]
    pub fn render(
        self,
        def: &crate::projects::store::RegAgent,
        window_name: &str,
        home: &std::path::Path,
        workspace: &std::path::Path,
        system_prompt: &str,
        skills: &[crate::projects::skills::ResolvedSkill],
    ) -> Result<crate::projects::spawn::Rendered, String> {
        match self {
            Backend::Kiro => kiro::render_kiro(def, window_name, home, system_prompt, skills),
            Backend::Claude => claude::render_claude(def, window_name, home, workspace, system_prompt, skills),
            Backend::Codex => codex::render_codex(def, window_name, home, system_prompt, skills),
            Backend::Grok => grok::render_grok(def, window_name, home, system_prompt, skills),
            Backend::Omp => omp::render_omp(def, window_name, home, system_prompt, skills),
        }
    }

    /// The backend's resume dialect on one persisted identity command.
    pub fn resume_command(self, cmd: &str, id: Option<&str>) -> String {
        match self {
            Backend::Kiro => kiro::resume_command(cmd, id),
            Backend::Claude => claude::resume_command(cmd, id),
            Backend::Codex => codex::resume_command(cmd, id),
            Backend::Grok => grok::resume_command(cmd, id),
            Backend::Omp => omp::resume_command(cmd, id),
        }
    }

    /// Repair this backend's on-disk surface in a managed home (hooks,
    /// settings, channel keys, recipe backfill). Probes for its own files and
    /// no-ops when the home is not this backend's.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    pub fn refresh(
        self,
        home: &std::path::Path,
        window_name: &str,
        workspace: &std::path::Path,
        notifications: &crate::agent_notifications::AgentNotificationHub,
    ) -> bool {
        match self {
            Backend::Kiro => kiro::refresh(home, window_name, &notifications.helper_command("kiro")),
            Backend::Claude => claude::refresh(home, workspace, &notifications.helper_command("claude")),
            Backend::Codex => codex::refresh(home, &notifications.helper_command("codex")),
            Backend::Grok => grok::refresh(home, &notifications.helper_command("grok")),
            Backend::Omp => omp::refresh(home, &notifications.helper_command("omp")),
        }
    }

    /// Read the CLI's own status furniture from a pane capture — each
    /// backend paints its own dialect, and reading one CLI's screen with
    /// another's grammar yields confident nonsense (agent-status.md).
    /// `agent` anchors kiro's positional status line; the other dialects
    /// ignore it.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
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
