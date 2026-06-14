//! Desktop-only glue between the tmux-mobile WS server and the agora bus.
//!
//! The agora `Bus` (group-chat coordination over SQLite + a broadcast channel)
//! runs in-process. This module:
//!   1. opens the store + builds the `Bus`,
//!   2. spawns agora's own axum router (MCP `/mcp` + dashboard) on a local port
//!      so external coding agents (kiro/claude/codex) can join the same room,
//!   3. adapts the `Bus` to the server's JSON-only [`AgoraBridge`] trait so the
//!      phone reaches the same room through the existing WS connection.
//!
//! This file is compiled ONLY on desktop (see lib.rs `#[cfg(...)]` gating); the
//! mobile build never references agora and passes `None` to the server.

use crate::server::AgoraBridge;
use crate::team::TeamConfig;
use agora::bus::Bus;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock, Weak};
use tokio::sync::broadcast;

/// Wraps the agora `Bus` and re-broadcasts its `Message` stream as JSON strings
/// (the trait boundary is JSON-only so no agora type leaks into server.rs).
pub struct AgoraBus {
    bus: Bus,
    /// Pre-serialized message fan-out for the WS push path. We bridge agora's
    /// `broadcast::Receiver<Message>` into a `broadcast::Sender<String>` once,
    /// here, rather than serializing per-connection.
    json_tx: broadcast::Sender<String>,
    /// Team supervisor config + one-shot start guard. The team is launched on
    /// demand (the phone's "start team" action), never automatically — spinning
    /// up real LLM agents is costly and must be the operator's choice.
    team_cfg: TeamConfig,
    team_started: AtomicBool,
    /// Weak self-handle so `start_team(&self)` can hand the supervisor an
    /// `Arc<dyn AgoraBridge>` to call back into. Set once, right after
    /// construction (see `start`).
    self_ref: OnceLock<Weak<AgoraBus>>,
}

impl AgoraBus {
    /// Open the store, build the bus, start the MCP/dashboard daemon, and start
    /// the Message→JSON re-broadcast pump. Returns the concrete bridge so the
    /// caller can both hand it to the WS server and start the team on it.
    pub fn start(db: &str, room: &str, bind: &str, model: &str) -> Result<Arc<AgoraBus>, Box<dyn std::error::Error>> {
        if let Some(parent) = std::path::Path::new(db).parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).ok();
            }
        }
        let conn = agora::store::open(db)?;
        let bus = Bus::new(conn, room.to_string());

        // Re-broadcast every bus message as a JSON string for the WS push path.
        let (json_tx, _) = broadcast::channel::<String>(1024);
        {
            let mut rx = bus.subscribe();
            let tx = json_tx.clone();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(m) => {
                            if let Ok(s) = serde_json::to_string(&m) {
                                let _ = tx.send(s); // ignore: no live receivers is fine
                            }
                        }
                        Err(broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(broadcast::error::RecvError::Closed) => return,
                    }
                }
            });
        }

        // Serve MCP + dashboard for external agents. Bind failure (e.g. port in
        // use) is non-fatal — the in-process bus still works for the phone; we
        // just log it so the operator can pick another AGORA_BIND.
        {
            println!("🜂 agora bus ready (room '{}', db {}); MCP + dashboard → http://{}/", room, db, bind);
            let bus = bus.clone();
            let bind = bind.to_string();
            tokio::spawn(async move {
                if let Err(e) = agora::web::serve(bus, &bind).await {
                    eprintln!("⚠️  agora daemon not listening on {}: {}", bind, e);
                }
            });
        }

        // Agents connect to the bus locally over MCP. If the bind is on
        // 0.0.0.0 (LAN dashboard), agents still dial 127.0.0.1 on that port.
        let port = bind.rsplit(':').next().unwrap_or("8787");
        let team_cfg = TeamConfig {
            session: "agora".to_string(),
            url: format!("http://127.0.0.1:{}", port),
            model: model.to_string(),
        };

        let me = Arc::new(AgoraBus {
            bus,
            json_tx,
            team_cfg,
            team_started: AtomicBool::new(false),
            self_ref: OnceLock::new(),
        });
        let _ = me.self_ref.set(Arc::downgrade(&me));
        Ok(me)
    }
}

impl AgoraBridge for AgoraBus {
    fn history(&self, limit: i64) -> serde_json::Value {
        let msgs = self.bus.history(limit).unwrap_or_default();
        serde_json::json!({ "messages": msgs })
    }

    fn roster(&self) -> serde_json::Value {
        let roster = self.bus.roster().unwrap_or_default();
        serde_json::json!({ "roster": roster })
    }

    fn post(&self, from: &str, body: &str, requires_reply: bool) -> Result<serde_json::Value, String> {
        self.bus
            .post(from, body, requires_reply)
            .map(|m| serde_json::to_value(m).unwrap_or(serde_json::Value::Null))
            .map_err(|e| e.to_string())
    }

    fn employees(&self) -> serde_json::Value {
        let employees = self.bus.employees().unwrap_or_default();
        serde_json::json!({ "employees": employees })
    }

    fn seed_employee(&self, name: &str, spec: &serde_json::Value) -> Result<(), String> {
        self.bus.seed_employee(name, spec).map_err(|e| e.to_string())
    }

    fn employee_specs(&self) -> Vec<(String, serde_json::Value, String)> {
        self.bus
            .employees()
            .unwrap_or_default()
            .into_iter()
            .map(|e| (e.name, e.spec, e.state))
            .collect()
    }

    fn start_team(&self) -> bool {
        // One-shot: only the first caller actually starts the supervisor.
        if self.team_started.swap(true, Ordering::SeqCst) {
            return false;
        }
        // Upgrade the weak self-handle into the Arc<dyn AgoraBridge> the
        // supervisor calls back through.
        match self.self_ref.get().and_then(|w| w.upgrade()) {
            Some(arc) => {
                let bridge: Arc<dyn AgoraBridge> = arc;
                crate::team::start(bridge, self.team_cfg.clone());
                true
            }
            None => {
                // Shouldn't happen (self_ref is set at construction); undo the
                // guard so a later retry can succeed.
                self.team_started.store(false, Ordering::SeqCst);
                false
            }
        }
    }

    fn team_started(&self) -> bool {
        self.team_started.load(Ordering::SeqCst)
    }

    fn subscribe(&self) -> broadcast::Receiver<String> {
        self.json_tx.subscribe()
    }
}
