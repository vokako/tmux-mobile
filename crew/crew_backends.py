"""Pluggable agent backends + tmux window launching for a tmux-mobile team.

Vendored & adapted from the standalone agora project's demo/. Two changes from
the original:
  1. The agora bus is NOT started here — it runs in-process inside the
     tmux-mobile desktop server. These scripts only point agents at the
     already-running bus (`AGORA_URL`, default http://127.0.0.1:8787).
  2. Each agent is launched in its OWN tmux WINDOW named after the agent
     (`tmux new-window -n <name>`), not a split pane. The Team tab maps an
     agent to its pane by matching window_name, so the human can tap an agent
     and preview its live execution state in the Terminal tab.

Every backend does the same four things, in its own dialect:
  1. connect to the agora bus over HTTP MCP with an `x-agent: <name>` header
  2. run unattended (auto-approve every tool)
  3. keep its config isolated (a throwaway home dir; your real configs untouched)
  4. carry a role + the shared AGENTS.md brief

`prepare_agent(backend, ...)` writes the backend's config and returns (env, cmd);
`launch_window(...)` runs that in a tmux window. Used by run.py and supervise.py.
"""
import json
import pathlib
import shlex
import subprocess
import time

HERE = pathlib.Path(__file__).resolve().parent
KEEPALIVE_HOOK = str(HERE / "hooks" / "keepalive.sh")

# Non-management agora tools (workers/reviewer get these; managers also get hire/fire).
WORKER_TOOLS = ["post", "wait", "list_agents", "history"]

KICK = ("你已接入 crew 群聊（协作规则见 AGENTS.md）。"
        "直接调用 wait 等待消息；被点名就用 post 回复发起人，没你的事就继续 wait；不要主动停止。")


def role_line(role, goal):
    """One-line role injected as the first message for claude/codex (no newlines)."""
    g = (goal or "").strip().replace("\n", " ")
    return f"你是「{role}」。{g} 请用中文、消息简短。".strip()


def full_prompt(role, goal, backstory):
    """Multi-line prompt embedded in Kiro's agent config."""
    return (f"你是「{role}」。\n目标：{(goal or '').strip()}\n背景：{(backstory or '').strip()}\n"
            "你和其他 agent、以及一位人类，在共享的『crew 群聊』里协作（通过 @crew 工具）。"
            "请始终用中文交流，消息保持简短。")


def prepare_agent(backend, *, name, role, goal, backstory, manage, url, demo, workspace, model=None):
    """Write `backend`'s isolated config for this agent; return (env, cmd, post_keys).

    `post_keys` are tmux keys to send a few seconds after launch (e.g. Claude's
    one-time folder-trust acceptance).
    """
    backend = (backend or "kiro").lower()
    if backend == "kiro":
        return _kiro(name, role, goal, backstory, manage, url, demo, workspace, model)
    if backend == "claude":
        return _claude(name, role, goal, manage, url, demo, model)
    if backend == "codex":
        return _codex(name, role, goal, manage, url, demo)
    raise ValueError(f"unknown backend: {backend}")


# ---- Kiro (kiro-cli) ----
def _kiro(name, role, goal, backstory, manage, url, demo, workspace, model):
    home = demo / "kiro-home"
    (home / "agents").mkdir(parents=True, exist_ok=True)
    (home / "settings").mkdir(parents=True, exist_ok=True)
    (home / "settings" / "cli.json").write_text(
        json.dumps({"chat.disableTrustAllConfirmation": True}, indent=2))
    agora = ["@crew"] if manage else [f"@crew/{t}" for t in WORKER_TOOLS]
    conf = {
        "name": name,
        "description": f"{role} on the agora bus",
        "prompt": full_prompt(role, goal, backstory),
        "tools": ["*"] + agora,
        "allowedTools": ["@builtin"] + agora,
        "resources": [f"file://{pathlib.Path(workspace) / 'AGENTS.md'}"],
        "mcpServers": {"crew": {"url": f"{url}/mcp", "headers": {"x-agent": name}}},
        "hooks": {"stop": [{"command": KEEPALIVE_HOOK}]},
    }
    (home / "agents" / f"{name}.json").write_text(json.dumps(conf, ensure_ascii=False, indent=2))
    env = {"KIRO_HOME": str(home)}
    m = model or "claude-sonnet-4.6"
    cmd = f"kiro-cli chat --agent {name} --model {m} --trust-all-tools {shlex.quote(KICK)}"
    return env, cmd, []


# ---- Claude Code (claude) ----
# Claude stores auth + onboarding state in its config home, so isolating CLAUDE_CONFIG_DIR
# would break login and trigger first-run onboarding. Instead we use the user's real
# (authed) config but inject the bus per-invocation with --mcp-config/--strict-mcp-config
# and the keepalive hook via --settings, so we never modify their config.
def _claude(name, role, goal, manage, url, demo, model):
    d = demo / "claude"
    d.mkdir(parents=True, exist_ok=True)
    mcpfile = d / f"{name}.mcp.json"
    mcpfile.write_text(json.dumps({"mcpServers": {"crew": {
        "type": "http", "url": f"{url}/mcp", "headers": {"x-agent": name}}}}, indent=2))
    settingsfile = d / f"{name}.settings.json"
    settingsfile.write_text(json.dumps({
        "hooks": {"Stop": [{"hooks": [{"type": "command", "command": KEEPALIVE_HOOK}]}]}
    }, indent=2))
    env = {}
    m = model or "sonnet"
    disallow = "" if manage else "--disallowedTools mcp__crew__hire mcp__crew__fire "
    first_msg = f"{role_line(role, goal)} {KICK}"
    # Start interactive (no positional prompt). Then: accept the folder-trust dialog,
    # type the kickoff, and submit it. (Passing the prompt positionally races the trust
    # dialog and gets dropped.)
    cmd = (f"claude --mcp-config {shlex.quote(str(mcpfile))} --strict-mcp-config "
           f"--settings {shlex.quote(str(settingsfile))} "
           f"--model {m} --dangerously-skip-permissions {disallow}".rstrip())
    post = [("enter", None), ("text", first_msg), ("enter", None)]
    return env, cmd, post


# ---- Codex (codex) ----
def _codex(name, role, goal, manage, url, demo):
    home = demo / "codex" / name
    home.mkdir(parents=True, exist_ok=True)
    gating = "" if manage else 'disabled_tools = ["hire", "fire"]\n'
    config = (
        "[mcp_servers.agora]\n"
        f'url = "{url}/mcp"\n'
        "enabled = true\n"
        "experimental_use_rmcp_client = true\n"
        f"{gating}"
        "\n[mcp_servers.agora.http_headers]\n"
        f'"x-agent" = "{name}"\n'
    )
    (home / "config.toml").write_text(config)
    env = {"CODEX_HOME": str(home)}
    first_msg = f"{role_line(role, goal)} {KICK}"
    cmd = f"codex --dangerously-bypass-approvals-and-sandbox {shlex.quote(first_msg)}"
    return env, cmd, []


def launch_window(session, name, env, cmd, workspace, post_keys=()):
    """Open a tmux WINDOW named `name`, set env, run `cmd`, send post-launch keys.

    The window is named after the agent so the Team tab can map agent→pane by
    window_name. Returns the new pane id.
    """
    pane = subprocess.check_output(
        ["tmux", "new-window", "-t", session, "-n", name, "-P", "-F", "#{pane_id}", "-c", str(workspace)],
        text=True,
    ).strip()
    time.sleep(1.0)
    prefix = " ".join(f"{k}={shlex.quote(v)}" for k, v in env.items())
    subprocess.run(["tmux", "send-keys", "-t", pane, f"{prefix} {cmd}", "Enter"], check=False)
    # Post-launch scripted steps, e.g. accept Claude's trust dialog then type the kick.
    # Each step is ("enter", None) to press Enter, or ("text", "...") to type a line.
    for kind, val in post_keys:
        time.sleep(4.0)
        if kind == "enter":
            subprocess.run(["tmux", "send-keys", "-t", pane, "Enter"], check=False)
        elif kind == "text":
            subprocess.run(["tmux", "send-keys", "-t", pane, "-l", val], check=False)
    return pane
