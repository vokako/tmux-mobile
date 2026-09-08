use crate::{config, tmux};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_INBOX_BYTES: u64 = 256 * 1024;
/// Chat-path budget for hook-sourced auto-replies: a final reply carries full
/// content.
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
const OWNER_MARKER: &str = "tmux-mobile-agent-notify";

// ── Room poster ──────────────────────────────────────────────────────────────

/// Minimal posting interface injected into the hook consumer so final replies
/// can be recorded and delivered without naming the agora bus or TeamBridge.
pub trait RoomPoster: Send + Sync {
    /// Record `body` in the room, then deliver it once to each reply target.
    /// `[reply]` deliveries create no reverse edge, preventing ping-pong.
    fn post_final(&self, session: &str, agent: &str, body: &str, reply_to: &[String]);
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
    pub grok: HookBackendStatus,
}

#[derive(Default)]
struct State {
    /// window key → the agent's own conversation id, from the last hook that
    /// carried one. In memory only: the durable copy is the project slot that
    /// the capturer stamps with it (`src-tauri/src/projects`), because that is
    /// what has to survive the reboot which loses tmux in the first place.
    sessions: HashMap<String, String>,
    /// window key → senders whose addressed request opened the current turn.
    /// Parsed from the stamped input at `userPromptSubmit`; `[reply]` and
    /// legacy `[done]` envelopes deliberately create no reverse edge.
    reply_targets: HashMap<String, Vec<String>>,
    /// Injected by the server after the team bus is ready. `None` on mobile.
    /// Box'd pointer stored here so it shares the Mutex with the rest of state.
    poster: Option<Arc<dyn RoomPoster>>,
}

#[derive(Clone)]
pub struct AgentNotificationHub {
    root: PathBuf,
    state: Arc<Mutex<State>>,
}

impl AgentNotificationHub {
    pub fn load() -> Self {
        Self::load_at(config::config_dir().join("agent-notifications"))
    }

    /// Test-only constructor for OTHER modules' boundary tests (team_rpc's
    /// retired-RPC pin): same as `load_at`, kept off the public API.
    #[cfg(test)]
    pub(crate) fn load_at_for_tests(root: PathBuf) -> Self {
        Self::load_at(root)
    }

    fn load_at(root: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(root.join("inbox"));
        // The unread-inbox UI retired 2026-09-01; nothing reads unread.json
        // any more, so a file left by an older build would sit on disk for
        // ever carrying stale reply summaries. Best-effort: a failure to
        // remove is not a failure to start.
        let _ = std::fs::remove_file(root.join("unread.json"));
        Self { root, state: Arc::new(Mutex::new(State::default())) }
    }

    /// Inject the room poster. Called once by the server after the team bus is
    /// ready. Desktop-only; mobile leaves this as `None`.
    pub fn set_room_poster(&self, poster: Arc<dyn RoomPoster>) {
        self.state.lock().unwrap().poster = Some(poster);
    }

    /// Record who should receive this turn's final reply. Human requests need
    /// no pane delivery; the Hub already shows the room.
    fn start_turn(&self, session: &str, window: usize, prompt: &str) {
        self.state.lock().unwrap().reply_targets.insert(
            window_key(session, window),
            reply_targets(prompt),
        );
    }

    fn take_reply_targets(&self, session: &str, window: usize) -> Vec<String> {
        let key = window_key(session, window);
        if let Some(targets) = self.state.lock().unwrap().reply_targets.remove(&key) {
            return targets;
        }
        crate::projects::telemetry::current_turn_prompt(session, window)
            .map(|prompt| reply_targets(&prompt))
            .unwrap_or_default()
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

    pub async fn run(self: Arc<Self>) {
        loop {
            // consume_inbox is synchronous end to end: directory scan, file
            // reads, tmux subprocesses, bus posts. On the runtime it would
            // stall every RPC sharing that worker for the duration.
            let hub = self.clone();
            let _ = tokio::task::spawn_blocking(move || hub.consume_inbox()).await;
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
                // The pane just painted a tool row — the freshest moment to
                // read its status furniture. Throttled + async inside.
                crate::projects::vitals::sniff_window_soon(&session, window);
            }
            return Ok(());
        }
        // userPromptSubmit marks the start of a turn. Its stamped envelope
        // identifies who should receive the final reply; the prompt itself is
        // also the input half of the transcript and the delivery receipt.
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if is_user_prompt_submit(&envelope) {
            let (session, window, _) = tmux::resolve_pane_id(&envelope.pane_id)?;
            if let Some(prompt) = envelope.payload.get("prompt").and_then(Value::as_str) {
                if !prompt.trim().is_empty() {
                    self.start_turn(&session, window, prompt);
                    crate::projects::telemetry::record_prompt(&session, window, prompt);
                }
            }
            // A turn just opened: sniff while the pane is fresh.
            crate::projects::vitals::sniff_window_soon(&session, window);
            return Ok(());
        }
        // Claude's `idle_prompt` is a NUDGE, not an ask (board #75, measured
        // 2026-09-02: every one of the 7 `input_required` rows in state.db sat
        // exactly 60 s after that window's `completed`, nothing in between —
        // it is Claude Code's "you have been idle for a minute" reminder,
        // fired AFTER the turn ended). Read as an ask it flipped every
        // finished Claude agent from `idle` to amber `waiting for you` a
        // minute later and left a "waiting for input" row in the feed. It is
        // dropped here — silently, not as an error, so already-spawned agents
        // whose hooks still match it do not spam the log — and the matchers
        // no longer subscribe to it. `agent_needs_input` stays a real ask.
        if is_idle_nudge(&envelope) {
            return Ok(());
        }
        let normalized = normalize(&envelope)?;
        let (session, window, pane) = tmux::resolve_pane_id(&envelope.pane_id)?;
        let timestamp = unix_seconds();
        // Resolve the reply edge BEFORE recording this stop. On a server
        // restart the in-memory edge is gone, so the durable activity log
        // recovers the prompt newer than the previous turn end.
        let reply_to = if normalized.kind == "completed" {
            self.take_reply_targets(&session, window)
        } else {
            Vec::new()
        };
        // Feed the telemetry channel BEFORE dedupe: dedupe is a notification-UI
        // concern; status derivation wants every observed fact.
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        {
            crate::projects::telemetry::record_notification(&session, window, &normalized.kind, timestamp);
            // A turn edge (stop, ask) means the CLI just repainted its footer —
            // the context-usage number is at its freshest right here.
            crate::projects::vitals::sniff_window_soon(&session, window);
        }

        // Stop hook final: record the answer and deliver it to the sender whose
        // addressed request opened this turn:
        //   1. Only managed windows (constraint 3): a .tmm/agents/<name> dir
        //      must exist, so direct or adopted agents never auto-post.
        //   2. There must be a reply body worth posting.
        //   3. `[reply]` inputs create no reverse edge, so delivery is one hop.
        #[cfg(not(any(target_os = "android", target_os = "ios")))]
        if normalized.kind == "completed" {
            self.maybe_auto_post(&session, window, &normalized, &reply_to);
        }

        // Remember the agent's own conversation id (even across duplicate
        // events): this map is how a restored window resumes the exact
        // conversation instead of starting a blank one. The unread-inbox UI
        // that used to be recorded here retired 2026-09-01 (owner: "原来我用的
        // 感觉不是很好用") — the project room's auto-post + read cursor and the
        // derived status dots are the one notification language now.
        let _ = pane;
        if let Some(id) = normalized.agent_session_id {
            self.state
                .lock()
                .unwrap()
                .sessions
                .insert(window_key(&session, window), id);
        }
        Ok(())
    }

    /// Record a managed agent's final reply and deliver it along this turn's
    /// reply edge.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    fn maybe_auto_post(
        &self,
        session: &str,
        window: usize,
        normalized: &Normalized,
        reply_to: &[String],
    ) {
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
        // Truncate at the chat-path budget.
        let body = truncate(reply, MAX_REPLY_CHARS);
        let poster = self.state.lock().unwrap().poster.clone();
        if let Some(p) = poster {
            p.post_final(session, &window_name, &body, reply_to);
        }
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
            grok: HookBackendStatus {
                supported: true,
                installed: json_file_contains(&grok_path(), OWNER_MARKER),
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
        install_grok(&grok_path(), &helper)?;
        Ok(self.hook_status())
    }

    pub fn remove_hooks(&self) -> Result<HookStatus, String> {
        remove_owned_hooks(&claude_path())?;
        remove_owned_hooks(&codex_path())?;
        if kiro_path().is_file() && json_file_contains(&kiro_path(), OWNER_MARKER) {
            std::fs::remove_file(kiro_path()).map_err(|e| e.to_string())?;
        }
        remove_kiro_default_hook(&kiro_default_path())?;
        if grok_path().is_file() && json_file_contains(&grok_path(), OWNER_MARKER) {
            std::fs::remove_file(grok_path()).map_err(|e| e.to_string())?;
        }
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
case "$backend" in claude|codex|kiro|grok|omp) ;; *) exit 0 ;; esac
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
    // kiro/claude/codex spell the key snake_case; grok camelCase with
    // snake_case VALUES ("pre_tool_use") — measured on grok 1.0.5.
    let event = payload
        .get("hook_event_name")
        .or_else(|| payload.get("hookEventName"))
        .and_then(Value::as_str)?;
    if !matches!(
        event,
        "PreToolUse" | "PostToolUse" | "preToolUse" | "postToolUse" | "pre_tool_use" | "post_tool_use"
    ) {
        return None;
    }
    let tool = payload
        .get("tool_name")
        .or_else(|| payload.get("toolName"))
        .or_else(|| payload.get("tool"))
        .and_then(Value::as_str)
        .unwrap_or("tool");
    // Best-effort one-arg detail: the file for edits, the command for shells.
    let detail = payload
        .get("tool_input")
        .or_else(|| payload.get("toolInput"))
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
    kind: String,
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
    let (_agent, kind) = match envelope.backend.as_str() {
        "claude" => {
            let kind = match (event.as_deref(), notification_type.as_deref()) {
                (Some("Notification"), Some("permission_prompt")) => "permission_required",
                (Some("Notification"), Some("agent_needs_input")) => "input_required",
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
        // omp payloads are GENERATED by our own telemetry extension in
        // claude's dialect (spawn::omp_telemetry_extension): `Stop` with
        // `last_assistant_message` + `session_id` is a turn's end; the
        // extension already filters auto-continuations (willContinue).
        "omp" => {
            if !matches!(event.as_deref(), Some("Stop")) {
                return Err("unsupported OMP event".into());
            }
            ("omp", "completed")
        }
        "grok" => {
            // grok's key is camelCase; a turn's true end is `stop` with
            // reason "end_turn" — a second observe-only stop fires at session
            // teardown ("shutdown"/"channel_closed") and must not read as a
            // completion (measured, grok 1.0.5). `stop_failure` is the API-
            // error end of a turn.
            let event = string_field(payload, &["hookEventName"]);
            let kind = match event.as_deref() {
                Some("stop") => {
                    let reason = string_field(payload, &["reason"]);
                    if reason.as_deref() != Some("end_turn") {
                        return Err("grok stop without end_turn is not a completion".into());
                    }
                    "completed"
                }
                Some("stop_failure") => "failed",
                _ => return Err("unsupported grok event".into()),
            };
            ("grok", kind)
        }
        _ => return Err("unsupported backend".into()),
    };
    // The raw reply text for the auto-post path (truncated to MAX_REPLY_CHARS
    // at the call site).
    let raw_reply = string_field(
        payload,
        &[
            "message",
            "last_assistant_message",
            "lastAssistantMessage",
            "assistant_response",
            "task_subject",
        ],
    );
    // Preserve the untruncated text for the auto-post path only when this is
    // a stop/completion event — other events have no reply body worth posting.
    let full_reply = if kind == "completed" { raw_reply } else { None };
    Ok(Normalized {
        kind: kind.into(),
        agent_session_id: string_field(payload, &["session_id", "sessionId"]),
        full_reply,
    })
}

/// Returns true when the envelope carries a `userPromptSubmit` event (kiro),
/// which marks the beginning of a new user turn. Used to reset the
/// `sent_this_turn` flag so the next stop can auto-post.
/// The `Notification` types the global Claude install subscribes to. No
/// `idle_prompt`: that is the idle nudge, not an ask (board #75).
fn claude_hooks_matcher() -> &'static str {
    "permission_prompt|agent_needs_input|agent_completed"
}

/// Claude Code's idle reminder (`Notification` / `idle_prompt`): fires ~60 s
/// after a turn ended with nobody typing. Not an ask — see `consume_file`.
fn is_idle_nudge(envelope: &InboxEnvelope) -> bool {
    envelope.backend == "claude"
        && envelope.payload.get("hook_event_name").and_then(Value::as_str) == Some("Notification")
        && envelope.payload.get("notification_type").and_then(Value::as_str) == Some("idle_prompt")
}

fn is_user_prompt_submit(envelope: &InboxEnvelope) -> bool {
    match envelope.backend.as_str() {
        "kiro" => envelope
            .payload
            .get("hook_event_name")
            .and_then(Value::as_str)
            .is_some_and(|e| e.eq_ignore_ascii_case("userpromptsubmit")),
        // claude and codex share kiro's snake_case key with PascalCase values
        // ("UserPromptSubmit"; codex measured on codex-cli 0.148.0 — payload
        // also carries `prompt` + `session_id`, same as kiro/claude). They
        // shipped WITHOUT this arm, which made the dedup flag sticky for
        // their windows: the first `tmm send` suppressed the auto-post for
        // every later turn, and deliveries were never acked.
        // omp speaks the same dialect by construction: its telemetry
        // extension (spawn::omp_telemetry_extension) EMITS claude-shaped
        // payloads, so no third spelling exists.
        "claude" | "codex" | "omp" => envelope
            .payload
            .get("hook_event_name")
            .and_then(Value::as_str)
            .is_some_and(|e| e.eq_ignore_ascii_case("userpromptsubmit")),
        // grok 1.0.5: camelCase key, snake_case value (measured).
        "grok" => envelope
            .payload
            .get("hookEventName")
            .and_then(Value::as_str)
            .is_some_and(|e| e.eq_ignore_ascii_case("user_prompt_submit")),
        _ => false,
    }
}

fn string_field(map: &serde_json::Map<String, Value>, keys: &[&str]) -> Option<String> {
    keys.iter().find_map(|key| {
        map.get(*key)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_owned)
    })
}

/// Senders of stamped chat requests in a submitted prompt. Automatic
/// `[reply]` and legacy `[done]` deliveries are results, not new requests.
fn reply_targets(prompt: &str) -> Vec<String> {
    let mut targets = Vec::new();
    for line in prompt.lines() {
        let Some(after_stamp) = line
            .strip_prefix("[tmm chat] ")
            .or_else(|| line.strip_prefix("[tmm chat ").and_then(|s| s.split_once("] ").map(|(_, rest)| rest)))
        else {
            continue;
        };
        let Some((sender, body)) = after_stamp.split_once(": ") else { continue };
        let sender = sender.trim();
        let body = body.trim_start();
        if sender.is_empty()
            || sender == "human"
            || body.starts_with("[reply]")
            || body.starts_with("[done]")
            || targets.iter().any(|s| s == sender)
        {
            continue;
        }
        targets.push(sender.to_string());
    }
    targets
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
fn grok_path() -> PathBuf {
    home_dir().join(".grok/hooks/tmux-mobile.json")
}

fn kiro_path() -> PathBuf {
    home_dir().join(".kiro/hooks/tmux-mobile.json")
}
fn kiro_default_path() -> PathBuf {
    home_dir().join(".kiro/agents/kiro_default.json")
}

/// ALWAYS quotes — deliberately not the quote-if-needed `shell_quote` the
/// launchers share (`team::backends`): this one writes hook command strings
/// into the agents' config files, and the quoted form is what
/// `patch_hooks`/`refresh_hooks` compare against on disk. Switching to the
/// minimal form would mark every existing agent's hooks stale once. It is
/// also the only quoter compiled on Android, where `team` does not exist.
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
        Some(claude_hooks_matcher()),
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

/// Global grok hooks: `~/.grok/hooks/tmux-mobile.json`, a file we own whole
/// (grok merges hook files, so ours never touches the user's). Stop is
/// filtered to end_turn by the normalizer; UserPromptSubmit resets the
/// same-turn dedup flag for direct grok windows the way kiro_default does.
fn install_grok(path: &Path, helper: &str) -> Result<(), String> {
    let cmd = format!("{helper} grok # {OWNER_MARKER}");
    write_json(
        path,
        &json!({
            "hooks": {
                "UserPromptSubmit": [ { "hooks": [ { "type": "command", "command": cmd } ] } ],
                "Stop": [ { "hooks": [ { "type": "command", "command": cmd } ] } ],
                "StopFailure": [ { "hooks": [ { "type": "command", "command": cmd } ] } ]
            }
        }),
    )
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

    /// A legacy build's unread.json is REMOVED at load (the retired unread
    /// inbox must not leave stale reply summaries on disk for ever), and the
    /// hub stays functional: the inbox dir exists for the helper to write into.
    #[test]
    fn load_removes_the_legacy_unread_file_and_keeps_the_inbox() {
        let root = std::env::temp_dir().join(format!("tmm-legacy-unread-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("unread.json"), r#"[{"summary":"stale"}]"#).unwrap();
        let hub = AgentNotificationHub::load_at(root.clone());
        assert!(!root.join("unread.json").exists(), "legacy file removed at load");
        assert!(root.join("inbox").is_dir(), "the hook inbox is still provisioned");
        // The surviving surfaces still answer.
        assert!(hub.agent_session_for("none", 0).is_none());
        let _ = std::fs::remove_dir_all(root);
    }

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
        // board #75: a real question is an ask; the 60 s idle reminder is not
        // an event at all (consume_file drops it before normalize).
        assert_eq!(
            normalize(&envelope(
                "claude",
                json!({"hook_event_name":"Notification","notification_type":"agent_needs_input"})
            ))
            .unwrap()
            .kind,
            "input_required"
        );
        let nudge = envelope(
            "claude",
            json!({"hook_event_name":"Notification","notification_type":"idle_prompt"}),
        );
        assert!(is_idle_nudge(&nudge));
        assert!(normalize(&nudge).is_err(), "no ask kind is ever derived from the idle nudge");
        assert!(!is_idle_nudge(&envelope("claude", json!({"hook_event_name":"Notification","notification_type":"permission_prompt"}))));
        assert!(!claude_hooks_matcher().contains("idle_prompt"), "new configs do not subscribe to the nudge");
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
        // grok (1.0.5, payloads measured live): camelCase keys, snake_case
        // event values. A turn's true end is stop + reason end_turn, and it
        // carries the reply in lastAssistantMessage.
        let done = normalize(&envelope(
            "grok",
            json!({"hookEventName":"stop","reason":"end_turn","lastAssistantMessage":"OK","sessionId":"01a0-abc"}),
        ))
        .unwrap();
        assert_eq!(done.kind, "completed");
        assert_eq!(done.full_reply.as_deref(), Some("OK"));
        assert_eq!(done.agent_session_id.as_deref(), Some("01a0-abc"));
        // The session-teardown stop ("shutdown"/"channel_closed") fires too and
        // must NOT read as a completion — it would double-post every reply.
        assert!(normalize(&envelope("grok", json!({"hookEventName":"stop","reason":"shutdown"}))).is_err());
        assert_eq!(
            normalize(&envelope("grok", json!({"hookEventName":"stop_failure","error":"rate_limit"})))
                .unwrap()
                .kind,
            "failed"
        );
        // The turn-start reset + tool telemetry recognize grok's spellings.
        assert!(is_user_prompt_submit(&envelope(
            "grok",
            json!({"hookEventName":"user_prompt_submit","prompt":"<user_query>\nhi\n</user_query>"})
        )));
        let (tool, detail) = tool_event_parts(&envelope(
            "grok",
            json!({"hookEventName":"pre_tool_use","toolName":"run_terminal_command","toolInput":{"command":"npm test"}}),
        ))
        .unwrap();
        assert_eq!((tool.as_str(), detail.as_str()), ("run_terminal_command", "npm test"));
    }

    /// claude and codex spell the turn start snake-key/Pascal-value
    /// ("hook_event_name":"UserPromptSubmit"). Codex measured on codex-cli
    /// 0.148.0 (payload carries prompt + session_id, arrives on hook stdin);
    /// claude's documented schema is the same family. Without these arms the
    /// dedup flag never reset for their windows — the first `tmm send`
    /// suppressed every later auto-post, and delivered lines were never acked.
    #[test]
    fn claude_and_codex_turn_starts_are_recognized() {
        let envelope = |backend: &str, payload: Value| InboxEnvelope {
            backend: backend.into(),
            pane_id: "%1".into(),
            payload,
        };
        for backend in ["claude", "codex"] {
            assert!(
                is_user_prompt_submit(&envelope(
                    backend,
                    json!({"hook_event_name":"UserPromptSubmit","prompt":"hi","session_id":"01a0-abc"})
                )),
                "{backend} turn start must be recognized"
            );
            // Tool events keep routing to telemetry, never to the reset path.
            assert!(!is_user_prompt_submit(&envelope(
                backend,
                json!({"hook_event_name":"PreToolUse","tool_name":"Read"})
            )));
        }
    }

    /// omp payloads come from our OWN telemetry extension
    /// (spawn::omp_telemetry_extension), which speaks claude's dialect by
    /// construction: UserPromptSubmit resets the turn flag, Stop is the
    /// completion carrying the reply and the session id.
    #[test]
    fn omp_events_ride_the_claude_dialect() {
        let envelope = |payload: Value| InboxEnvelope {
            backend: "omp".into(),
            pane_id: "%1".into(),
            payload,
        };
        assert!(is_user_prompt_submit(&envelope(
            json!({"hook_event_name":"UserPromptSubmit","prompt":"hi","session_id":"0199-abc"})
        )));
        let n = normalize(&envelope(
            json!({"hook_event_name":"Stop","last_assistant_message":"done and verified","session_id":"0199-abc"}),
        ))
        .unwrap();
        assert_eq!(n.kind, "completed");
        assert_eq!(n.full_reply.as_deref(), Some("done and verified"));
        assert_eq!(n.agent_session_id.as_deref(), Some("0199-abc"));
        // Tool events route to telemetry, and unknown events are errors.
        let (tool, detail) = tool_event_parts(&envelope(
            json!({"hook_event_name":"PreToolUse","tool_name":"bash","tool_input":{"command":"npm test"}}),
        ))
        .unwrap();
        assert_eq!((tool.as_str(), detail.as_str()), ("bash", "npm test"));
        assert!(normalize(&envelope(json!({"hook_event_name":"SessionStart"}))).is_err());
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
            let write = child
                .stdin
                .take()
                .unwrap()
                .write_all(br#"{"hook_event_name":"Stop"}"#);
            // With no TMUX_PANE the helper is REQUIRED to exit immediately,
            // before reading stdin. It may therefore close the pipe before
            // this parent write wins the race; BrokenPipe is the expected
            // shape of that successful early exit. With a pane, the payload
            // is load-bearing and must be accepted.
            if pane.is_some() {
                write.unwrap();
            }
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
        // A consumed envelope is removed.
        assert!(!root.join("inbox").join("1-prompt.json").exists(), "envelope consumed");

        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(root);
    }

    /// The same path with a SERVER RESTART in the middle — board #5. The agent
    /// is a separate process: it queued our line and submits it when its turn
    /// ends, which can be after we came back. The receipt used to die with the
    /// old process, so the recovered echo was filed as keyboard input (`via:
    /// local`) and the message stayed unconfirmed (owner, 2026-08-29: "后端的服务
    /// 有重启了，然后agent又收到指令确认hooks，这个hooks没有正确把之前的未确认的
    /// 消息变成已读状态，被单独写出来了"). Now the queue is in state.db, so the
    /// hook still recognises the prompt as ours.
    #[test]
    fn a_prompt_envelope_after_a_restart_is_still_a_delivery_receipt() {
        crate::projects::tests::use_test_store();
        let session = format!("tmm-restart-{}", std::process::id());
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

        let root = std::env::temp_dir().join(format!("tmm-restart-hub-{}", uuid::Uuid::new_v4()));
        let hub = AgentNotificationHub::load_at(root.clone());
        let line = "[tmm chat] human: @dev restart proof";
        crate::projects::telemetry::record_delivery(&session, pane.window, line);
        // ── the restart: a fresh process has no telemetry records at all.
        crate::projects::telemetry::forget_process_state(&session);

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
        assert_eq!(
            prompt.via, "app",
            "the echo of a line typed before the restart is still OUR delivery"
        );

        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(root);
    }

    /// The whole auto-post path, end to end: a real tmux window, a real managed
    /// home, a real inbox file carrying a real kiro `stop` payload — and the
    /// agent's final answer must land in the room and carry the turn's reply
    /// edge.
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
        struct Spy(std::sync::Mutex<Vec<(String, String, String, Vec<String>)>>);
        impl RoomPoster for Spy {
            fn post_final(&self, session: &str, agent: &str, body: &str, reply_to: &[String]) {
                self.0.lock().unwrap().push((
                    session.into(), agent.into(), body.into(), reply_to.to_vec()
                ));
            }
        }
        let spy = std::sync::Arc::new(Spy(std::sync::Mutex::new(Vec::new())));

        let root = std::env::temp_dir().join(format!("tmm-auto-hub-{}", uuid::Uuid::new_v4()));
        let hub = AgentNotificationHub::load_at(root.clone());
        hub.set_room_poster(spy.clone());
        let (_, win, _) = crate::tmux::resolve_pane_id(&pane_id).expect("pane resolves");
        hub.start_turn(&session, win, "[tmm chat 2026-09-08 08:00] lead: @dev fix it");
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
            let (s, agent, body, reply_to) = &posts[0];
            assert_eq!(s, &session);
            assert_eq!(agent, "dev", "posted as the agent, by window name");
            assert!(body.contains("Fixed the flaky test"), "the answer itself: {body:?}");
            assert_eq!(reply_to, &vec!["lead".to_string()]);
        } else {
            eprintln!("could not adopt a project — skipped the assertions");
        }

        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &session]).status();
        let _ = std::fs::remove_dir_all(&ws);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn reply_targets_are_addressed_once_and_replies_do_not_loop() {
        assert_eq!(
            reply_targets(
                "[tmm chat 2026-09-08 08:00] lead: @dev fix it\n\
                 [tmm chat 2026-09-08 08:01] reviewer: @dev check it\n\
                 [tmm chat 2026-09-08 08:02] lead: @dev one more thing"
            ),
            vec!["lead", "reviewer"]
        );
        assert!(reply_targets("[tmm chat 2026-09-08 08:03] human: @dev ship it").is_empty());
        assert!(reply_targets("[tmm chat 2026-09-08 08:04] lead: [reply] looks good").is_empty());
        assert!(reply_targets("[tmm chat 2026-09-08 08:05] worker: [done] old completion").is_empty());
    }

    #[test]
    fn a_restart_recovers_the_reply_edge_from_prompt_activity() {
        let session = format!("reply-restart-{}", uuid::Uuid::new_v4());
        crate::projects::telemetry::record_prompt(
            &session,
            2,
            "[tmm chat 2026-09-08 08:06] lead: @worker finish it",
        );
        let root = std::env::temp_dir().join(format!("tmm-reply-edge-{}", uuid::Uuid::new_v4()));
        let restarted = AgentNotificationHub::load_at(root.clone());
        assert_eq!(restarted.take_reply_targets(&session, 2), vec!["lead"]);
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
