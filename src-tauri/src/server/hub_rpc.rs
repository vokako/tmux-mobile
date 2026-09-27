//! hub_* RPC dispatch: the project hub (agents-v2). One chat room per project
//! (bus room `proj:<session>`), agent status declarations, and derived agent
//! states. This is the server side of the `tmm` CLI — the CLI-only message
//! substrate from docs/exec-plans/agents-v2.md (§4.1/§4.4): what an agent SAYS
//! arrives here via tmm; what we OBSERVE arrives via hooks into
//! projects::telemetry, and `hub_agents` joins the two at read time.
//!
//! Desktop-only in effect: everything needs state.db (the projects module,
//! desktop-gated — messages live in its `hub_msgs` table since board #107),
//! so mobile answers method-not-found and clients degrade gracefully.

use super::rpc::{param, Request, Response, RpcError};
#[cfg(any(target_os = "android", target_os = "ios"))]
use super::rpc::ERR_METHOD_NOT_FOUND;

/// Bus room for a project's hub chat.
///
/// Recorded ON THE PROJECT (schema v8) rather than derived from the session
/// name, because the session name can now change: renaming a project renames its
/// tmux session, and a room id derived from it would have left the conversation
/// behind. `proj:<session>` stays the FALLBACK — it is what every room created
/// before v8 is called, and what an untracked session gets.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) fn project_room(session: &str) -> String {
    crate::projects::project_for_session(session)
        .ok()
        .flatten()
        .map(|p| p.room)
        .filter(|r| !r.is_empty())
        .unwrap_or_else(|| format!("proj:{session}"))
}

/// The hub dispatcher proper (board #146): every arm is a `Result`, so a
/// missing param or a store error is a `?` with its wire code already chosen
/// (`RpcError`), and `handle_hub_request` below is the one place a
/// `Response` is built (`Response::from_outcome`). Arm order and every message are exactly what the
/// inline `match … return Response::err` shape produced.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn dispatch_hub(req: &Request, _notifications: Option<&crate::agent_notifications::AgentNotificationHub>) -> Result<serde_json::Value, RpcError> {
    use crate::projects::rooms;
    use crate::projects::telemetry;

    let p = &req.params;
    // The one method that is about EVERY room: when did we last talk in each?
    // It answers before the session gate below, because it has no session to
    // resolve — the sidebar asks it once to order the whole project list.
    if req.method == "hub_rooms" {
        // The sidebar's summary read: newest message per room AND every
        // hook-known window's derived state, keyed "<session>:<window>". Both
        // are about EVERY project, which is why this answers before the
        // session gate below.
        let mut states = serde_json::Map::new();
        for (s, w, st) in crate::projects::telemetry::all_states() {
            states.insert(format!("{s}:{w}"), serde_json::Value::String(st));
        }
        return Ok(serde_json::json!({ "rooms": crate::projects::rooms::room_latest(), "states": states }));
    }
    // The board twin of `hub_rooms`: issue counts per column for EVERY
    // project's board, one grouped read (board #39) — the Board sidebar
    // shows per-project counts and hides empty boards, and a per-project
    // `hub_board_list` walk is the N+1 this exists to prevent. No `session`
    // param by design, so it answers before the session gate like the other
    // all-rooms reads; same authenticated reader as everything here.
    if req.method == "hub_board_counts" {
        return Ok(crate::projects::board_counts().map_err(RpcError::Internal)?);
    }
    let asked = param(p, "session")?;
    // Resolve the caller's session name to the project's CURRENT one, ONCE, here.
    // Renaming a project renames its tmux session, but a running agent carries
    // `TMM_PROJECT` from the moment it started — and half of these methods reach
    // straight into tmux (`window_of_agent`, `list_panes`), where a stale name
    // finds nothing. Measured the hard way: right after a rename, a tmm call
    // answered "no window named 'builder-2' in session 'tmm-tasks'" — the deaf
    // agent again, one layer below the project lookup that already handled it.
    let current = crate::projects::project_for_session(asked)
        .ok()
        .flatten()
        .map(|proj| proj.session)
        .unwrap_or_else(|| asked.to_string());
    let session: &str = &current;
    let room = project_room(session);

    match req.method.as_str() {
        // Post to the project chat. `from` is the agent name (tmm exports
        // TMM_AGENT) or "human" for the operator.
        // A SLASH COMMAND is for the CLI, not for the model. `/model`, `/clear`,
        // `/compact`, `/tools` are things the agent's TUI interprets, and only
        // when they are the whole line — delivered the normal way, prefixed with
        // `[tmm chat …] human:`, they arrive as ordinary prose and the model
        // answers them instead of the CLI running them (owner, 2026-08-19: "支持
        // /命令 这个直接发送 不加消息时间戳之类的").
        //
        // So this path types the text VERBATIM into the agent's pane: no stamp,
        // no sender, no @address. It is recorded in the room as a lifecycle line
        // (`[tmm] `) rather than a message, because it is an instruction to a
        // program, not something said to a person — and record-only, so the
        // mention scanner never sees it.
        "hub_command" => {
            let agent = param(p, "agent")?;
            let text = param(p, "text")?.trim();
            if !text.starts_with('/') {
                return Err(RpcError::InvalidParams("a command must start with '/'".into()));
            }
            let ws = crate::projects::project_for_session(session).ok().flatten().map(|pr| pr.path);
            let panes = crate::tmux::list_panes(session).unwrap_or_default();
            let mut sent: Vec<String> = Vec::new();
            let mut refused: Vec<String> = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for pane in &panes {
                if !seen.insert(pane.window) || !pane.active {
                    continue;
                }
                if agent != "all" && pane.window_name != agent {
                    continue;
                }
                // Same managed-only gate as delivery: typing into a window the
                // user started by hand is not ours to do, and typing a slash
                // command into a SHELL would execute a stray path.
                if !crate::projects::is_managed_in(ws.as_deref(), &pane.window_name) {
                    continue;
                }
                let target = format!("{}:{}.{}", session, pane.window, pane.pane);
                match crate::tmux::send_command(&target, text) {
                    Ok(()) => sent.push(pane.window_name.clone()),
                    // Board #250: a pane in copy-mode refuses. The same plain
                    // warn as a refused chat line, on THAT window, whatever the
                    // others did: an `all` where one pane took it answers
                    // success, and the refused one must not vanish with it.
                    Err(e) => {
                        crate::projects::telemetry::record_undelivered(session, &pane.window_name, text, e.trim());
                        refused.push(format!("{}: {}", pane.window_name, e.trim()));
                    }
                }
            }
            if sent.is_empty() && !refused.is_empty() {
                return Err(RpcError::Internal(format!("not sent — {}", refused.join("; "))));
            }
            if sent.is_empty() {
                return Err(RpcError::InvalidParams(format!("no managed agent named '{agent}' in session '{session}'")));
            }
            let _ = rooms::post(&room, "human", &format!("[tmm] {} → {}", text, sent.join(", ")));
            Ok(serde_json::json!({ "sent": sent, "command": text }))
        }

        "hub_post" => {
            let raw_body = param(p, "body")?;
            let from = p.get("from").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("human");
            let is_status = p.get("status").and_then(|v| v.as_bool()).unwrap_or(false);
            let body = if is_status {
                format!("[tmm status working] {raw_body}")
            } else {
                raw_body.to_string()
            };
            // record_only = true means the message is stored but NEVER typed
            // into any agent's pane. Required for hook-sourced auto-replies:
            // if an automatic post addresses a peer, delivery would type into
            // that peer's pane, triggering their own stop hook, which then
            // auto-posts back — a ping-pong loop. The caller is responsible
            // for setting this when the origin is a hook.
            let record_only =
                is_status || p.get("record_only").and_then(|v| v.as_bool()).unwrap_or(false);
            let recipients = crate::address::mention_names(&body);
            let msg = rooms::post_routed(&room, from, &body, &recipients).map_err(RpcError::Internal)?;
            // DELIVERY: an idle agent sits at its prompt and reads
            // nothing — @mentions are typed into the mentioned agents'
            // panes so the chat actually reaches them. (An agent that
            // is mid-task sees the line queued in its input box.)
            // Hook-sourced posts skip delivery entirely to prevent
            // reply loops (see record_only comment above).
            if !record_only {
                let seq = msg.get("seq").and_then(|v| v.as_i64());
                let id = msg.get("id").and_then(|v| v.as_str()).unwrap_or_default();
                deliver_mentions(session, from, &body, &room, seq, id);
            }
            Ok(msg)
        }

        // Read the project chat, optionally incremental (`since_ts`, exclusive)
        // — the multica-style cursor so an agent polls without re-reading — or
        // backwards by page (`before_seq`), which is how a client reaches history
        // it never loaded. Nothing is ever pruned from the room, so paging is the
        // only honest way to keep a first load small (board #9).
        "hub_log" => {
            let limit = p.get("limit").and_then(|v| v.as_i64()).unwrap_or(100).clamp(1, 1000);
            let since_ts = p.get("since_ts").and_then(|v| v.as_i64()).unwrap_or(0);
            // The bus's own cursor is `seq`, the message's log position — stable,
            // gapless and already on every message the client holds, which a ts is
            // not (two messages can share a millisecond).
            let before_seq = p.get("before_seq").and_then(|v| v.as_i64()).filter(|n| *n > 0);
            let mut history = rooms::history_page(&room, before_seq, limit);
            // An archived message is hidden, not gone: the room's own store still
            // has it (that is what makes a restore free), so the hiding happens
            // here, on the way out.
            let hidden = crate::projects::archived_ids(&room);
            // Set by the since_ts branch below; false means "rows newer than the
            // cursor may still lie behind this page".
            let mut reached_cursor = true;
            if let Some(msgs) = history.get_mut("messages").and_then(|m| m.as_array_mut()) {
                // The RAW page's oldest position, read BEFORE any filtering. It is
                // the cursor of last resort: a page can lose every row to the
                // filters (one archived stretch, or a `since_ts` tail), and then a
                // survivor-derived cursor does not exist — the client would be told
                // `has_more: true` with nothing to ask for and the walk would stop
                // dead at a hidden run. The raw seq always advances, so the next
                // request lands strictly further back.
                let raw_oldest = msgs.first().and_then(|m| m.get("seq")).and_then(|v| v.as_i64());
                let raw_oldest_ts = msgs.first().and_then(|m| m.get("ts")).and_then(|v| v.as_i64());
                if since_ts > 0 {
                    msgs.retain(|m| m.get("ts").and_then(|t| t.as_i64()).unwrap_or(0) > since_ts);
                    // An incremental poll asks "what is NEWER than since_ts", and
                    // the page is the newest `limit` rows — so when more than
                    // `limit` messages arrived since the cursor, the older ones are
                    // simply not on it. `has_more` then means "newer-than-since_ts
                    // rows remain behind this page": true only while the RAW page
                    // did not reach back to since_ts (2026-09-03, review C — the
                    // client took the page as complete and its cursor jumped past
                    // the rest, a permanent hole in the feed).
                    reached_cursor = raw_oldest_ts.is_none_or(|ts| ts <= since_ts);
                }
                if !hidden.is_empty() {
                    msgs.retain(|m| {
                        !m.get("id").and_then(|v| v.as_str()).is_some_and(|i| hidden.iter().any(|h| h == i))
                    });
                }
                // Prefer a SURVIVING row's seq — a visible message is what the user
                // is looking at, so the next page continues from what they can see —
                // and fall back to the raw one when nothing survived.
                let oldest = msgs
                    .first()
                    .and_then(|m| m.get("seq"))
                    .and_then(|v| v.as_i64())
                    .or(raw_oldest);
                if let Some(obj) = history.as_object_mut() {
                    if let Some(seq) = oldest {
                        obj.insert("oldest_seq".into(), serde_json::json!(seq));
                    }
                }
            }
            if since_ts > 0 {
                if let Some(obj) = history.as_object_mut() {
                    let more = obj.get("has_more").and_then(|v| v.as_bool()).unwrap_or(false);
                    obj.insert("has_more".into(), serde_json::json!(more && !reached_cursor));
                }
            }
            Ok(history)
        }

        // Keyword search over the room's FULL history — `hub_log` pages, this
        // finds (owner, 2026-09-07: "log是不是也加上关键字搜索能力…也可以加一个
        // --global 跨项目会话全局搜索"). `grep` is a term LIST, any-match,
        // substring, ASCII-case-insensitive, against body and sender.
        // `global: true` widens the scope to EVERY room; each hit carries its
        // `room` field so a cross-project answer stays readable. Newest `limit`
        // hits, oldest first, archived messages filtered out per room on the
        // way — same one-way mirror as `hub_log`.
        "hub_search" => {
            let terms: Vec<String> = p
                .get("grep")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
                .unwrap_or_default();
            if terms.iter().all(|t| t.trim().is_empty()) {
                return Err(RpcError::InvalidParams("grep must be a non-empty array of search terms".into()));
            }
            let global = p.get("global").and_then(|v| v.as_bool()).unwrap_or(false);
            let limit = p.get("limit").and_then(|v| v.as_i64()).unwrap_or(50).clamp(1, 500);
            let scope = if global { None } else { Some(room.as_str()) };
            let mut result = rooms::search_messages(scope, &terms, limit);
            if let Some(msgs) = result.get_mut("messages").and_then(|m| m.as_array_mut()) {
                // Archived = hidden everywhere, including from search. The ids
                // are per room, so a global result set asks once per room it
                // actually touched.
                let rooms: std::collections::BTreeSet<String> = msgs
                    .iter()
                    .filter_map(|m| m.get("room").and_then(|v| v.as_str()).map(str::to_string))
                    .collect();
                for r in rooms {
                    let hidden = crate::projects::archived_ids(&r);
                    if hidden.is_empty() {
                        continue;
                    }
                    msgs.retain(|m| {
                        m.get("room").and_then(|v| v.as_str()) != Some(r.as_str())
                            || !m.get("id").and_then(|v| v.as_str()).is_some_and(|i| hidden.iter().any(|h| h == i))
                    });
                }
            }
            Ok(result)
        }

        // Deleting a message is TWO steps, because a transcript is a record and a
        // misclick on a record should be recoverable (owner, 2026-08-19): archive
        // hides it — reversibly, the message never leaves the room's store — and
        // deleting it IN the archive is what forgets it. `hub_msg_purge` is the
        // only one of the three that destroys anything.
        "hub_msg_archive" | "hub_msg_restore" | "hub_msg_purge" => {
            let ids: Vec<String> = p
                .get("ids")
                .and_then(|v| v.as_array())
                .map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                .unwrap_or_default();
            if ids.is_empty() {
                return Err(RpcError::InvalidParams("ids must be a non-empty array".into()));
            }
            match req.method.as_str() {
                "hub_msg_archive" => {
                    // The archive row carries a copy of the message, so the archive
                    // view needs no second lookup and no history window. The bodies
                    // come from the room, not from the client: what gets stored is
                    // what was actually said — and by EXACT id, so a message the
                    // user scrolled back to is as archivable as a fresh one (this
                    // used to scan the newest 1000, which quietly excluded
                    // everything older).
                    let mut done = 0usize;
                    for mid in &ids {
                        let Some(m) = rooms::message_by_id(&room, mid) else { continue };
                        let ok = crate::projects::archive_msg(
                            &room,
                            mid,
                            m.get("ts").and_then(|v| v.as_i64()).unwrap_or(0) as u64,
                            m.get("from").or_else(|| m.get("sender")).and_then(|v| v.as_str()).unwrap_or(""),
                            m.get("body").and_then(|v| v.as_str()).unwrap_or(""),
                        );
                        if ok.is_ok() {
                            done += 1;
                        }
                    }
                    Ok(serde_json::json!({ "archived": done }))
                }
                "hub_msg_restore" => {
                    let n = crate::projects::unarchive_msgs(&room, &ids).map_err(RpcError::Internal)?;
                    Ok(serde_json::json!({ "restored": n }))
                },
                _ => {
                    // Forget the message itself first: if that fails the archive row
                    // stays, so the message is still listed and can be tried again.
                    let n = rooms::delete_messages(&room, &ids).map_err(RpcError::Internal)?;
                    let _ = crate::projects::unarchive_msgs(&room, &ids);
                    Ok(serde_json::json!({ "deleted": n }))
                }
            }
        }

        // What is hidden in this room, newest first. Self-contained rows: the
        // archive is a list you review before forgetting anything.
        "hub_archive" => {
            let rows: Vec<serde_json::Value> = crate::projects::archived_msgs(&room)
                .into_iter()
                .map(|(id, ts, sender, body, at)| {
                    serde_json::json!({ "id": id, "ts": ts, "from": sender, "body": body, "archived_at": at })
                })
                .collect();
            Ok(serde_json::json!({ "messages": rows }))
        }

        // Derived agent states for a session: one row per live window, agent
        // detection + status derivation joined at read time.
        "hub_agents" => Ok(agent_states(session)),

        // ---- the project task board (owner, 2026-08-29): the human writes
        // issues on the board page, agents read/update through `tmm board`.
        // Session-scoped like the chat room; note/move/save record WHO acted.
        "hub_board_list" => Ok(crate::projects::board_list(session).map_err(RpcError::InvalidParams)?),
        "hub_board_get" => {
            let Some(issue_id) = p.get("id").and_then(|v| v.as_i64()) else {
                return Err(RpcError::InvalidParams("id required".into()));
            };
            Ok(crate::projects::board_get(session, issue_id).map_err(RpcError::InvalidParams)?)
        }
        "hub_board_save" => {
            let issue_id = p.get("id").and_then(|v| v.as_i64());
            let s = |k: &str| p.get(k).and_then(|v| v.as_str());
            let who = s("who").unwrap_or("human");
            // The board and the agents' live states are TWO AXES (an issue's
            // lifecycle vs a window's turn), joined at EVENTS, not merged: a
            // status change is recorded in the room so the flow is visible,
            // and a move to REVIEW is a HANDOFF — the reporter (created_by)
            // gets the line typed into its pane, because a review nobody is
            // told about is how issues rot in the third column (owner,
            // 2026-08-29: the ideal loop is 人填 issue → lead 派 → 子 agent
            // 完成交 review → 通过标 done). Reporter "human" reads the board
            // itself; the actor is never notified of its own move.
            let prev = issue_id.and_then(|iid| crate::projects::board_get(session, iid).ok());
            let saved = crate::projects::board_save(session, issue_id, s("title"), s("body"), s("status"), s("assignee"), who).map_err(RpcError::InvalidParams)?;
            if let (Some(prev), Some(new_status)) = (&prev, s("status")) {
                let old_status = prev["status"].as_str().unwrap_or("");
                if old_status != new_status {
                    // Titles are optional (board #31): every surface
                    // that NAMES an issue speaks issue_ref's fallback
                    // (title → body excerpt → #id), so a titleless
                    // issue never renders an empty head or a dangling
                    // separator.
                    let title = crate::projects::issue_ref(
                        prev["title"].as_str().unwrap_or(""),
                        prev["body"].as_str().unwrap_or(""),
                        saved,
                    );
                    let _ = rooms::post(&room, who, &format!("[tmm] board #{saved} {old_status} → {new_status} — {title}"));
                    if new_status == "review" {
                        let reporter = prev["created_by"].as_str().unwrap_or("");
                        if !reporter.is_empty() && reporter != "human" && reporter != who {
                            // The handoff CARRIES the mover's last note —
                            // their own account of what was done — so the
                            // reviewer can usually decide from the message
                            // (owner, 2026-08-30: concise, no busywork).
                            let last_note = prev["notes"].as_array()
                                .and_then(|n| n.last())
                                .and_then(|n| n["body"].as_str())
                                .map(|b| format!(" — {}", excerpt(b, NOTICE_EXCERPT)))
                                .unwrap_or_default();
                            let line = format!(
                                "[tmm chat {}] {who}: [board #{saved} review] {title}{last_note}. `tmm board move {saved} done` to accept, or note what to fix + move doing.",
                                stamp_now()
                            );
                            deliver_chat_line(session, reporter, &line);
                        }
                    }
                }
            }
            // A change SOMEBODY ELSE made to your issue is only real
            // once you hear about it (owner, 2026-08-30: "不然这个更
            // 改就没有起任何作用。消息就是发给被 assign 的人"): the
            // ASSIGNEE gets the change typed into its pane. The pure
            // half (`board_change_notice`) decides; skips are part of
            // its contract — the actor never hears its own edit, an
            // unassigned issue and the human assignee have nobody to
            // wake, and a save that CHANGES the assignee is a
            // (re)assignment with its own dispatch channel (the UI
            // @message), where a second line would be noise.
            if let Some(prev) = &prev {
                if let Some(what) = board_change_notice(prev, who, s("title"), s("body"), s("status"), s("assignee").is_some()) {
                    let assignee = prev["assignee"].as_str().unwrap_or("");
                    // The head names the issue as it NOW reads (new
                    // values win), through the same issue_ref fallback.
                    let title = crate::projects::issue_ref(
                        s("title").unwrap_or(prev["title"].as_str().unwrap_or("")),
                        s("body").unwrap_or(prev["body"].as_str().unwrap_or("")),
                        saved,
                    );
                    // The change itself travels in the line (values, not
                    // "something changed"); the `…` in a long excerpt is
                    // the one signal that `tmm board show` has more.
                    let line = format!(
                        "[tmm chat {}] {who}: [board #{saved}] {title}: {what}",
                        stamp_now()
                    );
                    deliver_chat_line(session, assignee, &line);
                }
            }
            Ok(serde_json::json!({ "ok": true, "id": saved }))
        }
        "hub_board_note" => {
            let Some(issue_id) = p.get("id").and_then(|v| v.as_i64()) else {
                return Err(RpcError::InvalidParams("id required".into()));
            };
            let body = p.get("body").and_then(|v| v.as_str()).unwrap_or("");
            let author = p.get("who").and_then(|v| v.as_str()).unwrap_or("human");
            crate::projects::board_note(session, issue_id, author, body).map_err(RpcError::InvalidParams)?;
            // A Board reply is communication, not just storage (board
            // #26): after the note is durable, wake the issue's current
            // assignee with the same targeted pane delivery/receipt path
            // used by review handoffs. Every miss is fail-soft — an
            // unassigned/human/self-owned issue has nobody to notify,
            // and an offline or unmanaged target reads the persisted
            // thread later instead of turning a successful note into an
            // RPC failure.
            if let Ok(issue) = crate::projects::board_get(session, issue_id) {
                if let Some((assignee, notice)) = board_note_notice(&issue, author, body) {
                    let line = format!("[tmm chat {}] {author}: {notice}", stamp_now());
                    deliver_chat_line(session, &assignee, &line);
                }
            }
            Ok(serde_json::json!({ "ok": true }))
        }
        "hub_board_delete" => {
            let Some(issue_id) = p.get("id").and_then(|v| v.as_i64()) else {
                return Err(RpcError::InvalidParams("id required".into()));
            };
            match crate::projects::board_delete(session, issue_id) {
                Ok(true) => Ok(serde_json::json!({ "ok": true })),
                Ok(false) => Err(RpcError::InvalidParams(format!("no issue #{issue_id} on this board"))),
                Err(e) => Err(RpcError::InvalidParams(e)),
            }
        }

        // The activity feed: recent observed telemetry events (tool calls,
        // status declarations, notifications) for the chat timeline. The durable
        // log keeps EVERYTHING (board #9), so this read is the bounded half:
        // newest page by default, `before_ts`/`before_id` walks backwards, and
        // `limit` is capped server-side however loudly a client asks.
        //
        // An older client sends only `since_ts` and still gets exactly what it
        // got before: the newest page, oldest first, under the same default cap.
        "hub_activity" => {
            let since_ts = p.get("since_ts").and_then(|v| v.as_u64()).unwrap_or(0);
            let limit = p
                .get("limit")
                .and_then(|v| v.as_u64())
                .map(|n| n as usize)
                .unwrap_or(telemetry::LOAD_EVENTS)
                .clamp(1, telemetry::MAX_PAGE_EVENTS);
            // The cursor is (ts, id): several events share one millisecond inside
            // a busy turn, so ts alone cannot address a position in the log. The
            // server hands `oldest: {ts, id}` back with every page, so a client
            // should always have the exact pair. Omitting `before_id` falls back
            // to "everything strictly older than that whole millisecond": it can
            // skip same-millisecond siblings the client had not received, but it
            // always makes PROGRESS, and a cursor that can loop for ever is the
            // worse failure for a scroll-to-load.
            let before = p
                .get("before_ts")
                .and_then(|v| v.as_u64())
                .map(|ts| (ts, p.get("before_id").and_then(|v| v.as_i64()).unwrap_or(0)));
            // A client asking for the feed is exactly when an undelivered line
            // matters, so account for the ones that timed out before reading.
            // Only on the LIVE page: a walk back through history must not make
            // the app warn about deliveries again.
            if before.is_none() {
                telemetry::sweep_deliveries(session);
            }
            let (events, has_more) = telemetry::events_page(session, since_ts, before, limit);
            // The oldest row of this page IS the cursor for the next one, handed
            // back so a client never has to reconstruct it.
            let oldest = events.first().map(|e| serde_json::json!({ "ts": e.ts, "id": e.id }));
            let (total, first_ts, _last_ts) = telemetry::events_stats(session);
            Ok(serde_json::json!({
                    "events": events,
                    "has_more": has_more,
                    "oldest": oldest,
                    "total": total,
                    "first_ts": first_ts,
                }))
        }

        // Stop / restart ONE agent. The window is the agent's life: killing it
        // ends the process and keeps the declaration, so `restart` is kill +
        // `projects::up_agent`, which recreates THAT slot's window from its
        // recipe and prefers the resume flags — the agent comes back to its
        // own conversation rather than to a blank prompt. Never the project-
        // wide `up`: that resumed every other stopped agent too (board #210).
        // Managed-only: we stop what we started.
        // Interrupt: type Escape into the agent's own pane — the only channel
        // that reaches a BUSY agent, since a chat message is read between
        // turns. Named key, never a raw \x1b: with extended-keys on, tmux
        // drops raw C0 bytes sent to a pane in extended mode. Server-side so
        // the CLI and the UI share ONE implementation.
        "hub_agent_interrupt" => {
            let agent = param(p, "agent")?;
            if crate::projects::managed_home(session, agent).is_none() {
                return Err(RpcError::InvalidParams(format!("'{agent}' is not an agent this app started")));
            }
            let Some(window) = window_of_agent(session, agent) else {
                return Err(RpcError::InvalidParams(format!("no window named '{agent}' in session '{session}'")));
            };
            // Reset the derived state BEFORE the key goes in, never after. A
            // cancelled turn produces no stop hook, so without this the newest
            // fact stays the `userPromptSubmit` that opened it and the card
            // reads `running` for ever; and an interrupted agent usually starts
            // something else within seconds, whose own turn re-derives
            // `running` — so a reset that raced the next turn would be
            // indistinguishable from no reset at all, i.e. an interrupt that
            // looked like it never landed (owner, 2026-08-29).
            telemetry::record_interrupt(session, agent);
            crate::tmux::send_keys(&format!("{session}:{window}"), "Escape", false).map_err(RpcError::Internal)?;
            // The room records what the app did on a person's behalf —
            // same rule as stop/restart/remove. The client's sys
            // grammar already speaks `interrupted` (amber: a turn was
            // cut short, not an ending); the feed row was the missing
            // half of the composer's interrupt affordance (owner,
            // 2026-08-24: "发送 interrupt 的状态在消息列表里也要展示").
            let _ = rooms::post(&room, agent, &format!("[tmm] interrupted {agent}"));
            Ok(serde_json::json!({ "interrupted": agent }))
        }

        // Eject an agent from the project: stop it, drop its slot, remove its
        // isolated home. Stop is the pause button, this is the delete button.
        "hub_agent_remove" => {
            let agent = param(p, "agent")?;
            let v = crate::projects::agent_remove(session, agent).map_err(RpcError::InvalidParams)?;
            let _ = rooms::post(&room, agent, &format!("[tmm] removed {agent}"));
            Ok(v)
        }

        "hub_agent_stop" | "hub_agent_restart" => {
            let agent = param(p, "agent")?;
            if crate::projects::managed_home(session, agent).is_none() {
                return Err(RpcError::InvalidParams(format!("'{agent}' is not an agent this app started")));
            }
            let restart = req.method == "hub_agent_restart";
            let live = window_of_agent(session, agent);
            // Stop needs something to stop. Restart does not: the isolated home
            // outlives the window, so `restart` doubles as "start it again"
            // after a stop — which is what the button does when it reads Start.
            match (live, restart) {
                (None, false) => {
                    return Err(RpcError::InvalidParams(format!("no window named '{agent}' in session '{session}'")));
                }
                (Some(window), _) => {
                    if let Err(e) = crate::tmux::kill_window(&format!("{session}:{window}")) {
                        return Err(RpcError::Internal(e));
                    }
                }
                (None, true) => {}
            }
            if !restart {
                let _ = rooms::post(&room, agent, &format!("[tmm] stopped {agent}"));
                return Ok(serde_json::json!({ "stopped": agent }));
            }
            // Recreate THIS slot from the declaration. A window younger than
            // the capture loop's 120 s rule may not be in it yet, so fall back
            // to a fresh spawn — that starts a new conversation instead of
            // resuming one, which is still better than an agent that does not
            // come back.
            let mut resumed = false;
            if let Ok(Some(project)) = crate::projects::project_for_session(session) {
                // Bring the agent's materials up to date with the CURRENT
                // definition + app-wide AGENTS.md first: the recipe replay is
                // verbatim, so whatever is on disk now is what the agent will
                // be. `refresh_agent` re-materializes prompt/config/recipe
                // when the window name resolves to a registry def; when it
                // cannot (a uniquified teammate, a team-role synthetic), the
                // hooks refresh below still repairs observation, exactly as
                // before.
                if !crate::projects::spawn::refresh_agent(&project.path, session, agent) {
                    crate::projects::spawn::refresh_hooks(&project.path, agent);
                }
                resumed = crate::projects::up_agent(&project.id, agent).unwrap_or(false)
                    && window_of_agent(session, agent).is_some();
            }
            if !resumed {
                let r = crate::projects::spawn::spawn(&crate::projects::spawn::SpawnRequest {
                    session, agent, brief: "", by: "", resume: true, ..Default::default()
                });
                if let Err(e) = r {
                    return Err(RpcError::Internal(format!("restart failed: {e}")));
                }
            }
            let _ = rooms::post(&room, agent, &format!("[tmm] restarted {agent}"));
            Ok(serde_json::json!({ "restarted": agent, "resumed": resumed }))
        }

        // Spawn a registry agent into this project (tmm spawn / the UI's
        // "+ agent"). Capped per project; no per-definition hiring gate.
        // A configured TEAM, started at once (board #74): every member spawns
        // as an ordinary managed agent; the room records one `spawned` line
        // per member, in the grammar the client already reads.
        "hub_spawn_team" => {
            let team = param(p, "team")?;
            let brief = p.get("brief").and_then(|v| v.as_str()).unwrap_or("");
            let by = p.get("by").and_then(|v| v.as_str()).unwrap_or("");
            let result = crate::projects::spawn::spawn_team(session, team, brief, by).map_err(RpcError::InvalidParams)?;
            let who = if by.is_empty() { "human" } else { by };
            let empty = Vec::new();
            for m in result.get("spawned").and_then(|v| v.as_array()).unwrap_or(&empty) {
                let win = m.get("window_name").and_then(|v| v.as_str()).unwrap_or("");
                let line = if brief.is_empty() {
                    format!("[tmm] spawned {win} — team {team}")
                } else {
                    format!("[tmm] spawned {win} — team {team}: {brief}")
                };
                let _ = rooms::post(&room, who, &line);
            }
            Ok(result)
        }
        "hub_spawn" => {
            let agent = param(p, "agent")?;
            let brief = p.get("brief").and_then(|v| v.as_str()).unwrap_or("");
            let by = p.get("by").and_then(|v| v.as_str()).unwrap_or("");
            let result = crate::projects::spawn::spawn(&crate::projects::spawn::SpawnRequest {
                session, agent, brief, by, ..Default::default()
            })
            .map_err(RpcError::InvalidParams)?;
            // The spawn is chat-visible: the room is the record.
            let who = if by.is_empty() { "human" } else { by };
            let win = result.get("window_name").and_then(|v| v.as_str()).unwrap_or(agent);
            // `[tmm] ` marks a lifecycle line: the client renders it
            // as a system row rather than a chat bubble. A machine
            // marker, not a glyph — how it LOOKS is the UI's call.
            let line = if brief.is_empty() {
                format!("[tmm] spawned {win}")
            } else {
                format!("[tmm] spawned {win} — {brief}")
            };
            let _ = rooms::post(&room, who, &line);
            Ok(result)
        }

        other => Err(RpcError::MethodNotFound(format!("unknown hub method: {other}"))),
    }
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) fn handle_hub_request(req: &Request, notifications: Option<&crate::agent_notifications::AgentNotificationHub>) -> Response {
    Response::from_outcome(req.id, dispatch_hub(req, notifications))
}


#[cfg(any(target_os = "android", target_os = "ios"))]
pub(super) fn handle_hub_request(req: &Request, _notifications: Option<&crate::agent_notifications::AgentNotificationHub>) -> Response {
    Response::err(req.id, ERR_METHOD_NOT_FOUND, "hub not available on this platform".into())
}

/// Window index whose NAME is the agent name. Spawned agents own their window
/// name (projects `up` renames by slot); adopted agents match by window name
/// too, which is the best identity tmux offers.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn window_of_agent(session: &str, agent: &str) -> Option<usize> {
    let panes = crate::tmux::list_panes(session).ok()?;
    panes.iter().find(|p| p.window_name == agent).map(|p| p.window)
}

/// Pump newly-appended hub messages to one connection (board #107). The wire
/// frame keeps the `team_message` method name the client has always listened
/// for — renaming the frame would break every deployed client for zero
/// behaviour change. Lagged receivers re-sync via hub_log on demand.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) async fn hub_push_loop(out_tx: tokio::sync::mpsc::UnboundedSender<super::Outbound>) {
    let mut rx = crate::projects::rooms::subscribe();
    loop {
        match rx.recv().await {
            Ok(msg_json) => {
                let frame = serde_json::json!({
                    "id": null,
                    "method": "team_message",
                    "params": { "message": serde_json::from_str::<serde_json::Value>(&msg_json).unwrap_or(serde_json::Value::Null) },
                });
                if out_tx.send(super::Outbound::Encrypted(serde_json::to_string(&frame).unwrap())).is_err() {
                    return; // send task gone
                }
            }
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }
}

/// Local wall-clock stamp for a line an agent will read: `2026-08-17 16:31`.
/// Minute precision on purpose — this is context, not a log timestamp.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) fn stamp_now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

/// Typing one stamped line into one managed agent's pane lives in
/// `projects::managed` since board #224 (the spawn path types a kimi brief
/// through the same door); this is the same function under its old name.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) use crate::projects::deliver_chat_line;

/// Pure half of the assignee notification (owner, 2026-08-30): given the
/// PREVIOUS issue row and what this save carries, decide whether the assignee
/// should hear about it and name the change. `None` when: nobody is assigned,
/// the assignee is the actor (your own edit is not news), the assignee is the
/// human (who reads the board itself), the save (re)assigns (that has its own
/// dispatch channel), the save MOVES the issue to done (board #30, owner
/// 2026-08-31: "如果任务我标记为done可以不用给agent发送提示了" — acceptance
/// ENDS the work, there is nothing left for the executor to act on, so even
/// title/body riding along with the closing save stay quiet; the room's
/// `[tmm] board … → done` line still records it, and a LATER edit to an
/// already-done issue — a reopen included — is ordinary news again), or
/// nothing actually changed (a save echoing the stored values is a no-op,
/// not an event).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn board_change_notice(
    prev: &serde_json::Value,
    who: &str,
    title: Option<&str>,
    body: Option<&str>,
    status: Option<&str>,
    assigns: bool,
) -> Option<String> {
    let assignee = prev["assignee"].as_str().unwrap_or("");
    if assignee.is_empty() || assignee == who || assignee == "human" || assigns {
        return None;
    }
    let mut changes: Vec<String> = Vec::new();
    if let Some(ns) = status {
        let old = prev["status"].as_str().unwrap_or("");
        // The closing act suppresses the WHOLE save, not just the status
        // atom: a mixed save's title/body arrived as part of accepting the
        // work, and a wake with the status line stripped would still wake.
        if ns == "done" && old != "done" {
            return None;
        }
        if old != ns {
            changes.push(format!("status {old} → {ns}"));
        }
    }
    if let Some(t) = title {
        if t != prev["title"].as_str().unwrap_or("") {
            changes.push(format!("title → \"{t}\""));
        }
    }
    if let Some(b) = body {
        if b != prev["body"].as_str().unwrap_or("") {
            changes.push(format!("body now: {}", excerpt(b, NOTICE_EXCERPT)));
        }
    }
    if changes.is_empty() { None } else { Some(changes.join("; ")) }
}

/// A reply on an issue reaches its CURRENT assignee (board #26). Pure half:
/// decide whether there is an agent to wake and carry enough issue context that
/// the recipient can act without first fetching the board. Delivery itself is
/// still `deliver_chat_line`, whose managed/live gate and receipt bookkeeping
/// are the authority. No target for unassigned/human/self replies — persistence
/// remains the fallback, never an RPC error or a duplicate self-prompt.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn board_note_notice(
    issue: &serde_json::Value,
    author: &str,
    body: &str,
) -> Option<(String, String)> {
    let assignee = issue["assignee"].as_str().unwrap_or("");
    if assignee.is_empty() || assignee == "human" || assignee == author {
        return None;
    }
    let id = issue["id"].as_i64().unwrap_or(0);
    let title = crate::projects::issue_ref(
        issue["title"].as_str().unwrap_or(""),
        issue["body"].as_str().unwrap_or(""),
        id,
    );
    let note = excerpt(body, NOTICE_EXCERPT);
    if note.is_empty() {
        return None;
    }
    let notice = format!(
        "[board #{id} reply] {title} — {note}. Reply on the issue with `tmm board note {id} \"...\"`."
    );
    Some((assignee.to_string(), notice))
}

/// The delivered-message budget (owner, 2026-08-30: "尽量保证我们发送的内容
/// 比较简洁，避免 Agent 去做过多无谓的消耗"): a notification CARRIES its
/// context so the reader usually needs no lookup round-trip, but never a
/// wall of text. The `…` is the truncation signal — an agent that sees it
/// knows `tmm board show N` has the rest; a message without it is complete.
const NOTICE_EXCERPT: usize = 400;

/// First `max` chars, cut on a char boundary, `…`-marked when shortened.
fn excerpt(s: &str, max: usize) -> String {
    let t = s.trim();
    if t.chars().count() <= max {
        return t.to_string();
    }
    let cut: String = t.chars().take(max).collect();
    format!("{}…", cut.trim_end())
}

const TEAM_CONTEXT_HISTORY_LIMIT: i64 = 1000;
const TEAM_CONTEXT_MAX_MESSAGES: usize = 40;
const TEAM_CONTEXT_MAX_CHARS: usize = 12 * 1024;
const TEAM_CONTEXT_BODY_CHARS: usize = 1600;

#[derive(Debug, Clone, PartialEq)]
struct RoutedChat {
    ts: i64,
    from: String,
    to: Vec<String>,
    body: String,
    hidden: bool,
}

fn context_noise(body: &str) -> bool {
    let body = body.trim_start();
    body.starts_with("[tmm] ")
        || body.starts_with("[tmm status ")
        || body.starts_with("[tmm done]")
}

/// Read sender → recipient edges from the room's durable `to` field. Older
/// hook replies with no stored route fall back to the senders waiting on that
/// agent; only legacy/test rows that lack `to` parse mentions from prose.
#[cfg(test)]
fn route_chat_history(messages: &[serde_json::Value], agents: &[String]) -> Vec<RoutedChat> {
    route_chat_history_hiding(messages, agents, &std::collections::HashSet::new())
}

fn route_chat_history_hiding(
    messages: &[serde_json::Value],
    agents: &[String],
    hidden_ids: &std::collections::HashSet<String>,
) -> Vec<RoutedChat> {
    let known: std::collections::HashSet<&str> = agents.iter().map(String::as_str).collect();
    let mut pending: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    let mut routed = Vec::new();
    for message in messages {
        if message
            .get("kind")
            .and_then(|value| value.as_str())
            .is_some_and(|kind| kind != "msg")
        {
            continue;
        }
        let from = message.get("from").and_then(|v| v.as_str()).unwrap_or("").trim();
        let body = message.get("body").and_then(|v| v.as_str()).unwrap_or("").trim();
        if from.is_empty() || body.is_empty() || context_noise(body) {
            continue;
        }
        let stored_to_field = message.get("to").and_then(|value| value.as_array());
        let stored_to: Vec<String> = stored_to_field
            .map(|values| {
                values
                    .iter()
                    .filter_map(|value| value.as_str())
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        let has_stored_to = stored_to_field.is_some();
        let mentions = if has_stored_to { stored_to } else { crate::address::mention_names(body) };
        let to = if mentions.is_empty() {
            if from == "human" {
                vec!["room".to_string()]
            } else {
                pending.remove(from).unwrap_or_else(|| vec!["room".to_string()])
            }
        } else {
            for mention in &mentions {
                let targets: Vec<&str> = if mention == "all" {
                    known.iter().copied().filter(|target| *target != from).collect()
                } else if has_stored_to || known.contains(mention.as_str()) {
                    vec![mention]
                } else {
                    Vec::new()
                };
                for target in targets {
                    let senders = pending.entry(target.to_string()).or_default();
                    if !senders.iter().any(|sender| sender == from) {
                        senders.push(from.to_string());
                    }
                }
            }
            mentions
        };
        routed.push(RoutedChat {
            ts: message.get("ts").and_then(|v| v.as_i64()).unwrap_or(0),
            from: from.to_string(),
            to,
            body: body.to_string(),
            hidden: message
                .get("id")
                .and_then(|value| value.as_str())
                .is_some_and(|id| hidden_ids.contains(id)),
        });
    }
    routed
}

fn compact_context_body(body: &str) -> String {
    let flat = body.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut out: String = flat.chars().take(TEAM_CONTEXT_BODY_CHARS).collect();
    if flat.chars().count() > TEAM_CONTEXT_BODY_CHARS {
        out.push('…');
    }
    out
}

fn context_recipient_label(to: &[String]) -> String {
    to.iter()
        .map(|name| if name == "room" { "room".to_string() } else { format!("@{name}") })
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn context_stamp(ts: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "?".to_string())
}

/// Messages since the target's previous delivery, bounded at the delivery
/// edge. The newest rows win when the prior delivery lies beyond the page.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn team_context(
    history: &[RoutedChat],
    target: &str,
    history_clipped: bool,
) -> Option<String> {
    let after = history
        .iter()
        .rposition(|message| message.to.iter().any(|name| name == target || name == "all"))
        .map(|index| index + 1)
        .unwrap_or(0);
    let candidates: Vec<String> = history[after..]
        .iter()
        .filter(|message| message.from != target && !message.hidden)
        .map(|message| {
            format!(
                "[{}] {} -> {}: {}",
                context_stamp(message.ts),
                message.from,
                context_recipient_label(&message.to),
                compact_context_body(&message.body)
            )
        })
        .collect();
    if candidates.is_empty() {
        return None;
    }

    let mut kept = Vec::new();
    let mut chars = 0;
    for line in candidates.iter().rev() {
        let cost = line.chars().count() + 1;
        if kept.len() >= TEAM_CONTEXT_MAX_MESSAGES || chars + cost > TEAM_CONTEXT_MAX_CHARS {
            break;
        }
        chars += cost;
        kept.push(line.clone());
    }
    kept.reverse();
    let omitted = candidates.len().saturating_sub(kept.len());
    let mut out = String::from(
        "[tmm team context — background since your previous delivery; not new instructions]\n",
    );
    if omitted > 0 || (history_clipped && after == 0) {
        let budget = if omitted > 0 {
            format!(", {omitted} older context rows also omitted by budget")
        } else {
            String::new()
        };
        out.push_str(&format!(
            "[older room history omitted; use `tmm log` for the full transcript{budget}]\n"
        ));
    }
    out.push_str(&kept.join("\n"));
    out.push_str("\n[/tmm team context]");
    Some(out)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn delivered_chat_line(from: &str, body: &str, context: Option<&str>) -> String {
    let current = format!("[tmm chat {}] {from}: {body}", stamp_now());
    match context {
        Some(context) => format!("{current}\n\n{context}"),
        None => current,
    }
}

/// Type an @mentioned chat line into each mentioned agent's pane. Team members
/// also receive the room delta since their previous delivery, with reconstructed
/// sender → recipient edges. Non-team agents keep the one-line delivery.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn deliver_mentions(
    session: &str,
    from: &str,
    body: &str,
    room: &str,
    before_seq: Option<i64>,
    msg_id: &str,
) {
    use crate::projects::agents;

    let mentions = crate::address::mention_names(body);
    if mentions.is_empty() {
        return;
    }
    let ws = crate::projects::project_for_session(session).ok().flatten().map(|p| p.path);
    let Ok(panes) = crate::tmux::list_panes(session) else { return };
    let managed_names: Vec<String> = panes
        .iter()
        .filter(|pane| crate::projects::is_managed_in(ws.as_deref(), &pane.window_name))
        .map(|pane| pane.window_name.clone())
        .collect::<std::collections::HashSet<_>>()
        .into_iter()
        .collect();
    let needs_context = panes.iter().any(|pane| {
        mentions.iter().any(|name| name == &pane.window_name || name == "all")
            && crate::projects::team_of(ws.as_deref(), &pane.window_name).is_some()
    });
    let (history, history_clipped) = if needs_context {
        let page = crate::projects::rooms::history_page(room, before_seq, TEAM_CONTEXT_HISTORY_LIMIT);
        let hidden: std::collections::HashSet<String> =
            crate::projects::archived_ids(room).into_iter().collect();
        let messages: Vec<serde_json::Value> = page
            .get("messages")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        (
            route_chat_history_hiding(&messages, &managed_names, &hidden),
            page.get("has_more").and_then(|v| v.as_bool()).unwrap_or(false),
        )
    } else {
        (Vec::new(), false)
    };
    let mut seen = std::collections::HashSet::new();
    for p in &panes {
        if !seen.insert(p.window) || !p.active {
            continue;
        }
        let is_agent = agents::detect_pane(ws.as_deref(), p).is_some();
        if !is_agent || p.window_name == from {
            continue;
        }
        // MANAGED windows only. `@all` would otherwise type into a kiro the user
        // started by hand in this directory — the app does not own that session,
        // and injecting a chat line into it is not ours to do. Same gate as
        // hub_agents' participant list and the stop-hook auto-post.
        if !crate::projects::is_managed_in(ws.as_deref(), &p.window_name) {
            continue;
        }
        let matched = mentions.iter().any(|m| m == &p.window_name || m == "all");
        if !matched {
            continue;
        }
        let target = format!("{}:{}.{}", session, p.window, p.pane);
        // The stamp is for the agent, not for us: a CLI reads this line inside a
        // conversation that may have been idle for hours, and "when was this
        // said" is context it otherwise has no way to recover — its own clock
        // only tells it `now`. Local wall time, minute precision; seconds would
        // be noise in a chat line.
        let context = crate::projects::team_of(ws.as_deref(), &p.window_name)
            .and_then(|_| team_context(&history, &p.window_name, history_clipped));
        let line = delivered_chat_line(from, body, context.as_deref());
        match crate::tmux::send_command(&target, &line) {
            Ok(()) => {
                // send_command only proves the pane existed. The delivery is
                // confirmed when that agent's userPromptSubmit hook echoes the
                // line back; until then it is pending, and telemetry reports it
                // if the echo never comes.
                crate::projects::telemetry::record_delivery(session, &p.window_name, &line, msg_id);
                // A line just landed in this pane: sniff its vitals once the TUI
                // has repainted (delayed + throttled inside).
                crate::projects::vitals::sniff_window_soon(session, &p.window_name);
            }
            // Nothing was recorded as delivered, so no echo and no sweep will
            // ever speak for this line: say so now (board #250 — the pane was
            // in copy-mode, the refusal a person reading scrollback earns).
            Err(e) => {
                crate::projects::telemetry::record_undelivered(session, &p.window_name, &line, e.trim());
            }
        }
    }
}

/// One row per live window: name, command, agent detection, derived status,
/// and whether the window is MANAGED — spawned from the registry into an
/// isolated home. Managed agents are chat participants; direct windows
/// (shells, agents the user started by hand) are terminal things and the UI
/// presents them only there. The marker is the isolated home dir itself:
/// <workspace>/.tmm/agents/<window_name>/ exists iff spawn materialized it.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn agent_states(session: &str) -> serde_json::Value {
    use crate::projects::{agents, telemetry};

    let ws = crate::projects::project_for_session(session)
        .ok()
        .flatten()
        .map(|p| p.path);
    let panes = crate::tmux::list_panes(session).unwrap_or_default();
    let activity: std::collections::HashMap<usize, u64> =
        crate::tmux::window_activity_times(session).into_iter().collect();

    // Group panes by window, represent each window by its active pane (the
    // same rule the projects capture uses).
    let mut windows: std::collections::BTreeMap<usize, &crate::tmux::TmuxPane> = std::collections::BTreeMap::new();
    for p in &panes {
        windows
            .entry(p.window)
            .and_modify(|cur| {
                if p.active {
                    *cur = p;
                }
            })
            .or_insert(p);
    }
    let live: Vec<String> = windows.values().map(|p| p.window_name.clone()).collect();
    telemetry::retain_windows(session, &live);
    crate::projects::vitals::retain_windows(session, &live);

    let rows: Vec<serde_json::Value> = windows
        .values()
        .map(|p| {
            let agent = agents::detect_pane(ws.as_deref(), p);
            let st = telemetry::derive(session, &p.window_name, activity.get(&p.window).copied().unwrap_or(0));
            let managed = agent.is_some() && crate::projects::is_managed_in(ws.as_deref(), &p.window_name);
            // What the agent's own status line says: model, context used, effort,
            // branch. There is no API for a CLI's live state, so it is SNIFFED
            // from the last lines of the pane — hence managed agents only (we
            // know their status line's shape), every field optional, and the
            // object omitted entirely when nothing could be read. One
            // capture-pane per agent, capped at 4 per project.
            // A miss is normal — the pane may be mid-repaint, a tool's output may
            // have pushed the status line up — so the reading REMEMBERS: gaps are
            // filled field by field from the last good one (1 h TTL), and the
            // memory is kept WARM by `sniff_window_soon` at every hook edge and
            // delivered chat line, so this poll usually just reads it. Treating
            // every miss as "no information" is what made the card blink empty.
            let vitals = if managed {
                crate::tmux::capture_pane_plain(&format!("{session}:{}", p.window), Some(0))
                    .map(|text| {
                        crate::projects::vitals::sniff_remembered(
                            session,
                            &p.window_name,
                            &text,
                            &p.window_name,
                            agent.map(|a| a.backend).unwrap_or(""),
                        )
                    })
                    .unwrap_or_default()
            } else {
                Default::default()
            };
            serde_json::json!({
                "window": p.window,
                "name": p.window_name,
                "command": p.current_command,
                "agent": agent.map(|a| a.backend),
                "managed": managed,
                // The configured team this window was started as part of
                // (board #74) — the Hub groups same-team cards; null = solo.
                "team": if managed { crate::projects::team_of(ws.as_deref(), &p.window_name) } else { None },
                "state": if agent.is_some() { st.state.as_str() } else { "shell" },
                "detail": st.detail,
                "since": st.since,
                "vitals": if vitals.is_empty() { serde_json::Value::Null } else { serde_json::to_value(&vitals).unwrap_or(serde_json::Value::Null) },
            })
        })
        .collect();
    serde_json::json!({ "session": session, "agents": rows })
}

#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod tests {
    use super::super::test_util::req;
    use super::*;
    use crate::projects::rooms;
    use crate::projects::telemetry;

    /// Every store-touching test seeds its own unique session/room: the test
    /// store (state.db under TMM_STATE_DB) is one file per process, and the
    /// hub's messages now live in it for real — the Bridge double died with
    /// TeamBridge (board #107).
    fn unique(prefix: &str) -> (String, String) {
        let session = format!("{prefix}-{}", uuid::Uuid::new_v4());
        let room = format!("proj:{session}");
        (session, room)
    }

    /// The assignee-notification decision (owner, 2026-08-30): pure, so the
    /// skips are pinned without tmux. The delivery half reuses the same
    /// `deliver_chat_line` the review handoff and done-summary edges use.
    #[test]
    fn board_change_notice_decides_who_hears() {
        let prev = serde_json::json!({
            "title": "T", "body": "B", "status": "doing", "assignee": "builder", "created_by": "human",
        });
        // A human status change reaches the assignee, named.
        assert_eq!(
            super::board_change_notice(&prev, "human", None, None, Some("todo"), false).as_deref(),
            Some("status doing → todo")
        );
        // Edits CARRY the new values (owner, 2026-08-30: the message is the
        // context — no lookup round-trip for a short change).
        assert_eq!(
            super::board_change_notice(&prev, "human", Some("T2"), Some("B2"), Some("review"), false).as_deref(),
            Some("status doing → review; title → \"T2\"; body now: B2")
        );
        // A long body is excerpted, and the `…` is the only pointer needed.
        let long = "x".repeat(500);
        let noticed = super::board_change_notice(&prev, "human", None, Some(&long), None, false).unwrap();
        assert!(noticed.starts_with("body now: xxx"));
        assert!(noticed.ends_with('…'));
        assert!(noticed.chars().count() < 420);
        // The actor's own edit is not news.
        assert_eq!(super::board_change_notice(&prev, "builder", None, None, Some("done"), false), None);
        // A save echoing the stored values is a no-op, not an event.
        assert_eq!(super::board_change_notice(&prev, "human", Some("T"), Some("B"), Some("doing"), false), None);
        // A (re)assignment has its own dispatch channel — no second line.
        assert_eq!(super::board_change_notice(&prev, "human", None, None, Some("todo"), true), None);
        // Nobody assigned / the human assignee: nobody to wake.
        let unassigned = serde_json::json!({ "title": "T", "body": "B", "status": "todo", "assignee": "" });
        assert_eq!(super::board_change_notice(&unassigned, "human", None, None, Some("doing"), false), None);
        let human_owned = serde_json::json!({ "title": "T", "body": "B", "status": "todo", "assignee": "human" });
        assert_eq!(super::board_change_notice(&human_owned, "lead", None, None, Some("doing"), false), None);
    }

    #[test]
    fn a_move_to_done_never_wakes_the_assignee() {
        let prev = serde_json::json!({
            "title": "T", "body": "B", "status": "review", "assignee": "builder",
        });
        // Acceptance is the END of the work — there is nothing for the
        // executor to act on, so the whole save stays quiet (board #30).
        assert_eq!(super::board_change_notice(&prev, "human", None, None, Some("done"), false), None);
        let todo = serde_json::json!({ "title": "T", "body": "B", "status": "todo", "assignee": "builder" });
        assert_eq!(super::board_change_notice(&todo, "human", None, None, Some("done"), false), None);
        // Even a MIXED save (title/body riding along with the acceptance) is
        // suppressed wholesale — the fields arrived as part of closing it.
        assert_eq!(
            super::board_change_notice(&prev, "human", Some("T2"), Some("B2"), Some("done"), false),
            None
        );
        // But an ALREADY-done issue edited later is ordinary news again: the
        // suppression is about the closing act, not the done state.
        let done = serde_json::json!({ "title": "T", "body": "B", "status": "done", "assignee": "builder" });
        assert_eq!(
            super::board_change_notice(&done, "human", Some("T2"), None, None, false).as_deref(),
            Some("title → \"T2\"")
        );
        // ... including when the save echoes status "done" back unchanged.
        assert_eq!(
            super::board_change_notice(&done, "human", None, Some("B2"), Some("done"), false).as_deref(),
            Some("body now: B2")
        );
        // And REOPENING (done → doing) is exactly the wake the rule preserves.
        assert_eq!(
            super::board_change_notice(&done, "human", None, None, Some("doing"), false).as_deref(),
            Some("status done → doing")
        );
    }

    #[test]
    fn board_note_notice_targets_only_the_other_agent() {
        let issue = serde_json::json!({
            "id": 26, "title": "Reply delivery", "assignee": "builder",
        });
        let (target, notice) = super::board_note_notice(
            &issue,
            "human",
            "please revise the retry path",
        )
        .expect("a human reply wakes the assigned agent");
        assert_eq!(target, "builder");
        assert_eq!(
            notice,
            "[board #26 reply] Reply delivery — please revise the retry path. Reply on the issue with `tmm board note 26 \"...\"`."
        );

        let long = "x".repeat(500);
        let (_, shortened) = super::board_note_notice(&issue, "lead", &long).unwrap();
        assert!(shortened.contains('…'), "a long note names that more is on the issue");
        assert!(shortened.chars().count() < 500, "the interrupt stays concise");

        assert_eq!(super::board_note_notice(&issue, "builder", "my own note"), None);
        let unassigned = serde_json::json!({ "id": 1, "title": "T", "assignee": "" });
        assert_eq!(super::board_note_notice(&unassigned, "human", "hello"), None);
        let human = serde_json::json!({ "id": 1, "title": "T", "assignee": "human" });
        assert_eq!(super::board_note_notice(&human, "lead", "hello"), None);

        // A TITLELESS issue (board #31) is named by its body through the
        // shared issue_ref fallback — never an empty head + dangling dash.
        let titleless = serde_json::json!({
            "id": 31, "title": "", "body": "issue标题可以为空，截前几个字显示", "assignee": "builder",
        });
        let (_, n) = super::board_note_notice(&titleless, "human", "ok").unwrap();
        assert_eq!(
            n,
            "[board #31 reply] issue标题可以为空，截前几个字显示 — ok. Reply on the issue with `tmm board note 31 \"...\"`."
        );
    }

    #[test]
    fn board_counts_answers_without_a_session_like_hub_rooms() {
        crate::projects::tests::use_test_store();
        let session = format!("counts-rpc-{}", uuid::Uuid::new_v4());
        crate::projects::board_save(&session, None, Some("count me"), None, None, None, "human").unwrap();
        // NO `session` in params, by design: the method is about EVERY board,
        // so it must answer BEFORE the session gate — a "session required"
        // error here means it slid below the gate (the hub_rooms precedent).
        let r = handle_hub_request(&req("hub_board_counts", serde_json::json!({})), None);
        assert!(r.error.is_none(), "{:?}", r.error.map(|e| e.message));
        let v = r.result.expect("result");
        let counts = v["counts"].as_object().expect("counts object");
        let row = counts.get(&session).expect("the seeded board is present");
        assert_eq!(row["todo"], 1);
        assert_eq!(row["doing"], 0, "zero-filled vocabulary over the wire");
        assert_eq!(row["total"], 1, "emptiness is one explicit field");
    }

    /// A page can lose EVERY row to the archive filter, and the walk still has to
    /// continue: `has_more` says there is history behind it, so if no cursor comes
    /// back with it the client has nothing to ask for and scroll-up stops dead at
    /// the hidden run. The cursor falls back to the RAW page's oldest seq — the
    /// position always advances, even when nothing survived to be shown.
    #[test]
    fn a_fully_hidden_page_still_hands_back_a_cursor_to_the_older_visible_ones() {
        crate::projects::tests::use_test_store();
        let (session, room) = unique("hid");
        // Five messages; the middle stretch (3rd and 4th) is archived, so one
        // whole page of two is invisible. seq is store-global, so every
        // position below is read off the seeded rows, never assumed.
        let seeded: Vec<serde_json::Value> = (1..=5)
            .map(|n| rooms::seed_msg(&room, &format!("{room}-m{n}"), n * 10, "human", &[], &format!("body{n}")))
            .collect();
        let seq = |n: usize| seeded[n - 1]["seq"].as_i64().unwrap();
        for n in [3, 4] {
            crate::projects::archive_msg(&room, seeded[n - 1]["id"].as_str().unwrap(), (n as u64) * 10, "human", "x").unwrap();
        }
        let page = |before: Option<i64>| {
            let mut params = serde_json::json!({ "session": session, "limit": 2 });
            if let Some(b) = before {
                params["before_seq"] = serde_json::json!(b);
            }
            handle_hub_request(&req("hub_log", params), None).result.expect("result")
        };

        // Page 1: raw [m4, m5], m4 hidden → the visible tail, cursor from the
        // SURVIVOR (what the user can actually see).
        let p1 = page(None);
        let bodies = |v: &serde_json::Value| {
            v["messages"].as_array().unwrap().iter()
                .map(|m| m["body"].as_str().unwrap().to_string()).collect::<Vec<_>>()
        };
        assert_eq!(bodies(&p1), vec!["body5"]);
        assert_eq!(p1["oldest_seq"], seq(5));
        assert_eq!(p1["has_more"], true);

        // Page 2: raw [m3, m4] — BOTH hidden. Nothing to render, but the page must
        // still carry the raw cursor or the walk cannot go on.
        let p2 = page(p1["oldest_seq"].as_i64());
        assert!(bodies(&p2).is_empty(), "the whole page is hidden");
        assert_eq!(p2["has_more"], true, "and there is more behind it");
        assert_eq!(p2["oldest_seq"], seq(3), "the raw oldest seq is the cursor of last resort");

        // Page 3: continuing from that cursor reaches the older VISIBLE messages,
        // which is the behaviour the fallback exists for.
        let p3 = page(p2["oldest_seq"].as_i64());
        assert_eq!(bodies(&p3), vec!["body1", "body2"]);
        assert_eq!(p3["has_more"], false, "that is the start of the conversation");
        assert_eq!(p3["oldest_seq"], seq(1));

        // Every visible message was reached exactly once across the walk.
        let seen: Vec<String> = [bodies(&p3), bodies(&p2), bodies(&p1)].concat();
        assert_eq!(seen, vec!["body1", "body2", "body5"]);
    }

    /// Board #9: the room keeps everything, so the client needs a way to ask for
    /// a SMALL first page and then walk back. Two contracts are pinned here — a
    /// client with no cursor gets exactly the newest page, and `before_seq`
    /// walks backwards with a cursor handed back for the step after it.
    #[test]
    fn hub_log_answers_the_newest_page_and_pages_backwards_on_request() {
        crate::projects::tests::use_test_store();
        let (session, room) = unique("page");
        let older = rooms::seed_msg(&room, &format!("{room}-1"), 50, "a", &[], "older");
        let old = rooms::seed_msg(&room, &format!("{room}-2"), 100, "a", &[], "old");
        let new = rooms::seed_msg(&room, &format!("{room}-3"), 200, "b", &[], "new");

        // A client with no cursor: the newest page under its limit.
        let r = handle_hub_request(&req("hub_log", serde_json::json!({ "session": session, "limit": 2 })), None);
        let v = r.result.expect("result");
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2, "the newest page");
        assert_eq!(msgs[0]["body"], "old", "oldest first");
        assert_eq!(v["has_more"], true, "and it says history remains behind it");
        assert_eq!(v["oldest_seq"], old["seq"], "the cursor for the next page back");
        assert_eq!(v["head_seq"], new["seq"]);

        // Scrolled up: the client asks for what is behind its oldest message.
        let r = handle_hub_request(
            &req("hub_log", serde_json::json!({ "session": session, "before_seq": old["seq"], "limit": 50 })),
            None,
        );
        let v = r.result.expect("result");
        assert_eq!(v["messages"].as_array().unwrap().len(), 1);
        assert_eq!(v["messages"][0]["body"], "older");
        assert_eq!(v["messages"][0]["id"], older["id"]);
        assert_eq!(v["has_more"], false, "the conversation begins there");

        // A nonsense cursor is no cursor: 0/negative means "the newest page",
        // never a query that could return nothing for ever.
        let r = handle_hub_request(
            &req("hub_log", serde_json::json!({ "session": session, "before_seq": 0, "limit": 2 })),
            None,
        );
        assert_eq!(r.result.expect("result")["messages"].as_array().unwrap().len(), 2);
    }

    /// `hub_search` finds instead of paging (owner, 2026-09-07): a term list,
    /// any-match, over the room's history — and the archive mirror applies to
    /// search exactly as it does to `hub_log`, or a hidden message would come
    /// back the moment someone greps for it.
    #[test]
    fn hub_search_matches_terms_validates_them_and_hides_the_archived() {
        crate::projects::tests::use_test_store();
        let (session, room) = unique("srch");
        rooms::seed_msg(&room, &format!("{room}-1"), 100, "a", &[], "old");
        let newer = rooms::seed_msg(&room, &format!("{room}-2"), 200, "b", &[], "new");
        // No terms (or all-blank terms) is a caller error, not "match everything".
        for bad in [serde_json::json!({ "session": session }),
                    serde_json::json!({ "session": session, "grep": ["  "] })] {
            let r = handle_hub_request(&req("hub_search", bad), None);
            assert!(r.error.is_some(), "empty grep must be rejected");
        }
        // Any-match over the room's history ("old" by a, "new" by b): one term
        // hits one message; two terms hit both; a sender name is searchable too.
        let search = |grep: serde_json::Value| {
            let r = handle_hub_request(
                &req("hub_search", serde_json::json!({ "session": session, "grep": grep })),
                None,
            );
            r.result.expect("result")["messages"].as_array().unwrap().clone()
        };
        assert_eq!(search(serde_json::json!(["OLD"])).len(), 1, "case-insensitive body match");
        assert_eq!(search(serde_json::json!(["old", "new"])).len(), 2, "a term list is any-match");
        assert_eq!(search(serde_json::json!(["b"])).len(), 1, "the sender matches too");
        // Archiving hides from search as it does from the log.
        crate::projects::archive_msg(&room, newer["id"].as_str().unwrap(), 200, "b", "new").unwrap();
        let msgs = search(serde_json::json!(["old", "new"]));
        assert_eq!(msgs.len(), 1, "the archived hit is filtered: {msgs:?}");
        assert_eq!(msgs[0]["body"], "old");
    }

    /// `global: true` widens the scope to EVERY room; each hit carries its
    /// `room` field so a cross-project answer stays readable. (The store
    /// enumerates rooms itself now — the "pageless bridge degrades to empty"
    /// contract died with the TeamBridge trait, board #107.)
    #[test]
    fn hub_search_global_searches_every_room_and_names_it() {
        crate::projects::tests::use_test_store();
        let (session_a, room_a) = unique("gsrch-a");
        let (_, room_b) = unique("gsrch-b");
        let needle = format!("needle-{}", uuid::Uuid::new_v4());
        rooms::seed_msg(&room_a, &format!("{room_a}-1"), 100, "a", &[], &format!("{needle} in a"));
        rooms::seed_msg(&room_b, &format!("{room_b}-1"), 200, "b", &[], &format!("{needle} in b"));
        let r = handle_hub_request(
            &req("hub_search", serde_json::json!({ "session": session_a, "grep": [needle], "global": true })),
            None,
        );
        let msgs = r.result.expect("result")["messages"].as_array().unwrap().clone();
        assert_eq!(msgs.len(), 2, "both rooms are searched: {msgs:?}");
        let rooms_hit: Vec<&str> = msgs.iter().map(|m| m["room"].as_str().unwrap()).collect();
        assert!(rooms_hit.contains(&room_a.as_str()) && rooms_hit.contains(&room_b.as_str()));
    }

    /// The activity feed's half of the same contract. The durable log keeps every
    /// event, so this read is where the bound lives: a default page, a hard cap,
    /// and a (ts, id) cursor handed back for walking backwards.
    #[test]
    fn hub_activity_pages_and_caps_and_reports_what_it_holds() {
        crate::projects::tests::use_test_store();
        let session = format!("act-rpc-{}", uuid::Uuid::new_v4());
        for n in 0..6 {
            telemetry::record_tool(&session, "w1", "Edit", &format!("f{n}.rs"));
        }
        // An older client sends only since_ts and gets the newest page.
        let r = handle_hub_request(
            &req("hub_activity", serde_json::json!({ "session": session, "since_ts": 0 })),
            None,
        );
        let v = r.result.expect("result");
        let evs = v["events"].as_array().unwrap();
        assert_eq!(evs.len(), 6, "everything this session has, under the default cap");
        assert!(v.get("has_more").is_some(), "the client is told whether more exists");

        // A limit is honoured, and the page carries the cursor for the next one.
        let r = handle_hub_request(
            &req("hub_activity", serde_json::json!({ "session": session, "limit": 2 })),
            None,
        );
        let v = r.result.expect("result");
        assert_eq!(v["events"].as_array().unwrap().len(), 2);
        let oldest = v["oldest"].clone();
        assert!(oldest["ts"].as_u64().unwrap() > 0, "a usable cursor, got {oldest:?}");

        // And it is a CAP, not a suggestion.
        let r = handle_hub_request(
            &req("hub_activity", serde_json::json!({ "session": session, "limit": 99999 })),
            None,
        );
        assert!(
            r.result.expect("result")["events"].as_array().unwrap().len()
                <= telemetry::MAX_PAGE_EVENTS
        );
    }

    #[test]
    fn hub_post_lands_in_the_project_room_with_the_sender() {
        crate::projects::tests::use_test_store();
        let (session, room) = unique("post");
        let r = handle_hub_request(
            &req("hub_post", serde_json::json!({ "session": session, "from": "lead", "body": "@reviewer 看一下" })),
            None,
        );
        assert!(r.error.is_none(), "{}", r.error.map(|e| e.message).unwrap_or_default());
        let msg = r.result.expect("the stored message comes back");
        assert_eq!(msg["room"], room);
        assert_eq!(msg["from"], "lead");
        assert_eq!(msg["to"], serde_json::json!(["reviewer"]), "the route is stored in the envelope");
        // And it is durably in the room, not just in the response.
        let page = rooms::history_page(&room, None, 10);
        let msgs = page["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["body"], "@reviewer 看一下");
    }

    #[test]
    fn hub_post_record_only_skips_delivery_but_stores_message() {
        crate::projects::tests::use_test_store();
        let (session, room) = unique("rec");
        let r = handle_hub_request(
            &req("hub_post", serde_json::json!({
                "session": session, "from": "lead",
                "body": "@reviewer 自动结果", "record_only": true
            })),
            None,
        );
        assert!(r.error.is_none(), "{}", r.error.map(|e| e.message).unwrap_or_default());
        // The message is stored in the room even though delivery was skipped.
        let msgs = rooms::history_page(&room, None, 10);
        let msgs = msgs["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1, "record-only posts are stored in the room");
        assert_eq!(msgs[0]["body"], "@reviewer 自动结果");
    }

    #[test]
    fn hub_post_status_is_an_ambient_agent_message() {
        crate::projects::tests::use_test_store();
        let (session, room) = unique("stat");
        let r = handle_hub_request(
            &req("hub_post", serde_json::json!({
                "session": session, "from": "lead",
                "body": "reviewing @reviewer output", "status": true
            })),
            None,
        );
        assert!(r.error.is_none(), "{}", r.error.map(|e| e.message).unwrap_or_default());
        let msgs = rooms::history_page(&room, None, 10);
        let msgs = msgs["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 1);
        assert_eq!(msgs[0]["from"], "lead");
        assert_eq!(msgs[0]["body"], "[tmm status working] reviewing @reviewer output");
    }

    #[test]
    fn team_context_reconstructs_routes_since_the_targets_previous_delivery() {
        let messages = vec![
            serde_json::json!({ "ts": 1000, "from": "lead", "to": ["writer"], "body": "draft it; source is a@b.example" }),
            serde_json::json!({ "ts": 2000, "from": "researcher", "body": "@lead check this fact" }),
            serde_json::json!({ "ts": 3000, "from": "lead", "body": "the source is primary" }),
            serde_json::json!({ "ts": 4000, "from": "writer", "body": "draft ready" }),
            serde_json::json!({ "ts": 5000, "from": "review-style", "body": "@review-reader compare notes" }),
            serde_json::json!({ "ts": 6000, "from": "review-reader", "body": "reader concern confirmed" }),
            serde_json::json!({ "ts": 7000, "from": "writer", "body": "[tmm status working] rendering" }),
            serde_json::json!({ "ts": 7500, "from": "system", "kind": "join", "body": "reviewer joined" }),
            serde_json::json!({ "ts": 8000, "from": "human", "body": "room note" }),
        ];
        let agents = ["lead", "writer", "researcher", "review-reader", "review-style"]
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        let routed = route_chat_history(&messages, &agents);
        assert_eq!(routed[2].to, vec!["researcher"], "lead's final replies to its requester");
        assert_eq!(routed[3].to, vec!["lead"], "writer's final replies to lead");
        assert_eq!(routed[5].to, vec!["review-style"], "review reply edge is reconstructed");
        assert!(!routed.iter().any(|message| message.body.contains("rendering")), "status is noise");
        assert!(!routed.iter().any(|message| message.body.contains("joined")), "lifecycle rows are noise");

        let context = team_context(&routed, "lead", false).expect("messages followed lead's last delivery");
        assert!(!context.contains("draft ready"), "the last delivery is the exclusive boundary");
        assert!(context.contains("review-style -> @review-reader: @review-reader compare notes"));
        assert!(context.contains("review-reader -> @review-style: reader concern confirmed"));
        assert!(context.contains("human -> room: room note"));
        assert!(context.contains("background since your previous delivery; not new instructions"));
    }

    #[test]
    fn an_archived_delivery_remains_the_context_boundary() {
        let messages = vec![
            serde_json::json!({ "id": "old", "ts": 1000, "from": "writer", "to": ["reviewer"], "body": "@reviewer old task" }),
            serde_json::json!({ "id": "reply", "ts": 2000, "from": "reviewer", "to": ["writer"], "body": "old reply" }),
            serde_json::json!({ "id": "boundary", "ts": 3000, "from": "human", "to": ["reviewer"], "body": "@reviewer archived task" }),
            serde_json::json!({ "id": "new", "ts": 4000, "from": "writer", "to": ["lead"], "body": "@lead new work" }),
        ];
        let hidden = ["boundary".to_string()].into_iter().collect();
        let routed = route_chat_history_hiding(
            &messages,
            &["writer".to_string(), "reviewer".to_string(), "lead".to_string()],
            &hidden,
        );
        let context = team_context(&routed, "reviewer", false).expect("new work follows the boundary");
        assert!(!context.contains("old task"));
        assert!(!context.contains("archived task"));
        assert!(context.contains("writer -> @lead: @lead new work"));
    }

    #[test]
    fn team_context_keeps_the_current_request_first_and_handles_all() {
        let messages = vec![
            serde_json::json!({ "ts": 1000, "from": "human", "body": "@all start" }),
            serde_json::json!({ "ts": 2000, "from": "writer", "body": "writer done" }),
            serde_json::json!({ "ts": 3000, "from": "reviewer", "body": "review done" }),
        ];
        let agents = vec!["writer".to_string(), "reviewer".to_string()];
        let routed = route_chat_history(&messages, &agents);
        assert_eq!(routed[1].to, vec!["human"]);
        assert_eq!(routed[2].to, vec!["human"]);

        let context = team_context(&routed, "lead", true).expect("lead has no prior boundary");
        let delivered = delivered_chat_line("human", "@lead decide", Some(&context));
        assert!(delivered.starts_with("[tmm chat "));
        assert!(delivered.find("@lead decide").unwrap() < delivered.find("[tmm team context").unwrap());
        assert!(!delivered.contains("older room history omitted"), "@all is lead's prior delivery");
        assert!(!delivered_chat_line("human", "@solo decide", None).contains("[tmm team context"));
    }

    /// Kills the test session and removes its workspace when dropped, so a
    /// test that fails part-way leaves no live session for the server on the
    /// same tmux to adopt as a project (2026-09-27, the #250 negative controls).
    struct KillOnDrop(String, std::path::PathBuf);
    impl Drop for KillOnDrop {
        fn drop(&mut self) {
            let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &format!("={}", self.0)]).stderr(std::process::Stdio::null()).status();
            let _ = std::fs::remove_dir_all(&self.1);
        }
    }

    /// Board #250 (validator 16:31), on the real delivery path: two
    /// multi-line messages to two DIFFERENT managed panes, posted at the same
    /// moment, each held between its paste buffer's load and paste. Each pane
    /// gets its own text and only its own; with the old shared buffer name one
    /// paste failed or landed in the other pane.
    #[test]
    fn concurrent_multiline_mentions_to_two_panes_each_arrive_whole() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-paste-hub-{}", uuid::Uuid::new_v4());
        let ws = std::env::temp_dir().join(format!("tmm-paste-hub-ws-{}", uuid::Uuid::new_v4()));
        for name in ["lead", "solo"] {
            let home = ws.join(".tmm/agents").join(name);
            std::fs::create_dir_all(&home).unwrap();
            std::fs::write(
                home.join("launch.json"),
                serde_json::json!({ "backend": "kiro", "cmd": format!("kiro-cli chat --agent {name}"), "team": "" }).to_string(),
            )
            .unwrap();
        }
        let created = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", &session, "-n", "lead", "-c", &ws.to_string_lossy(), "cat"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            eprintln!("no tmux server — skipping");
            let _ = std::fs::remove_dir_all(&ws);
            return;
        }
        // The session exists from here on: guard it before any later step
        // can panic (validator, #251 — a failed new-window used to leak it).
        let _cleanup = KillOnDrop(session.clone(), ws.clone());
        std::process::Command::new("tmux")
            .args(["new-window", "-d", "-t", &session, "-n", "solo", "-c", &ws.to_string_lossy(), "cat"])
            .status()
            .unwrap();
        crate::projects::adopt(&session, Some("paste-hub-test")).expect("adopt project");
        let posts: Vec<_> = ["lead", "solo"]
            .into_iter()
            .map(|name| {
                let session = session.clone();
                std::thread::spawn(move || {
                    crate::tmux::PASTE_GAP.with(|g| g.set(std::time::Duration::from_millis(150)));
                    handle_hub_request(
                        &req("hub_post", serde_json::json!({
                            "session": session, "from": "human",
                            "body": format!("@{name} first line for {name}\nsecond line for {name}"),
                        })),
                        None,
                    )
                })
            })
            .collect();
        for p in posts {
            let r = p.join().unwrap();
            assert!(r.error.is_none(), "{:?}", r.error.map(|e| e.message));
        }
        std::thread::sleep(std::time::Duration::from_millis(600));
        let lead = crate::tmux::capture_pane_plain(&format!("{session}:lead"), Some(0)).unwrap_or_default();
        let solo = crate::tmux::capture_pane_plain(&format!("{session}:solo"), Some(0)).unwrap_or_default();
        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(&ws);
        assert!(lead.contains("first line for lead") && lead.contains("second line for lead"), "lead: {lead:?}");
        assert!(solo.contains("first line for solo") && solo.contains("second line for solo"), "solo: {solo:?}");
        assert!(!lead.contains("for solo") && !solo.contains("for lead"), "no cross-pane text: {lead:?} / {solo:?}");
    }

    /// Board #250 (orchestrator 17:00/17:21), through hub_post: a message to
    /// an agent whose pane is in copy-mode is refused, not typed — the mode
    /// stays, no pending row is written, and ONE plain warn names the window,
    /// the reason and the line at once, with no delivery reference. The other recipient of the same message is
    /// delivered as usual.
    #[test]
    fn a_mention_to_a_pane_in_copy_mode_warns_at_once_and_owes_nothing() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-copymode-hub-{}", uuid::Uuid::new_v4());
        let ws = std::env::temp_dir().join(format!("tmm-copymode-hub-ws-{}", uuid::Uuid::new_v4()));
        for name in ["lead", "solo"] {
            let home = ws.join(".tmm/agents").join(name);
            std::fs::create_dir_all(&home).unwrap();
            std::fs::write(
                home.join("launch.json"),
                serde_json::json!({ "backend": "kiro", "cmd": format!("kiro-cli chat --agent {name}"), "team": "" }).to_string(),
            )
            .unwrap();
        }
        let created = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", &session, "-n", "lead", "-c", &ws.to_string_lossy(), "cat"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            eprintln!("no tmux server — skipping");
            let _ = std::fs::remove_dir_all(&ws);
            return;
        }
        // The session exists from here on: guard it before any later step
        // can panic (validator, #251 — a failed new-window used to leak it).
        let _cleanup = KillOnDrop(session.clone(), ws.clone());
        std::process::Command::new("tmux")
            .args(["new-window", "-d", "-t", &session, "-n", "solo", "-c", &ws.to_string_lossy(), "cat"])
            .status()
            .unwrap();
        crate::projects::adopt(&session, Some("copymode-hub-test")).expect("adopt project");
        let lead = format!("{session}:lead");
        crate::tmux::run_tmux(&["copy-mode", "-t", &lead]).unwrap();
        let r = handle_hub_request(
            &req("hub_post", serde_json::json!({ "session": session, "from": "human", "body": "@lead @solo read this" })),
            None,
        );
        assert!(r.error.is_none(), "{:?}", r.error.map(|e| e.message));
        let msg_id = r.result.as_ref().and_then(|m| m.get("id")).and_then(|v| v.as_str()).unwrap().to_string();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let still = crate::tmux::pane_in_mode(&lead).unwrap();
        let owed = crate::projects::telemetry::owed_message_ids(&session);
        let warns: Vec<_> = crate::projects::telemetry::recent_events(&session, 0).into_iter().filter(|e| e.kind == "warn").collect();
        crate::tmux::run_tmux(&["send-keys", "-t", &lead, "-X", "cancel"]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let lead_text = crate::tmux::capture_pane_plain(&lead, Some(0)).unwrap_or_default();
        let solo_text = crate::tmux::capture_pane_plain(&format!("{session}:solo"), Some(0)).unwrap_or_default();
        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(&ws);
        assert!(still, "copy-mode is never cancelled for a delivery");
        assert!(!lead_text.contains("read this"), "nothing typed into the reading pane: {lead_text:?}");
        assert!(solo_text.contains("read this"), "the other recipient still gets it: {solo_text:?}");
        assert_eq!(owed, vec![msg_id.clone()], "only solo's line is owed; lead's refused line is no pending row");
        assert_eq!(warns.len(), 1, "{warns:?}");
        assert_eq!(warns[0].window, "lead");
        assert!(warns[0].text.starts_with("undelivered (pane is in copy mode): "), "{}", warns[0].text);
        assert!(warns[0].text.contains("@lead @solo read this"), "the note carries the line: {}", warns[0].text);
        // No row, so no reference: `deliveries` names real rows only (#249),
        // and the wire form has no `deliveries` key at all.
        assert!(warns[0].deliveries.is_empty(), "{:?}", warns[0].deliveries);
        assert!(serde_json::to_value(&warns[0]).unwrap().get("deliveries").is_none());
    }

    /// Board #250 (validator 17:35): `hub_command` to `all` where one pane is
    /// in copy-mode and the other is not. The one that took it is recorded
    /// and answered as success; the refused one gets the same immediate plain
    /// warn as a refused chat line (target, reason, command; no deliveries),
    /// its mode kept, nothing typed. A single target that refuses stays an
    /// RPC error — nothing ran, so there is no success to repeat — and warns.
    #[test]
    fn a_command_to_all_with_one_pane_in_copy_mode_warns_for_it_and_runs_the_rest() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-copymode-cmd-{}", uuid::Uuid::new_v4());
        let ws = std::env::temp_dir().join(format!("tmm-copymode-cmd-ws-{}", uuid::Uuid::new_v4()));
        for name in ["lead", "solo"] {
            let home = ws.join(".tmm/agents").join(name);
            std::fs::create_dir_all(&home).unwrap();
            std::fs::write(
                home.join("launch.json"),
                serde_json::json!({ "backend": "kiro", "cmd": format!("kiro-cli chat --agent {name}"), "team": "" }).to_string(),
            )
            .unwrap();
        }
        let created = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", &session, "-n", "lead", "-c", &ws.to_string_lossy(), "cat"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            eprintln!("no tmux server — skipping");
            let _ = std::fs::remove_dir_all(&ws);
            return;
        }
        // The session exists from here on: guard it before any later step
        // can panic (validator, #251 — a failed new-window used to leak it).
        let _cleanup = KillOnDrop(session.clone(), ws.clone());
        std::process::Command::new("tmux")
            .args(["new-window", "-d", "-t", &session, "-n", "solo", "-c", &ws.to_string_lossy(), "cat"])
            .status()
            .unwrap();
        crate::projects::adopt(&session, Some("copymode-cmd-test")).expect("adopt project");
        let lead = format!("{session}:lead");
        crate::tmux::run_tmux(&["copy-mode", "-t", &lead]).unwrap();
        let all = handle_hub_request(&req("hub_command", serde_json::json!({ "session": session, "agent": "all", "text": "/compact now" })), None);
        let one = handle_hub_request(&req("hub_command", serde_json::json!({ "session": session, "agent": "lead", "text": "/clear" })), None);
        std::thread::sleep(std::time::Duration::from_millis(300));
        let still = crate::tmux::pane_in_mode(&lead).unwrap();
        let warns: Vec<_> = crate::projects::telemetry::recent_events(&session, 0).into_iter().filter(|e| e.kind == "warn").collect();
        let room: Vec<String> = handle_hub_request(&req("hub_log", serde_json::json!({ "session": session })), None)
            .result
            .and_then(|v| v.get("messages").and_then(|m| m.as_array()).cloned())
            .unwrap_or_default()
            .iter()
            .filter_map(|m| m.get("body").and_then(|b| b.as_str()).map(str::to_string))
            .collect();
        crate::tmux::run_tmux(&["send-keys", "-t", &lead, "-X", "cancel"]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(200));
        let lead_text = crate::tmux::capture_pane_plain(&lead, Some(0)).unwrap_or_default();
        let solo_text = crate::tmux::capture_pane_plain(&format!("{session}:solo"), Some(0)).unwrap_or_default();
        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(&ws);
        // all: the taker is the success, the refused one is a warn.
        assert!(all.error.is_none(), "{:?}", all.error.map(|e| e.message));
        assert_eq!(all.result.as_ref().and_then(|v| v.get("sent")).cloned(), Some(serde_json::json!(["solo"])));
        assert!(solo_text.contains("/compact now"), "{solo_text:?}");
        // single target, refused: an error, and a warn of its own.
        let err = one.error.expect("a single refused target is an error").message;
        assert!(err.contains("lead: pane is in copy mode"), "{err}");
        assert!(still, "copy-mode is never cancelled for a command");
        assert!(!lead_text.contains("/compact") && !lead_text.contains("/clear"), "nothing typed: {lead_text:?}");
        assert_eq!(room, vec!["[tmm] /compact now → solo".to_string()], "the room records only what ran");
        let texts: Vec<(&str, &str)> = warns.iter().map(|w| (w.window.as_str(), w.text.as_str())).collect();
        assert_eq!(
            texts,
            vec![("lead", "undelivered (pane is in copy mode): /compact now"), ("lead", "undelivered (pane is in copy mode): /clear")],
        );
        assert!(warns.iter().all(|w| w.deliveries.is_empty()), "no delivery reference");
    }

    /// Board #248, on the real delivery path: an in-word `@` (an email, `a@bob`)
    /// types into nobody and is stored with no recipient, while `(@bob)` —
    /// punctuation around a real address — reaches bob.
    #[test]
    fn an_in_word_at_delivers_to_nobody_but_a_bracketed_address_does() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-inword-{}", uuid::Uuid::new_v4());
        let ws = std::env::temp_dir().join(format!("tmm-inword-ws-{}", uuid::Uuid::new_v4()));
        let home = ws.join(".tmm/agents/bob");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("launch.json"),
            serde_json::json!({ "backend": "kiro", "cmd": "kiro-cli chat --agent bob", "team": "" }).to_string(),
        )
        .unwrap();
        let created = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", &session, "-n", "bob", "-c", &ws.to_string_lossy(), "cat"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            eprintln!("no tmux server — skipping");
            let _ = std::fs::remove_dir_all(&ws);
            return;
        }
        let _cleanup = KillOnDrop(session.clone(), ws.clone());
        crate::projects::adopt(&session, Some("inword-test")).expect("adopt project");
        let post = |body: &str| {
            let r = handle_hub_request(&req("hub_post", serde_json::json!({ "session": session, "from": "human", "body": body })), None);
            assert!(r.error.is_none(), "{:?}", r.error.map(|e| e.message));
            r.result.unwrap()
        };
        let email = post("mail me at a@bob.dev or ping a@bob");
        std::thread::sleep(std::time::Duration::from_millis(400));
        let after_email = crate::tmux::capture_pane_plain(&format!("{session}:bob"), Some(0)).unwrap_or_default();
        let bracket = post("(@bob) please look");
        std::thread::sleep(std::time::Duration::from_millis(400));
        let after_bracket = crate::tmux::capture_pane_plain(&format!("{session}:bob"), Some(0)).unwrap_or_default();
        assert!(!after_email.contains("mail me"), "an in-word @ types into nobody: {after_email:?}");
        assert_eq!(email.get("to"), Some(&serde_json::json!([])), "and is stored with no recipient: {email}");
        assert!(after_bracket.contains("(@bob) please look"), "a bracketed address reaches bob: {after_bracket:?}");
        assert_eq!(bracket.get("to"), Some(&serde_json::json!(["bob"])), "{bracket}");
    }

    #[test]
    fn mention_delivery_adds_context_only_to_the_team_member() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-team-context-{}", uuid::Uuid::new_v4());
        let ws = std::env::temp_dir().join(format!("tmm-team-context-ws-{}", uuid::Uuid::new_v4()));
        for (name, team) in [("lead", "content"), ("solo", "")] {
            let home = ws.join(".tmm/agents").join(name);
            std::fs::create_dir_all(&home).unwrap();
            std::fs::write(
                home.join("launch.json"),
                serde_json::json!({
                    "backend": "kiro",
                    "cmd": format!("kiro-cli chat --agent {name}"),
                    "team": team,
                })
                .to_string(),
            )
            .unwrap();
        }
        let created = std::process::Command::new("tmux")
            .args([
                "new-session", "-d", "-s", &session, "-n", "lead", "-c",
                &ws.to_string_lossy(), "cat",
            ])
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if !created {
            eprintln!("no tmux server — skipping");
            let _ = std::fs::remove_dir_all(&ws);
            return;
        }
        std::process::Command::new("tmux")
            .args(["new-window", "-d", "-t", &session, "-n", "solo", "-c", &ws.to_string_lossy(), "cat"])
            .status()
            .unwrap();
        crate::projects::adopt(&session, Some("team-context-test")).expect("adopt project");
        let room = project_room(&session);
        // Room history BEFORE the current message — the catch-up the team
        // member must receive. Stored routes are what the reconstruction reads.
        rooms::seed_msg(&room, &format!("{room}-1"), 1000, "human", &["lead".into()], "@lead old task");
        rooms::seed_msg(&room, &format!("{room}-2"), 2000, "writer", &["researcher".into()], "@researcher verify");
        rooms::seed_msg(&room, &format!("{room}-3"), 3000, "researcher", &[], "evidence ready");

        let response = handle_hub_request(
            &req("hub_post", serde_json::json!({
                "session": session, "from": "human", "body": "@lead @solo decide",
            })),
            None,
        );
        assert!(response.error.is_none(), "{:?}", response.error.map(|error| error.message));
        std::thread::sleep(std::time::Duration::from_millis(500));
        let lead = crate::tmux::capture_pane_plain(&format!("{session}:lead"), Some(0)).unwrap_or_default();
        let solo = crate::tmux::capture_pane_plain(&format!("{session}:solo"), Some(0)).unwrap_or_default();
        assert!(lead.contains("@lead @solo decide"), "lead receives the current request: {lead:?}");
        assert!(lead.contains("[tmm team context"), "team member receives catch-up: {lead:?}");
        assert!(lead.contains("writer -> @researcher"), "explicit route is named: {lead:?}");
        assert!(lead.contains("researcher -> @writer"), "automatic reply route is named: {lead:?}");
        assert!(!lead.contains("@lead @solo decide\n\n[tmm team context]\n[tmm chat"), "the current message is not part of its own catch-up");
        assert!(solo.contains("@lead @solo decide"), "solo receives the current request: {solo:?}");
        assert!(!solo.contains("[tmm team context"), "solo stays one-line: {solo:?}");
        // Each typed line names the message it carries (board #249), so its
        // echo can mark that message delivered on any client.
        let id = response.result.as_ref().and_then(|m| m.get("id")).and_then(|v| v.as_str()).unwrap().to_string();
        let owed = crate::projects::telemetry::owed_message_ids(&session);
        assert_eq!(owed, vec![id.clone(), id], "one row per typed pane, each naming the message");

        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// A Board reply is first persisted, then delivered into the assigned
    /// managed pane. This real-tmux edge pins the part a pure decision test
    /// cannot: the note actually reaches INPUT through `deliver_chat_line`.
    #[test]
    fn a_board_reply_is_persisted_and_typed_into_the_assignees_pane() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-board-reply-{}", uuid::Uuid::new_v4());
        let ws = std::env::temp_dir().join(format!("tmm-board-reply-ws-{}", uuid::Uuid::new_v4()));
        let home = ws.join(".tmm/agents/dev");
        std::fs::create_dir_all(&home).unwrap();
        std::fs::write(
            home.join("launch.json"),
            r#"{"backend":"kiro","cmd":"kiro-cli chat --agent dev"}"#,
        )
        .unwrap();
        let created = std::process::Command::new("tmux")
            .args([
                "new-session", "-d", "-s", &session, "-n", "dev", "-c",
                &ws.to_string_lossy(), "cat",
            ])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            eprintln!("no tmux server — skipping");
            let _ = std::fs::remove_dir_all(&ws);
            return;
        }
        crate::projects::adopt(&session, Some("board-reply-test")).expect("adopt test project");
        let issue_id = crate::projects::board_save(
            &session,
            None,
            Some("Retry edge"),
            Some("Keep the receipt semantics"),
            None,
            Some("dev"),
            "human",
        )
        .unwrap();

        let r = handle_hub_request(
            &req("hub_board_note", serde_json::json!({
                "session": session, "id": issue_id, "who": "human",
                "body": "Please cover the restart case",
            })),
            None,
        );
        assert!(r.error.is_none(), "{:?}", r.error.map(|e| e.message));

        let issue = crate::projects::board_get(&session, issue_id).unwrap();
        assert_eq!(issue["notes"][0]["body"], "Please cover the restart case");
        std::thread::sleep(std::time::Duration::from_millis(300));
        let pane = crate::tmux::capture_pane_plain(&format!("{session}:dev"), Some(0)).unwrap_or_default();
        assert!(pane.contains(&format!("[board #{issue_id} reply] Retry edge")), "pane: {pane:?}");
        assert!(pane.contains("Please cover the restart case"), "pane: {pane:?}");

        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// Stop/restart act on a process, so the gate is the same one delivery and
    /// auto-post use: only agents this app started. The room records the act.
    #[test]
    fn stopping_a_managed_agent_kills_the_window_and_says_so() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-stop-{}", std::process::id());
        let ws = std::env::temp_dir().join(format!("tmm-stop-ws-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(ws.join(".tmm/agents/dev")).unwrap();
        std::fs::write(ws.join(".tmm/agents/dev/launch.json"), "{}").unwrap();
        // Start it IN the workspace: `adopt` derives the project path from the
        // panes' cwd, so this is what makes managed_home resolve to <ws>/.tmm.
        let created = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", &session, "-n", "dev", "-c",
                   &ws.to_string_lossy(), "sleep 60"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            eprintln!("no tmux server — skipping");
            return;
        }
        // A project must claim the session for managed_home to resolve.
        let created_project = crate::projects::adopt(&session, Some("stop-test")).is_ok();

        let r = handle_hub_request(
            &req("hub_agent_stop", serde_json::json!({ "session": session, "agent": "dev" })),
            None,
        );
        if !created_project {
            eprintln!("could not adopt a project — skipping the positive half");
        } else {
            assert!(r.error.is_none(), "{:?}", r.error.map(|e| e.message));
            let panes = crate::tmux::list_panes(&session).unwrap_or_default();
            assert!(!panes.iter().any(|p| p.window_name == "dev"), "the window is gone");
            let msgs = rooms::history_page(&project_room(&session), None, 10);
            let msgs = msgs["messages"].as_array().unwrap();
            assert!(
                msgs.iter().any(|m| m["body"].as_str().unwrap_or("").contains("[tmm] stopped dev")),
                "the room records it: {msgs:?}"
            );
        }
        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(&ws);
    }

    /// Interrupt, against real tmux: the window SURVIVES (Escape cancels a
    /// turn, it does not kill a process), the derived state is reset to `idle`
    /// BEFORE the key is typed, and the room records the act — the feed row is
    /// half of the composer's interrupt affordance (owner, 2026-08-24: "发送
    /// interrupt 的状态在消息列表里也要展示出来").
    #[test]
    fn interrupting_a_managed_agent_leaves_the_window_and_says_so() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-int-{}", std::process::id());
        let ws = std::env::temp_dir().join(format!("tmm-int-ws-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(ws.join(".tmm/agents/dev")).unwrap();
        std::fs::write(ws.join(".tmm/agents/dev/launch.json"), "{}").unwrap();
        let created = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", &session, "-n", "dev", "-c",
                   &ws.to_string_lossy(), "sleep 60"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            eprintln!("no tmux server — skipping");
            return;
        }
        let created_project = crate::projects::adopt(&session, Some("int-test")).is_ok();
        // A turn is OPEN on that window: the state we are interrupting.
        telemetry::record_prompt(&session, "dev", "do the long thing");
        assert_eq!(telemetry::derive(&session, "dev", 0).state, "running");

        let r = handle_hub_request(
            &req("hub_agent_interrupt", serde_json::json!({ "session": session, "agent": "dev" })),
            None,
        );
        if !created_project {
            eprintln!("could not adopt a project — skipping the positive half");
        } else {
            assert!(r.error.is_none(), "{:?}", r.error.map(|e| e.message));
            let panes = crate::tmux::list_panes(&session).unwrap_or_default();
            assert!(panes.iter().any(|p| p.window_name == "dev"), "the window survives an interrupt");
            assert_eq!(
                telemetry::derive(&session, "dev", 0).state,
                "idle",
                "the cancelled turn is closed by the interrupt itself — no stop hook is coming"
            );
            let msgs = rooms::history_page(&project_room(&session), None, 10);
            let msgs = msgs["messages"].as_array().unwrap();
            assert!(
                msgs.iter().any(|m| m["body"].as_str().unwrap_or("").contains("[tmm] interrupted dev")),
                "the room records it: {msgs:?}"
            );
        }
        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn hub_log_since_ts_filters_older_messages() {
        crate::projects::tests::use_test_store();
        let (session, room) = unique("since");
        rooms::seed_msg(&room, &format!("{room}-1"), 100, "a", &[], "old");
        rooms::seed_msg(&room, &format!("{room}-2"), 200, "b", &[], "new");
        let r = handle_hub_request(
            &req("hub_log", serde_json::json!({ "session": session, "since_ts": 100 })),
            None,
        );
        let msgs = r.result.unwrap();
        let msgs = msgs.get("messages").and_then(|m| m.as_array()).unwrap();
        assert_eq!(msgs.len(), 1, "only the ts>100 message survives");
        assert_eq!(msgs[0].get("body").and_then(|b| b.as_str()), Some("new"));
    }

    /// Review C (2026-09-03): an incremental poll takes the newest `limit` rows
    /// and THEN drops what is older than `since_ts`. When more than `limit`
    /// messages arrived since the cursor, the older ones were never on the page
    /// and the client's cursor jumped past them — a permanent hole. On a since_ts
    /// query `has_more` therefore means "newer-than-since_ts rows remain behind
    /// this page", and the client walks `before_seq` until it reaches the cursor.
    #[test]
    fn hub_log_since_ts_reports_when_the_page_did_not_reach_the_cursor() {
        crate::projects::tests::use_test_store();
        // 250 messages, ts = n * 10. The client last saw ts 1000 (the 100th):
        // 150 messages are newer, one page holds 100.
        let (session, room) = unique("cursor");
        let seeded: Vec<serde_json::Value> = (1..=250)
            .map(|n| rooms::seed_msg(&room, &format!("{room}-m{n}"), n * 10, "human", &[], format!("b{n}").as_str()))
            .collect();
        let seq = |n: usize| seeded[n - 1]["seq"].as_i64().unwrap();
        let r = handle_hub_request(
            &req("hub_log", serde_json::json!({ "session": session, "since_ts": 1000, "limit": 100 })),
            None,
        );
        let v = r.result.expect("result");
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 100, "the newest page, all newer than the cursor");
        assert_eq!(msgs[0]["seq"], seq(151));
        assert_eq!(v["has_more"], true, "rows 101..150 are newer than the cursor and NOT on this page");
        assert_eq!(v["oldest_seq"], seq(151), "the cursor for the walk back");

        // The walk back: the page behind the 151st, still bounded by since_ts.
        let r = handle_hub_request(
            &req("hub_log", serde_json::json!({ "session": session, "since_ts": 1000, "limit": 100, "before_seq": seq(151) })),
            None,
        );
        let v = r.result.expect("result");
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 50, "rows 101..150 survive the since_ts filter");
        assert_eq!(msgs[0]["seq"], seq(101));
        assert_eq!(v["has_more"], false, "the raw page reached back past the cursor: nothing newer remains");

        // A poll whose page reaches the cursor says so even when the room has
        // plenty of older history behind it.
        let r = handle_hub_request(
            &req("hub_log", serde_json::json!({ "session": session, "since_ts": 2400, "limit": 100 })),
            None,
        );
        let v = r.result.expect("result");
        assert_eq!(v["messages"].as_array().unwrap().len(), 10);
        assert_eq!(v["has_more"], false, "everything newer than the cursor is on this page");

        // And a first load (no since_ts) keeps the paging meaning: history remains.
        let r = handle_hub_request(&req("hub_log", serde_json::json!({ "session": session, "limit": 100 })), None);
        assert_eq!(r.result.expect("result")["has_more"], true);
    }

    /// `crate::projects` is compiled out on android/ios, so every item in
    /// these files that reads it MUST carry the desktop cfg gate. Nothing in
    /// the normal loop catches a missing one: `cargo test`, `cargo build` and
    /// the dev server all target the desktop, where the module exists — the
    /// error only appears in `npm run build:android`, which nobody runs per
    /// change. It broke exactly that way (board #16): `deliver_chat_line` was
    /// inserted directly beneath `deliver_mentions`' `#[cfg]`, adopted the
    /// gate (a doc comment between an attribute and its item is legal, so the
    /// attribute binds to whatever item follows), and left `deliver_mentions`
    /// ungated — 10 errors, two commits before anyone noticed. `chrono` is a
    /// desktop-gated dependency exactly like `crate::projects` (Cargo.toml
    /// target block) and broke the build the same silent way on 2026-09-09
    /// (context_stamp/delivered_chat_line, found by the board #100 smoke
    /// build).
    ///
    /// Since board #129 the same contract holds for `src/backends/*.rs` (an
    /// UNGATED leaf: the enum and the hook payload dialects compile on the
    /// phone, while render/sniff/refresh/known reach into `crate::projects`
    /// and gate themselves item by item) and for `agent_notifications.rs`
    /// (the inbox consumer the phone compiles). So the guard is a source
    /// contract, checked on the desktop where it is cheap, instead of a
    /// cross-compile nobody runs. Negative control on 2026-09-09: removing
    /// the gate above `kiro::render_kiro` failed this test naming it.
    #[test]
    fn projects_readers_are_desktop_gated() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut files: Vec<std::path::PathBuf> =
            vec![root.join("server/hub_rpc.rs"), root.join("agent_notifications.rs")];
        let mut backends: Vec<_> = std::fs::read_dir(root.join("backends"))
            .expect("src/backends exists")
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|x| x == "rs"))
            .collect();
        backends.sort();
        assert!(backends.len() >= 6, "expected the five backend files + mod/shared, found {}", backends.len());
        files.extend(backends);

        let mut total = 0usize;
        let mut ungated: Vec<String> = Vec::new();
        for path in &files {
            let src = std::fs::read_to_string(path).unwrap();
            let (checked, bad) = ungated_desktop_readers(&src);
            total += checked;
            let name = path.strip_prefix(&root).unwrap().display().to_string();
            ungated.extend(bad.into_iter().map(|l| format!("{name}: {l}")));
        }

        assert!(
            ungated.is_empty(),
            "these read crate::projects / chrono with no desktop gate, so they break the Android build: {ungated:#?}"
        );
        // A guard that silently stops finding anything is not a guard. hub_rpc
        // alone holds the three delivery helpers the regression hit, and every
        // backend file gates at least its renderer; if the scan stops matching,
        // this count is the tripwire that says so.
        assert!(
            total >= 12,
            "expected at least 12 gated projects readers across these files, found {total} — did the scan stop matching?"
        );
    }

    /// The scan behind `projects_readers_are_desktop_gated`: every top-level
    /// item (`fn`, `use`, `impl` method) whose text reaches `crate::projects`
    /// or `chrono::` must have the desktop gate somewhere in the contiguous
    /// run of attributes and comments above it — the rule is "gated", not
    /// "gated on a particular line". Column-0 items and the methods of a
    /// column-0 `impl` are checked; anything deeper inherits its parent's
    /// gate, and the file's `#[cfg(test)]` module (column 0, always last) ends
    /// the scan. A gate on the item's own statements is also accepted (the
    /// inbox consumer's shape). Returns (items checked, offending signatures).
    fn ungated_desktop_readers(src: &str) -> (usize, Vec<String>) {
        const GATE: &str = "target_os = \"android\"";
        let lines: Vec<&str> = src.lines().collect();
        let end = lines.iter().position(|l| l.starts_with("#[cfg(test)]")).unwrap_or(lines.len());
        let is_fn = |t: &str| {
            ["fn ", "pub fn ", "pub(crate) fn ", "pub(super) fn "].iter().any(|p| t.starts_with(p))
        };
        let reaches = |text: &str| text.contains("crate::projects") || text.contains("chrono::");

        let mut checked = 0usize;
        let mut ungated = Vec::new();
        let mut depth = 0i32;
        let mut in_impl = false;
        for (i, line) in lines[..end].iter().enumerate() {
            let indent = line.len() - line.trim_start().len();
            let t = line.trim_start();
            if depth == 0 && t.starts_with("impl ") {
                in_impl = true;
            }
            let top_use = depth == 0 && (t.starts_with("use ") || t.starts_with("pub use ") || t.starts_with("pub(crate) use "));
            let item_fn = is_fn(t) && (depth == 0 || (depth == 1 && in_impl && indent == 4));
            if top_use || item_fn {
                // The item's text: a `use` is its line(s) up to `;`, a fn runs
                // until its braces balance again.
                let mut body = String::new();
                let mut d = 0i32;
                for l in &lines[i..] {
                    body.push_str(l);
                    body.push('\n');
                    d += l.chars().filter(|c| *c == '{').count() as i32;
                    d -= l.chars().filter(|c| *c == '}').count() as i32;
                    if top_use && l.contains(';') && d <= 0 {
                        break;
                    }
                    if item_fn && d <= 0 && body.contains('{') {
                        break;
                    }
                }
                if reaches(&body) {
                    checked += 1;
                    // A body that gates its own statements (`consume_file`
                    // reaches telemetry only under `#[cfg]` blocks) is the
                    // other legal shape; the cross-compile is what proves it
                    // covers every reference, this guard only rejects the
                    // wholly ungated item.
                    let mut gated = body.contains(GATE);
                    for l in lines[..i].iter().rev() {
                        let a = l.trim_start();
                        if !(a.starts_with('#') || a.starts_with("//")) {
                            break;
                        }
                        if a.contains(GATE) {
                            gated = true;
                            break;
                        }
                    }
                    if !gated {
                        ungated.push(t.to_string());
                    }
                }
            }
            depth += line.chars().filter(|c| *c == '{').count() as i32;
            depth -= line.chars().filter(|c| *c == '}').count() as i32;
            if depth <= 0 {
                in_impl = false;
            }
        }
        (checked, ungated)
    }
}
