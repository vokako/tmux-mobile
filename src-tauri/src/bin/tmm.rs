//! tmm — the agent's hands, and the geek's. CLI front for the tmux-mobile
//! project hub (agents-v2): send/read project chat, declare status, list
//! agents and projects. See docs/exec-plans/agents-v2.md §4.4 and
//! docs/design-docs/features/tmm-cli.md.
//!
//! Design contract (owner-set, load-bearing):
//! - FAIL SOFT, NEVER BLOCK: the server is optional. Connection failures are
//!   one line on stderr and exit code 2 within ~2s. No retries, no hangs — an
//!   agent calling `tmm send` inside a hook or a prompt must never stall.
//! - Tiered exit codes (multica convention): 0 ok, 1 local/tmux failure,
//!   2 network, 3 auth, 4 not found, 5 invalid params / usage.
//! - `--output json` on every read so agents and scripts consume reliably.
//! - Context from env: TMM_PROJECT (tmux session = project id), TMM_AGENT
//!   (window/agent name). Exported by the launcher; overridable by flags.
//! - `tmm task *` is the one subtree that is purely LOCAL (tmux only). It must
//!   keep working when no server is running, because what an agent most often
//!   wants to background is the server itself. It can never exit 2.

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tmux_mobile::tasks;
use tokio_tungstenite::tungstenite::Message;

const EXIT_OK: i32 = 0;
const EXIT_ERR: i32 = 1;
const EXIT_NET: i32 = 2;
const EXIT_AUTH: i32 = 3;
const EXIT_NOT_FOUND: i32 = 4;
const EXIT_USAGE: i32 = 5;

const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const RPC_TIMEOUT: Duration = Duration::from_secs(10);

const USAGE: &str = r#"tmm — talk to the tmux-mobile project hub

USAGE (agent):
  tmm send <text>                     post to the project chat (@name to address)
                    [--image <path|url>]   attach an image by REFERENCE (repeatable);
                                      a local path is resolved by the client
  tmm log [--since <ts>] [--limit N] [-f]   read chat; --since is exclusive, -f follows
  tmm status <working|waiting|blocked> [note]   declare what you are doing
  tmm done [summary]                  declare completion
  tmm spawn <agent> [--brief <text>]  spawn a registry agent into this project

USAGE (background tasks — LOCAL tmux only, no server needed, never exits 2):
  tmm task start <name> -- <cmd...>   run <cmd> detached in its own tmux window
                    [--session <s>]   where to put it (default: the session you
                                      are in, else "tmm-tasks")
                    [--replace]       take over a name a live task holds
  tmm task list                       every task, in every session, + state
  tmm task status <name>              running | exited:<code>  (exit 4 if gone)
  tmm task logs <name> [--limit N] [--grep <text>]   default 50 lines, from the end
  tmm task stop <name>                C-c, then TERM, then KILL; keeps the log
  tmm task rm <name>                  close a finished task's window

USAGE (human or agent — self-management):
  tmm agent list                      agents in this project and their states
  tmm agent interrupt <name>          cancel the turn it is running (Escape into its pane)
  tmm agent stop|restart <name>       stop it, or bring it back resuming its conversation
  tmm agent remove <name>             eject it: stop + forget its slot + delete its home
  tmm project list                    all projects
  tmm project create <path> [--name n] [--session s] [--with-agent kiro|claude|codex]
  tmm project up <session>            bring a project's tmux session up
  tmm project rename <session> --name "New name"   rename the label (session unchanged)
  tmm project delete <session>        forget the project and delete its agents' homes
  tmm project down <session>          kill the session, keep the declaration
  tmm project archive <session>       remove from projects (session survives)
  tmm registry list                   centrally-defined agents
  tmm registry save --name <n> --backend <kiro|claude|codex> [--system <text>]
                    [--model m] [--skills a,b] [--mcp <json>] [--can-hire]
  tmm registry delete <name>
  tmm skills list|delete|refresh <name>   app-managed skill store
  tmm skills save --name <n> --source <abs dir|github url>  (imports the files)
  tmm mcp list|delete <name>          central MCP server defs
  tmm mcp save --name <n> --def '<json>' 

CONTEXT:
  --project <session>   which project (default: $TMM_PROJECT)
  --agent <name>        who is speaking (default: $TMM_AGENT, else "human")
  --server <ws://host:port>  (default: $TMM_SERVER, else config.toml)
  --output json         machine-readable output

EXIT CODES: 0 ok · 1 local/tmux failure · 2 server unreachable · 3 auth
            4 not found · 5 usage
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
    // Everything after a bare `--` is a command to be run verbatim, so it must
    // never reach the flag parser — `-- cargo build --release` would otherwise
    // lose its `--release` to `flags`.
    let (head, cmdv) = match args.iter().position(|a| a == "--") {
        Some(i) => (&args[..i], args[i + 1..].to_vec()),
        None => (&args[..], Vec::new()),
    };
    let (flags, mut pos, repeated) = split_flags(head);
    if pos.is_empty() || flags.contains_key("help") {
        print!("{USAGE}");
        std::process::exit(if pos.is_empty() { EXIT_USAGE } else { EXIT_OK });
    }

    let json = flags.get("output").cloned().flatten().as_deref() == Some("json");

    // Local tmux subtree, dispatched before anything else: `Config::load()`
    // below seeds a token / machine id / team defaults into config.toml, and a
    // command that only talks to tmux has no business doing that.
    if pos[0] == "task" {
        cmd_task(&pos[1..], &cmdv, &flags, json);
        return;
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
        json,
    };

    let cmd = pos.remove(0);
    match (cmd.as_str(), pos) {
        ("send", rest) => {
            let text = rest.join(" ");
            // `--image` may repeat. An image is sent as a REFERENCE (an http(s)
            // URL, or a path on the machine the server runs on) appended as
            // markdown; the client resolves a local path through the file
            // service when it renders. Nothing is ever base64'd into a chat
            // message — the room is a log, not a blob store.
            let images: Vec<String> = repeated
                .iter()
                .filter(|(k, _)| k == "image")
                .map(|(_, v)| absolutize_ref(v))
                .collect();
            if text.is_empty() && images.is_empty() {
                fail(EXIT_USAGE, "send needs text: tmm send \"@reviewer 看一下\" [--image shot.png]");
            }
            let mut body = text;
            for src in &images {
                if !body.is_empty() {
                    body.push('\n');
                }
                body.push_str(&format!("![]({src})"));
            }
            let session = need_project(&ctx);
            let from = ctx.agent.clone().unwrap_or_else(|| "human".into());
            let r = rpc(&ctx, "hub_post", json!({ "session": session, "from": from, "body": body })).await;
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
        ("spawn", rest) => {
            let session = need_project(&ctx);
            let Some(agent) = rest.first().cloned() else {
                fail(EXIT_USAGE, "spawn needs a registry agent name: tmm spawn reviewer --brief \"...\"");
            };
            let brief = flags.get("brief").cloned().flatten().unwrap_or_default();
            let by = ctx.agent.clone().unwrap_or_default();
            let r = rpc(&ctx, "hub_spawn", json!({ "session": session, "agent": agent, "brief": brief, "by": by })).await;
            if ctx.json {
                println!("{r}");
            } else {
                let win = r.get("window_name").and_then(|v| v.as_str()).unwrap_or(&agent);
                println!("✓ spawned {win}");
            }
        }
        ("registry", rest) if rest.first().map(String::as_str) == Some("list") => {
            let r = rpc(&ctx, "registry_list", json!({})).await;
            if ctx.json {
                println!("{r}");
            } else {
                let empty = Vec::new();
                for a in r.get("agents").and_then(|v| v.as_array()).unwrap_or(&empty) {
                    let s = |k: &str| a.get(k).and_then(|v| v.as_str()).unwrap_or("");
                    let hire = if a.get("can_hire").and_then(|v| v.as_bool()).unwrap_or(false) { " (can hire)" } else { "" };
                    println!("{} [{}]{} — {}", s("name"), s("backend"), hire, s("system").chars().take(60).collect::<String>());
                }
            }
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
        // Everything the chat UI can do to ONE agent, so an agent can do it too
        // (owner: parity between the buttons and the CLI). `remove` is the
        // eject button — stop + forget the slot + delete the isolated home.
        ("agent", rest)
            if matches!(rest.first().map(String::as_str), Some("stop" | "restart" | "remove" | "interrupt")) =>
        {
            let action = rest[0].clone();
            let session = need_project(&ctx);
            let Some(name) = rest.get(1).cloned() else {
                fail(EXIT_USAGE, &format!("agent {action} needs a name: tmm agent {action} <name>"));
            };
            let method = match action.as_str() {
                "stop" => "hub_agent_stop",
                "restart" => "hub_agent_restart",
                "remove" => "hub_agent_remove",
                _ => "hub_agent_interrupt",
            };
            let r = rpc(&ctx, method, json!({ "session": session, "agent": name })).await;
            if ctx.json { println!("{r}"); } else { println!("✓ {action} {name}"); }
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
        // ---- self-management: the app manages itself through the same CLI
        // its agents use. An agent already holds a shell (it can run tmux or
        // edit files directly), so these commands ADD no authority — they
        // turn abilities it already has into a first-class, documented
        // interface. can_hire stays a resource gate on spawn only.
        ("project", rest) if rest.first().map(String::as_str) == Some("create") => {
            let Some(path) = rest.get(1).cloned() else {
                fail(EXIT_USAGE, "project create needs a path: tmm project create /path/to/dir [--name n] [--session s] [--with-agent kiro|claude|codex]");
            };
            let mut params = json!({ "path": path });
            for (flag, key) in [("name", "name"), ("session", "session"), ("with-agent", "agent")] {
                if let Some(Some(v)) = flags.get(flag) {
                    params[key] = json!(v);
                }
            }
            let r = rpc(&ctx, "project_create", params).await;
            if ctx.json {
                println!("{r}");
            } else {
                let proj = r.get("project").unwrap_or(&r);
                println!("✓ project {} (id {})",
                    proj.get("session").and_then(|v| v.as_str()).unwrap_or("?"),
                    proj.get("id").and_then(|v| v.as_str()).unwrap_or("?"));
            }
        }
        ("project", rest) if rest.first().map(String::as_str) == Some("rename") => {
            // Renames the LABEL. The session is the project's identity (and the
            // chat room's key), so it is deliberately untouched.
            let Some(name) = rest.get(1).cloned() else {
                fail(EXIT_USAGE, "project rename needs a session and a new name: tmm project rename <session> --name \"New name\"");
            };
            let Some(Some(new_name)) = flags.get("name").cloned() else {
                fail(EXIT_USAGE, "project rename needs --name: tmm project rename <session> --name \"New name\"");
            };
            let id = resolve_project_id(&ctx, &name).await;
            let r = rpc(&ctx, "project_rename", json!({ "id": id, "name": new_name })).await;
            if ctx.json { println!("{r}"); } else { println!("✓ renamed {name} → {new_name}"); }
        }
        ("project", rest) if matches!(rest.first().map(String::as_str), Some("up" | "down" | "archive" | "delete")) => {
            let action = rest[0].clone();
            let Some(name) = rest.get(1).cloned() else {
                fail(EXIT_USAGE, &format!("project {action} needs a session name: tmm project {action} <session>"));
            };
            let id = resolve_project_id(&ctx, &name).await;
            let method = match action.as_str() {
                "up" => "project_up",
                "down" => "project_down",
                "delete" => "project_delete",
                _ => "project_archive",
            };
            let r = rpc(&ctx, method, json!({ "id": id })).await;
            if ctx.json { println!("{r}"); } else { println!("✓ {action} {name}"); }
        }
        ("registry", rest) if rest.first().map(String::as_str) == Some("save") => {
            // Self-evolution: an agent can define NEW agents (or refine
            // existing ones) and then spawn them.
            let Some(Some(name)) = flags.get("name").cloned() else {
                fail(EXIT_USAGE, "registry save needs --name and --backend (and usually --system):\n  tmm registry save --name tester --backend kiro --system \"You run the test suite …\" [--skills a,b] [--mcp '<json array>'] [--can-hire]");
            };
            let backend = flags.get("backend").cloned().flatten().unwrap_or_default();
            // --skills is a comma list of refs; --mcp is a raw JSON array
            // (server-validated either way).
            let skills: Vec<String> = flags
                .get("skills").cloned().flatten()
                .map(|s| s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
                .unwrap_or_default();
            let def = json!({
                "name": name,
                "backend": backend,
                "model": flags.get("model").cloned().flatten().unwrap_or_default(),
                "system": flags.get("system").cloned().flatten().unwrap_or_default(),
                "skills": serde_json::to_string(&skills).unwrap(),
                "mcp": flags.get("mcp").cloned().flatten().unwrap_or_else(|| "[]".into()),
                "can_hire": flags.contains_key("can-hire"),
            });
            let r = rpc(&ctx, "registry_save", json!({ "def": def })).await;
            if ctx.json { println!("{r}"); } else { println!("✓ saved {name}"); }
        }
        ("skills", rest) if rest.first().map(String::as_str) == Some("list") => {
            let r = rpc(&ctx, "skills_list", json!({})).await;
            if ctx.json { println!("{r}"); } else {
                let empty = Vec::new();
                for sk in r.get("skills").and_then(|v| v.as_array()).unwrap_or(&empty) {
                    let s = |k: &str| sk.get(k).and_then(|v| v.as_str()).unwrap_or("");
                    println!("{} ← {} {}", s("name"), s("source"), s("description"));
                }
            }
        }
        ("skills", rest) if rest.first().map(String::as_str) == Some("save") => {
            let source = flags.get("source").cloned().flatten().or_else(|| flags.get("ref").cloned().flatten());
            let (Some(Some(name)), Some(source)) = (flags.get("name").cloned(), source) else {
                fail(EXIT_USAGE, "skills save needs --name and --source: tmm skills save --name git-review --source https://github.com/org/repo/tree/main/skills/git-review");
            };
            let def = json!({ "name": name, "source": source, "description": flags.get("description").cloned().flatten().unwrap_or_default() });
            let r = rpc(&ctx, "skills_save", json!({ "def": def })).await;
            if ctx.json { println!("{r}"); } else { println!("✓ imported {name}"); }
        }
        ("skills", rest) if rest.first().map(String::as_str) == Some("refresh") => {
            let Some(name) = rest.get(1).cloned() else { fail(EXIT_USAGE, "skills refresh <name>"); };
            let r = rpc(&ctx, "skills_refresh", json!({ "name": name })).await;
            if ctx.json { println!("{r}"); } else { println!("✓ refreshed {name}"); }
        }
        ("skills", rest) if rest.first().map(String::as_str) == Some("delete") => {
            let Some(name) = rest.get(1).cloned() else { fail(EXIT_USAGE, "skills delete <name>"); };
            let r = rpc(&ctx, "skills_delete", json!({ "name": name })).await;
            if ctx.json { println!("{r}"); } else { println!("✓ deleted {name}"); }
        }
        ("mcp", rest) if rest.first().map(String::as_str) == Some("list") => {
            let r = rpc(&ctx, "mcp_list", json!({})).await;
            if ctx.json { println!("{r}"); } else {
                let empty = Vec::new();
                for m in r.get("mcp").and_then(|v| v.as_array()).unwrap_or(&empty) {
                    let s = |k: &str| m.get(k).and_then(|v| v.as_str()).unwrap_or("");
                    println!("{}: {}", s("name"), s("def"));
                }
            }
        }
        ("mcp", rest) if rest.first().map(String::as_str) == Some("save") => {
            let (Some(Some(name)), Some(Some(defv))) = (flags.get("name").cloned(), flags.get("def").cloned()) else {
                fail(EXIT_USAGE, "mcp save needs --name and --def '<json>': tmm mcp save --name files --def '{\"command\":\"mcp-files\"}'");
            };
            let r = rpc(&ctx, "mcp_save", json!({ "def": { "name": name, "def": defv } })).await;
            if ctx.json { println!("{r}"); } else { println!("✓ saved {name}"); }
        }
        ("mcp", rest) if rest.first().map(String::as_str) == Some("delete") => {
            let Some(name) = rest.get(1).cloned() else { fail(EXIT_USAGE, "mcp delete <name>"); };
            let r = rpc(&ctx, "mcp_delete", json!({ "name": name })).await;
            if ctx.json { println!("{r}"); } else { println!("✓ deleted {name}"); }
        }
        ("registry", rest) if rest.first().map(String::as_str) == Some("delete") => {
            let Some(name) = rest.get(1).cloned() else {
                fail(EXIT_USAGE, "registry delete needs a name: tmm registry delete <name>");
            };
            let r = rpc(&ctx, "registry_delete", json!({ "name": name })).await;
            if ctx.json { println!("{r}"); } else { println!("✓ deleted {name}"); }
        }
        _ => {
            eprint!("{USAGE}");
            std::process::exit(EXIT_USAGE);
        }
    }
}

type Flags = std::collections::HashMap<String, Option<String>>;

/// `tmm task …` — background tasks as tmux windows. The only subtree that
/// opens no socket and reads no config, so it stays usable when the hub is
/// down. Errors map onto the tiered codes via `task_fail`; 2 is unreachable.
fn cmd_task(rest: &[String], cmdv: &[String], flags: &Flags, json: bool) {
    let verb = rest.first().map(String::as_str).unwrap_or("");
    let arg = rest.get(1).map(String::as_str);
    match verb {
        "start" => {
            let Some(name) = arg else {
                fail(EXIT_USAGE, "task start needs a name: tmm task start dev -- npm run dev");
            };
            if cmdv.is_empty() {
                fail(EXIT_USAGE, "task start needs a command after `--`: tmm task start dev -- npm run dev");
            }
            let session = flags.get("session").cloned().flatten();
            let t = tasks::start(name, cmdv, session.as_deref(), flags.contains_key("replace"))
                .unwrap_or_else(|e| task_fail(e));
            if json {
                println!("{}", task_value(&t));
            } else {
                println!("✓ started {} in {} (pane {}, pid {})", t.name, t.target(), t.pane, t.pid);
                println!("  logs: tmm task logs {}", t.name);
            }
        }
        "list" => {
            let rows = tasks::list();
            if json {
                let arr: Vec<Value> = rows.iter().map(task_value).collect();
                println!("{}", json!({ "tasks": arr }));
            } else if rows.is_empty() {
                println!("no tasks");
            } else {
                let now = tasks::unix_now();
                println!("{:<16} {:<11} {:>4}  {:<18} {}", "NAME", "STATE", "AGE", "TARGET", "COMMAND");
                for t in &rows {
                    println!(
                        "{:<16} {:<11} {:>4}  {:<18} {}",
                        t.name, t.state_str(), tasks::fmt_age(t.age(now)), t.target(), t.cmd
                    );
                }
            }
        }
        "status" => {
            let Some(name) = arg else { fail(EXIT_USAGE, "task status <name>") };
            match tasks::find(name) {
                Some(t) => {
                    if json {
                        println!("{}", task_value(&t));
                    } else {
                        // State first, so `tmm task status x | awk '{print $1}'`
                        // and a bare `$(...)` comparison both work.
                        println!(
                            "{} {} {} pid {}",
                            t.state_str(),
                            tasks::fmt_age(t.age(tasks::unix_now())),
                            t.target(),
                            t.pid
                        );
                    }
                }
                None => {
                    if json {
                        println!("{}", json!({ "name": name, "state": "missing" }));
                    } else {
                        println!("missing");
                    }
                    std::process::exit(EXIT_NOT_FOUND);
                }
            }
        }
        "logs" => {
            let Some(name) = arg else {
                fail(EXIT_USAGE, "task logs <name> [--limit N] [--grep <text>]");
            };
            let limit = flags
                .get("limit")
                .cloned()
                .flatten()
                .and_then(|v| v.parse::<usize>().ok())
                .unwrap_or(50);
            let grep = flags.get("grep").cloned().flatten();
            let text = tasks::logs(name, limit, grep.as_deref()).unwrap_or_else(|e| task_fail(e));
            if json {
                let lines: Vec<&str> = text.lines().collect();
                println!("{}", json!({ "name": name, "lines": lines }));
            } else if !text.is_empty() {
                println!("{text}");
            }
        }
        "stop" => {
            let Some(name) = arg else { fail(EXIT_USAGE, "task stop <name>") };
            let t = tasks::stop(name).unwrap_or_else(|e| task_fail(e));
            if json {
                println!("{}", task_value(&t));
            } else {
                println!("✓ stopped {} ({})", t.name, t.state_str());
            }
        }
        "rm" => {
            let Some(name) = arg else { fail(EXIT_USAGE, "task rm <name>") };
            let t = tasks::remove(name).unwrap_or_else(|e| task_fail(e));
            if json {
                println!("{}", json!({ "removed": t.name }));
            } else {
                println!("✓ removed {}", t.name);
            }
        }
        _ => {
            eprint!("{USAGE}");
            std::process::exit(EXIT_USAGE);
        }
    }
}

/// The module's typed errors are the reason this is not string sniffing.
fn task_fail(e: tasks::Error) -> ! {
    let code = match &e {
        tasks::Error::Invalid(_) => EXIT_USAGE,
        tasks::Error::NotFound(_) => EXIT_NOT_FOUND,
        tasks::Error::Tmux(_) => EXIT_ERR,
    };
    fail(code, &e.to_string())
}

fn task_value(t: &tasks::Task) -> Value {
    json!({
        "name": t.name,
        "state": t.state_str(),
        "exit_code": match &t.state {
            tasks::State::Exited(code) => Some(*code),
            _ => None,
        },
        "signal": match &t.state {
            tasks::State::Killed(sig) => Some(sig.as_str()),
            _ => None,
        },
        "session": t.session,
        "window": t.window,
        "pane": t.pane,
        "pid": t.pid,
        "started": t.started,
        "cmd": t.cmd,
    })
}

/// Accepts a session name or a raw project id; resolves via project_list so
/// humans and agents can address projects by the name they see in tmux.
async fn resolve_project_id(ctx: &Ctx, name: &str) -> String {
    let r = rpc(ctx, "project_list", json!({ "include_archived": true })).await;
    let empty = Vec::new();
    let rows = r.get("projects").and_then(|a| a.as_array()).unwrap_or(&empty);
    for p in rows {
        let proj = p.get("project").cloned().unwrap_or(Value::Null);
        let id = proj.get("id").and_then(|v| v.as_str()).unwrap_or("");
        let session = proj.get("session").and_then(|v| v.as_str()).unwrap_or("");
        if session == name || id == name {
            return id.to_string();
        }
    }
    fail(EXIT_NOT_FOUND, &format!("no project with session or id '{name}' — try `tmm project list`"))
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
/// The third return is every valued occurrence in order, so a flag that may be
/// REPEATED (`--image a.png --image b.png`) does not lose all but the last —
/// the map keeps one value per key by design and that is fine for the rest.
fn split_flags(args: &[String]) -> (std::collections::HashMap<String, Option<String>>, Vec<String>, Vec<(String, String)>) {
    const VALUED: &[&str] = &["project", "agent", "server", "output", "since", "limit", "brief",
                          "name", "session", "with-agent", "backend", "model", "system", "skills", "mcp",
                          "ref", "source", "description", "def", "grep", "image"];
    let mut flags = std::collections::HashMap::new();
    let mut pos = Vec::new();
    let mut repeats: Vec<(String, String)> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if let Some(name) = a.strip_prefix("--") {
            if VALUED.contains(&name) && i + 1 < args.len() {
                flags.insert(name.to_string(), Some(args[i + 1].clone()));
                repeats.push((name.to_string(), args[i + 1].clone()));
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
    (flags, pos, repeats)
}

/// An image reference as the client will have to resolve it. A URL is passed
/// through untouched; a filesystem path is made absolute against the agent's
/// cwd, because the reader is a phone in another room and "./shot.png" means
/// nothing there.
fn absolutize_ref(src: &str) -> String {
    let s = src.trim();
    if s.starts_with("http://") || s.starts_with("https://") || s.starts_with("data:") || s.starts_with('/') {
        return s.to_string();
    }
    if let Some(rest) = s.strip_prefix("~/") {
        if let Some(home) = std::env::var_os("HOME") {
            return std::path::Path::new(&home).join(rest).to_string_lossy().to_string();
        }
    }
    std::env::current_dir()
        .map(|d| d.join(s).to_string_lossy().to_string())
        .unwrap_or_else(|_| s.to_string())
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
        println!("[{}] {from}: {body}", local_stamp(ts));
    }
}

/// Epoch MILLISECONDS -> local `2026-08-17 16:31`, for a reader that wants to
/// know when something was said. The raw epoch was printed here before, which
/// told an agent nothing it could reason about. Falls back to the raw number if
/// the value is not a sane timestamp.
fn local_stamp(ts_ms: i64) -> String {
    chrono::DateTime::from_timestamp_millis(ts_ms)
        .map(|dt| dt.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ts_ms.to_string())
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
                        println!("[{}] {from}: {body}", local_stamp(ts));
                    }
                }
            }
            Err((code, msg)) => fail(code, &msg),
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeated_valued_flags_survive_the_map() {
        // The flag map keeps one value per key; `--image` may appear twice, and
        // the third return is what stops the first one being silently lost.
        let args: Vec<String> = ["send", "look", "--image", "a.png", "--image", "b.png"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let (flags, pos, repeated) = split_flags(&args);
        assert_eq!(pos, vec!["send", "look"], "the text stays positional");
        assert_eq!(flags.get("image").cloned().flatten().as_deref(), Some("b.png"), "map keeps the last");
        let images: Vec<&str> = repeated.iter().filter(|(k, _)| k == "image").map(|(_, v)| v.as_str()).collect();
        assert_eq!(images, vec!["a.png", "b.png"], "both reach the sender");
    }

    #[test]
    fn log_timestamps_are_readable_local_time() {
        // The rendered value is LOCAL, so assert the
        // shape and that it round-trips through the same conversion rather than
        // hard-coding a zone the CI box may not share.
        let ms = 1_755_419_460_000_i64;   // 2025-08-17 08:31:00 UTC
        let got = local_stamp(ms);
        assert_eq!(got.len(), 16, "YYYY-MM-DD HH:MM, got {got:?}");
        assert!(got.starts_with("2025-08-1"), "the right day, got {got:?}");
        assert_eq!(
            got,
            chrono::DateTime::from_timestamp_millis(ms)
                .unwrap()
                .with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M")
                .to_string()
        );
        // A nonsense value degrades to the raw number instead of panicking.
        assert_eq!(local_stamp(i64::MAX), i64::MAX.to_string());
    }

    #[test]
    fn image_references_are_resolved_for_a_reader_somewhere_else() {        // URLs pass through untouched.
        for url in ["https://x/y.png", "http://x/y.png", "data:image/png;base64,AA"] {
            assert_eq!(absolutize_ref(url), url);
        }
        // An absolute path is already meaningful on the server's machine.
        assert_eq!(absolutize_ref("/tmp/shot.png"), "/tmp/shot.png");
        // A relative one is not: the reader is a phone, not this shell.
        let cwd = std::env::current_dir().unwrap();
        assert_eq!(
            absolutize_ref("shot.png"),
            cwd.join("shot.png").to_string_lossy().to_string()
        );
        // `~` is the agent's home, expanded here rather than shipped as a tilde.
        if let Some(home) = std::env::var_os("HOME") {
            assert_eq!(
                absolutize_ref("~/shot.png"),
                std::path::Path::new(&home).join("shot.png").to_string_lossy().to_string()
            );
        }
    }
}
