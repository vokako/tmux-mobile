# Guidance: The Agent Bridge

> Tenets: 3 (real CLI), 5 (three primitives), 6 (CLI first),
> 8 (derive, never self-report).
> Review questions: **How does this message enter the pane? Which edge
> establishes this state? Can an agent perform the action itself? Can a
> person see it in the terminal?**
> Draft · 2026-09-09.

## 1. Principles

1. **Three primitives, no fourth:** text into panes, passive outgoing hooks,
   active outgoing `tmm` commands. Every collaboration feature must reduce
   to one of them.
2. **Leave the harness intact:** use documented CLI integration points.
   Prompts are files; model and effort belong in configuration; MCP uses
   native configuration; hooks use the CLI's own locations. Harness means
   behavior. Display-only settings such as Claude `statusLine` are allowed,
   as is observing screens for vitals or recovery.
3. **Derive status from turn edges:** `userPromptSubmit` opens a turn;
   `Stop`/`StopFailure` closes it. Pane activity is not work, `idle_prompt`
   is not a question, and an agent's `tmm status` is a description, not state.
4. **One reply edge:** hooks capture the final response and return it to
   every party whose request the turn carried. `[reply]` creates no reverse edge;
   hook-originated text is recorded, not delivered.
5. **Humans and agents read the same record:** the room is the only log,
   and every line typed into a pane is human-readable.
6. **Prompts describe process, not values:** explain message flow, history
   lookup, addressed replies and completion. Leave other details to tools'
   own documentation and skills.
7. **Prompt-only constraints eventually fail; enforce them through mechanisms.**

## 2. Required and Forbidden

**Required**
- Verify that the destination is an agent's input, not a shell that would
  execute the delivered text.
- Wait 200ms between text and Enter (codex 0.148.0 treats immediate Enter
  as pasted text). Use bracketed paste for text containing `\n`: tmux drops
  raw `\n` under extended-keys. Close kiro's `@` picker with Escape first,
  using its two footer sections rather than the backend name to detect it.
- Match `userPromptSubmit` echoes for delivery acknowledgment: ignore
  whitespace differences, account for truncation, use a queue rather than
  a single slot, and persist across restarts.
- Install a turn-start hook in every hook configuration that can auto-post.
- Recovery sends once per distinct error, uses exponential backoff, and
  relies on hook confirmation rather than disappearing screen text.
- On external interruption, reset derived state before sending Escape;
  external cancellation provides no stop edge.
- Hook helpers are fire-and-forget, read one line without waiting for EOF,
  run through `/bin/sh` and drop events without `TMUX_PANE`. Scan inboxes
  in the blocking pool.
- Keep a backend's hook installation and payload-reading contract in one file.
- Send complete, untruncated, concise handoffs to agents: who assigned whom,
  subject, note and requested action, in that order.
- Notify the assigned agent when a person manually changes board state,
  except when marking it done.

**Forbidden**
- Synthetic first messages without a brief, or system-only messages hidden
  from agents.
- Speaking for an agent in the background or silently injecting prompt text.
- Blocking an agent on `tmm`, making it a prerequisite, or letting its failure
  stop the agent's work.
- Deriving running/waiting from pane activity, screen keywords or self-report.
- Reading effort/model by scanning the whole screen. Anchor to structure:
  the agent-name row or the paint block at column 0.
- Repeating `tmm --help` in prompts, adding value speeches or speculative rules.

## 3. Review Checklist

- [ ] Does the new message path reduce to one of the three primitives?
- [ ] Is the pane input human-readable and stamped with `[tmm chat …]`?
- [ ] Was the target verified as agent input rather than a shell?
- [ ] Which hook edge establishes the state change? Is it being inferred
  from screen text or self-report instead?
- [ ] Was this hook event measured in each backend's dialect, with versions recorded?
- [ ] Are hook installation and reading logic in the same file?
- [ ] Will existing agents receive the new hooks/prompt at next launch?
- [ ] Do automatic recovery, continue and notification messages have
  deduplication, backoff and hook acknowledgment?
- [ ] Is there a corresponding `tmm` command, with CLI/UI parity?
- [ ] Do prompt/AGENTS.md changes describe process only? Can they be shorter?

## 4. Lessons

- 2026-08-16: treating `window_activity` newer than stop as working left
  every agent permanently working.
- 2026-08-16: delivering hook-originated text typed @name into another pane,
  triggering stop, reply and another turn. Enforce `record_only` at the call site.
- 2026-08-16/22: `userPromptSubmit` was installed only globally; claude/codex
  lacked it. After the first `tmm send`, a sticky flag suppressed all later
  auto-posts from that window.
- 2026-08-19 to 09-03: seven delivery-acknowledgment fixes covered queued
  lines falsely marked unconfirmed, overwritten single-slot pending state,
  newlines dropped by tmux, queues lost on restart, server echoes truncated
  at 1024 characters, kiro 2.18's `@` picker consuming Enter, and a
  1041-character paste whose acknowledgment stayed an empty ring.
- 2026-08-21: `tmm done` suppressed auto-post, losing the final reply from
  every turn ending with done. Skip only an exactly identical reply.
- 2026-08-26/27: persistent error text caused repeated continue messages;
  use a `request_id` signature and hook confirmation.
- 2026-08-26: full-screen effort matching picked a table cell and left a
  permanent false reading.
- 2026-08-29: external Escape had no stop edge, leaving status running;
  reset state before Esc.
- 2026-08-31/09-05: kiro hard line breaks defeated `-J` joining and error
  headers scrolled outside the fold, silently disabling recovery detection.
- 2026-09-02: Claude `idle_prompt` 60s after completion created false
  waiting state (board #75).
- 2026-09-03: board dispatch truncated at 400 characters lost information
  (#79); assignment omitted its note (#39).
- 2026-09-08: `tmm status waiting|blocked` and notification UI were removed.
  Fewer mechanisms reduce the agent's interpretation burden.
- Codex still failed with "always end with wait" in its prompt, leading
  to Stop-hook keepalive.
- 2026-08-18: "多此一举"; do not send a synthetic first message without a brief.
- 2026-09-07: the owner's high-priority guidance was to search
  `tmm log --grep` or ask directly when context is unclear, answer every
  addressed message, and consolidate queued replies.
