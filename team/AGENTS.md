# AGENTS.md — team communication contract

> **Every agent on every team reads this file at startup.** It defines ONLY how
> we talk to each other through the shared group chat — the protocol, not the
> work. Your role, your goal, and the team's workflow come from your own
> briefing (the team template). Keep this file role-agnostic and workflow-free;
> put who-does-what in the template.

## Who we are
A small group of AI agents plus one human operator, collaborating through a
shared **group chat** (the `@team` tools) over one working directory. The chat
coordinates; the files hold the work.

## The only two actions
- **`post(body, requires_reply)`** — say something. Address people with `@name`
  in the body (`@all` = everyone). Others read it and decide for themselves
  whether to act. Set `requires_reply=true` to *oblige* the people you `@`: the
  bus tracks them and refuses their `wait` until they answer. Use it when you
  genuinely need a response; leave it off for FYI.
- **`wait()`** — receive new messages plus the current roster. **You are refused
  while you still owe someone a reply** (the unanswered message is handed back) —
  answer first, then wait. **End every turn with `wait`** so you stay online.

> A reply = another `post` that `@`s the person who is owed. Saying "got it"
> without `@`-ing them does not clear the debt.

## Iron rule: work goes in files, the chat only points at it
- Put real output — code, docs, results — in the workspace **files**. Messages
  only **coordinate**: "wrote it to `src/foo.rs`, please review".
- **Never paste large content into the chat.** The authoritative state always
  lives in the files, never in a message.
- To see someone's work, read the file; to deliver, write the file then post a
  one-line pointer to it.

## Chat etiquette
- **`@`-mentioned = your turn.** A quick acknowledgement so others know it's
  picked up, then act.
- **Not mentioned = stay aware, don't grab it.** Keep `wait`-ing.
- **About to touch a file someone else might be editing → say so first.** The
  chat is how we avoid collisions.
- **Unsure → `@` the right person and ask** (they then owe you a reply).
- **Nothing for you → `wait`.** The whole team idle with no open debts means the
  round is done.

## Memory
Live memory is the chat; the durable, authoritative state is the workspace
files. Anything worth keeping beyond this session must be written to a file — if
it isn't in the repo, it doesn't exist.
