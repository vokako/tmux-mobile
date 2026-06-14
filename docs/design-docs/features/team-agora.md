# Team — multi-agent collaboration via the agora bus

> A letter to the next agent. This explains **why** the Team feature is shaped
> the way it is and the traps we avoided. Code comments cover the **how**.

## 1. What it is

The **Team** tab lets the human, from the phone, drop into a group chat with
several coding agents (Kiro / Claude Code / Codex) and coordinate them — and tap
any agent to preview its live tmux execution state in the Terminal tab.

It is built on **agora**, an experimental group-chat message bus for multi-agent
coordination (originally a standalone project at `~/agora`). agora is:

- An **append-only group chat** over SQLite. Everyone reads the same log.
- A **pull model**: agents call `wait` (blocking long-poll) to receive messages,
  `post` to speak. Addressing is by `@name` in the body; `requires_reply=true`
  makes the bus refuse an agent's `wait` until it answers (the "obligation
  graph"). This is enforced in the bus, not per-agent — agent-agnostic.
- Served as an **HTTP MCP server** (`/mcp`) for agents (zero-touch: a role prompt
  + an `x-agent` header) plus a dashboard / SSE / JSON API for humans.

Full agora rationale lives in the vendored crate and `~/agora/docs/`.

## 2. Integration shape — and why

The decisive choice: **vendor agora as an in-process, desktop-only sub-crate and
share ONE `Bus` between the agents' MCP daemon and the phone's WS server.**

```
                tmux-mobile desktop server (one process)
   ┌───────────────────────────────────────────────────────────┐
   │  agora::Bus  (SQLite + tokio::broadcast)                    │
   │     ├── agora::web::serve → axum :8787  /mcp + dashboard ───┼──► kiro/claude/codex agents
   │     ├── AgoraBridge (JSON trait) ──► tmux-mobile WS server ─┼──► phone Team tab
   │     └── team::supervisor (in-process) ─────────────────────┼──► launches agents into tmux
   └───────────────────────────────────────────────────────────┘
        ▲ agents each run in their OWN named tmux window
```

**Team launching is in-process** (`src-tauri/src/team.rs`): the phone's "Start
team" button calls `agora_start_team`, and the desktop server itself seeds the
default roster onto the bus and reconciles it into real agent windows in tmux —
no separate script, no extra process. The agent CLIs themselves still run as
their own processes in tmux (intrinsic, and what enables pane preview), but all
orchestration lives in the app. The standalone `team/` Python scripts are kept
as an optional advanced/headless path (custom rosters), not the default.

Why this shape, and what was rejected:

- **In-process, not a separate daemon the phone HTTP-hops to.** The phone already
  has an authenticated, encrypted WS link to the desktop server. Routing agora
  through it (a handful of `agora_*` RPC methods + an `agora_message` push) means
  **no second auth surface, no second port the phone must reach, no CORS, reuse
  of the existing E2E encryption + reconnect logic.** External agents still get
  the real MCP endpoint on `:8787` because the *same* `Bus` also runs agora's
  axum router. One room, two front doors.

- **Desktop-only, target-gated.** agora pulls in axum + rmcp + rusqlite — heavy,
  and pointless on a phone (the phone is a *client* of a desktop server, it never
  hosts the bus). The dependency is gated:
  `[target.'cfg(not(any(target_os = "android", target_os = "ios")))'.dependencies]`.
  Verified: `cargo tree --target aarch64-linux-android -i agora` prints nothing;
  the host target shows it. The Android build is byte-for-byte unaffected.

- **A JSON-only trait at the boundary (`server::AgoraBridge`).** `server.rs`
  compiles on mobile too, so it must not name any agora type. The bridge trait
  speaks only `serde_json::Value`; the concrete impl
  (`agora_bridge::AgoraBus`) lives in a desktop-only module. Mobile passes
  `None` and the `agora_*` methods return method-not-found, which the Team tab
  reads as "unavailable" and hides itself. This is the seam that keeps one
  codebase building for both targets.

- **Vendored, not a git submodule / path-dep on `~/agora`.** Per the repo creed,
  "not in the repo = doesn't exist." A submodule would make a clean checkout
  fail to build and couple us to an external experimental tree. We copied the
  library source into `src-tauri/crates/agora/` (dropping its CLI `main.rs` — the
  daemon runs in-process here) and kept its tests.

## 3. Agent ↔ tmux pane link (the "preview execution state" requirement)

The supervisor gives **each agent its own tmux window named after the agent**
(`tmux new-window -n <name>`, see `tmux::new_named_window`). The Team tab maps an
agent to its pane by matching `window_name == agent.name`
(`Team.svelte:previewAgent`), then calls the existing `openTerminal(...)`. So
tapping "worker" in the roster opens worker's live tmux pane in the Terminal tab
— the same subscribe/snapshot path every other pane uses. No new streaming code.

## 4. The in-process supervisor (`src-tauri/src/team.rs`)

`team::start` runs as a tokio task in the desktop server. On the operator's
"Start team" action (`agora_start_team`, one-shot guarded) it:

1. writes the shared brief (`AGENTS.md`/`CLAUDE.md`) + keepalive hook into
   `~/.config/tmux-mobile/team/workspace/` (both embedded via `include_str!`,
   so a packaged `.app` needs no external files);
2. seeds the built-in roster (manager / worker / reviewer) as employees on the
   bus (skipped if a team is already present — restart-safe);
3. runs a 3 s reconcile loop: a `requested`/`active` employee not yet launched
   gets its backend config written (kiro/claude/codex, ported from agora's
   `prepare_agent`) and a named tmux window opened running it; a `disabled`
   employee's window is killed. The same loop serves the initial team AND any
   runtime `hire`/`fire` the manager does.

The agent config dialects (kiro agent JSON, claude `--mcp-config`/`--settings`,
codex `config.toml`), the keepalive Stop-hook, and the role/goal prompts are all
carried over verbatim from agora's verified launcher.

### Optional: the standalone `team/` scripts

`team/` keeps the Python launcher (vendored from agora's `demo/`) for advanced /
headless use — e.g. a custom `team.yaml` roster. It targets the already-running
bus (no daemon) and uses named windows, same as the in-process path. Most users
never need it; the in-app button is the default. Run:
`cd team && uv run --with pyyaml python run.py` (server already running;
kiro-cli / claude / codex logged in).

## 5. Config

Desktop server reads (config.toml or env): `agora_bind`/`AGORA_BIND`
(default `127.0.0.1:8787`), `agora_db`/`AGORA_DB` (default
`~/.config/tmux-mobile/agora.db`), `agora_room`/`AGORA_ROOM` (default `main`).
Bus startup is best-effort: if it fails (e.g. port in use) the terminal server
still runs and the Team tab simply stays hidden.

> Security: agora's `/mcp` and `/api/*` have **no auth** — only the phone path is
> authenticated (it rides the token-authed WS). Keep `AGORA_BIND` on `127.0.0.1`
> unless you intend other LAN devices to reach the dashboard directly, and only
> on a trusted network.

## 6. Known edges / future

- **Roster status colors** are a coarse map (working/waiting/online). The dot
  doesn't yet distinguish "owes a reply" (deadlock risk) — the data is there
  (`/api/quiescence`) if we want a deadlock indicator in the tab.
- **Backends not all verified locally**: kiro + claude were verified end-to-end
  in the upstream agora project; codex needs `codex login`. Unchanged here.
- **Single room.** The bus supports a `room` column; the server runs one room.
- **No agora message persistence surfaced in the tab beyond history(200)** — the
  bus keeps the full SQLite log; the tab just loads a recent window.
