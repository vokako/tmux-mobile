//! Agent telemetry: the passive half of the v2 dual channel.
//!
//! What an agent SAYS goes through the `tmm` CLI (hub_post / hub_status /
//! hub_done). What we OBSERVE arrives here: hook notifications (it stopped, it
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
//! exception: the queue of lines we typed into a pane is mirrored into state.db,
//! because the agent that will echo them is a separate process that outlives our
//! restarts — see the durable-delivery block below.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

/// Tool activity within this window means "working".
const ACTIVE_SECS: u64 = 30;
/// An explicit `tmm status` declaration expires after this long so a crashed
/// agent cannot stay "working" forever on its own last words.
const EXPLICIT_TTL_SECS: u64 = 30 * 60;
/// How long a line we typed into a pane may wait for its `userPromptSubmit`
/// echo before we call the delivery unconfirmed. Typing is `send-keys`, which
/// succeeds as long as the pane exists — it says nothing about whether the CLI
/// accepted the text as a prompt (a shell would have executed it, a crashed pane
/// swallows it). The hook echo is the only end-to-end proof, so an unacked line
/// is reported rather than assumed. The clock does NOT run while a turn is open:
/// a busy agent QUEUES what we typed by design, so it is measured from the turn's
/// end (see `delivery_overdue`).
const DELIVERY_ACK_SECS: u64 = 45;
/// Prompt text kept per event. Long enough for a real instruction, short
/// enough that 120 of them stay a cheap in-memory ring.
const MAX_PROMPT_CHARS: usize = 1024;
/// Most outstanding typed lines kept per window. A busy agent's queue is short in
/// practice; this only stops an unbounded queue if nothing ever acks.
const MAX_PENDING: usize = 16;
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
    /// Explicit declaration via `tmm status <state> [note]`: (state, note, ts).
    explicit: Option<(String, String, u64)>,
    /// `tmm done [summary]`: (summary, ts). Also ENDS the turn.
    done: Option<(String, u64)>,
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
    /// Lines this app typed into the pane that no `userPromptSubmit` hook has
    /// echoed back yet: (line, ts), oldest first. A QUEUE, not a slot: a busy
    /// agent holds everything we type and submits it later, so two messages sent
    /// during one turn are both outstanding at once. With a single slot the second
    /// erased the first's record, and when the agent finally submitted the first
    /// line nothing matched it — so it was reported as a prompt the user had typed
    /// locally, and the message it belonged to never got its delivered mark (owner,
    /// 2026-08-20: "在对列里后续隔很久才响应的消息 … 显示到 input 上了，但是没有当成
    /// 已读的消息，给跳过了"). Entries leave on their echo or on the sweep.
    pending: Vec<(String, u64)>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentStatus {
    /// Derived state, one of exactly four: `running` (a turn is open — the
    /// agent accepted a prompt and has not stopped), `waiting` (blocked on the
    /// human), `idle` (no turn open) or `failed`.
    pub state: String,
    /// Human line explaining the state (explicit note, what it asked for, the
    /// last observed tool call, or a `tmm done` summary).
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
    pub window: usize,
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
}

fn is_zero(n: &i64) -> bool {
    *n == 0
}

fn events() -> &'static Mutex<HashMap<String, std::collections::VecDeque<ActivityEvent>>> {
    static EVENTS: OnceLock<Mutex<HashMap<String, std::collections::VecDeque<ActivityEvent>>>> = OnceLock::new();
    EVENTS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn push_event(session: &str, window: usize, kind: &str, text: String) {
    push_full(session, window, kind, text, String::new(), String::new());
}

fn push_event_via(session: &str, window: usize, kind: &str, text: String, via: String) {
    push_full(session, window, kind, text, String::new(), via);
}

fn push_full(session: &str, window: usize, kind: &str, text: String, tool: String, via: String) {
    push(
        session,
        ActivityEvent {
            id: 0,
            ts: now_ms(),
            window,
            kind: kind.into(),
            text,
            tool,
            via,
            state: String::new(),
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
        s.insert_activity(session, ev.window, ev.ts, &ev.kind, &ev.text, &ev.tool, &ev.via, &ev.state)
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

/// How much trace this session has: (events, oldest ts, newest ts). For the
/// client's "N of M loaded" and for anyone auditing what we keep.
pub fn events_stats(session: &str) -> (usize, u64, u64) {
    if cfg!(test) {
        return (0, 0, 0);
    }
    super::with_store(|s| s.activity_stats(session)).unwrap_or((0, 0, 0))
}

fn store() -> &'static Mutex<HashMap<(String, usize), Rec>> {
    static STORE: OnceLock<Mutex<HashMap<(String, usize), Rec>>> = OnceLock::new();
    STORE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn with_rec(session: &str, window: usize, f: impl FnOnce(&mut Rec)) {
    let mut map = store().lock().unwrap();
    f(map.entry((session.to_string(), window)).or_default());
}

// ── Outstanding deliveries survive OUR restart ────────────────────────────
//
// Everything else in this record is an OBSERVATION, and an observation we lost
// is simply one we no longer have — the next hook re-establishes the state. A
// pending delivery is different: it is a PROMISE made to a client ("this message
// was typed into the agent's pane; its receipt is coming"), and the thing that
// will keep it is a SEPARATE process that our restart does not touch. The agent
// holds our line in its own input queue and submits it minutes later; if the
// queue of what we typed died with the server, that echo arrives with nothing to
// match, so it is filed as a prompt the human typed at the keyboard (`via:
// local`, rendered as its own input row) and the message keeps its hollow ring
// for ever — the owner's report on board #5. So the queue is mirrored into
// state.db and folded back in lazily, once per window per process.

/// Is the durable half in play? In tests it is only once a test has pointed the
/// process at a throwaway database (`projects::tests::use_test_store`) — the same
/// rule the activity ring follows, so a unit test about in-memory behaviour can
/// never write rows into the developer's real state.db.
#[cfg(test)]
fn durable() -> bool {
    std::env::var_os("TMM_STATE_DB").is_some_and(|p| !p.is_empty())
}

#[cfg(not(test))]
fn durable() -> bool {
    true
}

/// Every write here is FAIL-SOFT: telemetry may never block or break the thing
/// it observes, and without a database the queue is exactly what it was before —
/// in-memory, good for this process's lifetime.
fn remember_delivery(session: &str, window: usize, line: &str, ts: u64) {
    if !durable() {
        return;
    }
    let _ = super::with_store(|s| s.insert_delivery(session, window, line, ts));
}

/// A line is settled — acked by its echo, or reported by the sweep.
fn forget_delivery(session: &str, window: usize, line: &str) {
    if !durable() {
        return;
    }
    let _ = super::with_store(|s| s.delete_delivery(session, window, line));
}

fn forget_window_deliveries(session: &str, window: usize) {
    if !durable() {
        return;
    }
    let _ = super::with_store(|s| s.clear_deliveries(session, Some(window)));
}

/// (session, window) keys whose durable queue this process has already folded
/// into memory. Once folded, memory is the working copy again — the hydration is
/// a recovery step, not a second source of truth.
fn hydrated() -> &'static Mutex<std::collections::HashSet<(String, usize)>> {
    static HYDRATED: OnceLock<Mutex<std::collections::HashSet<(String, usize)>>> = OnceLock::new();
    HYDRATED.get_or_init(|| Mutex::new(std::collections::HashSet::new()))
}

/// Fold the durable queue back into memory. `window` narrows it to one window
/// (the echo path, which must not pay for a query on every prompt once it has
/// recovered); `None` is the whole session (the sweep, which has to see queues
/// belonging to windows this process has not heard from at all).
///
/// A recovered line's ack clock starts NOW, not when it was typed: nothing was
/// listening for its echo while the server was down, and the alternative is to
/// sweep every recovered line as unconfirmed on the first feed read after a
/// restart — seconds before the echo we are trying to catch arrives. Same reason
/// the clock does not run while a turn is open (see `overdue_lines`): a delivery
/// is only overdue once it has had a real chance to come back.
fn hydrate(session: &str, window: Option<usize>) {
    if !durable() {
        return;
    }
    if let Some(w) = window {
        if hydrated().lock().unwrap().contains(&(session.to_string(), w)) {
            return;
        }
    }
    // Lines older than the recovery horizon are dropped once per process:
    // stale rows can only accumulate ACROSS restarts, since within one process
    // the sweep settles every line within its ack window.
    static PRUNED: OnceLock<()> = OnceLock::new();
    PRUNED.get_or_init(|| {
        let cutoff = now().saturating_sub(PENDING_MAX_AGE_SECS);
        let _ = super::with_store(|s| s.prune_deliveries(cutoff));
    });

    let rows = super::with_store(|s| s.pending_deliveries(session, window)).unwrap_or_default();
    let mut by_window: HashMap<usize, Vec<String>> = HashMap::new();
    for (w, line, _typed_at) in rows {
        by_window.entry(w).or_default().push(line);
    }
    // A queried window with no rows still counts as recovered, so the echo path
    // asks the database once and then stays in memory.
    if let Some(w) = window {
        by_window.entry(w).or_default();
    }
    let ts = now();
    for (w, lines) in by_window {
        let key = (session.to_string(), w);
        if !hydrated().lock().unwrap().insert(key) {
            continue; // another caller already folded this window in
        }
        if lines.is_empty() {
            continue;
        }
        with_rec(session, w, |r| {
            for line in lines {
                // Memory wins: a line typed by THIS process keeps its own clock.
                if r.pending.iter().any(|(l, _)| l == &line) {
                    continue;
                }
                r.pending.push((line, ts));
            }
            while r.pending.len() > MAX_PENDING {
                r.pending.remove(0);
            }
        });
    }
}

/// `tmm status <state> [note]` — explicit declaration by the agent.
///
/// The NOTE is the point of this call. Turn boundaries are observed for free
/// (hooks), so a state word tells us nothing we did not know; what the hooks
/// cannot see is what the agent is actually doing, and that only exists if the
/// agent says it (owner, 2026-08-19: "经常一直在做但是没有同步状态").
///
/// The note is no longer an event: `hub_status` POSTS it to the room as a message
/// from the agent, which is what the owner asked for ("status要用agent发送消息的
/// 形式显示") and also what makes it durable — the room is the record, and an
/// event was a thing that vanished on restart. What stays here is the part only
/// this record can answer: the explicit claim, which `derive_from` uses for
/// `waiting`/`blocked`.
pub fn record_status(session: &str, window: usize, state: &str, note: &str) {
    let (state, note, ts) = (state.to_string(), note.to_string(), now());
    with_rec(session, window, |r| r.explicit = Some((state, note, ts)));
}

/// `tmm done [summary]` — completion declared by the agent. Ends the turn.
pub fn record_done(session: &str, window: usize, summary: &str) {
    let (summary, ts) = (summary.to_string(), now());
    with_rec(session, window, |r| {
        r.done = Some((summary, ts));
        r.explicit = None; // done supersedes any earlier declaration
    });
}

/// A hook notification consumed by the AgentNotificationHub. Two different
/// facts arrive here and they are stored apart: a stop ENDS the turn, a
/// permission/input prompt means the agent is blocked on the human while the
/// turn stays open.
pub fn record_notification(session: &str, window: usize, kind: &str, ts: u64) {
    push_event(session, window, "notif", kind.to_string());
    let kind = kind.to_string();
    with_rec(session, window, |r| match kind.as_str() {
        "permission_required" | "input_required" => r.ask = Some((kind, ts)),
        _ => {
            r.end = Some((kind, ts));
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
/// Same shape as a `completed` stop, plus the two supersessions a stop cannot
/// make: `ask` goes (a cancelled turn is not still asking) and the agent's
/// explicit claim goes (a `blocked` note from before the interrupt would
/// outrank the end in `derive_from` when they share a second, and it describes
/// a turn that no longer exists).
pub fn record_interrupt(session: &str, window: usize) {
    let ts = now();
    with_rec(session, window, |r| {
        r.end = Some(("completed".to_string(), ts));
        r.ask = None;
        r.explicit = None;
    });
}

/// A hook tool event (isolated-home agents only, Phase B+): `("Edit",
/// "foo.rs")`. The event keeps the two parts apart for rendering; the status
/// record keeps the joined line, which is what "working — Edit foo.rs" shows.
pub fn record_tool(session: &str, window: usize, tool: &str, detail: &str) {
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
    });
    if dup {
        return;
    }
    push_full(session, window, "tool", detail.to_string(), tool.to_string(), String::new());
}

/// A line this app typed into an agent's pane (`deliver_mentions`). Held as a
/// pending delivery until the agent's `userPromptSubmit` hook echoes it back —
/// in memory and in state.db, because the agent's own queue outlives our process
/// (see the durable-delivery block above).
pub fn record_delivery(session: &str, window: usize, line: &str) {
    let (line, ts) = (line.to_string(), now());
    with_rec(session, window, |r| {
        // Re-typing the same line replaces its entry rather than queueing a
        // duplicate that could never be acked twice.
        r.pending.retain(|(l, _)| l != &line);
        r.pending.push((line.clone(), ts));
        // A queue that only grows is a leak; nobody types this many lines at one
        // agent without the sweep having something to say about it.
        while r.pending.len() > MAX_PENDING {
            r.pending.remove(0);
        }
    });
    remember_delivery(session, window, &line, ts);
}

/// The `userPromptSubmit` hook: the agent accepted a prompt. This is BOTH the
/// input half of the transcript (what the agent was asked, which no other
/// channel carries) and the delivery receipt for a line we typed.
///
/// Returns true when it acknowledged a pending delivery. Matching is
/// containment, not equality: the CLI may submit the line with its own
/// decoration, and an agent that is mid-task receives our line appended to
/// whatever it was already typing.
pub fn record_prompt(session: &str, window: usize, prompt: &str) -> bool {
    let text = truncate_chars(prompt, MAX_PROMPT_CHARS);
    let mut acked = false;
    let ts = now();
    // An echo may be the receipt for a line typed BEFORE this process started —
    // the agent is a separate process and held it in its own queue across our
    // restart. Recover that window's outstanding lines before matching, or the
    // receipt is lost and the prompt is filed as local keyboard input.
    hydrate(session, Some(window));
    // Whitespace-BLIND matching: a delivered line travels through tmux
    // send-keys and an agent TUI's composer before it comes back in the
    // userPromptSubmit echo, and that round trip does not preserve whitespace.
    // A composer may render a newline as a space or its own wrap — and worse,
    // tmux in extended-keys mode DROPPED the raw \n byte outright, so the echo
    // came back with the lines GLUED ("AgenticAI\nAgentic" → "AgenticAIAgentic")
    // and a squash-to-one-space canon could never contain it (owner,
    // 2026-08-22 "发送内容有换行 好像就不会被confirm", again 2026-08-24 "多行内容
    // …没办法正确已读，匹配有问题"). Stripping ALL whitespace on BOTH sides
    // forgives every rendering of a break — space, wrap, or nothing at all.
    // The characters still have to match in order, so this cannot ack the
    // wrong line.
    let canon_prompt = strip_ws(prompt);
    let mut settled: Vec<String> = Vec::new();
    with_rec(session, window, |r| {
        // Any outstanding line may be the one this prompt carries — a queue is
        // submitted in order, but an agent can also be steered, so match on
        // CONTENT and remove exactly what was found. One submitted prompt can
        // carry several queued lines at once, so this keeps going.
        let before = r.pending.len();
        r.pending.retain(|(line, _)| {
            let canon_line = strip_ws(line);
            let hit = canon_prompt.contains(&canon_line) || canon_line.contains(&canon_prompt);
            if hit {
                settled.push(line.clone());
            }
            !hit
        });
        acked = r.pending.len() < before;
        // A turn just opened. This is the ONE honest "it started working"
        // signal: pane activity cannot be it, because an agent TUI repaints its
        // prompt (spinner, status line, cursor) long after it finished.
        r.prompt = Some(ts);
        r.explicit = None; // a new turn supersedes the last turn's words
    });
    for line in &settled {
        forget_delivery(session, window, line);
    }
    push_event_via(
        session,
        window,
        "prompt",
        text,
        if acked { "app".into() } else { "local".into() },
    );
    acked
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
/// Which outstanding lines are overdue, oldest first. Per LINE, because the queue
/// holds several and they were typed at different times.
fn overdue_lines(rec: &Rec, now: u64) -> Vec<String> {
    if rec.pending.is_empty() {
        return Vec::new();
    }
    // activity_ts 0 on purpose: pane repaints must not keep a line pending
    // forever, and a window with no hook facts can never ack one anyway.
    if matches!(derive_from(rec, 0, now).state.as_str(), "running" | "waiting") {
        return Vec::new();
    }
    let turn_end = rec
        .end
        .as_ref()
        .map(|(_, t)| *t)
        .unwrap_or(0)
        .max(rec.done.as_ref().map(|(_, t)| *t).unwrap_or(0));
    rec.pending
        .iter()
        .filter(|(_, typed_ts)| {
            let reference = (*typed_ts).max(turn_end.min(now));
            now.saturating_sub(reference) >= DELIVERY_ACK_SECS
        })
        .map(|(line, _)| line.clone())
        .collect()
}

/// Is anything overdue? Kept as its own predicate because the rule — not the list
/// — is what the tests are about.
fn delivery_overdue(rec: &Rec, now: u64) -> bool {
    !overdue_lines(rec, now).is_empty()
}

/// Report deliveries that never came back as a prompt. Called before a client
/// reads the feed, which is exactly when the answer is wanted; a swept line is
/// cleared so the warning is emitted once.
pub fn sweep_deliveries(session: &str) {
    // The queues this process never saw typed belong here too: after a restart
    // they are exactly the lines whose fate nobody is tracking any more.
    hydrate(session, None);
    let now = now();
    let stale: Vec<(usize, String)> = {
        let mut map = store().lock().unwrap();
        map.iter_mut()
            .filter(|((s, _), _)| s == session)
            .flat_map(|((_, w), r)| {
                let due = overdue_lines(r, now);
                // Each line is reported once, so it leaves the queue with its
                // warning. The ones still in time stay outstanding.
                r.pending.retain(|(line, _)| !due.contains(line));
                due.into_iter().map(move |line| (*w, line)).collect::<Vec<_>>()
            })
            .collect()
    };
    for (window, line) in stale {
        // Reported means settled: the durable copy goes too, so the next restart
        // does not warn about it a second time.
        forget_delivery(session, window, &line);
        push_event(session, window, "warn", format!("unconfirmed: {}", truncate_chars(&line, 160)));
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
pub fn record_recovery(session: &str, window: usize, text: &str) {
    push_event(session, window, "warn", text.to_string());
}

/// Did this window produce any turn fact (accepted prompt, turn end, tool
/// call) at or after `t`? `projects::recovery` asks this to verify that a
/// typed `continue` actually became a turn: the error text stays painted on
/// the screen long after the agent moved on, so the screen cannot answer it
/// (owner, 2026-08-26 — the visible error re-triggered sends into a working
/// agent). Seconds, same clock as the turn facts themselves.
pub fn turn_fact_since(session: &str, window: usize, t: u64) -> bool {
    let map = store().lock().unwrap();
    map.get(&(session.to_string(), window)).is_some_and(|r| {
        r.prompt.is_some_and(|p| p >= t)
            || r.end.as_ref().is_some_and(|(_, ts)| *ts >= t)
            || r.tool.as_ref().is_some_and(|(_, ts)| *ts >= t)
    })
}

/// Drop records for windows that no longer exist (called opportunistically
/// with the live window set whenever someone lists a session's agents).
pub fn retain_windows(session: &str, live: &[usize]) {
    let dead: Vec<usize> = {
        let mut map = store().lock().unwrap();
        let dead = map
            .iter()
            .filter(|((s, w), _)| s == session && !live.contains(w))
            .map(|((_, w), _)| *w)
            .collect();
        map.retain(|(s, w), _| s != session || live.contains(w));
        dead
    };
    if dead.is_empty() {
        return;
    }
    // A window that is gone can never echo, so its durable queue is dead weight
    // that hydration would otherwise resurrect into a recycled window index.
    // Windows this process never held a record for are left to hydrate + sweep,
    // which reports them once and settles them — this path costs nothing on the
    // roster poll that calls it several times a minute.
    for w in &dead {
        forget_window_deliveries(session, *w);
        hydrated().lock().unwrap().remove(&(session.to_string(), *w));
    }
}

/// Drop everything this PROCESS holds for a session — exactly what a restart
/// does to the derived records and the hydration marks, and nothing more: the
/// durable delivery queue in state.db is deliberately untouched, because that is
/// the half whose survival board #5 is about. Test-only.
#[cfg(test)]
pub fn forget_process_state(session: &str) {
    store().lock().unwrap().retain(|(s, _), _| s != session);
    hydrated().lock().unwrap().retain(|(s, _)| s != session);
}

/// Derive the current status for (session, window). `activity_ts` is tmux's
/// window_activity for the window — used ONLY when the window has produced no
/// hook facts at all. Pure given the record + clock.
pub fn derive(session: &str, window: usize, activity_ts: u64) -> AgentStatus {
    let rec = store()
        .lock()
        .unwrap()
        .get(&(session.to_string(), window))
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
pub fn all_states() -> Vec<(String, usize, String)> {
    let t = now();
    let map = store().lock().unwrap();
    map.iter()
        .map(|((s, w), rec)| (s.clone(), *w, derive_from(rec, 0, t).state))
        .collect()
}

/// The state machine, in one place. A turn is a bracket: `userPromptSubmit`
/// opens it, `stop` / `tmm done` closes it, tool calls happen inside it, and a
/// permission prompt suspends it. So the rule is simply *which boundary is the
/// most recent fact*, and the four states fall out of that:
///
/// | newest fact                    | state   | since        |
/// |--------------------------------|---------|--------------|
/// | a failed stop                  | failed  | the stop     |
/// | an explicit `tmm status`       | that    | the claim    |
/// | a turn end (stop / done)       | idle    | the end      |
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
    let done_ts = rec.done.as_ref().map(|(_, t)| *t).unwrap_or(0);
    let ask_ts = rec.ask.as_ref().map(|(_, t)| *t).unwrap_or(0);
    let tool_ts = rec.tool.as_ref().map(|(_, t)| *t).unwrap_or(0);
    let prompt_ts = rec.prompt.unwrap_or(0);

    let turn_start = prompt_ts.max(tool_ts);
    let turn_end = end_ts.max(done_ts);
    let newest = turn_start.max(turn_end).max(ask_ts);

    // No hook has ever spoken for this window: fall back to pane activity,
    // which is all a hookless backend gives us.
    if newest == 0 && rec.explicit.is_none() {
        let state = if now.saturating_sub(activity_ts) < ACTIVE_SECS { "running" } else { "idle" };
        return AgentStatus { state: state.into(), detail: String::new(), since: activity_ts };
    }

    // A failed stop is the one distress signal we can observe. It stands until
    // a new turn starts.
    if end_kind == "failed" && end_ts >= turn_start && end_ts >= ask_ts {
        return AgentStatus { state: "failed".into(), detail: "failed".into(), since: end_ts };
    }

    // The agent's own words. What they are good for is the part we CANNOT
    // observe: "blocked on a credential", "waiting for the API spec". A claim of
    // `working` adds nothing — the turn bracket already knows a turn is open —
    // so it contributes its note and nothing else. That keeps one class of lie
    // out of the system: an agent cannot declare itself busy while its stop hook
    // says the turn is over.
    if let Some((state, note, ts)) = &rec.explicit {
        let claims_block = matches!(state.as_str(), "waiting" | "blocked");
        if claims_block && now.saturating_sub(*ts) < EXPLICIT_TTL_SECS && *ts >= newest {
            return AgentStatus { state: "waiting".into(), detail: note.clone(), since: *ts };
        }
    }

    // A turn that ended is rest, not distress: direct agents fire stop after
    // every exchange and never call `tmm done`.
    if turn_end >= turn_start && turn_end >= ask_ts {
        let summary = rec
            .done
            .as_ref()
            .filter(|(_, t)| *t == turn_end)
            .map(|(s, _)| s.clone())
            .unwrap_or_default();
        return AgentStatus { state: "idle".into(), detail: summary, since: turn_end };
    }

    // Blocked on the human, and nothing has happened since.
    if ask_ts > tool_ts && ask_ts >= prompt_ts {
        let kind = rec.ask.as_ref().map(|(k, _)| k.clone()).unwrap_or_default();
        return AgentStatus { state: "waiting".into(), detail: kind, since: ask_ts };
    }

    // A turn is open. `since` is when it opened; the detail is the agent's own
    // note if it left one this turn, else the last thing we saw it do.
    let note = rec
        .explicit
        .as_ref()
        .filter(|(_, note, ts)| !note.is_empty() && *ts >= prompt_ts)
        .map(|(_, note, _)| note.clone());
    let detail = note.unwrap_or_else(|| {
        rec.tool
            .as_ref()
            .filter(|(_, t)| *t >= prompt_ts)
            .map(|(l, _)| l.clone())
            .unwrap_or_default()
    });
    let since = if prompt_ts > 0 { prompt_ts } else { tool_ts };
    AgentStatus { state: "running".into(), detail, since }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec() -> Rec {
        Rec::default()
    }

    /// The sidebar's one cheap read: every window with hook facts, derived.
    /// Sessions are isolated keys, and a window nobody hooked is absent —
    /// which the client reads as idle.
    #[test]
    fn all_states_reports_every_hooked_window_with_its_derived_state() {
        record_prompt("allst-a", 2, "do the thing");
        record_notification("allst-b", 3, "completed", 12345);
        let states = all_states();
        let get = |s: &str, w: usize| {
            states.iter().find(|(ss, ww, _)| ss == s && *ww == w).map(|(_, _, st)| st.clone())
        };
        assert_eq!(get("allst-a", 2).as_deref(), Some("running"), "an open turn derives running");
        assert_eq!(get("allst-b", 3).as_deref(), Some("idle"), "a closed turn derives idle");
        assert_eq!(get("allst-a", 9), None, "no hook facts, no row — absence means idle");
    }

    /// A human interrupt closes the turn itself, because nothing else will: the
    /// backend fires no stop hook for a turn cancelled from outside, so the
    /// newest fact would stay the `userPromptSubmit` and the agent would read
    /// `running` for ever. The reset also supersedes a `blocked` claim from the
    /// turn that no longer exists — otherwise the explicit branch outranks the
    /// end when the two share a second.
    #[test]
    fn an_interrupt_closes_the_turn_it_cancelled() {
        record_prompt("int-a", 1, "do the long thing");
        record_status("int-a", 1, "blocked", "waiting on a credential");
        assert_eq!(derive("int-a", 1, 0).state, "waiting", "a claimed block stands");

        record_interrupt("int-a", 1);
        let after = derive("int-a", 1, 0);
        assert_eq!(after.state, "idle", "the cancelled turn is over");
        assert!(after.since > 0, "since moves to the interrupt");
        // Pane activity must not resurrect it either: a hooked window ignores
        // the repaint an interrupted TUI does on its way back to the prompt.
        assert_eq!(derive("int-a", 1, now()).state, "idle");
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
        r.pending = vec![("@builder-2 do the thing".into(), 1010)];
        // Ten minutes later the turn is STILL running. Nothing is overdue: the
        // line is sitting in the agent's input queue where we put it.
        assert!(!delivery_overdue(&r, 1010 + 600));
        // Blocked on a permission answer is the same situation: the queue holds
        // the line until the human answers.
        let mut asking = r.clone();
        asking.ask = Some(("permission_required".into(), 1100));
        assert!(!delivery_overdue(&asking, 1100 + 600));
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
        record_tool(&session, 4, "Read", "/w/src/lib.rs");
        record_tool(&session, 4, "Read", "/w/src/lib.rs");   // the Post half
        assert_eq!(rows(&session), 1, "one call, one row");

        // A different argument is a different call.
        record_tool(&session, 4, "Read", "/w/src/main.rs");
        assert_eq!(rows(&session), 2);
        // The same argument in ANOTHER window is that window's own call.
        record_tool(&session, 9, "Read", "/w/src/main.rs");
        assert_eq!(rows(&session), 3);
        // And the status record still carries the newest line either way.
        let st = derive(&session, 4, 0);
        assert!(st.detail.contains("main.rs"), "got {:?}", st.detail);
    }

    #[test]
    fn two_lines_queued_at_a_busy_agent_are_both_still_outstanding() {
        let session = format!("queue-test-{}", std::process::id());
        record_prompt(&session, 1, "start working");      // a turn is open
        record_delivery(&session, 1, "[tmm chat 00:05] human: first thing");
        record_delivery(&session, 1, "[tmm chat 00:06] human: second thing");

        // Both are held, oldest first — the second no longer erases the first.
        let held = |s: &str| {
            store().lock().unwrap().get(&(s.to_string(), 1)).map(|r| r.pending.clone()).unwrap_or_default()
        };
        assert_eq!(held(&session).len(), 2);
        assert!(held(&session)[0].0.contains("first thing"));

        // The agent works through the queue in order. Each echo acknowledges its
        // OWN line and leaves the other outstanding.
        assert!(record_prompt(&session, 1, "[tmm chat 00:05] human: first thing"),
            "the first queued line is acknowledged, however late it arrives");
        assert_eq!(held(&session).len(), 1);
        assert!(held(&session)[0].0.contains("second thing"));
        assert!(record_prompt(&session, 1, "[tmm chat 00:06] human: second thing"));
        assert!(held(&session).is_empty());

        // A prompt nobody typed for us is still local input.
        assert!(!record_prompt(&session, 1, "something the user typed at the keyboard"));
    }

    #[test]
    fn one_submitted_prompt_can_carry_the_whole_queue() {
        // kiro submits queued lines together; the hook then reports ONE prompt
        // containing several of ours. Each must be acknowledged, or the earlier
        // messages keep their hollow ring for ever.
        let session = format!("queue-batch-{}", std::process::id());
        record_prompt(&session, 2, "open the turn");
        record_delivery(&session, 2, "line one");
        record_delivery(&session, 2, "line two");
        assert!(record_prompt(&session, 2, "line one\nline two\n"));
        let left = store().lock().unwrap().get(&(session, 2)).map(|r| r.pending.clone()).unwrap_or_default();
        assert!(left.is_empty(), "both lines were in that prompt");
    }

    #[test]
    fn every_overdue_line_is_reported_not_just_the_first() {
        let mut r = rec();
        r.end = Some(("completed".into(), 1000));         // idle since 1000
        r.pending = vec![("older".into(), 900), ("newer".into(), 1000)];
        // Both past the window.
        let due = overdue_lines(&r, 1000 + DELIVERY_ACK_SECS);
        assert_eq!(due, vec!["older".to_string(), "newer".to_string()]);
        // Only the older one is past it: the fresh line keeps its own clock.
        let one = overdue_lines(&r, 900 + DELIVERY_ACK_SECS);
        assert_eq!(one, Vec::<String>::new(), "the clock runs from the turn end (1000)");
    }

    #[test]
    fn the_clock_starts_when_the_turn_ends() {
        let mut r = rec();
        r.prompt = Some(1000);
        r.pending = vec![("@builder-2 do the thing".into(), 1010)];
        r.end = Some(("completed".into(), 2000)); // the turn ended much later
        // The CLI gets the full window from the moment it could submit, not from
        // the moment we typed — 990 s after typing is still not overdue.
        assert!(!delivery_overdue(&r, 2000 + DELIVERY_ACK_SECS - 1));
        assert!(delivery_overdue(&r, 2000 + DELIVERY_ACK_SECS));
    }

    #[test]
    fn a_line_typed_at_an_idle_agent_keeps_its_own_clock() {
        let mut r = rec();
        r.prompt = Some(500);
        r.end = Some(("completed".into(), 600)); // idle since 600
        r.pending = vec![("@builder-2 hello".into(), 1000)]; // typed later
        assert!(!delivery_overdue(&r, 1000 + DELIVERY_ACK_SECS - 1));
        assert!(delivery_overdue(&r, 1000 + DELIVERY_ACK_SECS), "an idle agent had its chance");
    }

    #[test]
    fn a_hookless_window_is_still_swept() {
        // No hook facts at all (a hand-started codex, a backend without hooks):
        // it can never ack, and pane repaints must not keep the line pending
        // forever — the warning is the only signal the owner would get.
        let mut r = rec();
        r.pending = vec![("@codex hello".into(), 1000)];
        assert!(!delivery_overdue(&r, 1000 + 10));
        assert!(delivery_overdue(&r, 1000 + DELIVERY_ACK_SECS));
    }

    #[test]
    fn nothing_pending_is_never_overdue() {
        assert!(!delivery_overdue(&rec(), 9999));
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
    fn done_is_idle_and_carries_its_summary() {
        let mut r = rec();
        r.prompt = Some(1000);
        r.done = Some(("PR 已提交".into(), 1100));
        let s = derive_from(&r, 0, 2000);
        assert_eq!((s.state.as_str(), s.detail.as_str(), s.since), ("idle", "PR 已提交", 1100));
        // A NEW turn after done is running again, and does not keep the summary.
        r.prompt = Some(1200);
        let s = derive_from(&r, 0, 2000);
        assert_eq!((s.state.as_str(), s.detail.as_str(), s.since), ("running", "", 1200));
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

    /// What an agent says about itself is only trusted where we cannot observe:
    /// a block. A claim of `working` contributes its NOTE and no state, because
    /// the turn bracket already answers "is it running" and a self-declared
    /// state could contradict the hooks.
    #[test]
    fn an_explicit_claim_speaks_only_for_what_we_cannot_observe() {
        let mut r = rec();
        r.prompt = Some(1000);
        // waiting / blocked: unobservable, so the claim stands (and `blocked`
        // reads as waiting — the CLI's vocabulary is wider than the UI's).
        r.explicit = Some(("waiting".into(), "等接口定稿".into(), 1100));
        let s = derive_from(&r, 0, 1200);
        assert_eq!((s.state.as_str(), s.detail.as_str(), s.since), ("waiting", "等接口定稿", 1100));
        r.explicit = Some(("blocked".into(), "no creds".into(), 1100));
        assert_eq!(derive_from(&r, 0, 1200).state, "waiting");
        // A claim of working does NOT set the state; the open turn does, and the
        // note becomes the line the user reads.
        r.explicit = Some(("working".into(), "重写状态机".into(), 1100));
        let s = derive_from(&r, 0, 1200);
        assert_eq!((s.state.as_str(), s.detail.as_str(), s.since), ("running", "重写状态机", 1000),
            "state from the bracket, words from the agent, since = the turn start");
        // And it cannot outlive the turn: a stop is a newer fact.
        r.end = Some(("completed".into(), 1150));
        assert_eq!(derive_from(&r, 0, 1200).state, "idle");
        // A stale block expires, so a crashed agent does not wait forever.
        r.end = None;
        r.explicit = Some(("blocked".into(), "no creds".into(), 1100));
        assert_eq!(derive_from(&r, 0, 1100 + EXPLICIT_TTL_SECS + 1).state, "running",
            "the open turn still explains it");
        r.prompt = None;
        assert_eq!(derive_from(&r, 0, 1100 + EXPLICIT_TTL_SECS + 1).state, "idle");
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
        record_delivery("ack-test", 3, line);
        // The CLI submits our line (possibly with the agent's own leading text
        // when it was mid-typing) — containment, not equality.
        assert!(
            record_prompt("ack-test", 3, &format!("{line}\n")),
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
        record_delivery("ws-test", 1, line);
        // The composer turned newlines into single spaces and doubled one.
        assert!(
            record_prompt("ws-test", 1, "[tmm chat] human: @dev line one line two  line three"),
            "newline → space must still ack"
        );
        // CRLF and trailing whitespace on the echo side.
        record_delivery("ws-test", 2, "[tmm chat] human: do\nthe thing");
        assert!(record_prompt("ws-test", 2, "[tmm chat] human: do\r\nthe thing \n"));
        // tmux dropped the newline byte: the echo comes back GLUED — the
        // 2026-08-24 shape, measured live in the translator project
        // ("AgenticAI\nAgentic…" echoed as "AgenticAIAgentic…").
        record_delivery("ws-test", 4, "[tmm chat] human: @dev AgenticAI\nAgentic AI 基础设施");
        assert!(
            record_prompt("ws-test", 4, "[tmm chat] human: @dev AgenticAIAgentic AI 基础设施"),
            "newline → NOTHING must still ack"
        );
        // Different WORDS still refuse — tolerance must not become fuzz.
        record_delivery("ws-test", 3, "[tmm chat] human: alpha beta");
        assert!(!record_prompt("ws-test", 3, "[tmm chat] human: alpha gamma"));
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

    fn held(session: &str, window: usize) -> Vec<String> {
        store()
            .lock()
            .unwrap()
            .get(&(session.to_string(), window))
            .map(|r| r.pending.iter().map(|(l, _)| l.clone()).collect())
            .unwrap_or_default()
    }

    #[test]
    fn a_pending_delivery_is_still_acknowledged_after_a_server_restart() {
        crate::projects::tests::use_test_store();
        let session = format!("restart-ack-{}", uuid::Uuid::new_v4());
        let line = "[tmm chat 2026-08-29 16:00] human: @dev 部署一下\n第二行";
        record_delivery(&session, 1, line);

        simulate_restart(&session);
        assert!(held(&session, 1).is_empty(), "the in-process record really is gone");

        // The agent submits what it queued. Glued newlines (the tmux
        // extended-keys shape), so the whitespace-blind match is exercised on
        // the recovered line exactly as on a live one.
        assert!(
            record_prompt(&session, 1, "[tmm chat 2026-08-29 16:00] human: @dev 部署一下第二行"),
            "the recovered delivery must still be acknowledged as ours"
        );
        let e = recent_events(&session, 0).into_iter().last().unwrap();
        assert_eq!(
            (e.kind.as_str(), e.via.as_str()),
            ("prompt", "app"),
            "the receipt, not a separate line of keyboard input"
        );
        assert!(held(&session, 1).is_empty(), "acked lines leave the queue");

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
        record_prompt(&session, 2, "open the turn");
        record_delivery(&session, 2, "line one");
        record_delivery(&session, 2, "line two");
        // A line typed LONG before the restart — the durable row keeps its real
        // typing time, which is what would make the first sweep after a restart
        // report it as unconfirmed seconds before its echo arrives.
        let long_ago = now().saturating_sub(DELIVERY_ACK_SECS * 20);
        let _ = crate::projects::with_store(|s| s.insert_delivery(&session, 2, "stale line", long_ago));

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
        assert_eq!(held(&session, 2).len(), 3, "the whole queue is back, oldest first");

        // One submitted prompt carrying several of our lines still acks each of
        // them — the multi-message match is unchanged by the recovery.
        assert!(record_prompt(&session, 2, "line one\nline two"));
        assert_eq!(held(&session, 2), vec!["stale line".to_string()]);
        assert!(record_prompt(&session, 2, "stale line"));
        assert!(held(&session, 2).is_empty());
    }

    #[test]
    fn a_reported_line_is_not_resurrected_and_a_dead_window_is_forgotten() {
        crate::projects::tests::use_test_store();
        let session = format!("restart-sweep-{}", uuid::Uuid::new_v4());
        record_delivery(&session, 3, "@dev hello");
        // Past the ack window: the sweep reports it once and settles it.
        with_rec(&session, 3, |r| {
            let (line, ts) = r.pending[0].clone();
            r.pending = vec![(line, ts - DELIVERY_ACK_SECS - 1)];
        });
        sweep_deliveries(&session);
        assert_eq!(recent_events(&session, 0).iter().filter(|e| e.kind == "warn").count(), 1);

        // A restart must not find it again: reported IS settled, and warning a
        // second time about the same line is the noise this table could add.
        simulate_restart(&session);
        sweep_deliveries(&session);
        assert_eq!(
            recent_events(&session, 0).iter().filter(|e| e.kind == "warn").count(),
            1,
            "the durable copy left with the warning"
        );

        // And a window that no longer exists can never echo, so its queue goes
        // with the record instead of waiting for a recycled index to inherit it.
        record_delivery(&session, 4, "@gone hello");
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
            record_tool(&session, 1, "Edit", &format!("f{n}.rs"));
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
    fn a_prompt_typed_at_the_keyboard_is_recorded_as_local_input() {        assert!(!record_prompt("local-test", 1, "fix the flaky test"), "nothing was pending");
        let e = recent_events("local-test", 1).into_iter().next().unwrap();
        assert_eq!((e.kind.as_str(), e.via.as_str()), ("prompt", "local"));
        assert_eq!(e.text, "fix the flaky test", "the input half of the transcript");
    }

    #[test]
    fn an_unacknowledged_delivery_is_reported_once() {
        // Backdate the pending line past the ack window.
        record_delivery("sweep-test", 2, "[tmm chat] human: @dev hello");
        with_rec("sweep-test", 2, |r| {
            let (line, ts) = r.pending[0].clone();
            r.pending = vec![(line, ts - DELIVERY_ACK_SECS - 1)];
        });
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
        record_prompt("cap-test", 1, &long);
        let e = recent_events("cap-test", 0).into_iter().next().unwrap();
        assert_eq!(e.text.chars().count(), MAX_PROMPT_CHARS + 1, "capped plus the ellipsis");
    }

    #[test]
    fn store_roundtrip_and_window_retention() {
        record_status("tsess", 1, "waiting", "note");
        record_status("tsess", 2, "waiting", "");
        retain_windows("tsess", &[2]);
        let s1 = derive("tsess", 1, 0);
        assert_eq!(s1.state, "idle", "dropped window's record must be gone");
        let s2 = derive("tsess", 2, 0);
        assert_eq!(s2.state, "waiting", "the surviving record still answers");
        retain_windows("tsess", &[]);
    }
}
