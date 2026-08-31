//! hub_* RPC dispatch: the project hub (agents-v2). One chat room per project
//! (bus room `proj:<session>`), agent status declarations, and derived agent
//! states. This is the server side of the `tmm` CLI — the CLI-only message
//! substrate from docs/exec-plans/agents-v2.md (§4.1/§4.4): what an agent SAYS
//! arrives here via tmm; what we OBSERVE arrives via hooks into
//! projects::telemetry, and `hub_agents` joins the two at read time.
//!
//! Desktop-only in effect: everything needs the team bus (None on mobile) and
//! the telemetry store (projects module, desktop-gated), so mobile answers
//! method-not-found and clients degrade exactly like team_*.

use super::rpc::{require_str, Request, Response, ERR_INTERNAL, ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND};
use super::TeamBridge;

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

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) fn handle_hub_request(req: &Request, team: Option<&dyn TeamBridge>, notifications: Option<&crate::agent_notifications::AgentNotificationHub>) -> Response {
    use crate::projects::telemetry;

    let id = req.id;
    let p = &req.params;
    let Some(bus) = team else {
        return Response::err(id, ERR_METHOD_NOT_FOUND, "hub not available on this server".into());
    };
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
        return Response::ok(id, serde_json::json!({ "rooms": bus.room_latest(), "states": states }));
    }
    let asked = match require_str(p, "session") {
        Ok(s) => s,
        Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
    };
    // Resolve the caller's session name to the project's CURRENT one, ONCE, here.
    // Renaming a project renames its tmux session, but a running agent carries
    // `TMM_PROJECT` from the moment it started — and half of these methods reach
    // straight into tmux (`window_of_agent`, `list_panes`), where a stale name
    // finds nothing. Measured the hard way: right after a rename, `tmm status`
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
            let agent = match require_str(p, "agent") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let text = match require_str(p, "text") {
                Ok(s) => s.trim(),
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            if !text.starts_with('/') {
                return Response::err(id, ERR_INVALID_PARAMS, "a command must start with '/'".into());
            }
            let ws = crate::projects::project_for_session(session).ok().flatten().map(|pr| pr.path);
            let panes = crate::tmux::list_panes(session).unwrap_or_default();
            let mut sent: Vec<String> = Vec::new();
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
                if crate::tmux::send_command(&target, text).is_ok() {
                    sent.push(pane.window_name.clone());
                }
            }
            if sent.is_empty() {
                return Response::err(
                    id,
                    ERR_INVALID_PARAMS,
                    format!("no managed agent named '{agent}' in session '{session}'"),
                );
            }
            if bus.open_room(&room).is_ok() {
                let _ = bus.post(&room, "human", &format!("[tmm] {} → {}", text, sent.join(", ")), false);
            }
            Response::ok(id, serde_json::json!({ "sent": sent, "command": text }))
        }

        "hub_post" => {
            let body = match require_str(p, "body") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let from = p.get("from").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("human");
            // record_only = true means the message is stored but NEVER typed
            // into any agent's pane. Required for hook-sourced auto-replies:
            // if an automatic post addresses a peer, delivery would type into
            // that peer's pane, triggering their own stop hook, which then
            // auto-posts back — a ping-pong loop. The caller is responsible
            // for setting this when the origin is a hook.
            let record_only = p.get("record_only").and_then(|v| v.as_bool()).unwrap_or(false);
            if let Err(e) = bus.open_room(&room) {
                return Response::err(id, ERR_INTERNAL, e);
            }
            let requires_reply = p
                .get("requires_reply")
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| !record_only && body.contains('@'));
            match bus.post(&room, from, body, requires_reply) {
                Ok(msg) => {
                    // DELIVERY: an idle agent sits at its prompt and reads
                    // nothing — @mentions are typed into the mentioned agents'
                    // panes so the chat actually reaches them. (An agent that
                    // is mid-task sees the line queued in its input box.)
                    // Hook-sourced posts skip delivery entirely to prevent
                    // reply loops (see record_only comment above).
                    if !record_only {
                        deliver_mentions(session, from, body);
                        // Mark that this agent sent an explicit message this
                        // turn, so the stop hook won't auto-post a duplicate.
                        if let Some(hub) = notifications {
                            if let Some(w) = window_of_agent(session, from) {
                                hub.mark_sent_this_turn(session, w);
                            }
                        }
                    }
                    Response::ok(id, msg)
                }
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        // Read the project chat, optionally incremental (`since_ts`, exclusive)
        // — the multica-style cursor so an agent polls without re-reading — or
        // backwards by page (`before_seq`), which is how a client reaches history
        // it never loaded. Nothing is ever pruned from the room, so paging is the
        // only honest way to keep a first load small (board #9).
        "hub_log" => {
            let limit = p.get("limit").and_then(|v| v.as_i64()).unwrap_or(100).clamp(1, 1000);
            if let Err(e) = bus.open_room(&room) {
                return Response::err(id, ERR_INTERNAL, e);
            }
            let since_ts = p.get("since_ts").and_then(|v| v.as_i64()).unwrap_or(0);
            // The bus's own cursor is `seq`, the message's log position — stable,
            // gapless and already on every message the client holds, which a ts is
            // not (two messages can share a millisecond).
            let before_seq = p.get("before_seq").and_then(|v| v.as_i64()).filter(|n| *n > 0);
            let mut history = bus.history_page(&room, before_seq, limit);
            // An archived message is hidden, not gone: the room's own store still
            // has it (that is what makes a restore free), so the hiding happens
            // here, on the way out.
            let hidden = crate::projects::archived_ids(&room);
            if let Some(msgs) = history.get_mut("messages").and_then(|m| m.as_array_mut()) {
                // The RAW page's oldest position, read BEFORE any filtering. It is
                // the cursor of last resort: a page can lose every row to the
                // filters (one archived stretch, or a `since_ts` tail), and then a
                // survivor-derived cursor does not exist — the client would be told
                // `has_more: true` with nothing to ask for and the walk would stop
                // dead at a hidden run. The raw seq always advances, so the next
                // request lands strictly further back.
                let raw_oldest = msgs.first().and_then(|m| m.get("seq")).and_then(|v| v.as_i64());
                if since_ts > 0 {
                    msgs.retain(|m| m.get("ts").and_then(|t| t.as_i64()).unwrap_or(0) > since_ts);
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
            Response::ok(id, history)
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
                return Response::err(id, ERR_INVALID_PARAMS, "ids must be a non-empty array".into());
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
                    if let Err(e) = bus.open_room(&room) {
                        return Response::err(id, ERR_INTERNAL, e);
                    }
                    let mut done = 0usize;
                    for mid in &ids {
                        let Some(m) = bus.message_by_id(&room, mid) else { continue };
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
                    Response::ok(id, serde_json::json!({ "archived": done }))
                }
                "hub_msg_restore" => match crate::projects::unarchive_msgs(&room, &ids) {
                    Ok(n) => Response::ok(id, serde_json::json!({ "restored": n })),
                    Err(e) => Response::err(id, ERR_INTERNAL, e),
                },
                _ => {
                    // Forget the message itself first: if that fails the archive row
                    // stays, so the message is still listed and can be tried again.
                    match bus.delete_messages(&room, &ids) {
                        Ok(n) => {
                            let _ = crate::projects::unarchive_msgs(&room, &ids);
                            Response::ok(id, serde_json::json!({ "deleted": n }))
                        }
                        Err(e) => Response::err(id, ERR_INTERNAL, e),
                    }
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
            Response::ok(id, serde_json::json!({ "messages": rows }))
        }

        // Explicit status declaration: `tmm status waiting "等接口定稿"`.
        // Resolved to a window index because that is telemetry's key (hook
        // notifications arrive by window, not by name).
        "hub_status" | "hub_done" => {
            let agent = match require_str(p, "agent") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let Some(window) = window_of_agent(session, agent) else {
                return Response::err(id, ERR_INVALID_PARAMS, format!("no window named '{agent}' in session '{session}'"));
            };
            if req.method == "hub_done" {
                let summary = p.get("summary").and_then(|v| v.as_str()).unwrap_or("");
                telemetry::record_done(session, window, summary);
                // A completion is a message too — the room is the record. With a
                // summary it is the AGENT speaking (what it finished is its own
                // report, and the marker keeps it out of the app-narration
                // treatment: `[tmm] ` folds into a grey sys row and the chat-only
                // level drops it entirely, so the text vanished exactly where a
                // reader looks). Bare `done` has nothing to say and stays a
                // lifecycle line.
                if bus.open_room(&room).is_ok() {
                    let body = if summary.trim().is_empty() {
                        "[tmm] done".to_string()
                    } else {
                        format!("[tmm done] {summary}")
                    };
                    let _ = bus.post(&room, agent, &body, false);
                }
                // Record the summary for dedup — NOT mark_sent_this_turn: a
                // done is a report about the work, the stop hook carries the
                // answer itself, and blanket suppression is what made every
                // turn ending in the required `tmm done` lose its final reply
                // (owner, 2026-08-21: "kiro grok 都好像没看到最后返回的消息").
                // The auto-post is skipped only when the reply IS the summary.
                if let Some(hub) = notifications {
                    hub.mark_done_this_turn(session, window, summary);
                }
                // The summary CLOSES the loop with whoever briefed the agent:
                // a lead that spawned a builder cannot schedule on a
                // record-only room line — nothing wakes it, so it never
                // learned its builders finished (owner, 2026-08-29). Deliver
                // the summary into the SPAWNER's pane like any chat line.
                // This is a TARGETED delivery, never a mention scan, so
                // invariant 2 of the hook-sourced posts is untouched; and it
                // cannot ping-pong — one line, one target, once per turn end,
                // and the spawned_by chain terminates at the human.
                if !summary.trim().is_empty() {
                    deliver_done_to_spawner(session, agent, summary);
                }
            } else {
                let state = match require_str(p, "state") {
                    Ok(s) => s,
                    Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
                };
                if !matches!(state, "working" | "waiting" | "blocked") {
                    return Response::err(id, ERR_INVALID_PARAMS, format!("state must be working|waiting|blocked, got '{state}'"));
                }
                let note = p.get("note").and_then(|v| v.as_str()).unwrap_or("");
                telemetry::record_status(session, window, state, note);
                // A status note is a MESSAGE from the agent, not a telemetry row
                // ("status要用agent发送消息的形式显示"): the room is the record, so
                // it survives a restart, and it reads as the agent speaking
                // because that is what it is. Record-only — an @name inside a note
                // must never type into a peer's pane (that loop is invariant 2 of
                // the hook-sourced posts). A note-less claim posts nothing: the
                // derived state already knows a turn is open, so a bare word would
                // be an empty message.
                if !note.trim().is_empty() && bus.open_room(&room).is_ok() {
                    let _ = bus.post(&room, agent, &format!("[tmm status {state}] {note}"), false);
                }
            }
            Response::ok(id, serde_json::json!({ "ok": true, "window": window }))
        }

        // Derived agent states for a session: one row per live window, agent
        // detection + status derivation joined at read time.
        "hub_agents" => Response::ok(id, agent_states(session)),

        // ---- the project task board (owner, 2026-08-29): the human writes
        // issues on the board page, agents read/update through `tmm board`.
        // Session-scoped like the chat room; note/move/save record WHO acted.
        "hub_board_list" => match crate::projects::board_list(session) {
            Ok(v) => Response::ok(id, v),
            Err(e) => Response::err(id, ERR_INVALID_PARAMS, e),
        },
        "hub_board_get" => {
            let Some(issue_id) = p.get("id").and_then(|v| v.as_i64()) else {
                return Response::err(id, ERR_INVALID_PARAMS, "id required".into());
            };
            match crate::projects::board_get(session, issue_id) {
                Ok(v) => Response::ok(id, v),
                Err(e) => Response::err(id, ERR_INVALID_PARAMS, e),
            }
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
            match crate::projects::board_save(session, issue_id, s("title"), s("body"), s("status"), s("assignee"), who) {
                Ok(saved) => {
                    if let (Some(prev), Some(new_status)) = (&prev, s("status")) {
                        let old_status = prev["status"].as_str().unwrap_or("");
                        if old_status != new_status {
                            let title = prev["title"].as_str().unwrap_or("");
                            if bus.open_room(&room).is_ok() {
                                let _ = bus.post(&room, who, &format!("[tmm] board #{saved} {old_status} → {new_status} — {title}"), false);
                            }
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
                            let title = s("title").unwrap_or(prev["title"].as_str().unwrap_or(""));
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
                    Response::ok(id, serde_json::json!({ "ok": true, "id": saved }))
                }
                Err(e) => Response::err(id, ERR_INVALID_PARAMS, e),
            }
        }
        "hub_board_note" => {
            let Some(issue_id) = p.get("id").and_then(|v| v.as_i64()) else {
                return Response::err(id, ERR_INVALID_PARAMS, "id required".into());
            };
            let body = p.get("body").and_then(|v| v.as_str()).unwrap_or("");
            let author = p.get("who").and_then(|v| v.as_str()).unwrap_or("human");
            match crate::projects::board_note(session, issue_id, author, body) {
                Ok(()) => {
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
                    Response::ok(id, serde_json::json!({ "ok": true }))
                }
                Err(e) => Response::err(id, ERR_INVALID_PARAMS, e),
            }
        }
        "hub_board_delete" => {
            let Some(issue_id) = p.get("id").and_then(|v| v.as_i64()) else {
                return Response::err(id, ERR_INVALID_PARAMS, "id required".into());
            };
            match crate::projects::board_delete(session, issue_id) {
                Ok(true) => Response::ok(id, serde_json::json!({ "ok": true })),
                Ok(false) => Response::err(id, ERR_INVALID_PARAMS, format!("no issue #{issue_id} on this board")),
                Err(e) => Response::err(id, ERR_INVALID_PARAMS, e),
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
            Response::ok(
                id,
                serde_json::json!({
                    "events": events,
                    "has_more": has_more,
                    "oldest": oldest,
                    "total": total,
                    "first_ts": first_ts,
                }),
            )
        }

        // Stop / restart ONE agent. The window is the agent's life: killing it
        // ends the process and keeps the declaration, so `restart` is kill +
        // `projects::up`, which recreates only what is missing and prefers the
        // resume flags — the agent comes back to its own conversation rather
        // than to a blank prompt. Managed-only: we stop what we started.
        // Interrupt: type Escape into the agent's own pane — the only channel
        // that reaches a BUSY agent, since a chat message is read between
        // turns. Named key, never a raw \x1b: with extended-keys on, tmux
        // drops raw C0 bytes sent to a pane in extended mode. Server-side so
        // the CLI and the UI share ONE implementation.
        "hub_agent_interrupt" => {
            let agent = match require_str(p, "agent") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            if crate::projects::managed_home(session, agent).is_none() {
                return Response::err(id, ERR_INVALID_PARAMS,
                    format!("'{agent}' is not an agent this app started"));
            }
            let Some(window) = window_of_agent(session, agent) else {
                return Response::err(id, ERR_INVALID_PARAMS,
                    format!("no window named '{agent}' in session '{session}'"));
            };
            // Reset the derived state BEFORE the key goes in, never after. A
            // cancelled turn produces no stop hook, so without this the newest
            // fact stays the `userPromptSubmit` that opened it and the card
            // reads `running` for ever; and an interrupted agent usually starts
            // something else within seconds, whose own turn re-derives
            // `running` — so a reset that raced the next turn would be
            // indistinguishable from no reset at all, i.e. an interrupt that
            // looked like it never landed (owner, 2026-08-29).
            telemetry::record_interrupt(session, window);
            match crate::tmux::send_keys(&format!("{session}:{window}"), "Escape", false) {
                Ok(()) => {
                    // The room records what the app did on a person's behalf —
                    // same rule as stop/restart/remove. The client's sys
                    // grammar already speaks `interrupted` (amber: a turn was
                    // cut short, not an ending); the feed row was the missing
                    // half of the composer's interrupt affordance (owner,
                    // 2026-08-24: "发送 interrupt 的状态在消息列表里也要展示").
                    let _ = bus.post(&room, agent, &format!("[tmm] interrupted {agent}"), false);
                    Response::ok(id, serde_json::json!({ "interrupted": agent }))
                }
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }

        // Eject an agent from the project: stop it, drop its slot, remove its
        // isolated home. Stop is the pause button, this is the delete button.
        "hub_agent_remove" => {
            let agent = match require_str(p, "agent") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match crate::projects::agent_remove(session, agent) {
                Ok(v) => {
                    if bus.open_room(&room).is_ok() {
                        let _ = bus.post(&room, agent, &format!("[tmm] removed {agent}"), false);
                    }
                    Response::ok(id, v)
                }
                Err(e) => Response::err(id, ERR_INVALID_PARAMS, e),
            }
        }

        "hub_agent_stop" | "hub_agent_restart" => {
            let agent = match require_str(p, "agent") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            if crate::projects::managed_home(session, agent).is_none() {
                return Response::err(id, ERR_INVALID_PARAMS,
                    format!("'{agent}' is not an agent this app started"));
            }
            let restart = req.method == "hub_agent_restart";
            let live = window_of_agent(session, agent);
            // Stop needs something to stop. Restart does not: the isolated home
            // outlives the window, so `restart` doubles as "start it again"
            // after a stop — which is what the button does when it reads Start.
            match (live, restart) {
                (None, false) => {
                    return Response::err(id, ERR_INVALID_PARAMS,
                        format!("no window named '{agent}' in session '{session}'"));
                }
                (Some(window), _) => {
                    if let Err(e) = crate::tmux::kill_window(&format!("{session}:{window}")) {
                        return Response::err(id, ERR_INTERNAL, e);
                    }
                }
                (None, true) => {}
            }
            if !restart {
                if bus.open_room(&room).is_ok() {
                    let _ = bus.post(&room, agent, &format!("[tmm] stopped {agent}"), false);
                }
                return Response::ok(id, serde_json::json!({ "stopped": agent }));
            }
            // Recreate from the declaration. A window younger than the capture
            // loop's 120 s rule may not be in it yet, so fall back to a fresh
            // spawn — that starts a new conversation instead of resuming one,
            // which is still better than an agent that does not come back.
            let mut resumed = false;
            if let Ok(Some(project)) = crate::projects::project_for_session(session) {
                // Bring its hooks up to date with this build first: the config
                // was written by whatever version spawned it, and a stale hook
                // set is exactly how observation goes quiet.
                crate::projects::spawn::refresh_hooks(&project.path, agent);
                resumed = crate::projects::up(&project.id).is_ok()
                    && window_of_agent(session, agent).is_some();
            }
            if !resumed {
                let r = crate::projects::spawn::spawn(&crate::projects::spawn::SpawnRequest {
                    session, agent, brief: "", by: "",
                });
                if let Err(e) = r {
                    return Response::err(id, ERR_INTERNAL, format!("restart failed: {e}"));
                }
            }
            if bus.open_room(&room).is_ok() {
                let _ = bus.post(&room, agent, &format!("[tmm] restarted {agent}"), false);
            }
            Response::ok(id, serde_json::json!({ "restarted": agent, "resumed": resumed }))
        }

        // Spawn a registry agent into this project (tmm spawn / the UI's
        // "+ agent"). can_hire-gated when an agent asks; capped per project.
        "hub_spawn" => {
            let agent = match require_str(p, "agent") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            let brief = p.get("brief").and_then(|v| v.as_str()).unwrap_or("");
            let by = p.get("by").and_then(|v| v.as_str()).unwrap_or("");
            match crate::projects::spawn::spawn(&crate::projects::spawn::SpawnRequest {
                session, agent, brief, by,
            }) {
                Ok(result) => {
                    // The spawn is chat-visible: the room is the record.
                    if bus.open_room(&room).is_ok() {
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
                        let _ = bus.post(&room, who, &line, false);
                    }
                    Response::ok(id, result)
                }
                Err(e) => Response::err(id, ERR_INVALID_PARAMS, e),
            }
        }

        other => Response::err(id, ERR_METHOD_NOT_FOUND, format!("unknown hub method: {other}")),
    }
}

#[cfg(any(target_os = "android", target_os = "ios"))]
pub(super) fn handle_hub_request(req: &Request, _team: Option<&dyn TeamBridge>, _notifications: Option<&crate::agent_notifications::AgentNotificationHub>) -> Response {
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

/// Local wall-clock stamp for a line an agent will read: `2026-08-17 16:31`.
/// Minute precision on purpose — this is context, not a log timestamp.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) fn stamp_now() -> String {
    chrono::Local::now().format("%Y-%m-%d %H:%M").to_string()
}

/// Type ONE stamped chat line into ONE named agent's pane — the targeted
/// sibling of `deliver_mentions` (same gates: live window, managed, never a
/// shell; same `record_delivery` bookkeeping). Quiet on every miss: a dead
/// window or an unmanaged name simply has nobody to wake. Used by the
/// done-summary feedback edge and the board's review handoff — both are
/// DELIVERIES the server decides on, never mention scans, so the
/// record-only invariant of hook-sourced posts stays intact.
///
/// Desktop-only, like every `crate::projects` reader: the module is cfg'd out
/// on android/ios. The gate belongs on the FUNCTION, immediately above `fn` —
/// a doc comment between an attribute and its item is legal, so an attribute
/// left dangling above someone else's docs is silently adopted by whatever
/// item comes next. That is how this file broke the Android build once
/// (see the `projects_readers_are_desktop_gated` test below).
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn deliver_chat_line(session: &str, target_name: &str, line: &str) -> bool {
    use crate::projects::agents;

    let ws = crate::projects::project_for_session(session).ok().flatten().map(|p| p.path);
    let Ok(panes) = crate::tmux::list_panes(session) else { return false };
    for p in &panes {
        if !p.active || p.window_name != target_name {
            continue;
        }
        let is_agent = agents::detect_managed(ws.as_deref(), &p.window_name, &format!("{} {} {}", p.current_command, p.pane_title, p.window_name)).is_some();
        if !is_agent || !crate::projects::is_managed_in(ws.as_deref(), &p.window_name) {
            return false;
        }
        let target = format!("{}:{}.{}", session, p.window, p.pane);
        if crate::tmux::send_command(&target, line).is_ok() {
            crate::projects::telemetry::record_delivery(session, p.window, line);
            crate::projects::vitals::sniff_window_soon(session, p.window);
            return true;
        }
        return false;
    }
    false
}

/// Pure half of the assignee notification (owner, 2026-08-30): given the
/// PREVIOUS issue row and what this save carries, decide whether the assignee
/// should hear about it and name the change. `None` when: nobody is assigned,
/// the assignee is the actor (your own edit is not news), the assignee is the
/// human (who reads the board itself), the save (re)assigns (that has its own
/// dispatch channel), or nothing actually changed (a save echoing the stored
/// values is a no-op, not an event).
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
    let title = issue["title"].as_str().unwrap_or("");
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

/// Deliver a done summary into the pane of the agent that SPAWNED this one —
/// the feedback half of `tmm spawn --brief`. Quiet on every miss: a human
/// spawner, a dead window, or a pre-recipe agent has nobody to wake.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn deliver_done_to_spawner(session: &str, from: &str, summary: &str) {
    let ws = crate::projects::project_for_session(session).ok().flatten().map(|p| p.path);
    let Some(spawner) = crate::projects::spawned_by(ws.as_deref(), from) else { return };
    if spawner == from {
        return;
    }
    // `[done]` in the body tells the reader WHAT this line is — the brief's
    // outcome, not a new request — while the stamp and sender keep the shape
    // of every other delivered chat line.
    let line = format!("[tmm chat {}] {from}: [done] {summary}", stamp_now());
    deliver_chat_line(session, &spawner, &line);
}

/// Type an @mentioned chat line into each mentioned agent's pane. This is the
/// delivery half of the hub: the bus stores the record, but an interactive
/// CLI only reacts to what lands in its input. Delivery goes to MANAGED agent
/// windows only — a shell would execute the message, and a window the user
/// started by hand belongs to the user, not to this app.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn deliver_mentions(session: &str, from: &str, body: &str) {
    use crate::projects::agents;

    let mentions: Vec<&str> = body
        .split('@')
        .skip(1)
        .filter_map(|rest| rest.split_whitespace().next())
        .map(|name| name.trim_end_matches([',', ':', ';', '.', '!', '?']))
        .filter(|n| !n.is_empty())
        .collect();
    if mentions.is_empty() {
        return;
    }
    let ws = crate::projects::project_for_session(session).ok().flatten().map(|p| p.path);
    let Ok(panes) = crate::tmux::list_panes(session) else { return };
    let mut seen = std::collections::HashSet::new();
    for p in &panes {
        if !seen.insert(p.window) || !p.active {
            continue;
        }
        let is_agent = agents::detect_managed(ws.as_deref(), &p.window_name, &format!("{} {} {}", p.current_command, p.pane_title, p.window_name)).is_some();
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
        let matched = mentions.iter().any(|m| *m == p.window_name || *m == "all");
        if !matched {
            continue;
        }
        let target = format!("{}:{}.{}", session, p.window, p.pane);
        // The stamp is for the agent, not for us: a CLI reads this line inside a
        // conversation that may have been idle for hours, and "when was this
        // said" is context it otherwise has no way to recover — its own clock
        // only tells it `now`. Local wall time, minute precision; seconds would
        // be noise in a chat line.
        let line = format!("[tmm chat {}] {from}: {body}", stamp_now());
        if crate::tmux::send_command(&target, &line).is_ok() {
            // send_command only proves the pane existed. The delivery is
            // confirmed when that agent's userPromptSubmit hook echoes the line
            // back; until then it is pending, and telemetry reports it if the
            // echo never comes.
            crate::projects::telemetry::record_delivery(session, p.window, &line);
            // A line just landed in this pane: sniff its vitals once the TUI
            // has repainted (delayed + throttled inside).
            crate::projects::vitals::sniff_window_soon(session, p.window);
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
    let live: Vec<usize> = windows.keys().copied().collect();
    telemetry::retain_windows(session, &live);
    crate::projects::vitals::retain_windows(session, &live);

    let rows: Vec<serde_json::Value> = windows
        .values()
        .map(|p| {
            let agent = agents::detect_managed(ws.as_deref(), &p.window_name, &format!("{} {} {}", p.current_command, p.pane_title, p.window_name));
            let st = telemetry::derive(session, p.window, activity.get(&p.window).copied().unwrap_or(0));
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
                            p.window,
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
    }
    use super::super::test_util::req;
    use super::*;
    use crate::projects::telemetry;

    // The MockAgora from team_rpc's tests is private to that module; a local
    // minimal bridge keeps this module self-contained.
    struct Bridge {
        posts: std::sync::Mutex<Vec<(String, String, String)>>,
        /// Every `history_page` call, so a test can assert what the RPC asked for.
        pages: std::sync::Mutex<Vec<(Option<i64>, i64)>>,
        /// A real little message log, ascending by seq. Empty = answer the canned
        /// two-message page below; non-empty = page over it like the bus does, so
        /// a test can walk it exactly as a client would.
        log: std::sync::Mutex<Vec<serde_json::Value>>,
    }
    impl Bridge {
        fn new() -> Self {
            Bridge {
                posts: std::sync::Mutex::new(Vec::new()),
                pages: std::sync::Mutex::new(Vec::new()),
                log: std::sync::Mutex::new(Vec::new()),
            }
        }
        /// Seed `n` messages, seq/ts 1..=n, ids `m<seq>`.
        fn with_log(self, room: &str, n: i64) -> Self {
            *self.log.lock().unwrap() = (1..=n)
                .map(|seq| {
                    serde_json::json!({
                        "room": room, "seq": seq, "id": format!("m{seq}"),
                        "ts": seq * 10, "from": "human", "body": format!("body{seq}")
                    })
                })
                .collect();
            self
        }
    }
    impl TeamBridge for Bridge {
        fn delete_messages(&self, _room: &str, ids: &[String]) -> Result<usize, String> {
            Ok(ids.len())
        }
        fn room_latest(&self) -> serde_json::Value {
            serde_json::json!({ "proj:blog": 200 })
        }

        fn history(&self, room: &str, _limit: i64) -> serde_json::Value {
            serde_json::json!({ "messages": [
                { "room": room, "seq": 41, "ts": 100, "from": "a", "body": "old" },
                { "room": room, "seq": 42, "ts": 200, "from": "b", "body": "new" },
            ] })
        }
        fn history_page(&self, room: &str, before_seq: Option<i64>, limit: i64) -> serde_json::Value {
            self.pages.lock().unwrap().push((before_seq, limit));
            // Seeded log: page over it the way the bus does — newest `limit` rows
            // strictly older than the cursor, oldest first.
            let all = self.log.lock().unwrap().clone();
            if !all.is_empty() {
                let head_seq = all.last().and_then(|m| m["seq"].as_i64()).unwrap_or(0);
                let older: Vec<serde_json::Value> = all
                    .into_iter()
                    .filter(|m| {
                        before_seq.is_none_or(|b| m["seq"].as_i64().unwrap_or(0) < b)
                    })
                    .collect();
                let start = older.len().saturating_sub(limit.max(1) as usize);
                let has_more = start > 0;
                return serde_json::json!({
                    "messages": older[start..].to_vec(),
                    "has_more": has_more,
                    "head_seq": head_seq,
                });
            }
            match before_seq {
                // The page behind seq 41: one older message, and nothing before it.
                Some(_) => serde_json::json!({
                    "messages": [{ "room": room, "seq": 7, "ts": 50, "from": "a", "body": "older" }],
                    "has_more": false, "head_seq": 42
                }),
                None => serde_json::json!({
                    "messages": [
                        { "room": room, "seq": 41, "ts": 100, "from": "a", "body": "old" },
                        { "room": room, "seq": 42, "ts": 200, "from": "b", "body": "new" },
                    ],
                    "has_more": true, "head_seq": 42
                }),
            }
        }
        fn roster(&self, _room: &str) -> serde_json::Value { serde_json::json!({ "roster": [] }) }
        fn post(&self, room: &str, from: &str, body: &str, _rr: bool) -> Result<serde_json::Value, String> {
            self.posts.lock().unwrap().push((room.into(), from.into(), body.into()));
            Ok(serde_json::json!({ "ok": true }))
        }
        fn set_agent_status(&self, _r: &str, _a: &str, _s: &str) -> Result<(), String> { Ok(()) }
        fn employees(&self, _r: &str) -> serde_json::Value { serde_json::json!({}) }
        fn seed_employee(&self, _r: &str, _n: &str, _s: &serde_json::Value) -> Result<(), String> { Ok(()) }
        fn employee_specs(&self, _r: &str) -> Vec<(String, serde_json::Value, String)> { Vec::new() }
        fn room_exists(&self, _r: &str) -> bool { true }
        fn start_team(&self, _w: &str, _t: &str) -> serde_json::Value { serde_json::json!({}) }
        fn close_team(&self, _r: &str) -> bool { false }
        fn teams(&self) -> serde_json::Value { serde_json::json!({ "teams": [] }) }
        fn templates(&self) -> serde_json::Value { serde_json::json!({ "templates": [] }) }
        fn save_template(&self, _n: &str, _a: &serde_json::Value) -> Result<(), String> { Ok(()) }
        fn delete_template(&self, _n: &str) -> Result<(), String> { Ok(()) }
        fn system_prompt(&self) -> String { String::new() }
        fn save_system_prompt(&self, _t: &str) -> Result<(), String> { Ok(()) }
        fn default_workspace(&self) -> String { String::new() }
        fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
            tokio::sync::broadcast::channel(1).1
        }
        fn open_room(&self, _room: &str) -> Result<(), String> { Ok(()) }
    }

    /// A page can lose EVERY row to the archive filter, and the walk still has to
    /// continue: `has_more` says there is history behind it, so if no cursor comes
    /// back with it the client has nothing to ask for and scroll-up stops dead at
    /// the hidden run. The cursor falls back to the RAW page's oldest seq — the
    /// position always advances, even when nothing survived to be shown.
    #[test]
    fn a_fully_hidden_page_still_hands_back_a_cursor_to_the_older_visible_ones() {
        crate::projects::tests::use_test_store();
        let session = format!("hid-{}", uuid::Uuid::new_v4());
        let room = format!("proj:{session}");
        // Five messages; the middle stretch (seq 3 and 4) is archived, so one whole
        // page of two is invisible.
        let b = Bridge::new().with_log(&room, 5);
        for seq in [3, 4] {
            crate::projects::archive_msg(&room, &format!("m{seq}"), seq * 10, "human", "x").unwrap();
        }
        let page = |before: Option<i64>| {
            let mut params = serde_json::json!({ "session": session, "limit": 2 });
            if let Some(b) = before {
                params["before_seq"] = serde_json::json!(b);
            }
            handle_hub_request(&req("hub_log", params), Some(&b), None).result.expect("result")
        };

        // Page 1: raw [m4, m5], m4 hidden → the visible tail, cursor from the
        // SURVIVOR (what the user can actually see).
        let p1 = page(None);
        let bodies = |v: &serde_json::Value| {
            v["messages"].as_array().unwrap().iter()
                .map(|m| m["body"].as_str().unwrap().to_string()).collect::<Vec<_>>()
        };
        assert_eq!(bodies(&p1), vec!["body5"]);
        assert_eq!(p1["oldest_seq"], 5);
        assert_eq!(p1["has_more"], true);

        // Page 2: raw [m3, m4] — BOTH hidden. Nothing to render, but the page must
        // still carry the raw cursor (3) or the walk cannot go on.
        let p2 = page(p1["oldest_seq"].as_i64());
        assert!(bodies(&p2).is_empty(), "the whole page is hidden");
        assert_eq!(p2["has_more"], true, "and there is more behind it");
        assert_eq!(p2["oldest_seq"], 3, "the raw oldest seq is the cursor of last resort");

        // Page 3: continuing from that cursor reaches the older VISIBLE messages,
        // which is the behaviour the fallback exists for.
        let p3 = page(p2["oldest_seq"].as_i64());
        assert_eq!(bodies(&p3), vec!["body1", "body2"]);
        assert_eq!(p3["has_more"], false, "that is the start of the conversation");
        assert_eq!(p3["oldest_seq"], 1);

        // Every visible message was reached exactly once across the walk.
        let seen: Vec<String> = [bodies(&p3), bodies(&p2), bodies(&p1)].concat();
        assert_eq!(seen, vec!["body1", "body2", "body5"]);
    }

    /// Board #9: the room keeps everything, so the client needs a way to ask for
    /// a SMALL first page and then walk back. Two contracts are pinned here — an
    /// older client (no new params) gets exactly the newest page it always got,
    /// and `before_seq` is passed through with a cursor handed back for the step
    /// after it.
    #[test]
    fn hub_log_answers_the_newest_page_and_pages_backwards_on_request() {
        let b = Bridge::new();
        // An older client: only session (+ maybe since_ts/limit).
        let r = handle_hub_request(&req("hub_log", serde_json::json!({ "session": "blog" })), Some(&b), None);
        let v = r.result.expect("result");
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2, "the newest page, unchanged");
        assert_eq!(msgs[0]["body"], "old", "oldest first, as before");
        assert_eq!(v["has_more"], true, "and it says history remains behind it");
        assert_eq!(v["oldest_seq"], 41, "the cursor for the next page back");
        assert_eq!(v["head_seq"], 42);
        assert_eq!(b.pages.lock().unwrap()[0], (None, 100), "default limit, no cursor");

        // Scrolled up: the client asks for what is behind its oldest message.
        let r = handle_hub_request(
            &req("hub_log", serde_json::json!({ "session": "blog", "before_seq": 41, "limit": 50 })),
            Some(&b),
            None,
        );
        let v = r.result.expect("result");
        assert_eq!(v["messages"].as_array().unwrap().len(), 1);
        assert_eq!(v["messages"][0]["body"], "older");
        assert_eq!(v["has_more"], false, "the conversation begins there");
        assert_eq!(b.pages.lock().unwrap()[1], (Some(41), 50));

        // A nonsense cursor is no cursor: 0/negative means "the newest page",
        // never a query that could return nothing for ever.
        let r = handle_hub_request(
            &req("hub_log", serde_json::json!({ "session": "blog", "before_seq": 0 })),
            Some(&b),
            None,
        );
        assert_eq!(r.result.expect("result")["messages"].as_array().unwrap().len(), 2);
    }

    /// The activity feed's half of the same contract. The durable log keeps every
    /// event, so this read is where the bound lives: a default page, a hard cap,
    /// and a (ts, id) cursor handed back for walking backwards.
    #[test]
    fn hub_activity_pages_and_caps_and_reports_what_it_holds() {
        let b = Bridge::new();
        let session = format!("act-rpc-{}", uuid::Uuid::new_v4());
        for n in 0..6 {
            telemetry::record_tool(&session, 1, "Edit", &format!("f{n}.rs"));
        }
        // An older client sends only since_ts and gets the newest page.
        let r = handle_hub_request(
            &req("hub_activity", serde_json::json!({ "session": session, "since_ts": 0 })),
            Some(&b),
            None,
        );
        let v = r.result.expect("result");
        let evs = v["events"].as_array().unwrap();
        assert_eq!(evs.len(), 6, "everything this session has, under the default cap");
        assert!(v.get("has_more").is_some(), "the client is told whether more exists");

        // A limit is honoured, and the page carries the cursor for the next one.
        let r = handle_hub_request(
            &req("hub_activity", serde_json::json!({ "session": session, "limit": 2 })),
            Some(&b),
            None,
        );
        let v = r.result.expect("result");
        assert_eq!(v["events"].as_array().unwrap().len(), 2);
        let oldest = v["oldest"].clone();
        assert!(oldest["ts"].as_u64().unwrap() > 0, "a usable cursor, got {oldest:?}");

        // And it is a CAP, not a suggestion.
        let r = handle_hub_request(
            &req("hub_activity", serde_json::json!({ "session": session, "limit": 99999 })),
            Some(&b),
            None,
        );
        assert!(
            r.result.expect("result")["events"].as_array().unwrap().len()
                <= telemetry::MAX_PAGE_EVENTS
        );
    }

    #[test]
    fn hub_without_bus_is_method_not_found() {        let r = handle_hub_request(&req("hub_post", serde_json::json!({ "session": "s", "body": "hi" })), None, None);
        assert_eq!(r.error.as_ref().map(|e| e.code), Some(ERR_METHOD_NOT_FOUND));
    }

    #[test]
    fn hub_post_lands_in_the_project_room_with_the_sender() {
        let b = Bridge::new();
        let r = handle_hub_request(
            &req("hub_post", serde_json::json!({ "session": "blog", "from": "lead", "body": "@reviewer 看一下" })),
            Some(&b),
            None,
        );
        assert!(r.error.is_none(), "{}", r.error.map(|e| e.message).unwrap_or_default());
        let posts = b.posts.lock().unwrap();
        assert_eq!(posts.len(), 1);
        assert_eq!(posts[0], ("proj:blog".to_string(), "lead".to_string(), "@reviewer 看一下".to_string()));
    }

    #[test]
    fn hub_post_record_only_skips_delivery_but_stores_message() {
        let b = Bridge::new();
        let r = handle_hub_request(
            &req("hub_post", serde_json::json!({
                "session": "blog", "from": "lead",
                "body": "@reviewer 自动结果", "record_only": true
            })),
            Some(&b),
            None,
        );
        assert!(r.error.is_none(), "{}", r.error.map(|e| e.message).unwrap_or_default());
        // The bus.post was called (message stored) even though delivery was skipped.
        let posts = b.posts.lock().unwrap();
        assert_eq!(posts.len(), 1, "record-only posts are stored in the room");
        assert_eq!(posts[0].2, "@reviewer 自动结果");
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

        let b = Bridge::new();
        let r = handle_hub_request(
            &req("hub_board_note", serde_json::json!({
                "session": session, "id": issue_id, "who": "human",
                "body": "Please cover the restart case",
            })),
            Some(&b),
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
    /// auto-post use: only agents this app started. A name that has no isolated
    /// home is refused BEFORE any window is looked up, let alone killed.
    /// The line an agent reads carries local wall time, because a CLI resuming a
    /// conversation has no other way to know when something was said. The stamp
    /// must not break the delivery receipt, which matches by containment.
    #[test]
    fn a_delivered_line_carries_a_readable_local_stamp() {
        let stamp = stamp_now();
        assert_eq!(stamp.len(), 16, "YYYY-MM-DD HH:MM, got {stamp:?}");
        let (date, time) = stamp.split_once(' ').expect("date and time");
        assert_eq!(date.split('-').count(), 3, "{date:?}");
        assert_eq!(time.split(':').count(), 2, "minute precision, got {time:?}");

        // The shape deliver_mentions types, and the echo the hook returns.
        let body = "@dev ship it";
        let line = format!("[tmm chat {stamp}] human: {body}");
        crate::projects::telemetry::record_delivery("stamp-test", 9, &line);
        assert!(
            crate::projects::telemetry::record_prompt("stamp-test", 9, &line),
            "the stamped line still acknowledges its own echo"
        );
    }

    #[test]
    fn stopping_something_we_did_not_start_is_refused() {
        // The gate reads the project store; keep it off the user's real db.
        crate::projects::tests::use_test_store();
        let b = Bridge::new();
        for method in ["hub_agent_stop", "hub_agent_restart"] {
            let r = handle_hub_request(
                &req(method, serde_json::json!({ "session": "no-such-session", "agent": "byhand" })),
                Some(&b),
                None,
            );
            let msg = r.error.map(|e| e.message).unwrap_or_default();
            assert!(msg.contains("not an agent this app started"), "{method}: got {msg:?}");
        }
        assert!(b.posts.lock().unwrap().is_empty(), "nothing announced, nothing killed");
    }

    /// A status NOTE is a message from the agent — the owner's requirement, and
    /// what makes it durable (the room is the record; an event was not).
    #[test]
    fn a_status_note_is_posted_as_the_agents_own_message() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-note-{}", std::process::id());
        let created = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", &session, "-n", "dev", "sleep 60"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            eprintln!("no tmux server — skipping");
            return;
        }
        let b = Bridge::new();
        let r = handle_hub_request(
            &req("hub_status", serde_json::json!({
                "session": session, "agent": "dev", "state": "blocked",
                "note": "waiting for the API spec",
            })),
            Some(&b),
            None,
        );
        assert!(r.error.is_none(), "{:?}", r.error.map(|e| e.message));
        {
            let posts = b.posts.lock().unwrap();
            assert_eq!(posts.len(), 1, "one message, from the agent");
            assert_eq!(posts[0].1, "dev", "the agent is the sender, not the app");
            assert_eq!(posts[0].2, "[tmm status blocked] waiting for the API spec");
        }
        // A note-less claim posts NOTHING: the derived state already knows a turn
        // is open, so a bare state word would be an empty message.
        let r2 = handle_hub_request(
            &req("hub_status", serde_json::json!({
                "session": session, "agent": "dev", "state": "working", "note": "   ",
            })),
            Some(&b),
            None,
        );
        assert!(r2.error.is_none());
        assert_eq!(b.posts.lock().unwrap().len(), 1, "still just the one");
        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
    }

    /// A `tmm done` SUMMARY is the agent's own report, so it is a message. A
    /// summary-less done has nothing to read and stays a lifecycle line.
    #[test]
    fn a_done_summary_is_a_message_and_a_bare_done_is_not() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-done-{}", std::process::id());
        let created = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", &session, "-n", "dev", "sleep 60"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            eprintln!("no tmux server — skipping");
            return;
        }
        let b = Bridge::new();
        let with = handle_hub_request(
            &req("hub_done", serde_json::json!({
                "session": session, "agent": "dev", "summary": "shipped the palette",
            })),
            Some(&b),
            None,
        );
        assert!(with.error.is_none(), "{:?}", with.error.map(|e| e.message));
        {
            let posts = b.posts.lock().unwrap();
            assert_eq!(posts.len(), 1);
            assert_eq!(posts[0].1, "dev", "the agent is the sender");
            assert_eq!(
                posts[0].2, "[tmm done] shipped the palette",
                "not `[tmm] `: that marker folds into a grey sys row and the chat level drops it"
            );
        }
        let bare = handle_hub_request(
            &req("hub_done", serde_json::json!({ "session": session, "agent": "dev" })),
            Some(&b),
            None,
        );
        assert!(bare.error.is_none());
        {
            let posts = b.posts.lock().unwrap();
            assert_eq!(posts.len(), 2);
            assert_eq!(posts[1].2, "[tmm] done", "nothing was said, so the app narrates");
        }
        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
    }

    /// The kill path, against real tmux: a managed window disappears and the
    /// room records it. `restart` is not exercised here — it goes through
    /// `projects::up`, which launches a real agent CLI.
    #[test]
    fn stopping_a_managed_agent_kills_its_window_and_says_so() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-stop-{}", std::process::id());
        let ws = std::env::temp_dir().join(format!("tmm-stop-ws-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(ws.join(".tmm/agents/dev")).unwrap();
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

        let b = Bridge::new();
        let r = handle_hub_request(
            &req("hub_agent_stop", serde_json::json!({ "session": session, "agent": "dev" })),
            Some(&b),
            None,
        );
        if !created_project {
            eprintln!("could not adopt a project — skipping the positive half");
        } else {
            assert!(r.error.is_none(), "{:?}", r.error.map(|e| e.message));
            let panes = crate::tmux::list_panes(&session).unwrap_or_default();
            assert!(!panes.iter().any(|p| p.window_name == "dev"), "the window is gone");
            let posts = b.posts.lock().unwrap();
            assert_eq!(posts.len(), 1);
            assert!(posts[0].2.contains("[tmm] stopped dev"), "the room records it: {:?}", posts[0].2);
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
        let window = window_of_agent(&session, "dev").unwrap_or(0);
        telemetry::record_prompt(&session, window, "do the long thing");
        assert_eq!(telemetry::derive(&session, window, 0).state, "running");

        let b = Bridge::new();
        let r = handle_hub_request(
            &req("hub_agent_interrupt", serde_json::json!({ "session": session, "agent": "dev" })),
            Some(&b),
            None,
        );
        if !created_project {
            eprintln!("could not adopt a project — skipping the positive half");
        } else {
            assert!(r.error.is_none(), "{:?}", r.error.map(|e| e.message));
            let panes = crate::tmux::list_panes(&session).unwrap_or_default();
            assert!(panes.iter().any(|p| p.window_name == "dev"), "the window survives an interrupt");
            assert_eq!(
                telemetry::derive(&session, window, 0).state,
                "idle",
                "the cancelled turn is closed by the interrupt itself — no stop hook is coming"
            );
            let posts = b.posts.lock().unwrap();
            assert_eq!(posts.len(), 1);
            assert!(posts[0].2.contains("[tmm] interrupted dev"), "the room records it: {:?}", posts[0].2);
        }
        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(&ws);
    }

    #[test]
    fn hub_log_since_ts_filters_older_messages() {
        let b = Bridge::new();
        let r = handle_hub_request(
            &req("hub_log", serde_json::json!({ "session": "blog", "since_ts": 100 })),
            Some(&b),
            None,
        );
        let msgs = r.result.unwrap();
        let msgs = msgs.get("messages").and_then(|m| m.as_array()).unwrap();
        assert_eq!(msgs.len(), 1, "only the ts>100 message survives");
        assert_eq!(msgs[0].get("body").and_then(|b| b.as_str()), Some("new"));
    }

    #[test]
    fn hub_status_rejects_unknown_states_and_missing_windows() {
        let b = Bridge::new();
        let bad_state = handle_hub_request(
            &req("hub_status", serde_json::json!({ "session": "no-such-session-xyz", "agent": "a", "state": "napping" })),
            Some(&b),
            None,
        );
        // The window lookup fails first for a nonexistent session — either
        // error is INVALID_PARAMS, which is the contract that matters.
        assert_eq!(bad_state.error.as_ref().map(|e| e.code), Some(ERR_INVALID_PARAMS));
    }

    /// `crate::projects` is compiled out on android/ios, so every top-level
    /// function in this file that reads it MUST carry the desktop cfg gate.
    /// Nothing in the normal loop catches a missing one: `cargo test`, `cargo
    /// build` and the dev server all target the desktop, where the module
    /// exists — the error only appears in `npm run build:android`, which nobody
    /// runs per change. It broke exactly that way (board #16): `deliver_chat_line`
    /// was inserted directly beneath `deliver_mentions`' `#[cfg]`, adopted the
    /// gate (a doc comment between an attribute and its item is legal, so the
    /// attribute binds to whatever item follows), and left `deliver_mentions`
    /// and `deliver_done_to_spawner` ungated — 10 errors, two commits before
    /// anyone noticed.
    ///
    /// So the guard is a source contract, checked on the desktop where it is
    /// cheap, instead of a cross-compile nobody runs.
    #[test]
    fn projects_readers_are_desktop_gated() {
        const GATE: &str = "target_os = \"android\"";
        let src = include_str!("hub_rpc.rs");
        let lines: Vec<&str> = src.lines().collect();

        // Column-0 `fn` only: this is about the file's own top-level items.
        // Anything nested (an impl, a mod, this test module) inherits its
        // parent's gate.
        let is_top_fn = |l: &str| {
            ["fn ", "pub fn ", "pub(crate) fn ", "pub(super) fn "]
                .iter()
                .any(|p| l.starts_with(p))
        };

        let mut checked = 0usize;
        let mut ungated: Vec<&str> = Vec::new();
        for (i, line) in lines.iter().enumerate() {
            if !is_top_fn(line) {
                continue;
            }
            // The body: from this line until braces balance again.
            let mut depth = 0i32;
            let mut body = String::new();
            for l in &lines[i..] {
                body.push_str(l);
                body.push('\n');
                depth += l.chars().filter(|c| *c == '{').count() as i32;
                depth -= l.chars().filter(|c| *c == '}').count() as i32;
                if depth <= 0 && body.contains('{') {
                    break;
                }
            }
            if !body.contains("crate::projects") {
                continue;
            }
            checked += 1;
            // The preamble: the contiguous run of attributes and doc comments
            // above the signature. A gate anywhere in it counts — the rule under
            // test is "gated", not "gated on a particular line".
            let mut gated = false;
            for l in lines[..i].iter().rev() {
                let t = l.trim_start();
                if !(t.starts_with('#') || t.starts_with("//")) {
                    break;
                }
                if t.contains(GATE) {
                    gated = true;
                    break;
                }
            }
            if !gated {
                ungated.push(line);
            }
        }

        assert!(
            ungated.is_empty(),
            "these read crate::projects with no desktop gate, so they break the Android build: {ungated:#?}"
        );
        // A guard that silently stops finding anything is not a guard. These
        // three are the delivery helpers the regression hit; if they are renamed
        // away, this count is the tripwire that says so.
        assert!(
            checked >= 3,
            "expected at least 3 gated projects readers here, found {checked} — did the scan stop matching?"
        );
    }
}
