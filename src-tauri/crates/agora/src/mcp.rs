//! The MCP tool surface, served over Streamable HTTP.
//!
//! Identity is bound per-connection via the `x-agent` header (set in each agent's
//! MCP config), not via tool arguments. The agent's whole API is two tools:
//! `post` and `wait`. `list_agents` and bounded `read_history` exist for
//! occasional inspection but should not be called on every turn.

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

/// One agent-facing `wait` tool call spans several short bus wait slices. Kiro
/// caps MCP timeouts at 600 seconds, so the server returns after nine minutes
/// and leaves one minute for transport delivery. In normal idle operation the
/// supervisor cancels this call at eight minutes and sleeps the team first.
pub const MCP_WAIT_MAX_MS: u64 = 540_000;
const MCP_WAIT_MIN_SLICE_MS: u64 = 15_000;
const READ_HISTORY_DEFAULT_LIMIT: i64 = 20;
const READ_HISTORY_MAX_LIMIT: i64 = 100;

#[derive(Clone)]
pub struct AgoraMcp {
    provider: std::sync::Arc<dyn BusProvider>,
    tool_router: ToolRouter<Self>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct PostArgs {
    /// The message. Use @name for one agent or @all when everyone must reply.
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
    /// 540000 milliseconds.
    #[serde(default)]
    pub timeout_ms: Option<u64>,
}

#[derive(Debug, Deserialize, schemars::JsonSchema)]
pub struct ReadHistoryArgs {
    /// How many messages to return (default 20, max 100).
    #[serde(default)]
    pub limit: Option<i64>,
    /// Return messages strictly older than this sequence number. Omit for the
    /// newest page; use the next-page hint from a result to continue backward.
    #[serde(default)]
    pub before_seq: Option<i64>,
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

fn render_history_page(
    bus: &Bus,
    me: &str,
    args: ReadHistoryArgs,
) -> Result<String, ErrorData> {
    if matches!(args.before_seq, Some(seq) if seq <= 0) {
        return Err(err("before_seq must be a positive message sequence number"));
    }
    let limit = args
        .limit
        .unwrap_or(READ_HISTORY_DEFAULT_LIMIT)
        .clamp(1, READ_HISTORY_MAX_LIMIT);
    let mut messages = bus
        .history_before(args.before_seq, limit + 1)
        .map_err(|e| err(format!("read_history failed: {e}")))?;
    let has_older = messages.len() > limit as usize;
    if has_older {
        messages.remove(0);
    }
    if messages.is_empty() {
        return Ok(match args.before_seq {
            Some(seq) => format!("(no team messages before seq {seq})"),
            None => "(no team history)".to_string(),
        });
    }

    let first_seq = messages.first().unwrap().seq;
    let last_seq = messages.last().unwrap().seq;
    let lines: Vec<String> = messages
        .iter()
        .map(|message| format!("[{}] {}", message.seq, render_msg(message, me)))
        .collect();
    let older = if has_older {
        format!(
            "\n\nOlder messages are available. Call `read_history(before_seq={first_seq}, limit={limit})` only if needed."
        )
    } else {
        "\n\nThis is the oldest available page.".to_string()
    };
    Ok(format!(
        "Team history seq {first_seq}-{last_seq} ({} messages):\n{}{}",
        messages.len(),
        lines.join("\n"),
        older
    ))
}

fn is_discoverable_tool(name: &str) -> bool {
    name != "history"
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
        agent to reply. Leave it false for informational messages to named agents."
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
        are ignored, so one call stays parked for up to nine minutes."
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

    #[tool(
        description = "Read a small page of earlier Team messages only when prior context is \
        missing. The newest 20 messages are returned by default, oldest first within the page. \
        To go farther back, pass the `before_seq` from the result's next-page hint. Pages are \
        capped at 100 messages to avoid flooding your context; stop as soon as you have enough."
    )]
    async fn read_history(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(args): Parameters<ReadHistoryArgs>,
    ) -> Result<String, ErrorData> {
        let (bus, me) = identity(&*self.provider, &parts)?;
        render_history_page(&bus, &me, args)
    }

    /// Compatibility for clients that cached the pre-read_history tool name.
    /// It remains callable but is omitted from `list_tools`.
    #[tool(description = "Deprecated compatibility alias for read_history.")]
    async fn history(
        &self,
        Extension(parts): Extension<http::request::Parts>,
        Parameters(args): Parameters<ReadHistoryArgs>,
    ) -> Result<String, ErrorData> {
        let (bus, me) = identity(&*self.provider, &parts)?;
        render_history_page(&bus, &me, args)
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
            ..Default::default()
        }
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParam>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        let mut tools = self.tool_router.list_all();
        tools.retain(|tool| is_discoverable_tool(tool.name.as_ref()));
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
    fn read_history_is_small_and_pages_toward_older_messages() {
        let bus = Bus::new(store::open_in_memory().unwrap(), "main");
        for n in 1..=125 {
            bus.post("human", &format!("message-{n}"), false).unwrap();
        }

        let newest = render_history_page(
            &bus,
            "worker",
            ReadHistoryArgs { limit: None, before_seq: None },
        )
        .unwrap();
        assert!(newest.contains("seq 106-125 (20 messages)"));
        assert!(newest.contains("[106] human: message-106"));
        assert!(!newest.contains("message-105"));
        assert!(newest.contains("read_history(before_seq=106, limit=20)"));

        let older = render_history_page(
            &bus,
            "worker",
            ReadHistoryArgs { limit: Some(1_000), before_seq: Some(106) },
        )
        .unwrap();
        assert!(older.contains("seq 6-105 (100 messages)"));
        assert_eq!(older.lines().filter(|line| line.starts_with('[')).count(), 100);
        assert!(older.contains("read_history(before_seq=6, limit=100)"));

        let oldest = render_history_page(
            &bus,
            "worker",
            ReadHistoryArgs { limit: Some(20), before_seq: Some(6) },
        )
        .unwrap();
        assert!(oldest.contains("seq 1-5 (5 messages)"));
        assert!(oldest.contains("This is the oldest available page."));
        assert!(!oldest.contains("Older messages are available."));
    }

    #[test]
    fn read_history_rejects_invalid_cursor_and_hides_compatibility_alias() {
        let bus = Bus::new(store::open_in_memory().unwrap(), "main");
        assert!(render_history_page(
            &bus,
            "worker",
            ReadHistoryArgs { limit: None, before_seq: Some(0) },
        )
        .is_err());

        let mcp = AgoraMcp::new(std::sync::Arc::new(SingleRoom(bus)));
        let names: Vec<String> = mcp
            .tool_router
            .list_all()
            .into_iter()
            .map(|tool| tool.name.to_string())
            .collect();
        assert!(names.iter().any(|name| name == "read_history"));
        assert!(names.iter().any(|name| name == "history"));
        assert!(is_discoverable_tool("read_history"));
        assert!(!is_discoverable_tool("history"));

        let read_history = mcp
            .tool_router
            .list_all()
            .into_iter()
            .find(|tool| tool.name == "read_history")
            .unwrap();
        let properties = read_history
            .input_schema
            .get("properties")
            .and_then(serde_json::Value::as_object)
            .unwrap();
        assert!(properties.contains_key("limit"));
        assert!(properties.contains_key("before_seq"));
        assert!(mcp.get_info().instructions.is_none());
    }

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

    #[test]
    fn wait_budget_leaves_margin_before_kiro_client_boundary() {
        const KIRO_MCP_TIMEOUT_LIMIT_MS: u64 = 600_000;

        assert_eq!(MCP_WAIT_MAX_MS, 540_000);
        assert!(MCP_WAIT_MAX_MS + 60_000 <= KIRO_MCP_TIMEOUT_LIMIT_MS);
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
