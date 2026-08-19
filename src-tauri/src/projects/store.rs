//! Persistence for declarative projects.
//!
//! `state.db` is ours and holds only what the machine observed: which projects
//! exist, which windows they are made of, and the topology history. Anything a
//! human writes by hand (agent definitions with their skills) stays in files —
//! see `docs/exec-plans/projects-and-tasks.md` §5.
//!
//! Deliberately a separate database from `team.db`: that one is the vendored
//! `agora` bus schema and we do not mix tables into it.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Bumped when the schema changes; `migrate` is the only place that knows the
/// steps. Stored in SQLite's own `user_version` pragma.
const SCHEMA_VERSION: i64 = 7;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: String,
    pub name: String,
    /// Canonical workspace directory.
    pub path: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// tmux session name this project projects onto.
    pub session: String,
    /// Adopted from a session the user had already created, so its name is the
    /// user's and we never rename it.
    pub adopted: bool,
    pub autostart: bool,
    pub created_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_up_at: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_seen_at: Option<u64>,
    pub archived: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SlotKind {
    Shell,
    Agent,
}

impl SlotKind {
    pub fn as_str(self) -> &'static str {
        match self {
            SlotKind::Shell => "shell",
            SlotKind::Agent => "agent",
        }
    }
    pub fn parse(s: &str) -> Self {
        match s {
            "agent" => SlotKind::Agent,
            _ => SlotKind::Shell,
        }
    }
}

/// One window's intent. `cwd` is relative to the project path (empty = the
/// project root) so a moved workspace keeps working.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Slot {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub id: Option<i64>,
    pub ord: i64,
    pub window_name: String,
    pub cwd: String,
    pub kind: SlotKind,
    /// The command that owns the window. For an agent slot this is its launch
    /// line and `up` re-runs it; for a shell slot it is what we observed and it
    /// is NOT replayed (see decision 5 in the exec plan).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub command: Option<String>,
    pub auto_run: bool,
    /// The agent's OWN conversation id, as reported by its lifecycle hooks.
    /// This is what lets a restored window resume where it left off instead of
    /// opening a blank prompt. Sticky: once learned it is kept until the window
    /// reports a different one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
    pub first_seen_at: u64,
    /// Set once the window has existed long enough to be worth restoring.
    /// Unsettled slots are remembered but never recreated by `up`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub settled_at: Option<u64>,
}

impl Slot {
    pub fn is_settled(&self) -> bool {
        self.settled_at.is_some()
    }
}

pub struct Store {
    conn: Connection,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        let conn = Connection::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
        Self::init(conn)
    }

    #[cfg(test)]
    pub fn open_memory() -> Result<Self, String> {
        Self::init(Connection::open_in_memory().map_err(|e| e.to_string())?)
    }

    fn init(conn: Connection) -> Result<Self, String> {
        conn.execute_batch("PRAGMA journal_mode=WAL;")
            .map_err(|e| format!("pragma: {e}"))?;
        let mut store = Self { conn };
        // Migrations MUST run with foreign keys off: a schema change rebuilds
        // `projects`, and with enforcement on SQLite performs an implicit
        // DELETE FROM before the DROP, which cascades every slot and snapshot
        // away. Off has to be explicit — libsqlite3-sys builds its bundled
        // SQLite with SQLITE_DEFAULT_FOREIGN_KEYS=1, so the connection default
        // is ON, not the SQLite upstream default of OFF.
        store
            .conn
            .execute_batch("PRAGMA foreign_keys=OFF;")
            .map_err(|e| format!("pragma: {e}"))?;
        store.migrate()?;
        store
            .conn
            .execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| format!("pragma: {e}"))?;
        Ok(store)
    }

    fn migrate(&mut self) -> Result<(), String> {
        let version: i64 = self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .map_err(|e| format!("read user_version: {e}"))?;
        if version >= SCHEMA_VERSION {
            return Ok(());
        }
        if version < 1 {
            self.conn
                .execute_batch(
                    "CREATE TABLE projects (
                       id           TEXT PRIMARY KEY,
                       name         TEXT NOT NULL,
                       path         TEXT NOT NULL UNIQUE,
                       icon         TEXT,
                       session      TEXT NOT NULL,
                       adopted      INTEGER NOT NULL DEFAULT 0,
                       autostart    INTEGER NOT NULL DEFAULT 0,
                       created_at   INTEGER NOT NULL,
                       last_up_at   INTEGER,
                       last_seen_at INTEGER,
                       archived_at  INTEGER
                     );
                     CREATE TABLE slots (
                       id            INTEGER PRIMARY KEY,
                       project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                       ord           INTEGER NOT NULL,
                       window_name   TEXT NOT NULL,
                       cwd           TEXT NOT NULL DEFAULT '',
                       kind          TEXT NOT NULL,
                       command       TEXT,
                       auto_run      INTEGER NOT NULL DEFAULT 0,
                       first_seen_at INTEGER NOT NULL,
                       settled_at    INTEGER,
                       UNIQUE (project_id, window_name)
                     );
                     CREATE TABLE snapshots (
                       id            INTEGER PRIMARY KEY,
                       project_id    TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
                       at            INTEGER NOT NULL,
                       topology_json TEXT NOT NULL
                     );
                     CREATE INDEX snapshots_project ON snapshots(project_id, at DESC);",
                )
                .map_err(|e| format!("migrate to 1: {e}"))?;
        }
        if version < 2 {
            // v1 made `path` unique, which was wrong: two sessions may
            // legitimately sit in the same directory (several of them parked in
            // $HOME is the normal case), and adopting the second one failed with
            // "already project X". A project's identity is its SESSION — that is
            // the thing it projects onto, and two projects fighting over one
            // session name is the real conflict. Paths are merely indexed.
            self.conn
                .execute_batch(
                    "CREATE TABLE projects_v2 (
                       id           TEXT PRIMARY KEY,
                       name         TEXT NOT NULL,
                       path         TEXT NOT NULL,
                       icon         TEXT,
                       session      TEXT NOT NULL UNIQUE,
                       adopted      INTEGER NOT NULL DEFAULT 0,
                       autostart    INTEGER NOT NULL DEFAULT 0,
                       created_at   INTEGER NOT NULL,
                       last_up_at   INTEGER,
                       last_seen_at INTEGER,
                       archived_at  INTEGER
                     );
                     INSERT INTO projects_v2
                       SELECT id, name, path, icon, session, adopted, autostart,
                              created_at, last_up_at, last_seen_at, archived_at
                         FROM projects;
                     DROP TABLE projects;
                     ALTER TABLE projects_v2 RENAME TO projects;
                     CREATE INDEX projects_path ON projects(path);",
                )
                .map_err(|e| format!("migrate to 2: {e}"))?;
        }
        if version < 3 {
            // Remember which conversation each agent window was in, so `up`
            // resumes it instead of starting over.
            self.conn
                .execute_batch("ALTER TABLE slots ADD COLUMN agent_session_id TEXT;")
                .map_err(|e| format!("migrate to 3: {e}"))?;
        }
        if version < 4 {
            // Topology snapshots are gone. The declaration IS the last observed
            // state — closing a project does not touch it and a restart reads it
            // back — so a 20-deep history answered a question nobody had, while
            // `restore` could not even deliver: it rewrote the declaration
            // without projecting it, and on a live project the next capture tick
            // threw that away because live tmux is the truth. Two days of real
            // use produced exactly one snapshot per project: the one written at
            // adopt, identical to the current declaration.
            self.conn
                .execute_batch("DROP TABLE IF EXISTS snapshots;")
                .map_err(|e| format!("migrate to 4: {e}"))?;
        }
        if version < 5 {
            // Agents-v2: the agent registry. One centrally-defined agent =
            // backend + persona + skills + MCP servers + hire permission,
            // materialized into an ISOLATED per-agent home at spawn time so the
            // user's global CLI config never interferes. Skills are string refs
            // (local name or github url — resolved by the shared skills
            // resolver); MCP servers are embedded JSON defs. Both live in the
            // agent row: composition is by value here, the assets themselves
            // are external (skill dirs / the MCP servers they point at).
            self.conn
                .execute_batch(
                    "CREATE TABLE reg_agents (
                       name       TEXT PRIMARY KEY,
                       backend    TEXT NOT NULL,
                       model      TEXT NOT NULL DEFAULT '',
                       system     TEXT NOT NULL DEFAULT '',
                       skills     TEXT NOT NULL DEFAULT '[]',
                       mcp        TEXT NOT NULL DEFAULT '[]',
                       can_hire   INTEGER NOT NULL DEFAULT 0,
                       created_at INTEGER NOT NULL,
                       updated_at INTEGER NOT NULL
                     );",
                )
                .map_err(|e| format!("migrate to 5: {e}"))?;
        }
        if version < 6 {
            // Skills and MCP servers become first-class central assets
            // (owner: "集中化管理"): define once, reference from any agent by
            // NAME. A skill entry maps a name to a resolvable ref (local dir
            // or github url); an mcp entry holds the server def JSON. Agent
            // defs keep their existing columns — string entries in them now
            // resolve through these tables at spawn, inline values still work.
            self.conn
                .execute_batch(
                    "CREATE TABLE reg_skills (
                       name        TEXT PRIMARY KEY,
                       ref_        TEXT NOT NULL,
                       description TEXT NOT NULL DEFAULT '',
                       updated_at  INTEGER NOT NULL
                     );
                     CREATE TABLE reg_mcp (
                       name       TEXT PRIMARY KEY,
                       def        TEXT NOT NULL,
                       updated_at INTEGER NOT NULL
                     );",
                )
                .map_err(|e| format!("migrate to 6: {e}"))?;
        }
        if version < 7 {
            // Skills become APP-OWNED (owner: "存到你管理的目录里"): the files
            // live in <state dir>/skills/<name>/, and the row records the
            // SOURCE they were imported from (local path or git url) plus
            // when it was last synced — the source is sync metadata, not the
            // thing agents load. ref_ carried the same string; rename + add
            // the timestamp via rebuild.
            self.conn
                .execute_batch(
                    "PRAGMA foreign_keys=OFF;
                     CREATE TABLE reg_skills_v7 (
                       name        TEXT PRIMARY KEY,
                       source      TEXT NOT NULL,
                       description TEXT NOT NULL DEFAULT '',
                       synced_at   INTEGER,
                       updated_at  INTEGER NOT NULL
                     );
                     INSERT INTO reg_skills_v7 (name, source, description, synced_at, updated_at)
                       SELECT name, ref_, description, NULL, updated_at FROM reg_skills;
                     DROP TABLE reg_skills;
                     ALTER TABLE reg_skills_v7 RENAME TO reg_skills;
                     PRAGMA foreign_keys=ON;",
                )
                .map_err(|e| format!("migrate to 7: {e}"))?;
        }
        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| format!("set user_version: {e}"))
    }

    // ---- projects -------------------------------------------------------

    pub fn insert_project(&self, p: &Project) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO projects
                   (id, name, path, icon, session, adopted, autostart, created_at, archived_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL)",
                params![
                    p.id,
                    p.name,
                    p.path,
                    p.icon,
                    p.session,
                    p.adopted as i64,
                    p.autostart as i64,
                    p.created_at as i64,
                ],
            )
            .map(|_| ())
            .map_err(|e| format!("insert project: {e}"))
    }

    pub fn list_projects(&self, include_archived: bool) -> Result<Vec<Project>, String> {
        let sql = if include_archived {
            "SELECT id, name, path, icon, session, adopted, autostart, created_at,
                    last_up_at, last_seen_at, archived_at
               FROM projects ORDER BY COALESCE(last_seen_at, created_at) DESC"
        } else {
            "SELECT id, name, path, icon, session, adopted, autostart, created_at,
                    last_up_at, last_seen_at, archived_at
               FROM projects WHERE archived_at IS NULL
              ORDER BY COALESCE(last_seen_at, created_at) DESC"
        };
        let mut stmt = self.conn.prepare(sql).map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], row_to_project)
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("list projects: {e}"))
    }

    pub fn project(&self, id: &str) -> Result<Option<Project>, String> {
        self.conn
            .query_row(
                "SELECT id, name, path, icon, session, adopted, autostart, created_at,
                        last_up_at, last_seen_at, archived_at
                   FROM projects WHERE id = ?1",
                params![id],
                row_to_project,
            )
            .optional()
            .map_err(|e| format!("get project: {e}"))
    }

    /// A project's identity is the tmux session it projects onto.
    pub fn project_by_session(&self, session: &str) -> Result<Option<Project>, String> {
        self.conn
            .query_row(
                "SELECT id, name, path, icon, session, adopted, autostart, created_at,
                        last_up_at, last_seen_at, archived_at
                   FROM projects WHERE session = ?1",
                params![session],
                row_to_project,
            )
            .optional()
            .map_err(|e| format!("get project by session: {e}"))
    }

    pub fn session_taken_by_other(&self, session: &str, id: &str) -> Result<bool, String> {
        self.conn
            .query_row(
                "SELECT 1 FROM projects WHERE session = ?1 AND id <> ?2 LIMIT 1",
                params![session, id],
                |_| Ok(()),
            )
            .optional()
            .map(|hit| hit.is_some())
            .map_err(|e| format!("check session: {e}"))
    }

    pub fn set_archived(&self, id: &str, archived: bool, now: u64) -> Result<(), String> {
        let at = if archived { Some(now as i64) } else { None };
        self.conn
            .execute("UPDATE projects SET archived_at = ?2 WHERE id = ?1", params![id, at])
            .map(|_| ())
            .map_err(|e| format!("archive project: {e}"))
    }

    pub fn set_autostart(&self, id: &str, autostart: bool) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE projects SET autostart = ?2 WHERE id = ?1",
                params![id, autostart as i64],
            )
            .map(|_| ())
            .map_err(|e| format!("set autostart: {e}"))
    }

    pub fn mark_up(&self, id: &str, now: u64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE projects SET last_up_at = ?2, last_seen_at = ?2 WHERE id = ?1",
                params![id, now as i64],
            )
            .map(|_| ())
            .map_err(|e| format!("mark up: {e}"))
    }

    pub fn mark_seen(&self, id: &str, now: u64) -> Result<(), String> {
        self.conn
            .execute(
                "UPDATE projects SET last_seen_at = ?2 WHERE id = ?1",
                params![id, now as i64],
            )
            .map(|_| ())
            .map_err(|e| format!("mark seen: {e}"))
    }

    // ---- slots ----------------------------------------------------------

    pub fn slots(&self, project_id: &str) -> Result<Vec<Slot>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, ord, window_name, cwd, kind, command, auto_run,
                        first_seen_at, settled_at, agent_session_id
                   FROM slots WHERE project_id = ?1 ORDER BY ord",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map(params![project_id], |r| {
                Ok(Slot {
                    id: r.get(0)?,
                    ord: r.get(1)?,
                    window_name: r.get(2)?,
                    cwd: r.get(3)?,
                    kind: SlotKind::parse(&r.get::<_, String>(4)?),
                    command: r.get(5)?,
                    auto_run: r.get::<_, i64>(6)? != 0,
                    first_seen_at: r.get::<_, i64>(7)? as u64,
                    settled_at: r.get::<_, Option<i64>>(8)?.map(|v| v as u64),
                    agent_session_id: r.get(9)?,
                })
            })
            .map_err(|e| e.to_string())?;
        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("list slots: {e}"))
    }

    /// Replace a project's whole slot list in one transaction. The declaration
    /// is always written as a set, never patched row by row, so a capture can
    /// never leave a half-applied topology behind.
    /// Forget a project entirely: the row plus its slots (FK cascade). Archive
    /// hides a project and is reversible; this is the "I am done with it" verb,
    /// so the caller is responsible for tearing the session down first.
    pub fn delete_project(&self, id: &str) -> Result<bool, String> {
        let n = self
            .conn
            .execute("DELETE FROM projects WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    /// Drop ONE slot by window name — "this agent is no longer part of the
    /// project", as opposed to `replace_slots`, which is the capture loop
    /// rewriting the whole declaration.
    pub fn delete_slot(&self, project_id: &str, window_name: &str) -> Result<bool, String> {
        let n = self
            .conn
            .execute(
                "DELETE FROM slots WHERE project_id = ?1 AND window_name = ?2",
                params![project_id, window_name],
            )
            .map_err(|e| e.to_string())?;
        Ok(n > 0)
    }

    pub fn replace_slots(&mut self, project_id: &str, slots: &[Slot]) -> Result<(), String> {
        let tx = self.conn.transaction().map_err(|e| e.to_string())?;
        tx.execute("DELETE FROM slots WHERE project_id = ?1", params![project_id])
            .map_err(|e| format!("clear slots: {e}"))?;
        {
            let mut stmt = tx
                .prepare(
                    "INSERT INTO slots
                       (project_id, ord, window_name, cwd, kind, command, auto_run,
                        first_seen_at, settled_at, agent_session_id)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                )
                .map_err(|e| e.to_string())?;
            for (i, s) in slots.iter().enumerate() {
                stmt.execute(params![
                    project_id,
                    i as i64,
                    s.window_name,
                    s.cwd,
                    s.kind.as_str(),
                    s.command,
                    s.auto_run as i64,
                    s.first_seen_at as i64,
                    s.settled_at.map(|v| v as i64),
                    s.agent_session_id,
                ])
                .map_err(|e| format!("insert slot {}: {e}", s.window_name))?;
            }
        }
        tx.commit().map_err(|e| format!("commit slots: {e}"))
    }

    // ---- agent registry (agents-v2) ------------------------------------

    pub fn reg_list(&self) -> Result<Vec<RegAgent>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT name, backend, model, system, skills, mcp, can_hire FROM reg_agents ORDER BY name")
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
                "SELECT name, backend, model, system, skills, mcp, can_hire FROM reg_agents WHERE name = ?1",
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
                "INSERT INTO reg_agents (name, backend, model, system, skills, mcp, can_hire, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8)
                 ON CONFLICT(name) DO UPDATE SET
                   backend=?2, model=?3, system=?4, skills=?5, mcp=?6, can_hire=?7, updated_at=?8",
                rusqlite::params![
                    a.name,
                    a.backend,
                    a.model,
                    a.system,
                    a.skills,
                    a.mcp,
                    a.can_hire as i64,
                    now as i64,
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

    /// Seed the built-in defs once (empty table only) so `+ agent` has
    /// something to offer out of the box. Mirrors the prototype's roster.
    pub fn reg_seed(&self, now: u64) -> Result<(), String> {
        let count: i64 = self
            .conn
            .query_row("SELECT COUNT(*) FROM reg_agents", [], |r| r.get(0))
            .map_err(|e| e.to_string())?;
        if count > 0 {
            return Ok(());
        }
        let seeds = [
            RegAgent {
                name: "lead".into(),
                backend: "kiro".into(),
                model: String::new(),
                system: "You are the project lead. Break the task down, do the core work yourself, and delegate well-scoped pieces to other agents when it genuinely helps. You may spawn agents with `tmm spawn <registry-name> --brief \"...\"` — check `tmm registry list` for who is available. Keep the human informed of decisions, not process.\n\nHow replies reach the room: the end of every turn is captured automatically from the stop hook and posted to the project room. You do not need to repeat a final result with `tmm send` — doing so wastes a call and the dedup filter drops it anyway. Use `tmm send` while a turn is in flight: that is the only way to report progress on a long task before it finishes. `tmm status waiting|blocked` is for being stuck on something outside your control — announcing that you are working is pointless, since turn boundaries are observed from your own hooks. Use `tmm done` to mark completion; its summary can be one line because the full result is already in the room.\n\nAddressing a teammate with @name is a delivery action, not a mention: `tmm send \"@reviewer 请审\"` types that line into the reviewer's pane and interrupts whatever they are doing. Only address someone when the intent is to hand off a task or ask a question that needs a reply. Never put credentials or secrets in a message — room contents are persisted and rendered to mobile clients.".into(),
                skills: "[]".into(),
                mcp: "[]".into(),
                can_hire: true,
            },
            RegAgent {
                name: "reviewer".into(),
                backend: "claude".into(),
                model: String::new(),
                system: "You are a code reviewer. Read the diff or branch you are briefed on, verify the change does what it claims, and report concrete findings (file:line) — no style nitpicks unless they hide bugs. Reply to whoever briefed you.\n\nHow replies reach the room: the end of every turn is captured automatically and posted to the project room. You do not need to repeat your findings with `tmm send`. Use `tmm send` only to address a specific teammate mid-task or to report a blocker; use `tmm done` to mark completion with a one-line summary.".into(),
                skills: "[]".into(),
                mcp: "[]".into(),
                can_hire: false,
            },
            RegAgent {
                name: "docs".into(),
                backend: "codex".into(),
                model: String::new(),
                system: "You are the docs writer. Keep design docs and READMEs in sync with the change you are briefed on. Plain words, specifics over superlatives.\n\nHow replies reach the room: the end of every turn is captured automatically and posted to the project room. You do not need to call `tmm send` to report completion. Use `tmm send` only to address a specific teammate or report a blocker; use `tmm done` to mark completion with a one-line summary.".into(),
                skills: "[]".into(),
                mcp: "[]".into(),
                can_hire: false,
            },
        ];
        for s in &seeds {
            self.reg_save(s, now)?;
        }
        Ok(())
    }


    // ---- central skills / MCP assets (agents-v2, state.db v6) ----------

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
    #[serde(default)]
    pub system: String,
    /// JSON array of skill refs (local names or github URLs).
    #[serde(default = "empty_json_array")]
    pub skills: String,
    /// JSON array of MCP server defs ({name, command/url, args, env, headers}).
    #[serde(default = "empty_json_array")]
    pub mcp: String,
    #[serde(default)]
    pub can_hire: bool,
}

fn empty_json_array() -> String {
    "[]".to_string()
}

fn row_to_reg_agent(r: &rusqlite::Row<'_>) -> rusqlite::Result<RegAgent> {
    Ok(RegAgent {
        name: r.get(0)?,
        backend: r.get(1)?,
        model: r.get(2)?,
        system: r.get(3)?,
        skills: r.get(4)?,
        mcp: r.get(5)?,
        can_hire: r.get::<_, i64>(6)? != 0,
    })
}

fn row_to_project(r: &rusqlite::Row<'_>) -> rusqlite::Result<Project> {
    Ok(Project {
        id: r.get(0)?,
        name: r.get(1)?,
        path: r.get(2)?,
        icon: r.get(3)?,
        session: r.get(4)?,
        adopted: r.get::<_, i64>(5)? != 0,
        autostart: r.get::<_, i64>(6)? != 0,
        created_at: r.get::<_, i64>(7)? as u64,
        last_up_at: r.get::<_, Option<i64>>(8)?.map(|v| v as u64),
        last_seen_at: r.get::<_, Option<i64>>(9)?.map(|v| v as u64),
        archived: r.get::<_, Option<i64>>(10)?.is_some(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(id: &str) -> Project {
        Project {
            id: id.into(),
            name: id.into(),
            path: format!("/tmp/{id}"),
            icon: None,
            session: id.into(),
            adopted: false,
            autostart: false,
            created_at: 100,
            last_up_at: None,
            last_seen_at: None,
            archived: false,
        }
    }

    fn slot(name: &str, ord: i64) -> Slot {
        Slot {
            id: None,
            ord,
            window_name: name.into(),
            cwd: String::new(),
            kind: SlotKind::Shell,
            command: None,
            auto_run: false,
            agent_session_id: None,
            first_seen_at: 100,
            settled_at: Some(200),
        }
    }

    #[test]
    fn projects_round_trip_and_archive_hides_without_deleting() {
        let store = Store::open_memory().unwrap();
        store.insert_project(&project("alpha")).unwrap();
        store.insert_project(&project("beta")).unwrap();
        assert_eq!(store.list_projects(false).unwrap().len(), 2);

        store.set_archived("beta", true, 300).unwrap();
        let visible = store.list_projects(false).unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].id, "alpha");
        let all = store.list_projects(true).unwrap();
        assert_eq!(all.len(), 2);
        assert!(all.iter().find(|p| p.id == "beta").unwrap().archived);

        store.set_archived("beta", false, 400).unwrap();
        assert_eq!(store.list_projects(false).unwrap().len(), 2);
    }

    #[test]
    fn replace_slots_is_a_set_write() {
        let mut store = Store::open_memory().unwrap();
        store.insert_project(&project("alpha")).unwrap();
        store
            .replace_slots("alpha", &[slot("shell", 0), slot("kiro", 1)])
            .unwrap();
        store.replace_slots("alpha", &[slot("kiro", 0)]).unwrap();
        let slots = store.slots("alpha").unwrap();
        assert_eq!(slots.len(), 1);
        assert_eq!(slots[0].window_name, "kiro");
        assert_eq!(slots[0].ord, 0);
    }

    #[test]
    fn deleting_a_project_takes_its_slots() {
        let mut store = Store::open_memory().unwrap();
        store.insert_project(&project("alpha")).unwrap();
        store.replace_slots("alpha", &[slot("shell", 0)]).unwrap();
        store
            .conn
            .execute("DELETE FROM projects WHERE id = 'alpha'", [])
            .unwrap();
        assert!(store.slots("alpha").unwrap().is_empty());
    }

    #[test]
    fn session_conflicts_are_detectable() {
        let store = Store::open_memory().unwrap();
        store.insert_project(&project("alpha")).unwrap();
        assert!(store.session_taken_by_other("alpha", "beta").unwrap());
        assert!(!store.session_taken_by_other("alpha", "alpha").unwrap());
        assert!(!store.session_taken_by_other("nope", "beta").unwrap());
        assert_eq!(store.project_by_session("alpha").unwrap().unwrap().id, "alpha");
        assert!(store.project_by_session("nope").unwrap().is_none());
    }

    /// v1 shipped `path` as UNIQUE, which rejected the second session in a
    /// directory. The migration must lift that constraint, put uniqueness on
    /// `session` instead, and carry the existing rows across — including the
    /// children, which a naive `DROP TABLE projects` would cascade away.
    #[test]
    fn migrating_a_v1_database_keeps_its_rows_and_moves_the_unique_constraint() {
        let dir = std::env::temp_dir().join("tmm-store-migrate");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("state.db");

        let v1 = Connection::open(&path).unwrap();
        v1.execute_batch(
            "CREATE TABLE projects (
               id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL UNIQUE,
               icon TEXT, session TEXT NOT NULL, adopted INTEGER NOT NULL DEFAULT 0,
               autostart INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL,
               last_up_at INTEGER, last_seen_at INTEGER, archived_at INTEGER);
             CREATE TABLE slots (
               id INTEGER PRIMARY KEY,
               project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
               ord INTEGER NOT NULL, window_name TEXT NOT NULL,
               cwd TEXT NOT NULL DEFAULT '', kind TEXT NOT NULL, command TEXT,
               auto_run INTEGER NOT NULL DEFAULT 0, first_seen_at INTEGER NOT NULL,
               settled_at INTEGER, UNIQUE (project_id, window_name));
             CREATE TABLE snapshots (
               id INTEGER PRIMARY KEY,
               project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
               at INTEGER NOT NULL, topology_json TEXT NOT NULL);
             INSERT INTO projects (id, name, path, session, created_at)
               VALUES ('old', 'old', '/w/shared', 'old', 100);
             INSERT INTO slots (project_id, ord, window_name, cwd, kind, first_seen_at, settled_at)
               VALUES ('old', 0, 'editor', '', 'shell', 100, 200);
             INSERT INTO snapshots (project_id, at, topology_json) VALUES ('old', 100, '[]');
             PRAGMA user_version = 1;",
        )
        .unwrap();
        drop(v1);

        let store = Store::open(&path).unwrap();
        assert_eq!(store.list_projects(false).unwrap().len(), 1, "row carried over");
        assert_eq!(store.slots("old").unwrap().len(), 1, "children survived the rebuild");

        let mut second = project("second");
        second.path = "/w/shared".into(); // same directory as `old`
        store
            .insert_project(&second)
            .expect("a second project in the same directory is allowed now");

        let mut clash = project("clash");
        clash.session = "old".into();
        assert!(
            store.insert_project(&clash).is_err(),
            "two projects must not fight over one tmux session"
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn registry_crud_roundtrip() {
        let store = Store::open_memory().unwrap();
        store.reg_seed(100).unwrap();
        let seeded = store.reg_list().unwrap();
        assert_eq!(seeded.len(), 3, "lead/reviewer/docs seeds");
        assert!(seeded.iter().any(|a| a.name == "lead" && a.can_hire));
        // Seeding twice must not duplicate.
        store.reg_seed(200).unwrap();
        assert_eq!(store.reg_list().unwrap().len(), 3);

        // Upsert edits in place.
        let mut lead = store.reg_get("lead").unwrap().unwrap();
        lead.model = "claude-opus-4.6".into();
        store.reg_save(&lead, 300).unwrap();
        assert_eq!(store.reg_get("lead").unwrap().unwrap().model, "claude-opus-4.6");
        assert_eq!(store.reg_list().unwrap().len(), 3, "save by name is an upsert");

        assert!(store.reg_delete("docs").unwrap());
        assert!(!store.reg_delete("docs").unwrap(), "second delete is a no-op");
        assert_eq!(store.reg_list().unwrap().len(), 2);
    }

    #[test]
    fn v4_to_v6_migration_adds_registry_and_assets() {
        // An existing v4 db (projects+slots only) must gain reg_agents.
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE projects (id TEXT PRIMARY KEY, name TEXT NOT NULL, path TEXT NOT NULL,
               icon TEXT, session TEXT NOT NULL UNIQUE, adopted INTEGER NOT NULL DEFAULT 0,
               autostart INTEGER NOT NULL DEFAULT 0, created_at INTEGER NOT NULL,
               last_up_at INTEGER, last_seen_at INTEGER, archived_at INTEGER);
             CREATE TABLE slots (id INTEGER PRIMARY KEY,
               project_id TEXT NOT NULL REFERENCES projects(id) ON DELETE CASCADE,
               ord INTEGER NOT NULL, window_name TEXT NOT NULL, cwd TEXT NOT NULL DEFAULT '',
               kind TEXT NOT NULL, command TEXT, auto_run INTEGER NOT NULL DEFAULT 0,
               first_seen_at INTEGER NOT NULL, settled_at INTEGER, agent_session_id TEXT,
               UNIQUE (project_id, window_name));
             PRAGMA user_version = 4;",
        )
        .unwrap();
        let store = Store::init(conn).unwrap();
        store.reg_seed(1).unwrap();
        assert_eq!(store.reg_list().unwrap().len(), 3);
        let v: i64 = store.conn.query_row("PRAGMA user_version", [], |r| r.get(0)).unwrap();
        assert_eq!(v, SCHEMA_VERSION);
        // v6 assets exist and are usable on a migrated db.
        store.skill_save(&RegSkill { name: "s".into(), source: "github.com/x/y".into(), description: String::new(), synced_at: None }, 1).unwrap();
        assert_eq!(store.skills_list().unwrap().len(), 1);
    }
}
