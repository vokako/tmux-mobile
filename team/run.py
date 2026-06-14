#!/usr/bin/env python3
"""Team launcher — spin up a multi-agent team that joins tmux-mobile's team bus.

Vendored & adapted from the standalone agora project. KEY DIFFERENCE: the agora
bus runs IN-PROCESS inside the tmux-mobile desktop server, so this script does
NOT start a daemon. It assumes the tmux-mobile app (or `cargo run --bin server`)
is already running with its team bus up (default http://127.0.0.1:8787), then:

  1. seeds the DESIRED roster from team.yaml (POST /api/employees)
  2. starts the supervisor, which reconciles that roster into real agents, each
     in its own named tmux window inside the per-workspace tmm-team session

You drive the team from the phone's Team tab (or the dashboard / CLI). No task
is posted automatically — message the team yourself.

Run it:  cd team && uv run --with pyyaml python run.py
Prereqs: kiro-cli (and/or claude / codex) logged in; tmux-mobile server running.
"""
import json, os, pathlib, shutil, subprocess, sys, time, urllib.request

try:
    import yaml
except ImportError:
    sys.exit("PyYAML required. Run via:  uv run --with pyyaml python run.py")

HERE = pathlib.Path(__file__).resolve().parent
sys.path.insert(0, str(HERE))
import team_backends  # noqa: E402

SESSION = os.environ.get("TEAM_SESSION", "tmm-team-team")
DEMO = pathlib.Path(os.environ.get("TEAM_DEMO_DIR", "/tmp/tmux-mobile-team"))
KIRO_HOME = DEMO / "kiro-home"
WORK = pathlib.Path(os.environ.get("TEAM_WORKSPACE", str(DEMO / "workspace")))

cfg = yaml.safe_load((HERE / "team.yaml").read_text())
srv = cfg.get("server", {})
# Where the tmux-mobile server's in-process bus listens. team.yaml's `bind` is
# the server's TEAM_BIND; agents + CLI always connect locally.
bind = srv.get("bind", "127.0.0.1:8787")
port = bind.rsplit(":", 1)[-1]
model = srv.get("model", "claude-sonnet-4.6")
url = os.environ.get("TEAM_URL", f"http://127.0.0.1:{port}")
agents = cfg["agents"]


def tmux(*args):
    subprocess.run(["tmux", *args], check=False)


def tmux_out(*args):
    return subprocess.check_output(["tmux", *args], text=True).strip()


# 1. Confirm the bus is reachable (the desktop server must be running).
print(f"==> checking team bus at {url}")
reachable = False
for _ in range(10):
    try:
        urllib.request.urlopen(f"{url}/api/roster", timeout=1)
        reachable = True
        break
    except Exception:
        time.sleep(0.3)
if not reachable:
    sys.exit(f"team bus not reachable at {url}.\n"
             f"Start the tmux-mobile app or `cd src-tauri && cargo run --bin server` first\n"
             f"(its TEAM_BIND must match {bind}).")

# 2. Fresh isolated workspace (your real ~/.kiro is never touched).
print(f"==> setting up isolated team home at {DEMO}")
if DEMO.exists():
    shutil.rmtree(DEMO)
WORK.mkdir(parents=True, exist_ok=True)
# Shared brief: Kiro loads it via `resources`; Claude reads CLAUDE.md and Codex
# reads AGENTS.md from the workspace — so write both names.
shared_brief = HERE / "AGENTS.md"
if shared_brief.exists():
    shutil.copy(shared_brief, WORK / "AGENTS.md")
    shutil.copy(shared_brief, WORK / "CLAUDE.md")

# 3. Ensure the tmux session exists (agents get one window each inside it).
if subprocess.run(["tmux", "has-session", "-t", SESSION], stderr=subprocess.DEVNULL).returncode != 0:
    tmux_out("new-session", "-d", "-P", "-F", "#{pane_id}", "-s", SESSION, "-n", "team", "-c", str(WORK))
    tmux("set-option", "-t", SESSION, "history-limit", "100000")
print(f"==> tmux session '{SESSION}' ready")

# 4. Seed the DESIRED roster: every team member is an employee. The supervisor
#    reconciles this into windows (one launch path for the initial team AND any
#    runtime hires the manager makes).
for name, a in agents.items():
    backend = a.get("backend", "kiro")
    spec = {
        "role": a.get("role", name), "goal": a.get("goal", ""),
        "backstory": a.get("backstory", ""), "backend": backend,
        "manage": bool(a.get("manage", False)),
        "model": a.get("model") or (model if backend == "kiro" else None),
    }
    body = json.dumps({"name": name, "spec": spec}).encode()
    req = urllib.request.Request(f"{url}/api/employees", data=body,
                                 headers={"content-type": "application/json"})
    try:
        urllib.request.urlopen(req, timeout=4)
    except Exception as ex:
        print(f"    seed {name} failed: {ex}")
print("    seeded team:", ", ".join(f"{n}[{a.get('backend', 'kiro')}]" for n, a in agents.items()))

# 5. Supervisor: turns the seeded roster + manager's hire/fire into real windows.
supervise_env = {**os.environ, "TEAM_URL": url, "TEAM_SESSION": SESSION,
                 "KIRO_HOME": str(KIRO_HOME), "TEAM_MODEL": model,
                 "TEAM_WORKSPACE": str(WORK), "TEAM_DEMO": str(DEMO),
                 "TEAM_HIRE_BACKEND": srv.get("hire_backend", "kiro")}
subprocess.Popen([sys.executable, str(HERE / "supervise.py")],
                 env=supervise_env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)

print(f"""
==> team is coming online (isolated; your ~/.kiro is untouched). NO task posted.
    bus       : {url}/   (dashboard; also reachable from the phone's Team tab)
    tmux      : tmux attach -t {SESSION}   (one window per agent)
    workspace : {WORK}
    model     : {model}

    Drive the team from the phone's Team tab, the dashboard, or your own MCP CLI.
    Tap an agent in the Team tab to preview its live tmux window.

    Stop the team:  tmux kill-session -t {SESSION}
""")
