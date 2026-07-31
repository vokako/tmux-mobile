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

# human-facing
tmm agent list                       windows + agent detection + derived state
tmm project list                     ● live / ○ down, session + path
```

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
| notification `permission_required` / `input_required` | hooks | `waiting`, never stuck |
| notification `completed`/`failed`, no done | hooks | `waiting`, → `stuck` after 180s quiet |
| pane activity < 30s | tmux `window_activity` | `working` (hook-poor backends degrade here, not to a lie) |
| nothing | — | `idle` |

Precedence is "latest fact wins", with fresh tool activity beating a stale
declaration and `done` superseding both. The explicit-declaration TTL exists
so a crashed agent cannot stay `working` on its own last words. Notifications
feed the store from the hub's inbox consumer *before* dedupe (dedupe is a
notification-UI concern; telemetry wants every fact). Window records are
dropped when the window disappears (`retain_windows` on every `hub_agents`).

## Verified

Against the live server: `project list` (6 projects, ● markers), `send` →
`log` roundtrip (message in `proj:tmux` with ts cursor working), `status
waiting "note"` → `agent list` shows `waiting — note`, `done` → `idle`,
`--output json` on all reads. Dead server → exit 2 in 21ms; wrong token →
exit 3; `team_status` shows `[]` teams (proj rooms filtered). Unit tests:
derivation table (9 cases), hub dispatch (4 cases), all in `cargo test --lib`.
