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

mod activity;
mod board;
mod registry;
mod rooms;
mod schema;
pub use registry::{RegSkill, RegMcp, RegAgent, RegTeam};
pub use activity::ActivityRow;
pub use rooms::HubMsg;

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
    /// The bus room this project's chat lives in, recorded once so a rename of
    /// the session cannot orphan the conversation. `proj:<first session>` for
    /// everything that existed before schema v8.
    #[serde(default)]
    pub room: String,
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
        store.heal()?;
        store
            .conn
            .execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| format!("pragma: {e}"))?;
        Ok(store)
    }

    /// Tables that must simply EXIST, created idempotently on every open.
    ///
    /// A version step is the right home for a schema change with data to carry
    /// forward; `deliveries` has none — it holds only lines still waiting for
    /// their echo, seconds to minutes old. What it does need is to be there even
    /// when `user_version` LIES about it, which is not hypothetical: a binary
    /// built from a tree where the version bump had landed and its migration
    /// block had not stamps the database at the new version without the table,
    /// and every later build then skips the step for ever (measured on this dev
    /// host, 2026-08-29 — the watcher rebuilt in the seconds between the two
    /// edits). The v13 step below still creates it for a database coming from
    /// v12; this is the floor under both.
    fn heal(&mut self) -> Result<(), String> {
        // Additive Board state gets the same partial-build protection as
        // deliveries below: never trust a version stamp more than the schema.
        self.ensure_issue_edit_lock()?;
        self.ensure_issue_numbering()?;
        self.conn
            .execute_batch(
                "CREATE TABLE IF NOT EXISTS deliveries (
                   id      INTEGER PRIMARY KEY AUTOINCREMENT,
                   session TEXT NOT NULL,
                   window  INTEGER NOT NULL,
                   line    TEXT NOT NULL,
                   ts      INTEGER NOT NULL,
                   UNIQUE (session, window, line)
                 );
                 CREATE INDEX IF NOT EXISTS deliveries_session ON deliveries(session, window);
                 CREATE TABLE IF NOT EXISTS hub_msgs (
                   seq     INTEGER PRIMARY KEY AUTOINCREMENT,
                   id      TEXT NOT NULL UNIQUE,
                   ts      INTEGER NOT NULL,
                   room    TEXT NOT NULL,
                   sender  TEXT NOT NULL,
                   to_json TEXT NOT NULL DEFAULT '[]',
                   kind    TEXT NOT NULL DEFAULT 'msg',
                   body    TEXT NOT NULL
                 );
                 CREATE INDEX IF NOT EXISTS hub_msgs_room_seq ON hub_msgs(room, seq);
                 CREATE TABLE IF NOT EXISTS meta (
                   key   TEXT PRIMARY KEY,
                   value TEXT NOT NULL
                 );",
            )
            .map_err(|e| format!("heal deliveries: {e}"))?;
        // Board #120: a binary built between the v20 stamp and its migration
        // block must still get the name columns (the v13 lesson, same floor).
        self.ensure_activity_names()?;
        self.ensure_delivery_names()?;
        self.ensure_delivery_duplicates()
    }

    // ---- archived messages ----------------------------------------------

    // ---- hub messages (board #107) ---------------------------------------
    //
    // The project room's transcript. This is the store `server/hub_rpc.rs`
    // reads and writes directly — the agora bus that used to hold it was
    // deleted whole with the Team system (board #100). `seq` is the paging
    // cursor: stable, gapless within a room, already on every message a
    // client holds (a millisecond timestamp is not — two messages can share
    // one). Nothing is ever pruned; hiding is `msg_archive`'s job, deletion
    // is explicit.

    // ---- activity log ---------------------------------------------------

    // ---- outstanding deliveries -----------------------------------------

    // ---- projects -------------------------------------------------------

    pub fn insert_project(&self, p: &Project) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO projects
                   (id, name, path, icon, session, adopted, autostart, created_at, archived_at, room)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, NULL, ?9)",
                params![
                    p.id,
                    p.name,
                    p.path,
                    p.icon,
                    p.session,
                    p.adopted as i64,
                    p.autostart as i64,
                    p.created_at as i64,
                    if p.room.is_empty() { format!("proj:{}", p.session) } else { p.room.clone() },
                ],
            )
            .map(|_| ())
            .map_err(|e| format!("insert project: {e}"))
    }

    pub fn list_projects(&self, include_archived: bool) -> Result<Vec<Project>, String> {
        let sql = if include_archived {
            "SELECT id, name, path, icon, session, adopted, autostart, created_at,
                    last_up_at, last_seen_at, archived_at, room
               FROM projects ORDER BY COALESCE(last_seen_at, created_at) DESC"
        } else {
            "SELECT id, name, path, icon, session, adopted, autostart, created_at,
                    last_up_at, last_seen_at, archived_at, room
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
                        last_up_at, last_seen_at, archived_at, room
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
                        last_up_at, last_seen_at, archived_at, room
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
                // prev_session is a live alias for already-running agents.
                // Reusing it for another project would make one name resolve
                // to two owners and is exactly the cross-project ambiguity
                // Board #41 forbids.
                "SELECT 1 FROM projects
                  WHERE (session = ?1 OR prev_session = ?1) AND id <> ?2
                  LIMIT 1",
                params![session, id],
                |_| Ok(()),
            )
            .optional()
            .map(|hit| hit.is_some())
            .map_err(|e| format!("check session: {e}"))
    }

    /// Rename a project. `false` when no such row — a rename of nothing is a
    /// caller error, not a silent no-op. Only the LABEL moves: `session` is the
    /// project's identity (and the chat room's key), so it stays put.
    pub fn set_name(&self, id: &str, name: &str) -> Result<bool, String> {
        self.conn
            .execute("UPDATE projects SET name = ?2 WHERE id = ?1", params![id, name])
            .map(|n| n > 0)
            .map_err(|e| format!("rename project: {e}"))
    }

    /// Move a project onto a different tmux session name, remembering the old
    /// one. `prev_session` is what keeps an ALREADY RUNNING agent working: its
    /// `TMM_PROJECT` env var holds the name the session had when it started, and
    /// a process cannot be told otherwise.
    pub fn set_session(&mut self, id: &str, session: &str, prev: &str) -> Result<bool, String> {
        let tx = self.conn.transaction().map_err(|e| format!("rename session transaction: {e}"))?;
        let n = tx
            .execute(
                "UPDATE projects SET session = ?2, prev_session = ?3 WHERE id = ?1",
                params![id, session, prev],
            )
            .map_err(|e| format!("rename session: {e}"))?;
        if n > 0 {
            // The Board is project-scoped by session. Move its rows in the
            // SAME transaction as the declaration, or a rename makes the
            // board disappear under the old key and a later project reusing
            // that name inherits it (board #41). issue_notes follow by the
            // globally unique issue_id and need no rewrite.
            if session != prev {
                // The local-number allocator is Board state too. Move even an
                // empty board's sequence so deleting its latest card and then
                // renaming cannot make the next number restart/reuse. A stale
                // orphan at the free destination name is safe to discard.
                tx.execute("DELETE FROM issue_sequences WHERE session = ?1", params![session])
                    .map_err(|e| format!("clear destination Board sequence: {e}"))?;
                tx.execute(
                    "UPDATE issue_sequences SET session = ?1 WHERE session = ?2",
                    params![session, prev],
                )
                .map_err(|e| format!("rename Board sequence: {e}"))?;
            }
            tx.execute(
                "UPDATE issues SET session = ?1 WHERE session = ?2",
                params![session, prev],
            )
            .map_err(|e| format!("rename board session: {e}"))?;
        }
        tx.commit().map_err(|e| format!("commit session rename: {e}"))?;
        Ok(n > 0)
    }

    /// A project by the session name it used to have. Only the most recent
    /// previous name is kept: two renames in a row leave the oldest one
    /// unresolvable, which costs a restarted agent nothing and keeps this to one
    /// column instead of a table.
    pub fn project_by_prev_session(&self, session: &str) -> Result<Option<Project>, String> {
        self.conn
            .query_row(
                "SELECT id, name, path, icon, session, adopted, autostart, created_at,
                        last_up_at, last_seen_at, archived_at, room
                   FROM projects WHERE prev_session = ?1",
                params![session],
                row_to_project,
            )
            .optional()
            .map_err(|e| format!("get project by previous session: {e}"))
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
    /// Forget a project entirely: its Board task state, then the row plus its
    /// slots (FK cascade). Archive hides a project and is reversible; this is
    /// the "I am done with it" verb, so the caller is responsible for tearing
    /// the session down first.
    pub fn delete_project(&mut self, id: &str) -> Result<bool, String> {
        let tx = self.conn.transaction().map_err(|e| format!("delete project transaction: {e}"))?;
        let session: Option<String> = tx
            .query_row("SELECT session FROM projects WHERE id = ?1", params![id], |r| r.get(0))
            .optional()
            .map_err(|e| format!("find project to delete: {e}"))?;
        let Some(session) = session else {
            tx.commit().map_err(|e| format!("commit missing project delete: {e}"))?;
            return Ok(false);
        };
        // Archive is reversible and keeps the board. Permanent delete means
        // the project stops existing: remove its task state before releasing
        // the session name, or a new project with that name would inherit the
        // old issues (board #41). issue_notes cascade from issues at runtime.
        tx.execute("DELETE FROM issues WHERE session = ?1", params![session])
            .map_err(|e| format!("delete project board: {e}"))?;
        tx.execute("DELETE FROM issue_sequences WHERE session = ?1", params![session])
            .map_err(|e| format!("delete project Board sequence: {e}"))?;
        let n = tx
            .execute("DELETE FROM projects WHERE id = ?1", params![id])
            .map_err(|e| e.to_string())?;
        tx.commit().map_err(|e| format!("commit project delete: {e}"))?;
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

    // ---- agent teams (board #74) -----------------------------------------

    // ---- central skills / MCP assets (agents-v2, state.db v6) ----------

    // ---- the project task board (issues) ----------------------------------

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
        room: r.get::<_, Option<String>>(11)?.unwrap_or_default(),
    })
}

#[cfg(test)]
pub(super) mod test_support {
    //! Row builders the family test modules share (board #147).
    use super::*;

    pub(super) fn project(id: &str) -> Project {
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
            room: String::new(),
        }
    }

    pub(super) fn issue_row_id(store: &Store, session: &str, number: i64) -> i64 {
        store.conn.query_row(
            "SELECT id FROM issues WHERE session = ?1 AND project_number = ?2",
            rusqlite::params![session, number],
            |r| r.get(0),
        ).unwrap()
    }

    pub(super) fn slot(name: &str, ord: i64) -> Slot {
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
}

#[cfg(test)]
mod tests {
    use super::test_support::*;
    use super::*;

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
    fn deleting_a_project_takes_its_slots_and_board() {
        let mut store = Store::open_memory().unwrap();
        store.insert_project(&project("alpha")).unwrap();
        store.replace_slots("alpha", &[slot("shell", 0)]).unwrap();
        let issue = store
            .issue_save("alpha", None, Some("private task"), None, None, None, "human", 10)
            .unwrap();
        store.issue_note("alpha", issue, "human", "private note", 11).unwrap();
        let row_id = issue_row_id(&store, "alpha", issue);

        assert!(store.delete_project("alpha").unwrap());
        assert!(store.slots("alpha").unwrap().is_empty(), "slots cascade with the project");
        assert!(store.issues_list("alpha").unwrap().is_empty(), "the released session name keeps no Board rows");
        let notes: i64 = store
            .conn
            .query_row("SELECT COUNT(*) FROM issue_notes WHERE issue_id = ?1", [row_id], |r| r.get(0))
            .unwrap();
        assert_eq!(notes, 0, "issue notes cascade with the permanently deleted board");
        store.insert_project(&project("alpha")).unwrap();
        assert_eq!(
            store.issue_save("alpha", None, Some("fresh project"), None, None, None, "human", 12).unwrap(),
            1,
            "permanent project deletion releases its numbering sequence too",
        );
    }

    #[test]
    fn renaming_a_project_moves_its_board_and_closes_the_old_key() {
        let mut store = Store::open_memory().unwrap();
        store.insert_project(&project("alpha")).unwrap();
        let issue = store
            .issue_save("alpha", None, Some("rename-safe"), Some("body"), None, None, "human", 10)
            .unwrap();
        store.issue_note("alpha", issue, "human", "kept with it", 11).unwrap();
        let removed = store.issue_save("alpha", None, Some("seen #2"), None, None, None, "human", 11).unwrap();
        assert_eq!(removed, 2);
        assert!(store.issue_delete("alpha", removed).unwrap());

        assert!(store.set_session("alpha", "beta", "alpha").unwrap());
        assert!(store.issues_list("alpha").unwrap().is_empty(), "the old session key is closed");
        let moved = store.issue_get("beta", issue).unwrap().expect("the board follows the project");
        assert_eq!(moved["title"], "rename-safe");
        assert_eq!(moved["notes"].as_array().unwrap().len(), 1, "notes follow their hidden globally unique row id");
        assert_eq!(
            store.issue_save("beta", None, Some("after rename"), None, None, None, "human", 12).unwrap(),
            3,
            "rename carries the per-project sequence, including deleted high numbers",
        );
        // The old name cannot be used as a cross-project write handle after
        // the rename; every CRUD path still requires session + id.
        assert!(store.issue_save("alpha", Some(issue), None, None, Some("done"), None, "x", 12).is_err());
        assert!(store.issue_note("alpha", issue, "x", "sneak", 12).is_err());
        assert!(!store.issue_delete("alpha", issue).unwrap());
        assert!(store.issue_get("beta", issue).unwrap().is_some());
    }

    #[test]
    fn session_conflicts_are_detectable() {
        let mut store = Store::open_memory().unwrap();
        store.insert_project(&project("alpha")).unwrap();
        assert!(store.session_taken_by_other("alpha", "beta").unwrap());
        assert!(!store.session_taken_by_other("alpha", "alpha").unwrap());
        assert!(!store.session_taken_by_other("nope", "beta").unwrap());
        assert_eq!(store.project_by_session("alpha").unwrap().unwrap().id, "alpha");
        assert!(store.project_by_session("nope").unwrap().is_none());

        store.set_session("alpha", "beta", "alpha").unwrap();
        assert!(store.session_taken_by_other("alpha", "new-project").unwrap(), "the previous name remains a reserved live alias");
        assert!(!store.session_taken_by_other("alpha", "alpha").unwrap(), "the alias still belongs to its own project");
    }

}
