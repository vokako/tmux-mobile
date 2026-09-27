//! The registry: agent, team, skill and MCP definitions, and the shipped defaults.
//!
//! One family of the store (board #147): an `impl Store` block over the same
//! connection, the same migration ladder, the same tests — only the file moved.

use super::*;

/// The `omp` default's system text — shared by `reg_seed` (fresh installs)
/// and the v18 backfill migration (existing installs), so the two cannot
/// drift.
pub(super) const DEFAULT_OMP_SYSTEM: &str = "You are a powerful 10x developer running on OMP (oh-my-pi) who can handle any task with decisive execution and minimal words.";

/// The omp default's model (owner, 2026-09-07: "设定默认的 omp agent 模型为
/// fable 5.1"). `bedrock-extra` is the provider the user's `~/.omp/agent/
/// models.yml` declares (the bundled omp catalog lacks Fable 5.1 on Bedrock);
/// `render_omp` carries that file into every isolated home, so the selector
/// resolves for spawned agents. On a machine without the catalog entry, omp
/// warns and falls back to its own default — the same soft degradation the
/// claude seed's Bedrock pin relies on.
pub(super) const DEFAULT_OMP_MODEL: &str = "bedrock-extra/global.anthropic.claude-fable-5-1";

/// The `kimi` default's system text — shared by `reg_seed` and the v22
/// backfill migration (board #224), so the two cannot drift. The model stays
/// EMPTY: a kimi model is an alias of the user's own `[models]` table, which
/// `render_kimi` carries into every isolated home together with the user's
/// `default_model` — so "backend default" already means the owner's Bedrock
/// K3 here and needs no per-machine alias in the seed.
pub(super) const DEFAULT_KIMI_SYSTEM: &str = "You are a powerful 10x developer running on Kimi Code who can handle any task with decisive execution and minimal words.";

pub(super) const LEGACY_DEFAULT_KIRO_SYSTEM: &str = "You are a powerful 10x developer running on Kiro CLI who can handle any task with decisive execution and minimal words.";

pub(super) const VERBOSE_DEFAULT_KIRO_SYSTEM: &str = concat!(
    "You are a powerful 10x developer running on Kiro CLI who can handle any task with decisive execution and minimal words.",
    "\n\nGit workflow:\n",
    "- Before changing tracked files, create or reuse a dedicated Git worktree and task branch. If this session already runs inside that task worktree, use it; otherwise keep the launch checkout for reading, coordination, and final integration only.\n",
    "- Use the worktree's absolute path for every file edit, test, build, and Git command. Verify and commit in the worktree before integrating.\n",
    "- Preserve the user's configured Git author. Every commit you materially author must end with exactly one trailer, separated from the body by a blank line:\n",
    "  Co-authored-by: Kiro Agent <244629292+kiro-agent@users.noreply.github.com>\n",
    "- Do not add the Kiro trailer when you only review or integrate someone else's commit."
);

pub(super) const DEFAULT_KIRO_SYSTEM: &str = concat!(
    "You are a powerful 10x developer running on Kiro CLI who can handle any task with decisive execution and minimal words.",
    "\n\nFor code changes, use a dedicated Git worktree instead of the launch checkout. Preserve the user's configured Git author and add this trailer to every commit you author:\n",
    "Co-authored-by: Kiro Agent <244629292+kiro-agent@users.noreply.github.com>"
);

/// A central skill asset. The FILES live in the app-managed skills dir; the
/// `source` (local path or git url) is where they were imported from and what
/// a refresh re-syncs against.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegSkill {
    pub name: String,
    pub source: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub synced_at: Option<u64>,
}

/// A central MCP server def: name → def JSON ({command,args,env} or {url,headers}).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegMcp {
    pub name: String,
    pub def: String,
}

/// A registry agent definition. `skills` and `mcp` are stored as JSON text
/// (refs and defs respectively) — parsed only at spawn time.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegAgent {
    pub name: String,
    pub backend: String,
    #[serde(default)]
    pub model: String,
    /// Reasoning effort (low|medium|high|…, backend-specific). Empty = the
    /// backend's default, same contract as `model`.
    #[serde(default)]
    pub effort: String,
    /// What a line typed at the BUSY agent does (board #245): `queue` waits
    /// for the turn to end and becomes the next prompt, `steer` goes into the
    /// running turn. `queue` is the default for every agent; only a backend
    /// whose switch is measured (`Backend::switches_input_mode`) accepts
    /// `steer`, and its backend file renders the value into its own config.
    #[serde(default = "default_input_mode")]
    pub input_mode: String,
    #[serde(default)]
    pub system: String,
    /// JSON array of skill refs (local names or github URLs).
    #[serde(default = "empty_json_array")]
    pub skills: String,
    /// JSON array of MCP server defs ({name, command/url, args, env, headers}).
    #[serde(default = "empty_json_array")]
    pub mcp: String,
}

pub(super) fn empty_json_array() -> String {
    "[]".to_string()
}

pub(crate) fn default_input_mode() -> String {
    "queue".to_string()
}

/// An agent TEAM (board #74): a named list of members. `members` is a JSON
/// array of `teams::Member` — kept as text for the same reason an agent's
/// `skills` is: the row is edited whole and never joined against.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RegTeam {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default = "empty_json_array")]
    pub members: String,
}

pub(super) fn row_to_reg_team(r: &rusqlite::Row<'_>) -> rusqlite::Result<RegTeam> {
    Ok(RegTeam { name: r.get(0)?, description: r.get(1)?, members: r.get(2)? })
}

pub(super) fn row_to_reg_agent(r: &rusqlite::Row<'_>) -> rusqlite::Result<RegAgent> {
    Ok(RegAgent {
        name: r.get(0)?,
        backend: r.get(1)?,
        model: r.get(2)?,
        effort: r.get(3)?,
        system: r.get(4)?,
        skills: r.get(5)?,
        mcp: r.get(6)?,
        input_mode: r.get(7)?,
    })
}

impl Store {
    pub fn reg_list(&self) -> Result<Vec<RegAgent>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT name, backend, model, effort, system, skills, mcp, input_mode
                   FROM reg_agents
                  ORDER BY CASE name
                    WHEN 'kiro' THEN 0
                    WHEN 'codex' THEN 1
                    WHEN 'claude' THEN 2
                    WHEN 'grok' THEN 3
                    WHEN 'omp' THEN 4
                    WHEN 'kimi' THEN 5
                    ELSE 6
                  END, name"
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_reg_agent)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn reg_get(&self, name: &str) -> Result<Option<RegAgent>, String> {
        self.conn
            .query_row(
                "SELECT name, backend, model, effort, system, skills, mcp, input_mode FROM reg_agents WHERE name = ?1",
                [name],
                row_to_reg_agent,
            )
            .map(Some)
            .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e.to_string()) })
    }

    /// Upsert by name — the registry is edited whole-row (like team templates).
    pub fn reg_save(&self, a: &RegAgent, now: u64) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO reg_agents (name, backend, model, effort, system, skills, mcp, input_mode, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?9, ?8, ?8)
                 ON CONFLICT(name) DO UPDATE SET
                   backend=?2, model=?3, effort=?4, system=?5, skills=?6, mcp=?7, input_mode=?9, updated_at=?8",
                rusqlite::params![
                    a.name,
                    a.backend,
                    a.model,
                    a.effort,
                    a.system,
                    a.skills,
                    a.mcp,
                    now as i64,
                    a.input_mode,
                ],
            )
            .map(|_| ())
            .map_err(|e| format!("save agent {}: {e}", a.name))
    }

    pub fn reg_delete(&self, name: &str) -> Result<bool, String> {
        self.conn
            .execute("DELETE FROM reg_agents WHERE name = ?1", [name])
            .map(|n| n > 0)
            .map_err(|e| e.to_string())
    }

    pub fn teams_list(&self) -> Result<Vec<RegTeam>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, description, members FROM reg_teams ORDER BY name")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_reg_team)
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn team_get(&self, name: &str) -> Result<Option<RegTeam>, String> {
        self.conn
            .query_row("SELECT name, description, members FROM reg_teams WHERE name = ?1", [name], row_to_reg_team)
            .map(Some)
            .or_else(|e| if e == rusqlite::Error::QueryReturnedNoRows { Ok(None) } else { Err(e.to_string()) })
    }

    /// Upsert by name — whole-row, like `reg_save`.
    pub fn team_save(&self, t: &RegTeam, now: u64) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO reg_teams (name, description, members, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?4)
                 ON CONFLICT(name) DO UPDATE SET description=?2, members=?3, updated_at=?4",
                rusqlite::params![t.name, t.description, t.members, now as i64],
            )
            .map(|_| ())
            .map_err(|e| format!("save team {}: {e}", t.name))
    }

    pub fn team_delete(&self, name: &str) -> Result<bool, String> {
        self.conn
            .execute("DELETE FROM reg_teams WHERE name = ?1", [name])
            .map(|n| n > 0)
            .map_err(|e| e.to_string())
    }

    /// Seed the six backend-native defaults once (empty table only).
    /// `docs` / `reviewer` and `*-default` aliases are deliberately retired:
    /// one obvious entry per backend, with the defaults pinned to the top of
    /// every registry consumer by `reg_list`.
    pub fn reg_seed(&self, now: u64) -> Result<(), String> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM reg_agents", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if count > 0 {
            // Upgrade only the two known untouched Kiro defaults. Existing
            // installs often customize model/effort/skills/MCP while leaving
            // the seeded system text alone; a narrow SQL update preserves every
            // one of those fields. A custom persona must never be overwritten.
            self.conn
                .execute(
                    "UPDATE reg_agents SET system=?1, updated_at=?2 WHERE name='kiro' AND system IN (?3, ?4)",
                    params![
                        DEFAULT_KIRO_SYSTEM,
                        now as i64,
                        LEGACY_DEFAULT_KIRO_SYSTEM,
                        VERBOSE_DEFAULT_KIRO_SYSTEM
                    ],
                )
                .map_err(|e| format!("upgrade default Kiro agent: {e}"))?;
            return Ok(());
        }
        // backend-seeds:begin — the shipped default agent per backend. The
        // ONE place outside src/backends/ allowed to spell backend names: a
        // seed is registry DATA (a name, a persona), not backend knowledge,
        // and the literal guard (backends::tests) skips this fenced region.
        let seeds = [
            RegAgent {
                name: "kiro".into(),
                backend: "kiro".into(),
                model: String::new(),
                effort: String::new(),
                input_mode: "queue".into(),
                system: DEFAULT_KIRO_SYSTEM.into(),
                skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
                mcp: "[]".into(),
            },
            RegAgent {
                name: "codex".into(),
                backend: "codex".into(),
                model: String::new(),
                effort: String::new(),
                input_mode: "queue".into(),
                system: "You are a powerful 10x developer running on Codex who can handle any task with decisive execution and minimal words.".into(),
                skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
                mcp: r#"[{"name":"kiro-web-search","command":"uvx","args":["kiro-web-search==0.1.3"]}]"#.into(),
            },
            RegAgent {
                name: "claude".into(),
                backend: "claude".into(),
                model: "global.anthropic.claude-fable-5-1[1m]".into(),
                effort: String::new(),
                input_mode: "queue".into(),
                system: "You are a powerful 10x developer running on Claude Code who can handle any task with decisive execution and minimal words.".into(),
                skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
                mcp: r#"[{"name":"kiro-web-search","command":"uvx","args":["kiro-web-search==0.1.3"]}]"#.into(),
            },
            RegAgent {
                name: "grok".into(),
                backend: "grok".into(),
                model: String::new(),
                effort: String::new(),
                input_mode: "queue".into(),
                system: "You are a powerful 10x developer running on Grok who can handle any task with decisive execution and minimal words.".into(),
                skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
                mcp: r#"[{"name":"kiro-web-search","command":"uvx","args":["kiro-web-search==0.1.3"]}]"#.into(),
            },
            RegAgent {
                name: "omp".into(),
                backend: "omp".into(),
                model: DEFAULT_OMP_MODEL.into(),
                effort: String::new(),
                input_mode: "queue".into(),
                system: DEFAULT_OMP_SYSTEM.into(),
                skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
                // omp ships its own web_search tool — no MCP search shim needed.
                mcp: "[]".into(),
            },
            RegAgent {
                name: "kimi".into(),
                backend: "kimi".into(),
                model: String::new(),
                effort: String::new(),
                input_mode: "queue".into(),
                system: DEFAULT_KIMI_SYSTEM.into(),
                skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
                // kimi's built-in search is the Moonshot service (a Kimi API
                // key); on Bedrock it has none, so the MCP shim like codex.
                mcp: r#"[{"name":"kiro-web-search","command":"uvx","args":["kiro-web-search==0.1.3"]}]"#.into(),
            },
        ];
        // backend-seeds:end
        for s in &seeds {
            self.reg_save(s, now)?;
        }
        Ok(())
    }

    pub fn skills_list(&self) -> Result<Vec<RegSkill>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, source, description, synced_at FROM reg_skills ORDER BY name")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| {
                Ok(RegSkill {
                    name: r.get(0)?,
                    source: r.get(1)?,
                    description: r.get(2)?,
                    synced_at: r.get::<_, Option<i64>>(3)?.map(|v| v as u64),
                })
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn skill_get(&self, name: &str) -> Result<Option<RegSkill>, String> {
        Ok(self.skills_list()?.into_iter().find(|s| s.name == name))
    }

    pub fn skill_save(&self, sk: &RegSkill, now: u64) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO reg_skills (name, source, description, synced_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(name) DO UPDATE SET source=?2, description=?3, synced_at=?4, updated_at=?5",
                rusqlite::params![sk.name, sk.source, sk.description, sk.synced_at.map(|v| v as i64), now as i64],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn skill_delete(&self, name: &str) -> Result<bool, String> {
        self.conn
            .execute("DELETE FROM reg_skills WHERE name = ?1", [name])
            .map(|n| n > 0)
            .map_err(|e| e.to_string())
    }

    pub fn mcp_list(&self) -> Result<Vec<RegMcp>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, def FROM reg_mcp ORDER BY name")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok(RegMcp { name: r.get(0)?, def: r.get(1)? }))
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    pub fn mcp_save(&self, m: &RegMcp, now: u64) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO reg_mcp (name, def, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(name) DO UPDATE SET def=?2, updated_at=?3",
                rusqlite::params![m.name, m.def, now as i64],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn mcp_delete(&self, name: &str) -> Result<bool, String> {
        self.conn
            .execute("DELETE FROM reg_mcp WHERE name = ?1", [name])
            .map(|n| n > 0)
            .map_err(|e| e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_crud_roundtrip() {
        let store = Store::open_memory().unwrap();
        store.reg_seed(100).unwrap();
        let seeded = store.reg_list().unwrap();
        assert_eq!(
            seeded.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["kiro", "codex", "claude", "grok", "omp", "kimi"],
            "the six backend defaults are the fixed leading group"
        );
        assert!(seeded.iter().all(|a| a.skills == r#"["tmm-cli","mem","mcp-cli"]"#));
        let expected_kiro_system = concat!(
            "You are a powerful 10x developer running on Kiro CLI who can handle any task with decisive execution and minimal words.",
            "\n\nFor code changes, use a dedicated Git worktree instead of the launch checkout. Preserve the user's configured Git author and add this trailer to every commit you author:\n",
            "Co-authored-by: Kiro Agent <244629292+kiro-agent@users.noreply.github.com>"
        );
        assert_eq!(seeded[0].system, expected_kiro_system, "the default Kiro persona stays concise");
        assert_eq!(seeded[0].mcp, "[]", "Kiro uses its built-in web search");
        assert_eq!(seeded[4].mcp, "[]", "OMP ships its own web_search tool");
        assert!(seeded[1..4].iter().all(|a| a.mcp.contains("kiro-web-search")));
        assert!(seeded[5].mcp.contains("kiro-web-search"), "Bedrock K3 has no built-in search");
        assert_eq!(seeded[5].model, "", "kimi's model is the user's own default_model");
        assert!(!seeded.iter().any(|a| matches!(a.name.as_str(), "docs" | "reviewer")));

        // Seeding twice must not duplicate.
        store.reg_seed(200).unwrap();
        assert_eq!(store.reg_list().unwrap().len(), 6);

        // A custom definition alphabetically before the defaults stays AFTER
        // their fixed group; the rest of the list is alphabetical.
        let custom = RegAgent {
            name: "aaa-custom".into(),
            backend: "kiro".into(),
            model: String::new(),
            effort: String::new(),
            input_mode: "queue".into(),
            system: "custom".into(),
            skills: "[]".into(),
            mcp: "[]".into(),
        };
        store.reg_save(&custom, 250).unwrap();
        assert_eq!(
            store.reg_list().unwrap().iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["kiro", "codex", "claude", "grok", "omp", "kimi", "aaa-custom"]
        );

        // Upsert edits in place.
        let mut kiro = store.reg_get("kiro").unwrap().unwrap();
        kiro.model = "gpt-5.6-sol".into();
        store.reg_save(&kiro, 300).unwrap();
        assert_eq!(store.reg_get("kiro").unwrap().unwrap().model, "gpt-5.6-sol");
        assert_eq!(store.reg_list().unwrap().len(), 7, "save by name is an upsert");

        assert!(store.reg_delete("aaa-custom").unwrap());
        assert!(!store.reg_delete("aaa-custom").unwrap(), "second delete is a no-op");
        assert_eq!(store.reg_list().unwrap().len(), 6);
    }

    #[test]
    fn registry_seed_upgrades_only_known_default_kiro_systems() {
        let store = Store::open_memory().unwrap();
        let legacy = "You are a powerful 10x developer running on Kiro CLI who can handle any task with decisive execution and minimal words.";
        let mut kiro = RegAgent {
            name: "kiro".into(),
            backend: "kiro".into(),
            model: "gpt-5.6-sol".into(),
            effort: "high".into(),
            input_mode: "queue".into(),
            system: legacy.into(),
            skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
            mcp: "[]".into(),
        };
        store.reg_save(&kiro, 100).unwrap();

        store.reg_seed(200).unwrap();
        let upgraded = store.reg_get("kiro").unwrap().unwrap();
        assert!(upgraded.system.contains("dedicated Git worktree"));
        assert!(upgraded
            .system
            .contains("Co-authored-by: Kiro Agent <244629292+kiro-agent@users.noreply.github.com>"));
        assert_eq!(upgraded.model, "gpt-5.6-sol", "a prompt upgrade preserves the chosen model");
        assert_eq!(upgraded.effort, "high", "a prompt upgrade preserves effort");
        assert_eq!(upgraded.skills, kiro.skills, "a prompt upgrade preserves skills");
        assert_eq!(upgraded.mcp, kiro.mcp, "a prompt upgrade preserves MCP");

        kiro.system = VERBOSE_DEFAULT_KIRO_SYSTEM.into();
        store.reg_save(&kiro, 300).unwrap();
        store.reg_seed(400).unwrap();
        assert_eq!(
            store.reg_get("kiro").unwrap().unwrap().system,
            DEFAULT_KIRO_SYSTEM,
            "the first verbose worktree prompt also upgrades to the concise one"
        );

        kiro.system = "My deliberately customized Kiro persona.".into();
        store.reg_save(&kiro, 500).unwrap();
        store.reg_seed(600).unwrap();
        assert_eq!(
            store.reg_get("kiro").unwrap().unwrap().system,
            "My deliberately customized Kiro persona.",
            "seeding never overwrites a custom system prompt"
        );
    }
}
