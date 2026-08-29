---
name: tmm-cli
description: The full tmm CLI reference — project chat, background tasks, agent/project self-management, the central registry. Use when you need a tmm capability beyond the basics your system prompt teaches (send/log/status/done), e.g. running a long command as a background task, spawning a teammate, restarting a stuck agent, or managing projects and registry definitions from the command line.
---

# The tmm CLI

`tmm` is how an agent talks to the tmux-mobile project hub and manages its
own workspace. It is fail-soft by design: if the server is unreachable it
exits 2 in ~20ms and NEVER blocks — telemetry must not stop your work.

Exit codes: `0` ok · `1` local/tmux failure · `2` server unreachable ·
`3` auth · `4` not found · `5` usage.

Context flags on every command: `--project <session>` (default
`$TMM_PROJECT`), `--agent <name>` (default `$TMM_AGENT`, else `human`),
`--server <ws://host:port>`, `--output json` for machine-readable output.

## Chat (the basics, for completeness)

```bash
tmm send "@name message"        # post to the project chat; @name types into
                                #   that agent's pane and INTERRUPTS them,
                                #   @human addresses the operator
tmm send "..." --image <path|url>   # attach an image by REFERENCE (repeatable)
tmm log [--since <ts>] [--limit N] [-f]   # read chat; -f follows
tmm status working|waiting|blocked "<note>"  # what you are doing NOW — the
                                #   note is the point, it shows in the chat
tmm done "summary"              # declare the briefed task complete
```

Progress is ambient (`tmm status`), messages are addressed (`tmm send`).
A `tmm send` interrupts its reader — keep it for questions, decisions,
results that need a person.

## The task board — shared kanban, humans and agents alike

The project has ONE board (todo / doing / review / done). The human writes
issues on the board page; you read and update the same issues here. If your
task matches a board issue, keep that issue current — the board is the
plan's record, and a stale column misleads everyone who schedules by it.

```bash
tmm board                       # the whole board, grouped by column
tmm board show <id>             # one issue + its note thread
tmm board add "title" [--body <text>] [--assignee <name>]
tmm board take <id>             # claim it: assignee = you, status = doing
tmm board move <id> review      # done working — hand it to review
tmm board move <id> done        # accepted
tmm board note <id> "found the root cause in store.rs; fix is one COALESCE"
```

Conventions that keep the board honest:
- `take` before you start — two agents editing one issue's territory is the
  conflict the board exists to prevent.
- `note` decisions and findings ON the issue (not only in chat): the issue
  outlives the conversation and is what the next reader loads first.
- `move <id> review` when YOUR part is done — this is a HANDOFF, not a
  label: the issue's reporter is notified automatically (the line lands in
  their pane) and reviews it. Only the reviewer moves it to `done`; if it
  needs fixes they `note` what to fix and move it back to `doing`.
- Board status is the ISSUE's lifecycle; `tmm status` is YOUR live turn.
  They are different axes — an agent can be `running` on something else
  while its issue sits in `review`. Keep both current: `tmm status` for
  what you are doing right now, the board for where the work stands.
- Every status change is recorded in the room (`[tmm] board #N a → b`),
  so the chat shows the flow without anyone narrating it.

The ideal loop (and who moves what): the human (or lead) files the issue →
the lead assigns it (the assignment lands in the assignee's pane) → the
assignee `take`s it (todo → doing) → finishes and `move review` (the
reporter is notified) → the reporter accepts with `move done`, or `note`s
fixes and moves it back to `doing`.

## Background tasks — LOCAL tmux, no server needed

The one subtree that works even when the server is down. Each task is a
tmux window with window-scoped `remain-on-exit`, so status and logs
survive the command exiting.

```bash
tmm task start <name> -- <cmd...>   # run detached in its own tmux window
      [--session <s>]               # default: your session, else "tmm-tasks"
      [--replace]                   # take over a name a live task holds
tmm task list                       # every task, in every session, + state
tmm task status <name>              # running | exited:<code>  (exit 4 if gone)
tmm task logs <name> [--limit N] [--grep <text>]   # default 50 lines, tail
tmm task stop <name>                # C-c, then TERM, then KILL; keeps the log
tmm task rm <name>                  # close a finished task's window
```

Use a task for anything long-running you would otherwise foreground: dev
servers, watchers, builds. Prefer `--replace` over stop+start when
retaking a name.

## Agents — manage yourself and your teammates

```bash
tmm agent list                      # who is here and their derived states
tmm agent interrupt <name>          # cancel the turn it is RUNNING
                                    #   (types Escape into its pane — a chat
                                    #   message is only read between turns)
tmm agent stop <name>               # stop the process (slot survives)
tmm agent restart <name>            # bring it back, resuming its conversation
tmm agent remove <name>             # eject: stop + forget slot + delete home
tmm spawn <registry-name> [--brief "<text>"]   # hire a teammate into this
                                    #   project; no brief = it waits silently
```

## Projects

```bash
tmm project list
tmm project create <path> [--name n] [--session s] [--with-agent kiro|claude|codex|grok]
tmm project up <session>            # bring the tmux session up (recreates
                                    #   missing windows, relaunches agents)
tmm project down <session>          # kill the session, KEEP the declaration
tmm project rename <session> --name "New name"
tmm project archive <session>       # forget the project (session survives)
tmm project delete <session>        # forget it AND delete its agents' homes
```

## Registry — central agent/skill/MCP definitions

```bash
tmm registry list
tmm registry save --name <n> --backend <kiro|claude|codex|grok>
      [--system <text>] [--model m] [--effort low|...|high]
      [--skills a,b] [--mcp <json>] [--can-hire]
tmm registry delete <name>
tmm skills list                     # the app-managed skill store
tmm skills save --name <n> --source <abs dir|github url>   # import files
tmm skills refresh <name>           # re-sync from the recorded source
tmm skills delete <name>            # (built-in skills refuse: they reseed)
tmm mcp list|save|delete            # central MCP server defs (the catalog;
                                    #   calling MCP tools is the mcp-cli skill)
```
