# Team (Crew) — multi-agent collaboration

> A letter to the next agent. This explains **why** the Team feature is shaped
> the way it is and the traps we avoided. Code comments cover the **how**.

## 1. What it is

The **Team** tab lets the human, from the phone, drop into a group chat with
several coding agents (Kiro / Claude Code / Codex) and coordinate them — and tap
any agent to preview its live tmux execution state in the Terminal tab.

It is built on **agora**, an experimental group-chat message bus (originally a
standalone project at `~/agora`). agora is:

- An **append-only group chat** over SQLite. Everyone reads the same log.
- A **pull model**: agents call `wait` (blocking long-poll) to receive messages,
  `post` to speak. Addressing is by `@name` in the body; `requires_reply=true`
  makes the bus refuse an agent's `wait` until it answers (the "obligation
  graph"). Enforced in the bus, not per-agent — agent-agnostic.
- Served as an **HTTP MCP server** (`/mcp`) for agents (zero-touch: a role prompt
  + an `x-agent` header) plus a dashboard / SSE / JSON API for humans.

### Naming: "crew" is ours, "agora" is the library

"agora" is the upstream project's codename. Everything **we** built and
everything the **user** sees is branded **Crew** (the Team tab, `crew_*` RPCs,
`CREW_*` config, `tmm-crew-*` tmux sessions, `crew.rs`/`crew_bridge.rs`,
`crew/` scripts). The **vendored library crate keeps its real name `agora`**
(`src-tauri/crates/agora/`) — renaming a faithful third-party copy would only
obscure its origin. So `use agora::bus::Bus` inside `crew_bridge.rs` is
deliberate; everything around it is `crew`.

## 2. Integration shape — and why

The decisive choice: **vendor agora as an in-process, desktop-only sub-crate and
share ONE `Bus` between the agents' MCP daemon and the phone's WS server.**

```
                tmux-mobile desktop server (one process)
   ┌───────────────────────────────────────────────────────────┐
   │  agora::Bus  (SQLite + tokio::broadcast)                    │
   │     ├── agora::web::serve → axum :8787  /mcp + dashboard ───┼──► kiro/claude/codex agents
   │     ├── CrewBridge (JSON trait) ──► tmux-mobile WS server ──┼──► phone Team tab
   │     └── crew::supervisor (in-process) ─────────────────────┼──► launches agents into tmux
   └───────────────────────────────────────────────────────────┘
        ▲ agents each run in their OWN named window of a
          per-workspace session: tmm-crew-<workspace-slug>
```

**Launching is in-process** (`src-tauri/src/crew.rs`): the phone's "Start team"
button calls `crew_start_team` with a chosen **workspace** (the agents' working
dir), and the desktop server itself seeds the default roster onto the bus and
reconciles it into real agent windows in tmux — no separate script, no extra
process. The agent CLIs still run as their own tmux processes (intrinsic, and
what enables pane preview), but all orchestration lives in the app. The
standalone `crew/` Python scripts remain as an optional advanced/headless path.

Why this shape, and what was rejected:

- **In-process, not a separate daemon the phone HTTP-hops to.** The phone already
  has an authenticated, encrypted WS link to the desktop server. Routing crew
  through it (a handful of `crew_*` RPC methods + a `crew_message` push) means
  **no second auth surface, no second port the phone must reach, no CORS, reuse
  of the existing E2E encryption + reconnect logic.** External agents still get
  the real MCP endpoint on `:8787` because the *same* `Bus` also runs agora's
  axum router. One room, two front doors.

- **Desktop-only, target-gated.** agora pulls in axum + rmcp + rusqlite — heavy,
  and pointless on a phone (the phone is a *client*, it never hosts the bus). The
  dependency is gated:
  `[target.'cfg(not(any(target_os = "android", target_os = "ios")))'.dependencies]`.
  Verified: `cargo tree --target aarch64-linux-android -i agora` prints nothing;
  the host target shows it. The Android build is unaffected.

- **A JSON-only trait at the boundary (`server::CrewBridge`).** `server.rs`
  compiles on mobile too, so it must not name any agora type. The trait speaks
  only `serde_json::Value`; the concrete impl (`crew_bridge::CrewBus`) lives in a
  desktop-only module. Mobile passes `None` and the `crew_*` methods return
  method-not-found, which the Team tab reads as "unavailable" and hides itself.
  This seam keeps one codebase building for both targets.

- **Vendored, not a git submodule / path-dep on `~/agora`.** Per the repo creed,
  "not in the repo = doesn't exist." We copied the library source into
  `src-tauri/crates/agora/` (dropping its CLI `main.rs` — the daemon runs
  in-process) and kept its tests.

## 2b. Multiple teams = isolated rooms

The bus is **room-aware**: each team is its own chat **room** (id = the
workspace slug), fully isolated — separate message log, roster, and obligation
graph. One daemon serves all rooms.

- **agora crate**: a `BusProvider` trait (`room → Bus`) that `mcp.rs`/`web.rs`
  resolve per request — agents via an **`x-room` header**, the human API via a
  `?room=` query. `SingleRoom` wraps one `Bus` for the single-room/test path.
  The store was already fully room-keyed, so only routing changed.
- **Host (`crew_bridge.rs`)**: `CrewBus` is a **registry** of rooms (each a
  `Bus` lazily opened on the shared db, with its own pump into one merged push
  channel) and *is* the `BusProvider` for the MCP daemon. Unknown rooms are
  refused until a team is started — a stray agent can't conjure a room by
  guessing a header; only the operator's "New team" creates one.
- **The phone** passes the active `room` with every `crew_*` RPC and filters the
  `crew_message` push by each message's `room`, so the view never mixes teams.
- **Agents** are launched with an `x-room` header for their team's room, so
  kiro/claude/codex each join the right chat.
- **Team tab UI**: a header dropdown lists active teams (room · agent count),
  "+ New team" opens the workspace picker, and "×" closes the active team
  (`crew_close_team` kills its tmux session; the chat log persists in the db, so
  re-starting the same workspace resumes its history).

## 3. Per-workspace crews (the "limited to a directory" requirement)

A crew is tied to a **workspace** = the agents' shared working directory (their
real project). The phone defaults it to the current terminal session's cwd
(`fsCwd`), shows it, and lets the user edit it before starting. The room id IS
the workspace slug, so the team, its session `tmm-crew-<slug>`, and its config
home all share one identifier.

- The tmux session is **`tmm-crew-<slug>`** where `slug` is the sanitized
  workspace basename (`crew::workspace_slug`, mirrored in `Team.svelte:slugify`
  — they MUST agree, or pane preview can't find windows). tmux names can't
  contain `:` or `.`, so the slug strips them. The `tmm-crew-` prefix lets the
  app recognize crew sessions (e.g. the PanePicker labels their panes by agent
  name, not `current_command`).
- Multiple crews coexist: different workspaces → different `tmm-crew-<slug>`
  sessions, each with its own config home `~/.config/tmux-mobile/crew/<slug>/`.
- **We never write our brief into the user's project.** `AGENTS.md` + the
  keepalive hook live in the private per-crew home; kiro loads the brief via an
  absolute `resources` path, claude/codex are pointed at it in their kick. The
  user's workspace stays clean — agents only `cd` into it and read/write the
  files the task actually needs.

## 4. Agent ↔ tmux pane link (the "preview execution state" requirement)

Each agent runs in its own tmux window **named after the agent**
(`tmux::new_named_window`). Agent → pane mapping is `window_name == agent.name`
within `tmm-crew-<slug>`. Two surfaces use it:

- **Mobile / narrow**: the roster chip → `Team.svelte:previewAgent` →
  `openTerminal(...)`, jumping to that agent's pane in the Terminal tab (same
  subscribe/snapshot path as any pane). The PanePicker dropdown also labels
  `tmm-crew-*` panes by agent name.

- **Desktop (≥900px, non-touch)**: the Team tab is a **two-pane split** — a live
  agent terminal **grid** on the left, the chat on the right, with a draggable
  splitter (ratio persisted in `tmux_crew_gridfrac`, default 0.6). See
  `AgentGrid.svelte`:
  - one cell per employee, laid out near-square: `cols = ceil(√n)`,
    `rows = ceil(n/cols)`;
  - each cell is a **chromeless** embedded `Terminal` (new prop — no
    window-switcher bar, window-list poll skipped, since the cell is pinned to
    one agent). Read-only until clicked; clicking activates it (xterm focus +
    full interaction), exactly like an active split cell;
  - cell font = app font − 2 (glanceable previews);
  - panes resolved via a 2 s `listSessionsWithPanes` poll, so cells fill in as
    agents launch; offline employees show a placeholder until their window
    appears.
  No quick-switch chrome — the grid is fixed by agent count, by design.

## 5. The in-process supervisor (`src-tauri/src/crew.rs`)

`crew::start(bridge, cfg, workspace)` runs as a tokio task. On "Start team"
(`crew_start_team`, one-shot guarded) it:

1. derives `slug` + session `tmm-crew-<slug>`, writes the brief + keepalive hook
   into the private per-crew home (embedded via `include_str!`, so a packaged
   `.app` needs no external files);
2. seeds the built-in roster (manager / worker / reviewer) as employees, unless a
   crew is already present (restart-safe);
3. runs a 3 s reconcile loop launching each employee into a named window;
   `disabled` employees' windows are killed. Same loop serves the initial team
   AND the manager's runtime `hire`/`fire`.

**Idempotent across restarts (dup-window fix).** Before launching, the loop
checks tmux for an existing window with the agent's name in the session and
**adopts** it instead of opening a second. The earlier in-memory-only tracking
re-launched every agent when the server restarted, piling up duplicate
manager/worker/reviewer windows (observed: a 10-window `agora` session). See
`tmux::find_window_by_name`.

The agent config dialects (kiro agent JSON, claude `--mcp-config`/`--settings`,
codex `config.toml`), the keepalive Stop-hook, and the role/goal prompts carry
over from agora's verified launcher. The MCP server the agents connect to is
named **`crew`** in their configs (so MCP tool names are `mcp__crew__hire` etc).

### Optional: the standalone `crew/` scripts

`crew/` keeps the Python launcher for advanced / headless use (custom
`team.yaml` roster). It targets the already-running bus (no daemon) and uses
named windows. Most users never need it; the in-app button is the default.
Run: `cd crew && uv run --with pyyaml python run.py`.

## 6. The obligation bug we hit and fixed

First live run spammed `@human 在线` dozens of times. Root cause: the human
operator is never a registered agent, so when the human's `@worker` message
(with `requires_reply`) obligated the worker, the worker's reply `@human …`
resolved to no registered recipient — `mentioned_names` dropped it — and the
debt **never cleared**. The worker stayed `Blocked` forever and re-replied every
tick (keepalive amplified it). Fix (`agora::bus::post`): debt discharge now
matches **any** raw `@`-mentioned creditor, registered or not, so an agent can
always answer the human. New obligations still only attach to registered agents.
Regression test: `can_discharge_debt_to_unregistered_human`.

## 7. Config

Desktop server reads (config.toml or env, `AGORA_*` accepted as legacy aliases):
`crew_bind`/`CREW_BIND` (default `127.0.0.1:8787`), `crew_db`/`CREW_DB` (default
`~/.config/tmux-mobile/crew.db`), `crew_room`/`CREW_ROOM` (default `main`),
`crew_model`/`CREW_MODEL` (default `claude-sonnet-4.6`). Bus startup is
best-effort: on failure the terminal server still runs and the Team tab stays
hidden.

> Security: the MCP `/mcp` and `/api/*` have **no auth** — only the phone path is
> authenticated (it rides the token-authed WS). Keep `CREW_BIND` on `127.0.0.1`
> unless you intend other LAN devices to reach the dashboard directly, and only
> on a trusted network.

## 8. Known edges / future

- **`requires_reply` defaulting.** The phone infers `requires_reply=true` from an
  `@mention`. After the §6 fix this is safe, but a casual "@worker 你在线吗" still
  creates a real obligation. Consider making it opt-in if teams feel too rigid.
- **Roster status colors** are coarse (working/waiting/online); no "owes a reply"
  / deadlock indicator yet (data is in `/api/quiescence`).
- **Backends not all verified locally**: kiro + claude verified upstream; codex
  needs `codex login`.
- **History window**: the tab loads `crew_history(200)`; the full log lives in
  SQLite.
