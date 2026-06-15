# AGENTS.md — how we work as one team

> **Every agent on every team reads this at startup.** It is the shared contract:
> how we talk and move together. It says nothing about who does what — your role,
> goal, and workflow come from your own briefing. Keep it lean.

## We are one team
One shared goal, one working directory, one group chat (the `@team` tools), plus
a human operator. Optimize for the team's result, not your own activity. The chat
coordinates us; the files hold the work.

## The two actions
- **`post(body, requires_reply)`** — say something. Address people with `@name`
  (`@all` = everyone). Set `requires_reply=true` to *oblige* a reply: the bus
  refuses the people you `@` from `wait`-ing until they answer. Use it only when
  you genuinely need a response.
  > **Address whoever should act** with `@name` or `@all`. An unaddressed message
  > is treated as low-priority chatter and may not reach others promptly — only
  > leave it unaddressed for a true FYI that needs no timely response.
- **`wait()`** — receive new messages and the roster. You are refused while you
  owe a reply (it is handed back) — answer first. **End every turn with `wait`**
  so you stay online.

> A reply = another `post` that `@`s the person owed; "got it" does not clear the debt.

## Work in files, not in messages
Put real output — code, docs, results — in the workspace **files**; messages only
point at it ("wrote `src/foo.rs`, please review"). Never paste large content into
the chat — the authoritative state lives in the files. To read someone's work,
open the file.

## Act at the right moment
- **`@`-mentioned = your turn.** Immediately reply that you're on it — a one-line
  "got it, working on X" — so no one is left wondering whether it's being handled;
  then do the work, and report back when it's done.
- **Stay in your lane.** Do the work assigned to you and don't take on what belongs
  to another role — clear division of labor keeps us from colliding or duplicating.
  If something looks unassigned or mis-assigned, ask the manager rather than grab it.
- **Don't act just to look busy.** Step in unprompted only when you're idle AND
  someone posted a *finished* result that's genuinely your concern — never build on
  a half-done intermediate. If the exchange isn't about you, `wait`.
- **Before touching a file someone else may edit, say so** — that's how we avoid
  collisions.
- **Report promptly but tersely:** a line or two — what you produced and where.
- **Nothing for you → `wait`.** The whole team idle with no open debts = round done.

## Memory
Live memory is the chat; durable state is the files. If it isn't in the repo, it
doesn't exist.
