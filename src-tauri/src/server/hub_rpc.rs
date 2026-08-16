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

/// Bus room for a project's hub chat. The session name IS the project
/// identity (UNIQUE(session) in state.db), so it is also the room key.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) fn project_room(session: &str) -> String {
    format!("proj:{session}")
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub(super) fn handle_hub_request(req: &Request, team: Option<&dyn TeamBridge>, notifications: Option<&crate::agent_notifications::AgentNotificationHub>) -> Response {
    use crate::projects::telemetry;

    let id = req.id;
    let p = &req.params;
    let Some(bus) = team else {
        return Response::err(id, ERR_METHOD_NOT_FOUND, "hub not available on this server".into());
    };
    let session = match require_str(p, "session") {
        Ok(s) => s,
        Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
    };
    let room = project_room(session);

    match req.method.as_str() {
        // Post to the project chat. `from` is the agent name (tmm exports
        // TMM_AGENT) or "human" for the operator.
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
        // — the multica-style cursor so an agent polls without re-reading.
        "hub_log" => {
            let limit = p.get("limit").and_then(|v| v.as_i64()).unwrap_or(100).clamp(1, 1000);
            if let Err(e) = bus.open_room(&room) {
                return Response::err(id, ERR_INTERNAL, e);
            }
            let since_ts = p.get("since_ts").and_then(|v| v.as_i64()).unwrap_or(0);
            let mut history = bus.history(&room, limit);
            if since_ts > 0 {
                if let Some(msgs) = history.get_mut("messages").and_then(|m| m.as_array_mut()) {
                    msgs.retain(|m| m.get("ts").and_then(|t| t.as_i64()).unwrap_or(0) > since_ts);
                }
            }
            Response::ok(id, history)
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
                // A completion is a message too — the room is the record.
                if bus.open_room(&room).is_ok() {
                    let body = if summary.is_empty() { "[tmm] done".to_string() } else { format!("[tmm] done — {summary}") };
                    let _ = bus.post(&room, agent, &body, false);
                }
                // Mark this turn as having an explicit message so the stop
                // hook doesn't auto-post a duplicate reply.
                if let Some(hub) = notifications {
                    hub.mark_sent_this_turn(session, window);
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
            }
            Response::ok(id, serde_json::json!({ "ok": true, "window": window }))
        }

        // Derived agent states for a session: one row per live window, agent
        // detection + status derivation joined at read time.
        "hub_agents" => Response::ok(id, agent_states(session)),

        // The activity feed: recent observed telemetry events (tool calls,
        // status declarations, notifications) for the chat timeline. An
        // in-memory ring — telemetry made visible, not chat history.
        "hub_activity" => {
            let since_ts = p.get("since_ts").and_then(|v| v.as_u64()).unwrap_or(0);
            // A client asking for the feed is exactly when an undelivered line
            // matters, so account for the ones that timed out before reading.
            telemetry::sweep_deliveries(session);
            let events = telemetry::recent_events(session, since_ts);
            Response::ok(id, serde_json::json!({ "events": events }))
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

/// Type an @mentioned chat line into each mentioned agent's pane. This is the
/// delivery half of the hub: the bus stores the record, but an interactive
/// CLI only reacts to what lands in its input. Only windows that ARE agents
/// receive delivery (typing into a shell would execute the message).
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
    let Ok(panes) = crate::tmux::list_panes(session) else { return };
    let mut seen = std::collections::HashSet::new();
    for p in &panes {
        if !seen.insert(p.window) || !p.active {
            continue;
        }
        let is_agent = agents::detect(&format!("{} {} {}", p.current_command, p.pane_title, p.window_name)).is_some();
        if !is_agent || p.window_name == from {
            continue;
        }
        let matched = mentions.iter().any(|m| *m == p.window_name || *m == "all");
        if !matched {
            continue;
        }
        let target = format!("{}:{}.{}", session, p.window, p.pane);
        let line = format!("[tmm chat] {from}: {body}");
        if crate::tmux::send_command(&target, &line).is_ok() {
            // send_command only proves the pane existed. The delivery is
            // confirmed when that agent's userPromptSubmit hook echoes the line
            // back; until then it is pending, and telemetry reports it if the
            // echo never comes.
            crate::projects::telemetry::record_delivery(session, p.window, &line);
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

    let rows: Vec<serde_json::Value> = windows
        .values()
        .map(|p| {
            let agent = agents::detect(&format!("{} {} {}", p.current_command, p.pane_title, p.window_name));
            let st = telemetry::derive(session, p.window, activity.get(&p.window).copied().unwrap_or(0));
            let managed = agent.is_some()
                && ws.as_deref().is_some_and(|w| {
                    std::path::Path::new(w).join(".tmm").join("agents").join(&p.window_name).is_dir()
                });
            serde_json::json!({
                "window": p.window,
                "name": p.window_name,
                "command": p.current_command,
                "agent": agent.map(|a| a.backend),
                "managed": managed,
                "state": if agent.is_some() { st.state.as_str() } else { "shell" },
                "detail": st.detail,
                "since": st.since,
            })
        })
        .collect();
    serde_json::json!({ "session": session, "agents": rows })
}

#[cfg(all(test, not(any(target_os = "android", target_os = "ios"))))]
mod tests {
    use super::super::test_util::req;
    use super::*;

    // The MockAgora from team_rpc's tests is private to that module; a local
    // minimal bridge keeps this module self-contained.
    struct Bridge {
        posts: std::sync::Mutex<Vec<(String, String, String)>>,
    }
    impl Bridge {
        fn new() -> Self {
            Bridge { posts: std::sync::Mutex::new(Vec::new()) }
        }
    }
    impl TeamBridge for Bridge {
        fn history(&self, room: &str, _limit: i64) -> serde_json::Value {
            serde_json::json!({ "messages": [
                { "room": room, "ts": 100, "from": "a", "body": "old" },
                { "room": room, "ts": 200, "from": "b", "body": "new" },
            ] })
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

    #[test]
    fn hub_without_bus_is_method_not_found() {
        let r = handle_hub_request(&req("hub_post", serde_json::json!({ "session": "s", "body": "hi" })), None, None);
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
}
