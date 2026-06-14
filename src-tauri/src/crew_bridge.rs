//! Desktop-only glue between the tmux-mobile WS server and the crew bus.
//!
//! Multi-team manager. Each **team** is an isolated chat **room** (= the
//! workspace slug) backed by its own `agora::Bus` on the shared SQLite db. A
//! single MCP daemon serves them all (agents pick a room via the `x-room`
//! header); the phone passes the active room with each `crew_*` RPC. Every
//! room's messages funnel into one re-broadcast channel (each `Message` carries
//! its `room`), and the phone filters to the team currently in view.
//!
//! Compiled ONLY on desktop (lib.rs `#[cfg(...)]`); mobile passes `None`.

use crate::crew::{self, CrewConfig};
use crate::server::CrewBridge;
use agora::bus::{Bus, BusProvider};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::broadcast;

/// One live team: its bus + launch metadata.
struct Team {
    bus: Bus,
    workspace: String,
    session: String,   // tmm-crew-<room>
    started: bool,     // supervisor launched for this room
}

pub struct CrewBus {
    /// Open db path (rooms are opened lazily against it).
    db: String,
    /// room -> Team. Guarded by a std Mutex (never held across .await).
    teams: Mutex<HashMap<String, Team>>,
    /// Merged message fan-out for the WS push path (all rooms).
    json_tx: broadcast::Sender<String>,
    /// Server-level launcher config (bus URL + default model).
    cfg: CrewConfig,
    self_ref: OnceLock<Weak<CrewBus>>,
}

impl CrewBus {
    /// Open the store dir, start the MCP/dashboard daemon (room-aware), and
    /// return the manager. No team is created until the operator starts one.
    pub fn start(db: &str, _room: &str, bind: &str, model: &str) -> Result<Arc<CrewBus>, Box<dyn std::error::Error>> {
        if let Some(parent) = std::path::Path::new(db).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        let (json_tx, _) = broadcast::channel::<String>(1024);
        let port = bind.rsplit(':').next().unwrap_or("8787");
        let cfg = CrewConfig {
            url: format!("http://127.0.0.1:{}", port),
            model: model.to_string(),
        };

        let me = Arc::new(CrewBus {
            db: db.to_string(),
            teams: Mutex::new(HashMap::new()),
            json_tx,
            cfg,
            self_ref: OnceLock::new(),
        });
        let _ = me.self_ref.set(Arc::downgrade(&me));

        // Serve MCP + dashboard for external agents. The provider is the manager
        // itself (room-aware). Bind failure is non-fatal — the phone path still
        // works in-process.
        {
            println!("🜂 crew manager ready (db {}); MCP + dashboard → http://{}/", db, bind);
            let provider: Arc<dyn BusProvider> = me.clone();
            let bind = bind.to_string();
            tokio::spawn(async move {
                if let Err(e) = agora::web::serve(provider, &bind).await {
                    eprintln!("⚠️  crew daemon not listening on {}: {}", bind, e);
                }
            });
        }

        Ok(me)
    }

    /// Get an existing room's bus, or open + register it (lazily) and start its
    /// re-broadcast pump. `workspace` is recorded on first open.
    fn ensure_room(&self, room: &str, workspace: &str) -> Result<Bus, String> {
        {
            let teams = self.teams.lock().unwrap();
            if let Some(t) = teams.get(room) {
                return Ok(t.bus.clone());
            }
        }
        // Open a fresh connection to the shared db, scoped to this room.
        let conn = agora::store::open(&self.db).map_err(|e| e.to_string())?;
        let bus = Bus::new(conn, room.to_string());
        // Pump this room's messages into the merged push channel.
        {
            let mut rx = bus.subscribe();
            let tx = self.json_tx.clone();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(m) => {
                            if let Ok(s) = serde_json::to_string(&m) {
                                let _ = tx.send(s);
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return,
                    }
                }
            });
        }
        let mut teams = self.teams.lock().unwrap();
        // Double-checked: another thread may have inserted while we opened.
        if let Some(t) = teams.get(room) {
            return Ok(t.bus.clone());
        }
        teams.insert(
            room.to_string(),
            Team {
                bus: bus.clone(),
                workspace: workspace.to_string(),
                session: format!("tmm-crew-{}", room),
                started: false,
            },
        );
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
impl BusProvider for CrewBus {
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

impl CrewBridge for CrewBus {
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

    fn employees(&self, room: &str) -> serde_json::Value {
        let employees = self.room_bus(room).and_then(|b| b.employees().ok()).unwrap_or_default();
        serde_json::json!({ "employees": employees })
    }

    fn seed_employee(&self, room: &str, name: &str, spec: &serde_json::Value) -> Result<(), String> {
        let bus = self.room_bus(room).ok_or_else(|| format!("unknown team '{room}'"))?;
        bus.seed_employee(name, spec).map_err(|e| e.to_string())
    }

    fn employee_specs(&self, room: &str) -> Vec<(String, serde_json::Value, String)> {
        self.room_bus(room)
            .and_then(|b| b.employees().ok())
            .unwrap_or_default()
            .into_iter()
            .map(|e| (e.name, e.spec, e.state))
            .collect()
    }

    fn start_team(&self, workspace: &str) -> serde_json::Value {
        let ws = if workspace.trim().is_empty() { self.default_workspace() } else { workspace.trim().to_string() };
        let room = crew::workspace_slug(&ws);

        // Open/register the room, then mark it started (one-shot per room).
        if let Err(e) = self.ensure_room(&room, &ws) {
            return serde_json::json!({ "started": false, "room": room, "workspace": ws, "error": e });
        }
        let already = {
            let mut teams = self.teams.lock().unwrap();
            match teams.get_mut(&room) {
                Some(t) => { let was = t.started; t.started = true; was }
                None => false,
            }
        };
        if already {
            return serde_json::json!({ "started": false, "room": room, "workspace": ws });
        }
        match self.self_ref.get().and_then(|w| w.upgrade()) {
            Some(arc) => {
                let bridge: Arc<dyn CrewBridge> = arc;
                crew::start(bridge, self.cfg.clone(), room.clone(), ws.clone());
                serde_json::json!({ "started": true, "room": room, "workspace": ws })
            }
            None => {
                if let Some(t) = self.teams.lock().unwrap().get_mut(&room) { t.started = false; }
                serde_json::json!({ "started": false, "room": room, "workspace": ws, "error": "manager gone" })
            }
        }
    }

    fn close_team(&self, room: &str) -> bool {
        let session = {
            let mut teams = self.teams.lock().unwrap();
            teams.remove(room).map(|t| t.session)
        };
        match session {
            Some(s) => {
                // Kill the tmux session (best-effort). The chat log stays in the
                // db, so re-starting the same workspace resumes its history.
                let _ = crate::tmux::kill_session(&s);
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
                    "room": t.session.strip_prefix("tmm-crew-").unwrap_or(&t.session),
                    "workspace": t.workspace,
                    "session": t.session,
                    "started": t.started,
                    "agents": agents,
                })
            })
            .collect();
        serde_json::json!({ "teams": list })
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
