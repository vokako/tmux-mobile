---
name: mem
description: Read and write the hierarchical _MEMORY.md memory tree with the mem CLI — durable per-directory project memory that survives sessions. Use at the START of work in a directory (mem context) to inherit recorded facts and decisions, when searching for something you know was recorded (mem search), and whenever you learn a durable fact, make a decision, or get corrected (mem add) so the next session does not rediscover it.
---

# The mem CLI — hierarchical project memory

`mem` maintains a tree of `_MEMORY.md` documents, one per directory that
has decisions of its own. A directory inherits everything recorded above
it, so facts live at the LEVEL they hold: repo-wide truths at the root,
module quirks next to the module.

If `mem` is not on PATH, skip silently — memory is never a blocker.

## The working loop

```bash
mem context [PATH]      # BEFORE touching a directory: every memory document
                        #   from the tree root down to PATH, root first,
                        #   each entry cut to its assertion
mem search <terms...>   # when you know it was recorded but not where;
                        #   a whole question works — it is split on spaces
mem add "<claim>"       # AFTER learning something durable (see below)
```

Read `mem context` at the start of real work in a tree; write `mem add`
the moment a fact is confirmed — not at the end of the session.

## Writing facts

```bash
mem add "<one-line claim>"                # append to ## Memory of $PWD
      -e 'cargo test -p foo'             # verifying command or reference
      -d <dir>                           # write another directory's memory
      -g                                 # tree root: holds EVERYWHERE
      --why                              # a decision/rationale → ## Why
      --supersedes <LINE>                # mark an entry outdated, put this after
      -n                                 # dry-run: show line + destination
```

What deserves an entry: a correction you received, a confirmed approach
and WHY it mattered, a non-obvious constraint, a command that verifies a
claim. What does not: anything the repo or chat history already records.
One claim per entry, one line, no trailing date (added for you).
Attach `-e` evidence whenever a command can verify the claim — `mem
verify` later turns staleness into a failing command.

## Reading and maintaining

```bash
mem tree                # the shape of the whole tree, one line per document
mem show [PATH]         # one document + the sibling files it points at
mem global              # the root alone: facts that hold everywhere
mem context --full      # the reasoning behind every entry (expensive read)
mem context --summary   # titles, summaries, decisions only
mem new <dir>           # skeleton for a directory with decisions of its own
mem rm <line>           # delete one entry by the line it starts on
mem sync                # rebuild generated ## Children regions
mem lint                # format drift, missing dates, broken references
mem verify              # run the evidence commands the facts carry
```

Prefer `--supersedes` over deleting: a superseded entry records that the
old belief existed and was wrong, which is itself a fact worth keeping.
