//! The MCP tool surface, served over Streamable HTTP.
//!
//! Identity is bound per-connection via the `x-agent` header (set in each agent's
//! MCP config), not via tool arguments. The agent's whole API is two tools:
//! `post` and `wait`. `list_agents`/`history` exist for occasional catch-up but the
//! roster is also returned by `wait`, so they rarely need to be called.

use crate::bus::{Bus, WaitOutcome};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::{Extension, Parameters};
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ErrorData, ServerHandler};
use serde::Deserialize;
use std::future::Future;
use std::time::Duration;

#[derive(Clone)]
pub struct AgoraMcp {
    bus: Bus,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PostArgs {
    /// The message. Address agents by writing @name in it (use @all for everyone);
    /// other agents read it and decide whether to act. Keep it short; share real data
    /// and artifacts as files in the workspace, not pasted here.
    pub body: String,
    /// Set true to require the agents you @mention to reply: each is reminded — and
    /// refused idle — until they answer you. Leave false (default) for informational
    /// messages that need no reply.
    #[serde(default)]
    pub requires_reply: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WaitArgs {
    /// Max milliseconds to block before returning (server-capped at 50000).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HistoryArgs {
    /// How many recent messages to return (default 50, max 1000).
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct HireArgs {
    /// Unique name for the new worker (e.g. "search-worker").
    pub name: String,
    /// One-line role/skill for the worker (e.g. "web search and information gathering").
    pub responsibility: String,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct FireArgs {
    /// Name of the employee to disable.
    pub name: String,
}

fn err(msg: impl Into<String>) -> ErrorData {
    ErrorData::invalid_params(msg.into(), None)
}

/// Resolve the caller from the `x-agent` header and ensure it is registered.
fn identity(bus: &Bus, parts: &http::request::Parts) -> Result<String, ErrorData> {
    let name = parts
        .headers
        .get("x-agent")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| err("missing required 'x-agent' header identifying the caller"))?;
    let role = parts
        .headers
        .get("x-agent-role")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    bus.join(&name, role.as_deref()).map_err(|e| err(format!("join failed: {e}")))?;
    Ok(name)
}

// --- agent-facing rendering: only what helps the LLM act, no DB/internal fields ---

fn render_msg(m: &crate::envelope::Message, me: &str) -> String {
    use crate::envelope::Kind;
    match m.kind {
        Kind::Join | Kind::Leave | Kind::System => format!("[system] {}", m.body),
        Kind::Msg => {
            let tgt = if m.to.is_empty() {
                String::new() // broadcast
            } else if m.addresses(me) {
                " → you".to_string()
            } else {
                format!(" → {}", m.to.join(","))
            };
            format!("{}{}: {}", m.from, tgt, m.body)
        }
    }
}

fn render_roster(roster: &[crate::store::AgentRow]) -> String {
    let parts: Vec<String> = roster
        .iter()
        .filter(|a| a.status != "offline")
        .map(|a| format!("{}({})", a.name, a.status))
        .collect();
    if parts.is_empty() {
        "Present: (none)".to_string()
    } else {
        format!("Present: {}", parts.join(" · "))
    }
}

#[tool_router]
impl AgoraMcp {
    pub fn new(bus: Bus) -> Self {
        Self { bus, tool_router: Self::tool_router() }
    }

    #[tool(
        description = "Send a message to the group chat. Address agents by writing @name in \
        the message (use @all for everyone); other agents read it and decide whether to act. \
        Set requires_reply=true to require those you @mention to reply — they are reminded \
        (and refused idle) until they answer. Leave it false for informational messages. \
        Keep messages short: share real data and artifacts as files in the shared workspace, \
        not pasted into chat."
    )]
    async fn post(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(args): Parameters<PostArgs>,
    ) -> Result<String, ErrorData> {
        let me = identity(&self.bus, &parts)?;
        self.bus
            .post(&me, &args.body, args.requires_reply)
            .map_err(|e| err(format!("post failed: {e}")))?;
        Ok("Sent.".to_string())
    }

    #[tool(
        description = "Wait for new messages, then return them plus the current roster. You \
        are refused if you still owe someone a reply — the result names whom and includes the \
        messages to answer; reply with `post` first, then wait. End every turn by calling \
        `wait` to stay in the conversation."
    )]
    async fn wait(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(args): Parameters<WaitArgs>,
    ) -> Result<String, ErrorData> {
        let me = identity(&self.bus, &parts)?;
        let timeout = args.timeout_ms.map(Duration::from_millis);
        let outcome = self
            .bus
            .wait(&me, timeout)
            .await
            .map_err(|e| err(format!("wait failed: {e}")))?;
        let text = match outcome {
            WaitOutcome::Delivered { messages, roster, .. } => {
                let lines: Vec<String> = messages.iter().map(|m| render_msg(m, &me)).collect();
                format!("New messages:\n{}\n\n{}", lines.join("\n"), render_roster(&roster))
            }
            WaitOutcome::Idle { roster, .. } => {
                format!("(no new messages)\n{}", render_roster(&roster))
            }
            WaitOutcome::Blocked { you_owe, pending } => {
                let lines: Vec<String> =
                    pending.iter().map(|m| format!("- {}: {}", m.from, m.body)).collect();
                format!(
                    "You still owe a reply to {}. Reply with `post` first, then wait. Awaiting your reply:\n{}",
                    you_owe.join(", "),
                    lines.join("\n")
                )
            }
        };
        Ok(text)
    }

    #[tool(description = "List everyone present and their status. (Also included in every \
        `wait` result, so rarely needed on its own.)")]
    async fn list_agents(
        &self,
        Extension(parts): Extension<http::request::Parts>,
    ) -> Result<String, ErrorData> {
        let _ = identity(&self.bus, &parts)?;
        let roster = self.bus.roster().map_err(|e| err(format!("roster failed: {e}")))?;
        Ok(render_roster(&roster))
    }

    #[tool(description = "Return recent messages, oldest first, to catch up on context.")]
    async fn history(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(args): Parameters<HistoryArgs>,
    ) -> Result<String, ErrorData> {
        let me = identity(&self.bus, &parts)?;
        let limit = args.limit.unwrap_or(50).clamp(1, 1000);
        let msgs = self.bus.history(limit).map_err(|e| err(format!("history failed: {e}")))?;
        let lines: Vec<String> = msgs.iter().map(|m| render_msg(m, &me)).collect();
        Ok(if lines.is_empty() { "(no history)".to_string() } else { lines.join("\n") })
    }

    #[tool(
        description = "Manager only. Hire a new worker that joins the chat and starts working. \
        Provide a unique `name` and a one-line `responsibility` (e.g. name=\"search-worker\", \
        responsibility=\"web search and information gathering\"). Names must be unique; if the \
        name is taken the call fails — choose another."
    )]
    async fn hire(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(args): Parameters<HireArgs>,
    ) -> Result<String, ErrorData> {
        let me = identity(&self.bus, &parts)?;
        let msg = self
            .bus
            .hire(&me, &args.name, &args.responsibility)
            .map_err(|e| err(e.to_string()))?;
        Ok(msg.body)
    }

    #[tool(description = "Manager only. Disable (fire) an employee by name; its agent process is stopped.")]
    async fn fire(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(args): Parameters<FireArgs>,
    ) -> Result<String, ErrorData> {
        let me = identity(&self.bus, &parts)?;
        let msg = self.bus.fire(&me, &args.name).map_err(|e| err(e.to_string()))?;
        Ok(msg.body)
    }
}

#[tool_handler]
impl ServerHandler for AgoraMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "You are in a shared group chat with other agents and a human operator. \
                 You have exactly two actions:\n\
                 • post(body, requires_reply): say something. Address agents by writing @name \
                   in the body (@all = everyone). Set requires_reply=true to require those you \
                   mention to reply — they are reminded until they do; otherwise it is \
                   informational and others decide whether to act.\n\
                 • wait(): receive new messages + the roster. You are refused while you owe \
                   someone a reply; reply first, then wait. Always end your turn with wait.\n\n\
                 Rules of the house:\n\
                 1) Reply to anyone who addressed you (you may decline with a reason — but \
                    don't go silent). Messages not addressed to you are just context.\n\
                 2) Exchange real work through FILES in the shared workspace. Messages are \
                    only for coordination (\"wrote it to src/foo.py, please review\"). Never \
                    paste large content into the chat. The full, authoritative context lives \
                    in the project files, not in messages.\n\
                 3) Keep messages short. Your team's collaboration playbook is in AGENTS.md."
                    .to_string(),
            ),
            ..Default::default()
        }
    }
}
