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
const DEDUPE_SECS: u64 = 3;
const OWNER_MARKER: &str = "tmux-mobile-agent-notify";

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
            state: Arc::new(Mutex::new(State { unread })),
            tx,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    pub fn snapshot(&self) -> Value {
        let state = self.state.lock().unwrap();
        snapshot_value(&state)
    }

    pub fn mark_read(&self, session: &str, window: usize) -> Result<Value, String> {
        let changed = {
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
        let inbox = self.root.join("inbox");
        let Ok(entries) = std::fs::read_dir(&inbox) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("json") {
                continue;
            }
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
        let normalized = normalize(&envelope)?;
        let (session, window, pane) = tmux::resolve_pane_id(&envelope.pane_id)?;
        let timestamp = unix_seconds();
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

    fn record(&self, item: AgentNotification) -> Result<(), String> {
        let changed = {
            let mut state = self.state.lock().unwrap();
            let key = window_key(&item.session, item.window);
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

struct Normalized {
    agent: String,
    kind: String,
    summary: String,
    agent_session_id: Option<String>,
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
    let summary = string_field(
        payload,
        &[
            "message",
            "last_assistant_message",
            "assistant_response",
            "task_subject",
        ],
    )
    .map(|s| truncate(&s, MAX_SUMMARY_CHARS))
    .unwrap_or_default();
    Ok(Normalized {
        agent: agent.into(),
        kind: kind.into(),
        summary,
        agent_session_id: string_field(payload, &["session_id"]),
    })
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
            "hooks": [{
                "name": OWNER_MARKER,
                "trigger": "Stop",
                "action": { "type": "command", "command": format!("{helper} kiro # {OWNER_MARKER}") },
                "enabled": true
            }]
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
    write_json(path, &root)
}

fn remove_kiro_default_hook(path: &Path) -> Result<(), String> {
    if !path.exists() || !json_file_contains(path, OWNER_MARKER) {
        return Ok(());
    }
    let mut root = read_json_object(path)?;
    if let Some(hooks) = root.get_mut("hooks").and_then(Value::as_object_mut) {
        if let Some(stop) = hooks.get_mut("stop").and_then(Value::as_array_mut) {
            stop.retain(|value| !value.to_string().contains(OWNER_MARKER));
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
