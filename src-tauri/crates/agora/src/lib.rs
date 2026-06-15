//! agora — a group-chat message bus for multi-agent coordination.
//!
//! The crate is layered:
//! - [`envelope`]: the message types (transport-independent).
//! - [`store`]: SQLite-backed durable log + roster.
//! - [`bus`]: coordination logic — join/post/wait, bus-side reply discipline,
//!   presence, and quiescence, with broadcast-based wakeups.
//!
//! The binary (`src/main.rs`) wires the bus to a single daemon that serves MCP over
//! HTTP for agents and a live dashboard for humans.

pub mod bus;
pub mod envelope;
pub mod mcp;
pub mod store;
pub mod web;
