//! The project hub's chat rooms (board #107).
//!
//! One room per project (`proj:<session>`, or the room recorded on the
//! project row — see `server/hub_rpc::project_room`). Messages live in
//! state.db (`hub_msgs`), one of the two truth stores (tenet 7); the agora
//! bus that used to hold them was deleted whole with the desktop Team system
//! (owner 2026-09-09, board #100). Rooms are implicit: a room exists exactly
//! when it has messages — there is no register/open step, because a registry
//! separate from the data it indexes is a second source of truth.
//!
//! Message JSON keeps the exact wire shape the bus produced —
//! `{seq, id, ts, room, from, to, kind, body}` — so clients, `tmm log`
//! cursors and the team-context route reconstruction are untouched.
//!
//! The push half is one process-wide broadcast channel: every append is
//! re-serialized and fanned out to each connection's push loop (the
//! `team_message` WS frame, name kept for client compatibility). The
//! obligation graph the bus kept (`requires_reply`) is gone with it — the
//! hub never read it; reply behaviour is owned by hooks and the single
//! reply edge (tenet: derive, never declare).

use std::sync::OnceLock;
use tokio::sync::broadcast;

use super::store::HubMsg;
use super::{now, with_store};

/// Merged message fan-out for the WS push path (all rooms). 1024 deep like
/// the bus channel it replaces; a lagged receiver re-syncs via hub_log.
fn channel() -> &'static broadcast::Sender<String> {
    static TX: OnceLock<broadcast::Sender<String>> = OnceLock::new();
    TX.get_or_init(|| broadcast::channel(1024).0)
}

/// A receiver of newly-appended messages across ALL rooms, each pre-serialized
/// to JSON (the `room` field is inside). The client filters to the room in view.
pub fn subscribe() -> broadcast::Receiver<String> {
    channel().subscribe()
}

fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// The stored row in the wire shape every consumer already speaks.
fn message_json(m: &HubMsg) -> serde_json::Value {
    let to: serde_json::Value =
        serde_json::from_str(&m.to_json).unwrap_or_else(|_| serde_json::json!([]));
    serde_json::json!({
        "seq": m.seq,
        "id": m.id,
        "ts": m.ts,
        "room": m.room,
        "from": m.sender,
        "to": to,
        "kind": m.kind,
        "body": m.body,
    })
}

/// Post with server-known recipients stored in the message envelope — the
/// durable route `team_context` reconstruction reads. Returns the stored
/// message JSON and broadcasts it to every connected client.
pub fn post_routed(
    room: &str,
    from: &str,
    body: &str,
    to: &[String],
) -> Result<serde_json::Value, String> {
    let id = uuid::Uuid::new_v4().to_string();
    let to_json = serde_json::to_string(to).unwrap_or_else(|_| "[]".into());
    let stored = with_store(|s| s.hub_append(room, &id, now_ms(), from, &to_json, "msg", body))?;
    let json = message_json(&stored);
    let _ = channel().send(json.to_string());
    Ok(json)
}

/// Post without recipients (lifecycle `[tmm] …` lines, status rows).
pub fn post(room: &str, from: &str, body: &str) -> Result<serde_json::Value, String> {
    post_routed(room, from, body, &[])
}

/// One PAGE of `room`'s messages, oldest first, walking backwards:
/// `{ messages, has_more, head_seq }`. `before_seq` is exclusive; `None` is
/// the newest page.
pub fn history_page(room: &str, before_seq: Option<i64>, limit: i64) -> serde_json::Value {
    let (msgs, has_more, head_seq) =
        with_store(|s| s.hub_page(room, before_seq, limit)).unwrap_or((Vec::new(), false, 0));
    serde_json::json!({
        "messages": msgs.iter().map(message_json).collect::<Vec<_>>(),
        "has_more": has_more,
        "head_seq": head_seq,
    })
}

/// ONE message by its id, however old — exact lookup, never a page scan.
pub fn message_by_id(room: &str, id: &str) -> Option<serde_json::Value> {
    with_store(|s| s.hub_message_by_id(room, id))
        .ok()
        .flatten()
        .map(|m| message_json(&m))
}

/// The newest `limit` messages matching ANY of `terms`, oldest first:
/// `{ "messages": [...] }`. `room = None` searches EVERY room; each hit's
/// `room` field says where it was said.
pub fn search_messages(room: Option<&str>, terms: &[String], limit: i64) -> serde_json::Value {
    let msgs = with_store(|s| s.hub_search(room, terms, limit)).unwrap_or_default();
    serde_json::json!({ "messages": msgs.iter().map(message_json).collect::<Vec<_>>() })
}

/// Newest message timestamp (ms) per room: `{ "<room>": ts }` — orders the
/// project list, so it covers every room that ever spoke.
pub fn room_latest() -> serde_json::Value {
    let map: serde_json::Map<String, serde_json::Value> = with_store(|s| s.hub_room_latest())
        .unwrap_or_default()
        .into_iter()
        .map(|(room, ts)| (room, serde_json::json!(ts)))
        .collect();
    serde_json::Value::Object(map)
}

/// Forget messages by id, for good — the irreversible half of deleting
/// (`msg_archive` is the reversible half).
pub fn delete_messages(room: &str, ids: &[String]) -> Result<usize, String> {
    with_store(|s| s.hub_delete(room, ids))
}

/// Test seeding with a controlled clock (the paging and `since_ts` contracts
/// in `server/hub_rpc` need deterministic timestamps). Returns the stored
/// message JSON; does NOT broadcast.
#[cfg(test)]
pub(crate) fn seed_msg(
    room: &str,
    id: &str,
    ts: i64,
    from: &str,
    to: &[String],
    body: &str,
) -> serde_json::Value {
    let to_json = serde_json::to_string(to).unwrap_or_else(|_| "[]".into());
    let stored =
        with_store(|s| s.hub_append(room, id, ts, from, &to_json, "msg", body)).expect("seed");
    message_json(&stored)
}

// ---- one-off legacy import (board #107) -----------------------------------

const IMPORT_FLAG: &str = "hub_import_done";

/// Where the retired Team bus kept its database. The `TEAM_DB` env var and the
/// `team_db` config key (with its `crew_db`/`agora_db` pre-rebrand aliases) are
/// read RAW here rather than through `config::Config`, because the Team system
/// and its config surface are deleted — this importer is the one consumer left,
/// and it must keep finding a legacy custom path after that deletion.
fn legacy_db_path() -> std::path::PathBuf {
    if let Some(p) = std::env::var_os("TEAM_DB").filter(|p| !p.is_empty()) {
        return std::path::PathBuf::from(p);
    }
    let config = crate::config::config_dir().join("config.toml");
    if let Ok(text) = std::fs::read_to_string(&config) {
        if let Ok(value) = text.parse::<toml::Value>() {
            for key in ["team_db", "crew_db", "agora_db"] {
                if let Some(path) = value.get(key).and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                    return std::path::PathBuf::from(path);
                }
            }
        }
    }
    crate::config::config_dir().join("team.db")
}

/// Import every `proj:*` room's transcript out of the legacy team.db, exactly
/// once (a `meta` flag remembers). The file itself is left on disk, unread
/// from then on — the room is the only record, and losing chat history to a
/// storage migration is not acceptable. Called at server startup; every miss
/// (no file, no flag write) is fail-soft.
pub fn import_legacy() {
    let already = with_store(|s| s.meta_get(IMPORT_FLAG)).ok().flatten().is_some();
    if already {
        return;
    }
    let legacy = legacy_db_path();
    if legacy.is_file() {
        match with_store(|s| s.hub_import_from(&legacy)) {
            Ok(n) => println!("🜂 hub: imported {n} legacy messages from {}", legacy.display()),
            Err(e) => {
                // Leave the flag unset so the next start retries: an import
                // that failed halfway is re-run, and INSERT OR IGNORE keeps
                // the retry additive.
                eprintln!("⚠️  hub: legacy import failed: {e}");
                return;
            }
        }
    }
    let _ = with_store(|s| s.meta_set(IMPORT_FLAG, "1"));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_posted_message_keeps_the_bus_wire_shape_and_pages_backwards() {
        crate::projects::tests::use_test_store();
        let room = format!("proj:rooms-{}", uuid::Uuid::new_v4());
        let mut rx = subscribe();
        let m1 = post_routed(&room, "human", "@lead start", &["lead".into()]).unwrap();
        // The wire shape is the bus's: from/to/kind/seq/id/ts/room/body.
        assert_eq!(m1["from"], "human");
        assert_eq!(m1["to"], serde_json::json!(["lead"]));
        assert_eq!(m1["kind"], "msg");
        assert_eq!(m1["room"], room);
        assert!(m1["seq"].as_i64().unwrap() > 0);
        assert!(!m1["id"].as_str().unwrap().is_empty());
        // The append was pushed to subscribers, pre-serialized.
        let pushed: serde_json::Value =
            serde_json::from_str(&rx.try_recv().expect("a push frame")).unwrap();
        assert_eq!(pushed, m1);

        let m2 = post(&room, "lead", "done").unwrap();
        assert_eq!(m2["to"], serde_json::json!([]));

        // Newest page, oldest first; head_seq names the room's newest.
        let page = history_page(&room, None, 100);
        let msgs = page["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["body"], "@lead start");
        assert_eq!(page["has_more"], false);
        assert_eq!(page["head_seq"], m2["seq"]);

        // Walking backwards: the page behind m2 is m1, and the walk ends there.
        let back = history_page(&room, m2["seq"].as_i64(), 1);
        assert_eq!(back["messages"].as_array().unwrap().len(), 1);
        assert_eq!(back["messages"][0]["id"], m1["id"]);
        assert_eq!(back["has_more"], false);

        // Exact lookup, search, room ordering, deletion.
        assert_eq!(message_by_id(&room, m1["id"].as_str().unwrap()).unwrap(), m1);
        let hits = search_messages(Some(&room), &["START".into()], 10);
        assert_eq!(hits["messages"].as_array().unwrap().len(), 1, "case-insensitive body match");
        let latest = room_latest();
        assert_eq!(latest[&room], m2["ts"], "the room's newest ts orders the sidebar");
        assert_eq!(delete_messages(&room, &[m1["id"].as_str().unwrap().to_string()]).unwrap(), 1);
        assert_eq!(history_page(&room, None, 100)["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn legacy_import_copies_proj_rooms_once_and_skips_team_rooms() {
        crate::projects::tests::use_test_store();
        let dir = std::env::temp_dir().join(format!("tmm-import-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let legacy = dir.join("team.db");
        {
            let conn = rusqlite::Connection::open(&legacy).unwrap();
            conn.execute_batch(
                "CREATE TABLE messages (
                   seq INTEGER PRIMARY KEY AUTOINCREMENT, id TEXT NOT NULL UNIQUE,
                   ts INTEGER NOT NULL, room TEXT NOT NULL, sender TEXT NOT NULL,
                   to_json TEXT NOT NULL, kind TEXT NOT NULL, body TEXT NOT NULL
                 );
                 INSERT INTO messages (seq, id, ts, room, sender, to_json, kind, body) VALUES
                   (1, 'a', 100, 'proj:blog', 'human', '[\"lead\"]', 'msg', 'old task'),
                   (2, 'b', 200, 'content-team', 'manager', '[]', 'msg', 'team room stays'),
                   (3, 'c', 300, 'proj:blog', 'lead', '[]', 'msg', 'old reply');",
            )
            .unwrap();
        }
        let n = crate::projects::with_store(|s| s.hub_import_from(&legacy)).unwrap();
        assert_eq!(n, 2, "proj:* rooms only");
        let page = history_page("proj:blog", None, 10);
        let msgs = page["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        // Seq/id/ts survive the copy — cursors held by clients stay valid.
        assert_eq!(msgs[0]["seq"], 1);
        assert_eq!(msgs[1]["seq"], 3);
        assert_eq!(msgs[0]["id"], "a");
        // The team room did not come along.
        assert!(history_page("content-team", None, 10)["messages"].as_array().unwrap().is_empty());
        // A retry is additive-idempotent (INSERT OR IGNORE), never a duplicate.
        let again = crate::projects::with_store(|s| s.hub_import_from(&legacy)).unwrap();
        assert_eq!(again, 0);
        // New appends continue ABOVE the imported log positions.
        let fresh = post("proj:blog", "human", "new work").unwrap();
        assert!(fresh["seq"].as_i64().unwrap() > 3);
        let _ = std::fs::remove_dir_all(&dir);
    }
}

// ---- archived messages (moved from projects/mod.rs, board #152) ----------
//
// The archive is OUR state (state.db): hiding a message is this app's idea.
// These are the four verbs the hub RPCs need, each fail-soft in the direction
// that keeps the UI honest — a read that fails hides nothing, a write that
// fails is reported.

/// Ids hidden in a room. A failure here must not blank the conversation, so it
/// degrades to "nothing is hidden".
pub fn archived_ids(room: &str) -> Vec<String> {
    with_store(|s| s.archived_ids(room)).unwrap_or_default()
}

/// The archive itself, newest first, each row carrying its own copy of the
/// message.
pub fn archived_msgs(room: &str) -> Vec<(String, u64, String, String, u64)> {
    with_store(|s| s.archived_msgs(room)).unwrap_or_default()
}

/// Hide one message.
pub fn archive_msg(room: &str, msg_id: &str, ts: u64, sender: &str, body: &str) -> Result<(), String> {
    with_store(|s| s.archive_msg(room, msg_id, ts, sender, body, now()))
}

/// Take messages out of the archive — a restore, or the bookkeeping half of a
/// purge once the messages themselves are gone.
pub fn unarchive_msgs(room: &str, ids: &[String]) -> Result<usize, String> {
    with_store(|s| s.unarchive_msgs(room, ids))
}
