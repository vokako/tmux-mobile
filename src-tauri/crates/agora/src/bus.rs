//! The bus: coordination logic over the store.
//!
//! The agent's whole mental model is two actions: `post(to, body)` and `wait`.
//! Discipline is enforced by an obligation graph maintained here. When A posts to a
//! recipient B:
//!
//! - if A already owes B a response, the post discharges it (A is replying);
//! - otherwise, B now owes A a response (A is asking).
//!
//! Broadcasts (no recipient) create no obligations. Only registered agents can owe
//! (the human operator is never obligated). `wait` is refused while you owe anyone.
//! This removes the need for message kinds or reply-ids, and cannot ping-pong.

use crate::envelope::{Kind, Message, ALL_TOKENS};
use crate::store::{self, AgentRow};
use anyhow::Result;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

/// Liveness ladder. `last_seen` is refreshed by every bus call (`post`, the
/// `wait` loop touches it ~every second while parked) AND by the agent's tool
/// hooks (`Bus::heartbeat` while it is heads-down working). The display status
/// then ages along: fresh → its real status (idle/thinking/working); silent past
/// `STALE_TTL_MS` → `hardworking` (deep, head-down work, no recent heartbeat);
/// silent past `STALLED_TTL_MS` → `stalled` (needs a restart — the supervisor
/// self-heals it). `STALE_TTL_MS` sits above a typical inter-tool/thinking gap so
/// a heartbeating agent never flaps between working and hardworking.
const STALE_TTL_MS: i64 = 90_000;
const STALLED_TTL_MS: i64 = 1_800_000; // 30 min: → "stalled" (mirrors the supervisor self-heal)
const MAX_WAIT_MS: u64 = 50_000;
const POLL_INTERVAL_MS: u64 = 1_000;
/// Push throttling: messages that aren't addressed to you and aren't from the
/// human (i.e. other agents' chatter) are HELD rather than waking you. Hold them
/// for at most this long before flushing anyway, so an agent is never starved of
/// room context even when it is never @-mentioned.
const MAX_PUSH_WINDOW_MS: i64 = 300_000;

#[derive(Clone)]
pub struct Bus {
    conn: Arc<Mutex<rusqlite::Connection>>,
    tx: broadcast::Sender<Message>,
    room: String,
}

/// Resolves a room name to its [`Bus`]. The MCP + web layers route each request
/// to a room (agents via an `x-room` header, the human API via `?room=`), so a
/// single daemon can serve many isolated chat rooms. A single-room deployment
/// uses [`SingleRoom`]; the tmux-mobile host supplies a multi-room registry.
pub trait BusProvider: Send + Sync {
    /// The bus for `room`, creating/opening it if needed. `None` only if the
    /// provider refuses the room (e.g. unknown room in a fixed deployment).
    fn bus_for(&self, room: &str) -> Option<Bus>;
    /// Default room when a request omits one (header/query absent).
    fn default_room(&self) -> String {
        "main".to_string()
    }
}

/// Trivial single-room provider: every request maps to one fixed [`Bus`].
#[derive(Clone)]
pub struct SingleRoom(pub Bus);

impl BusProvider for SingleRoom {
    fn bus_for(&self, _room: &str) -> Option<Bus> {
        Some(self.0.clone())
    }
    fn default_room(&self) -> String {
        self.0.room().to_string()
    }
}

/// An outstanding obligation: `debtor` owes `creditor` a response.
#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct Obligation {
    pub debtor: String,
    pub creditor: String,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(tag = "outcome", rename_all = "snake_case")]
pub enum WaitOutcome {
    /// Refused: reply to these agents before you may wait. `pending` re-surfaces the
    /// actual unanswered @-messages so you know exactly what to respond to.
    Blocked { you_owe: Vec<String>, pending: Vec<Message> },
    /// New messages delivered; roster included so you needn't list agents separately.
    Delivered { messages: Vec<Message>, roster: Vec<AgentRow>, cursor: i64 },
    /// Nothing arrived within the timeout; you remain waiting.
    Idle { roster: Vec<AgentRow>, cursor: i64 },
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum Quiescence {
    Empty,
    Active,
    Done,
    Deadlock { open: Vec<Obligation> },
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct JoinResult {
    pub cursor: i64,
    pub fresh: bool,
    pub roster: Vec<AgentRow>,
}

impl Bus {
    pub fn new(conn: rusqlite::Connection, room: impl Into<String>) -> Self {
        let (tx, _rx) = broadcast::channel(1024);
        Bus { conn: Arc::new(Mutex::new(conn)), tx, room: room.into() }
    }

    /// Build a room-scoped bus over a connection SHARED with other rooms. All
    /// rooms then funnel through one `Mutex<Connection>` — under SQLite WAL a
    /// single writer avoids the inter-connection write contention you'd get
    /// from one connection per room. Each room still has its own broadcast
    /// channel (per-room live wakeups); isolation stays at the `room` column.
    pub fn with_shared(conn: Arc<Mutex<rusqlite::Connection>>, room: impl Into<String>) -> Self {
        let (tx, _rx) = broadcast::channel(1024);
        Bus { conn, tx, room: room.into() }
    }

    pub fn room(&self) -> &str {
        &self.room
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Message> {
        self.tx.subscribe()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, rusqlite::Connection> {
        self.conn.lock().expect("bus mutex poisoned")
    }

    /// Register an agent and announce its arrival.
    pub fn join(&self, name: &str, role: Option<&str>) -> Result<JoinResult> {
        let (cursor, fresh, join_msg, roster) = {
            let conn = self.lock();
            let (cursor, fresh) = store::upsert_agent(&conn, &self.room, name, role)?;
            // If this agent was hired at runtime, mark the employee record active.
            if let Some(e) = store::get_employee(&conn, &self.room, name)? {
                if e.state == "requested" {
                    store::set_employee_state(&conn, &self.room, name, "active")?;
                }
            }
            let join_msg = if fresh {
                let body = match role {
                    Some(r) => format!("{name} joined as {r}."),
                    None => format!("{name} joined."),
                };
                Some(store::append(&conn, &self.room, name, &[], Kind::Join, &body)?)
            } else {
                None
            };
            let roster = store::roster(&conn, &self.room)?;
            (cursor, fresh, join_msg, roster)
        };
        if let Some(m) = join_msg {
            let _ = self.tx.send(m);
        }
        Ok(JoinResult { cursor, fresh, roster: apply_presence(roster) })
    }

    /// Resolve a `to` list into concrete recipient names (expanding all/* to the
    /// roster), excluding the sender itself.
    fn resolve_recipients(&self, conn: &rusqlite::Connection, from: &str, to: &[String]) -> Result<Vec<String>> {
        let all = to.iter().any(|t| ALL_TOKENS.contains(&t.to_ascii_lowercase().as_str()));
        let names: Vec<String> = if all {
            store::roster(conn, &self.room)?.into_iter().map(|a| a.name).collect()
        } else {
            to.to_vec()
        };
        let mut out: Vec<String> = Vec::new();
        for n in names {
            if !n.eq_ignore_ascii_case(from) && !out.iter().any(|x| x.eq_ignore_ascii_case(&n)) {
                out.push(n);
            }
        }
        Ok(out)
    }

    /// Post a message. Recipients come from `@name` mentions in the body. If
    /// `requires_reply` is set, each mentioned (registered) agent owes the sender a
    /// reply — and `wait` will refuse to let them idle until they answer. A message that
    /// mentions someone the sender owes always discharges that debt (it's a reply).
    pub fn post(&self, from: &str, body: &str, requires_reply: bool) -> Result<Message> {
        let msg = {
            let conn = self.lock();
            let mentions = mentioned_names(&conn, &self.room, body)?; // @name in body (+ "all")
            let recipients = self.resolve_recipients(&conn, from, &mentions)?;
            let msg = store::append(&conn, &self.room, from, &mentions, Kind::Msg, body)?;

            // Discharge debts FIRST, against ALL raw @-mentions — not just
            // registered ones. The creditor may be the human operator, who is
            // never in the roster (`mentioned_names` would drop "@human"). If
            // we only cleared debts to registered recipients, an agent could
            // never answer the human: its `wait` stays Blocked forever and it
            // re-replies on every tick (the real-world "@human 在线" spam loop).
            // You can always discharge a reply you owe by addressing that name.
            for creditor in store::owes(&conn, &self.room, from)? {
                if raw_mentions_creditor(body, &creditor) {
                    store::clear_obligation(&conn, &self.room, from, &creditor)?;
                }
            }

            // Then create new obligations: a registered agent you @mention with
            // requires_reply now owes you a reply.
            for b in &recipients {
                if requires_reply
                    && !b.eq_ignore_ascii_case(from)
                    && store::get_agent(&conn, &self.room, b)?.is_some()
                {
                    store::add_obligation(&conn, &self.room, b, from, &msg.id)?;
                }
            }
            // Posting is a bus/coordination action, NOT heads-down work — keep
            // the sender's activity status as-is (so a `thinking` agent that
            // sends a message stays `thinking`, and a `working` one stays
            // `working`); only real work tools promote to `working`. We still
            // refresh liveness so the sender never looks stale.
            store::touch(&conn, &self.room, from)?;
            msg
        };
        let _ = self.tx.send(msg.clone());
        Ok(msg)
    }

    /// Block until new messages arrive, or until timeout. Once caught up, refuses to
    /// idle while you still owe someone a reply.
    pub async fn wait(&self, agent: &str, timeout: Option<Duration>) -> Result<WaitOutcome> {
        let mut rx = self.tx.subscribe();
        let budget = timeout.unwrap_or(Duration::from_millis(MAX_WAIT_MS)).min(Duration::from_millis(MAX_WAIT_MS));
        let deadline = Instant::now() + budget;

        loop {
            // 1. Decide what to deliver. We DELIVER (and advance the cursor) only
            //    when the new batch holds something this agent should react to: a
            //    message addressed to it (@name / @all), or ANY message from the
            //    human. Other agents' un-addressed chatter is HELD — the cursor
            //    stays put, the agent isn't woken — to cut needless wakeups. Held
            //    content is flushed the moment a trigger arrives (so the agent gets
            //    full context in ONE batch), or once the oldest held message ages
            //    past MAX_PUSH_WINDOW_MS, so nothing is starved forever.
            let (delivered, roster, report_cursor) = {
                let conn = self.lock();
                store::touch(&conn, &self.room, agent).ok();
                let cursor = store::get_agent(&conn, &self.room, agent)?.map(|a| a.cursor).unwrap_or(0);
                let batch = store::messages_after(&conn, &self.room, cursor, 500)?;
                let max_seq = batch.last().map(|m| m.seq).unwrap_or(cursor);
                let roster = apply_presence(store::roster(&conn, &self.room)?);
                let is_agent = |name: &str| roster.iter().any(|a| a.name.eq_ignore_ascii_case(name));
                let foreign: Vec<Message> =
                    batch.into_iter().filter(|m| !m.from.eq_ignore_ascii_case(agent)).collect();
                // Worth waking for: addressed to me (@me/@all), or a human message.
                let triggered = foreign.iter().any(|m| {
                    m.addresses(agent) || (matches!(m.kind, Kind::Msg) && !is_agent(&m.from))
                });
                // Safety valve: don't hold the oldest pending message forever.
                let aged_out = foreign
                    .first()
                    .map(|m| crate::envelope::now_ms() - m.ts >= MAX_PUSH_WINDOW_MS)
                    .unwrap_or(false);
                if !foreign.is_empty() && (triggered || aged_out) {
                    if max_seq > cursor {
                        store::set_cursor(&conn, &self.room, agent, max_seq)?;
                    }
                    (foreign, roster, max_seq)
                } else {
                    // Hold: nothing for me yet. Keep the cursor so the held messages
                    // flush on a later trigger/window; report the un-advanced cursor.
                    (Vec::new(), roster, cursor)
                }
            };
            if !delivered.is_empty() {
                let conn = self.lock();
                // Just received message(s) — about to process them, quick reply
                // expected. The first tool/heartbeat promotes this to `working`.
                store::set_status(&conn, &self.room, agent, "thinking")?;
                return Ok(WaitOutcome::Delivered { messages: delivered, roster, cursor: report_cursor });
            }

            // 2. Caught up: refuse to go idle while you owe someone a reply, and
            //    re-surface the exact messages you must answer to emphasize them.
            {
                let conn = self.lock();
                let owed = store::owed_with_msg(&conn, &self.room, agent)?;
                if !owed.is_empty() {
                    let you_owe: Vec<String> = owed.iter().map(|(c, _)| c.clone()).collect();
                    let mut pending = Vec::new();
                    for (_creditor, msg_id) in &owed {
                        if let Some(m) = store::message_by_id(&conn, &self.room, msg_id)? {
                            pending.push(m);
                        }
                    }
                    return Ok(WaitOutcome::Blocked { you_owe, pending });
                }
                store::set_status(&conn, &self.room, agent, "idle")?;
            }

            // 3. Nothing to do and nothing owed: park until a new message or the timeout.
            let now = Instant::now();
            if now >= deadline {
                return Ok(WaitOutcome::Idle { roster, cursor: report_cursor });
            }
            let nap = (deadline - now).min(Duration::from_millis(POLL_INTERVAL_MS));
            tokio::select! {
                r = rx.recv() => { let _ = r; }
                _ = tokio::time::sleep(nap) => {}
            }
        }
    }

    pub fn roster(&self) -> Result<Vec<AgentRow>> {
        let conn = self.lock();
        Ok(apply_presence(store::roster(&conn, &self.room)?))
    }

    /// Out-of-band liveness ping from the agent's tool hooks (each tool / prompt
    /// while it is heads-down on a turn). It both refreshes `last_seen` AND marks
    /// the agent `working` — sustained tool activity is exactly what distinguishes
    /// `working` from the brief `thinking` window right after a message arrives.
    /// No-op (Ok) if the agent has no roster row yet.
    pub fn heartbeat(&self, agent: &str) -> Result<()> {
        let conn = self.lock();
        store::set_status(&conn, &self.room, agent, "working")
    }

    /// Force an agent's stored status. Used by the supervisor's idle-sleep:
    /// `"sleeping"` when it Esc-parks a quiet team, `"idle"` when it wakes them.
    /// `apply_presence` treats `"sleeping"` like `"offline"` — it is never aged
    /// into `stalled` — so a deliberately-parked agent keeps the sleep label
    /// until it rejoins (a fresh `wait` sets `idle`/`thinking`). No-op (Ok) if
    /// the agent has no roster row yet.
    pub fn set_status(&self, agent: &str, status: &str) -> Result<()> {
        let conn = self.lock();
        store::set_status(&conn, &self.room, agent, status)
    }

    pub fn history(&self, limit: i64) -> Result<Vec<Message>> {
        let conn = self.lock();
        store::history(&conn, &self.room, limit)
    }

    // ---- runtime team management (manager-only via agent config) ----

    /// Hire a new employee (a skill-specialised worker). Validates the name is free.
    /// On success records a `requested` employee and announces it; a supervisor then
    /// launches the actual agent process, which joins and flips it to `active`.
    pub fn hire(&self, manager: &str, name: &str, responsibility: &str) -> Result<Message> {
        let name = name.trim();
        if name.is_empty() {
            anyhow::bail!("employee name must not be empty");
        }
        // Opaque launcher spec. No `backend` here — the supervisor launches hires on its
        // configured default backend. role/goal carry the manager's intent.
        let spec = serde_json::json!({
            "role": responsibility, "goal": responsibility, "backstory": "", "manage": false,
        });
        self.seed_employee(name, &spec)?;
        let body = format!("🆕 {manager} hired **{name}** (role: {responsibility}). Coming online…");
        let msg = {
            let conn = self.lock();
            store::append(&conn, &self.room, "system", &[], Kind::System, &body)?
        };
        let _ = self.tx.send(msg.clone());
        Ok(msg)
    }

    /// Register an employee in the desired roster (used both by `hire` and by the
    /// initial-team seeding at startup). Silent: no chat announcement. Validates the
    /// name is free (not an online agent, not an existing non-disabled employee).
    pub fn seed_employee(&self, name: &str, spec: &serde_json::Value) -> Result<()> {
        let name = name.trim();
        let conn = self.lock();
        let online = apply_presence(store::roster(&conn, &self.room)?)
            .into_iter()
            .any(|a| a.name.eq_ignore_ascii_case(name) && a.status != "offline");
        let employed = store::get_employee(&conn, &self.room, name)?
            .map(|e| e.state != "disabled")
            .unwrap_or(false);
        if online || employed {
            anyhow::bail!("the name '{name}' is already taken; choose another");
        }
        store::create_employee(&conn, &self.room, name, spec)?;
        Ok(())
    }

    /// Disable (fire) an employee: mark it disabled, drop its obligations, announce it.
    /// A supervisor then stops the agent's process.
    pub fn fire(&self, manager: &str, name: &str) -> Result<Message> {
        let name = name.trim();
        {
            let conn = self.lock();
            if store::get_employee(&conn, &self.room, name)?.is_none() {
                anyhow::bail!("no employee named '{name}'");
            }
            store::set_employee_state(&conn, &self.room, name, "disabled")?;
            store::clear_agent_obligations(&conn, &self.room, name)?;
            store::set_status(&conn, &self.room, name, "offline")?;
        }
        let body = format!("👋 {manager} disabled employee **{name}**.");
        let msg = {
            let conn = self.lock();
            store::append(&conn, &self.room, "system", &[], Kind::System, &body)?
        };
        let _ = self.tx.send(msg.clone());
        Ok(msg)
    }

    pub fn employees(&self) -> Result<Vec<crate::store::Employee>> {
        let conn = self.lock();
        store::list_employees(&conn, &self.room)
    }

    /// Forget all persisted state for this room (messages, roster, obligations,
    /// employees). For an explicit (re)start or close of a team — NOT recovery,
    /// which adopts a still-running team and its log as-is.
    pub fn reset_room(&self) -> Result<()> {
        let conn = self.lock();
        store::clear_room(&conn, &self.room)
    }

    pub fn quiescence(&self) -> Result<Quiescence> {
        let conn = self.lock();
        let roster = apply_presence(store::roster(&conn, &self.room)?);
        let online: Vec<&AgentRow> = roster.iter().filter(|a| a.status != "offline").collect();
        if online.is_empty() {
            return Ok(Quiescence::Empty);
        }
        if !online.iter().all(|a| a.status == "idle") {
            return Ok(Quiescence::Active);
        }
        let obs = store::all_obligations(&conn, &self.room)?;
        if obs.is_empty() {
            Ok(Quiescence::Done)
        } else {
            Ok(Quiescence::Deadlock {
                open: obs.into_iter().map(|(debtor, creditor)| Obligation { debtor, creditor }).collect(),
            })
        }
    }
}

/// Overlay the liveness ladder onto stored statuses (read-time only — never
/// written back). An agent we have not heard from ages from its real status →
/// `hardworking` (deep work, heartbeats stopped) → `stalled` (needs a restart).
/// `idle`/`online` agents are kept fresh by the wait-loop touch, so if one DOES
/// go stale its loop has died — that's `stalled`, not `hardworking`. An agent
/// that explicitly left/was fired carries the stored `offline`, left untouched.
fn apply_presence(mut roster: Vec<AgentRow>) -> Vec<AgentRow> {
    let now = crate::envelope::now_ms();
    for a in &mut roster {
        if a.status == "offline" {
            continue; // deliberately gone — don't relabel
        }
        if a.status == "sleeping" {
            continue; // deliberately parked by the supervisor — don't age to stalled
        }
        let age = now - a.last_seen;
        if age > STALLED_TTL_MS {
            a.status = "stalled".to_string();
        } else if age > STALE_TTL_MS {
            a.status = if a.status == "working" || a.status == "thinking" {
                "hardworking".to_string()
            } else {
                "stalled".to_string()
            };
        }
    }
    roster
}

/// True if `body` contains an `@creditor` token (or `@all`/`@*`), regardless of
/// whether `creditor` is a registered agent. Used to discharge a debt to ANY
/// addressee — crucially the human operator, who never joins the roster. ASCII
/// `[A-Za-z0-9_-]` tokens only, matching `mentioned_names`'s tokenizer.
fn raw_mentions_creditor(body: &str, creditor: &str) -> bool {
    let bytes = body.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'-')
            {
                j += 1;
            }
            if j > start {
                let tok = &body[start..j];
                if tok.eq_ignore_ascii_case(creditor)
                    || ALL_TOKENS.contains(&tok.to_ascii_lowercase().as_str())
                {
                    return true;
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    false
}

/// Find `@name` mentions in `body` that match a known agent/employee name (or an
/// `all`/`*` token). Used so writing "@manager" in a message addresses manager, the
/// same as passing it in `to`. Only ASCII `[A-Za-z0-9_-]` names are matched.
fn mentioned_names(conn: &rusqlite::Connection, room: &str, body: &str) -> Result<Vec<String>> {
    let mut known: Vec<String> = store::roster(conn, room)?.into_iter().map(|a| a.name).collect();
    for e in store::list_employees(conn, room)? {
        if !known.iter().any(|k| k.eq_ignore_ascii_case(&e.name)) {
            known.push(e.name);
        }
    }
    let bytes = body.as_bytes();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'@' {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len()
                && (bytes[j].is_ascii_alphanumeric() || bytes[j] == b'_' || bytes[j] == b'-')
            {
                j += 1;
            }
            if j > start {
                let tok = &body[start..j];
                let matched = if ALL_TOKENS.contains(&tok.to_ascii_lowercase().as_str()) {
                    Some("all".to_string())
                } else {
                    known.iter().find(|k| k.eq_ignore_ascii_case(tok)).cloned()
                };
                if let Some(name) = matched {
                    if !out.iter().any(|x| x.eq_ignore_ascii_case(&name)) {
                        out.push(name);
                    }
                }
            }
            i = j;
        } else {
            i += 1;
        }
    }
    Ok(out)
}


#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::AgentRow;

    fn row(name: &str, status: &str, age_ms: i64) -> AgentRow {
        AgentRow {
            name: name.to_string(),
            role: None,
            status: status.to_string(),
            cursor: 0,
            joined_at: 0,
            last_seen: crate::envelope::now_ms() - age_ms,
        }
    }

    #[test]
    fn presence_overlay_is_tiered() {
        let out = apply_presence(vec![
            row("worker", "working", 1_000),                  // heartbeating → keep
            row("ponderer", "thinking", 5_000),               // just got a msg → keep
            row("parked", "idle", 5_000),                     // touched recently → keep
            row("grinding", "working", STALE_TTL_MS + 10_000),// silent → hardworking
            row("crashed", "idle", STALE_TTL_MS + 10_000),    // parked but stale = loop died → stalled
            row("wedged", "working", STALLED_TTL_MS + 10_000),// silent 30min+ → stalled
            row("left", "offline", STALLED_TTL_MS + 10_000),  // explicit → stays offline
        ]);
        let by = |n: &str| out.iter().find(|a| a.name == n).unwrap().status.clone();
        assert_eq!(by("worker"), "working");
        assert_eq!(by("ponderer"), "thinking");
        assert_eq!(by("parked"), "idle");
        assert_eq!(by("grinding"), "hardworking", "stale in-turn agent is head-down, not removed");
        assert_eq!(by("crashed"), "stalled", "a parked agent that stops touching has a dead loop");
        assert_eq!(by("wedged"), "stalled");
        assert_eq!(by("left"), "offline", "explicit offline is never relabelled");
    }

    #[test]
    fn sleeping_is_never_aged_into_stalled() {
        // The supervisor parks a quiet team by setting status="sleeping" and
        // Esc-ing the wait call, so the agent stops touching the bus. Even long
        // past STALLED_TTL_MS it must stay "sleeping" (a deliberate state) and
        // never be relabelled — otherwise the UI would show a red "stalled"
        // node for a team that is simply idle and asleep.
        let out = apply_presence(vec![
            row("napper", "sleeping", STALLED_TTL_MS + 60_000),
            row("dozer", "sleeping", 1_000),
        ]);
        let by = |n: &str| out.iter().find(|a| a.name == n).unwrap().status.clone();
        assert_eq!(by("napper"), "sleeping");
        assert_eq!(by("dozer"), "sleeping");
    }
}
