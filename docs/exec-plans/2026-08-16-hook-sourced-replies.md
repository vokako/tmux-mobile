# Hook-Sourced Replies Execution Plan

## Problem

In chat mode a human sees nothing of what an agent said. A reply reaches the
project room only when the agent chooses to call `tmm send`, so visibility
depends on prompt compliance rather than on a mechanism. Tool activity is
likewise invisible for windows the user started by hand.

The `stop` hook already carries the answer. `normalize()` reads
`assistant_response` (via `string_field(payload, &["message",
"last_assistant_message", "assistant_response", "task_subject"])`) but routes it
only into `AgentNotification.summary` for the notification UI;
`record_notification()` pushes a kind string and drops the body. The material for
a complete chat timeline is already arriving and being discarded.

## Measured Facts

Probed against kiro-cli 2.16.2 on this machine. `kiro-cli agent validate`
rejects unknown hook names, so the set is exactly five: `agentSpawn`,
`userPromptSubmit`, `preToolUse`, `postToolUse`, `stop`.

- `stop` payload: `hook_event_name`, `cwd`, `session_id`, `assistant_response`.
  `session_id` is present in interactive mode and absent under
  `--no-interactive`.
- `preToolUse` carries `tool_name` and `tool_input`, where `tool_input.summary`
  is a human-readable description written by the agent itself.
- `postToolUse` adds `tool_response` with `success`, `exit_status`, `stdout`.
- No hook payload carries usage or cost. The v3 trigger set is larger and also
  has none.
- `install_kiro_default()` (`src-tauri/src/agent_notifications.rs:652`) installs
  only `stop`, so direct and adopted windows emit lifecycle events and no tool
  events.
- `feedLevel` defaults to `'status'`
  (`src/lib/hub/hub-prefs.svelte.ts:13`) and `timelineItems()` skips every
  `kind === 'tool'` event below level `'tools'`. The control exists only in
  Settings, not in the Hub surface.

## Division of Labour

A hook observes; `tmm` intends. The boundary is addressing: a hook delivers text
with no recipient, so it can only broadcast into the room for the human to read.
`tmm send` names a target and `hub_post` types that line into the target's pane,
which is a delivery action no hook can imply.

- **Hook-sourced text: record only, never deliver.**
- **`tmm send`: addressing plus delivery.**

`stop` fires only at the end of a turn, so a long turn stays mute. `tmm send`
and `tmm status` therefore keep a role — reporting progress and blockers while
work is in flight, and speaking to teammates — while final results become
automatic.

## Constraints That Must Live In Code

Prompt text cannot enforce any of these.

1. **Same-turn de-duplication.** An agent that calls `tmm send` and then stops
   would post twice. Mark turn start from the `userPromptSubmit` hook; if a
   `tmm send` or `tmm done` occurred during that turn, skip the automatic post.
   Keep `tmm done` — it is a state transition, not only a message.
   The turn-start hook must be installed in **every** config that can auto-post,
   which means `render_kiro` in `projects/spawn.rs` first — managed agents are
   the only windows constraint 3 lets through, and the global `kiro_default.json`
   they do not use. Without it the flag is sticky and one `tmm send` disables
   the auto-post for the rest of that window's life.
2. **Hook-origin text must not trigger delivery.** `hub_post` types into a
   pane whenever it sees an addressed name. If an automatic post is handled as
   an ordinary post, an agent whose reply addresses a peer will land in that
   peer's pane, whose own `stop` then posts again, and the two can ping-pong.
   Skipping the sender's own window does not prevent this. Flag hook-origin
   messages record-only at the call site.
3. **Managed windows only.** The global `~/.kiro/agents/kiro_default.json` used
   by direct windows also contains a `stop` hook. Without a gate, any kiro the
   user starts by hand in any directory begins posting into a project room.
   Filter on the managed flag already computed for `hub_agents`.
4. **A separate length budget.** `MAX_SUMMARY_CHARS` is 240
   (`src-tauri/src/agent_notifications.rs:11`) and is the notification-summary
   budget. The chat path needs its own ceiling in the 4–8 KB range; the inbox
   file limit of 256 KB is not the binding constraint.

## Wiring

The inbox consumer touches telemetry and notification state only, so posting to
a room needs a bus handle. The symmetric precedent is
`crate::projects::set_agent_sessions(notifications.clone())` at
`src-tauri/src/server/mod.rs:199`, backed by
`src-tauri/src/projects/mod.rs:64`. Inject a room poster the same way:
desktop-only, `None` on mobile, degrading exactly as the `team_*` and `hub_*`
paths already do. No new architecture.

Relevant call sites:

| Concern | Location |
|---|---|
| Hook payload normalisation, summary truncation | `src-tauri/src/agent_notifications.rs` (`MAX_SUMMARY_CHARS` :11, truncate :427) |
| Default hook set for direct windows | `src-tauri/src/agent_notifications.rs:652` |
| Activity ring | `src-tauri/src/projects/telemetry.rs:127` |
| Room post handler | `src-tauri/src/server/hub_rpc.rs:40` |
| Injection precedent | `src-tauri/src/server/mod.rs:199`, `src-tauri/src/projects/mod.rs:64` |
| Feed level filter | `src/lib/hub/hub-prefs.svelte.ts:13` |

## Prompt Changes

The agent contract changes shape, so the prompt must change with it. State the
mechanism, not just the rule, so an agent can reason about edge cases.

- **Do not report a final result.** The end of a turn is captured and posted
  automatically. Repeating it with `tmm send` is redundant; de-duplication will
  drop it, so the call is wasted.
- **Do speak while working.** Only turn ends are captured, so a long turn is
  silent until it finishes. Use `tmm status working|waiting|blocked` and
  `tmm send` for progress and blockers.
- **Address a teammate explicitly to reach them.** An addressed `tmm send` is
  typed into that agent's pane. It is a delivery, not a mention — do not address
  someone unless the intent is to interrupt them.
- **`tmm done` still marks completion**, and its summary can now be one line
  because the result itself is already in the room.
- **Never put credentials in a message.** Room contents are persisted and
  rendered to mobile clients.

## Acceptance Criteria

- A managed kiro window's final reply appears in the project room with no
  `tmm send` call.
- A turn containing an explicit `tmm send` or `tmm done` produces exactly one
  room message, not two.
- An automatic post whose body addresses another agent does not type into that
  agent's pane, and cannot start a reply loop.
- A direct or adopted kiro window never posts automatically.
- Replies longer than the notification budget survive into chat, truncated only
  at the chat ceiling.
- Tool events are visible for direct windows once `preToolUse` / `postToolUse`
  are installed, and the feed level control is reachable from the Hub.

## Out Of Scope

- Credit tracking. Hooks carry no usage; the source is
  `user_turn_metadata.usage_info` in the kiro-cli session store, which is not
  written until a session ends. A real-time path exists (a detached, delayed
  pane capture keyed off `stop`) but it needs its own plan and the only
  persistent-storage change in this area.
- Backends other than kiro. Claude exposes per-message token usage through the
  transcript file and codex is unverified here. Leave their columns empty rather
  than inferring.
