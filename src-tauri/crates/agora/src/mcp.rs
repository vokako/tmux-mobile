//! The MCP tool surface, served over Streamable HTTP.
//!
//! Identity is bound per-connection via the `x-agent` header (set in each agent's
//! MCP config), not via tool arguments. The agent's whole API is two tools:
//! `post` and `wait`. `list_agents`/`history` exist for occasional catch-up but the
//! roster is also returned by `wait`, so they rarely need to be called.

use crate::bus::{Bus, BusProvider, WaitOutcome};
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::tool::{Extension, Parameters, ToolCallContext};
use rmcp::model::{
    CallToolRequestParam, CallToolResult, ListToolsResult, PaginatedRequestParam,
    ServerCapabilities, ServerInfo,
};
use rmcp::service::RequestContext;
use rmcp::{tool, tool_router, ErrorData, RoleServer, ServerHandler};
use serde::Deserialize;
use std::future::Future;
use std::time::{Duration, Instant};

/// One agent-facing `wait` tool call spans several short bus wait slices. This
/// stays below the supervisor's five-minute idle-sleep threshold, so an idle
/// team causes at most one empty model turn before the next call is cancelled.
pub const MCP_WAIT_MAX_MS: u64 = 240_000;
const MCP_WAIT_MIN_SLICE_MS: u64 = 15_000;

#[derive(Clone)]
pub struct AgoraMcp {
    provider: std::sync::Arc<dyn BusProvider>,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PostArgs {
    /// The message. Use @name for one agent or @all when everyone must reply.
    /// Keep it short; share real artifacts as workspace files.
    pub body: String,
    /// Set true to require named agents you @mention to reply: each is reminded —
    /// and refused idle — until they answer you. @all always requires every other
    /// agent to reply, even when this flag is false. Leave false (default) for
    /// informational messages to named agents that need no reply.
    #[serde(default)]
    pub requires_reply: bool,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct WaitArgs {
    /// Internal polling slice in milliseconds (range 15000-50000, default
    /// 50000). Empty slices are ignored; one tool call remains parked for up to
    /// 240000 milliseconds.
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

/// Resolve the target room from the `x-room` header (or the provider default),
/// then the caller from `x-agent`, register them, and return (bus, name).
fn identity(
    provider: &dyn BusProvider,
    parts: &http::request::Parts,
) -> Result<(Bus, String), ErrorData> {
    let room = parts
        .headers
        .get("x-room")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| provider.default_room());
    let bus = provider
        .bus_for(&room)
        .ok_or_else(|| err(format!("unknown room '{room}'")))?;
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
    Ok((bus, name))
}

/// Does `name` carry the `manage` flag in its employee spec for `room`? Only
/// managers may see or call hire/fire — gated here at the server so a
/// non-manager's tools/list never even includes them, regardless of which CLI
/// backend the agent runs. Unknown/unseeded callers (and specs without `manage`)
/// are NOT allowed.
fn agent_can_manage(provider: &dyn BusProvider, room: &str, name: &str) -> bool {
    provider
        .bus_for(room)
        .and_then(|b| b.employees().ok())
        .unwrap_or_default()
        .into_iter()
        .find(|e| e.name.eq_ignore_ascii_case(name))
        .and_then(|e| e.spec.get("manage").and_then(|v| v.as_bool()))
        .unwrap_or(false)
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

async fn wait_across_idle_slices(
    bus: &Bus,
    agent: &str,
    slice: Duration,
    total: Duration,
) -> anyhow::Result<WaitOutcome> {
    let deadline = Instant::now() + total;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        let outcome = bus.wait(agent, Some(slice.min(remaining))).await?;
        match outcome {
            WaitOutcome::Idle { .. } if Instant::now() < deadline => continue,
            other => return Ok(other),
        }
    }
}

fn wait_slice(timeout_ms: Option<u64>) -> Duration {
    Duration::from_millis(
        timeout_ms
            .unwrap_or(crate::bus::MAX_WAIT_SLICE_MS)
            .clamp(MCP_WAIT_MIN_SLICE_MS, crate::bus::MAX_WAIT_SLICE_MS),
    )
}

#[tool_router]
impl AgoraMcp {
    pub fn new(provider: std::sync::Arc<dyn BusProvider>) -> Self {
        Self { provider, tool_router: Self::tool_router() }
    }

    #[tool(
        description = "Send a message to the group chat. Address agents by writing @name in \
        the message (use @all for everyone); other agents read it and decide whether to act. \
        Set requires_reply=true to require named agents you @mention to reply — they are \
        reminded (and refused idle) until they answer. @all always requires every other \
        agent to reply. Leave it false for informational messages to named agents. \
        Keep messages short: share real data and artifacts as files in the shared workspace, \
        not pasted into chat."
    )]
    async fn post(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(args): Parameters<PostArgs>,
    ) -> Result<String, ErrorData> {
        let (bus, me) = identity(&*self.provider, &parts)?;
        bus.post(&me, &args.body, args.requires_reply)
            .map_err(|e| err(format!("post failed: {e}")))?;
        Ok("Sent.".to_string())
    }

    #[tool(
        description = "Wait for new messages, then return them plus the current roster. You \
        are refused if you still owe someone a reply — the result names whom and includes the \
        messages to answer; reply with `post` first, then wait. Empty internal polling slices \
        are ignored, so one call stays parked for up to four minutes. End every turn by calling \
        `wait` to stay in the conversation."
    )]
    async fn wait(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(args): Parameters<WaitArgs>,
    ) -> Result<String, ErrorData> {
        let (bus, me) = identity(&*self.provider, &parts)?;
        let outcome = wait_across_idle_slices(
            &bus,
            &me,
            wait_slice(args.timeout_ms),
            Duration::from_millis(MCP_WAIT_MAX_MS),
        )
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
        let (bus, _) = identity(&*self.provider, &parts)?;
        let roster = bus.roster().map_err(|e| err(format!("roster failed: {e}")))?;
        Ok(render_roster(&roster))
    }

    #[tool(description = "Return recent messages, oldest first, to catch up on context.")]
    async fn history(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(args): Parameters<HistoryArgs>,
    ) -> Result<String, ErrorData> {
        let (bus, me) = identity(&*self.provider, &parts)?;
        let limit = args.limit.unwrap_or(50).clamp(1, 1000);
        let msgs = bus.history(limit).map_err(|e| err(format!("history failed: {e}")))?;
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
        let (bus, me) = identity(&*self.provider, &parts)?;
        let msg = bus
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
        let (bus, me) = identity(&*self.provider, &parts)?;
        let msg = bus.fire(&me, &args.name).map_err(|e| err(e.to_string()))?;
        Ok(msg.body)
    }
}

impl AgoraMcp {
    /// Whether the calling agent (resolved from the x-agent/x-room headers) may
    /// manage — i.e. carries `manage` in its employee spec. Gates hire/fire.
    fn caller_can_manage(&self, ctx: &RequestContext<RoleServer>) -> bool {
        let Some(parts) = ctx.extensions.get::<http::request::Parts>() else { return false };
        let Some(name) = parts
            .headers
            .get("x-agent")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
        else {
            return false;
        };
        let room = parts
            .headers
            .get("x-room")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| self.provider.default_room());
        agent_can_manage(&*self.provider, &room, name)
    }
}

impl ServerHandler for AgoraMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            instructions: Some(
                "You are in a shared group chat with other agents and a human operator. \
                 You have exactly two actions:\n\
                 • post(body, requires_reply): say something. Address agents by writing @name \
                   in the body. Set requires_reply=true to require named agents to reply. \
                   @all addresses everyone and always requires every other agent to reply, \
                   regardless of the flag; otherwise messages are informational.\n\
                 • wait(): receive new messages + the roster. You are refused while you owe \
                   someone a reply; reply first, then wait. Always end your turn with wait.\n\n\
                 Rules of the house:\n\
                 1) Reply to anyone who addressed you. @all addresses you exactly like your \
                    own name (you may decline with a reason — but don't go silent). Messages \
                    not addressed to you are just context.\n\
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

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut tools = self.tool_router.list_all();
        if !self.caller_can_manage(&context) {
            tools.retain(|t| t.name.as_ref() != "hire" && t.name.as_ref() != "fire");
        }
        Ok(ListToolsResult::with_all_items(tools))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParam,
        context: RequestContext<RoleServer>,
    ) -> Result<CallToolResult, ErrorData> {
        if matches!(request.name.as_ref(), "hire" | "fire") && !self.caller_can_manage(&context) {
            return Err(err("hire and fire are disabled for this agent"));
        }
        let tcc = ToolCallContext::new(self, request, context);
        self.tool_router.call(tcc).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bus::{Bus, SingleRoom};
    use crate::store;

    #[test]
    fn only_manage_flagged_agents_can_hire_fire() {
        // hire/fire visibility/permission keys off the employee `manage` flag, so
        // a template with no manage=true agent exposes neither tool to anyone.
        let bus = Bus::new(store::open_in_memory().unwrap(), "main");
        bus.seed_employee("boss", &serde_json::json!({ "manage": true })).unwrap();
        bus.seed_employee("worker", &serde_json::json!({ "manage": false })).unwrap();
        let provider = SingleRoom(bus);
        assert!(agent_can_manage(&provider, "main", "boss"), "manager may hire/fire");
        assert!(!agent_can_manage(&provider, "main", "worker"), "worker may not");
        assert!(!agent_can_manage(&provider, "main", "ghost"), "unseeded caller may not");
    }

    #[tokio::test]
    async fn wait_coalesces_idle_slices_until_total_budget() {
        let bus = Bus::new(store::open_in_memory().unwrap(), "main");
        bus.join("worker", None).unwrap();
        let started = Instant::now();

        let outcome = wait_across_idle_slices(
            &bus,
            "worker",
            Duration::from_millis(15),
            Duration::from_millis(70),
        )
        .await
        .unwrap();

        assert!(matches!(outcome, WaitOutcome::Idle { .. }));
        assert!(
            started.elapsed() >= Duration::from_millis(60),
            "an internal idle slice returned to the caller too early"
        );
    }

    #[test]
    fn wait_slice_is_bounded_away_from_busy_polling() {
        assert_eq!(wait_slice(None), Duration::from_millis(50_000));
        assert_eq!(wait_slice(Some(1)), Duration::from_millis(15_000));
        assert_eq!(wait_slice(Some(500_000)), Duration::from_millis(50_000));
    }

    #[tokio::test]
    async fn wait_delivers_message_after_an_ignored_idle_slice() {
        let bus = Bus::new(store::open_in_memory().unwrap(), "main");
        bus.join("worker", None).unwrap();
        let sender = bus.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(45)).await;
            sender.post("human", "@worker continue", false).unwrap();
        });

        let outcome = wait_across_idle_slices(
            &bus,
            "worker",
            Duration::from_millis(20),
            Duration::from_millis(200),
        )
        .await
        .unwrap();

        match outcome {
            WaitOutcome::Delivered { messages, .. } => {
                assert_eq!(messages.last().unwrap().body, "@worker continue");
            }
            other => panic!("expected delivery from the original wait call, got {other:?}"),
        }
    }
}
