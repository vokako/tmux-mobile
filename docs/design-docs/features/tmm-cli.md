# tmm CLI — the agent's hands

## What this is

`tmm` (binary at `src-tauri/src/bin/tmm.rs`) is the CLI front for the project
hub: one chat room per project, agent status declarations, derived agent
states. It is the **only active interface an agent has** to the rest of the
system — the CLI-only substrate decision from
`docs/exec-plans/agents-v2.md` §4.1: what an agent *says* goes through `tmm`,
what we *observe* arrives through hooks into `projects::telemetry`, and the
two channels are joined at read time.

There is deliberately no MCP tool surface for this. MCP requires per-backend
config materialization and three different schema dialects; `tmm` requires
one line in a system prompt. The agora MCP daemon still exists for Team
(legacy) and is not extended.

## Commands

```
# agent-facing (context from $TMM_PROJECT / $TMM_AGENT, exported by the launcher)
tmm send "<text>"                    post to the project chat; @name addresses
tmm log [--since <ts>] [--limit N] [-f]   read chat; --since is exclusive (ms)
tmm status <working|waiting|blocked> [note]
tmm done [summary]                   completion; also posts "✔ done — summary"

# human-facing AND agent-facing — self-management
tmm agent list                       windows + agent detection + derived state
tmm project list                     ● live / ○ down, session + path
tmm project create <path> [--name n] [--session s] [--with-agent kiro|claude|codex]
tmm project up|down|archive <session>
tmm registry list                    centrally-defined agents
tmm registry save --name <n> --backend <b> [--system <text>] [--skills a,b] [--mcp <json>] [--can-hire]
tmm registry delete <name>
tmm spawn <agent> [--brief <text>]   spawn a registry agent into this project
```

## Self-management: the app operates itself through its own CLI

Everything the UI can do to projects and the registry, `tmm` can do — and the
spawn prompt tells agents so. A lead can set up a whole project
(`tmm project create` → `up`), define a NEW kind of agent
(`tmm registry save`) and then spawn it: definition → instantiation →
delegation, all inside one conversation. This adds no authority: an agent
already holds a shell (it can run tmux or edit files directly), so first-class
commands only replace ad-hoc power with a documented, observable interface.
`can_hire` stays a resource gate on spawn — it is about fan-out control, not
security. `project up/down/archive` accept the SESSION NAME (resolved to the
project id via project_list), because the session is what agents and humans
actually see.

Verified with a real agent: a spawned lead briefed to "create a project at
/tmp/evolve" ran `tmm project create /tmp/evolve --session evolve`, verified
with `tmm project list`, and reported done — 21s end to end.

Global flags: `--project <session>`, `--agent <name>`, `--server <ws://…>`,
`--output json` (every read). Token from `config.toml`, overridable with
`$TMM_TOKEN`; server from `$TMM_SERVER`, default `ws://127.0.0.1:<port>`.

## The two hard rules

**Fail soft, never block.** The server is optional (agents-v2 principle 4):
an agent is a plain CLI process in a tmux window and must keep working when
the server is down. `tmm` enforces this with a 2s connect timeout, a 10s RPC
timeout, no retries: a dead server is one stderr line and exit 2, measured at
~20ms for connection-refused. Anything that calls `tmm` from a hook or a
prompt can treat it as fire-and-forget.

**Tiered exit codes** (multica's convention, adopted after reading its CLI
docs): `0` ok · `2` server unreachable · `3` auth rejected · `4` not found
(method missing on this server — mobile or old build) · `5` usage/params.
Agents and scripts branch on the class without parsing error prose.

## Server side

`hub_*` RPCs in `src-tauri/src/server/hub_rpc.rs`, dispatched by prefix in
`connection.rs` exactly like `team_*`:

- `hub_post { session, from, body, requires_reply? }` — room auto-opens.
- `hub_log { session, since_ts?, limit? }` — incremental cursor filters on
  message `ts` (epoch ms) in our layer.
- `hub_status { session, agent, state, note? }` — resolves the agent NAME to
  a window index (telemetry's key) via the window-name match; rejects unknown
  states/names with invalid-params.
- `hub_done { session, agent, summary? }` — records completion AND posts a
  `✔ done` line to the room: the chat is the record.
- `hub_agents { session }` — one row per live window: name, command, agent
  detection (`projects::agents::detect`), derived state.

The room is `proj:<session>` on the same agora bus that Team uses —
`TeamBridge::open_room` provisions it with **no tmux session, no roster, no
supervisor, no workspace history mirror** (empty workspace skips the `.tmm/`
mirror on purpose: a project dir should not grow dot-dirs because someone
chatted). `teams()` filters `proj:*` rooms out so they never appear in the
Team switcher; `recover()` skips them (no tmux session to find) and they
re-open lazily on the next post/read.

Mobile: no bus → every `hub_*` method answers method-not-found (`tmm` maps it
to exit 4). Same degradation contract as `team_*`.

## Status derivation (`projects/telemetry.rs`)

Status is **derived from observed facts**, never self-reported state kept on
faith (§4.3). The store is in-memory, keyed `(session, window_index)` — the
same granularity as a hook notification and a project slot.

| fact | source | verdict |
|---|---|---|
| tool event < 30s ago | hooks (isolated homes, Phase B+) | `working` + activity line |
| explicit `tmm status` | agent | as declared, expires after 30 min |
| `tmm done` | agent | `idle`, summary as detail |
| notification `permission_required` / `input_required` | hooks | `waiting` (fresh pane activity resolves it — an answered prompt self-corrects) |
| notification `completed` (Stop) | hooks | `idle` — a finished turn is REST, not distress. Direct agents fire Stop after every exchange and never call `tmm done`; the old stop-without-done→stuck rule (Team-supervisor era) branded every long-idle window as broken |
| notification `failed` (StopFailure) | hooks | `failed` — the one distress signal we can honestly observe |
| pane activity < 30s | tmux `window_activity` | `working` (hook-poor backends degrade here, not to a lie) |
| nothing | — | `idle` |

Precedence is "latest fact wins", with fresh tool activity beating a stale
declaration and `done` superseding both. The explicit-declaration TTL exists
so a crashed agent cannot stay `working` on its own last words. Notifications
feed the store from the hub's inbox consumer *before* dedupe (dedupe is a
notification-UI concern; telemetry wants every fact). Window records are
dropped when the window disappears (`retain_windows` on every `hub_agents`).

## Managed vs direct windows

Two kinds of windows, presented apart (owner decision, 2026-08-01). MANAGED
agents were spawned from the registry: isolated home, tmm-wired, kicked —
they are chat participants (cards, DM targets). DIRECT windows are everything
else: shells and agents the user started by hand — they are terminal things
and appear only inside the terminal drawer's window list (tagged "direct").
The marker is the isolated home itself: `hub_agents` sets `managed` iff
`<workspace>/.tmm/agents/<window_name>/` exists. Shells never get chat
affordances; a hand-started kiro can still be @-delivered to (useful), but
the chat roster does not advertise it.

## Delivery: how a chat line reaches an agent

The bus stores the record, but an interactive CLI only reacts to what lands in
its input box. On `hub_post`, every `@name` (or `@all`) whose name matches an
AGENT window in the session gets the line typed into its pane as
`[tmm chat] from: body` — an idle agent wakes and acts on it, a busy agent
sees it queued in its input. Shells never receive delivery (typing into a
shell would EXECUTE the message), and the sender's own window is skipped.

## Spawn: the starter pistol

An agent CLI boots into an interactive prompt and does nothing until spoken
to. The brief in the system prompt is context; the KICK — a fixed first user
message passed as the CLI's positional arg ("Start now… run `tmm done` when
complete") — is what makes it move. Without it the agent sat at its prompt
forever (observed live before the fix).

## Verified

Against the live server: `project list` (6 projects, ● markers), `send` →
`log` roundtrip (message in `proj:tmux` with ts cursor working), `status
waiting "note"` → `agent list` shows `waiting — note`, `done` → `idle`,
`--output json` on all reads. Dead server → exit 2 in 21ms; wrong token →
exit 3; `team_status` shows `[]` teams (proj rooms filtered). Unit tests:
derivation table (9 cases), hub dispatch (4 cases), all in `cargo test --lib`.
