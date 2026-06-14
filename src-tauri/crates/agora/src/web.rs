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
use rmcp::transport::streamable_http_server::StreamableHttpService;
use serde::Deserialize;
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

#[derive(Clone)]
struct AppState {
    bus: Bus,
}

/// Build the full axum router (MCP + dashboard) for the given bus.
pub fn router(bus: Bus) -> Router {
    let mcp_service = StreamableHttpService::new(
        {
            let bus = bus.clone();
            move || Ok(AgoraMcp::new(bus.clone()))
        },
        Arc::new(LocalSessionManager::default()),
        Default::default(),
    );

    Router::new()
        .route("/", get(dashboard))
        .route("/events", get(sse))
        .route("/api/post", post(api_post))
        .route("/api/roster", get(api_roster))
        .route("/api/quiescence", get(api_quiescence))
        .route("/api/history", get(api_history))
        .route("/api/employees", get(api_employees).post(api_seed_employee))
        .nest_service("/mcp", mcp_service)
        .with_state(AppState { bus })
}

/// Bind `addr` and serve the full app (MCP + dashboard) until the process ends.
/// Used by the tmux-mobile desktop server to expose the in-process bus to
/// external coding agents without pulling axum into the host crate.
pub async fn serve(bus: Bus, addr: &str) -> anyhow::Result<()> {
    let app = router(bus);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn dashboard() -> Html<&'static str> {
    Html(DASHBOARD_HTML)
}

/// Live message stream. Replays recent history, then streams new messages.
async fn sse(State(st): State<AppState>) -> Sse<impl tokio_stream::Stream<Item = Result<Event, Infallible>>> {
    let rx = st.bus.subscribe();
    let history = st.bus.history(100).unwrap_or_default();

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

#[derive(Debug, Deserialize)]
struct PostBody {
    #[serde(default)]
    from: Option<String>,
    body: String,
    /// If omitted, inferred: a message containing an @mention requires a reply.
    #[serde(default)]
    requires_reply: Option<bool>,
}

async fn api_post(State(st): State<AppState>, Json(b): Json<PostBody>) -> impl IntoResponse {
    let from = b.from.unwrap_or_else(|| "human".to_string());
    let rr = b.requires_reply.unwrap_or_else(|| b.body.contains('@'));
    match st.bus.post(&from, &b.body, rr) {
        Ok(m) => Json(m).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn api_roster(State(st): State<AppState>) -> impl IntoResponse {
    match st.bus.roster() {
        Ok(r) => Json(r).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn api_quiescence(State(st): State<AppState>) -> impl IntoResponse {
    match st.bus.quiescence() {
        Ok(q) => Json(q).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

async fn api_employees(State(st): State<AppState>) -> impl IntoResponse {
    match st.bus.employees() {
        Ok(e) => Json(e).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct SeedBody {
    name: String,
    #[serde(default)]
    spec: serde_json::Value,
}

/// Seed the desired roster with an employee (used by run.py for the initial team).
async fn api_seed_employee(State(st): State<AppState>, Json(b): Json<SeedBody>) -> impl IntoResponse {
    match st.bus.seed_employee(&b.name, &b.spec) {
        Ok(()) => Json(serde_json::json!({"ok": true, "name": b.name})).into_response(),
        Err(e) => (axum::http::StatusCode::BAD_REQUEST, e.to_string()).into_response(),
    }
}

#[derive(Debug, Deserialize)]
struct HistoryQuery {
    #[serde(default)]
    limit: Option<i64>,
}

async fn api_history(State(st): State<AppState>, Query(q): Query<HistoryQuery>) -> impl IntoResponse {
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    match st.bus.history(limit) {
        Ok(m) => Json(m).into_response(),
        Err(e) => (axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()).into_response(),
    }
}

const DASHBOARD_HTML: &str = include_str!("dashboard.html");
