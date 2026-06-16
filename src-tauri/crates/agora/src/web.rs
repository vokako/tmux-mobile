//! The web layer: a single axum app that serves both the MCP endpoint (for agents)
//! and a live dashboard (for humans), backed by the same [`Bus`].
//!
//! Routes:
//! - `POST /mcp`            — MCP over Streamable HTTP (agents connect here)
//! - `GET  /`               — dashboard HTML
//! - `GET  /events`         — Server-Sent Events: the live message stream
//! - `POST /api/post`       — human posts a message (the human is a chat participant)
//! - `GET  /api/roster`     — roster + presence
//! - `GET  /api/quiescence` — done / deadlock / active classification
//! - `GET  /api/history`    — recent messages

use crate::bus::Bus;
use crate::mcp::AgoraMcp;
use axum::extract::{Query, State};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{Html, IntoResponse};
use axum::routing::{get, post};
use axum::{Json, Router};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[derive(Clone)]
struct AppState {
    provider: Arc<dyn crate::bus::BusProvider>,
}

impl AppState {
    /// Resolve the bus for a `?room=` query (or the provider default).
    fn bus(&self, room: &Option<String>) -> Option<Bus> {
        let room = room.clone().unwrap_or_else(|| self.provider.default_room());
        self.provider.bus_for(&room)
    }
}

/// Build the full axum router (MCP + dashboard) for the given bus provider.
/// Each request is routed to a room: agents via the `x-room` header, the human
/// API via a `?room=` query (both default to the provider's default room).
pub fn router(provider: Arc<dyn crate::bus::BusProvider>) -> Router {
    // Stateless mode: each POST is handled on its own, with NO server-side
    // session id and NO mandatory `initialize` handshake. This is essential for
    // the in-process daemon: it lives in the desktop server process, so every
    // backend restart wipes any in-memory session map. With stateful sessions, a
    // client still presenting its OLD `Mcp-Session-Id` after a restart is
    // rejected (`401 Session not found`; a no-session request gets `422 expect
    // initialize`) and rmcp does not auto-re-handshake — the agent hangs on
    // `wait` forever (roster goes offline → the UI spins "coming online"
    // indefinitely). Our tool surface is genuinely stateless: identity is
    // resolved per-request from the `x-agent`/`x-room` headers and all state
    // lives in SQLite, and the agent loop is pure request/response (`post`/
    // `wait`), needing no server→client push. So statelessness costs us nothing
    // and lets any fresh request work with no init — which is what lets a nudged
    // agent reconnect after a restart (see team_bridge::recover_running_teams
    // and team::nudge_session_agents).
    let mcp_service = StreamableHttpService::new(
        {
            let provider = provider.clone();
            move || Ok(AgoraMcp::new(provider.clone()))
        },
        Arc::new(LocalSessionManager::default()),
        StreamableHttpServerConfig { stateful_mode: false, ..Default::default() },
    );

    Router::new()
        .route("/", get(dashboard))
        .route("/events", get(sse))
        .route("/api/post", post(api_post))
        .route("/api/roster", get(api_roster))
        .route("/api/quiescence", get(api_quiescence))
        .route("/api/history", get(api_history))
        .route("/api/employees", get(api_employees).post(api_seed_employee))
        .route("/api/heartbeat", post(api_heartbeat))
        .nest_service("/mcp", mcp_service)
        .with_state(AppState { provider })
}

/// Convenience for a single-room deployment (e.g. tests): wrap one `Bus`.
pub fn router_single(bus: Bus) -> Router {
    router(Arc::new(crate::bus::SingleRoom(bus)))
}

/// Bind `addr` and serve the full app (MCP + dashboard) until the process ends.
/// Used by the tmux-mobile desktop server to expose the in-process bus(es) to
/// external coding agents without pulling axum into the host crate.
pub async fn serve(provider: Arc<dyn crate::bus::BusProvider>, addr: &str) -> anyhow::Result<()> {
    let app = router(provider);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

const ERR_NO_ROOM: (axum::http::StatusCode, &str) =
    (axum::http::StatusCode::NOT_FOUND, "unknown room");

async fn dashboard() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

/// Live message stream for the dashboard. Replays recent history, then streams
/// new messages for the `?room=` room (default room if omitted).
async fn sse(State(st): State<AppState>, Query(q): Query<RoomQuery>) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    // Fall back to the default room's bus if the requested room is unknown, so
    // the dashboard SSE never 500s; an empty stream is fine.
    let bus = st.bus(&q.room).or_else(|| st.bus(&None)).expect("default room must exist");
    let rx = bus.subscribe();
    let history = bus.history(100).unwrap_or_default();

    let replay = tokio_stream::iter(history.into_iter().map(|m| {
        Ok(Event::default()
            .event("message")
            .data(serde_json::to_string(&m).unwrap_or_default()))
    }));

    let live = BroadcastStream::new(rx).filter_map(|res| match res {
        Ok(m) => Some(Ok(Event::default()
            .event("message")
            .data(serde_json::to_string(&m).unwrap_or_default()))),
        Err(_) => None, // lagged: dashboard re-syncs via /api/history if needed
    });

    Sse::new(replay.chain(live)).keep_alive(KeepAlive::new().interval(Duration::from_secs(15)))
}

/// `?room=` selector shared by the read-only GET handlers.
#[derive(Debug, Deserialize)]
struct RoomQuery {
    #[serde(default)]
    room: Option<String>,
    #[serde(default)]
    limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
struct PostBody {
    #[serde(default)]
    from: Option<String>,
    body: String,
    /// If omitted, inferred: a message containing an @mention requires a reply.
    #[serde(default)]
    requires_reply: Option<bool>,
    #[serde(default)]
    room: Option<String>,
}

async fn api_post(State(st): State<AppState>, Json(b): Json<PostBody>) -> axum::response::Response {
    let Some(bus) = st.bus(&b.room) else { return ERR_NO_ROOM.into_response() };
    let from = b.from.unwrap_or_else(|| "human".to_string());
    let rr = b.requires_reply.unwrap_or_else(|| b.body.contains('@'));
    match bus.post(&from, &b.body, rr) {
        Ok(m) => Json(m).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

/// `POST /api/heartbeat` — an agent's tool hook pings this each tool/prompt so a
/// heads-down working agent keeps a fresh `last_seen` instead of decaying to
/// `unreachable`. Identity comes from the same headers agents send to `/mcp`:
/// `x-agent` (who) + `x-room` (which team). Best-effort: always 204.
async fn api_heartbeat(State(st): State<AppState>, headers: axum::http::HeaderMap) -> axum::response::Response {
    let hdr = |k: &str| headers.get(k).and_then(|v| v.to_str().ok()).map(|s| s.to_string());
    let agent = hdr("x-agent").unwrap_or_default();
    if agent.is_empty() {
        return (axum::http::StatusCode::BAD_REQUEST, "missing x-agent").into_response();
    }
    let Some(bus) = st.bus(&hdr("x-room")) else { return ERR_NO_ROOM.into_response() };
    let _ = bus.heartbeat(&agent);
    axum::http::StatusCode::NO_CONTENT.into_response()
}

async fn api_roster(State(st): State<AppState>, Query(q): Query<RoomQuery>) -> axum::response::Response {
    let Some(bus) = st.bus(&q.room) else { return ERR_NO_ROOM.into_response() };
    match bus.roster() {
        Ok(r) => Json(r).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn api_quiescence(State(st): State<AppState>, Query(q): Query<RoomQuery>) -> axum::response::Response {
    let Some(bus) = st.bus(&q.room) else { return ERR_NO_ROOM.into_response() };
    match bus.quiescence() {
        Ok(qx) => Json(qx).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn api_employees(State(st): State<AppState>, Query(q): Query<RoomQuery>) -> axum::response::Response {
    let Some(bus) = st.bus(&q.room) else { return ERR_NO_ROOM.into_response() };
    match bus.employees() {
        Ok(e) => Json(e).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct SeedBody {
    name: String,
    #[serde(default)]
    spec: serde_json::Value,
    #[serde(default)]
    room: Option<String>,
}

/// Seed the desired roster with an employee (used by run.py for the initial team).
async fn api_seed_employee(State(st): State<AppState>, Json(b): Json<SeedBody>) -> axum::response::Response {
    let Some(bus) = st.bus(&b.room) else { return ERR_NO_ROOM.into_response() };
    match bus.seed_employee(&b.name, &b.spec) {
        Ok(()) => Json(serde_json::json!({"ok": true, "name": b.name})).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

async fn api_history(State(st): State<AppState>, Query(q): Query<RoomQuery>) -> axum::response::Response {
    let Some(bus) = st.bus(&q.room) else { return ERR_NO_ROOM.into_response() };
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    match bus.history(limit) {
        Ok(m) => Json(m).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

const DASHBOARD_HTML: &str = include_str!("dashboard.html");
