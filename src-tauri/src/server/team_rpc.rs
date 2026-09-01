//! team_* and agent-notification RPC dispatch, plus the push loops that
//! forward bus messages / notification events to the client as JSON pushes.
//! Split from server.rs 2026-07-22 — content unchanged.

use std::sync::Arc;

use crate::agent_notifications::AgentNotificationHub;

use super::rpc::{require_str, Request, Response, ERR_INTERNAL, ERR_INVALID_PARAMS, ERR_METHOD_NOT_FOUND};
use super::{Outbound, TeamBridge};

/// Dispatch `team_*` RPC methods to the in-process bus bridge. Returns a
/// method-not-found error when no bus is wired (mobile builds, or desktop with
/// team disabled) so the client can degrade gracefully (hide the Team tab).
pub(super) fn handle_team_request(req: &Request, team: Option<&dyn TeamBridge>) -> Response {
    let id = req.id;
    let p = &req.params;
    let Some(bus) = team else {
        return Response::err(id, ERR_METHOD_NOT_FOUND, "team bus not available on this server".into());
    };
    // Most methods operate on a specific team `room` (the phone's active team).
    let room = p.get("room").and_then(|v| v.as_str()).unwrap_or("");
    match req.method.as_str() {
        // Bus availability + the team list + a default workspace to pre-fill.
        // Team-agnostic, so the Team tab can render the switcher before picking
        // an active room.
        "team_status" => Response::ok(id, serde_json::json!({
            "available": true,
            "teams": bus.teams().get("teams").cloned().unwrap_or(serde_json::json!([])),
            "templates": bus.templates().get("templates").cloned().unwrap_or(serde_json::json!([])),
            "system_prompt": bus.system_prompt(),
            "default_workspace": bus.default_workspace(),
        })),
        "team_teams" => Response::ok(id, bus.teams()),
        // Global system prompt (prepended to every agent's brief).
        "team_system_prompt_save" => {
            let text = p.get("text").and_then(|v| v.as_str()).unwrap_or("");
            match bus.save_system_prompt(text) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }
        // Roster templates (named agent rosters the user can edit).
        "team_templates" => Response::ok(id, bus.templates()),
        "team_template_save" => {
            let name = match require_str(p, "name") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            // Full team definition: { env?, mcp?, skills?, prompt?, agents }.
            // (Legacy callers sending just `agents` still work — fall back to it.)
            let def = p.get("def").cloned()
                .unwrap_or_else(|| serde_json::json!({ "agents": p.get("agents").cloned().unwrap_or(serde_json::json!([])) }));
            match bus.save_template(name, &def) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true, "name": name })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }
        "team_template_delete" => {
            let name = match require_str(p, "name") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            match bus.delete_template(name) {
                Ok(()) => Response::ok(id, serde_json::json!({ "ok": true })),
                Err(e) => Response::err(id, ERR_INVALID_PARAMS, e),
            }
        }
        "team_history" => {
            let limit = p.get("limit").and_then(|v| v.as_i64()).unwrap_or(100).clamp(1, 1000);
            Response::ok(id, bus.history(room, limit))
        }
        "team_roster" => Response::ok(id, bus.roster(room)),
        "team_employees" => Response::ok(id, bus.employees(room)),
        // Operator action: spin up a team in `workspace` from `template`.
        // The stable workspace+template room keeps same-directory Teams
        // isolated. Idempotent; returns { room, started, workspace }.
        "team_start_team" => {
            let workspace = p.get("workspace").and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .unwrap_or_else(|| bus.default_workspace());
            let template = p.get("template").and_then(|v| v.as_str()).unwrap_or("default");
            Response::ok(id, bus.start_team(&workspace, template))
        }
        // Stop a team: kill its tmux session, forget it (chat log persists).
        "team_close_team" => {
            if room.is_empty() {
                return Response::err(id, ERR_INVALID_PARAMS, "missing required param: room".into());
            }
            Response::ok(id, serde_json::json!({ "closed": bus.close_team(room) }))
        }
        "team_post" => {
            if room.is_empty() {
                return Response::err(id, ERR_INVALID_PARAMS, "missing required param: room".into());
            }
            let body = match require_str(p, "body") {
                Ok(s) => s,
                Err(e) => return Response::err(id, ERR_INVALID_PARAMS, e),
            };
            // The human is "you" on the phone; default sender name "human"
            // matches team's dashboard/CLI convention so the operator shows
            // up consistently across surfaces.
            let from = p.get("from").and_then(|v| v.as_str()).filter(|s| !s.is_empty()).unwrap_or("human");
            // Mirror the dashboard: an @mention implies a reply is wanted
            // unless the client says otherwise.
            let requires_reply = p
                .get("requires_reply")
                .and_then(|v| v.as_bool())
                .unwrap_or_else(|| body.contains('@'));
            match bus.post(room, from, body, requires_reply) {
                Ok(msg) => Response::ok(id, serde_json::json!({ "message": msg })),
                Err(e) => Response::err(id, ERR_INTERNAL, e),
            }
        }
        other => Response::err(id, ERR_METHOD_NOT_FOUND, format!("unknown team method: {}", other)),
    }
}

/// The `agent_notifications_*` unread-inbox RPCs retired 2026-09-01 with the
/// old notification-dot UI (owner: "原来我用的感觉不是很好用") — the project
/// room's auto-post + read cursor and the derived status dots replaced it.
/// Only the hook management surface remains.
pub(super) fn handle_notification_request(req: &Request, hub: &AgentNotificationHub) -> Response {
    let id = req.id;
    match req.method.as_str() {
        "agent_hooks_status" => Response::ok(id, serde_json::to_value(hub.hook_status()).unwrap()),
        "agent_hooks_install" => match hub.install_hooks() {
            Ok(status) => Response::ok(id, serde_json::to_value(status).unwrap()),
            Err(error) => Response::err(id, ERR_INTERNAL, error),
        },
        "agent_hooks_remove" => match hub.remove_hooks() {
            Ok(status) => Response::ok(id, serde_json::to_value(status).unwrap()),
            Err(error) => Response::err(id, ERR_INTERNAL, error),
        },
        other => Response::err(id, ERR_METHOD_NOT_FOUND, format!("unknown agent notification method: {other}")),
    }
}

pub(super) async fn team_push_loop(out_tx: tokio::sync::mpsc::UnboundedSender<Outbound>, team: Arc<dyn TeamBridge>) {
    let mut rx = team.subscribe();
    loop {
        match rx.recv().await {
            Ok(msg_json) => {
                let frame = serde_json::json!({
                    "id": null,
                    "method": "team_message",
                    "params": { "message": serde_json::from_str::<serde_json::Value>(&msg_json).unwrap_or(serde_json::Value::Null) },
                });
                if out_tx.send(Outbound::Encrypted(serde_json::to_string(&frame).unwrap())).is_err() {
                    return; // send task gone
                }
            }
            // Lagged: the client re-syncs via team_history on demand, so just
            // keep receiving from the current tail.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
            Err(tokio::sync::broadcast::error::RecvError::Closed) => return,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::test_util::req;

    // ─── the retired unread-inbox RPCs stay retired (board #37) ─────────
    /// An old client still calls `agent_notifications_list`/`mark_read`; it
    /// must get a soft METHOD_NOT_FOUND — never a panic, never a resurrected
    /// snapshot — while the hook-management surface on the SAME dispatcher
    /// keeps answering.
    #[test]
    fn retired_notification_rpcs_degrade_soft_and_hooks_survive() {
        let root = std::env::temp_dir().join(format!("tmm-retired-rpc-{}", uuid::Uuid::new_v4()));
        let hub = AgentNotificationHub::load_at_for_tests(root.clone());
        for method in ["agent_notifications_list", "agent_notifications_mark_read"] {
            let resp = handle_notification_request(
                &req(method, serde_json::json!({ "session": "s", "window": 0 })),
                &hub,
            );
            let err = resp.error.expect("retired method answers with an error");
            assert_eq!(err.code, super::super::rpc::ERR_METHOD_NOT_FOUND, "{method}");
            assert!(resp.result.is_none(), "{method} must not return a snapshot");
        }
        let resp = handle_notification_request(&req("agent_hooks_status", serde_json::json!({})), &hub);
        assert!(resp.error.is_none() && resp.result.is_some(), "hook status still answers");
        let _ = std::fs::remove_dir_all(root);
    }

    // ─── team WS proxy dispatch ─────────────────────────────────────────
    // A tiny in-memory TeamBridge stand-in so we can exercise
    // handle_team_request without pulling in the real (desktop-only) bus.
    struct MockAgora;
    impl TeamBridge for MockAgora {
        fn delete_messages(&self, _room: &str, ids: &[String]) -> Result<usize, String> {
            Ok(ids.len())
        }
        fn room_latest(&self) -> serde_json::Value {
            serde_json::Value::Object(Default::default())
        }

        fn history(&self, _room: &str, limit: i64) -> serde_json::Value {
            serde_json::json!({ "messages": [], "echo_limit": limit })
        }
        fn roster(&self, _room: &str) -> serde_json::Value {
            serde_json::json!({ "roster": [{ "name": "worker", "status": "idle" }] })
        }
        fn post(&self, room: &str, from: &str, body: &str, requires_reply: bool) -> Result<serde_json::Value, String> {
            Ok(serde_json::json!({ "room": room, "from": from, "body": body, "requires_reply": requires_reply }))
        }
        fn set_agent_status(&self, _room: &str, _agent: &str, _status: &str) -> Result<(), String> {
            Ok(())
        }
        fn employees(&self, _room: &str) -> serde_json::Value {
            serde_json::json!({ "employees": [] })
        }
        fn seed_employee(&self, _room: &str, _name: &str, _spec: &serde_json::Value) -> Result<(), String> {
            Ok(())
        }
        fn employee_specs(&self, _room: &str) -> Vec<(String, serde_json::Value, String)> {
            Vec::new()
        }
        fn room_exists(&self, _room: &str) -> bool { true }
        fn start_team(&self, workspace: &str, template: &str) -> serde_json::Value {
            serde_json::json!({ "started": true, "room": "ws", "workspace": workspace, "template": template })
        }
        fn close_team(&self, _room: &str) -> bool {
            true
        }
        fn teams(&self) -> serde_json::Value {
            serde_json::json!({ "teams": [] })
        }
        fn templates(&self) -> serde_json::Value {
            serde_json::json!({ "templates": [{ "name": "default", "agents": [] }] })
        }
        fn save_template(&self, _name: &str, _agents: &serde_json::Value) -> Result<(), String> {
            Ok(())
        }
        fn delete_template(&self, _name: &str) -> Result<(), String> {
            Ok(())
        }
        fn system_prompt(&self) -> String {
            String::new()
        }
        fn save_system_prompt(&self, _text: &str) -> Result<(), String> {
            Ok(())
        }
        fn default_workspace(&self) -> String {
            "/tmp/ws".to_string()
        }
        fn subscribe(&self) -> tokio::sync::broadcast::Receiver<String> {
            let (tx, rx) = tokio::sync::broadcast::channel(4);
            let _ = tx; // keep the sender alive only as long as needed for the type
            rx
        }
        fn open_room(&self, _room: &str) -> Result<(), String> {
            Ok(())
        }
    }

    #[test]
    fn team_request_without_bus_is_method_not_found() {
        let r = handle_team_request(&req("team_roster", serde_json::json!({})), None);
        assert_eq!(r.error.as_ref().map(|e| e.code), Some(ERR_METHOD_NOT_FOUND));
    }

    #[test]
    fn team_roster_returns_roster() {
        let bus = MockAgora;
        let r = handle_team_request(&req("team_roster", serde_json::json!({ "room": "ws" })), Some(&bus));
        let roster = r.result.unwrap();
        assert_eq!(roster["roster"][0]["name"], "worker");
    }

    #[test]
    fn team_post_requires_room_and_body_and_infers_reply_from_mention() {
        let bus = MockAgora;
        // Missing room → invalid params.
        let no_room = handle_team_request(&req("team_post", serde_json::json!({ "body": "hi" })), Some(&bus));
        assert_eq!(no_room.error.as_ref().map(|e| e.code), Some(ERR_INVALID_PARAMS));

        // Missing body → invalid params.
        let bad = handle_team_request(&req("team_post", serde_json::json!({ "room": "ws" })), Some(&bus));
        assert_eq!(bad.error.as_ref().map(|e| e.code), Some(ERR_INVALID_PARAMS));

        // @mention with no explicit flag → requires_reply inferred true.
        let mentioned = handle_team_request(
            &req("team_post", serde_json::json!({ "room": "ws", "body": "@worker do X" })),
            Some(&bus),
        );
        assert_eq!(mentioned.result.unwrap()["message"]["requires_reply"], true);

        // Plain broadcast → requires_reply inferred false; default sender "human".
        let plain = handle_team_request(
            &req("team_post", serde_json::json!({ "room": "ws", "body": "hello team" })),
            Some(&bus),
        );
        let msg = plain.result.unwrap();
        assert_eq!(msg["message"]["requires_reply"], false);
        assert_eq!(msg["message"]["from"], "human");

        // Explicit requires_reply=false overrides the @mention inference.
        let forced = handle_team_request(
            &req("team_post", serde_json::json!({ "room": "ws", "body": "@worker fyi", "requires_reply": false })),
            Some(&bus),
        );
        assert_eq!(forced.result.unwrap()["message"]["requires_reply"], false);
    }

    #[test]
    fn team_status_lists_teams_without_a_room() {
        let bus = MockAgora;
        let r = handle_team_request(&req("team_status", serde_json::json!({})), Some(&bus));
        let s = r.result.unwrap();
        assert_eq!(s["available"], true);
        assert!(s["teams"].is_array());
    }

    #[test]
    fn team_unknown_method_is_method_not_found() {
        let bus = MockAgora;
        let r = handle_team_request(&req("team_bogus", serde_json::json!({})), Some(&bus));
        assert_eq!(r.error.as_ref().map(|e| e.code), Some(ERR_METHOD_NOT_FOUND));
    }

    // ─── encode/decode_wire_payload roundtrip ────────────────────────────
}
