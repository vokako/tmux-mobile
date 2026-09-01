# Agent Notifications

> **2026-09-01: the notification UI half of this system is RETIRED** (owner:
> "原来我用的感觉不是很好用"). The unread inbox (`unread.json`), the
> `agent_notifications_list`/`mark_read` RPCs, the `agent_notification` push and
> every attention dot (Terminal window chips, Sessions rows, PanePicker,
> Projects) are gone: the project room's auto-posted replies + read cursor and
> the derived status dots (hub) carry the signal now. What SURVIVES of this
> design is the event path itself — hooks → helper → inbox → `consume_file` —
> which feeds telemetry (status derivation), the stop-hook auto-post, and the
> per-window conversation-id memory that `up --resume` depends on.

## Purpose

Agent notifications turn structured lifecycle events from coding agents running
inside tmux into persistent, per-window attention markers in tmux-mobile. The
first supported backends are Claude Code, Codex CLI, and Kiro CLI.

Terminal output is not the source of truth. Prompts and completion text change
between versions, themes, models, and languages. Each backend's lifecycle hooks
are the authoritative input; tmux activity and bell flags remain unrelated,
low-confidence signals.

## Event Path

```text
agent lifecycle hook
  -> notification helper writes one atomic inbox file
  -> desktop server consumes the inbox
  -> NotificationHub stores unread window state
  -> existing authenticated WebSocket pushes agent_notification
  -> Sessions and Terminal window chips render attention markers
```

The helper uses a filesystem inbox under tmux-mobile's config directory rather
than a second HTTP listener. This avoids another exposed port and authentication
surface. It writes a temporary file and renames it into the inbox so the server
never reads a partial payload. The hook does not connect to the desktop server:
if the server is stopped, the event remains in the inbox and is consumed on the
next start. Notification delivery is advisory, so helper setup/write failures
are silently ignored and always return success to the Agent CLI. Hook payloads
are single-line JSON; helpers read one line rather than waiting for stdin EOF,
because some interactive CLIs keep the pipe open until the hook exits. Hook
commands invoke the helper through `/bin/sh` instead of executing it directly:
macOS may attach `com.apple.provenance` to app-generated scripts and kill direct
execution with status 137.

## Backend Mapping

- Claude Code: `Notification` maps permission/input/completion notification
  types precisely; `Stop` provides a completion fallback.
- Codex CLI: `PermissionRequest` maps to permission-required and `Stop` maps to
  completed.
- Kiro CLI: `stop` maps to completed. Current Kiro hooks do not expose a
  permission-wait event with the same precision as Claude or Codex. For Kiro
  2.x, installation asks Kiro itself to materialize an editable `kiro_default`
  config from the built-in agent, then merges only the owned Stop hook. The
  installer also writes the v3 workspace/global hook format for forward
  compatibility; it never edits unrelated custom agents.

Duplicate completion events from one turn are collapsed by backend session,
pane, event kind, and a short time window.

## Tmux Identity

Hook processes inherit `TMUX_PANE` from the agent process. The helper records
that stable pane id (for example `%18`). The server resolves it through tmux to
the current `session:window.pane` target. Notifications aggregate at window
level because that is the navigation unit shown in Terminal's chip bar.

Events without `TMUX_PANE`, or whose pane no longer exists when consumed, are
discarded: guessing from cwd or process names can associate attention with the
wrong session. This also makes globally installed hooks inert when an Agent is
running outside tmux.

## Unread Model

Unread state is persisted by the desktop server, not only held by one client.
It therefore survives WebSocket reconnects and mobile app backgrounding.

- `completed`: normal attention marker
- `input_required` and `permission_required`: urgent marker
- `failed`: error marker

Opening the corresponding window calls `agent_notifications_mark_read`. The
server then broadcasts the complete unread snapshot so every connected client
converges. Listing notifications after authentication supplies the initial
snapshot for a reconnecting client.

Terminal chrome deliberately filters attention dots for recognized
`tmm-team-*` sessions: Team already presents Agent status, and repeating the
marker on its session/window chips adds noise. The filter is downstream of this
unread model. Hook ingestion, server persistence, system notification delivery,
and ordinary session dots are unchanged; Sessions remains a complete unread
overview.

## Hook Management

Settings exposes install/status/remove actions in the Connection tab.
Installation is additive: tmux-mobile identifies only hook entries that invoke
its generated helper and preserves all unrelated user configuration. Reinstall
replaces owned entries with the current absolute helper command, which repairs
older quoted-tilde commands that shells cannot expand. Codex may require the
user to approve the new hook once from `/hooks`.

Team-managed agents receive the same hooks in their generated private backend
configuration automatically. They do not depend on the user's global hook
installation.

The generated helper and inbox are private to the local user's config directory.
Payloads are bounded and notification summaries are truncated before
