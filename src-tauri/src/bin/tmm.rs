//! tmm — the agent's hands, and the geek's. CLI front for the tmux-mobile
//! project hub (agents-v2): send/read project chat, declare status, list
//! agents and projects. See docs/exec-plans/agents-v2.md §4.4 and
//! docs/design-docs/features/tmm-cli.md.
//!
//! Design contract (owner-set, load-bearing):
//! - FAIL SOFT, NEVER BLOCK: the server is optional. Connection failures are
//!   one line on stderr and exit code 2 within ~2s. No retries, no hangs — an
//!   agent calling `tmm send` inside a hook or a prompt must never stall.
//! - Tiered exit codes (multica convention): 0 ok, 2 network, 3 auth,
//!   4 not found, 5 invalid params / usage.
//! - `--output json` on every read so agents and scripts consume reliably.
//! - Context from env: TMM_PROJECT (tmux session = project id), TMM_AGENT
//!   (window/agent name). Exported by the launcher; overridable by flags.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio_tungstenite::tungstenite::Message;

const EXIT_OK: i32 = 0;
const EXIT_NET: i32 = 2;
const EXIT_AUTH: i32 = 3;
const EXIT_NOT_FOUND: i32 = 4;
const EXIT_USAGE: i32 = 5;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const RPC_TIMEOUT: Duration = Duration::from_secs(10);

const USAGE: &str = r#"tmm — talk to the tmux-mobile project hub

USAGE (agent):
  tmm send <text>                     post to the project chat (@name to address)
  tmm log [--since <ts>] [--limit N] [-f]   read chat; --since is exclusive, -f follows
  tmm status <working|waiting|blocked> [note]   declare what you are doing
  tmm done [summary]                  declare completion

USAGE (human):
  tmm agent list                      agents in this project and their states
  tmm project list                    all projects

CONTEXT:
  --project <session>   which project (default: $TMM_PROJECT)
  --agent <name>        who is speaking (default: $TMM_AGENT, else "human")
  --server <ws://host:port>  (default: $TMM_SERVER, else config.toml)
  --output json         machine-readable output

EXIT CODES: 0 ok · 2 server unreachable · 3 auth · 4 not found · 5 usage
"#;

struct Ctx {
    server: String,
    token: String,
    project: Option<String>,
    agent: Option<String>,
    json: bool,
}

fn fail(code: i32, msg: &str) -> ! {
    eprintln!("tmm: {msg}");
    std::process::exit(code);
}

#[tokio::main(flavor = "current_thread")]
async fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (flags, mut pos) = split_flags(&args);
    if pos.is_empty() || flags.contains_key("help") {
        print!("{USAGE}");
        std::process::exit(if pos.is_empty() { EXIT_USAGE } else { EXIT_OK });
    }

    let cfg = tmux_mobile::config::Config::load();
    let server = flags
        .get("server")
        .cloned()
        .flatten()
        .or_else(|| std::env::var("TMM_SERVER").ok())
        .unwrap_or_else(|| format!("ws://127.0.0.1:{}", cfg.port));
    let ctx = Ctx {
        server,
        token: std::env::var("TMM_TOKEN").ok().unwrap_or(cfg.token),
        project: flags.get("project").cloned().flatten().or_else(|| std::env::var("TMM_PROJECT").ok()).filter(|s| !s.is_empty()),
        agent: flags.get("agent").cloned().flatten().or_else(|| std::env::var("TMM_AGENT").ok()).filter(|s| !s.is_empty()),
        json: flags.get("output").cloned().flatten().as_deref() == Some("json"),
    };

    let cmd = pos.remove(0);
    match (cmd.as_str(), pos) {
        ("send", rest) => {
            let text = rest.join(" ");
            if text.is_empty() {
                fail(EXIT_USAGE, "send needs text: tmm send \"@reviewer 看一下\"");
            }
            let session = need_project(&ctx);
            let from = ctx.agent.clone().unwrap_or_else(|| "human".into());
            let r = rpc(&ctx, "hub_post", json!({ "session": session, "from": from, "body": text })).await;
            if ctx.json {
                println!("{r}");
            } else {
                println!("✓ sent");
            }
        }
        ("log", _) => {
            let session = need_project(&ctx);
            let since = flags.get("since").cloned().flatten().and_then(|s| s.parse::<i64>().ok()).unwrap_or(0);
            let limit = flags.get("limit").cloned().flatten().and_then(|s| s.parse::<i64>().ok()).unwrap_or(100);
            if flags.contains_key("f") || flags.contains_key("follow") {
                follow_log(&ctx, &session, since, limit).await;
            } else {
                let r = rpc(&ctx, "hub_log", json!({ "session": session, "since_ts": since, "limit": limit })).await;
                print_log(&ctx, &r);
            }
        }
        ("status", rest) => {
            let session = need_project(&ctx);
            let agent = need_agent(&ctx);
            let Some(state) = rest.first().cloned() else {
                fail(EXIT_USAGE, "status needs a state: tmm status waiting \"等接口定稿\"");
            };
            let note = rest[1..].join(" ");
            let r = rpc(&ctx, "hub_status", json!({ "session": session, "agent": agent, "state": state, "note": note })).await;
            if ctx.json { println!("{r}"); } else { println!("✓ {state}"); }
        }
        ("done", rest) => {
            let session = need_project(&ctx);
            let agent = need_agent(&ctx);
            let summary = rest.join(" ");
            let r = rpc(&ctx, "hub_done", json!({ "session": session, "agent": agent, "summary": summary })).await;
            if ctx.json { println!("{r}"); } else { println!("✓ done"); }
        }
        ("agent", rest) if rest.first().map(String::as_str) == Some("list") => {
            let session = need_project(&ctx);
            let r = rpc(&ctx, "hub_agents", json!({ "session": session })).await;
            if ctx.json {
                println!("{r}");
            } else {
                let empty = Vec::new();
                let rows = r.get("agents").and_then(|a| a.as_array()).unwrap_or(&empty);
                for a in rows {
                    let s = |k: &str| a.get(k).and_then(|v| v.as_str()).unwrap_or("");
                    let w = a.get("window").and_then(|v| v.as_u64()).unwrap_or(0);
                    let agent = a.get("agent").and_then(|v| v.as_str());
                    let detail = s("detail");
                    let tail = if detail.is_empty() { String::new() } else { format!(" — {detail}") };
                    match agent {
                        Some(b) => println!("{w}: {} [{b}] {}{tail}", s("name"), s("state")),
                        None => println!("{w}: {} (shell)", s("name")),
                    }
                }
            }
        }
        ("project", rest) if rest.first().map(String::as_str) == Some("list") => {
            let r = rpc(&ctx, "project_list", json!({})).await;
            if ctx.json {
                println!("{r}");
            } else {
                let empty = Vec::new();
                let rows = r.get("projects").and_then(|a| a.as_array()).unwrap_or(&empty);
                for p in rows {
                    let live = p.get("live").and_then(|v| v.as_bool()).unwrap_or(false);
                    let proj = p.get("project").cloned().unwrap_or(Value::Null);
                    let s = |k: &str| proj.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
                    println!("{} {} ({})", if live { "●" } else { "○" }, s("session"), s("path"));
                }
            }
        }
        _ => {
            eprint!("{USAGE}");
            std::process::exit(EXIT_USAGE);
        }
    }
}

fn need_project(ctx: &Ctx) -> String {
    ctx.project.clone().unwrap_or_else(|| {
        fail(EXIT_USAGE, "no project: set $TMM_PROJECT or pass --project <session>")
    })
}

fn need_agent(ctx: &Ctx) -> String {
    ctx.agent.clone().unwrap_or_else(|| {
        fail(EXIT_USAGE, "no agent identity: set $TMM_AGENT or pass --agent <name>")
    })
}

/// `--flag value` / `--flag` / `-f` → map; the rest are positionals.
/// Flags known to take a value consume the next arg; boolean flags don't.
fn split_flags(args: &[String]) -> (std::collections::HashMap<String, Option<String>>, Vec<String>) {
    const VALUED: &[&str] = &["project", "agent", "server", "output", "since", "limit"];
    let mut flags = std::collections::HashMap::new();
    let mut pos = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(name) = a.strip_prefix("--") {
            if VALUED.contains(&name) && i + 1 < args.len() {
                flags.insert(name.to_string(), Some(args[i + 1].clone()));
                i += 2;
                continue;
            }
            flags.insert(name.to_string(), None);
        } else if let Some(name) = a.strip_prefix('-') {
            flags.insert(name.to_string(), None);
        } else {
            pos.push(a.clone());
        }
        i += 1;
    }
    (flags, pos)
}

/// One connect → auth → call → close round trip. All failure modes funnel to
/// the tiered exit codes; success returns the RPC result value.
async fn rpc(ctx: &Ctx, method: &str, params: Value) -> Value {
    match try_rpc(ctx, method, params).await {
        Ok(v) => v,
        Err((code, msg)) => fail(code, &msg),
    }
}

async fn try_rpc(ctx: &Ctx, method: &str, params: Value) -> Result<Value, (i32, String)> {
    let connect = tokio_tungstenite::connect_async(&ctx.server);
    let (mut ws, _) = tokio::time::timeout(CONNECT_TIMEOUT, connect)
        .await
        .map_err(|_| (EXIT_NET, format!("server not reachable at {} (2s timeout) — is tmux-mobile running?", ctx.server)))?
        .map_err(|e| (EXIT_NET, format!("server not reachable at {}: {e}", ctx.server)))?;

    // Plain token auth (loopback default). id 1 = auth, id 2 = the call.
    let auth = json!({ "id": 1, "method": "auth", "params": { "token": ctx.token } });
    ws.send(Message::Text(auth.to_string().into()))
        .await
        .map_err(|e| (EXIT_NET, format!("send failed: {e}")))?;
    let auth_reply = read_reply(&mut ws, 1).await?;
    if auth_reply.get("error").is_some() {
        return Err((EXIT_AUTH, "auth rejected — check token in config.toml or $TMM_TOKEN".into()));
    }

    let call = json!({ "id": 2, "method": method, "params": params });
    ws.send(Message::Text(call.to_string().into()))
        .await
        .map_err(|e| (EXIT_NET, format!("send failed: {e}")))?;
    let reply = read_reply(&mut ws, 2).await?;
    let _ = ws.close(None).await;

    if let Some(err) = reply.get("error") {
        let code = err.get("code").and_then(|c| c.as_i64()).unwrap_or(0);
        let msg = err.get("message").and_then(|m| m.as_str()).unwrap_or("unknown error").to_string();
        // -32601 method-not-found → the hub isn't on this server (mobile, or
        // old build). -32602 covers "no window named X" style lookups.
        let exit = match code {
            -32601 => EXIT_NOT_FOUND,
            -32602 => EXIT_USAGE,
            _ => 1,
        };
        return Err((exit, msg));
    }
    Ok(reply.get("result").cloned().unwrap_or(Value::Null))
}

async fn read_reply(
    ws: &mut (impl StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin),
    want_id: u64,
) -> Result<Value, (i32, String)> {
    let deadline = tokio::time::Instant::now() + RPC_TIMEOUT;
    loop {
        let msg = tokio::time::timeout_at(deadline, ws.next())
            .await
            .map_err(|_| (EXIT_NET, "rpc timeout".to_string()))?
            .ok_or((EXIT_NET, "connection closed".to_string()))?
            .map_err(|e| (EXIT_NET, format!("recv failed: {e}")))?;
        if let Message::Text(text) = msg {
            if let Ok(v) = serde_json::from_str::<Value>(&text) {
                // Skip pushes (no id) and other ids — we only await ours.
                if v.get("id").and_then(|i| i.as_u64()) == Some(want_id) {
                    return Ok(v);
                }
            }
        }
    }
}

fn print_log(ctx: &Ctx, r: &Value) {
    if ctx.json {
        println!("{r}");
        return;
    }
    let empty = Vec::new();
    for m in r.get("messages").and_then(|m| m.as_array()).unwrap_or(&empty) {
        let from = m.get("from").and_then(|v| v.as_str()).unwrap_or("?");
        let body = m.get("body").and_then(|v| v.as_str()).unwrap_or("");
        let ts = m.get("ts").and_then(|v| v.as_i64()).unwrap_or(0);
        println!("[{ts}] {from}: {body}");
    }
}

/// `-f`: poll with the since cursor. Polling (not push) keeps the CLI a plain
/// request/response client — no long-lived socket state to get wrong, and a
/// dead server surfaces as one error line, not a silent stall.
async fn follow_log(ctx: &Ctx, session: &str, mut since: i64, limit: i64) {
    loop {
        let r = try_rpc(ctx, "hub_log", json!({ "session": session, "since_ts": since, "limit": limit })).await;
        match r {
            Ok(v) => {
                let empty = Vec::new();
                let msgs = v.get("messages").and_then(|m| m.as_array()).unwrap_or(&empty);
                for m in msgs {
                    let ts = m.get("ts").and_then(|t| t.as_i64()).unwrap_or(0);
                    if ts > since {
                        since = ts;
                    }
                    if ctx.json {
                        println!("{m}");
                    } else {
                        let from = m.get("from").and_then(|x| x.as_str()).unwrap_or("?");
                        let body = m.get("body").and_then(|x| x.as_str()).unwrap_or("");
                        println!("[{ts}] {from}: {body}");
                    }
                }
            }
            Err((code, msg)) => fail(code, &msg),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
