//! Desktop-only glue between the tmux-mobile WS server and the team bus.
//!
//! Multi-team manager. Each **team** is an isolated chat **room**, identified
//! by a stable workspace+template slug and backed by its own `agora::Bus` on the
//! shared SQLite db. A single MCP daemon serves them all (agents pick a room via
//! the `x-room` header); the phone passes the active room with each `team_*`
//! RPC. Every room's messages funnel into one re-broadcast channel (each
//! `Message` carries its `room`), and the phone filters to the team currently
//! in view.
//!
//! Compiled ONLY on desktop (lib.rs `#[cfg(...)]`); mobile passes `None`.

use crate::team::{self, TeamConfig};
use crate::server::TeamBridge;
use agora::bus::{Bus, BusProvider};
use agora::envelope::Message;
use agora::store::SharedConn;
use std::collections::HashMap;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::broadcast;

const SESSION_PREFIX: &str = "tmm-team-";

/// One live team: its bus + launch metadata.
struct Team {
    bus: Bus,
    workspace: String,
    template: String,  // roster template this team was started from
    session: String,   // tmm-team-<room>
    history_path: Option<PathBuf>,
    started: bool,     // supervisor launched for this room
    pump: tokio::task::JoinHandle<()>, // the Message→JSON re-broadcast task
}

pub struct TeamManager {
    /// One shared SQLite connection for ALL rooms (WAL single-writer).
    conn: SharedConn,
    /// room -> Team. Guarded by a std Mutex (never held across .await).
    teams: Mutex<HashMap<String, Team>>,
    /// Merged message fan-out for the WS push path (all rooms).
    json_tx: broadcast::Sender<String>,
    /// Server-level launcher config (bus URL + default model).
    cfg: TeamConfig,
    /// Persistent room→workspace+template identity registry. It retains closed
    /// Teams so the same pair can resume the same room history later.
    meta_path: std::path::PathBuf,
    self_ref: OnceLock<Weak<TeamManager>>,
}

impl TeamManager {
    /// Open the store, start the MCP/dashboard daemon (room-aware), and recover
    /// any teams still running in tmux from a previous run.
    pub fn start(db: &str, _room: &str, bind: &str, model: &str) -> Result<Arc<TeamManager>, Box<dyn std::error::Error>> {
        // Write the built-in default roster template if the teams/ dir is empty,
        // so the user always has something to pick + edit.
        team::ensure_templates_seeded();
        if let Some(parent) = std::path::Path::new(db).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        let conn = agora::store::open_shared(db)?;
        let (json_tx, _) = broadcast::channel::<String>(1024);
        let port = bind.rsplit(':').next().unwrap_or("8787");
        let cfg = TeamConfig {
            url: format!("http://127.0.0.1:{}", port),
            model: model.to_string(),
            system_prompt: String::new(),
            team_rules: crate::config::Config::load().team_rules,
            team_kick: crate::config::Config::load().team_kick,
            codex_profile: crate::config::Config::load().team_codex_profile,
        };
        // The room identity registry lives next to the db so restart recovery
        // and close/relaunch both retain workspace+template ownership.
        let meta_path = Path::new(db)
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("teams.json");

        let me = Arc::new(TeamManager {
            conn,
            teams: Mutex::new(HashMap::new()),
            json_tx,
            cfg,
            meta_path,
            self_ref: OnceLock::new(),
        });
        let _ = me.self_ref.set(Arc::downgrade(&me));

        // Serve MCP + dashboard for external agents. The provider is the manager
        // itself (room-aware). Bind failure is non-fatal — the phone path still
        // works in-process.
        {
            println!("🜂 team manager ready (db {}); MCP + dashboard → http://{}/", db, bind);
            let provider: Arc<dyn BusProvider> = me.clone();
            let bind = bind.to_string();
            tokio::spawn(async move {
                if let Err(e) = agora::web::serve(provider, &bind).await {
                    eprintln!("⚠️  team daemon not listening on {}: {}", bind, e);
                }
            });
        }

        me.recover_running_teams();
        Ok(me)
    }

    /// On startup, find teams still alive in tmux (sessions named tmm-team-*)
    /// and resume supervising them — the server can restart without abandoning
    /// running agents. Their workspace comes from the persisted meta map.
    fn recover_running_teams(self: &Arc<Self>) {
        let meta = self.load_meta();
        for session in crate::tmux::list_team_sessions(SESSION_PREFIX) {
            let room = match session.strip_prefix(SESSION_PREFIX) {
                Some(r) if !r.is_empty() => r.to_string(),
                _ => continue,
            };
            let (workspace, template) = meta
                .get(&room)
                .map(|(w, t)| (w.clone(), t.clone()))
                .unwrap_or_default();
            if self.ensure_room(&room, &workspace, &template).is_err() {
                continue;
            }
            // Mark started + relaunch the reconcile loop, which ADOPTS the
            // existing agent windows (preserving each agent's conversation
            // context + in-flight work) rather than reopening them.
            if let Some(t) = self.teams.lock().unwrap().get_mut(&room) {
                t.started = true;
            }
            if let Some(arc) = self.self_ref.get().and_then(|w| w.upgrade()) {
                // Freeze the pre-recovery windows before the supervisor starts.
                // A missing agent may launch during the nudge delay; that fresh
                // CLI must not be interrupted during its first-run setup.
                let adopted = crate::tmux::list_named_windows(&session);
                let bridge: Arc<dyn TeamBridge> = arc;
                team::start(bridge.clone(), self.cfg.clone(), room.clone(), workspace, template);
                // After one heartbeat interval, reconnect only adopted idle
                // waits that did not recover. Active work is never interrupted.
                team::nudge_adopted_agents(bridge, room.clone(), adopted);
                println!("🜂 team: recovered running team '{}' (adopting agents)", room);
            }
        }
    }

    /// room → (workspace, template), persisted to teams.json.
    fn load_meta(&self) -> HashMap<String, (String, String)> {
        let v: serde_json::Value = std::fs::read_to_string(&self.meta_path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or(serde_json::Value::Null);
        let mut map = HashMap::new();
        if let Some(obj) = v.as_object() {
            for (room, entry) in obj {
                let ws = entry.get("workspace").and_then(|x| x.as_str()).unwrap_or("").to_string();
                let tpl = entry.get("template").and_then(|x| x.as_str()).unwrap_or("default").to_string();
                map.insert(room.clone(), (ws, tpl));
            }
        }
        map
    }

    /// Persist room→{workspace,template} for every known team (best-effort).
    /// Existing closed entries are retained as durable identity mappings.
    fn save_meta(&self) {
        let mut map = std::fs::read_to_string(&self.meta_path)
            .ok()
            .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default();
        for (room, team) in self.teams.lock().unwrap().iter() {
            map.insert(
                room.clone(),
                serde_json::json!({
                    "workspace": team.workspace,
                    "template": team.template,
                }),
            );
        }
        if let Ok(s) = serde_json::to_string_pretty(&serde_json::Value::Object(map)) {
            let _ = std::fs::write(&self.meta_path, s);
        }
    }

    fn known_room_for(&self, workspace: &str, template: &str) -> Option<String> {
        let meta = self.load_meta();
        let matches = |room: &str| {
            meta.get(room).is_some_and(|(known_workspace, known_template)| {
                team::same_workspace(known_workspace, workspace)
                    && known_template == template
            })
        };
        let current = team::team_slug(workspace, template);
        if matches(&current) {
            return Some(current);
        }
        let legacy = team::workspace_slug(workspace);
        if matches(&legacy) {
            return Some(legacy);
        }
        let mut rooms: Vec<String> = meta
            .iter()
            .filter(|(_, (known_workspace, known_template))| {
                team::same_workspace(known_workspace, workspace)
                    && known_template == template
            })
            .map(|(room, _)| room.clone())
            .collect();
        rooms.sort();
        rooms.into_iter().next()
    }

    /// Get an existing room's bus, or register it over the shared connection and
    /// start its re-broadcast pump. `workspace`/`template` recorded on first open.
    fn ensure_room(&self, room: &str, workspace: &str, template: &str) -> Result<Bus, String> {
        {
            let teams = self.teams.lock().unwrap();
            if let Some(t) = teams.get(room) {
                return Ok(t.bus.clone());
            }
        }
        // All rooms share ONE connection (no per-room write contention).
        let bus = Bus::with_shared(self.conn.clone(), room.to_string());
        // Subscribe before the snapshot. Messages committed during the snapshot
        // are queued and then skipped by seq if already included, never missed.
        let mut rx = bus.subscribe();
        let history_path = workspace_history_path(workspace, room);
        let mut mirrored_seq = history_path
            .as_deref()
            .map(|path| write_history_snapshot(&bus, path))
            .transpose()?
            .unwrap_or(0);
        // Pump this room's messages into the merged push channel. The handle is
        // stored on the Team so close_team can abort it (otherwise reopening the
        // same room would spawn a second pump → duplicate pushes).
        let pump = {
            let tx = self.json_tx.clone();
            let mirror_bus = bus.clone();
            let mirror_path = history_path.clone();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(m) => {
                            if let Some(path) = mirror_path.as_deref() {
                                if m.seq > mirrored_seq {
                                    match append_history_message(path, &m) {
                                        Ok(()) => mirrored_seq = m.seq,
                                        Err(e) => {
                                            eprintln!("⚠️  team: history mirror append failed: {}", e);
                                            if let Ok(seq) = write_history_snapshot(&mirror_bus, path) {
                                                mirrored_seq = seq;
                                            }
                                        }
                                    }
                                }
                            }
                            if let Ok(s) = serde_json::to_string(&m) {
                                let _ = tx.send(s);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => {
                            if let Some(path) = mirror_path.as_deref() {
                                if let Ok(seq) = write_history_snapshot(&mirror_bus, path) {
                                    mirrored_seq = seq;
                                }
                            }
                        }
                        Err(broadcast::error::RecvError::Closed) => return,
                    }
                }
            })
        };
        {
            let mut teams = self.teams.lock().unwrap();
            // Double-checked: another thread may have inserted while we built.
            if let Some(t) = teams.get(room) {
                pump.abort(); // discard the duplicate pump we just spawned
                return Ok(t.bus.clone());
            }
            teams.insert(
                room.to_string(),
                Team {
                    bus: bus.clone(),
                    workspace: workspace.to_string(),
                    template: if template.is_empty() { "default".to_string() } else { template.to_string() },
                    session: format!("{}{}", SESSION_PREFIX, room),
                    history_path,
                    started: false,
                    pump,
                },
            );
        }
        self.save_meta();
        Ok(bus)
    }

    /// Bus for a known room (no creation). None if the room isn't registered.
    fn room_bus(&self, room: &str) -> Option<Bus> {
        self.teams.lock().unwrap().get(room).map(|t| t.bus.clone())
    }
}

// The manager IS the MCP/web room provider: agents' `x-room` header selects a
// team. We only serve rooms that already exist (a team must be started first),
// so an agent can't spin up a phantom room by guessing a header.
impl BusProvider for TeamManager {
    fn bus_for(&self, room: &str) -> Option<Bus> {
        self.room_bus(room)
    }
    fn default_room(&self) -> String {
        // First registered team, else "main" (harmless; bus_for returns None
        // until a team exists).
        self.teams
            .lock()
            .unwrap()
            .keys()
            .next()
            .cloned()
            .unwrap_or_else(|| "main".to_string())
    }
}

impl TeamBridge for TeamManager {
    fn history(&self, room: &str, limit: i64) -> serde_json::Value {
        let msgs = self.room_bus(room).and_then(|b| b.history(limit).ok()).unwrap_or_default();
        serde_json::json!({ "messages": msgs })
    }

    fn roster(&self, room: &str) -> serde_json::Value {
        let roster = self.room_bus(room).and_then(|b| b.roster().ok()).unwrap_or_default();
        serde_json::json!({ "roster": roster })
    }

    fn post(&self, room: &str, from: &str, body: &str, requires_reply: bool) -> Result<serde_json::Value, String> {
        let bus = self.room_bus(room).ok_or_else(|| format!("unknown team '{room}'"))?;
        bus.post(from, body, requires_reply)
            .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
            .map_err(|e| e.to_string())
    }

    fn set_agent_status(&self, room: &str, agent: &str, status: &str) -> Result<(), String> {
        let bus = self.room_bus(room).ok_or_else(|| format!("unknown team '{room}'"))?;
        bus.set_status(agent, status).map_err(|e| e.to_string())
    }

    fn employees(&self, room: &str) -> serde_json::Value {
        let employees = self.room_bus(room).and_then(|b| b.employees().ok()).unwrap_or_default();
        serde_json::json!({ "employees": employees })
    }

    fn seed_employee(&self, room: &str, name: &str, spec: &serde_json::Value) -> Result<(), String> {
        let bus = self.room_bus(room).ok_or_else(|| format!("unknown team '{room}'"))?;
        bus.seed_employee(name, spec).map_err(|e| e.to_string())
    }

    fn room_exists(&self, room: &str) -> bool {
        self.teams.lock().unwrap().contains_key(room)
    }

    fn employee_specs(&self, room: &str) -> Vec<(String, serde_json::Value, String)> {
        self.room_bus(room)
            .and_then(|b| b.employees().ok())
            .unwrap_or_default()
            .into_iter()
            .map(|e| (e.name, e.spec, e.state))
            .collect()
    }

    fn start_team(&self, workspace: &str, template: &str) -> serde_json::Value {
        let ws = if workspace.trim().is_empty() { self.default_workspace() } else { workspace.trim().to_string() };
        let tpl = if template.trim().is_empty() { "default".to_string() } else { template.trim().to_string() };

        // Self-heal the built-in templates (the teams/ dir may have been deleted)
        // and refuse up front if the chosen roster is missing/empty — otherwise
        // the team would "start" with zero agents and the UI would spin forever.
        team::ensure_templates_seeded();
        if team::read_template(&tpl).is_empty() {
            return serde_json::json!({ "started": false, "workspace": ws,
                "error": format!("template '{}' not found or empty", tpl) });
        }

        // Preserve idempotency and live legacy recovery. An old workspace-only
        // room may already represent this exact pair; return it rather than
        // launching a second copy under the new workspace+template ID.
        let existing = self
            .teams
            .lock()
            .unwrap()
            .iter()
            .find(|(_, t)| team::same_workspace(&t.workspace, &ws) && t.template == tpl)
            .map(|(room, t)| (room.clone(), t.started));
        if let Some((room, true)) = &existing {
            return serde_json::json!({
                "started": false,
                "room": room,
                "workspace": ws,
                "template": tpl,
                "active": true,
            });
        }

        let room = existing
            .map(|(room, _)| room)
            .or_else(|| self.known_room_for(&ws, &tpl))
            .unwrap_or_else(|| team::team_slug(&ws, &tpl));

        // Open/register the room, then mark it started (one-shot per room).
        if let Err(e) = self.ensure_room(&room, &ws, &tpl) {
            return serde_json::json!({ "started": false, "room": room, "workspace": ws, "error": e });
        }
        let already = {
            let mut teams = self.teams.lock().unwrap();
            match teams.get_mut(&room) {
                Some(t) => {
                    let was = t.started;
                    t.started = true;
                    if !was { t.template = tpl.clone(); }
                    was
                }
                None => false,
            }
        };
        if already {
            return serde_json::json!({ "started": false, "room": room, "workspace": ws });
        }
        // An explicit launch gets a fresh runtime roster while retaining the
        // room transcript. The chosen template defines who comes online, and
        // replacement agents can recover prior context from the history mirror.
        if let Some(bus) = self.room_bus(&room) {
            if let Err(e) = bus.reset_runtime() {
                eprintln!("⚠️  team: reset runtime for room '{}' failed: {}", room, e);
            }
        }
        self.save_meta();
        match self.self_ref.get().and_then(|w| w.upgrade()) {
            Some(arc) => {
                let bridge: Arc<dyn TeamBridge> = arc;
                team::start(bridge, self.cfg.clone(), room.clone(), ws.clone(), tpl.clone());
                serde_json::json!({ "started": true, "room": room, "workspace": ws, "template": tpl })
            }
            None => {
                if let Some(t) = self.teams.lock().unwrap().get_mut(&room) { t.started = false; }
                serde_json::json!({ "started": false, "room": room, "workspace": ws, "error": "manager gone" })
            }
        }
    }

    fn close_team(&self, room: &str) -> bool {
        let removed = {
            let mut teams = self.teams.lock().unwrap();
            teams.remove(room)
        };
        match removed {
            Some(t) => {
                t.pump.abort(); // stop the re-broadcast task (no leak / no dup on reopen)
                if let Some(path) = t.history_path.as_deref() {
                    if let Err(e) = write_history_snapshot(&t.bus, path) {
                        eprintln!("⚠️  team: final history mirror failed: {}", e);
                    }
                }
                // Kill the tmux session and clear only runtime state. The SQLite
                // log and workspace transcript survive for a later relaunch.
                let _ = crate::tmux::kill_session(&t.session);
                let _ = t.bus.reset_runtime();
                self.save_meta(); // retain identity; no live session means no recovery
                true
            }
            None => false,
        }
    }

    fn teams(&self) -> serde_json::Value {
        let teams = self.teams.lock().unwrap();
        let list: Vec<serde_json::Value> = teams
            .values()
            .map(|t| {
                let agents = t.bus.roster().map(|r| r.iter().filter(|a| a.status != "offline").count()).unwrap_or(0);
                serde_json::json!({
                    "room": t.session.strip_prefix(SESSION_PREFIX).unwrap_or(&t.session),
                    "workspace": t.workspace,
                    "template": t.template,
                    "session": t.session,
                    "started": t.started,
                    "agents": agents,
                })
            })
            .collect();
        serde_json::json!({ "teams": list })
    }

    fn templates(&self) -> serde_json::Value {
        serde_json::json!({ "templates": team::read_all_templates() })
    }

    fn save_template(&self, name: &str, agents: &serde_json::Value) -> Result<(), String> {
        team::save_template(name, agents)
    }

    fn delete_template(&self, name: &str) -> Result<(), String> {
        team::delete_template(name)
    }

    fn system_prompt(&self) -> String {
        team::read_system_prompt()
    }

    fn save_system_prompt(&self, text: &str) -> Result<(), String> {
        team::save_system_prompt(text)
    }

    fn default_workspace(&self) -> String {
        dirs::home_dir()
            .map(|h| h.to_string_lossy().into_owned())
            .unwrap_or_else(|| std::env::var("HOME").unwrap_or_else(|_| ".".to_string()))
    }

    fn subscribe(&self) -> broadcast::Receiver<String> {
        self.json_tx.subscribe()
    }
}

fn workspace_history_path(workspace: &str, room: &str) -> Option<PathBuf> {
    let workspace = Path::new(workspace.trim());
    if workspace.as_os_str().is_empty() || !workspace.is_dir() {
        return None;
    }
    Some(team::team_runtime_dir(
        workspace.to_string_lossy().as_ref(),
        room,
    ).join("team-history.jsonl"))
}

fn write_history_snapshot(bus: &Bus, path: &Path) -> Result<i64, String> {
    let messages = bus.history(i64::MAX).map_err(|e| e.to_string())?;
    let parent = path
        .parent()
        .ok_or_else(|| format!("history path has no parent: {}", path.display()))?;
    std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let gitignore = parent.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(&gitignore, "*\n").map_err(|e| e.to_string())?;
    }
    let tmp = path.with_extension("jsonl.tmp");
    {
        let file = std::fs::File::create(&tmp).map_err(|e| e.to_string())?;
        let mut writer = BufWriter::new(file);
        for message in &messages {
            serde_json::to_writer(&mut writer, message).map_err(|e| e.to_string())?;
            writer.write_all(b"\n").map_err(|e| e.to_string())?;
        }
        writer.flush().map_err(|e| e.to_string())?;
    }
    #[cfg(windows)]
    if path.exists() {
        std::fs::remove_file(path).map_err(|e| e.to_string())?;
    }
    std::fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    Ok(messages.last().map(|m| m.seq).unwrap_or(0))
}

fn append_history_message(path: &Path, message: &Message) -> Result<(), String> {
    let file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| e.to_string())?;
    let mut writer = BufWriter::new(file);
    serde_json::to_writer(&mut writer, message).map_err(|e| e.to_string())?;
    writer.write_all(b"\n").map_err(|e| e.to_string())?;
    writer.flush().map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::TeamBridge;

    /// Build a manager backed by a temp db, WITHOUT the network daemon (we only
    /// exercise the room registry + bus routing here).
    fn manager() -> Arc<TeamManager> {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir()
            .join(format!("teamtest-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).ok();
        let db = dir.join("team.db").to_string_lossy().into_owned();
        let conn = agora::store::open_shared(&db).unwrap();
        let (json_tx, _) = broadcast::channel::<String>(64);
        let me = Arc::new(TeamManager {
            conn,
            teams: Mutex::new(HashMap::new()),
            json_tx,
            cfg: TeamConfig { url: "http://127.0.0.1:0".into(), model: "m".into(), system_prompt: String::new(), team_rules: String::new(), team_kick: String::new(), codex_profile: String::new() },
            meta_path: dir.join("teams.json"),
            self_ref: OnceLock::new(),
        });
        let _ = me.self_ref.set(Arc::downgrade(&me));
        me
    }

    #[tokio::test]
    async fn unknown_room_is_refused_until_registered() {
        let m = manager();
        // No room yet → bus_for/post refuse.
        assert!(m.room_bus("alpha").is_none());
        assert!(m.post("alpha", "human", "hi", false).is_err());
        // Registering the room opens it.
        m.ensure_room("alpha", "/tmp/alpha", "default").unwrap();
        assert!(m.room_bus("alpha").is_some());
        assert!(m.post("alpha", "human", "hi", false).is_ok());
    }

    #[tokio::test]
    async fn rooms_are_isolated() {
        let m = manager();
        m.ensure_room("alpha", "/tmp/shared", "default").unwrap();
        m.ensure_room("beta", "/tmp/shared", "triad").unwrap();
        m.post("alpha", "human", "hello alpha", false).unwrap();
        m.post("beta", "human", "hello beta", false).unwrap();
        m.room_bus("alpha")
            .unwrap()
            .seed_employee("worker", &serde_json::json!({ "role": "alpha" }))
            .unwrap();
        m.room_bus("beta")
            .unwrap()
            .seed_employee("lead", &serde_json::json!({ "role": "beta" }))
            .unwrap();

        let a = m.history("alpha", 100);
        let b = m.history("beta", 100);
        let a_bodies: Vec<String> = a["messages"].as_array().unwrap().iter()
            .map(|x| x["body"].as_str().unwrap_or("").to_string()).collect();
        let b_bodies: Vec<String> = b["messages"].as_array().unwrap().iter()
            .map(|x| x["body"].as_str().unwrap_or("").to_string()).collect();
        assert!(a_bodies.iter().any(|s| s == "hello alpha"));
        assert!(!a_bodies.iter().any(|s| s == "hello beta"), "beta leaked into alpha");
        assert!(b_bodies.iter().any(|s| s == "hello beta"));
        assert!(!b_bodies.iter().any(|s| s == "hello alpha"), "alpha leaked into beta");
        assert_eq!(m.employee_specs("alpha")[0].0, "worker");
        assert_eq!(m.employee_specs("beta")[0].0, "lead");
    }

    #[tokio::test]
    async fn same_workspace_history_mirrors_are_disjoint() {
        let m = manager();
        let workspace = m.meta_path.parent().unwrap().join("shared-workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace = workspace.to_string_lossy().into_owned();
        let alpha = team::team_slug(&workspace, "default");
        let beta = team::team_slug(&workspace, "triad");
        let alpha_path =
            team::team_runtime_dir(&workspace, &alpha).join("team-history.jsonl");
        let beta_path =
            team::team_runtime_dir(&workspace, &beta).join("team-history.jsonl");

        m.ensure_room(&alpha, &workspace, "default").unwrap();
        m.ensure_room(&beta, &workspace, "triad").unwrap();
        m.post(&alpha, "human", "alpha only", false).unwrap();
        m.post(&beta, "human", "beta only", false).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        assert_ne!(alpha_path, beta_path);
        let alpha_text = std::fs::read_to_string(alpha_path).unwrap();
        let beta_text = std::fs::read_to_string(beta_path).unwrap();
        assert!(alpha_text.contains("alpha only"));
        assert!(!alpha_text.contains("beta only"));
        assert!(beta_text.contains("beta only"));
        assert!(!beta_text.contains("alpha only"));
    }

    #[tokio::test]
    async fn start_reuses_an_active_legacy_workspace_template_pair() {
        let m = manager();
        let workspace = m.meta_path.parent().unwrap().join("legacy-workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace = workspace.to_string_lossy().into_owned();
        let legacy_room = team::workspace_slug(&workspace);
        m.ensure_room(&legacy_room, &workspace, "default").unwrap();
        m.teams.lock().unwrap().get_mut(&legacy_room).unwrap().started = true;

        let result = m.start_team(&workspace, "default");

        assert_eq!(result["room"], legacy_room);
        assert_eq!(result["started"], false);
        assert_eq!(m.teams.lock().unwrap().len(), 1);
    }

    #[tokio::test]
    async fn close_retains_each_workspace_template_room_identity() {
        let m = manager();
        let workspace = m.meta_path.parent().unwrap().join("identity-workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace = workspace.to_string_lossy().into_owned();
        let legacy = team::workspace_slug(&workspace);
        let triad = team::team_slug(&workspace, "triad");

        m.ensure_room(&legacy, &workspace, "default").unwrap();
        m.ensure_room(&triad, &workspace, "triad").unwrap();
        assert!(m.close_team(&legacy));
        assert!(m.close_team(&triad));

        assert_eq!(
            m.known_room_for(&workspace, "default").as_deref(),
            Some(legacy.as_str())
        );
        assert_eq!(
            m.known_room_for(&workspace, "triad").as_deref(),
            Some(triad.as_str())
        );
        assert!(m.teams.lock().unwrap().is_empty(), "closed Teams stay out of UI");
    }

    #[tokio::test]
    async fn reset_runtime_forgets_roster_but_keeps_log() {
        let m = manager();
        m.ensure_room("alpha", "/tmp/alpha", "default").unwrap();
        let bus = m.room_bus("alpha").unwrap();
        bus.seed_employee("manager", &serde_json::json!({ "role": "manager" }))
            .unwrap();
        m.post("alpha", "human", "hello", false).unwrap();
        assert!(!m.employees("alpha")["employees"]
            .as_array()
            .unwrap()
            .is_empty());
        assert!(!m.history("alpha", 100)["messages"]
            .as_array()
            .unwrap()
            .is_empty());

        bus.reset_runtime().unwrap();
        assert!(
            m.employees("alpha")["employees"]
                .as_array()
                .unwrap()
                .is_empty(),
            "employees cleared"
        );
        assert_eq!(
            m.history("alpha", 100)["messages"][0]["body"],
            "hello",
            "log retained"
        );
    }

    #[tokio::test]
    async fn workspace_history_mirrors_live_messages_and_survives_close() {
        let m = manager();
        let workspace = m.meta_path.parent().unwrap().join("workspace");
        std::fs::create_dir_all(&workspace).unwrap();
        let workspace = workspace.to_string_lossy().into_owned();
        let history_path = team::team_runtime_dir(&workspace, "alpha").join("team-history.jsonl");
        let gitignore_path = team::team_runtime_dir(&workspace, "alpha").join(".gitignore");

        m.ensure_room("alpha", &workspace, "default").unwrap();
        m.post("alpha", "human", "first decision", false).unwrap();
        m.post("alpha", "worker", "implemented it", false).unwrap();
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        let read_messages = || {
            std::fs::read_to_string(&history_path)
                .unwrap()
                .lines()
                .map(|line| serde_json::from_str::<Message>(line).unwrap())
                .collect::<Vec<_>>()
        };
        assert_eq!(
            read_messages()
                .iter()
                .map(|m| m.body.as_str())
                .collect::<Vec<_>>(),
            ["first decision", "implemented it"]
        );
        assert_eq!(std::fs::read_to_string(gitignore_path).unwrap(), "*\n");

        assert!(m.close_team("alpha"));
        m.ensure_room("alpha", &workspace, "default").unwrap();
        assert_eq!(
            m.history("alpha", 100)["messages"]
                .as_array()
                .unwrap()
                .len(),
            2
        );
        assert_eq!(
            read_messages().len(),
            2,
            "snapshot rebuild must not duplicate messages"
        );
    }

    #[tokio::test]
    async fn teams_lists_registered_rooms() {
        let m = manager();
        m.ensure_room("alpha", "/tmp/alpha", "default").unwrap();
        let teams = m.teams();
        let arr = teams["teams"].as_array().unwrap();
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["room"], "alpha");
        assert_eq!(arr[0]["workspace"], "/tmp/alpha");
    }
}
