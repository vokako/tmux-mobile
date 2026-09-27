//! Agent telemetry: the passive half of the v2 dual channel.
//!
//! What an agent SAYS goes through `hub_post`; what we OBSERVE arrives here:
//! hook notifications (it stopped, it
//! wants permission), the prompts it accepted, and tmux window activity. Status
//! is DERIVED from those facts at read time — an agent never fills in a form,
//! and a backend with poor hook coverage (codex) degrades to pane-activity
//! granularity instead of lying. A finished turn (Stop) is REST, not distress:
//! "stuck" detection by stop-without-done was a Team-supervisor-era rule and
//! mislabeled every long-idle direct agent — the only distress we can honestly
//! observe is a failed stop. See docs/exec-plans/agents-v2.md §4.1/§4.3.
//!
//! `userPromptSubmit` carries the INPUT half, which nothing else does: a prompt
//! typed at the keyboard exists nowhere in the room, and a line we typed into a
//! pane is only *sent*, never *confirmed*. Recording both closes the loop —
//! `record_delivery` remembers what we typed, the echo acknowledges it, and
//! `sweep_deliveries` reports the ones that never arrived.
//!
//! The store is a process-global map keyed by (session, window index) — the
//! same granularity as a project slot and a hook notification. Records are
//! small and bounded by the number of live windows; entries for windows that
//! no longer exist are dropped opportunistically on write. Observations are
//! process-local on purpose (the next hook re-establishes them), with ONE
//! exception: the queue of lines we typed into a pane lives ONLY in state.db,
//! because the agent that will echo them is a separate process that outlives our
//! restarts — see the delivery block below (board #249).

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Tool activity within this window means "working".
const ACTIVE_SECS: u64 = 30;
/// How long a line we typed into a pane may wait for its `userPromptSubmit`
/// echo before we call the delivery unconfirmed. Typing is `send-keys`, which
/// succeeds as long as the pane exists — it says nothing about whether the CLI
/// accepted the text as a prompt (a shell would have executed it, a crashed pane
/// swallows it). The hook echo is the only end-to-end proof, so an unacked line
/// is reported rather than assumed. The clock does NOT run while a turn is open:
/// a busy agent QUEUES what we typed by design, so it is measured from the turn's
/// end (see `overdue_rows`).
const DELIVERY_ACK_SECS: u64 = 45;
/// Prompt text kept per event. Long enough for a real instruction, short
/// enough that 120 of them stay a cheap in-memory ring.
const MAX_PROMPT_CHARS: usize = 1024;
/// How long an outstanding line stays worth recovering across a restart. Beyond
/// this the agent that would have echoed it is long gone, so resurrecting the
/// record would only produce a stale warning.
const PENDING_MAX_AGE_SECS: u64 = 24 * 3600;
/// How close two identical tool events have to be to count as the same call.
/// `preToolUse` and `postToolUse` arrive milliseconds apart; an agent genuinely
/// running the same command twice takes longer than this.
const TOOL_DEDUPE_SECS: u64 = 5;

#[derive(Debug, Clone, Default)]
struct Rec {
    /// Turn START: the `userPromptSubmit` hook. The agent accepted a prompt, so
    /// a turn is open from here until an end arrives.
    prompt: Option<u64>,
    /// Turn END: ("completed" | "failed", ts) from the stop / StopFailure hook.
    /// Separate from `ask` because they are different questions — "is a turn
    /// running" vs "is it blocked on me" — and one Option cannot answer both
    /// once a permission prompt overwrites a stop.
    end: Option<(String, u64)>,
    /// The agent is blocked on the human: permission_required | input_required.
    ask: Option<(String, u64)>,
    /// Last hook tool event: (activity line, ts). Work observed inside a turn.
    tool: Option<(String, u64)>,
    /// When each fact above ARRIVED, in the order of the durable activity log
    /// (board #249; see `arrival`). Hook timestamps are whole seconds, so a
    /// stop and the next prompt often share one; the old `end >= start` tie
    /// read that turn as idle, and the sweep reported its queued lines
    /// mid-turn. The order breaks only that tie; seconds stay the ordering
    /// authority. 0 = never set.
    prompt_seq: u64,
    end_seq: u64,
    ask_seq: u64,
    tool_seq: u64,
}

static NEXT_ARRIVAL: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);

/// The next arrival number, in the persisted activity log's id order (board
/// #249, validator): the counter starts above the newest activity row id this
/// process finds, so a fact observed now always sorts after every fact an
/// earlier process wrote — a process-local counter restarting at 1 would sort
/// a new stop BEFORE a turn edge recovered from the log. 0 means "no fact".
fn arrival() -> u64 {
    seed_arrival();
    NEXT_ARRIVAL.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// A fact recovered from the durable log keeps its own row id as its order,
/// and the counter moves past it, so anything observed later sorts after it.
fn recovered_arrival(row_id: i64) -> u64 {
    seed_arrival();
    let id = row_id.max(1) as u64;
    NEXT_ARRIVAL.fetch_max(id + 1, std::sync::atomic::Ordering::Relaxed);
    id
}

fn seed_arrival() {
    static SEEDED: OnceLock<()> = OnceLock::new();
    SEEDED.get_or_init(|| {
        let top = queue(|s| s.max_activity_id()).unwrap_or(0).max(0) as u64;
        NEXT_ARRIVAL.fetch_max(top + 1, std::sync::atomic::Ordering::Relaxed);
    });
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentStatus {
    /// Derived state, one of exactly four: `running` (a turn is open — the
    /// agent accepted a prompt and has not stopped), `waiting` (blocked on the
    /// human), `idle` (no turn open) or `failed`.
    pub state: String,
    /// Human line explaining the state (what it asked for or the last observed
    /// tool call).
    pub detail: String,
    /// When the state began — the turn's start for `running`, so a client can
    /// render "running 2m14s" without keeping its own clock.
    pub since: u64,
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// Event timestamps are MILLISECONDS, and they have to be real ones. They were
/// `now() * 1000`, so every event inside the same second carried an identical
/// timestamp while chat messages carried true millis — the client's sort then
/// had nothing to order them by and a turn's tool calls could land after the
/// reply they produced.
fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

/// The activity FEED: recent observed events per session, newest last. This
/// is telemetry made visible in the chat timeline (owner ask: show tool
/// calls / status changes between the final replies) — an in-memory ring,
/// NOT chat history: it never touches the bus db and dies with the server.
const EVENTS_CAP: usize = 120;
/// NOTHING is pruned from the durable log. This is a stated goal, not an
/// oversight: the owner wants the trace COMPLETE so it can be analysed (board #9,
/// 2026-08-29: "我希望整个 trace 是非常完整的，以便我们后续去做一些分析").
///
/// It used to keep 2000 events per session, pruned every 256 inserts. Measured on
/// this host the day it was removed: the busiest session held 4046 rows, so the
/// cap stood to delete the older HALF of it, and the only reason it had not is
/// that the prune counter was per process and every restart postponed it — an
/// accident, not a policy. And a deleted row is indistinguishable from an event
/// that never happened, which is exactly what makes a trace unanalysable. The
/// whole log is ~1.2 MB of text for ten days of heavy use, so there was nothing to
/// buy by trimming it.
///
/// If a retention is ever wanted it belongs in `Config` with a documented key, not
/// in an env var of its own — `Store::prune_activity` is the primitive it would
/// call. What is bounded instead is the READ (see `events_page`): the cost of a
/// long history belongs where it can be limited without destroying anything.
/// Events one page returns when the caller does not say. The tail of the history,
/// which is what a client renders first.
pub const LOAD_EVENTS: usize = 600;
/// The most a single page may return however the caller asks. A hard ceiling, so
/// one RPC can never turn into a multi-megabyte frame: measured on this host, the
/// newest 600 events of the busiest session already serialize to ~277 KB. It is a
/// PAGE cap, not a history horizon — `before_ts`/`before_id` walk as far back as
/// the log goes.
pub const MAX_PAGE_EVENTS: usize = 1000;

#[derive(Debug, Clone, serde::Serialize)]
pub struct ActivityEvent {
    /// The durable log's row id, and the second half of the paging cursor: a busy
    /// turn writes several events inside one millisecond, so `ts` alone cannot
    /// address a position in the log. 0 for an event read from the in-memory ring
    /// (no database), which is also why a client must treat it as opaque.
    #[serde(skip_serializing_if = "is_zero")]
    pub id: i64,
    /// Epoch MILLISECONDS to merge directly with bus message timestamps.
    pub ts: u64,
    pub window: String,
    /// tool | status | notif | prompt | warn
    pub kind: String,
    pub text: String,
    /// `tool` events only: the tool's NAME, kept apart from its argument so the
    /// client can render the scannable half differently. `text` is the argument.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub tool: String,
    /// Provenance, `prompt` events only: `app` when the text is the line this
    /// app typed into the pane (so the event doubles as the delivery receipt),
    /// `local` when it was typed at the keyboard. Empty for every other kind.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub via: String,
    /// `status` events only: the state the agent DECLARED. Kept apart from the
    /// note for the same reason a tool's name is — the client renders the two
    /// halves differently, and a note is the half a human reads.
    #[serde(skip_serializing_if = "str::is_empty")]
    pub state: String,
    /// The delivery rows this event is about (board #249). The ONE
    /// correlation key is the delivery row id: a `prompt` echo lists the rows
    /// it settled, a `warn` the one row it reported. `msg` is the chat message
    /// that row carries, where one exists. A client marks those messages
    /// delivered whether or not it has them loaded, draws no INPUT row for a
    /// settling echo, and drops a warn whose row a later echo settled. Empty
    /// for every other event and for rows before v24.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub deliveries: Vec<DeliveryRef>,
}

/// One delivery row named by an event: its id, and the message it carries.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DeliveryRef {
    pub id: i64,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub msg: String,
}

impl DeliveryRef {
    fn of(row: &super::store::DeliveryRow) -> Self {
        DeliveryRef { id: row.id, msg: row.msg_id.clone() }
    }
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

fn events() -> &'static Mutex<HashMap<String, std::collections::VecDeque<ActivityEvent>>> {
    static EVENTS: OnceLock<Mutex<HashMap<String, std::collections::VecDeque<ActivityEvent>>>> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn push_event(session: &str, window: &str, kind: &str, text: String) {
    push_full(session, window, kind, text, String::new(), String::new(), Vec::new());
}

fn push_full(session: &str, window: &str, kind: &str, text: String, tool: String, via: String, deliveries: Vec<DeliveryRef>) {
    push(
        session,
        ActivityEvent {
            id: 0,
            ts: now_ms(),
            window: window.to_string(),
            kind: kind.into(),
            text,
            tool,
            via,
            state: String::new(),
            deliveries,
        },
    );
}

/// The one place the ring is appended to and trimmed. The event also goes to
/// state.db, because a restart used to erase the whole feed while the messages
/// around it survived: a conversation with holes in it.
fn push(session: &str, ev: ActivityEvent) {
    persist(session, &ev);
    let mut map = events().lock().unwrap();
    let q = map.entry(session.to_string()).or_default();
    q.push_back(ev);
    while q.len() > EVENTS_CAP {
        q.pop_front();
    }
}

/// Unit tests exercise the ring and the derive rules; the durable log is tested
/// directly in `store.rs`. Keeping the two apart is what stops `cargo test` from
/// writing rows for invented sessions into the developer's real state.db.
#[cfg(test)]
fn persist(_session: &str, _ev: &ActivityEvent) {}

/// Write one event to the durable log. FAIL-SOFT, always: telemetry may never
/// block or break the thing it observes, and the ring is still there for this
/// process's lifetime if the database is unavailable (a mobile build, a
/// read-only disk).
#[cfg(not(test))]
fn persist(session: &str, ev: &ActivityEvent) {
    let written = super::with_store(|s| {
        let refs = if ev.deliveries.is_empty() { String::new() } else { serde_json::to_string(&ev.deliveries).unwrap_or_default() };
        s.insert_activity(session, &ev.window, ev.ts, &ev.kind, &ev.text, &ev.tool, &ev.via, &ev.state, &refs)
    });
    if let Err(e) = written {
        // Fail-soft, but not SILENT: a lost write is a hole in the trace, and the
        // whole point of the log is that it is complete. Reported on the first
        // failure and every 100th after that, so a broken database is visible in
        // the server log without becoming the server log.
        static FAILURES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = FAILURES.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
        if n == 1 || n % 100 == 0 {
            eprintln!("⚠️  activity log write failed ({n} so far): {e}");
        }
    }
    // And that is all: no prune. The trace is kept whole (see the note on
    // EVENTS_CAP / LOAD_EVENTS) and the READ is what is bounded.
}

/// One page of the activity feed, oldest first, plus whether older events exist.
///
/// This is the read side of "keep everything": the log is complete, so the COST
/// of a long history has to live here, bounded — `limit` is clamped to
/// `MAX_PAGE_EVENTS`, and a client walks backwards with `before` instead of ever
/// asking for the whole thing (measured: the newest 600 events of the busiest
/// session are ~277 KB of JSON; the full log is ~1.2 MB of text, which is not
/// something to put through a phone's socket on every project switch).
///
/// Three shapes, exactly as `Store::activity_page`: no cursor = the newest page,
/// `since_ts` = the incremental tail, `before` = the page older than that cursor.
/// `since_ts` and `before` are independent, so a client may page backwards while
/// still polling the tail.
///
/// The ring is the fallback for a build with no database at all; it holds only
/// this process's newest events, so it answers a first page and reports no more.
pub fn events_page(
    session: &str,
    since_ts: u64,
    before: Option<(u64, i64)>,
    limit: usize,
) -> (Vec<ActivityEvent>, bool) {
    let limit = limit.clamp(1, MAX_PAGE_EVENTS);
    // The database is off under `cfg(test)` for the same reason the whole ring is:
    // a unit test must not write invented sessions into the developer's state.db.
    let paged = if cfg!(test) {
        Err(String::new())
    } else {
        super::with_store(|s| s.activity_page(session, since_ts, before, limit))
    };
    if let Ok((rows, has_more)) = paged {
        if !rows.is_empty() {
            let events = rows
                .into_iter()
                .map(|r| ActivityEvent {
                    id: r.id,
                    ts: r.ts,
                    window: r.window,
                    kind: r.kind,
                    text: r.text,
                    tool: r.tool,
                    via: r.via,
                    state: r.state,
                    deliveries: serde_json::from_str(&r.deliveries).unwrap_or_default(),
                })
                .collect();
            return (events, has_more);
        }
    }
    let ring: Vec<ActivityEvent> = events()
        .lock()
        .unwrap()
        .get(session)
        .map(|q| {
            q.iter()
                .filter(|e| {
                    e.ts > since_ts && before.map(|(bts, _)| e.ts < bts).unwrap_or(true)
                })
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    let skipped = ring.len().saturating_sub(limit);
    (ring.into_iter().skip(skipped).collect(), false)
}

/// Events newer than `since_ts` (ms, exclusive), oldest first — the newest page,
/// which is what a first load wants: the END of a conversation, not its
/// beginning. Kept as the plain read for callers that do not page.
pub fn recent_events(session: &str, since_ts: u64) -> Vec<ActivityEvent> {
    events_page(session, since_ts, None, LOAD_EVENTS).0
}

/// The prompt newer than this window's last turn end. The durable path lets a
/// stop hook recover its reply edge after the server restarted mid-turn.
pub fn current_turn_prompt(session: &str, window: &str) -> Option<String> {
    if !cfg!(test) {
        return super::with_store(|s| s.current_turn_prompt(session, window))
            .ok()
            .flatten();
    }
    let map = events().lock().unwrap();
    let rows = map.get(session)?;
    for event in rows.iter().rev().filter(|event| event.window == window) {
        if event.kind == "prompt" {
            return Some(event.text.clone());
        }
        if event.kind == "notif" && matches!(event.text.as_str(), "completed" | "failed") {
            return None;
        }
    }
    None
}

/// How much trace this session has: (events, oldest ts, newest ts). For the
/// client's "N of M loaded" and for anyone auditing what we keep.
pub fn events_stats(session: &str) -> (usize, u64, u64) {
    if cfg!(test) {
        return (0, 0, 0);
    }
    super::with_store(|s| s.activity_stats(session)).unwrap_or((0, 0, 0))
}

fn store() -> &'static Mutex<HashMap<(String, String), Rec>> {
    static STORE: OnceLock<Mutex<HashMap<(String, String), Rec>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_rec(session: &str, window: &str, f: impl FnOnce(&mut Rec)) {
    let mut map = store().lock().unwrap();
    f(map.entry((session.to_string(), window.to_string())).or_default());
}

// ── Outstanding deliveries: ONE queue, in state.db (board #249) ─────────
//
// A pending delivery is a PROMISE made to a client ("this line was typed into
// the agent's pane; its receipt is coming"), kept by a SEPARATE process that
// our restart does not touch: the agent holds the line in its own input queue
// and submits it minutes later. The `deliveries` table is the only copy. It
// used to be mirrored in memory with a 16-line cap and folded back after a
// restart through an unordered map; the two copies disagreed — the memory
// evicted lines the table kept, the fold kept a random 16 — and every line
// memory had lost echoed back as keyboard input (`via: local`, an INPUT row,
// a hollow ring for ever; board #5, #249).

/// The delivery table, through the store. Tests point the process at a
/// throwaway database first, so a unit test never writes into the
/// developer's real state.db.
fn queue<T>(f: impl FnOnce(&mut super::store::Store) -> Result<T, String>) -> Result<T, String> {
    #[cfg(test)]
    crate::projects::tests::use_test_store();
    super::with_store(f)
}

/// Per session: the newest delivery row that existed when this process first
/// touched the session's queue, and when that was. Rows at or below it were
/// typed by an earlier process, and nothing was listening for their echoes
/// while it was down, so their ack clock starts at that moment rather than at
/// their typing time — or the first sweep after a restart reports them
/// unconfirmed seconds before the echo arrives. Rows above it were typed by
/// this process and keep their own clock.
fn recovery_marks() -> &'static Mutex<HashMap<String, (i64, u64)>> {
    static MARKS: OnceLock<Mutex<HashMap<String, (i64, u64)>>> = OnceLock::new();
    MARKS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn recovery_mark(session: &str) -> (i64, u64) {
    if let Some(m) = recovery_marks().lock().unwrap().get(session) {
        return *m;
    }
    // Lines older than the recovery horizon are dropped once per process: an
    // echo that never came in a day is not coming.
    static PRUNED: OnceLock<()> = OnceLock::new();
    PRUNED.get_or_init(|| {
        let cutoff = now().saturating_sub(PENDING_MAX_AGE_SECS);
        let _ = queue(|s| s.prune_deliveries(cutoff));
    });
    let top = queue(|s| s.max_delivery_id(session)).unwrap_or(0);
    let mark = *recovery_marks().lock().unwrap().entry(session.to_string()).or_insert((top, now()));
    recover_open_turns(session);
    mark
}

/// A turn that was OPEN when an earlier process last heard of it is still
/// open (board #249, validator): a server restart in the middle of a 20-minute
/// turn left the window with no facts, it derived idle, and 45 s later the
/// sweep reported every line the running CLI was still holding. The durable
/// activity log holds the turn edges, so the open prompt is restored from it
/// (never from pane activity) into a window this process has no facts for.
/// Its stop, if it fired while we were down, is in the hook inbox and closes
/// the turn when it is read. Known gap: an interrupt is not logged, so a turn
/// interrupted just before a restart reopens until that agent's next hook.
fn recover_open_turns(session: &str) {
    for (window, ts_ms, row_id) in queue(|s| s.open_turns(session)).unwrap_or_default() {
        with_rec(session, &window, |r| {
            if r.prompt.is_none() && r.end.is_none() && r.tool.is_none() && r.ask.is_none() {
                r.prompt = Some(ts_ms / 1000);
                r.prompt_seq = recovered_arrival(row_id);
            }
        });
    }
}

fn forget_window_deliveries(session: &str, window: &str) {
    let _ = queue(|s| s.clear_deliveries(session, Some(window)));
}

/// A hook notification consumed by the AgentNotificationHub. Two different
/// facts arrive here and they are stored apart: a stop ENDS the turn, a
/// permission/input prompt means the agent is blocked on the human while the
/// turn stays open.
pub fn record_notification(session: &str, window: &str, kind: &str, ts: u64) {
    push_event(session, window, "notif", kind.to_string());
    let kind = kind.to_string();
    with_rec(session, window, |r| match kind.as_str() {
        "permission_required" | "input_required" => {
            r.ask = Some((kind, ts));
            r.ask_seq = arrival();
        }
        _ => {
            r.end = Some((kind, ts));
            r.end_seq = arrival();
            r.ask = None; // a finished turn cannot still be asking
        }
    });
}

/// A human interrupted the turn (`hub_agent_interrupt` / `tmm agent
/// interrupt`). We are ENDING the turn on the agent's behalf, before the Escape
/// is typed: the hooks report the edges of a turn the agent runs, and a turn
/// cancelled from outside has no edge of its own — the backend fires no stop for
/// a cancelled turn, so the newest fact would stay the `userPromptSubmit` that
/// opened it and the card would read `running` for as long as the agent stayed
/// alive. And the interrupted agent very often starts something else within
/// seconds, which would re-derive `running` from a NEW turn — indistinguishable
/// from the old one never having stopped, so the interrupt looked like it had
/// not landed (owner, 2026-08-29). Resetting first makes the effect visible in
/// the gap, and a real turn that starts afterwards is a fact of its own.
///
/// Same shape as a `completed` stop, with `ask` cleared because a cancelled
/// turn is not still asking.
pub fn record_interrupt(session: &str, window: &str) {
    let ts = now();
    with_rec(session, window, |r| {
        r.end = Some(("completed".to_string(), ts));
        r.end_seq = arrival();
        r.ask = None;
    });
}

/// A hook tool event (isolated-home agents only, Phase B+): `("Edit",
/// "foo.rs")`. The event keeps the two parts apart for rendering; the status
/// record keeps the joined line, which is what "working — Edit foo.rs" shows.
/// A tool call that is the CLI's housekeeping (kiro v3's post-turn `memory`
/// auto-capture, board #227): recorded as a tool ONLY while this window's
/// turn is open — then it is the agent using the tool mid-turn — and dropped
/// otherwise, so it never reopens a finished turn. Returns whether it counted.
pub fn record_housekeeping_tool(session: &str, window: &str, tool: &str, detail: &str) -> bool {
    let open = store()
        .lock()
        .unwrap()
        .get(&(session.to_string(), window.to_string()))
        .is_some_and(|r| matches!(derive_from(r, 0, now()).state.as_str(), "running" | "waiting"));
    if open {
        record_tool(session, window, tool, detail);
    }
    open
}

pub fn record_tool(session: &str, window: &str, tool: &str, detail: &str) {
    // A tool call is proof the model answered: whatever transient-error retry
    // budget this window was burning refills (recovery rule 3).
    super::recovery::note_tool_activity(session, window);
    let line = if detail.is_empty() { tool.to_string() } else { format!("{tool} {detail}") };
    let ts = now();
    // ONE row per call. We subscribe to both `preToolUse` and `postToolUse` (a
    // backend may only send one of them), and they carry the same tool and the
    // same argument — so a lane showed every call twice, milliseconds apart. The
    // pair is collapsed here rather than in the hook config, because an
    // already-spawned agent keeps the config it was started with: fixing it at
    // the source would only help agents spawned later.
    let mut dup = false;
    with_rec(session, window, |r| {
        dup = r
            .tool
            .as_ref()
            .is_some_and(|(prev, at)| prev == &line && ts.saturating_sub(*at) <= TOOL_DEDUPE_SECS);
        r.tool = Some((line.clone(), ts));
        r.tool_seq = arrival();
    });
    if dup {
        return;
    }
    push_full(session, window, "tool", detail.to_string(), tool.to_string(), String::new(), Vec::new());
}

/// A line this app typed into an agent's pane (`deliver_mentions`). Held as a
/// pending delivery until the agent's `userPromptSubmit` hook echoes it back.
/// Two deliveries of the SAME body are two rows (board #122): the pane really
/// was typed into twice, and each echo settles one. FAIL-SOFT like all
/// telemetry: without a database the line is simply untracked, and its echo
/// files as keyboard input.
/// `msg_id` is the chat message the line carries ('' for a line with none:
/// a board notice, a reply, a typed first prompt) — the echo names it, so a
/// client marks that message delivered without holding it (board #249).
pub fn record_delivery(session: &str, window: &str, line: &str, msg_id: &str) {
    recovery_mark(session); // read BEFORE the insert: this row is ours
    // The window's record exists from its first delivery, so `retain_windows`
    // sees it and drops its queue when the window goes.
    with_rec(session, window, |_| {});
    let _ = queue(|s| s.insert_delivery(session, window, line, now(), msg_id));
}

/// The `userPromptSubmit` hook: the agent accepted a prompt. This is BOTH the
/// input half of the transcript (what the agent was asked, which no other
/// channel carries) and the delivery receipt for a line we typed.
///
/// Returns true when it acknowledged a pending delivery. Matching is
/// containment, not equality: the CLI may submit the line with its own
/// decoration, and an agent that is mid-task receives our line appended to
/// whatever it was already typing.
pub fn record_prompt(session: &str, window: &str, prompt: &str) -> bool {
    let text = truncate_chars(prompt, MAX_PROMPT_CHARS);
    let ts = now();
    recovery_mark(session);
    let rows = queue(|s| s.pending_deliveries(session, Some(window))).unwrap_or_default();
    let settled = settled_by(&rows, prompt);
    // A row counts only if THIS echo deleted it: two echoes racing for one
    // row cannot both call it theirs.
    let won: Vec<&super::store::DeliveryRow> = settled
        .into_iter()
        .filter(|row| queue(|s| s.delete_delivery_id(row.id)).unwrap_or(false))
        .collect();
    let acked = !won.is_empty();
    let refs: Vec<DeliveryRef> = won.iter().map(|row| DeliveryRef::of(row)).collect();
    // A turn just opened. This is the ONE honest "it started working" signal:
    // pane activity cannot be it, because an agent TUI repaints its prompt
    // (spinner, status line, cursor) long after it finished.
    with_rec(session, window, |r| {
        r.prompt = Some(ts);
        r.prompt_seq = arrival();
    });
    push_full(
        session,
        window,
        "prompt",
        text,
        String::new(),
        if acked { "app".into() } else { "local".into() },
        refs,
    );
    acked
}

/// Which outstanding rows this echo carries, in typed order. Pure.
///
/// Whitespace-BLIND matching: a delivered line travels through tmux
/// send-keys and an agent TUI's composer before it comes back in the
/// userPromptSubmit echo, and that round trip does not preserve whitespace.
/// A composer may render a newline as a space or its own wrap — and worse,
/// tmux in extended-keys mode DROPPED the raw \n byte outright, so the echo
/// came back with the lines GLUED ("AgenticAI\nAgentic" → "AgenticAIAgentic")
/// and a squash-to-one-space canon could never contain it (owner,
/// 2026-08-22 "发送内容有换行 好像就不会被confirm", again 2026-08-24 "多行内容
/// …没办法正确已读，匹配有问题"). Stripping ALL whitespace on BOTH sides
/// forgives every rendering of a break. The characters still have to match
/// in order, so this cannot ack the wrong line.
///
/// Any outstanding line may be the one this prompt carries — a queue is
/// submitted in order, but an agent can also be steered, so match on CONTENT.
/// One submitted prompt can carry several queued lines at once; a line the
/// prompt carries ONCE settles only ONE of its duplicates (board #122): each
/// occurrence in the echo is one receipt, spent oldest-first.
fn settled_by<'a>(rows: &'a [super::store::DeliveryRow], prompt: &str) -> Vec<&'a super::store::DeliveryRow> {
    let canon_prompt = strip_ws(prompt);
    let mut spent: HashMap<String, usize> = HashMap::new();
    let mut reverse_spent = false;
    let mut out = Vec::new();
    for row in rows {
        let canon_line = strip_ws(&row.line);
        if canon_line.is_empty() {
            // An all-whitespace line has no shape to match; settle it as
            // containment always did rather than pin it forever.
            out.push(row);
            continue;
        }
        let budget = canon_prompt.matches(canon_line.as_str()).count();
        let used = spent.entry(canon_line.clone()).or_insert(0);
        if *used < budget {
            *used += 1;
            out.push(row);
            continue;
        }
        // The truncation-aware side: the stored echo may be cut while the full
        // line is longer (canon_line strictly longer, containing the whole
        // prompt). One such settle per echo — it is one submission.
        if !reverse_spent && canon_line.len() > canon_prompt.len() && canon_line.contains(&canon_prompt) {
            reverse_spent = true;
            out.push(row);
        }
    }
    out
}

/// ALL whitespace removed (space, tab, CR, LF). The delivery-receipt
/// containment match runs on THIS form — see `record_prompt` for why anything
/// gentler lost multi-line messages: byte-exact lost them to newline→space,
/// squash-to-one-space lost them to newline→NOTHING (tmux dropped the byte).
fn strip_ws(s: &str) -> String {
    s.chars().filter(|c| !c.is_whitespace()).collect()
}

/// Is a typed line overdue for its echo? Pure, so the rule is testable.
///
/// A QUEUED line is not a lost line. An agent that is mid-turn holds what we
/// typed in its input queue — kiro says so on screen ("Type to queue") — and
/// submits it when the turn ends, which for a long turn is many minutes later.
/// Warning about that was warning about the system working: the owner saw
/// perfectly good messages marked unconfirmed ("有一些queue的指令，没办法马上
/// confirm，这个不用立刻就unconfirm", 2026-08-19).
///
/// So the ack clock does not run while a turn is OPEN — `running` (working) or
/// `waiting` (blocked on a permission answer) both mean the queue is holding it.
/// It starts at the turn's END, giving the CLI the same 45 s from the moment it
/// could actually submit the line. A line typed into an idle agent is unchanged:
/// its clock starts when we typed it.
/// Which outstanding rows are overdue, oldest first. Per ROW, because the
/// queue holds several typed at different times, and two identical bodies are
/// two promises. `mark` is the session's recovery mark (see
/// `recovery_mark`): a row at or below it has been waited for only since then.
fn overdue_rows(rec: &Rec, rows: &[super::store::DeliveryRow], mark: (i64, u64), now: u64) -> Vec<i64> {
    if rows.is_empty() {
        return Vec::new();
    }
    // activity_ts 0 on purpose: pane repaints must not keep a line pending
    // forever, and a window with no hook facts can never ack one anyway.
    if matches!(derive_from(rec, 0, now).state.as_str(), "running" | "waiting") {
        return Vec::new();
    }
    let turn_end = rec.end.as_ref().map(|(_, t)| *t).unwrap_or(0);
    rows.iter()
        .filter(|row| !row.warned)
        .filter(|row| {
            let listened = if row.id <= mark.0 { mark.1 } else { 0 };
            let reference = row.ts.max(listened).max(turn_end.min(now));
            now.saturating_sub(reference) >= DELIVERY_ACK_SECS
        })
        .map(|row| row.id)
        .collect()
}

/// Report deliveries that never came back as a prompt. Called before a client
/// reads the feed, which is exactly when the answer is wanted. A report does
/// NOT settle the row (board #249): the line may still be in the CLI's queue,
/// and a late real echo must still find it and settle it as ours, retracting
/// the warn by its row id. The persisted `warned` mark makes the report
/// happen once per row, across sweeps and restarts; the 24 h prune bounds the
/// rows that never echo.
pub fn sweep_deliveries(session: &str) {
    let mark = recovery_mark(session);
    let rows = queue(|s| s.pending_deliveries(session, None)).unwrap_or_default();
    if rows.is_empty() {
        return;
    }
    let now = now();
    let mut by_window: std::collections::BTreeMap<String, Vec<super::store::DeliveryRow>> = Default::default();
    for row in rows {
        by_window.entry(row.window.clone()).or_default().push(row);
    }
    for (window, rows) in by_window {
        let rec = store()
            .lock()
            .unwrap()
            .get(&(session.to_string(), window.clone()))
            .cloned()
            .unwrap_or_default();
        for id in overdue_rows(&rec, &rows, mark, now) {
            if !queue(|s| s.mark_delivery_warned(id)).unwrap_or(false) {
                continue; // settled, or reported by another sweep, meanwhile
            }
            let Some(row) = rows.iter().find(|r| r.id == id) else { continue };
            push_full(
                session,
                &window,
                "warn",
                format!("unconfirmed: {}", truncate_chars(&row.line, 160)),
                String::new(),
                String::new(),
                vec![DeliveryRef::of(row)],
            );
        }
    }
}

fn truncate_chars(input: &str, max: usize) -> String {
    let mut out: String = input.chars().take(max).collect();
    if input.chars().count() > max {
        out.push('…');
    }
    out
}

/// An auto-recovery action (`projects::recovery`) made visible: rendered as a
/// `warn` row because it is ABOUT a window that needed intervention, and warns
/// stay visible at every feed detail level.
pub fn record_recovery(session: &str, window: &str, text: &str) {
    push_event(session, window, "warn", text.to_string());
}

/// Did this window produce any turn fact (accepted prompt, turn end, tool
/// call) at or after `t`? `projects::recovery` asks this to verify that a
/// typed `continue` actually became a turn: the error text stays painted on
/// the screen long after the agent moved on, so the screen cannot answer it
/// (owner, 2026-08-26 — the visible error re-triggered sends into a working
/// agent). Seconds, same clock as the turn facts themselves.
pub fn turn_fact_since(session: &str, window: &str, t: u64) -> bool {
    let map = store().lock().unwrap();
    map.get(&(session.to_string(), window.to_string())).is_some_and(|r| {
        r.prompt.is_some_and(|p| p >= t)
            || r.end.as_ref().is_some_and(|(_, ts)| *ts >= t)
            || r.tool.as_ref().is_some_and(|(_, ts)| *ts >= t)
    })
}

/// Drop records for windows that no longer exist (called opportunistically
/// with the live window set whenever someone lists a session's agents).
pub fn retain_windows(session: &str, live: &[String]) {
    let dead: Vec<String> = {
        let mut map = store().lock().unwrap();
        let dead = map
            .iter()
            .filter(|((s, w), _)| s == session && !live.contains(w))
            .map(|((_, w), _)| w.clone())
            .collect();
        map.retain(|(s, w), _| s != session || live.contains(w));
        dead
    };
    if dead.is_empty() {
        return;
    }
    // A window that is gone can never echo, so its queue is dead weight.
    // Windows this process never held a record for are left to the sweep,
    // which reports them once and settles them — this path costs nothing on the
    // roster poll that calls it several times a minute.
    for w in &dead {
        forget_window_deliveries(session, w);
    }
}

/// Test-only: the message id each outstanding row carries, in typed order.
#[cfg(test)]
pub fn owed_message_ids(session: &str) -> Vec<String> {
    queue(|s| s.pending_deliveries(session, None)).unwrap_or_default().into_iter().map(|r| r.msg_id).collect()
}

/// Drop everything this PROCESS holds for a session — exactly what a restart
/// does to the derived records and the recovery mark, and nothing more: the
/// delivery queue in state.db is deliberately untouched, because its survival
/// is what board #5 is about. Test-only.
#[cfg(test)]
pub fn forget_process_state(session: &str) {
    store().lock().unwrap().retain(|(s, _), _| s != session);
    recovery_marks().lock().unwrap().remove(session);
}

/// Derive the current status for (session, window). `activity_ts` is tmux's
/// window_activity for the window — used ONLY when the window has produced no
/// hook facts at all. Pure given the record + clock.
pub fn derive(session: &str, window: &str, activity_ts: u64) -> AgentStatus {
    let rec = store()
        .lock()
        .unwrap()
        .get(&(session.to_string(), window.to_string()))
        .cloned()
        .unwrap_or_default();
    derive_from(&rec, activity_ts, now())
}

/// Every window with any hook facts, with its derived state — the sidebar's
/// one cheap read across ALL projects at once (owner, 2026-08-24: the project
/// list should show "当前几个 Agent 的简单 logo 状态"). Pure memory: no tmux
/// call, no pane sniff, so `hub_rooms` can afford it on every poll. Windows
/// that never produced a hook fact are simply absent — the client reads
/// absence as idle, which is what no-facts honestly means.
pub fn all_states() -> Vec<(String, String, String)> {
    let t = now();
    let map = store().lock().unwrap();
    map.iter()
        .map(|((s, w), rec)| (s.clone(), w.clone(), derive_from(rec, 0, t).state))
        .collect()
}

/// The state machine, in one place. A turn is a bracket: `userPromptSubmit`
/// opens it, `stop` closes it, tool calls happen inside it, and a
/// permission prompt suspends it. So the rule is simply *which boundary is the
/// most recent fact*, and the four states fall out of that:
///
/// | newest fact                    | state   | since        |
/// |--------------------------------|---------|--------------|
/// | a failed stop                  | failed  | the stop     |
/// | a turn end (stop)              | idle    | the end      |
/// | an ask (permission / input)    | waiting | the ask      |
/// | a turn start (prompt / tool)   | running | the START    |
///
/// `since` for `running` is the turn's start, not the newest event, so "running
/// 2m14s" means the turn has been open that long.
///
/// Pane activity is NOT a work signal for a window with hooks. It used to be,
/// and that was the bug: an agent TUI repaints after replying (spinner, status
/// line, blinking cursor), so `window_activity` was always newer than the stop
/// and every finished agent read as "working" forever. Windows with no hook
/// coverage at all (codex today, anything hand-started) still fall back to it,
/// because for them the alternative is no signal at all.
fn derive_from(rec: &Rec, activity_ts: u64, now: u64) -> AgentStatus {
    let (end_kind, end_ts) = rec.end.clone().unwrap_or_default();
    let ask_ts = rec.ask.as_ref().map(|(_, t)| *t).unwrap_or(0);
    let tool_ts = rec.tool.as_ref().map(|(_, t)| *t).unwrap_or(0);
    let prompt_ts = rec.prompt.unwrap_or(0);

    let turn_start = prompt_ts.max(tool_ts);
    let turn_end = end_ts;
    let newest = turn_start.max(turn_end).max(ask_ts);
    // Ordering keys: (second, arrival). Equal seconds are decided by which
    // fact arrived later (board #249); unequal seconds exactly as before.
    let prompt_k = (prompt_ts, rec.prompt_seq);
    let tool_k = (tool_ts, rec.tool_seq);
    let end_k = (end_ts, rec.end_seq);
    let ask_k = (ask_ts, rec.ask_seq);
    let start_k = prompt_k.max(tool_k);

    // No hook has ever spoken for this window: fall back to pane activity,
    // which is all a hookless backend gives us.
    if newest == 0 {
        let state = if now.saturating_sub(activity_ts) < ACTIVE_SECS { "running" } else { "idle" };
        return AgentStatus { state: state.into(), detail: String::new(), since: activity_ts };
    }

    // A failed stop is the one distress signal we can observe. It stands until
    // a new turn starts.
    if end_kind == "failed" && end_k >= start_k && end_k >= ask_k {
        return AgentStatus { state: "failed".into(), detail: "failed".into(), since: end_ts };
    }

    // A turn that ended is rest, not distress.
    if end_k >= start_k && end_k >= ask_k {
        return AgentStatus { state: "idle".into(), detail: String::new(), since: turn_end };
    }

    // Blocked on the human, and nothing has happened since.
    if ask_k > tool_k && ask_k >= prompt_k {
        let kind = rec.ask.as_ref().map(|(k, _)| k.clone()).unwrap_or_default();
        return AgentStatus { state: "waiting".into(), detail: kind, since: ask_ts };
    }

    // A turn is open. Its last observed tool explains what it is doing.
    let detail = rec
        .tool
        .as_ref()
        .filter(|_| tool_k >= prompt_k)
        .map(|(l, _)| l.clone())
        .unwrap_or_default();
    let since = if prompt_ts > 0 { prompt_ts } else { tool_ts };
    AgentStatus { state: "running".into(), detail, since }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> Rec {
        Rec::default()
    }

    /// An outstanding row as the store returns it: typed at `ts`, by this
    /// process unless `id` is at or below the recovery mark.
    fn row(id: i64, line: &str, ts: u64) -> crate::projects::store::DeliveryRow {
        crate::projects::store::DeliveryRow { id, window: "w".into(), line: line.into(), ts, msg_id: String::new(), warned: false }
    }
    const MARK: (i64, u64) = (0, 0);

    /// The window's outstanding lines, in typed order — the table IS the queue.
    fn held(session: &str, window: &str) -> Vec<String> {
        crate::projects::tests::use_test_store();
        crate::projects::with_store(|s| s.pending_deliveries(session, Some(window)))
            .unwrap()
            .into_iter()
            .map(|r| r.line)
            .collect()
    }

    /// Age every outstanding row of a session past the ack window.
    fn backdate(session: &str) {
        crate::projects::tests::use_test_store();
        let past = (now() - DELIVERY_ACK_SECS - 1) as i64;
        crate::projects::with_store(|s| {
            s.backdate_deliveries(session, past as u64)
        })
        .unwrap();
        recovery_marks().lock().unwrap().insert(session.to_string(), (0, 0));
    }

    /// The sidebar's one cheap read: every window with hook facts, derived.
    /// Sessions are isolated keys, and a window nobody hooked is absent —
    /// which the client reads as idle.
    #[test]
    fn all_states_reports_every_hooked_window_with_its_derived_state() {
        record_prompt("allst-a", "w2", "do the thing");
        record_notification("allst-b", "w3", "completed", 12345);
        let states = all_states();
        let get = |s: &str, w: &str| {
            states.iter().find(|(ss, ww, _)| ss == s && ww == w).map(|(_, _, st)| st.clone())
        };
        assert_eq!(get("allst-a", "w2").as_deref(), Some("running"), "an open turn derives running");
        assert_eq!(get("allst-b", "w3").as_deref(), Some("idle"), "a closed turn derives idle");
        assert_eq!(get("allst-a", "w9"), None, "no hook facts, no row — absence means idle");
    }

    /// A human interrupt closes the turn itself, because nothing else will: the
    /// backend fires no stop hook for a turn cancelled from outside, so the
    /// newest fact would stay the `userPromptSubmit` and the agent would read
    /// `running` for ever.
    #[test]
    fn an_interrupt_closes_the_turn_it_cancelled() {
        record_prompt("int-a", "w1", "do the long thing");
        assert_eq!(derive("int-a", "w1", 0).state, "running");

        record_interrupt("int-a", "w1");
        let after = derive("int-a", "w1", 0);
        assert_eq!(after.state, "idle", "the cancelled turn is over");
        assert!(after.since > 0, "since moves to the interrupt");
        // Pane activity must not resurrect it either: a hooked window ignores
        // the repaint an interrupted TUI does on its way back to the prompt.
        assert_eq!(derive("int-a", "w1", now()).state, "idle");
        // And a REAL new turn afterwards is a fact of its own, so it reads
        // running again — which is the point of resetting FIRST. Checked on the
        // pure state machine because the record's clock is in seconds: an
        // interrupt and the prompt it enables share one second in a test.
        let mut r = rec();
        r.prompt = Some(2000);
        r.end = Some(("completed".into(), 2000)); // interrupted
        assert_eq!(derive_from(&r, 0, 2000).state, "idle");
        r.prompt = Some(2001); // the agent was given something else to do
        assert_eq!(derive_from(&r, 0, 2001).state, "running");
    }

    // ── Delivery acknowledgement: when is a typed line actually overdue?

    /// The owner's report: a line typed at a BUSY agent is queued by its CLI and
    /// submitted when the turn ends, so warning after 45 s was warning about the
    /// system working as designed.
    #[test]
    fn a_queued_line_is_not_an_unconfirmed_line() {
        let mut r = rec();
        r.prompt = Some(1000); // a turn is open — the agent is working
        let rows = [row(1, "@builder-2 do the thing", 1010)];
        // Ten minutes later the turn is STILL running. Nothing is overdue: the
        // line is sitting in the agent's input queue where we put it.
        assert!(overdue_rows(&r, &rows, MARK, 1010 + 600).is_empty());
        // Blocked on a permission answer is the same situation: the queue holds
        // the line until the human answers.
        let mut asking = r.clone();
        asking.ask = Some(("permission_required".into(), 1100));
        assert!(overdue_rows(&asking, &rows, MARK, 1100 + 600).is_empty());
    }

    /// The bug: `pending` was ONE slot, so a second message typed at a busy agent
    /// erased the first one's record. When the agent finally submitted the first
    /// queued line, nothing matched it — so it was reported as a prompt the user
    /// had typed locally (rendered as an "input" row) and the message it belonged
    /// to never got its delivered mark.
    /// Pre + Post for one call are one ROW. Both hooks are subscribed (a backend
    /// may only send one), they carry the same tool and argument, and they arrive
    /// milliseconds apart — so the lane used to show every call twice.
    #[test]
    fn a_pre_and_post_pair_is_one_row() {
        let session = format!("tool-dedupe-{}", std::process::id());
        let rows = |s: &str| {
            recent_events(s, 0).into_iter().filter(|e| e.kind == "tool").count()
        };
        record_tool(&session, "w4", "Read", "/w/src/lib.rs");
        record_tool(&session, "w4", "Read", "/w/src/lib.rs");   // the Post half
        assert_eq!(rows(&session), 1, "one call, one row");

        // A different argument is a different call.
        record_tool(&session, "w4", "Read", "/w/src/main.rs");
        assert_eq!(rows(&session), 2);
        // The same argument in ANOTHER window is that window's own call.
        record_tool(&session, "w9", "Read", "/w/src/main.rs");
        assert_eq!(rows(&session), 3);
        // And the status record still carries the newest line either way.
        let st = derive(&session, "w4", 0);
        assert!(st.detail.contains("main.rs"), "got {:?}", st.detail);
    }

    #[test]
    fn two_lines_queued_at_a_busy_agent_are_both_still_outstanding() {
        let session = format!("queue-test-{}", std::process::id());
        record_prompt(&session, "w1", "start working");      // a turn is open
        record_delivery(&session, "w1", "[tmm chat 00:05] human: first thing", "");
        record_delivery(&session, "w1", "[tmm chat 00:06] human: second thing", "");

        // Both are held, oldest first — the second no longer erases the first.
        assert_eq!(held(&session, "w1").len(), 2);
        assert!(held(&session, "w1")[0].contains("first thing"));

        // The agent works through the queue in order. Each echo acknowledges its
        // OWN line and leaves the other outstanding.
        assert!(record_prompt(&session, "w1", "[tmm chat 00:05] human: first thing"),
            "the first queued line is acknowledged, however late it arrives");
        assert_eq!(held(&session, "w1").len(), 1);
        assert!(held(&session, "w1")[0].contains("second thing"));
        assert!(record_prompt(&session, "w1", "[tmm chat 00:06] human: second thing"));
        assert!(held(&session, "w1").is_empty());

        // A prompt nobody typed for us is still local input.
        assert!(!record_prompt(&session, "w1", "something the user typed at the keyboard"));
    }

    #[test]
    fn one_submitted_prompt_can_carry_the_whole_queue() {
        // kiro submits queued lines together; the hook then reports ONE prompt
        // containing several of ours. Each must be acknowledged, or the earlier
        // messages keep their hollow ring for ever.
        let session = format!("queue-batch-{}", std::process::id());
        record_prompt(&session, "w2", "open the turn");
        record_delivery(&session, "w2", "line one", "");
        record_delivery(&session, "w2", "line two", "");
        assert!(record_prompt(&session, "w2", "line one\nline two\n"));
        assert!(held(&session, "w2").is_empty(), "both lines were in that prompt");
    }

    #[test]
    fn every_overdue_line_is_reported_not_just_the_first() {
        let mut r = rec();
        r.end = Some(("completed".into(), 1000));         // idle since 1000
        let rows = [row(1, "older", 900), row(2, "newer", 1000)];
        // Both past the window.
        assert_eq!(overdue_rows(&r, &rows, MARK, 1000 + DELIVERY_ACK_SECS), vec![1, 2]);
        // Only the older one is past it: the fresh line keeps its own clock.
        assert!(overdue_rows(&r, &rows, MARK, 900 + DELIVERY_ACK_SECS).is_empty(), "the clock runs from the turn end (1000)");
    }

    #[test]
    fn the_clock_starts_when_the_turn_ends() {
        let mut r = rec();
        r.prompt = Some(1000);
        let rows = [row(1, "@builder-2 do the thing", 1010)];
        r.end = Some(("completed".into(), 2000)); // the turn ended much later
        // The CLI gets the full window from the moment it could submit, not from
        // the moment we typed — 990 s after typing is still not overdue.
        assert!(overdue_rows(&r, &rows, MARK, 2000 + DELIVERY_ACK_SECS - 1).is_empty());
        assert_eq!(overdue_rows(&r, &rows, MARK, 2000 + DELIVERY_ACK_SECS), vec![1]);
    }

    #[test]
    fn a_line_typed_at_an_idle_agent_keeps_its_own_clock() {
        let mut r = rec();
        r.prompt = Some(500);
        r.end = Some(("completed".into(), 600)); // idle since 600
        let rows = [row(1, "@builder-2 hello", 1000)]; // typed later
        assert!(overdue_rows(&r, &rows, MARK, 1000 + DELIVERY_ACK_SECS - 1).is_empty());
        assert_eq!(overdue_rows(&r, &rows, MARK, 1000 + DELIVERY_ACK_SECS), vec![1], "an idle agent had its chance");
    }

    #[test]
    fn a_hookless_window_is_still_swept() {
        // No hook facts at all (a hand-started codex, a backend without hooks):
        // it can never ack, and pane repaints must not keep the line pending
        // forever — the warning is the only signal the owner would get.
        let rows = [row(1, "@codex hello", 1000)];
        assert!(overdue_rows(&rec(), &rows, MARK, 1000 + 10).is_empty());
        assert_eq!(overdue_rows(&rec(), &rows, MARK, 1000 + DELIVERY_ACK_SECS), vec![1]);
    }

    #[test]
    fn nothing_pending_is_never_overdue() {
        assert!(overdue_rows(&rec(), &[], MARK, 9999).is_empty());
    }

    /// The bug this machine replaced: an agent that had just answered kept
    /// reading "working" forever, because its TUI repaints the prompt after
    /// every reply and pane activity was treated as work.
    #[test]
    fn a_finished_turn_is_idle_no_matter_how_busy_the_pane_looks() {
        let mut r = rec();
        r.prompt = Some(1000);
        r.tool = Some(("Edit src/lib.rs".into(), 1010));
        r.end = Some(("completed".into(), 1020));
        // Pane activity 500s of it, all newer than the stop — a repainting TUI.
        let s = derive_from(&r, 1500, 1520);
        assert_eq!(s.state, "idle", "a repainting prompt is not work");
        assert_eq!(s.since, 1020, "idle since the turn ended");
    }

    #[test]
    fn a_turn_is_running_from_its_prompt_until_it_stops() {
        let mut r = rec();
        r.prompt = Some(1000);
        let s = derive_from(&r, 0, 1300);
        assert_eq!(s.state, "running");
        assert_eq!(s.since, 1000, "since = the turn's start, so elapsed is the TURN's age");
        // A long think with no tool calls stays running: only an end ends it.
        assert_eq!(derive_from(&r, 0, 1000 + 3600).state, "running");
        // Tools inside the turn keep it running and describe it.
        r.tool = Some(("Edit a.rs".into(), 1200));
        let s = derive_from(&r, 0, 1300);
        assert_eq!((s.state.as_str(), s.detail.as_str(), s.since), ("running", "Edit a.rs", 1000));
    }

    #[test]
    fn tools_without_a_prompt_still_mean_running() {
        // Claude/codex may deliver tool events without a turn-start hook.
        let mut r = rec();
        r.tool = Some(("Bash npm test".into(), 1000));
        let s = derive_from(&r, 0, 1005);
        assert_eq!((s.state.as_str(), s.since), ("running", 1000));
    }

    #[test]
    fn an_ask_suspends_the_turn_and_an_answer_resumes_it() {
        let mut r = rec();
        r.prompt = Some(1000);
        r.ask = Some(("permission_required".into(), 1100));
        let s = derive_from(&r, 0, 9000);
        assert_eq!(s.state, "waiting", "blocked on the human, however long it waits");
        assert_eq!(s.detail, "permission_required");
        // A tool call after the ask means it got answered.
        r.tool = Some(("Bash rm -rf build".into(), 1200));
        assert_eq!(derive_from(&r, 0, 1250).state, "running");
        // And a stop after the ask ends the turn, ask or no ask.
        r.tool = None;
        r.end = Some(("completed".into(), 1300));
        assert_eq!(derive_from(&r, 0, 1350).state, "idle");
    }

    #[test]
    fn a_failed_stop_is_the_one_distress_signal() {
        let mut r = rec();
        r.prompt = Some(900);
        r.end = Some(("failed".into(), 1000));
        assert_eq!(derive_from(&r, 5000, 5000).state, "failed", "and pane noise cannot clear it");
        // Only a new turn clears it.
        r.prompt = Some(1100);
        assert_eq!(derive_from(&r, 0, 1200).state, "running");
    }

    /// kiro v3 runs its memory auto-capture as `memory` tool calls AFTER the
    /// Stop (board #227; measured on two agents, three calls 10–30 s after
    /// `completed`, no new prompt). Recorded as work they reopened the turn and
    /// the card said "working" until the next message. A housekeeping call
    /// counts only inside an open turn — there it is the agent's own use of
    /// the tool — and is dropped after the end.
    #[test]
    fn housekeeping_after_the_stop_does_not_reopen_the_turn() {
        let session = format!("hk-{}", uuid::Uuid::new_v4());
        record_prompt(&session, "w1", "[tmm chat 10:42] human: @aws-expert 1000 字也不用这么大");
        assert!(record_housekeeping_tool(&session, "w1", "memory", ""), "mid-turn the agent may use the tool");
        assert_eq!(derive(&session, "w1", 0).state, "running");
        record_tool(&session, "w1", "execute_bash", "python3 compile.py");
        record_notification(&session, "w1", "completed", now());
        assert_eq!(derive(&session, "w1", 0).state, "idle");
        let rows = recent_events(&session, 0).len();
        for _ in 0..3 {
            assert!(!record_housekeeping_tool(&session, "w1", "memory", ""), "the auto-capture is not work");
        }
        assert_eq!(derive(&session, "w1", 0).state, "idle", "three memory calls after the Stop leave the agent idle");
        assert_eq!(recent_events(&session, 0).len(), rows, "and leave no rows in the feed");
        // A window that never spoke has no open turn either.
        assert!(!record_housekeeping_tool(&session, "w2", "memory", ""));
        assert_eq!(recent_events(&session, 0).len(), rows);
    }

    #[test]
    fn a_window_with_no_hooks_at_all_falls_back_to_pane_activity() {
        let s = derive_from(&rec(), 1000, 1005);
        assert_eq!(s.state, "running", "a hookless backend has nothing else");
        assert_eq!(derive_from(&rec(), 1000, 1000 + ACTIVE_SECS + 1).state, "idle");
    }

    #[test]
    fn a_typed_line_is_acknowledged_by_the_prompt_hook_that_echoes_it() {
        let line = "[tmm chat] human: @dev ship it";
        record_delivery("ack-test", "w3", line, "");
        // The CLI submits our line (possibly with the agent's own leading text
        // when it was mid-typing) — containment, not equality.
        assert!(
            record_prompt("ack-test", "w3", &format!("{line}\n")),
            "the echo must acknowledge the pending delivery"
        );
        let evs = recent_events("ack-test", 0);
        assert_eq!(evs.last().unwrap().kind, "prompt");
        assert_eq!(evs.last().unwrap().via, "app", "an acked prompt came from this app");
        // Nothing pending any more, so the sweep stays silent.
        sweep_deliveries("ack-test");
        assert_eq!(recent_events("ack-test", 0).len(), 1, "no warning for a delivered line");
    }

    /// A multi-line message NEVER matched its echo. Two shapes, two owner
    /// reports: the composer renders a newline its own way (a space, a wrap —
    /// 2026-08-22 "发送内容有换行 好像就不会被confirm"), and tmux in
    /// extended-keys mode DROPPED the raw \n byte outright so the echo came
    /// back with the lines GLUED together (2026-08-24 "多行内容…没办法正确已
    /// 读，匹配有问题"). Matching is whitespace-BLIND now: every rendering of
    /// a break forgives — space, wrap, or nothing — while the characters
    /// still have to match in order.
    #[test]
    fn a_multi_line_delivery_is_acknowledged_despite_whitespace_drift() {
        let line = "[tmm chat] human: @dev line one\nline two\n  line three";
        record_delivery("ws-test", "w1", line, "");
        // The composer turned newlines into single spaces and doubled one.
        assert!(
            record_prompt("ws-test", "w1", "[tmm chat] human: @dev line one line two  line three"),
            "newline → space must still ack"
        );
        // CRLF and trailing whitespace on the echo side.
        record_delivery("ws-test", "w2", "[tmm chat] human: do\nthe thing", "");
        assert!(record_prompt("ws-test", "w2", "[tmm chat] human: do\r\nthe thing \n"));
        // tmux dropped the newline byte: the echo comes back GLUED — the
        // 2026-08-24 shape, measured live in the translator project
        // ("AgenticAI\nAgentic…" echoed as "AgenticAIAgentic…").
        record_delivery("ws-test", "w4", "[tmm chat] human: @dev AgenticAI\nAgentic AI 基础设施", "");
        assert!(
            record_prompt("ws-test", "w4", "[tmm chat] human: @dev AgenticAIAgentic AI 基础设施"),
            "newline → NOTHING must still ack"
        );
        // Different WORDS still refuse — tolerance must not become fuzz.
        record_delivery("ws-test", "w3", "[tmm chat] human: alpha beta", "");
        assert!(!record_prompt("ws-test", "w3", "[tmm chat] human: alpha gamma"));
    }

    // ── Board #5: an outstanding delivery survives OUR restart ─────────────
    //
    // The agent is a separate process: it holds the line we typed in its own
    // input queue and submits it whenever its turn ends. If the server restarts
    // in between, the echo used to arrive with nothing to match — filed as a
    // prompt the human typed at the keyboard (`via: local`, its own input row)
    // while the message it belonged to kept its hollow ring for ever (owner,
    // 2026-08-29). These tests are about the recovery, so they need the durable
    // half: `use_test_store` points the whole test process at a throwaway db.

    /// What the process loses when the binary restarts: every derived record and
    /// every hydration mark. state.db is untouched, which is the point.
    fn simulate_restart(session: &str) {
        forget_process_state(session);
    }

    #[test]
    fn a_pending_delivery_is_still_acknowledged_after_a_server_restart() {
        crate::projects::tests::use_test_store();
        let session = format!("restart-ack-{}", uuid::Uuid::new_v4());
        let line = "[tmm chat 2026-08-29 16:00] human: @dev 部署一下\n第二行";
        record_delivery(&session, "w1", line, "");

        simulate_restart(&session);
        assert_eq!(held(&session, "w1").len(), 1, "the table IS the queue: the restart took nothing");

        // The agent submits what it queued. Glued newlines (the tmux
        // extended-keys shape), so the whitespace-blind match is exercised on
        // the recovered line exactly as on a live one.
        assert!(
            record_prompt(&session, "w1", "[tmm chat 2026-08-29 16:00] human: @dev 部署一下第二行"),
            "the recovered delivery must still be acknowledged as ours"
        );
        let e = recent_events(&session, 0).into_iter().last().unwrap();
        assert_eq!(
            (e.kind.as_str(), e.via.as_str()),
            ("prompt", "app"),
            "the receipt, not a separate line of keyboard input"
        );
        assert!(held(&session, "w1").is_empty(), "acked lines leave the queue");

        // Settled for good: another restart finds nothing to recover, so the
        // sweep cannot warn about a message that WAS delivered.
        simulate_restart(&session);
        sweep_deliveries(&session);
        assert_eq!(
            recent_events(&session, 0).iter().filter(|e| e.kind == "warn").count(),
            0,
            "a delivered line is never reported unconfirmed"
        );
    }

    #[test]
    fn a_restart_recovers_the_whole_queue_and_restarts_its_ack_clock() {
        crate::projects::tests::use_test_store();
        let session = format!("restart-queue-{}", uuid::Uuid::new_v4());
        record_prompt(&session, "w2", "open the turn");
        record_delivery(&session, "w2", "line one", "");
        record_delivery(&session, "w2", "line two", "");
        // A line typed LONG before the restart — the durable row keeps its real
        // typing time, which is what would make the first sweep after a restart
        // report it as unconfirmed seconds before its echo arrives.
        let long_ago = now().saturating_sub(DELIVERY_ACK_SECS * 20);
        crate::projects::with_store(|s| s.insert_delivery(&session, "w2", "stale line", long_ago, "")).unwrap();

        simulate_restart(&session);

        // The sweep is the first thing that touches the session after a restart
        // (a client reading the feed). It recovers the queue and warns about
        // NOTHING: nobody was listening for these echoes while we were down, so
        // the ack clock starts here.
        sweep_deliveries(&session);
        assert_eq!(
            recent_events(&session, 0).iter().filter(|e| e.kind == "warn").count(),
            0,
            "a recovered line gets a real chance to come back"
        );
        assert_eq!(held(&session, "w2").len(), 3, "the whole queue is back, oldest first");

        // One submitted prompt carrying several of our lines still acks each of
        // them — the multi-message match is unchanged by the recovery.
        assert!(record_prompt(&session, "w2", "line one\nline two"));
        assert_eq!(held(&session, "w2"), vec!["stale line".to_string()]);
        assert!(record_prompt(&session, "w2", "stale line"));
        assert!(held(&session, "w2").is_empty());
    }

    #[test]
    fn a_reported_line_is_not_resurrected_and_a_dead_window_is_forgotten() {
        crate::projects::tests::use_test_store();
        let session = format!("restart-sweep-{}", uuid::Uuid::new_v4());
        record_delivery(&session, "w3", "@dev hello", "");
        // Past the ack window: the sweep reports it once and settles it.
        backdate(&session);
        sweep_deliveries(&session);
        assert_eq!(recent_events(&session, 0).iter().filter(|e| e.kind == "warn").count(), 1);

        // A restart must not report it again: the row stays outstanding for a
        // late echo (board #249), but its warned mark is persisted.
        simulate_restart(&session);
        sweep_deliveries(&session);
        assert_eq!(
            recent_events(&session, 0).iter().filter(|e| e.kind == "warn").count(),
            1,
            "reported once, across a restart"
        );
        // And the late real echo still settles it as ours, naming the row the
        // warn named, so a client can retract exactly that warn.
        let warned_id = recent_events(&session, 0).into_iter().find(|e| e.kind == "warn").unwrap().deliveries[0].id;
        assert!(record_prompt(&session, "w3", "@dev hello"), "a warned row is still matchable");
        let echo = recent_events(&session, 0).into_iter().filter(|e| e.kind == "prompt").last().unwrap();
        assert_eq!((echo.via.as_str(), echo.deliveries[0].id), ("app", warned_id));

        // And a window that no longer exists can never echo, so its queue goes
        // with the record instead of waiting for a recycled index to inherit it.
        record_delivery(&session, "w4", "@gone hello", "");
        retain_windows(&session, &[]);
        simulate_restart(&session);
        assert_eq!(
            crate::projects::with_store(|s| s.pending_deliveries(&session, None)).unwrap().len(),
            0,
            "a dead window's queue is dropped"
        );
    }

    // ── Board #9: the trace is COMPLETE, and the READ is what is bounded ────

    /// Nothing is thrown away, full stop. The old default deleted every event past
    /// 2000 per session, sporadically (the prune counter was per process, so a
    /// restart postponed it) — history lost that nobody asked to lose, and analysis
    /// cannot tell a deleted row from an event that never happened (owner, board
    /// #9: "我希望整个 trace 是非常完整的，以便我们后续去做一些分析"). A retention,
    /// if one is ever wanted, belongs in `Config` with a documented key; this test
    /// stands guard over the DEFAULT being "keep it all".
    #[test]
    fn the_durable_trace_is_never_pruned_behind_the_users_back() {
        // The write path holds no retention constant and no prune call any more:
        // the only pruning primitive left is `Store::prune_activity`, which nothing
        // in the recording path calls.
        let src = include_str!("telemetry.rs");
        let body = src.split("mod tests").next().unwrap();
        assert!(
            !body.contains("s.prune_activity("),
            "the recorder must not prune: a complete trace is the goal, not a side effect"
        );
        assert!(
            !body.contains("TMM_ACTIVITY_KEEP"),
            "no private env knob — a retention would go through Config, documented"
        );
        // And what IS bounded is the read, by a page cap.
        assert_eq!(MAX_PAGE_EVENTS, 1000);
        assert!(LOAD_EVENTS <= MAX_PAGE_EVENTS);
    }

    /// A page is bounded however loudly the caller asks, and it walks backwards.
    /// (The durable, indexed version is tested in `store.rs`; here the database is
    /// off, so this is the no-db fallback — which must still honour the cap rather
    /// than answering with everything it holds.)
    #[test]
    fn a_feed_page_is_capped_and_walks_back_from_its_own_cursor() {
        let session = format!("page-{}", uuid::Uuid::new_v4());
        for n in 0..10 {
            record_tool(&session, "w1", "Edit", &format!("f{n}.rs"));
        }
        let (newest, _) = events_page(&session, 0, None, 4);
        assert_eq!(newest.len(), 4, "the caller's limit is honoured");
        assert!(newest.last().unwrap().text.contains("f9"), "the page ENDS at the newest");
        assert!(newest.first().unwrap().text.contains("f6"), "oldest first within the page");

        // An absurd limit is clamped, not obeyed: one RPC may never turn into a
        // multi-megabyte frame.
        let (capped, _) = events_page(&session, 0, None, usize::MAX);
        assert!(capped.len() <= MAX_PAGE_EVENTS);

        // Walking back from the page's own oldest row yields older rows only.
        let cursor = (newest.first().unwrap().ts, newest.first().unwrap().id);
        let (older, _) = events_page(&session, 0, Some(cursor), 4);
        assert!(
            older.iter().all(|e| e.ts <= cursor.0),
            "a backwards page never returns anything newer than its cursor"
        );
        // And the plain read is still the newest page, unchanged for callers that
        // do not page.
        let plain = recent_events(&session, 0);
        assert!(plain.last().unwrap().text.contains("f9"));
    }

    #[test]
    fn a_prompt_typed_at_the_keyboard_is_recorded_as_local_input() {        assert!(!record_prompt("local-test", "w1", "fix the flaky test"), "nothing was pending");
        let e = recent_events("local-test", 1).into_iter().next().unwrap();
        assert_eq!((e.kind.as_str(), e.via.as_str()), ("prompt", "local"));
        assert_eq!(e.text, "fix the flaky test", "the input half of the transcript");
    }

    #[test]
    fn an_unacknowledged_delivery_is_reported_once() {
        // Backdate the pending line past the ack window.
        record_delivery("sweep-test", "w2", "[tmm chat] human: @dev hello", "");
        backdate("sweep-test");
        sweep_deliveries("sweep-test");
        let warns: Vec<String> = recent_events("sweep-test", 0)
            .iter()
            .filter(|e| e.kind == "warn")
            .map(|e| e.text.clone())
            .collect();
        assert_eq!(warns.len(), 1, "one report per unacked line");
        assert!(warns[0].contains("hello"), "the report names the line, got {:?}", warns[0]);
        sweep_deliveries("sweep-test");
        assert_eq!(
            recent_events("sweep-test", 0).iter().filter(|e| e.kind == "warn").count(),
            1,
            "sweeping again must not re-report"
        );
    }

    #[test]
    fn prompt_text_is_capped() {
        let long = "x".repeat(MAX_PROMPT_CHARS + 50);
        record_prompt("cap-test", "w1", &long);
        let e = recent_events("cap-test", 0).into_iter().next().unwrap();
        assert_eq!(e.text.chars().count(), MAX_PROMPT_CHARS + 1, "capped plus the ellipsis");
    }

    #[test]
    fn store_roundtrip_and_window_retention() {
        record_prompt("tsess", "w1", "one");
        record_prompt("tsess", "w2", "two");
        retain_windows("tsess", &["w2".to_string()]);
        let s1 = derive("tsess", "w1", 0);
        assert_eq!(s1.state, "idle", "dropped window's record must be gone");
        let s2 = derive("tsess", "w2", 0);
        assert_eq!(s2.state, "running", "the surviving record still answers");
        retain_windows("tsess", &[]);
    }
    /// Board #120, the reproduction that motivated the name key: with
    /// `renumber-windows on`, killing a lower window shifts every higher
    /// index and a NEWCOMER inherits the freed number. Keyed by index, dev's
    /// open turn painted rev's card and rev's history painted the newcomer's
    /// (measured 2026-09-09: @788 "dev" resolved 2 → 1 after the kill). Keyed
    /// by NAME, the same pane resolves to the same identity before and after
    /// the shift, and the newcomer starts with no facts.
    #[test]
    fn an_index_shift_cannot_move_turn_edges_between_agents() {
        let session = format!("tmm-idx-{}", std::process::id());
        let _ = std::process::Command::new("tmux").args(["kill-session", "-t", &format!("={session}")]).status();
        let created = std::process::Command::new("tmux")
            .args(["new-session", "-d", "-s", &session, "-n", "shell0", "sleep 60"])
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !created {
            eprintln!("no tmux server — skipping");
            return;
        }
        let run = |args: &[&str]| { let _ = std::process::Command::new("tmux").args(args).status(); };
        run(&["new-window", "-d", "-t", &format!("={session}:"), "-n", "dev", "sleep 60"]);
        run(&["set-option", "-t", &format!("={session}:"), "renumber-windows", "on"]);
        let pane_id = String::from_utf8(
            std::process::Command::new("tmux")
                .args(["display-message", "-p", "-t", &format!("={session}:dev"), "#{pane_id}"])
                .output()
                .unwrap()
                .stdout,
        )
        .unwrap()
        .trim()
        .to_string();

        // Ingest exactly as the hook consumer does: resolve the pane, record.
        let (s1, w1, _) = crate::tmux::resolve_pane_id(&pane_id).expect("pane resolves");
        assert_eq!(w1, "dev", "the resolved key is the identity, not a number");
        record_prompt(&s1, &w1, "open a turn");
        assert_eq!(derive(&s1, "dev", 0).state, "running");

        // The shift: kill the lower window; a newcomer takes a freed index.
        run(&["kill-window", "-t", &format!("={session}:shell0")]);
        run(&["new-window", "-d", "-t", &format!("={session}:"), "-n", "newcomer", "sleep 60"]);

        // The SAME pane still resolves to the SAME key…
        let (_, w2, _) = crate::tmux::resolve_pane_id(&pane_id).expect("pane still resolves");
        assert_eq!(w2, "dev", "an index shift does not change the identity");
        assert_eq!(derive(&s1, "dev", 0).state, "running", "dev keeps its own open turn");
        // …and the newcomer inherits NOTHING (under index keys it inherited
        // whatever record sat at its number).
        assert_eq!(derive(&s1, "newcomer", 0).state, "idle", "no facts, honestly idle");

        run(&["kill-session", "-t", &format!("={session}")]);
    }

    /// Board #122 (1): two deliveries with the SAME body are two promises, and
    /// each echo settles exactly ONE of them, oldest first. The old upsert
    /// collapsed them into one entry, so the second echo found nothing and was
    /// filed as local keyboard input — the same hollow-ring symptom as the
    /// single-slot bug, from a third cause.
    #[test]
    fn identical_bodies_are_two_receipts_settled_one_per_echo() {
        let session = format!("dup-{}", uuid::Uuid::new_v4());
        let line = "[tmm chat 2026-09-09 06:30] human: @dev continue";
        record_delivery(&session, "dev", line, "");
        record_delivery(&session, "dev", line, "");
        let held = || held(&session, "dev").len();
        assert_eq!(held(), 2, "two deliveries of one body are two promises");

        // The CLI submits the queued lines one prompt at a time: each echo
        // settles exactly one entry, and both read as OUR delivery.
        assert!(record_prompt(&session, "dev", line), "first echo acks");
        assert_eq!(held(), 1, "one promise left");
        assert!(record_prompt(&session, "dev", line), "second echo acks the second, not via local");
        assert_eq!(held(), 0);

        // And ONE submission carrying the line TWICE settles both at once.
        record_delivery(&session, "dev", line, "");
        record_delivery(&session, "dev", line, "");
        let both = format!("{line}\n{line}");
        assert!(record_prompt(&session, "dev", &both));
        assert_eq!(held(), 0, "a double-carrying echo settles both");
    }

    // ── Board #249: the table is the ONE queue ────────────────────────────

    /// Validator's case: the server restarts in the middle of a long turn and
    /// no hook arrives for more than the ack window. The open turn is restored
    /// from the durable activity log, so the 30 lines the CLI still holds are
    /// not swept, and their echoes settle as ours.
    #[test]
    fn a_restart_mid_turn_keeps_the_turn_open_past_the_ack_window() {
        crate::projects::tests::use_test_store();
        let session = format!("midturn-{}", uuid::Uuid::new_v4());
        let opened = (now() - 600) * 1000;
        // The durable edges an earlier process wrote (persist() is off in tests).
        crate::projects::with_store(|s| {
            s.insert_activity(&session, "w1", opened - 5000, "notif", "completed", "", "", "", "")?;
            s.insert_activity(&session, "w1", opened, "prompt", "the 20-minute turn", "", "app", "", "")
        })
        .unwrap();
        let lines: Vec<String> = (0..30).map(|n| format!("[tmm chat 12:{n:02}] validator: @builder q{n}")).collect();
        for l in &lines {
            record_delivery(&session, "w1", l, "");
        }
        simulate_restart(&session);
        // The first touch after the restart recovers; then 46 s pass with no
        // hook from that window (lines and mark aged past the ack window).
        recovery_mark(&session);
        backdate(&session);
        recovery_marks().lock().unwrap().insert(session.clone(), (i64::MAX, now() - DELIVERY_ACK_SECS - 1));
        sweep_deliveries(&session);
        assert_eq!(derive(&session, "w1", 0).state, "running", "the open turn came back from the log");
        assert_eq!(recent_events(&session, 0).iter().filter(|e| e.kind == "warn").count(), 0, "held lines are not swept");
        for l in &lines {
            assert!(record_prompt(&session, "w1", l), "{l} is still ours");
        }
        // A window whose last edge was a stop is NOT reopened.
        let closed = format!("closed-{}", uuid::Uuid::new_v4());
        crate::projects::with_store(|s| {
            s.insert_activity(&closed, "w1", opened, "prompt", "done turn", "", "app", "", "")?;
            s.insert_activity(&closed, "w1", opened + 1000, "notif", "completed", "", "", "", "")
        })
        .unwrap();
        recovery_mark(&closed);
        assert_eq!(derive(&closed, "w1", 0).state, "idle");
    }

    /// The echo names the messages it settled (board #249), so a client marks
    /// them delivered without holding them. A line without a message (a board
    /// notice) settles and names nothing; keyboard input names nothing.
    #[test]
    fn an_echo_names_the_messages_it_settled() {
        let session = format!("acks-{}", uuid::Uuid::new_v4());
        record_delivery(&session, "w1", "[tmm chat 12:41] validator: @builder one", "m-41");
        record_delivery(&session, "w1", "[tmm chat 12:42] validator: @builder two", "m-42");
        record_delivery(&session, "w1", "[board #241 reply] notice", "");
        assert!(record_prompt(&session, "w1", "[tmm chat 12:41] validator: @builder one\n[tmm chat 12:42] validator: @builder two"));
        assert!(record_prompt(&session, "w1", "[board #241 reply] notice"));
        record_prompt(&session, "w1", "typed at the keyboard");
        let msgs: Vec<Vec<String>> = recent_events(&session, 0).into_iter().filter(|e| e.kind == "prompt")
            .map(|e| e.deliveries.into_iter().map(|d| d.msg).collect()).collect();
        assert_eq!(msgs, vec![vec!["m-41".to_string(), "m-42".to_string()], vec![String::new()], vec![]],
            "each settled ROW is named, with its message where it has one");
        let wire = serde_json::to_value(recent_events(&session, 0).remove(0)).unwrap();
        assert_eq!(wire["deliveries"][0]["msg"], "m-41");
        assert!(wire["deliveries"][0]["id"].as_i64().unwrap() > 0, "the row id is the key");
        let notice = serde_json::to_value(recent_events(&session, 0).remove(1)).unwrap();
        assert!(notice["deliveries"][0].get("msg").is_none(), "a notice row has no message");
        let local = serde_json::to_value(recent_events(&session, 0).pop().unwrap()).unwrap();
        assert!(local.get("deliveries").is_none(), "absent, not [], when it settled none");
    }

    /// 13:12:38 in the trace: a stop and the next queued prompt in the SAME
    /// second. `end >= start` on whole seconds read the new turn as idle, the
    /// ack clock started, and 45 s later the sweep reported the lines the
    /// running agent was still holding. Arrival order decides the tie.
    #[test]
    fn a_prompt_after_a_stop_in_the_same_second_opens_the_turn() {
        let session = format!("tie-{}", uuid::Uuid::new_v4());
        let t = now();
        record_prompt(&session, "w1", "turn one");
        record_notification(&session, "w1", "completed", t);
        record_prompt(&session, "w1", "turn two, queued"); // same second
        assert_eq!(derive(&session, "w1", 0).state, "running", "the later arrival wins the tie");
        record_delivery(&session, "w1", "@builder still queued", "");
        backdate(&session);
        sweep_deliveries(&session);
        assert_eq!(recent_events(&session, 0).iter().filter(|e| e.kind == "warn").count(), 0, "a running turn is not swept");
        // And the reverse order in one second is a finished turn.
        record_notification(&session, "w1", "completed", now());
        assert_eq!(derive(&session, "w1", 0).state, "idle");
    }

    /// 20 lines typed at a busy window and echoed in order are all OUR
    /// deliveries. With the 16-line memory cap the oldest four echoed as
    /// keyboard input (INPUT rows) while their rows stayed in the table.
    #[test]
    fn twenty_lines_at_a_busy_window_all_echo_as_ours() {
        let session = format!("q20-{}", uuid::Uuid::new_v4());
        record_prompt(&session, "w1", "a long turn opens");
        let lines: Vec<String> = (0..20).map(|n| format!("[tmm chat 12:{n:02}] validator: @builder line {n}")).collect();
        for l in &lines {
            record_delivery(&session, "w1", l, "");
        }
        assert_eq!(held(&session, "w1").len(), 20, "no silent cap");
        for l in &lines {
            assert!(record_prompt(&session, "w1", l), "{l} is ours");
        }
        let vias: Vec<String> = recent_events(&session, 0).into_iter().filter(|e| e.kind == "prompt").map(|e| e.via).skip(1).collect();
        assert_eq!(vias, vec!["app".to_string(); 20]);
        assert!(held(&session, "w1").is_empty());
    }

    /// 30 lines outstanding across a restart keep their typed order, and none
    /// is swept inside the grace the recovery gives them. The old fold kept a
    /// random 16 of them through an unordered map.
    #[test]
    fn thirty_pending_lines_survive_a_restart_in_order() {
        let session = format!("q30-{}", uuid::Uuid::new_v4());
        record_prompt(&session, "w1", "a long turn opens");
        let lines: Vec<String> = (0..30).map(|n| format!("[tmm chat 12:{n:02}] orchestrator: @builder item {n}")).collect();
        for l in &lines {
            record_delivery(&session, "w1", l, "");
        }
        simulate_restart(&session);
        sweep_deliveries(&session);
        assert_eq!(recent_events(&session, 0).iter().filter(|e| e.kind == "warn").count(), 0, "the recovery grace holds");
        assert_eq!(held(&session, "w1"), lines, "typed order, all thirty");
        for l in &lines {
            assert!(record_prompt(&session, "w1", l));
        }
        let vias: Vec<String> = recent_events(&session, 0).into_iter().filter(|e| e.kind == "prompt").map(|e| e.via).skip(1).collect();
        assert_eq!(vias, vec!["app".to_string(); 30], "not one of them reads as keyboard input");
    }

    /// Three identical bodies settle one per echo, and the sweep reports only
    /// the ones still owed — each promise once.
    #[test]
    fn duplicate_bodies_settle_one_by_one_and_sweep_by_row() {
        let session = format!("qdup-{}", uuid::Uuid::new_v4());
        let line = "[tmm chat 12:00] human: @dev again";
        for _ in 0..3 {
            record_delivery(&session, "dev", line, "");
        }
        assert!(record_prompt(&session, "dev", line));
        assert_eq!(held(&session, "dev").len(), 2);
        // That echo opened a turn, and a running agent is still holding the
        // other two; the sweep only speaks once the turn has ended.
        backdate(&session);
        sweep_deliveries(&session);
        assert_eq!(recent_events(&session, 0).iter().filter(|e| e.kind == "warn").count(), 0, "queued, not lost");
        // The turn ended long enough ago for the clock: the stop arrived after
        // the prompt that opened it, both past the ack window.
        let ended = now() - DELIVERY_ACK_SECS - 1;
        with_rec(&session, "dev", |r| {
            r.prompt = Some(ended);
            r.prompt_seq = arrival();
            r.end = Some(("completed".into(), ended));
            r.end_seq = arrival();
        });
        sweep_deliveries(&session);
        assert_eq!(recent_events(&session, 0).iter().filter(|e| e.kind == "warn").count(), 2, "each owed promise reported once");
        assert_eq!(held(&session, "dev").len(), 2, "reported, still outstanding for a late echo");
        sweep_deliveries(&session);
        simulate_restart(&session);
        sweep_deliveries(&session);
        assert_eq!(recent_events(&session, 0).iter().filter(|e| e.kind == "warn").count(), 2, "twice more, across a restart: still once each");
    }

    /// A row typed by an earlier process waits from the recovery mark, a row
    /// typed by this one from its own typing time. Pure.
    #[test]
    fn a_recovered_row_waits_from_the_recovery_mark() {
        let r = rec();
        let rows = [row(5, "before the restart", 100), row(6, "after it", 1000)];
        let mark = (5, 1000); // rows <= 5 were typed before a restart at 1000
        assert_eq!(overdue_rows(&r, &rows, mark, 1000 + DELIVERY_ACK_SECS - 1), Vec::<i64>::new());
        assert_eq!(overdue_rows(&r, &rows, mark, 1000 + DELIVERY_ACK_SECS), vec![5, 6]);
        assert_eq!(overdue_rows(&r, &rows, MARK, 100 + DELIVERY_ACK_SECS), vec![5], "without a mark it keeps its typing time");
    }

    /// Orchestrator's key rule (#249): two identical notices with no message,
    /// both reported; one late echo settles exactly one row and names it, so
    /// exactly one warn is retracted and its sibling stays reported and owed.
    #[test]
    fn a_late_echo_settles_one_of_two_warned_identical_notices() {
        let session = format!("wdup-{}", uuid::Uuid::new_v4());
        let notice = "[board #241 reply] Release 1.0.0 — status review → doing";
        record_delivery(&session, "w1", notice, "");
        record_delivery(&session, "w1", notice, "");
        backdate(&session);
        sweep_deliveries(&session);
        let warns: Vec<i64> = recent_events(&session, 0).into_iter().filter(|e| e.kind == "warn").map(|e| e.deliveries[0].id).collect();
        assert_eq!(warns.len(), 2);
        assert_ne!(warns[0], warns[1], "each warn names its own row");
        assert!(record_prompt(&session, "w1", notice));
        let echo = recent_events(&session, 0).into_iter().filter(|e| e.kind == "prompt").last().unwrap();
        assert_eq!(echo.deliveries.iter().map(|d| d.id).collect::<Vec<_>>(), vec![warns[0]], "the oldest row, and only it");
        assert_eq!(held(&session, "w1").len(), 1, "the sibling is still owed");
        assert!(record_prompt(&session, "w1", notice), "and its own late echo settles it too");
        let last = recent_events(&session, 0).into_iter().filter(|e| e.kind == "prompt").last().unwrap();
        assert_eq!(last.deliveries[0].id, warns[1]);
    }

    /// Validator's restart tie (#249): a turn edge recovered from the log keeps
    /// the log's order, and a stop observed AFTER the restart in the same
    /// second still sorts after it — even when the recovered row id is larger
    /// than any number this process has handed out.
    #[test]
    fn a_stop_after_a_restart_sorts_after_a_recovered_prompt_in_the_same_second() {
        crate::projects::tests::use_test_store();
        let session = format!("rtie-{}", uuid::Uuid::new_v4());
        let t = now();
        crate::projects::with_store(|s| s.insert_activity(&session, "w1", t * 1000, "prompt", "long turn", "", "app", "", "")).unwrap();
        let (_, _, row_id) = crate::projects::with_store(|s| s.open_turns(&session)).unwrap().remove(0);
        // Simulate a log far ahead of this process's counter: the recovered
        // order must still move the counter past it.
        let far = row_id + 1_000_000_000;
        simulate_restart(&session);
        with_rec(&session, "w1", |r| {
            r.prompt = Some(t);
            r.prompt_seq = recovered_arrival(far);
        });
        assert_eq!(derive(&session, "w1", 0).state, "running", "the recovered turn is open");
        record_notification(&session, "w1", "completed", t); // same second, after the restart
        assert_eq!(derive(&session, "w1", 0).state, "idle", "the later stop wins the tie");
        record_prompt(&session, "w1", "next queued line"); // and the next prompt, same second
        assert_eq!(derive(&session, "w1", 0).state, "running");
        // A stop then a restart in the same second then a new prompt: the stop
        // is not recovered as open, and the new prompt opens the turn.
        let closed = format!("rtie2-{}", uuid::Uuid::new_v4());
        crate::projects::with_store(|s| {
            s.insert_activity(&closed, "w1", t * 1000, "prompt", "turn", "", "app", "", "")?;
            s.insert_activity(&closed, "w1", t * 1000, "notif", "completed", "", "", "", "")
        })
        .unwrap();
        recovery_mark(&closed);
        assert_eq!(derive(&closed, "w1", 0).state, "idle");
        record_prompt(&closed, "w1", "new prompt, same second");
        assert_eq!(derive(&closed, "w1", 0).state, "running");
    }

    /// Validator's variant (#249): of two identical msg-less deliveries only
    /// the FIRST is warned (the second was typed later and is still in its
    /// ack window). The first late echo settles and names the warned row; the
    /// second echo settles the unwarned sibling. Both read as ours.
    #[test]
    fn only_the_warned_twin_is_named_by_its_late_echo() {
        let session = format!("wone-{}", uuid::Uuid::new_v4());
        let notice = "[board #242 reply] status review → doing";
        record_delivery(&session, "w1", notice, "");
        backdate(&session); // only this row is past the window
        record_delivery(&session, "w1", notice, "");
        sweep_deliveries(&session);
        let warns: Vec<i64> = recent_events(&session, 0).into_iter().filter(|e| e.kind == "warn").map(|e| e.deliveries[0].id).collect();
        let rows: Vec<i64> = crate::projects::with_store(|s| s.pending_deliveries(&session, Some("w1"))).unwrap().iter().map(|r| r.id).collect();
        assert_eq!(warns, vec![rows[0]], "only the first is reported");
        assert!(record_prompt(&session, "w1", notice));
        assert!(record_prompt(&session, "w1", notice));
        let echoes: Vec<(String, i64)> = recent_events(&session, 0).into_iter().filter(|e| e.kind == "prompt")
            .map(|e| (e.via, e.deliveries[0].id)).collect();
        assert_eq!(echoes, vec![("app".to_string(), rows[0]), ("app".to_string(), rows[1])],
            "the first echo retracts the first warn; the second settles the unwarned twin");
        assert!(held(&session, "w1").is_empty());
    }

    /// Validator 14:40: the same-second restart order in a REAL fresh process.
    /// The parent writes an earlier process's turn edges into a scratch
    /// database, then runs this test binary again as a child (a new process,
    /// so a new counter) against it. Two cases, one second each:
    /// (1) stop, restart, new prompt -> running;
    /// (2) open prompt, restart (recovered from the log), stop -> idle, then
    ///     prompt -> running.
    #[test]
    fn same_second_order_survives_a_real_process_restart() {
        let dir = std::env::temp_dir().join(format!("tmm-fresh-proc-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("state.db");
        let t = now();
        {
            let store = crate::projects::store::Store::open(&db).unwrap();
            // Enough earlier rows that recovered ids are well above a fresh
            // process's first counter values.
            for n in 0..50 {
                store.insert_activity("filler", "w9", n, "tool", "x", "Edit", "", "", "").unwrap();
            }
            store.insert_activity("fp-stop", "w1", t * 1000, "prompt", "turn", "", "app", "", "").unwrap();
            store.insert_activity("fp-stop", "w1", t * 1000, "notif", "completed", "", "", "", "").unwrap();
            store.insert_activity("fp-open", "w1", t * 1000, "prompt", "long turn", "", "app", "", "").unwrap();
        }
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "projects::telemetry::tests::fresh_process_child", "--ignored", "--test-threads=1", "--nocapture"])
            .env("TMM_TEST_CHILD_DB", &db)
            .env("TMM_FRESH_T", t.to_string())
            .output()
            .unwrap();
        let log = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        let _ = std::fs::remove_dir_all(&dir);
        assert!(out.status.success(), "child failed:\n{log}");
        assert!(log.contains("1 passed"), "the child really ran:\n{log}");
    }

    /// The child half of `same_second_order_survives_a_real_process_restart`.
    /// Ignored on its own: it needs the parent's scratch database.
    #[test]
    #[ignore]
    fn fresh_process_child() {
        let t: u64 = std::env::var("TMM_FRESH_T").expect("run by the parent").parse().unwrap();
        // (1) stop -> restart -> prompt, one second: nothing is recovered as open.
        recovery_mark("fp-stop");
        assert_eq!(derive("fp-stop", "w1", 0).state, "idle", "a stopped turn is not reopened");
        with_rec("fp-stop", "w1", |r| {
            r.prompt = Some(t);
            r.prompt_seq = arrival();
        });
        assert_eq!(derive("fp-stop", "w1", 0).state, "running", "the new prompt opens the turn");
        // (2) the open prompt is recovered with its row id; a stop observed by
        // this new process in the same second must sort after it.
        recovery_mark("fp-open");
        assert_eq!(derive("fp-open", "w1", 0).state, "running", "recovered from the log");
        record_notification("fp-open", "w1", "completed", t);
        assert_eq!(derive("fp-open", "w1", 0).state, "idle", "the later stop wins the same-second tie");
        with_rec("fp-open", "w1", |r| {
            r.prompt = Some(t);
            r.prompt_seq = arrival();
        });
        assert_eq!(derive("fp-open", "w1", 0).state, "running");
    }

    /// The matcher over rows, pure: order, duplicates and truncation.
    #[test]
    fn settled_by_spends_one_receipt_per_occurrence() {
        let rows = [row(1, "a b", 0), row(2, "a b", 0), row(3, "other", 0), row(4, "a very long line cut short", 0)];
        let ids = |p: &str| settled_by(&rows, p).into_iter().map(|r| r.id).collect::<Vec<_>>();
        assert_eq!(ids("a\nb"), vec![1], "one occurrence settles the oldest duplicate");
        assert_eq!(ids("a b\na b"), vec![1, 2]);
        assert_eq!(ids("a very long line cut"), vec![4], "a truncated echo settles the longer line");
        assert_eq!(ids("nothing of ours"), Vec::<i64>::new());
    }
}
