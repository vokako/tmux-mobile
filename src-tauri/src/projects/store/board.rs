//! The task board: issues, their notes and per-project numbering.
//!
//! One family of the store (board #147): an `impl Store` block over the same
//! connection, the same migration ladder, the same tests — only the file moved.

use super::*;

impl Store {
    /// Allocate one visible number atomically for a session. The sequence is
    /// advanced before the issue INSERT; a failed insert may leave a gap (like
    /// AUTOINCREMENT) but can never reuse a number that was already observed.
    pub(super) fn next_issue_number(&self, session: &str) -> Result<i64, String> {
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
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
