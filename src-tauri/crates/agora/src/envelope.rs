//! The message envelope.
//!
//! Simplified model (see docs/design-docs/agora.md):
//! - An agent only ever does two things: `post(to, body)` and `wait`.
//! - There is no `kind`/`in_reply_to` to manage. A message is just *who said what to
//!   whom*. Whether a reply is owed is tracked by the bus as an obligation graph
//!   ("who owes whom a response"), derived from addressing — not from message types.
//! - `to` empty = broadcast (everyone reads, nobody is obligated).
//! - `to` set = directed; reply obligations are controlled by the bus. `@all`
//!   always requires replies, while named mentions may be informational.

use serde::{Deserialize, Serialize};

/// Message kind. Agents always post `Msg`; the others are bus-generated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, schemars::JsonSchema, Default)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    /// A normal message posted by a participant.
    #[default]
    Msg,
    /// Presence: an agent came online.
    Join,
    /// Presence: an agent went offline.
    Leave,
    /// Bus/operator system notice.
    System,
}

impl Kind {
    pub fn as_str(self) -> &'static str {
        match self {
            Kind::Msg => "msg",
            Kind::Join => "join",
            Kind::Leave => "leave",
            Kind::System => "system",
        }
    }
}

impl std::str::FromStr for Kind {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "msg" | "message" => Ok(Kind::Msg),
            "join" => Ok(Kind::Join),
            "leave" => Ok(Kind::Leave),
            "system" => Ok(Kind::System),
            other => Err(format!("unknown message kind: {other}")),
        }
    }
}

/// Sentinel recipients that mean "everyone".
pub const ALL_TOKENS: [&str; 2] = ["all", "*"];

/// A message in the group chat.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
pub struct Message {
    /// Monotonic sequence number assigned by the store (the log position).
    pub seq: i64,
    pub id: String,
    /// Unix epoch milliseconds when the bus accepted the message.
    pub ts: i64,
    pub room: String,
    /// Sender name.
    pub from: String,
    /// Addressees. Empty = broadcast. May contain agent names or an `all`/`*` token.
    pub to: Vec<String>,
    pub kind: Kind,
    /// Natural-language body (markdown). Keep it terse — share data via files, not chat.
    pub body: String,
}

impl Message {
    /// True if the message is directed (has at least one explicit addressee).
    pub fn is_directed(&self) -> bool {
        !self.to.is_empty()
    }

    /// True if `agent` is among the addressees (directly or via an `all`/`*` token).
    pub fn addresses(&self, agent: &str) -> bool {
        self.to.iter().any(|t| {
            t.eq_ignore_ascii_case(agent) || ALL_TOKENS.contains(&t.to_ascii_lowercase().as_str())
        })
    }
}

/// Current unix time in milliseconds.
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
