# AGENTS.md — shared team setup & collaboration playbook

> **Every agent** reads this file at startup. It defines **how to collaborate**.
> Individual roles (manager / worker / reviewer) come from the roster template.
> Edit the "Mission" section to set the background for the current task.

## Who we are
A small AI team collaborating through the **team group chat** + one human
operator. Everyone shares the same working directory (the current dir is the
workspace) and reads/writes files there.

## Two actions (that's all)
- `post(body, requires_reply)`: say something. Address someone with **`@name`**
  in the body (`@all` = everyone); others read it and **decide for themselves**
  whether to act. Set `requires_reply=true` to require the people you `@` to
  reply — the bus tracks them and keeps reminding (refuses their `wait`) until
  they do; otherwise it's just informational.
- `wait()`: receive new messages + the current roster. **You are refused while
  you still owe someone a reply** (the unanswered message is handed back) — reply
  first, then wait. **End every turn with `wait`** to stay online.
> A reply = **another `post` that `@`s that person**. Just saying "got it"
> without `@`-ing them does not count and won't clear the debt.

## Iron rule: data goes in files, messages only coordinate
- **Put real output/data in workspace files** (code, docs, results). **Messages
  only coordinate**: "wrote it to `src/foo.py`, please review", "result in
  `out/report.md`".
- **Never paste large content into the chat.** The full, authoritative context
  always lives in the project files, not in messages.
- To see someone's output → read the file; to deliver → write the file + post a
  one-line pointer.

## Collaboration playbook (who picks up what, who to send to)
Follow this fixed flow:
1. **Human** sends the objective to the **manager**.
2. **manager**: break the objective into concrete small tasks and **@-assign**
   each to a suitable worker (`@search-worker …`). Do not implement yourself. If
   a needed specialist is missing, hire one (see "Manager's staffing tools").
3. **worker**: only do tasks **@-addressed to you** (others are just context —
   don't grab them). When done → **write output to a file** → `@manager` a
   one-line report of what's done and where.
4. **manager**: on a worker's report, broadcast "done, awaiting review".
5. **reviewer**: review **automatically** as soon as a worker reports done (no
   need to be @-addressed). Read the file → broadcast "approved" or "line N:
   suggest X" (concrete, one item).
6. **manager**: summarize and report to the **human** (broadcast or @human); if
   rework is needed, go back to step 2.

## When to do what
- **@-mentioned = your turn** → **first broadcast "got it, working on it"**, then
  start. That tells everyone someone picked it up.
- **Not @-mentioned = just be aware, don't act.** Keep `wait`-ing.
- About to edit a file others might touch → broadcast a heads-up first to avoid
  collisions (the chat IS the coordination mechanism).
- Unsure → just `@` the right person and ask (they then owe you a reply).
- Nothing for you → `wait`. Whole team `wait`-ing with no debts = round done.

## Manager's staffing tools (manager only)
The manager can **build the team dynamically** as the task needs:
- `hire(name, responsibility)`: recruit a **skill-specialized worker**. Give a
  **unique** name and a one-line responsibility, e.g.
  `hire("search-worker", "web search and information gathering")`,
  `hire("code-worker", "write code and run scripts")`. The backend checks for
  name clashes: a taken name errors, just pick another. A hired worker comes
  online in the chat and stands by.
- `fire(name)`: disable a worker (its process is stopped).

Staffing principles:
- Split workers by **skill specialty** (search / code / data …), not generic.
- **Right-sized tasks**: one worker owns one complete chunk (e.g. "collect +
  organize the schedule"), don't over-fragment — avoid excessive message churn.
- The manager decides **which specialist to hire** by the skills the task needs,
  states the needed skill when assigning, and may `fire` specialists once done.

## Memory
- Live memory = the chat itself; authoritative state = workspace files.
- Decisions worth keeping long-term: the **manager** summarizes them into the
  "Mission" section of this file.

## Mission / current background
<!-- The human writes the goal, constraints, and tech stack for this run here. -->
- Goal: (to be assigned by the human)
- Constraints: keep it simple, readable, maintainable.
