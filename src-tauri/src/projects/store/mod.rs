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

mod schema;

/// One stored hub message (board #107) — the row shape of `hub_msgs`, column
/// names mirroring the agora `messages` table it replaced so the legacy
/// import is a straight copy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HubMsg {
    pub seq: i64,
    pub id: String,
    pub ts: i64,
    pub room: String,
    pub sender: String,
    /// JSON array of recipient names (`[]` = broadcast).
    pub to_json: String,
    pub kind: String,
    pub body: String,
}

/// Shared row mapper for every `SELECT seq, id, ts, room, sender, to_json,
/// kind, body FROM …` read (hub_msgs and the legacy import alike).
fn hub_msg_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<HubMsg> {
    Ok(HubMsg {
        seq: r.get(0)?,
        id: r.get(1)?,
        ts: r.get(2)?,
        room: r.get(3)?,
        sender: r.get(4)?,
        to_json: r.get(5)?,
        kind: r.get(6)?,
        body: r.get(7)?,
    })
}

/// The `omp` default's system text — shared by `reg_seed` (fresh installs)
/// and the v18 backfill migration (existing installs), so the two cannot
/// drift.
const DEFAULT_OMP_SYSTEM: &str = "You are a powerful 10x developer running on OMP (oh-my-pi) who can handle any task with decisive execution and minimal words.";

/// The omp default's model (owner, 2026-09-07: "设定默认的 omp agent 模型为
/// fable 5.1"). `bedrock-extra` is the provider the user's `~/.omp/agent/
/// models.yml` declares (the bundled omp catalog lacks Fable 5.1 on Bedrock);
/// `render_omp` carries that file into every isolated home, so the selector
/// resolves for spawned agents. On a machine without the catalog entry, omp
/// warns and falls back to its own default — the same soft degradation the
/// claude seed's Bedrock pin relies on.
const DEFAULT_OMP_MODEL: &str = "bedrock-extra/global.anthropic.claude-fable-5-1";

const LEGACY_DEFAULT_KIRO_SYSTEM: &str = "You are a powerful 10x developer running on Kiro CLI who can handle any task with decisive execution and minimal words.";
const VERBOSE_DEFAULT_KIRO_SYSTEM: &str = concat!(
    "You are a powerful 10x developer running on Kiro CLI who can handle any task with decisive execution and minimal words.",
    "\n\nGit workflow:\n",
    "- Before changing tracked files, create or reuse a dedicated Git worktree and task branch. If this session already runs inside that task worktree, use it; otherwise keep the launch checkout for reading, coordination, and final integration only.\n",
    "- Use the worktree's absolute path for every file edit, test, build, and Git command. Verify and commit in the worktree before integrating.\n",
    "- Preserve the user's configured Git author. Every commit you materially author must end with exactly one trailer, separated from the body by a blank line:\n",
    "  Co-authored-by: Kiro Agent <244629292+kiro-agent@users.noreply.github.com>\n",
    "- Do not add the Kiro trailer when you only review or integrate someone else's commit."
);
const DEFAULT_KIRO_SYSTEM: &str = concat!(
    "You are a powerful 10x developer running on Kiro CLI who can handle any task with decisive execution and minimal words.",
    "\n\nFor code changes, use a dedicated Git worktree instead of the launch checkout. Preserve the user's configured Git author and add this trailer to every commit you author:\n",
    "Co-authored-by: Kiro Agent <244629292+kiro-agent@users.noreply.github.com>"
);

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
    /// The window NAME (board #120); pre-v20 rows read back their index as a
    /// decimal string.
    pub window: String,
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

    /// Allocate one visible number atomically for a session. The sequence is
    /// advanced before the issue INSERT; a failed insert may leave a gap (like
    /// AUTOINCREMENT) but can never reuse a number that was already observed.
    fn next_issue_number(&self, session: &str) -> Result<i64, String> {
        self.conn
            .query_row(
                "INSERT INTO issue_sequences (session, next_number) VALUES (?1, 2)
                 ON CONFLICT(session) DO UPDATE SET
                   next_number = issue_sequences.next_number + 1
                 RETURNING next_number - 1",
                [session],
                |r| r.get(0),
            )
            .map_err(|e| format!("allocate Board project number: {e}"))
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

    // ---- hub messages (board #107) ---------------------------------------
    //
    // The project room's transcript. This is the store `server/hub_rpc.rs`
    // reads and writes directly — the agora bus that used to hold it was
    // deleted whole with the Team system (board #100). `seq` is the paging
    // cursor: stable, gapless within a room, already on every message a
    // client holds (a millisecond timestamp is not — two messages can share
    // one). Nothing is ever pruned; hiding is `msg_archive`'s job, deletion
    // is explicit.

    /// Append one message and hand the stored row back (seq assigned here).
    pub fn hub_append(
        &self,
        room: &str,
        id: &str,
        ts: i64,
        sender: &str,
        to_json: &str,
        kind: &str,
        body: &str,
    ) -> Result<HubMsg, String> {
        self.conn
            .execute(
                "INSERT INTO hub_msgs (id, ts, room, sender, to_json, kind, body)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                rusqlite::params![id, ts, room, sender, to_json, kind, body],
            )
            .map_err(|e| format!("append hub message: {e}"))?;
        let seq = self.conn.last_insert_rowid();
        Ok(HubMsg {
            seq,
            id: id.to_string(),
            ts,
            room: room.to_string(),
            sender: sender.to_string(),
            to_json: to_json.to_string(),
            kind: kind.to_string(),
            body: body.to_string(),
        })
    }

    /// One page of `room`'s messages, oldest first, walking backwards.
    /// `before_seq` is exclusive; `None` is the newest page. Returns
    /// `(messages, has_more, head_seq)` — `head_seq` is the room's newest seq.
    pub fn hub_page(
        &self,
        room: &str,
        before_seq: Option<i64>,
        limit: i64,
    ) -> Result<(Vec<HubMsg>, bool, i64), String> {
        let limit = limit.clamp(1, 1000);
        let head_seq: i64 = self
            .conn
            .query_row(
                "SELECT COALESCE(MAX(seq), 0) FROM hub_msgs WHERE room = ?1",
                rusqlite::params![room],
                |r| r.get(0),
            )
            .map_err(|e| format!("hub head seq: {e}"))?;
        // Fetch limit+1 newest-first to learn has_more, then flip to oldest-first.
        let mut stmt = self
            .conn
            .prepare(
                "SELECT seq, id, ts, room, sender, to_json, kind, body FROM hub_msgs
                 WHERE room = ?1 AND (?2 IS NULL OR seq < ?2)
                 ORDER BY seq DESC LIMIT ?3",
            )
            .map_err(|e| format!("prepare hub page: {e}"))?;
        let mut rows: Vec<HubMsg> = stmt
            .query_map(rusqlite::params![room, before_seq, limit + 1], hub_msg_row)
            .map_err(|e| format!("query hub page: {e}"))?
            .filter_map(Result::ok)
            .collect();
        let has_more = rows.len() as i64 > limit;
        rows.truncate(limit as usize);
        rows.reverse();
        Ok((rows, has_more, head_seq))
    }

    /// ONE message by its id, however old it is.
    pub fn hub_message_by_id(&self, room: &str, id: &str) -> Result<Option<HubMsg>, String> {
        self.conn
            .query_row(
                "SELECT seq, id, ts, room, sender, to_json, kind, body FROM hub_msgs
                 WHERE room = ?1 AND id = ?2",
                rusqlite::params![room, id],
                hub_msg_row,
            )
            .optional()
            .map_err(|e| format!("hub message by id: {e}"))
    }

    /// The newest `limit` messages matching ANY of `terms` (substring,
    /// ASCII-case-insensitive, body or sender), oldest first. `room = None`
    /// searches every room.
    ///
    /// The match runs in SQL (board #124): the old shape selected every row
    /// of the scope newest-first and filtered with `contains()` in Rust —
    /// fine while hits were dense, but a rare or absent term materialized the
    /// whole table (34 ms at 50k rows, measured). `LIKE` with `%`/`_`/`\`
    /// escaped keeps substring semantics; `lower()` matches the old
    /// `to_ascii_lowercase` exactly because SQLite's `lower()` is ASCII-only
    /// without ICU — case-insensitivity for ASCII, byte-verbatim for
    /// everything else, which is the behaviour the Rust path always had.
    pub fn hub_search(
        &self,
        room: Option<&str>,
        terms: &[String],
        limit: i64,
    ) -> Result<Vec<HubMsg>, String> {
        let limit = limit.clamp(1, 500);
        let terms: Vec<String> = terms
            .iter()
            .map(|t| t.trim().to_ascii_lowercase())
            .filter(|t| !t.is_empty())
            .collect();
        if terms.is_empty() {
            return Ok(Vec::new());
        }
        // One `%term%` pattern per term, LIKE-escaped so a literal `%`/`_` in
        // the query stays literal.
        let escape = |t: &str| t.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
        let patterns: Vec<String> = terms.iter().map(|t| format!("%{}%", escape(t))).collect();
        let clause = patterns
            .iter()
            .enumerate()
            .map(|(i, _)| {
                let p = i + 2; // ?1 is the room
                format!("lower(body) LIKE ?{p} ESCAPE '\\' OR lower(sender) LIKE ?{p} ESCAPE '\\'")
            })
            .collect::<Vec<_>>()
            .join(" OR ");
        let sql = format!(
            "SELECT seq, id, ts, room, sender, to_json, kind, body FROM hub_msgs
             WHERE (?1 IS NULL OR room = ?1) AND ({clause})
             ORDER BY seq DESC LIMIT {limit}"
        );
        let mut stmt = self.conn.prepare(&sql).map_err(|e| format!("prepare hub search: {e}"))?;
        let mut args: Vec<Box<dyn rusqlite::ToSql>> = vec![Box::new(room.map(str::to_string))];
        for p in &patterns {
            args.push(Box::new(p.clone()));
        }
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())), hub_msg_row)
            .map_err(|e| format!("query hub search: {e}"))?;
        let mut hits: Vec<HubMsg> = rows.filter_map(Result::ok).collect();
        hits.reverse();
        Ok(hits)
    }

    /// Newest message timestamp (ms) per room — what orders the project list.
    pub fn hub_room_latest(&self) -> Result<Vec<(String, i64)>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT room, MAX(ts) FROM hub_msgs GROUP BY room")
            .map_err(|e| format!("prepare room latest: {e}"))?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?)))
            .map_err(|e| format!("query room latest: {e}"))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// Forget messages by id, for good — the irreversible half of deleting
    /// (`msg_archive` is the reversible half).
    pub fn hub_delete(&self, room: &str, ids: &[String]) -> Result<usize, String> {
        let mut n = 0;
        for id in ids {
            n += self
                .conn
                .execute(
                    "DELETE FROM hub_msgs WHERE room = ?1 AND id = ?2",
                    rusqlite::params![room, id],
                )
                .map_err(|e| format!("delete hub message: {e}"))?;
        }
        Ok(n)
    }

    /// One-shot flag store (v19 `meta`).
    pub fn meta_get(&self, key: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT value FROM meta WHERE key = ?1",
                rusqlite::params![key],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("meta get: {e}"))
    }

    pub fn meta_set(&self, key: &str, value: &str) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO meta (key, value) VALUES (?1, ?2)
                 ON CONFLICT(key) DO UPDATE SET value = ?2",
                rusqlite::params![key, value],
            )
            .map(|_| ())
            .map_err(|e| format!("meta set: {e}"))
    }

    /// The one-off legacy import (board #107): copy every `proj:*` room out of
    /// an agora `team.db` whose schema this mirrors. Seq/id/ts are preserved so
    /// existing clients' cursors stay valid; INSERT OR IGNORE makes a retried
    /// partial import safe. Team rooms (`tmm-team-*` sessions' slugs) are the
    /// deleted feature's data and stay behind.
    pub fn hub_import_from(&mut self, legacy_db: &Path) -> Result<usize, String> {
        let src = Connection::open_with_flags(
            legacy_db,
            rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
        )
        .map_err(|e| format!("open legacy team.db: {e}"))?;
        let mut stmt = src
            .prepare(
                "SELECT seq, id, ts, room, sender, to_json, kind, body FROM messages
                 WHERE room LIKE 'proj:%' ORDER BY seq",
            )
            .map_err(|e| format!("prepare legacy read: {e}"))?;
        let rows: Vec<HubMsg> = stmt
            .query_map([], hub_msg_row)
            .map_err(|e| format!("read legacy messages: {e}"))?
            .filter_map(Result::ok)
            .collect();
        drop(stmt);
        let tx = self.conn.transaction().map_err(|e| format!("import tx: {e}"))?;
        let mut n = 0;
        for m in &rows {
            n += tx
                .execute(
                    "INSERT OR IGNORE INTO hub_msgs (seq, id, ts, room, sender, to_json, kind, body)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![m.seq, m.id, m.ts, m.room, m.sender, m.to_json, m.kind, m.body],
                )
                .map_err(|e| format!("import hub message: {e}"))?;
        }
        tx.commit().map_err(|e| format!("commit import: {e}"))?;
        Ok(n)
    }

    // ---- activity log ---------------------------------------------------

    /// Append one observed event. Called on every hook, so it stays a single
    /// INSERT and its failure is the caller's to ignore.
    pub fn insert_activity(
        &self,
        session: &str,
        window: &str,
        ts: u64,
        kind: &str,
        text: &str,
        tool: &str,
        via: &str,
        state: &str,
    ) -> Result<(), String> {
        // `window` (the INDEX column) is 0 for name-keyed rows; `win` carries
        // the identity (board #120). Old rows read back via the COALESCE below.
        self.conn
            .execute(
                "INSERT INTO activity (session, window, win, ts, kind, text, tool, via, state)
                 VALUES (?1, 0, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                rusqlite::params![session, window, ts as i64, kind, text, tool, via, state],
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

    /// The prompt that opened the currently unclosed turn, if any. Used to
    /// recover an automatic reply edge after a server restart.
    pub fn current_turn_prompt(&self, session: &str, window: &str) -> Result<Option<String>, String> {
        self.conn
            .query_row(
                "SELECT text FROM activity
                 WHERE session = ?1 AND COALESCE(NULLIF(win, ''), CAST(window AS TEXT)) = ?2 AND kind = 'prompt'
                   AND id > COALESCE((
                     SELECT MAX(id) FROM activity
                     WHERE session = ?1 AND COALESCE(NULLIF(win, ''), CAST(window AS TEXT)) = ?2 AND kind = 'notif'
                       AND text IN ('completed', 'failed')
                   ), 0)
                 ORDER BY id DESC LIMIT 1",
                rusqlite::params![session, window],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| format!("query current turn prompt: {e}"))
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
                "SELECT id, COALESCE(NULLIF(win, ''), CAST(window AS TEXT)), ts, kind, text, tool, via, state FROM activity
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
                        window: r.get::<_, String>(1)?,
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
        window: &str,
        line: &str,
        ts: u64,
    ) -> Result<(), String> {
        // A plain INSERT (board #122): the same line delivered again is a new
        // promise with its own row and its own echo.
        self.conn
            .execute(
                "INSERT INTO deliveries (session, win, line, ts) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![session, window, line, ts as i64],
            )
            .map(|_| ())
            .map_err(|e| format!("insert delivery: {e}"))
    }

    /// Outstanding lines, oldest first. `window` narrows it to one window; `None`
    /// is the whole session, which is what the sweep asks for.
    pub fn pending_deliveries(
        &self,
        session: &str,
        window: Option<&str>,
    ) -> Result<Vec<(String, String, u64)>, String> {
        let (sql, args): (&str, Vec<Box<dyn rusqlite::ToSql>>) = match window {
            Some(w) => (
                "SELECT win, line, ts FROM deliveries
                 WHERE session = ?1 AND win = ?2 ORDER BY id",
                vec![Box::new(session.to_string()), Box::new(w.to_string())],
            ),
            None => (
                "SELECT win, line, ts FROM deliveries WHERE session = ?1 ORDER BY id",
                vec![Box::new(session.to_string())],
            ),
        };
        let mut stmt = self
            .conn
            .prepare(sql)
            .map_err(|e| format!("prepare deliveries: {e}"))?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(args.iter().map(|a| a.as_ref())), |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)? as u64))
            })
            .map_err(|e| format!("query deliveries: {e}"))?;
        Ok(rows.filter_map(Result::ok).collect())
    }

    /// A line is settled — acknowledged by its echo, or reported as unconfirmed.
    /// Either way it stops being outstanding.
    /// Settle ONE row of a possibly-duplicated line, oldest first (board #122).
    pub fn delete_one_delivery(&self, session: &str, window: &str, line: &str) -> Result<bool, String> {
        self.conn
            .execute(
                "DELETE FROM deliveries WHERE id = (
                   SELECT id FROM deliveries
                   WHERE session = ?1 AND win = ?2 AND line = ?3
                   ORDER BY id LIMIT 1
                 )",
                rusqlite::params![session, window, line],
            )
            .map(|n| n > 0)
            .map_err(|e| format!("delete one delivery: {e}"))
    }

    pub fn delete_delivery(&self, session: &str, window: &str, line: &str) -> Result<bool, String> {
        self.conn
            .execute(
                "DELETE FROM deliveries WHERE session = ?1 AND win = ?2 AND line = ?3",
                rusqlite::params![session, window, line],
            )
            .map(|n| n > 0)
            .map_err(|e| format!("delete delivery: {e}"))
    }

    /// Forget every outstanding line of a window (it no longer exists, so it can
    /// never ack) or of a whole session.
    pub fn clear_deliveries(&self, session: &str, window: Option<&str>) -> Result<usize, String> {
        match window {
            Some(w) => self.conn.execute(
                "DELETE FROM deliveries WHERE session = ?1 AND win = ?2",
                rusqlite::params![session, w],
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

    pub fn reg_list(&self) -> Result<Vec<RegAgent>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT name, backend, model, effort, system, skills, mcp, can_hire
                   FROM reg_agents
                  ORDER BY CASE name
                    WHEN 'kiro' THEN 0
                    WHEN 'codex' THEN 1
                    WHEN 'claude' THEN 2
                    WHEN 'grok' THEN 3
                    WHEN 'omp' THEN 4
                    ELSE 5
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

    // ---- agent teams (board #74) -----------------------------------------

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

    /// Seed the five backend-native Manager defaults once (empty table only).
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
                system: DEFAULT_KIRO_SYSTEM.into(),
                skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
                mcp: "[]".into(),
                can_hire: true,
            },
            RegAgent {
                name: "codex".into(),
                backend: "codex".into(),
                model: String::new(),
                effort: String::new(),
                system: "You are a powerful 10x developer running on Codex who can handle any task with decisive execution and minimal words.".into(),
                skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
                mcp: r#"[{"name":"kiro-web-search","command":"uvx","args":["kiro-web-search==0.1.3"]}]"#.into(),
                can_hire: true,
            },
            RegAgent {
                name: "claude".into(),
                backend: "claude".into(),
                model: "global.anthropic.claude-fable-5-1[1m]".into(),
                effort: String::new(),
                system: "You are a powerful 10x developer running on Claude Code who can handle any task with decisive execution and minimal words.".into(),
                skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
                mcp: r#"[{"name":"kiro-web-search","command":"uvx","args":["kiro-web-search==0.1.3"]}]"#.into(),
                can_hire: true,
            },
            RegAgent {
                name: "grok".into(),
                backend: "grok".into(),
                model: String::new(),
                effort: String::new(),
                system: "You are a powerful 10x developer running on Grok who can handle any task with decisive execution and minimal words.".into(),
                skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
                mcp: r#"[{"name":"kiro-web-search","command":"uvx","args":["kiro-web-search==0.1.3"]}]"#.into(),
                can_hire: true,
            },
            RegAgent {
                name: "omp".into(),
                backend: "omp".into(),
                model: DEFAULT_OMP_MODEL.into(),
                effort: String::new(),
                system: DEFAULT_OMP_SYSTEM.into(),
                skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
                // omp ships its own web_search tool — no MCP search shim needed.
                mcp: "[]".into(),
                can_hire: true,
            },
        ];
        // backend-seeds:end
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
                "SELECT i.project_number, i.title, i.body, i.status, i.assignee, i.created_by,
                        i.created_at, i.updated_at, i.agent_touched,
                        (SELECT COUNT(*) FROM issue_notes n WHERE n.issue_id = i.id)
                   FROM issues i WHERE i.session = ?1
                  ORDER BY i.updated_at DESC",
            )
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([session], |r| {
                let assignee = r.get::<_, String>(4)?;
                let agent_touched = r.get::<_, i64>(8)? != 0;
                Ok(serde_json::json!({
                    "id": r.get::<_, i64>(0)?,
                    "title": r.get::<_, String>(1)?,
                    "body": r.get::<_, String>(2)?,
                    "status": r.get::<_, String>(3)?,
                    "assignee": assignee,
                    "created_by": r.get::<_, String>(5)?,
                    "created_at": r.get::<_, i64>(6)?,
                    "updated_at": r.get::<_, i64>(7)?,
                    "editable": assignee.is_empty() && !agent_touched,
                    "notes": r.get::<_, i64>(9)?,
                }))
            })
            .map_err(|e| e.to_string())?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| e.to_string())?;
        Ok(rows)
    }

    /// Issue counts per (session, status) — ONE grouped query over every
    /// board (board #39): the sidebar wants "which projects have work, how
    /// much, in which column" for ALL projects at once, and asking
    /// `issues_list` per project is the N+1 the grouped read exists to
    /// prevent. Raw rows here; the projects layer shapes them (zero-fill,
    /// totals) so the SQL stays a plain aggregate.
    pub fn issue_counts(&self) -> Result<Vec<(String, String, i64)>, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT session, status, COUNT(*) FROM issues GROUP BY session, status")
            .map_err(|e| e.to_string())?;
        let rows = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, i64>(2)?)))
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
                "SELECT id, project_number, title, body, status, assignee, created_by,
                        created_at, updated_at, agent_touched
                   FROM issues WHERE session = ?1 AND project_number = ?2",
                rusqlite::params![session, id],
                |r| {
                    let assignee = r.get::<_, String>(5)?;
                    let agent_touched = r.get::<_, i64>(9)? != 0;
                    Ok((r.get::<_, i64>(0)?, serde_json::json!({
                        "id": r.get::<_, i64>(1)?,
                        "title": r.get::<_, String>(2)?,
                        "body": r.get::<_, String>(3)?,
                        "status": r.get::<_, String>(4)?,
                        "assignee": assignee,
                        "created_by": r.get::<_, String>(6)?,
                        "created_at": r.get::<_, i64>(7)?,
                        "updated_at": r.get::<_, i64>(8)?,
                        "editable": assignee.is_empty() && !agent_touched,
                    })))
                },
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some((row_id, mut issue)) = issue else { return Ok(None) };
        let mut stmt = self
            .conn
            .prepare("SELECT author, body, at FROM issue_notes WHERE issue_id = ?1 ORDER BY at, id")
            .map_err(|e| e.to_string())?;
        let notes = stmt
            .query_map([row_id], |r| {
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
                // A title is OPTIONAL (board #31) — the one rule is that an
                // issue is never CONTENTLESS: at least one of title/body must
                // say something. An empty title is stored EMPTY, verbatim;
                // display fallbacks are the reader's job (`issue_ref`), never
                // fabricated into persistence.
                let title = title.unwrap_or("").trim();
                if title.is_empty() && body.unwrap_or("").trim().is_empty() {
                    return Err("an issue needs a title or a body".into());
                }
                let number = self.next_issue_number(session)?;
                self.conn
                    .execute(
                        "INSERT INTO issues
                           (session, project_number, title, body, status, assignee, created_by, created_at, updated_at, agent_touched)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?8, ?9)",
                        rusqlite::params![
                            session,
                            number,
                            title,
                            body.unwrap_or(""),
                            status.unwrap_or("todo"),
                            assignee.unwrap_or(""),
                            who,
                            now,
                            (who != "human") as i64
                        ],
                    )
                    .map_err(|e| e.to_string())?;
                Ok(number)
            }
            Some(id) => {
                // The contentless rule holds through PATCHES too: a save may
                // clear the title (Some("")) only if what it leaves behind —
                // patched or kept — still has a body, and vice versa.
                let cur: Option<(String, String, String, bool)> = self
                    .conn
                    .query_row(
                        "SELECT title, body, assignee, agent_touched
                           FROM issues WHERE session = ?1 AND project_number = ?2",
                        rusqlite::params![session, id],
                        |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get::<_, i64>(3)? != 0)),
                    )
                    .optional()
                    .map_err(|e| e.to_string())?;
                let Some((cur_title, cur_body, cur_assignee, agent_touched)) = cur else {
                    return Err(format!("no issue #{id} on this board"));
                };
                // The original brief is history after dispatch or Agent work.
                // Workflow fields remain patchable — status/assignee are how
                // the issue continues moving — but title/body cannot be
                // rewritten under the discussion that already followed them.
                if (title.is_some() || body.is_some())
                    && (!cur_assignee.is_empty() || agent_touched)
                {
                    return Err("issue title/body are locked after assignment or Agent activity".into());
                }
                if title.unwrap_or(&cur_title).trim().is_empty()
                    && body.unwrap_or(&cur_body).trim().is_empty()
                {
                    return Err("an issue needs a title or a body".into());
                }
                let n = self
                    .conn
                    .execute(
                        "UPDATE issues SET
                           title    = COALESCE(?3, title),
                           body     = COALESCE(?4, body),
                           status   = COALESCE(?5, status),
                           assignee = COALESCE(?6, assignee),
                           updated_at = ?7,
                           agent_touched = CASE WHEN ?8 <> 'human' THEN 1 ELSE agent_touched END
                         WHERE session = ?1 AND project_number = ?2",
                        rusqlite::params![session, id, title, body, status, assignee, now, who],
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
        // Resolve the session-local visible number to the hidden global row
        // key in the same gated statement. Notes keep their existing FK.
        let row_id: Option<i64> = self
            .conn
            .query_row(
                "UPDATE issues
                    SET updated_at = ?3,
                        agent_touched = CASE WHEN ?4 <> 'human' THEN 1 ELSE agent_touched END
                  WHERE session = ?1 AND project_number = ?2
                  RETURNING id",
                rusqlite::params![session, id, now, author],
                |r| r.get(0),
            )
            .optional()
            .map_err(|e| e.to_string())?;
        let Some(row_id) = row_id else {
            return Err(format!("no issue #{id} on this board"));
        };
        self.conn
            .execute(
                "INSERT INTO issue_notes (issue_id, author, body, at) VALUES (?1, ?2, ?3, ?4)",
                rusqlite::params![row_id, author, body, now],
            )
            .map(|_| ())
            .map_err(|e| e.to_string())
    }

    pub fn issue_delete(&self, session: &str, id: i64) -> Result<bool, String> {
        self.conn
            .execute(
                "DELETE FROM issues WHERE session = ?1 AND project_number = ?2",
                rusqlite::params![session, id],
            )
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

fn row_to_reg_team(r: &rusqlite::Row<'_>) -> rusqlite::Result<RegTeam> {
    Ok(RegTeam { name: r.get(0)?, description: r.get(1)?, members: r.get(2)? })
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

    /// The durable half of the delivery receipt (board #5). The rows outlive the
    /// process that typed the lines, so the SQL has to answer three questions:
    /// what is still outstanding for a window, what for a whole session (the
    /// sweep's question), and nothing at all for a neighbouring session.
    #[test]
    fn outstanding_deliveries_are_kept_per_window_and_settle_once() {
        let store = Store::open_memory().unwrap();
        store.insert_delivery("s", "w1", "hello", 100).unwrap();
        // Delivering the same body again is a SECOND promise with its own row
        // (board #122): the pane was typed into twice, two echoes are coming,
        // and each settles one row, oldest first.
        store.insert_delivery("s", "w1", "hello", 150).unwrap();
        store.insert_delivery("s", "w2", "other", 120).unwrap();
        store.insert_delivery("t", "w1", "elsewhere", 130).unwrap();

        let all = store.pending_deliveries("s", None).unwrap();
        assert_eq!(all, vec![
            ("w1".to_string(), "hello".to_string(), 100),
            ("w1".to_string(), "hello".to_string(), 150),
            ("w2".to_string(), "other".to_string(), 120),
        ]);
        assert_eq!(store.pending_deliveries("s", Some("w2")).unwrap().len(), 1);
        assert_eq!(store.pending_deliveries("t", None).unwrap().len(), 1, "sessions never cross");

        // Each echo settles ONE row, oldest first; leaving past the last is
        // not an error.
        assert!(store.delete_one_delivery("s", "w1", "hello").unwrap());
        assert_eq!(store.pending_deliveries("s", Some("w1")).unwrap(), vec![("w1".to_string(), "hello".to_string(), 150)], "the older row went first");
        assert!(store.delete_one_delivery("s", "w1", "hello").unwrap());
        assert!(!store.delete_one_delivery("s", "w1", "hello").unwrap());
        // A window that no longer exists can never echo: drop its whole queue.
        assert_eq!(store.clear_deliveries("s", Some("w2")).unwrap(), 1);
        assert!(store.pending_deliveries("s", None).unwrap().is_empty());

        // The recovery horizon: a line nobody ever acked is forgotten rather
        // than resurrected days later, and the fresh one stays.
        store.insert_delivery("t", "w2", "ancient", 10).unwrap();
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

        // A CONTENTLESS create is refused; a body alone is enough (board
        // #31: the title is optional) and the empty title persists EMPTY —
        // no fabricated fallback in storage.
        assert!(store.issue_save("proj", None, Some("  "), None, None, None, "human", 600).is_err());
        assert!(store.issue_save("proj", None, None, Some("  "), None, None, "human", 600).is_err());
        let bare = store.issue_save("proj", None, None, Some("body only, no title"), None, None, "human", 610).unwrap();
        assert_eq!(bare, 2, "rejected contentless creates do not consume a visible number");
        let got = store.issue_get("proj", bare).unwrap().unwrap();
        assert_eq!(got["title"].as_str().unwrap(), "");
        assert_eq!(got["body"].as_str().unwrap(), "body only, no title");
        // A patch may CLEAR the title while a body remains…
        store.issue_save("proj", Some(bare), Some(""), None, None, None, "human", 620).unwrap();
        // …but never the last content: emptying the body of a title-less
        // issue (or both at once) is refused, patched-and-kept alike.
        assert!(store.issue_save("proj", Some(bare), None, Some("  "), None, None, "human", 630).is_err());
        assert!(store.issue_save("proj", Some(bare), Some(""), Some(""), None, None, "human", 630).is_err());
        // The other direction: clearing the BODY is fine while a title holds.
        store.issue_save("proj", Some(bare), Some("now titled"), Some(""), None, None, "human", 640).unwrap();
        assert!(store.issue_delete("proj", bare).unwrap());

        assert!(store.issue_delete("proj", id).unwrap());
        assert!(store.issue_get("proj", id).unwrap().is_none());
    }

    #[test]
    fn original_issue_text_locks_on_assignment_or_agent_activity() {
        let store = Store::open_memory().unwrap();

        let id = store.issue_save("proj", None, Some("brief"), Some("body"), None, None, "human", 100).unwrap();
        assert_eq!(store.issue_get("proj", id).unwrap().unwrap()["editable"], true);

        // Assignment locks immediately, but undoing it before the Agent acts
        // reopens the text: current assignment and durable activity are the
        // two independent gates from board #43.
        store.issue_save("proj", Some(id), None, None, None, Some("builder"), "human", 110).unwrap();
        assert_eq!(store.issue_get("proj", id).unwrap().unwrap()["editable"], false);
        assert!(store.issue_save("proj", Some(id), Some("rewrite"), None, None, None, "human", 120).is_err());
        store.issue_save("proj", Some(id), None, None, None, Some(""), "human", 130).unwrap();
        assert_eq!(store.issue_get("proj", id).unwrap().unwrap()["editable"], true);
        store.issue_save("proj", Some(id), Some("clarified"), None, None, None, "human", 140).unwrap();

        // Any Agent mutation is durable: unassigning or moving back cannot
        // make the original brief editable again. Workflow still moves.
        store.issue_save("proj", Some(id), None, None, Some("doing"), Some("builder"), "builder", 150).unwrap();
        store.issue_save("proj", Some(id), None, None, Some("todo"), Some(""), "human", 160).unwrap();
        let got = store.issue_get("proj", id).unwrap().unwrap();
        assert_eq!(got["editable"], false);
        assert_eq!(got["title"], "clarified");
        assert!(store.issue_save("proj", Some(id), None, Some("rewrite history"), None, None, "human", 170).is_err());

        let noted = store.issue_save("proj", None, Some("note lock"), None, None, None, "human", 200).unwrap();
        store.issue_note("proj", noted, "builder", "started", 210).unwrap();
        assert_eq!(store.issue_get("proj", noted).unwrap().unwrap()["editable"], false);

        let agent_owned = store.issue_save("proj", None, Some("agent filed"), None, None, None, "builder", 300).unwrap();
        assert_eq!(store.issue_get("proj", agent_owned).unwrap().unwrap()["editable"], false);
    }

    #[test]
    fn issue_numbers_start_at_one_per_project_and_never_reuse() {
        let store = Store::open_memory().unwrap();
        let a1 = store.issue_save("a", None, Some("a1"), None, None, None, "human", 10).unwrap();
        let b1 = store.issue_save("b", None, Some("b1"), None, None, None, "human", 11).unwrap();
        let a2 = store.issue_save("a", None, Some("a2"), None, None, None, "human", 12).unwrap();
        assert_eq!((a1, b1, a2), (1, 1, 2), "each project owns a sequence starting at one");

        // The same visible #1 resolves independently through session + number.
        store.issue_save("a", Some(1), None, None, Some("doing"), None, "human", 13).unwrap();
        store.issue_note("b", 1, "human", "only b", 14).unwrap();
        assert_eq!(store.issue_get("a", 1).unwrap().unwrap()["title"], "a1");
        assert_eq!(store.issue_get("a", 1).unwrap().unwrap()["notes"].as_array().unwrap().len(), 0);
        assert_eq!(store.issue_get("b", 1).unwrap().unwrap()["title"], "b1");
        assert_eq!(store.issue_get("b", 1).unwrap().unwrap()["notes"].as_array().unwrap().len(), 1);

        // Deleting the high card never reuses a number somebody may have seen
        // in chat/CLI history; each session advances independently.
        assert!(store.issue_delete("a", 2).unwrap());
        assert_eq!(store.issue_save("a", None, Some("a3"), None, None, None, "human", 15).unwrap(), 3);
        assert_eq!(store.issue_save("b", None, Some("b2"), None, None, None, "human", 16).unwrap(), 2);
        assert!(store.issue_get("a", 2).unwrap().is_none(), "the deleted number stays a gap");
        assert!(store.issue_delete("a", 1).unwrap());
        assert!(store.issue_delete("a", 3).unwrap());
        assert_eq!(
            store.issue_save("a", None, Some("a4"), None, None, None, "human", 17).unwrap(),
            4,
            "even an empty board remembers its next number until the project is permanently deleted",
        );
    }

    #[test]
    fn issue_counts_group_across_sessions_in_one_read() {
        let store = Store::open_memory().unwrap();
        // Two boards, mixed statuses — the grouped read must keep them apart
        // and count within (session, status), never across (board #39).
        let a1 = store.issue_save("proj-a", None, Some("t1"), None, None, None, "human", 100).unwrap();
        store.issue_save("proj-a", None, Some("t2"), None, None, None, "human", 110).unwrap();
        store.issue_save("proj-a", Some(a1), None, None, Some("doing"), None, "human", 120).unwrap();
        let b1 = store.issue_save("proj-b", None, Some("t3"), None, None, None, "human", 130).unwrap();
        store.issue_save("proj-b", Some(b1), None, None, Some("done"), None, "human", 140).unwrap();

        let rows = store.issue_counts().unwrap();
        let n = |s: &str, st: &str| rows.iter().find(|(a, b, _)| a == s && b == st).map(|(_, _, n)| *n);
        assert_eq!(n("proj-a", "todo"), Some(1));
        assert_eq!(n("proj-a", "doing"), Some(1));
        assert_eq!(n("proj-b", "done"), Some(1));
        // A (session, status) cell with no issues yields NO row — absence,
        // not zero: the shaping layer owns the vocabulary fill.
        assert_eq!(n("proj-a", "done"), None);
        assert_eq!(n("proj-b", "todo"), None);
        // A session with no issues at all appears nowhere.
        assert!(!rows.iter().any(|(s, _, _)| s == "proj-empty"));
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

    #[test]
    fn registry_crud_roundtrip() {
        let store = Store::open_memory().unwrap();
        store.reg_seed(100).unwrap();
        let seeded = store.reg_list().unwrap();
        assert_eq!(
            seeded.iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["kiro", "codex", "claude", "grok", "omp"],
            "the five backend defaults are the fixed leading group"
        );
        assert!(seeded.iter().all(|a| a.can_hire), "every default is a Manager");
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
        assert!(!seeded.iter().any(|a| matches!(a.name.as_str(), "docs" | "reviewer")));

        // Seeding twice must not duplicate.
        store.reg_seed(200).unwrap();
        assert_eq!(store.reg_list().unwrap().len(), 5);

        // A custom definition alphabetically before the defaults stays AFTER
        // their fixed group; the rest of the list is alphabetical.
        let custom = RegAgent {
            name: "aaa-custom".into(),
            backend: "kiro".into(),
            model: String::new(),
            effort: String::new(),
            system: "custom".into(),
            skills: "[]".into(),
            mcp: "[]".into(),
            can_hire: false,
        };
        store.reg_save(&custom, 250).unwrap();
        assert_eq!(
            store.reg_list().unwrap().iter().map(|a| a.name.as_str()).collect::<Vec<_>>(),
            ["kiro", "codex", "claude", "grok", "omp", "aaa-custom"]
        );

        // Upsert edits in place.
        let mut kiro = store.reg_get("kiro").unwrap().unwrap();
        kiro.model = "gpt-5.6-sol".into();
        store.reg_save(&kiro, 300).unwrap();
        assert_eq!(store.reg_get("kiro").unwrap().unwrap().model, "gpt-5.6-sol");
        assert_eq!(store.reg_list().unwrap().len(), 6, "save by name is an upsert");

        assert!(store.reg_delete("aaa-custom").unwrap());
        assert!(!store.reg_delete("aaa-custom").unwrap(), "second delete is a no-op");
        assert_eq!(store.reg_list().unwrap().len(), 5);
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
            system: legacy.into(),
            skills: r#"["tmm-cli","mem","mcp-cli"]"#.into(),
            mcp: "[]".into(),
            can_hire: true,
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
        assert!(upgraded.can_hire, "a prompt upgrade preserves Manager status");

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

    #[test]
    fn the_activity_log_survives_and_stays_bounded() {
        let store = Store::open_memory().unwrap();
        for n in 0..5u64 {
            store
                .insert_activity("s1", "w3", 1000 + n, "tool", &format!("file{n}.rs"), "Edit", "", "")
                .unwrap();
        }
        // Another session's rows never leak into this one's feed.
        store.insert_activity("s2", "w1", 1002, "tool", "other.rs", "Read", "", "").unwrap();

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

    #[test]
    fn current_turn_prompt_survives_the_server_process() {
        let store = Store::open_memory().unwrap();
        store.insert_activity("s", "w2", 1000, "notif", "completed", "", "", "").unwrap();
        store.insert_activity("s", "w2", 1100, "prompt", "[tmm chat] lead: work", "", "app", "").unwrap();
        store.insert_activity("s", "w2", 1200, "tool", "file.rs", "Edit", "", "").unwrap();
        assert_eq!(
            store.current_turn_prompt("s", "w2").unwrap().as_deref(),
            Some("[tmm chat] lead: work")
        );
        store.insert_activity("s", "w2", 1300, "notif", "completed", "", "", "").unwrap();
        assert_eq!(store.current_turn_prompt("s", "w2").unwrap(), None);
    }

    /// Paging backwards through a complete log (board #9). The cursor is (ts, id)
    /// because a busy turn writes several events inside ONE millisecond: a
    /// ts-only cursor either skips them or loops on them for ever.
    #[test]
    fn the_activity_log_pages_backwards_without_losing_a_millisecond_tie() {
        let store = Store::open_memory().unwrap();
        // Six events, and three of them share ts 1002 — the shape a real turn has.
        for (n, ts) in [1000u64, 1001, 1002, 1002, 1002, 1003].into_iter().enumerate() {
            store.insert_activity("s", "w1", ts, "tool", &format!("e{n}"), "Edit", "", "").unwrap();
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

    /// The same walk at VOLUME, past the per-page ceiling: 2500 events, pages of
    /// 250. A page cap is not a history horizon — the trace is kept whole, so a
    /// client must be able to reach all of it, and reach each row exactly once.
    #[test]
    fn the_activity_log_walks_past_the_page_ceiling_without_a_gap() {
        let mut store = Store::open_memory().unwrap();
        const TOTAL: usize = 2500;
        let tx = store.conn.transaction().unwrap();
        for n in 0..TOTAL {
            // Deliberately coarse timestamps: 5 events per millisecond, so the
            // walk is forced through same-ms ties on nearly every page boundary.
            tx.execute(
                "INSERT INTO activity (session, window, ts, kind, text, tool, via, state)
                 VALUES ('s', 1, ?1, 'tool', ?2, 'Edit', '', '')",
                rusqlite::params![1_000 + (n as i64) / 5, format!("e{n}")],
            )
            .unwrap();
        }
        tx.commit().unwrap();
        assert_eq!(store.activity_stats("s").unwrap().0, TOTAL);

        let mut cursor: Option<(u64, i64)> = None;
        let mut seen: Vec<String> = Vec::new();
        let mut pages = 0;
        loop {
            let (page, has_more) = store.activity_page("s", 0, cursor, 250).unwrap();
            if page.is_empty() {
                break;
            }
            pages += 1;
            assert!(page.len() <= 250);
            // Oldest first, and strictly older than the cursor we came from.
            assert!(page.windows(2).all(|w| (w[0].ts, w[0].id) < (w[1].ts, w[1].id)));
            if let Some((cts, cid)) = cursor {
                let last = page.last().unwrap();
                assert!((last.ts, last.id) < (cts, cid), "page {pages} overlapped its cursor");
            }
            cursor = Some((page[0].ts, page[0].id));
            let mut older: Vec<String> = page.into_iter().map(|r| r.text).collect();
            older.append(&mut seen);
            seen = older;
            if !has_more {
                break;
            }
        }
        assert_eq!(pages, 10, "2500 rows in pages of 250");
        assert_eq!(seen.len(), TOTAL, "every event exactly once — no duplicate, no hole");
        let expected: Vec<String> = (0..TOTAL).map(|n| format!("e{n}")).collect();
        assert_eq!(seen, expected, "and the walk reassembles the log in order");
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
    /// Board #124: the match moved into SQL `LIKE`, and a literal `%` or `_`
    /// in the query must stay literal — unescaped they are wildcards, and
    /// "50%" would match "50x" while "r_s" matched any "rXs".
    #[test]
    fn hub_search_escapes_like_wildcards() {
        let store = Store::open_memory().unwrap();
        store.hub_append("proj:esc", "e1", 10, "human", "[]", "msg", "take 50% off today").unwrap();
        store.hub_append("proj:esc", "e2", 20, "human", "[]", "msg", "take 50x off today").unwrap();
        store.hub_append("proj:esc", "e3", 30, "human", "[]", "msg", "an under_score name").unwrap();
        store.hub_append("proj:esc", "e4", 40, "human", "[]", "msg", "an underXscore name").unwrap();
        let hits = |term: &str| {
            store.hub_search(Some("proj:esc"), &[term.to_string()], 50).unwrap()
                .into_iter().map(|m| m.id).collect::<Vec<_>>()
        };
        assert_eq!(hits("50%"), vec!["e1"], "a literal % is not a wildcard");
        assert_eq!(hits("under_s"), vec!["e3"], "a literal _ is not a wildcard");
        assert_eq!(hits("UNDER_S"), vec!["e3"], "ASCII case-insensitive, as before");
        assert!(hits("zzz").is_empty());
    }

}
