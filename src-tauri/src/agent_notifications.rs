use crate::{config, tmux};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::broadcast;

const MAX_INBOX_BYTES: u64 = 256 * 1024;
const MAX_SUMMARY_CHARS: usize = 240;
/// Chat-path budget for hook-sourced auto-replies. Separate from the 240-char
/// notification summary: a final reply carries full content.
const MAX_REPLY_CHARS: usize = 6 * 1024;
/// How much of a tool's argument is kept. It was 80 characters, which is shorter
/// than the paths this app's own agents work with: every row in the lane ended in
/// `…` after a third of a line, with the rest of a wide screen blank next to it —
/// and the argument is the half of a tool call worth reading (owner, 2026-08-20:
/// "工具调用的参数没有显示全，后边被压缩成 ... 了，屏幕的宽度没有有效利用"). 2 KB
/// covers every real path and command; beyond that it is a script, and its first
/// two thousand characters identify it. The lane pans, so length costs no layout,
/// and the durable log is capped by ROW COUNT, so it costs no unbounded storage.
const MAX_TOOL_DETAIL_CHARS: usize = 2048;
const DEDUPE_SECS: u64 = 3;
const OWNER_MARKER: &str = "tmux-mobile-agent-notify";

// ── Room poster ──────────────────────────────────────────────────────────────

/// Minimal posting interface injected into the hub so hook-sourced replies can
/// land in the project room without naming the agora bus or the TeamBridge.
///
/// **INVARIANT**: implementations MUST set `record_only = true` on every call
/// that originates from a hook. Hook-sourced text must never trigger delivery
/// (typed into agent panes), or addressed replies create ping-pong loops.
/// The flag is enforced at the hub_post call site, not here.
pub trait RoomPoster: Send + Sync {
    /// Post `body` into the project room for `session` on behalf of `agent`.
    /// `record_only`: when true, the message is stored but NOT delivered
    /// (typed) into any agent's pane, regardless of @-mentions in the body.
    fn post_to_room(&self, session: &str, agent: &str, body: &str, record_only: bool);
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct AgentNotification {
    pub id: String,
    pub agent: String,
    pub kind: String,
    pub pane_id: String,
    pub session: String,
    pub window: usize,
    pub pane: usize,
    pub target: String,
    pub summary: String,
    pub timestamp: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_session_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct InboxEnvelope {
    backend: String,
    pane_id: String,
    payload: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookBackendStatus {
    pub supported: bool,
    pub installed: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct HookStatus {
    pub helper: bool,
    pub claude: HookBackendStatus,
    pub codex: HookBackendStatus,
    pub kiro: HookBackendStatus,
}

#[derive(Default)]
struct State {
    unread: HashMap<String, AgentNotification>,
    /// window key → the agent's own conversation id, from the last hook that
    /// carried one. In memory only: the durable copy is the project slot that
    /// the capturer stamps with it (`src-tauri/src/projects`), because that is
    /// what has to survive the reboot which loses tmux in the first place.
    sessions: HashMap<String, String>,
    /// window key → true when the agent issued a `tmm send` or `tmm done`
    /// during the current turn. Reset at `userPromptSubmit` (turn start).
    /// Used to suppress the automatic stop-hook post so a turn that already
    /// reported itself does not produce a second identical message.
    sent_this_turn: HashMap<String, bool>,
    /// Injected by the server after the team bus is ready. `None` on mobile.
    /// Box'd pointer stored here so it shares the Mutex with the rest of state.
    poster: Option<Arc<dyn RoomPoster>>,
}

#[derive(Clone)]
pub struct AgentNotificationHub {
    root: PathBuf,
    state: Arc<Mutex<State>>,
    tx: broadcast::Sender<String>,
}

impl AgentNotificationHub {
    pub fn load() -> Self {
        Self::load_at(config::config_dir().join("agent-notifications"))
    }

    fn load_at(root: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(root.join("inbox"));
        let unread = std::fs::read(root.join("unread.json"))
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Vec<AgentNotification>>(&bytes).ok())
            .unwrap_or_default()
            .into_iter()
            .map(|item| (window_key(&item.session, item.window), item))
            .collect();
        let (tx, _) = broadcast::channel(64);
        Self {
            root,
            state: Arc::new(Mutex::new(State {
                unread,
                sessions: HashMap::new(),
                sent_this_turn: HashMap::new(),
                poster: None,
            })),
            tx,
        }
    }

    /// Inject the room poster. Called once by the server after the team bus is
    /// ready. Desktop-only; mobile leaves this as `None`.
    pub fn set_room_poster(&self, poster: Arc<dyn RoomPoster>) {
        self.state.lock().unwrap().poster = Some(poster);
    }

    /// Called by `tmm send` / `tmm done` to record that this window's current
    /// turn already produced an explicit message. The stop hook will skip the
    /// automatic post for this turn.
    pub fn mark_sent_this_turn(&self, session: &str, window: usize) {
        self.state.lock().unwrap().sent_this_turn.insert(window_key(session, window), true);
    }

    /// Called by the `userPromptSubmit` hook to mark the start of a new turn.
    /// Clears the "sent this turn" flag so the upcoming stop can auto-post.
    pub fn reset_sent_this_turn(&self, session: &str, window: usize) {
        self.state.lock().unwrap().sent_this_turn.remove(&window_key(session, window));
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    pub fn snapshot(&self) -> Value {
        let state = self.state.lock().unwrap();
        snapshot_value(&state)
    }

    /// The agent conversation id last reported by a hook in this tmux window,
    /// if any. Used by the project capturer to stamp the slot, so `up` can
    /// resume that conversation rather than open a fresh one.
    pub fn agent_session_for(&self, session: &str, window: usize) -> Option<String> {
        self.state
            .lock()
            .ok()?
            .sessions
            .get(&window_key(session, window))
            .cloned()
    }

    pub fn mark_read(&self, session: &str, window: usize) -> Result<Value, String> {        let changed = {
            let mut state = self.state.lock().unwrap();
            state.unread.remove(&window_key(session, window)).is_some()
        };
        if changed {
            self.persist()?;
            self.broadcast_snapshot();
        }
        Ok(self.snapshot())
    }

    pub async fn run(self: Arc<Self>) {
        loop {
            self.consume_inbox();
            tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        }
    }

    fn consume_inbox(&self) {
        for path in inbox_files(&self.root.join("inbox")) {
            if let Err(error) = self.consume_file(&path) {
                eprintln!(
                    "⚠️  agent notification ignored ({}): {}",
                    path.display(),
                    error
                );
            }
            let _ = std::fs::remove_file(path);
        }
    }

    fn consume_file(&self, path: &Path) -> Result<(), String> {
        let metadata = std::fs::metadata(path).map_err(|e| e.to_string())?;
        if metadata.len() > MAX_INBOX_BYTES {
            return Err("payload too large".into());
        }
        let envelope: InboxEnvelope =
            serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
                .map_err(|e| format!("invalid envelope: {e}"))?;
        // Tool events (pre/postToolUse from isolated-home agents, Phase B+)
        // are TELEMETRY, not notifications: record the live activity line and
        // stop — no unread dot, no dedupe, no persistence.
        if let Some((tool, detail)) = tool_event_parts(&envelope) {
            #[cfg(not(any(target_os = "android", target_os = "ios")))]
            {
                let (session, window, _) = tmux::resolve_pane_id(&envelope.pane_id)?;
                crate::projects::telemetry::record_tool(&session, window, &tool, &detail);
            }
            return Ok(());
        }
        // userPromptSubmit marks the start of a new turn: clear the
        // "sent this turn" flag so the upcoming stop can auto-post, and record
        // the prompt itself — the input half of the transcript, and the receipt
        // for anything `deliver_mentions` typed into this pane.
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if is_user_prompt_submit(&envelope) {
            let (session, window, _) = tmux::resolve_pane_id(&envelope.pane_id)?;
            self.reset_sent_this_turn(&session, window);
            if let Some(prompt) = envelope.payload.get("prompt").and_then(Value::as_str) {
                if !prompt.trim().is_empty() {
                    crate::projects::telemetry::record_prompt(&session, window, prompt);
                }
            }
            return Ok(());
        }
        let normalized = normalize(&envelope)?;
        let (session, window, pane) = tmux::resolve_pane_id(&envelope.pane_id)?;
        let timestamp = unix_seconds();
        // Feed the telemetry channel BEFORE dedupe: dedupe is a notification-UI
        // concern; status derivation wants every observed fact.
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        crate::projects::telemetry::record_notification(&session, window, &normalized.kind, timestamp);

        // Stop hook auto-post: post the agent's final reply to the project
        // room when all conditions are met:
        //   1. Only managed windows (constraint 3): a .tmm/agents/<name> dir
        //      must exist, so direct or adopted agents never auto-post.
        //   2. Skip if the agent already sent an explicit tmm send/done this
        //      turn (constraint 1 — same-turn dedup).
        //   3. There must be a reply body worth posting.
        //   4. The post is record-only (constraint 2): never typed into panes.
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if normalized.kind == "completed" {
            self.maybe_auto_post(&session, window, &normalized);
        }

        let item = AgentNotification {
            id: format!("{}-{}-{}", timestamp, std::process::id(), pane),
            agent: normalized.agent,
            kind: normalized.kind,
            pane_id: envelope.pane_id,
            target: format!("{}:{}.{}", session, window, pane),
            session,
            window,
            pane,
            summary: normalized.summary,
            timestamp,
            agent_session_id: normalized.agent_session_id,
        };
        self.record(item)
    }

    /// Post the agent's final reply to the project room when the window is
    /// managed, the agent hasn't already sent this turn, and there is a body.
    ///
    /// **INVARIANT**: always called with `record_only = true`. This function
    /// must never pass `false`; delivery of hook-sourced text into agent panes
    /// creates ping-pong reply loops.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    fn maybe_auto_post(&self, session: &str, window: usize, normalized: &Normalized) {
        // Nothing to post — skip.
        let reply = normalized.full_reply.as_deref().unwrap_or_default();
        if reply.is_empty() {
            return;
        }
        // Constraint 3: managed-only gate. `projects::managed_home` is the ONE
        // definition of "an agent this app created" — shared with hub_agents'
        // participant list and with delivery, so the three cannot drift apart.
        let window_name = match tmux::list_panes(session).ok().and_then(|panes| {
            panes.into_iter().find(|p| p.window == window).map(|p| p.window_name)
        }) {
            Some(n) => n,
            None => return, // session or window vanished between hook and poll
        };
        if crate::projects::managed_home(session, &window_name).is_none() {
            return;
        }
        // Constraint 1: same-turn dedup.
        if self.already_sent_this_turn(session, window) {
            return;
        }
        // Constraint 4: truncate at the chat-path budget.
        let body = truncate(reply, MAX_REPLY_CHARS);
        // Constraint 2: record_only = true. The poster implementation enforces
        // this at hub_post: no @-mention delivery, no pane typing.
        let poster = self.state.lock().unwrap().poster.clone();
        if let Some(p) = poster {
            p.post_to_room(session, &window_name, &body, true);
        }
    }

    /// Constraint 1 (same-turn dedup): skip the automatic post when the agent
    /// already spoke this turn via `tmm send` / `tmm done`.
    fn already_sent_this_turn(&self, session: &str, window: usize) -> bool {
        self.state
            .lock()
            .unwrap()
            .sent_this_turn
            .get(&window_key(session, window))
            .copied()
            .unwrap_or(false)
    }

    fn record(&self, item: AgentNotification) -> Result<(), String> {
        let changed = {
            let mut state = self.state.lock().unwrap();
            let key = window_key(&item.session, item.window);
            // Remember the conversation id even for a duplicate event: this map
            // is how a restored window resumes the exact conversation instead
            // of starting a blank one.
            if let Some(id) = item.agent_session_id.clone() {
                state.sessions.insert(key.clone(), id);
            }
            let duplicate = state.unread.get(&key).is_some_and(|old| {
                let same_turn = old.agent == item.agent
                    && old.pane_id == item.pane_id
                    && old.agent_session_id == item.agent_session_id
                    && item.timestamp.saturating_sub(old.timestamp) <= DEDUPE_SECS;
                same_turn
                    && (old.kind == item.kind || (is_urgent(&old.kind) && item.kind == "completed"))
            });
            if duplicate {
                false
            } else {
                state.unread.insert(key, item);
                true
            }
        };
        if changed {
            self.persist()?;
            self.broadcast_snapshot();
        }
        Ok(())
    }

    fn persist(&self) -> Result<(), String> {
        std::fs::create_dir_all(&self.root).map_err(|e| e.to_string())?;
        let data = {
            let state = self.state.lock().unwrap();
            let mut items: Vec<_> = state.unread.values().cloned().collect();
            items.sort_by_key(|item| item.timestamp);
            serde_json::to_vec_pretty(&items).map_err(|e| e.to_string())?
        };
        atomic_write(&self.root.join("unread.json"), &data)
    }

    fn broadcast_snapshot(&self) {
        let _ = self.tx.send(self.snapshot().to_string());
    }

    pub fn hook_status(&self) -> HookStatus {
        HookStatus {
            helper: self.helper_path().is_file(),
            claude: HookBackendStatus {
                supported: true,
                installed: json_file_contains(&claude_path(), OWNER_MARKER),
            },
            codex: HookBackendStatus {
                supported: true,
                installed: json_file_contains(&codex_path(), OWNER_MARKER),
            },
            kiro: HookBackendStatus {
                supported: true,
                installed: json_file_contains(&kiro_path(), OWNER_MARKER)
                    || json_file_contains(&kiro_default_path(), OWNER_MARKER),
            },
        }
    }

    pub fn install_hooks(&self) -> Result<HookStatus, String> {
        self.write_helper()?;
        let helper = format!(
            "/bin/sh {}",
            shell_quote(&self.helper_path().to_string_lossy())
        );
        install_claude(&claude_path(), &helper)?;
        install_codex(&codex_path(), &helper)?;
        install_kiro(&kiro_path(), &helper)?;
        install_kiro_default(&kiro_default_path(), &helper)?;
        Ok(self.hook_status())
    }

    pub fn remove_hooks(&self) -> Result<HookStatus, String> {
        remove_owned_hooks(&claude_path())?;
        remove_owned_hooks(&codex_path())?;
        if kiro_path().is_file() && json_file_contains(&kiro_path(), OWNER_MARKER) {
            std::fs::remove_file(kiro_path()).map_err(|e| e.to_string())?;
        }
        remove_kiro_default_hook(&kiro_default_path())?;
        Ok(self.hook_status())
    }

    pub fn helper_command(&self, backend: &str) -> String {
        format!(
            "/bin/sh {} {} # {}",
            shell_quote(&self.helper_path().to_string_lossy()),
            backend,
            OWNER_MARKER
        )
    }

    pub fn ensure_helper(&self) -> Result<(), String> {
        self.write_helper()
    }

    fn helper_path(&self) -> PathBuf {
        self.root.join(OWNER_MARKER)
    }

    fn write_helper(&self) -> Result<(), String> {
        std::fs::create_dir_all(self.root.join("inbox")).map_err(|e| e.to_string())?;
        let inbox = shell_quote(&self.root.join("inbox").to_string_lossy());
        let script = format!(
            r#"#!/bin/sh
umask 077
exec 2>/dev/null
backend="${{1:-}}"
case "$backend" in claude|codex|kiro) ;; *) exit 0 ;; esac
pane="${{TMUX_PANE:-}}"
case "$pane" in %*[!0-9]*|%|"") exit 0 ;; esac
inbox={inbox}
mkdir -p "$inbox" || exit 0
tmp=$(mktemp "$inbox/.tmp.XXXXXX") || exit 0
trap 'rm -f "$tmp"' EXIT
payload=
IFS= read -r payload || true
[ -n "$payload" ] || exit 0
printf '{{"backend":"%s","pane_id":"%s","payload":%s}}\n' "$backend" "$pane" "$payload" > "$tmp" || exit 0
mv "$tmp" "$inbox/$(date +%s)-$$.json" || exit 0
trap - EXIT
exit 0
"#
        );
        std::fs::write(self.helper_path(), script).map_err(|e| e.to_string())?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(self.helper_path(), std::fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }
}

/// Bridge to the project capturer, which asks by (session, window) because that
/// is the granularity of a project slot. Implemented here so `projects` never
/// names the notification types — it only knows its own trait.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
impl crate::projects::capture::AgentSessions for AgentNotificationHub {
    fn agent_session_for(&self, session: &str, window: usize) -> Option<String> {
        AgentNotificationHub::agent_session_for(self, session, window)
    }
}

/// Inbox files in the order the hooks WROTE them.
///
/// `read_dir` yields filesystem order, which is arbitrary. That mattered: every
/// event is timestamped when it is CONSUMED, so consuming a turn's `stop` before
/// its tool calls stamped the agent's reply earlier than the work that produced
/// it, and the chat rendered the tool calls after the answer (owner report,
/// 2026-08-16). The helper names files `<epoch_secs>-<pid>.json`, so the leading
/// number is the ordering key, with the whole name as the tie-break.
fn inbox_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    files.sort_by_key(|p| {
        let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
        let secs = name.split('-').next().and_then(|s| s.parse::<u64>().ok()).unwrap_or(0);
        (secs, name)
    });
    files
}

/// PreToolUse/PostToolUse across backends → `(tool name, detail)`
/// (`("Edit", "src/lib.rs")`), or None when this is a lifecycle notification.
/// The two parts stay SEPARATE all the way to the client: the tool name is the
/// scannable part of a step row, so it is rendered differently from its
/// argument, and joining them here would force the client to re-split a string
/// on a space that a Windows path or a shell command can contain.
fn tool_event_parts(envelope: &InboxEnvelope) -> Option<(String, String)> {
    let payload = envelope.payload.as_object()?;
    let event = payload.get("hook_event_name").and_then(Value::as_str)?;
    if !matches!(event, "PreToolUse" | "PostToolUse" | "preToolUse" | "postToolUse") {
        return None;
    }
    let tool = payload
        .get("tool_name")
        .or_else(|| payload.get("tool"))
        .and_then(Value::as_str)
        .unwrap_or("tool");
    // Best-effort one-arg detail: the file for edits, the command for shells.
    let detail = payload
        .get("tool_input")
        .and_then(Value::as_object)
        .and_then(|i| {
            i.get("file_path")
                .or_else(|| i.get("path"))
                .or_else(|| i.get("command"))
                .and_then(Value::as_str)
        })
        .unwrap_or("");
    Some((tool.to_string(), truncate(detail, MAX_TOOL_DETAIL_CHARS)))
}

struct Normalized {
    agent: String,
    kind: String,
    summary: String,
    agent_session_id: Option<String>,
    /// The full reply text from a stop event, before any truncation. `None`
    /// for non-stop events. Used by the auto-post path, which applies the
    /// larger `MAX_REPLY_CHARS` budget instead of the notification summary cap.
    full_reply: Option<String>,
}

fn normalize(envelope: &InboxEnvelope) -> Result<Normalized, String> {
    let payload = envelope
        .payload
        .as_object()
        .ok_or("payload must be an object")?;
    let event = string_field(payload, &["hook_event_name"]);
    let notification_type = string_field(payload, &["notification_type"]);
    let (agent, kind) = match envelope.backend.as_str() {
        "claude" => {
            let kind = match (event.as_deref(), notification_type.as_deref()) {
                (Some("Notification"), Some("permission_prompt")) => "permission_required",
                (Some("Notification"), Some("idle_prompt" | "agent_needs_input")) => {
                    "input_required"
                }
                (Some("Notification"), Some("agent_completed")) | (Some("Stop"), _) => "completed",
                (Some("StopFailure"), _) => "failed",
                _ => return Err("unsupported Claude event".into()),
            };
            ("claude", kind)
        }
        "codex" => {
            let kind = match event.as_deref() {
                Some("PermissionRequest") => "permission_required",
                Some("Stop") => "completed",
                _ => return Err("unsupported Codex event".into()),
            };
            ("codex", kind)
        }
        "kiro" => {
            if !matches!(event.as_deref(), Some("stop" | "Stop")) {
                return Err("unsupported Kiro event".into());
            }
            ("kiro", "completed")
        }
        _ => return Err("unsupported backend".into()),
    };
    // The raw reply text, shared between the notification summary (truncated
    // to MAX_SUMMARY_CHARS) and the auto-post path (truncated to MAX_REPLY_CHARS
    // at the call site).
    let raw_reply = string_field(
        payload,
        &[
            "message",
            "last_assistant_message",
            "assistant_response",
            "task_subject",
        ],
    );
    let summary = raw_reply
        .as_deref()
        .map(|s| truncate(s, MAX_SUMMARY_CHARS))
        .unwrap_or_default();
    // Preserve the untruncated text for the auto-post path only when this is
    // a stop/completion event — other events have no reply body worth posting.
    let full_reply = if kind == "completed" { raw_reply } else { None };
    Ok(Normalized {
        agent: agent.into(),
        kind: kind.into(),
        summary,
        agent_session_id: string_field(payload, &["session_id"]),
        full_reply,
    })
}

/// Returns true when the envelope carries a `userPromptSubmit` event (kiro),
/// which marks the beginning of a new user turn. Used to reset the
/// `sent_this_turn` flag so the next stop can auto-post.
fn is_user_prompt_submit(envelope: &InboxEnvelope) -> bool {
    envelope.backend == "kiro"
        && envelope
            .payload
            .get("hook_event_name")
            .and_then(Value::as_str)
            .is_some_and(|e| e.eq_ignore_ascii_case("userpromptsubmit"))
}

fn string_field(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    })
}

fn truncate(input: &str, max: usize) -> String {
    let mut out: String = input.chars().take(max).collect();
    if input.chars().count() > max {
        out.push('…');
    }
    out
}

fn window_key(session: &str, window: usize) -> String {
    format!("{session}:{window}")
}
fn is_urgent(kind: &str) -> bool {
    kind != "completed"
}

fn snapshot_value(state: &State) -> Value {
    let mut items: Vec<_> = state.unread.values().cloned().collect();
    items.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    json!({ "unread": items })
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes).map_err(|e| e.to_string())?;
    std::fs::rename(tmp, path).map_err(|e| e.to_string())
}

fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
}
fn claude_path() -> PathBuf {
    home_dir().join(".claude/settings.json")
}
fn codex_path() -> PathBuf {
    home_dir().join(".codex/hooks.json")
}
fn kiro_path() -> PathBuf {
    home_dir().join(".kiro/hooks/tmux-mobile.json")
}
fn kiro_default_path() -> PathBuf {
    home_dir().join(".kiro/agents/kiro_default.json")
}

fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

fn read_json_object(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    let value: Value = serde_json::from_slice(&std::fs::read(path).map_err(|e| e.to_string())?)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(format!("{} must contain a JSON object", path.display()))
    }
}

fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    atomic_write(
        path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
}

fn command_hook(command: String) -> Value {
    json!({ "type": "command", "command": command })
}

fn install_claude(path: &Path, helper: &str) -> Result<(), String> {
    let mut root = read_json_object(path)?;
    let hooks = root
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .ok_or("Claude hooks must be an object")?;
    add_claude_event(
        hooks,
        "Notification",
        Some("permission_prompt|idle_prompt|agent_needs_input|agent_completed"),
        format!("{helper} claude # {OWNER_MARKER}"),
    )?;
    add_claude_event(
        hooks,
        "Stop",
        None,
        format!("{helper} claude # {OWNER_MARKER}"),
    )?;
    add_claude_event(
        hooks,
        "StopFailure",
        None,
        format!("{helper} claude # {OWNER_MARKER}"),
    )?;
    write_json(path, &root)
}

fn add_claude_event(
    hooks: &mut serde_json::Map<String, Value>,
    event: &str,
    matcher: Option<&str>,
    command: String,
) -> Result<(), String> {
    let entries = hooks
        .entry(event)
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| format!("Claude {event} hooks must be an array"))?;
    // Replace our own entry instead of merely detecting it. Older releases
    // could persist a quoted `~` path that shells cannot expand.
    entries.retain(|value| !value.to_string().contains(OWNER_MARKER));
    let mut entry = json!({ "hooks": [command_hook(command)] });
    if let Some(matcher) = matcher {
        entry
            .as_object_mut()
            .unwrap()
            .insert("matcher".into(), json!(matcher));
    }
    entries.push(entry);
    Ok(())
}

fn install_codex(path: &Path, helper: &str) -> Result<(), String> {
    let mut root = read_json_object(path)?;
    let hooks = root
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .ok_or("Codex hooks must be an object")?;
    add_codex_event(
        hooks,
        "PermissionRequest",
        format!("{helper} codex # {OWNER_MARKER}"),
    )?;
    add_codex_event(hooks, "Stop", format!("{helper} codex # {OWNER_MARKER}"))?;
    write_json(path, &root)
}

fn add_codex_event(
    hooks: &mut serde_json::Map<String, Value>,
    event: &str,
    command: String,
) -> Result<(), String> {
    let entries = hooks
        .entry(event)
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or_else(|| format!("Codex {event} hooks must be an array"))?;
    entries.retain(|value| !value.to_string().contains(OWNER_MARKER));
    entries.push(json!({ "hooks": [command_hook(command)] }));
    Ok(())
}

fn install_kiro(path: &Path, helper: &str) -> Result<(), String> {
    write_json(
        path,
        &json!({
            "version": "v1",
            "hooks": [
                {
                    "name": OWNER_MARKER,
                    "trigger": "Stop",
                    "action": { "type": "command", "command": format!("{helper} kiro # {OWNER_MARKER}") },
                    "enabled": true
                },
                {
                    "name": format!("{OWNER_MARKER}-turn"),
                    "trigger": "UserPromptSubmit",
                    "action": { "type": "command", "command": format!("{helper} kiro # {OWNER_MARKER}") },
                    "enabled": true
                }
            ]
        }),
    )
}

fn find_kiro_cli() -> Option<PathBuf> {
    let home = home_dir();
    [
        home.join(".local/bin/kiro-cli"),
        PathBuf::from("/opt/homebrew/bin/kiro-cli"),
        PathBuf::from("/usr/local/bin/kiro-cli"),
    ]
    .into_iter()
    .find(|path| path.is_file())
    .or_else(|| {
        std::env::var_os("PATH").and_then(|paths| {
            std::env::split_paths(&paths)
                .map(|path| path.join("kiro-cli"))
                .find(|path| path.is_file())
        })
    })
}

fn install_kiro_default(path: &Path, helper: &str) -> Result<(), String> {
    if !path.exists() {
        let cli = find_kiro_cli().ok_or("kiro-cli is not installed")?;
        let dir = path.parent().ok_or("invalid Kiro agent path")?;
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        let output = std::process::Command::new(cli)
            .args([
                "agent",
                "create",
                "kiro_default",
                "--from",
                "kiro_default",
                "--directory",
            ])
            .arg(dir)
            .env("EDITOR", "true")
            .output()
            .map_err(|e| format!("failed to create kiro_default: {e}"))?;
        if !output.status.success() {
            return Err(format!(
                "failed to create kiro_default: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            ));
        }
    }
    let mut root = read_json_object(path)?;
    let hooks = root
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    let hooks = hooks
        .as_object_mut()
        .ok_or("kiro_default hooks must be an object")?;
    let stop = hooks
        .entry("stop")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("kiro_default stop hooks must be an array")?;
    stop.retain(|value| !value.to_string().contains(OWNER_MARKER));
    stop.push(json!({ "command": format!("{helper} kiro # {OWNER_MARKER}") }));
    // userPromptSubmit fires at the start of each user turn. We use it to
    // reset the "sent this turn" flag so the next stop can auto-post.
    let user_prompt = hooks
        .entry("userPromptSubmit")
        .or_insert_with(|| json!([]))
        .as_array_mut()
        .ok_or("kiro_default userPromptSubmit hooks must be an array")?;
    user_prompt.retain(|value| !value.to_string().contains(OWNER_MARKER));
    user_prompt.push(json!({ "command": format!("{helper} kiro # {OWNER_MARKER}") }));
    write_json(path, &root)
}

fn remove_kiro_default_hook(path: &Path) -> Result<(), String> {
    if !path.exists() || !json_file_contains(path, OWNER_MARKER) {
        return Ok(());
    }
    let mut root = read_json_object(path)?;
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        for key in &["stop", "userPromptSubmit"] {
            if let Some(arr) = hooks.get_mut(*key).and_then(Value::as_array_mut) {
                arr.retain(|value| !value.to_string().contains(OWNER_MARKER));
            }
        }
        hooks.retain(|_, value| value.as_array().is_none_or(|items| !items.is_empty()));
    }
    write_json(path, &root)
}

fn remove_owned_hooks(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }
    let mut root = read_json_object(path)?;
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        for value in hooks.values_mut() {
            if let Some(entries) = value.as_array_mut() {
                entries.retain(|entry| !entry.to_string().contains(OWNER_MARKER));
            }
        }
        hooks.retain(|_, value| value.as_array().is_none_or(|entries| !entries.is_empty()));
    }
    write_json(path, &root)
}

fn json_file_contains(path: &Path, needle: &str) -> bool {
    std::fs::read_to_string(path).is_ok_and(|text| text.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Consume order IS render order, because an event is stamped when it is
    /// consumed. Filesystem order is arbitrary, so the listing sorts by the
    /// epoch prefix the helper writes.
    #[test]
    fn inbox_is_consumed_in_the_order_the_hooks_wrote_it() {
        let dir = std::env::temp_dir().join(format!("tmm-inbox-order-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        // Written out of order on purpose, including a second-boundary pair.
        for name in ["1755300010-42.json", "1755300002-7.json", "1755300002-3.json", "notes.txt"] {
            std::fs::write(dir.join(name), "{}").unwrap();
        }
        let got: Vec<String> = inbox_files(&dir)
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        assert_eq!(
            got,
            vec!["1755300002-3.json", "1755300002-7.json", "1755300010-42.json"],
            "oldest first, non-json ignored"
        );
        assert!(inbox_files(&dir.join("missing")).is_empty(), "no inbox yet is not an error");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn normalizes_backend_events() {
        let envelope = |backend: &str, payload: Value| InboxEnvelope {
            backend: backend.into(),
            pane_id: "%1".into(),
            payload,
        };
        assert_eq!(
            normalize(&envelope(
                "claude",
                json!({"hook_event_name":"Notification","notification_type":"permission_prompt"})
            ))
            .unwrap()
            .kind,
            "permission_required"
        );
        assert_eq!(
            normalize(&envelope("codex", json!({"hook_event_name":"Stop"})))
                .unwrap()
                .kind,
            "completed"
        );
        assert_eq!(
            normalize(&envelope("kiro", json!({"hook_event_name":"stop"})))
                .unwrap()
                .kind,
            "completed"
        );
    }

    #[test]
    fn hook_merge_preserves_unrelated_entries() {
        let root = std::env::temp_dir().join(format!("tmm-agent-hooks-{}", uuid::Uuid::new_v4()));
        let path = root.join("settings.json");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(&path, r#"{"theme":"dark","hooks":{"Stop":[{"hooks":[{"type":"command","command":"other"}]}]}}"#).unwrap();
        install_claude(&path, "'/tmp/helper'").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("other"));
        assert!(text.contains(OWNER_MARKER));
        remove_owned_hooks(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("other"));
        assert!(!text.contains(OWNER_MARKER));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reinstall_replaces_stale_owned_hook_command() {
        let root =
            std::env::temp_dir().join(format!("tmm-agent-hook-migrate-{}", uuid::Uuid::new_v4()));
        let path = root.join("hooks.json");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            &path,
            format!(
                r#"{{"hooks":{{"Stop":[{{"hooks":[{{"type":"command","command":"'~/.config/old-helper' codex # {OWNER_MARKER}"}}]}}]}}}}"#
            ),
        )
        .unwrap();

        install_codex(&path, "'/absolute/current-helper'").unwrap();

        let text = std::fs::read_to_string(&path).unwrap();
        assert!(!text.contains("~/.config/old-helper"));
        assert!(text.contains("/absolute/current-helper"));
        assert_eq!(text.matches(OWNER_MARKER).count(), 2);
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    #[test]
    fn helper_is_tmux_scoped_and_best_effort_without_server() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        let root = std::env::temp_dir().join(format!("tmm-agent-helper-{}", uuid::Uuid::new_v4()));
        let hub = AgentNotificationHub::load_at(root.clone());
        hub.write_helper().unwrap();
        let helper = hub.helper_path();

        let run = |pane: Option<&str>| {
            let mut command = Command::new(&helper);
            command
                .arg("codex")
                .env_remove("TMUX_PANE")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::piped());
            if let Some(pane) = pane {
                command.env("TMUX_PANE", pane);
            }
            let mut child = command.spawn().unwrap();
            child
                .stdin
                .take()
                .unwrap()
                .write_all(br#"{"hook_event_name":"Stop"}"#)
                .unwrap();
            child.wait_with_output().unwrap()
        };

        let outside_tmux = run(None);
        assert!(outside_tmux.status.success());
        assert!(std::fs::read_dir(root.join("inbox"))
            .unwrap()
            .next()
            .is_none());

        // No server or inbox consumer is running for this isolated root. Keep
        // stdin open after one JSON line to model CLIs that wait for the hook
        // before closing their pipe; the helper must still exit promptly.
        let mut command = Command::new(&helper);
        command
            .arg("codex")
            .env("TMUX_PANE", "%42")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        let mut child = command.spawn().unwrap();
        let mut stdin = child.stdin.take().unwrap();
        stdin
            .write_all(b"{\"hook_event_name\":\"Stop\"}\n")
            .unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(1);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "helper waited for stdin EOF"
            );
            std::thread::sleep(std::time::Duration::from_millis(10));
        };
        assert!(status.success());
        drop(stdin);

        let event = std::fs::read_dir(root.join("inbox"))
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        let envelope: InboxEnvelope =
            serde_json::from_slice(&std::fs::read(event).unwrap()).unwrap();
        assert_eq!(envelope.backend, "codex");
        assert_eq!(envelope.pane_id, "%42");

        // Even a local delivery failure must never surface as an Agent hook
        // failure; notifications are advisory.
        std::fs::remove_dir_all(root.join("inbox")).unwrap();
        std::fs::write(root.join("inbox"), "not a directory").unwrap();
        let unavailable_inbox = run(Some("%42"));
        assert!(unavailable_inbox.status.success());
        assert!(unavailable_inbox.stderr.is_empty());

        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn record_deduplicates_one_turn() {
        let root = std::env::temp_dir().join(format!("tmm-agent-state-{}", uuid::Uuid::new_v4()));
        let hub = AgentNotificationHub::load_at(root.clone());
        let item = AgentNotification {
            id: "1".into(),
            agent: "codex".into(),
            kind: "completed".into(),
            pane_id: "%1".into(),
            session: "work".into(),
            window: 2,
            pane: 0,
            target: "work:2.0".into(),
            summary: String::new(),
            timestamp: 100,
            agent_session_id: Some("session".into()),
        };
        hub.record(item.clone()).unwrap();
        let mut duplicate = item;
        duplicate.id = "2".into();
        duplicate.timestamp = 102;
        hub.record(duplicate).unwrap();
        assert_eq!(hub.snapshot()["unread"].as_array().unwrap().len(), 1);
        let mut urgent = hub.snapshot()["unread"][0].clone();
        urgent["id"] = json!("3");
        urgent["kind"] = json!("permission_required");
        urgent["timestamp"] = json!(110);
        hub.record(serde_json::from_value(urgent).unwrap()).unwrap();
        let mut completion = hub.snapshot()["unread"][0].clone();
        completion["id"] = json!("4");
        completion["kind"] = json!("completed");
        completion["timestamp"] = json!(111);
        hub.record(serde_json::from_value(completion).unwrap())
            .unwrap();
        assert_eq!(hub.snapshot()["unread"][0]["kind"], "permission_required");
        let _ = std::fs::remove_dir_all(root);
    }

    /// End-to-end over the real inbox and a real tmux pane: a
    /// `userPromptSubmit` envelope must land in telemetry as the input half of
    /// the transcript AND acknowledge a line we typed. The payload shape here
    /// is the one measured from kiro-cli 2.16.2 — `{hook_event_name, cwd,
    /// prompt}` — so the field name this depends on is pinned by a test rather
    /// than by a comment.
    #[test]
    fn a_prompt_envelope_becomes_input_telemetry_and_a_delivery_receipt() {
        let session = format!("tmm-prompt-{}", std::process::id());
        let created = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", &session, "sleep 30"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            eprintln!("no tmux server — skipping");
            return;
        }
        let panes = crate::tmux::list_panes(&session).unwrap_or_default();
        let pane = panes.first().expect("the new session has a pane").clone();
        // resolve_pane_id wants tmux's own `%N` id, which TmuxPane doesn't carry.
        let pane_id = String::from_utf8(
            std::process::Command::new("tmux")
                .args(["display-message", "-p", "-t", &session, "#{pane_id}"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();

        let root = std::env::temp_dir().join(format!("tmm-prompt-hub-{}", uuid::Uuid::new_v4()));
        let hub = AgentNotificationHub::load_at(root.clone());
        // What deliver_mentions types into the pane.
        let line = "[tmm chat] human: @dev ship it";
        crate::projects::telemetry::record_delivery(&session, pane.window, line);

        std::fs::create_dir_all(root.join("inbox")).unwrap();
        let envelope = json!({
            "backend": "kiro",
            "pane_id": pane_id,
            "payload": {
                "hook_event_name": "userPromptSubmit",
                "cwd": "/tmp",
                "prompt": line,
            }
        });
        std::fs::write(
            root.join("inbox").join("1-prompt.json"),
            serde_json::to_vec(&envelope).unwrap(),
        )
        .unwrap();
        hub.consume_inbox();

        let events = crate::projects::telemetry::recent_events(&session, 0);
        let prompt = events.iter().find(|e| e.kind == "prompt").expect("prompt recorded");
        assert_eq!(prompt.text, line, "the input half of the transcript");
        assert_eq!(prompt.via, "app", "and the receipt for the line we typed");
        // A consumed envelope is removed, and no unread notification is created
        // for a turn start.
        assert!(!root.join("inbox").join("1-prompt.json").exists(), "envelope consumed");
        assert_eq!(hub.snapshot()["unread"].as_array().unwrap().len(), 0, "a turn start is not a notification");

        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(root);
    }

    /// The whole auto-post path, end to end: a real tmux window, a real managed
    /// home, a real inbox file carrying a real kiro `stop` payload — and the
    /// agent's final answer must land in the room, record-only, with no
    /// `tmm send` anywhere. This is the behaviour the owner reported missing;
    /// the cause was a config on disk written before `userPromptSubmit` existed
    /// (see `spawn::refresh_hooks`), not this path.
    #[test]
    fn a_stop_payload_posts_the_agents_final_answer_to_the_room() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-auto-{}", std::process::id());
        let ws = std::env::temp_dir().join(format!("tmm-auto-ws-{}", uuid::Uuid::new_v4()));
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
        let adopted = crate::projects::adopt(&session, Some("auto-test")).is_ok();
        let pane_id = String::from_utf8(
            std::process::Command::new("tmux")
                .args(["display-message", "-p", "-t", &session, "#{pane_id}"])
                .output().unwrap().stdout,
        ).unwrap().trim().to_string();

        // A poster that records what it was asked to post.
        struct Spy(std::sync::Mutex<Vec<(String, String, String, bool)>>);
        impl RoomPoster for Spy {
            fn post_to_room(&self, session: &str, agent: &str, body: &str, record_only: bool) {
                self.0.lock().unwrap().push((session.into(), agent.into(), body.into(), record_only));
            }
        }
        let spy = std::sync::Arc::new(Spy(std::sync::Mutex::new(Vec::new())));

        let root = std::env::temp_dir().join(format!("tmm-auto-hub-{}", uuid::Uuid::new_v4()));
        let hub = AgentNotificationHub::load_at(root.clone());
        hub.set_room_poster(spy.clone());
        std::fs::create_dir_all(root.join("inbox")).unwrap();
        // Exactly the payload measured from kiro-cli 2.16.2.
        std::fs::write(
            root.join("inbox").join("1-stop.json"),
            serde_json::to_vec(&json!({
                "backend": "kiro",
                "pane_id": pane_id,
                "payload": {
                    "hook_event_name": "stop",
                    "cwd": ws.to_string_lossy(),
                    "session_id": "conv-1",
                    "assistant_response": "Fixed the flaky test: it assumed a 4 MB read is slow."
                }
            })).unwrap(),
        ).unwrap();
        hub.consume_inbox();

        if adopted {
            let posts = spy.0.lock().unwrap();
            assert_eq!(posts.len(), 1, "the final answer is posted exactly once");
            let (s, agent, body, record_only) = &posts[0];
            assert_eq!(s, &session);
            assert_eq!(agent, "dev", "posted as the agent, by window name");
            assert!(body.contains("Fixed the flaky test"), "the answer itself: {body:?}");
            assert!(*record_only, "hook-sourced text must never be delivered into panes");
        } else {
            eprintln!("could not adopt a project — skipped the assertions");
        }

        // Same turn, second stop after an explicit send: no second message.
        hub.mark_sent_this_turn(&session, 0);
        std::fs::write(
            root.join("inbox").join("2-stop.json"),
            serde_json::to_vec(&json!({
                "backend": "kiro",
                "pane_id": pane_id,
                "payload": { "hook_event_name": "stop", "assistant_response": "again" }
            })).unwrap(),
        ).unwrap();
        hub.consume_inbox();
        assert_eq!(spy.0.lock().unwrap().len(), if adopted { 1 } else { 0 }, "one turn, one message");

        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn turn_start_clears_the_sent_flag_so_later_turns_still_auto_post() {
        let root = std::env::temp_dir().join(format!("tmm-agent-turn-{}", uuid::Uuid::new_v4()));
        let hub = AgentNotificationHub::load_at(root.clone());
        assert!(!hub.already_sent_this_turn("work", 2));
        // Turn N: the agent reports progress itself, so its stop must be mute.
        hub.mark_sent_this_turn("work", 2);
        assert!(hub.already_sent_this_turn("work", 2), "same turn must not post twice");
        // Turn N+1 begins. The userPromptSubmit envelope the helper delivers is
        // the ONLY reset — recognizing it is what keeps the flag from sticking.
        let turn_start = InboxEnvelope {
            backend: "kiro".into(),
            pane_id: "%1".into(),
            payload: json!({"hook_event_name": "userPromptSubmit"}),
        };
        assert!(is_user_prompt_submit(&turn_start), "turn start must be recognized");
        hub.reset_sent_this_turn("work", 2);
        assert!(
            !hub.already_sent_this_turn("work", 2),
            "a send in one turn must not suppress the auto-post of every later turn"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn kiro_agent_merge_preserves_existing_hooks() {
        let root = std::env::temp_dir().join(format!("tmm-kiro-hooks-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        let path = root.join("dev.json");
        std::fs::write(&path, r#"{"name":"dev","hooks":{"postToolUse":[{"command":"lint"}],"stop":[{"command":"other"}]}}"#).unwrap();
        install_kiro_default(&path, "'/tmp/helper'").unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("lint"));
        assert!(text.contains("other"));
        assert!(text.contains(OWNER_MARKER));
        remove_kiro_default_hook(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("lint"));
        assert!(text.contains("other"));
        assert!(!text.contains(OWNER_MARKER));
        let _ = std::fs::remove_dir_all(root);
    }
}
