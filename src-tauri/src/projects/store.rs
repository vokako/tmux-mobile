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
const SCHEMA_VERSION: i64 = 13;

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

/// One row of the durable activity log. `id` is the rowid, which doubles as the
/// paging cursor's tiebreak: several events share one millisecond inside a busy
/// turn, so a ts-only cursor would skip or repeat them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActivityRow {
    pub id: i64,
    pub window: usize,
    pub ts: u64,
    pub kind: String,
    pub text: String,
    pub tool: String,
    pub via: String,
    pub state: String,
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
                 CREATE INDEX IF NOT EXISTS deliveries_session ON deliveries(session, window);",
            )
            .map_err(|e| format!("heal deliveries: {e}"))
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
        if version < 8 {
            // Renaming a project renames its tmux SESSION too — the session name
            // is what the Terminal and `tmux ls` show, so leaving it behind made
            // one thing wear two names (owner, 2026-08-19). Two additive columns
            // make that safe, and additive is the point: an ALTER cannot cascade
            // children away the way a table rebuild can.
            //
            //  · `room` decouples the chat from the name. The bus room used to be
            //    derived as `proj:<session>`, so a rename would have orphaned the
            //    conversation; now the room id is recorded once and never moves.
            //  · `prev_session` keeps the OLD name resolvable. A running agent has
            //    `TMM_PROJECT=<session>` baked into its environment, so without
            //    this every `tmm send/status/done` from an already-started agent
            //    would fail until it was restarted.
            self.conn
                .execute_batch(
                    "ALTER TABLE projects ADD COLUMN room TEXT NOT NULL DEFAULT '';
                     ALTER TABLE projects ADD COLUMN prev_session TEXT;
                     UPDATE projects SET room = 'proj:' || session WHERE room = '';",
                )
                .map_err(|e| format!("migrate to 8: {e}"))?;
        }
        if version < 9 {
            // v9: the activity log becomes durable. Tool calls, prompts, receipts
            // and status notes lived in a 120-entry in-memory ring, so a server
            // restart erased every tool lane in the conversation while the
            // messages around them survived — a feed with holes in it (owner,
            // 2026-08-19: "后台的工具调用 status之类的是不是没有持久化，好像重启就
            // 没了"). One flat table, written fail-soft: telemetry may never
            // block the thing it observes.
            self.conn
                .execute_batch(
                    "CREATE TABLE activity (
                       id      INTEGER PRIMARY KEY AUTOINCREMENT,
                       session TEXT NOT NULL,
                       window  INTEGER NOT NULL,
                       ts      INTEGER NOT NULL,
                       kind    TEXT NOT NULL,
                       text    TEXT NOT NULL DEFAULT '',
                       tool    TEXT NOT NULL DEFAULT '',
                       via     TEXT NOT NULL DEFAULT '',
                       state   TEXT NOT NULL DEFAULT ''
                     );
                     CREATE INDEX activity_session_ts ON activity(session, ts);",
                )
                .map_err(|e| format!("migrate to 9: {e}"))?;
        }
        if version < 10 {
            // v10: archived chat messages. Deleting a message is two steps —
            // archive hides it, and deleting it IN the archive forgets it for good
            // (owner, 2026-08-19) — so the middle state needs somewhere to live.
            // It lives HERE, in our own database, because `agora` (team.db) is a
            // faithful copy of an upstream crate and this is our feature, not its.
            //
            // The row carries a SNAPSHOT of the message: the archive view is then
            // self-contained (no join across two databases, no dependence on how
            // far back the room history was fetched), and a restore is just
            // dropping the row — the message itself never left team.db.
            self.conn
                .execute_batch(
                    "CREATE TABLE msg_archive (
                       room        TEXT NOT NULL,
                       msg_id      TEXT NOT NULL,
                       ts          INTEGER NOT NULL,
                       sender      TEXT NOT NULL,
                       body        TEXT NOT NULL,
                       archived_at INTEGER NOT NULL,
                       PRIMARY KEY (room, msg_id)
                     );",
                )
                .map_err(|e| format!("migrate to 10: {e}"))?;
        }
        if version < 11 {
            // v11: reasoning effort on the agent definition (owner, 2026-08-22:
            // "agent配置里应该有thinking effort的配置选项"). A plain column, not
            // a table rebuild, so no foreign-key dance is needed. Empty means
            // the backend's default, same contract as `model`.
            self.conn
                .execute_batch("ALTER TABLE reg_agents ADD COLUMN effort TEXT NOT NULL DEFAULT '';")
                .map_err(|e| format!("migrate to 11: {e}"))?;
        }
        if version < 12 {
            // v12: the project task BOARD (owner, 2026-08-29: "引入一个新的看板
            // 功能…人类有一个看板页面，能写任务issue，agent也可以读任务，修改任务
            // 状态，在看板上记录信息状态"). Keyed by SESSION like the chat room —
            // the board belongs to the project's conversation, not its folder.
            // Status is a fixed four-column vocabulary (todo/doing/review/done);
            // notes are the issue's own thread, separate from the chat.
            self.conn
                .execute_batch(
                    "CREATE TABLE issues (
                       id         INTEGER PRIMARY KEY,
                       session    TEXT NOT NULL,
                       title      TEXT NOT NULL,
                       body       TEXT NOT NULL DEFAULT '',
                       status     TEXT NOT NULL DEFAULT 'todo',
                       assignee   TEXT NOT NULL DEFAULT '',
                       created_by TEXT NOT NULL DEFAULT '',
                       created_at INTEGER NOT NULL,
                       updated_at INTEGER NOT NULL
                     );
                     CREATE INDEX issues_session ON issues(session, status, updated_at DESC);
                     CREATE TABLE issue_notes (
                       id       INTEGER PRIMARY KEY,
                       issue_id INTEGER NOT NULL REFERENCES issues(id) ON DELETE CASCADE,
                       author   TEXT NOT NULL DEFAULT '',
                       body     TEXT NOT NULL,
                       at       INTEGER NOT NULL
                     );
                     CREATE INDEX issue_notes_issue ON issue_notes(issue_id, at);",
                )
                .map_err(|e| format!("migrate to 12: {e}"))?;
        }
        if version < 13 {
            // v13: outstanding DELIVERIES become durable. A line this app typed
            // into an agent's pane waits for the agent's `userPromptSubmit` echo
            // to confirm it, and that queue lived only in this process's memory —
            // so a server restart between the typing and the echo lost the
            // receipt: the hook could no longer attribute the prompt to us, the
            // event came back `via: local` (rendered as a prompt the human typed
            // at the keyboard) and the message it belonged to kept its hollow
            // ring for ever (owner, 2026-08-29: "发送了一条消息，然后后端的服务
            // 有重启了，然后agent又收到指令确认hooks，这个hooks没有正确把之前的
            // 未确认的消息变成已读状态，被单独写出来了"). An agent survives our
            // restart — it is a separate process, holding our line in its own
            // input queue — so the record of what we typed has to survive it too.
            //
            // One row per outstanding line, keyed by the line itself: re-typing
            // the same text replaces its entry rather than queueing a duplicate
            // that could never be acked twice, exactly like the in-memory queue.
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
                     CREATE INDEX IF NOT EXISTS deliveries_session ON deliveries(session, window);",
                )
                .map_err(|e| format!("migrate to 13: {e}"))?;
        }
        self.conn
            .pragma_update(None, "user_version", SCHEMA_VERSION)
            .map_err(|e| format!("set user_version: {e}"))
    }

    // ---- archived messages ----------------------------------------------

    /// Hide a message: it stays in the room's own store, we stop showing it.
    /// Idempotent, so archiving twice is not an error the UI has to handle.
    pub fn archive_msg(
        &self,
        room: &str,
        msg_id: &str,
        ts: u64,
        sender: &str,
        body: &str,
        now: u64,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO msg_archive (room, msg_id, ts, sender, body, archived_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(room, msg_id) DO UPDATE SET archived_at = ?6",
                rusqlite::params![room, msg_id, ts as i64, sender, body, now as i64],
            )
            .map(|_| ())
            .map_err(|e| format!("archive message: {e}"))
    }

    /// The ids hidden in a room — what `hub_log` filters the history against.
    pub fn archived_ids(&self, room: &str) -> Result<Vec<String>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT msg_id FROM msg_archive WHERE room = ?1")
            .map_err(|e| format!("prepare archived ids: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params![room], |r| r.get::<_, String>(0))
            .map_err(|e| format!("query archived ids: {e}"))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// The archive itself, newest first — a list you review before forgetting.
    pub fn archived_msgs(
        &self,
        room: &str,
    ) -> Result<Vec<(String, u64, String, String, u64)>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT msg_id, ts, sender, body, archived_at FROM msg_archive
                 WHERE room = ?1 ORDER BY archived_at DESC, ts DESC",
            )
            .map_err(|e| format!("prepare archive: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params![room], |r| {
                Ok((
                    r.get::<_, String>(0)?,
                    r.get::<_, i64>(1)? as u64,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)? as u64,
                ))
            })
            .map_err(|e| format!("query archive: {e}"))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// Take messages back out of the archive (restore), or forget the archive rows
    /// after the messages themselves have been deleted (purge). Returns how many
    /// rows went away.
    pub fn unarchive_msgs(&self, room: &str, ids: &[String]) -> Result<usize, String> {
        let mut n = 0;
        for id in ids {
            n += self
                .conn
                .execute(
                    "DELETE FROM msg_archive WHERE room = ?1 AND msg_id = ?2",
                    rusqlite::params![room, id],
                )
                .map_err(|e| format!("unarchive message: {e}"))?;
        }
        Ok(n)
    }

    // ---- activity log ---------------------------------------------------

    /// Append one observed event. Called on every hook, so it stays a single
    /// INSERT and its failure is the caller's to ignore.
    pub fn insert_activity(
        &self,
        session: &str,
        window: usize,
        ts: u64,
        kind: &str,
        text: &str,
        tool: &str,
        via: &str,
        state: &str,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO activity (session, window, ts, kind, text, tool, via, state)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![session, window as i64, ts as i64, kind, text, tool, via, state],
            )
            .map(|_| ())
            .map_err(|e| format!("insert activity: {e}"))
    }

    /// Events newer than `since_ts` (ms, exclusive), oldest first. `limit` caps
    /// the NEWEST end: a first load wants the tail of a long history, not its
    /// head, so the rows are selected descending and then reversed.
    pub fn activity_since(
        &self,
        session: &str,
        since_ts: u64,
        limit: usize,
    ) -> Result<Vec<ActivityRow>, String> {
        self.activity_page(session, since_ts, None, limit).map(|(rows, _)| rows)
    }

    /// One page of the activity log, always returned OLDEST FIRST so a caller can
    /// append it to a feed without re-sorting.
    ///
    /// Three shapes, one query:
    /// * neither cursor — the newest `limit` rows (what a first load wants: the
    ///   END of a conversation, not its beginning);
    /// * `since_ts > 0` — the tail newer than it, which is the incremental poll;
    /// * `before` — the page strictly OLDER than that (ts, id) cursor, which is
    ///   how a client walks backwards through history it has not loaded yet.
    ///
    /// The cursor is (ts, id) rather than ts alone because event timestamps are
    /// milliseconds and a busy turn puts several rows in one millisecond — paging
    /// on ts alone would either skip them or loop on them. `id` is the rowid, so
    /// the `(session, ts)` index already orders by (ts, id) and the keyset
    /// comparison stays index-only.
    ///
    /// `has_more` reports whether anything older than the returned page exists,
    /// measured by asking for one row more than the caller wanted. Without it a
    /// client cannot tell "you have everything" from "your page happened to end
    /// exactly at the limit".
    pub fn activity_page(
        &self,
        session: &str,
        since_ts: u64,
        before: Option<(u64, i64)>,
        limit: usize,
    ) -> Result<(Vec<ActivityRow>, bool), String> {
        let (b_ts, b_id) = match before {
            Some((ts, id)) => (ts as i64, id),
            None => (i64::MAX, i64::MAX),
        };
        let mut stmt = self
            .conn
            .prepare(
                "SELECT id, window, ts, kind, text, tool, via, state FROM activity
                 WHERE session = ?1 AND ts > ?2
                   AND (ts < ?3 OR (ts = ?3 AND id < ?4))
                 ORDER BY ts DESC, id DESC LIMIT ?5",
            )
            .map_err(|e| format!("prepare activity: {e}"))?;
        let rows = stmt
            .query_map(
                rusqlite::params![session, since_ts as i64, b_ts, b_id, limit as i64 + 1],
                |r| {
                    Ok(ActivityRow {
                        id: r.get(0)?,
                        window: r.get::<_, i64>(1)? as usize,
                        ts: r.get::<_, i64>(2)? as u64,
                        kind: r.get(3)?,
                        text: r.get(4)?,
                        tool: r.get(5)?,
                        via: r.get(6)?,
                        state: r.get(7)?,
                    })
                },
            )
            .map_err(|e| format!("query activity: {e}"))?;
        let mut out: Vec<ActivityRow> = rows.filter_map(Result::ok).collect();
        let has_more = out.len() > limit;
        out.truncate(limit);
        out.reverse();
        Ok((out, has_more))
    }

    /// How many events this session has ever recorded (and the whole log's oldest
    /// timestamp), for the storage audit and for a client that wants to say "3 of
    /// 4046 loaded".
    pub fn activity_stats(&self, session: &str) -> Result<(usize, u64, u64), String> {
        self.conn
            .query_row(
                "SELECT COUNT(*), COALESCE(MIN(ts), 0), COALESCE(MAX(ts), 0)
                 FROM activity WHERE session = ?1",
                rusqlite::params![session],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)? as usize,
                        r.get::<_, i64>(1)? as u64,
                        r.get::<_, i64>(2)? as u64,
                    ))
                },
            )
            .map_err(|e| format!("activity stats: {e}"))
    }

    /// Keep the newest `keep` events of a session and forget the rest. A log
    /// nobody prunes is a log that eventually costs more than it is worth.
    pub fn prune_activity(&self, session: &str, keep: usize) -> Result<usize, String> {
        self.conn
            .execute(
                "DELETE FROM activity WHERE session = ?1 AND id NOT IN
                   (SELECT id FROM activity WHERE session = ?1 ORDER BY id DESC LIMIT ?2)",
                rusqlite::params![session, keep as i64],
            )
            .map_err(|e| format!("prune activity: {e}"))
    }

    // ---- outstanding deliveries -----------------------------------------

    /// Remember a line we typed into a pane, so its `userPromptSubmit` echo can
    /// still be recognised as OUR delivery after a server restart. Upsert on the
    /// line: the in-memory queue replaces a re-typed line rather than holding two
    /// copies of it, and the durable half must not disagree.
    pub fn insert_delivery(
        &self,
        session: &str,
        window: usize,
        line: &str,
        ts: u64,
    ) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO deliveries (session, window, line, ts) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(session, window, line) DO UPDATE SET ts = ?4",
                rusqlite::params![session, window as i64, line, ts as i64],
            )
            .map(|_| ())
            .map_err(|e| format!("insert delivery: {e}"))
    }

    /// Outstanding lines, oldest first. `window` narrows it to one window; `None`
    /// is the whole session, which is what the sweep asks for.
    pub fn pending_deliveries(
        &self,
        session: &str,
        window: Option<usize>,
    ) -> Result<Vec<(usize, String, u64)>, String> {
        let (sql, args): (&str, Vec<Box<dyn rusqlite::ToSql>>) = match window {
            Some(w) => (
                "SELECT window, line, ts FROM deliveries
                 WHERE session = ?1 AND window = ?2 ORDER BY id",
                vec![Box::new(session.to_string()), Box::new(w as i64)],
            ),
            None => (
                "SELECT window, line, ts FROM deliveries WHERE session = ?1 ORDER BY id",
                vec![Box::new(session.to_string())],
            ),
        };
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| format!("prepare deliveries: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
                Ok((r.get::<_, i64>(0)? as usize, r.get::<_, String>(1)?, r.get::<_, i64>(2)? as u64))
            })
            .map_err(|e| format!("query deliveries: {e}"))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// A line is settled — acknowledged by its echo, or reported as unconfirmed.
    /// Either way it stops being outstanding.
    pub fn delete_delivery(&self, session: &str, window: usize, line: &str) -> Result<bool, String> {
        self.conn
            .execute(
                "DELETE FROM deliveries WHERE session = ?1 AND window = ?2 AND line = ?3",
                rusqlite::params![session, window as i64, line],
            )
            .map(|n| n > 0)
            .map_err(|e| format!("delete delivery: {e}"))
    }

    /// Forget every outstanding line of a window (it no longer exists, so it can
    /// never ack) or of a whole session.
    pub fn clear_deliveries(&self, session: &str, window: Option<usize>) -> Result<usize, String> {
        match window {
            Some(w) => self.conn.execute(
                "DELETE FROM deliveries WHERE session = ?1 AND window = ?2",
                rusqlite::params![session, w as i64],
            ),
            None => self
                .conn
                .execute("DELETE FROM deliveries WHERE session = ?1", rusqlite::params![session]),
        }
        .map_err(|e| format!("clear deliveries: {e}"))
    }

    /// Drop lines typed before `cutoff`. A delivery nobody ever acked is not
    /// worth resurrecting days later — the agent that would have echoed it is
    /// long gone — and this keeps the table bounded without a sweep of its own.
    pub fn prune_deliveries(&self, cutoff: u64) -> Result<usize, String> {
        self.conn
            .execute("DELETE FROM deliveries WHERE ts < ?1", rusqlite::params![cutoff as i64])
            .map_err(|e| format!("prune deliveries: {e}"))
    }

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
                "SELECT 1 FROM projects WHERE session = ?1 AND id <> ?2 LIMIT 1",
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
    pub fn set_session(&self, id: &str, session: &str, prev: &str) -> Result<bool, String> {
        self.conn
            .execute(
                "UPDATE projects SET session = ?2, prev_session = ?3 WHERE id = ?1",
                params![id, session, prev],
            )
            .map(|n| n > 0)
            .map_err(|e| format!("rename session: {e}"))
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
            .prepare("SELECT name, backend, model, effort, system, skills, mcp, can_hire FROM reg_agents ORDER BY name")
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
                "SELECT name, backend, model, effort, system, skills, mcp, can_hire FROM reg_agents WHERE name = ?1",
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
                "INSERT INTO reg_agents (name, backend, model, effort, system, skills, mcp, can_hire, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
                 ON CONFLICT(name) DO UPDATE SET
                   backend=?2, model=?3, effort=?4, system=?5, skills=?6, mcp=?7, can_hire=?8, updated_at=?9",
                rusqlite::params![
                    a.name,
                    a.backend,
                    a.model,
                    a.effort,
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
                effort: String::new(),
                system: "You are the project lead. Break the task down, do the core work yourself, and delegate well-scoped pieces to other agents when it genuinely helps. You may spawn agents with `tmm spawn <registry-name> --brief \"...\"` — check `tmm registry list` for who is available. Keep the human informed of decisions, not process.\n\nHow replies reach the room: the end of every turn is captured automatically from the stop hook and posted to the project room. You do not need to repeat a final result with `tmm send` — doing so wastes a call and the dedup filter drops it anyway. Use `tmm send` while a turn is in flight: that is the only way to report progress on a long task before it finishes. `tmm status waiting|blocked` is for being stuck on something outside your control — announcing that you are working is pointless, since turn boundaries are observed from your own hooks. Use `tmm done` to mark completion; its summary can be one line because the full result is already in the room.\n\nAddressing a teammate with @name is a delivery action, not a mention: `tmm send \"@reviewer 请审\"` types that line into the reviewer's pane and interrupts whatever they are doing. Only address someone when the intent is to hand off a task or ask a question that needs a reply. Never put credentials or secrets in a message — room contents are persisted and rendered to mobile clients.".into(),
                skills: "[]".into(),
                mcp: "[]".into(),
                can_hire: true,
            },
            RegAgent {
                name: "reviewer".into(),
                backend: "claude".into(),
                model: String::new(),
                effort: String::new(),
                system: "You are a code reviewer. Read the diff or branch you are briefed on, verify the change does what it claims, and report concrete findings (file:line) — no style nitpicks unless they hide bugs. Reply to whoever briefed you.\n\nHow replies reach the room: the end of every turn is captured automatically and posted to the project room. You do not need to repeat your findings with `tmm send`. Use `tmm send` only to address a specific teammate mid-task or to report a blocker; use `tmm done` to mark completion with a one-line summary.".into(),
                skills: "[]".into(),
                mcp: "[]".into(),
                can_hire: false,
            },
            RegAgent {
                name: "docs".into(),
                backend: "codex".into(),
                model: String::new(),
                effort: String::new(),
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

    // ---- the project task board (issues) ----------------------------------

    /// All issues of one project's board, newest movement first inside each
    /// status. `notes` is a COUNT here — the thread comes with `issue_get`.
    pub fn issues_list(&self, session: &str) -> Result<Vec<serde_json::Value>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT i.id, i.title, i.body, i.status, i.assignee, i.created_by,
                        i.created_at, i.updated_at,
                        (SELECT COUNT(*) FROM issue_notes n WHERE n.issue_id = i.id)
                   FROM issues i WHERE i.session = ?1
                  ORDER BY i.updated_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([session], |r| {
                Ok(serde_json::json!({
                    "id": r.get::<_, i64>(0)?,
                    "title": r.get::<_, String>(1)?,
                    "body": r.get::<_, String>(2)?,
                    "status": r.get::<_, String>(3)?,
                    "assignee": r.get::<_, String>(4)?,
                    "created_by": r.get::<_, String>(5)?,
                    "created_at": r.get::<_, i64>(6)?,
                    "updated_at": r.get::<_, i64>(7)?,
                    "notes": r.get::<_, i64>(8)?,
                }))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// One issue with its full note thread, or None.
    pub fn issue_get(&self, session: &str, id: i64) -> Result<Option<serde_json::Value>, String> {
        let issue = self
            .conn
            .query_row(
                "SELECT id, title, body, status, assignee, created_by, created_at, updated_at
                   FROM issues WHERE session = ?1 AND id = ?2",
                rusqlite::params![session, id],
                |r| {
                    Ok(serde_json::json!({
                        "id": r.get::<_, i64>(0)?,
                        "title": r.get::<_, String>(1)?,
                        "body": r.get::<_, String>(2)?,
                        "status": r.get::<_, String>(3)?,
                        "assignee": r.get::<_, String>(4)?,
                        "created_by": r.get::<_, String>(5)?,
                        "created_at": r.get::<_, i64>(6)?,
                        "updated_at": r.get::<_, i64>(7)?,
                    }))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(mut issue) = issue else { return Ok(None) };
        let mut stmt = self
            .conn
            .prepare("SELECT author, body, at FROM issue_notes WHERE issue_id = ?1 ORDER BY at, id")
            .map_err(|e| e.to_string())?;
        let notes = stmt
            .query_map([id], |r| {
                Ok(serde_json::json!({
                    "author": r.get::<_, String>(0)?,
                    "body": r.get::<_, String>(1)?,
                    "at": r.get::<_, i64>(2)?,
                }))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        issue["notes"] = serde_json::Value::Array(notes);
        Ok(Some(issue))
    }

    /// Create (id = None) or update. Update patches only the given fields, so
    /// an agent's `move` cannot erase a body the human wrote meanwhile.
    #[allow(clippy::too_many_arguments)]
    pub fn issue_save(
        &self,
        session: &str,
        id: Option<i64>,
        title: Option<&str>,
        body: Option<&str>,
        status: Option<&str>,
        assignee: Option<&str>,
        who: &str,
        now: i64,
    ) -> Result<i64, String> {
        match id {
            None => {
                let title = title.unwrap_or("").trim();
                if title.is_empty() {
                    return Err("an issue needs a title".into());
                }
                self.conn
                    .execute(
                        "INSERT INTO issues (session, title, body, status, assignee, created_by, created_at, updated_at)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
                        rusqlite::params![
                            session,
                            title,
                            body.unwrap_or(""),
                            status.unwrap_or("todo"),
                            assignee.unwrap_or(""),
                            who,
                            now
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                Ok(self.conn.last_insert_rowid())
            }
            Some(id) => {
                let n = self
                    .conn
                    .execute(
                        "UPDATE issues SET
                           title    = COALESCE(?3, title),
                           body     = COALESCE(?4, body),
                           status   = COALESCE(?5, status),
                           assignee = COALESCE(?6, assignee),
                           updated_at = ?7
                         WHERE session = ?1 AND id = ?2",
                        rusqlite::params![session, id, title, body, status, assignee, now],
                    )
                    .map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err(format!("no issue #{id} on this board"));
                }
                Ok(id)
            }
        }
    }

    pub fn issue_note(&self, session: &str, id: i64, author: &str, body: &str, now: i64) -> Result<(), String> {
        let body = body.trim();
        if body.is_empty() {
            return Err("an empty note says nothing".into());
        }
        // Session-scoped existence check first: a note must not attach to
        // another project's issue through a guessed id.
        let n = self
            .conn
            .execute(
                "UPDATE issues SET updated_at = ?3 WHERE session = ?1 AND id = ?2",
                rusqlite::params![session, id, now],
            )
            .map_err(|e| e.to_string())?;
        if n == 0 {
            return Err(format!("no issue #{id} on this board"));
        }
        self.conn
            .execute(
                "INSERT INTO issue_notes (issue_id, author, body, at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![id, author, body, now],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn issue_delete(&self, session: &str, id: i64) -> Result<bool, String> {
        self.conn
            .execute("DELETE FROM issues WHERE session = ?1 AND id = ?2", rusqlite::params![session, id])
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
    /// Reasoning effort (low|medium|high|…, backend-specific). Empty = the
    /// backend's default, same contract as `model`.
    #[serde(default)]
    pub effort: String,
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
        effort: r.get(3)?,
        system: r.get(4)?,
        skills: r.get(5)?,
        mcp: r.get(6)?,
        can_hire: r.get::<_, i64>(7)? != 0,
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
        room: r.get::<_, Option<String>>(11)?.unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The durable half of the delivery receipt (board #5). The rows outlive the
    /// process that typed the lines, so the SQL has to answer three questions:
    /// what is still outstanding for a window, what for a whole session (the
    /// sweep's question), and nothing at all for a neighbouring session.
    #[test]
    fn outstanding_deliveries_are_kept_per_window_and_settle_once() {
        let store = Store::open_memory().unwrap();
        store.insert_delivery("s", 1, "hello", 100).unwrap();
        // Re-typing the same line is the same outstanding line with a new clock,
        // never a second row that could never be acked twice.
        store.insert_delivery("s", 1, "hello", 150).unwrap();
        store.insert_delivery("s", 2, "other", 120).unwrap();
        store.insert_delivery("t", 1, "elsewhere", 130).unwrap();

        let all = store.pending_deliveries("s", None).unwrap();
        assert_eq!(all, vec![(1, "hello".to_string(), 150), (2, "other".to_string(), 120)]);
        assert_eq!(store.pending_deliveries("s", Some(2)).unwrap().len(), 1);
        assert_eq!(store.pending_deliveries("t", None).unwrap().len(), 1, "sessions never cross");

        // Acked or reported, a line leaves — and leaving twice is not an error.
        assert!(store.delete_delivery("s", 1, "hello").unwrap());
        assert!(!store.delete_delivery("s", 1, "hello").unwrap());
        // A window that no longer exists can never echo: drop its whole queue.
        assert_eq!(store.clear_deliveries("s", Some(2)).unwrap(), 1);
        assert!(store.pending_deliveries("s", None).unwrap().is_empty());

        // The recovery horizon: a line nobody ever acked is forgotten rather
        // than resurrected days later, and the fresh one stays.
        store.insert_delivery("t", 2, "ancient", 10).unwrap();
        assert_eq!(store.prune_deliveries(100).unwrap(), 1);
        assert_eq!(store.pending_deliveries("t", None).unwrap().len(), 1);
    }

    #[test]
    fn board_issues_live_move_and_remember() {
        let store = Store::open_memory().unwrap();
        // Create, then patch FIELD BY FIELD: an agent's `move` must not erase
        // the body the human wrote meanwhile (COALESCE semantics).
        let id = store
            .issue_save("proj", None, Some("fix login"), Some("the flow breaks at step 2"), None, None, "human", 100)
            .unwrap();
        store.issue_save("proj", Some(id), None, None, Some("doing"), Some("builder"), "builder", 200).unwrap();
        let got = store.issue_get("proj", id).unwrap().unwrap();
        assert_eq!(got["status"], "doing");
        assert_eq!(got["assignee"], "builder");
        assert_eq!(got["body"], "the flow breaks at step 2", "move kept the body");
        assert_eq!(got["created_by"], "human");

        // Notes thread in order and bump updated_at; the count rides the list.
        store.issue_note("proj", id, "builder", "root cause found", 300).unwrap();
        store.issue_note("proj", id, "human", "ship it", 400).unwrap();
        let got = store.issue_get("proj", id).unwrap().unwrap();
        let notes = got["notes"].as_array().unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0]["author"], "builder");
        assert_eq!(got["updated_at"], 400, "a note is board activity");
        let list = store.issues_list("proj").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0]["notes"], 2);

        // The board is SESSION-scoped: another project sees nothing, and a
        // guessed id cannot cross boards (note, update, get, delete alike).
        assert!(store.issues_list("other").unwrap().is_empty());
        assert!(store.issue_get("other", id).unwrap().is_none());
        assert!(store.issue_note("other", id, "x", "sneak", 500).is_err());
        assert!(store.issue_save("other", Some(id), None, None, Some("done"), None, "x", 500).is_err());
        assert!(!store.issue_delete("other", id).unwrap());

        // A title-less create is refused; delete cascades the notes.
        assert!(store.issue_save("proj", None, Some("  "), None, None, None, "human", 600).is_err());
        assert!(store.issue_delete("proj", id).unwrap());
        assert!(store.issue_get("proj", id).unwrap().is_none());
    }

    /// The heal step, which is not hypothetical: a dev binary built in the
    /// seconds between the version bump and its migration block stamped this
    /// host's real state.db at v13 with no `deliveries` table, and a
    /// version-gated CREATE would have skipped it for ever after that.
    #[test]
    fn a_database_stamped_at_the_current_version_without_its_table_heals_on_open() {
        let dir = std::env::temp_dir().join(format!("tmm-store-heal-{}", uuid::Uuid::new_v4()));
        let path = dir.join("state.db");
        {
            let store = Store::open(&path).unwrap();
            store.conn.execute_batch("DROP TABLE deliveries;").unwrap();
            store.conn.pragma_update(None, "user_version", SCHEMA_VERSION).unwrap();
            assert!(store.pending_deliveries("s", None).is_err(), "the table really is gone");
        }
        let store = Store::open(&path).unwrap();
        store.insert_delivery("s", 1, "hello", 100).unwrap();
        assert_eq!(store.pending_deliveries("s", None).unwrap().len(), 1, "healed on open");
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

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
            room: String::new(),
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

    #[test]
    fn the_activity_log_survives_and_stays_bounded() {
        let store = Store::open_memory().unwrap();
        for n in 0..5u64 {
            store
                .insert_activity("s1", 3, 1000 + n, "tool", &format!("file{n}.rs"), "Edit", "", "")
                .unwrap();
        }
        // Another session's rows never leak into this one's feed.
        store.insert_activity("s2", 1, 1002, "tool", "other.rs", "Read", "", "").unwrap();

        let all = store.activity_since("s1", 0, 100).unwrap();
        assert_eq!(all.len(), 5);
        assert_eq!(all.first().unwrap().ts, 1000, "oldest first");
        assert_eq!(all.last().unwrap().ts, 1004);
        assert_eq!(all[0].kind, "tool");
        assert_eq!(all[0].tool, "Edit", "the tool name is kept apart from its detail");

        // `since_ts` is exclusive — the client's cursor must not replay a row.
        let tail = store.activity_since("s1", 1002, 100).unwrap();
        assert_eq!(tail.len(), 2);
        assert_eq!(tail[0].ts, 1003);

        // A limit takes the NEWEST rows: a first load wants the tail of a long
        // history, not its beginning.
        let capped = store.activity_since("s1", 0, 2).unwrap();
        assert_eq!(capped.len(), 2);
        assert_eq!(capped[0].ts, 1003);
        assert_eq!(capped[1].ts, 1004);

        // Pruning is no longer automatic (board #9: nothing is thrown away unless
        // a retention was asked for), but it remains the primitive that a
        // configured retention uses — newest kept, other sessions untouched.
        let dropped = store.prune_activity("s1", 2).unwrap();
        assert_eq!(dropped, 3);
        let left = store.activity_since("s1", 0, 100).unwrap();
        assert_eq!(left.len(), 2);
        assert_eq!(left[0].ts, 1003);
        assert_eq!(store.activity_since("s2", 0, 100).unwrap().len(), 1);
    }

    /// Paging backwards through a complete log (board #9). The cursor is (ts, id)
    /// because a busy turn writes several events inside ONE millisecond: a
    /// ts-only cursor either skips them or loops on them for ever.
    #[test]
    fn the_activity_log_pages_backwards_without_losing_a_millisecond_tie() {
        let store = Store::open_memory().unwrap();
        // Six events, and three of them share ts 1002 — the shape a real turn has.
        for (n, ts) in [1000u64, 1001, 1002, 1002, 1002, 1003].into_iter().enumerate() {
            store.insert_activity("s", 1, ts, "tool", &format!("e{n}"), "Edit", "", "").unwrap();
        }
        assert_eq!(store.activity_stats("s").unwrap(), (6, 1000, 1003));

        // The newest page, oldest first, and there IS more behind it.
        let (page1, more1) = store.activity_page("s", 0, None, 2).unwrap();
        assert!(more1, "four older events remain");
        assert_eq!(page1.iter().map(|r| r.text.clone()).collect::<Vec<_>>(), vec!["e4", "e5"]);

        // Walk back with the page's own oldest row as the cursor. The tie at 1002
        // is respected: e3 comes next, not e1 and not e4 again.
        let cur = |p: &Vec<ActivityRow>| (p[0].ts, p[0].id);
        let (page2, more2) = store.activity_page("s", 0, Some(cur(&page1)), 2).unwrap();
        assert!(more2);
        assert_eq!(page2.iter().map(|r| r.text.clone()).collect::<Vec<_>>(), vec!["e2", "e3"]);
        let (page3, more3) = store.activity_page("s", 0, Some(cur(&page2)), 2).unwrap();
        assert_eq!(page3.iter().map(|r| r.text.clone()).collect::<Vec<_>>(), vec!["e0", "e1"]);
        assert!(!more3, "the whole log has been walked, and it says so");

        // Every row appeared exactly once — the property a lazy-loading client
        // depends on (no duplicates to dedupe, no gaps to explain).
        let mut seen: Vec<String> =
            [page1, page2, page3].concat().into_iter().map(|r| r.text).collect();
        seen.sort();
        assert_eq!(seen, vec!["e0", "e1", "e2", "e3", "e4", "e5"]);

        // A cursor with no id tiebreak means "just before that whole millisecond".
        let (before_ms, _) = store.activity_page("s", 0, Some((1002, 0)), 10).unwrap();
        assert_eq!(before_ms.iter().map(|r| r.text.clone()).collect::<Vec<_>>(), vec!["e0", "e1"]);

        // `since_ts` and `before` compose: the window between two cursors. The
        // cursor is EXCLUSIVE on the pair, so the newest row's own (ts, id)
        // excludes exactly itself.
        let head = store.activity_page("s", 0, None, 1).unwrap().0;
        let (window, _) =
            store.activity_page("s", 1000, Some((head[0].ts, head[0].id)), 10).unwrap();
        assert_eq!(window.len(), 4, "1001 and the three 1002s");
    }

    #[test]
    fn archiving_hides_a_message_and_restoring_gives_it_back() {
        let store = Store::open_memory().unwrap();
        store.archive_msg("proj:a", "m1", 100, "human", "a test probe", 900).unwrap();
        store.archive_msg("proj:a", "m2", 200, "dev", "another", 901).unwrap();
        store.archive_msg("proj:b", "m3", 300, "human", "other room", 902).unwrap();

        // The filter list is per room: another room's archive cannot hide a
        // message here.
        let mut ids = store.archived_ids("proj:a").unwrap();
        ids.sort();
        assert_eq!(ids, vec!["m1", "m2"]);
        assert_eq!(store.archived_ids("proj:b").unwrap(), vec!["m3"]);

        // The archive view is self-contained: it carries the message itself, so it
        // needs no join across two databases and no history window.
        let rows = store.archived_msgs("proj:a").unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, "m2", "newest archived first");
        assert_eq!(rows[0].2, "dev");
        assert_eq!(rows[1].3, "a test probe");

        // Archiving twice is not an error — the UI must not have to care.
        store.archive_msg("proj:a", "m1", 100, "human", "a test probe", 950).unwrap();
        assert_eq!(store.archived_msgs("proj:a").unwrap().len(), 2);

        // Restoring is dropping the row; the message never left the room's store.
        assert_eq!(store.unarchive_msgs("proj:a", &["m1".to_string()]).unwrap(), 1);
        assert_eq!(store.archived_ids("proj:a").unwrap(), vec!["m2"]);
        // Unknown ids are simply not there.
        assert_eq!(store.unarchive_msgs("proj:a", &["nope".to_string()]).unwrap(), 0);
    }
}
