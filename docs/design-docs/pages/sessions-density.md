# Sessions Page — Density & Navigation Redesign

## Context
Original sessions page was built around a conventional "folder with children"
metaphor: each tmux session is a card, auto-expanded on load, panes listed
below. Works fine for 2-3 sessions. Falls apart once you have 10+ parallel
sessions running coding agents — finding "the one where Kiro was working on
proj-X" means scrolling through a sea of visually-identical cards.

User feedback: "I'm running many tasks in parallel, the list is long, I
spend too long finding the one I want. Can you redesign how sessions are
shown and selected so I can get to the one I want faster?"

Core pain points teased out:
1. **Weak identity** — each session shows only name + attached flag + window
   count. Real distinguishing info (what AI is inside, which project, when I
   last touched it) is hidden until you expand.
2. **Visual heaviness** — bordered cards with 14px padding; 5-6 fit on a
   phone screen before scrolling.
3. **Double work** — default auto-expand shows every session's pane list,
   making each card even taller, yet still buries the one useful summary.
4. **No search** — with 10+ sessions, linear scanning is the only option.
5. **MRU invisible** — `last_opened` MRU sort was already there, but there
   was no visible "these are the ones you use most" surface.
6. **Roundabout switching** — switching sessions is Terminal → Back → find →
   expand → tap pane. That's 4 interactions for the thing you do most.

## Decision

### Layout
```
  ⌕ [ search box ...............................]  ×
  [chip A] [chip B] [chip C] [chip D] [chip E]  ← MRU horizontal scroll

  ● main       kiro ~/proj-A     · 2m   [×]
  ● work       zsh  ~/dotfiles   · 5h   [3w] [×]
  ● test       claude ~/scratch  · 1d   [×]
    ...
  [ + New Session ]  [ refresh ]
```

### Information density per row
A single row contains, left-to-right:
`dot · name · (AI icon OR cmd) · cwd-segment · rel-time · Nw-badge? · kill`

Decisions on what survives truncation: `name` and `trailing cluster` are
fixed-width (truncate name at 40% of row width); the middle `meta` section
is `flex: 1` with `overflow: hidden` and absorbs whatever room is left.
cwd drops before cmd.

This is deliberately unglamorous — no icons just for decoration, no avatars,
no progress bars. Density comes from trusting the user to read a row, not
from visuals carrying meaning that isn't there.

### Interaction model
- **Single-pane session**: one tap on the row opens it. No expand step.
- **Multi-window session**: tap toggles pane list expansion. User picks
  which window to open. (We can't guess which is "primary" when there are
  truly N parallel windows.)
- **Search**: live filter across name + cmd + cwd + window_name + AI tag.
  Auto-expands any session that has pane-level matches so the specific
  match is visible.
- **MRU chips**: top 5 recently-opened **AI sessions** (sessions where any
  pane runs a known coding-agent CLI — Kiro/Claude/OpenClaw), excluding the
  currently-active one. Single tap switches. AI-filtered because that's the
  workflow that actually benefits from a 1-tap surface — plain zsh/node/vim
  sessions are easily reached via search / the full list.
- The same chip strip is embedded in the **Terminal page's window switcher**
  when expanded: a horizontal tab bar pinned to the top of the Terminal
  view, holding both the current session's windows and up to 5 other AI
  sessions in one scroll strip, separated by a vertical rule. Collapsed
  state falls back to the existing small floating AI-icon button in the
  top-right (minimal footprint when not in use). One scroll switcher for
  everything — current-session windows and other-session AI chips share
  the same click handler (`onSwitchPane`), so the user doesn't need to
  learn two patterns. Horizontal layout was chosen over the previous
  vertical floating panel because the vertical form took half the viewport
  height when expanded; the horizontal bar is ~40 px tall and doesn't
  eat terminal real estate the way the vertical one did.

### Sort order (unchanged from previous)
1. Currently-active session (pinned top)
2. Sessions with `last_opened`, by `last_opened` descending
3. Never-opened sessions in server baseline order

### Backend change
`TmuxPane` gains a `current_path: String` field, populated from tmux's
`#{pane_current_path}` format specifier. Needed for the inline cwd hint;
serves double duty as a search target.

## Alternatives Considered

### "Command palette" (Cmd+P style)
Ship a modal search that lets you type and fuzzy-match. Would have been
great on desktop but this is a mobile-first app — the keyboard takes over
half the screen and the palette metaphor doesn't quite translate. A
persistent top-of-page search bar gets 80% of the value with none of the
modal-on-mobile awkwardness.

### Grouping by "project" / folder prefix
Auto-detect session prefixes (`work-*`, `perso-*`) and group accordingly.
Rejected: too much magic, too many false groupings, and users with a flat
naming scheme get no benefit. The search box subsumes this: if you prefix
your sessions `work-`, typing `work` already filters them.

### "Task board" view with per-session cards & live previews
Render a small xterm preview of each pane, so you can see "yes Kiro is
still thinking". Rejected: cost (N live subscriptions) vs benefit (we'd
need 16+ pixels of preview to be recognizable, and the AI icon already
tells you it's the Kiro session). Maybe revisit as a feature flag later.

### Pinning / favorites
Let users pin sessions to the top. Rejected in v1 — MRU already approximates
this ("you use it → it stays on top"). Add if user feedback demands it.

## Trade-offs

**Gained**:
- ~3x information density per session (inline cwd + cmd/AI + rel-time).
- Search scales gracefully to any session count.
- MRU chips make "switch back" one gesture.
- Single-pane sessions: one tap to open (was: one tap to expand, one to
  enter — though the old version also auto-expanded, hiding this).

**Lost**:
- Default-expanded pane list is gone. Users who liked seeing all panes at
  once now have to tap. Judged acceptable: a session with 5+ panes
  auto-expanded used to fill the entire phone screen on its own.
- The pane list, when expanded, is slightly less prominent (inset from the
  session row, smaller font). Intentional — it's contextual detail, not
  primary surface.

## Lessons Learned

### "Density" ≠ "smaller fonts"
My first instinct was to shrink everything. That's the wrong axis — you
don't help someone find a session faster by making each row harder to
read. You help them by packing more *identity* into the same row, and
cutting rows that aren't earning their vertical space (removing the
auto-expand default was the biggest single win).

### Meta row must survive on narrow screens
First pass had `name · meta · trailing` all with equal `flex: 1`. On
iPhone mini width the session name truncated before the cwd did, which
is exactly backwards: the name is what you're scanning for. Fixed with
`name { max-width: 40%; flex-shrink: 0 }` + `trailing { flex-shrink: 0 }`
so only `meta` absorbs pressure, and `meta` drops its cwd piece first via
`overflow: hidden` on the inner spans.

### Eager-load panes (not lazy)
The original code only loaded panes on expansion. Summary rendering needs
pane data for every session — so we load it eagerly in `refresh()`. This
is cheap (one `list-panes` per session, < 50ms each, in parallel) and the
UX payoff is huge: you see `~/proj-A · kiro` immediately, without having
to expand anything. Verify this remains cheap if session counts go beyond
~50 — past that we might want to batch or paginate the initial load.

### MRU chips are the "power user" surface
For a user with 3 sessions the chips row is clutter. But for 15+ sessions
— the actual pain point this redesign addresses — the chips carry the
"I'm switching between A and B" workflow. Hiding chips during search keeps
them out of the way when the user is doing the "I know what I want" flow.

### Do not grow i18n keys silently
Added `searchSessions`, `noMatches`, `noSessions`, `justNow`, `minAbbr`,
`hourAbbr`, `dayAbbr` keys. Both `en` and `zh` branches updated in the
same commit — drift here is especially painful to debug because an untran-
slated key renders as literal string and looks almost-right.

### Vertical "panel" switchers eat the viewport
The first Terminal-page switcher draft was a vertical panel (inherited
from the old right-top floating design). Expanded it covered ~50% of the
viewport height on mobile — which is exactly the wrong trade-off when the
whole point of the page is to see the terminal. Swapped to a horizontal
top bar: ~40 px tall, scrolls laterally instead of vertically, and does
not fight the terminal for vertical real estate. Collapsed state kept as
a small floating AI-icon so the default idle cost stays near zero.

General rule: on mobile, if a panel is always-visible or often-visible,
spend horizontal space (scroll laterally) before vertical space. Vertical
"accordions" / "drawers" work when they're modal or rare, but a switcher
that's meant to be glanced at during other work is not rare.
