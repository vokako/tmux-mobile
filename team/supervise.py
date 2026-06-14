#!/usr/bin/env python3
"""Team supervisor — turns hire/fire decisions into real agent windows.

Vendored & adapted from the standalone agora project. It talks to the agora bus
that runs IN-PROCESS inside the tmux-mobile desktop server (default
http://127.0.0.1:8787), so it never starts its own daemon.

Polls the bus's /api/employees:
  - state 'requested'/'active' not yet launched -> generate a worker config + a
    named tmux window (skipped if an agent with that name is already online)
  - state 'disabled' -> kill its window

The bus stays a pure coordination layer; all process orchestration lives here.
Started by run.py; exits when the tmux session goes away.
"""
import json
import os
import sys
import time
import urllib.request

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import agora_demo  # noqa: E402

import pathlib

URL = os.environ.get("AGORA_URL", "http://127.0.0.1:8787")
SESSION = os.environ.get("AGORA_SESSION", "agora")
WORK = os.environ.get("AGORA_WORKSPACE", ".")
DEMO = pathlib.Path(os.environ.get("AGORA_DEMO", "/tmp/tmux-mobile-team"))
HIRE_BACKEND = os.environ.get("AGORA_HIRE_BACKEND", "kiro")
INTERVAL = float(os.environ.get("AGORA_SUPERVISE_INTERVAL", "3"))

import subprocess


def session_alive():
    return subprocess.run(["tmux", "has-session", "-t", SESSION],
                          stderr=subprocess.DEVNULL).returncode == 0


def get_json(path):
    try:
        with urllib.request.urlopen(f"{URL}{path}", timeout=4) as r:
            return json.load(r)
    except Exception:
        return None


launched = {}   # name -> pane_id (None if it was already online)
killed = set()

while session_alive():
    employees = get_json("/api/employees") or []
    roster = get_json("/api/roster") or []
    online = {a["name"]: a["status"] for a in roster}

    for e in employees:
        name, state = e["name"], e["state"]
        if state == "disabled":
            if name in launched and name not in killed:
                pane = launched.get(name)
                if pane:
                    # Kill the whole window the agent ran in.
                    subprocess.run(["tmux", "kill-window", "-t", pane], stderr=subprocess.DEVNULL)
                killed.add(name)
            continue
        if name in launched:
            continue
        # Already running (e.g. supervisor restarted): adopt without relaunching.
        if online.get(name) and online[name] != "offline":
            launched[name] = None
            continue
        # Launch this employee from its opaque spec. `backend` defaults to the configured
        # hire backend (runtime hires carry no backend; the seeded team does).
        spec = e.get("spec") or {}
        backend = spec.get("backend") or HIRE_BACKEND
        env, cmd, post_keys = agora_demo.prepare_agent(
            backend, name=name, role=spec.get("role", name), goal=spec.get("goal", ""),
            backstory=spec.get("backstory", ""), manage=bool(spec.get("manage", False)),
            url=URL, demo=DEMO, workspace=pathlib.Path(WORK), model=spec.get("model"))
        try:
            pane = agora_demo.launch_window(SESSION, name, env, cmd, WORK, post_keys)
            launched[name] = pane
            print(f"[supervise] launched '{name}' ({backend}) in window {pane}", flush=True)
        except Exception as ex:
            print(f"[supervise] failed to launch '{name}': {ex}", flush=True)

    time.sleep(INTERVAL)
