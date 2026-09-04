# Sessions Page — Density & Navigation Redesign

## Shared UI tokens

High-frequency navigation surfaces use the global `--ui-*` tokens defined in
`App.svelte`: 31px bars, 24px controls, 4px gaps, pill controls, 7px compact
corners, 9px panel corners, 11px control text, and one fast transition timing.
Terminal, Team, Settings, Sessions, Files, and `AgentChip` consume these tokens
so later density changes remain synchronized instead of drifting per page.

Keyboard focus uses one accent outline globally, and disabled buttons share one
opacity/cursor treatment. Page-specific active states may still differ by
meaning, but should use the same accent border/background pair.

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
`dot · name · (AI icon OR cmd) · rel-time · Nw-badge? · kill`

Decisions on what survives truncation: `name` and `trailing cluster` are
fixed-width (truncate name at 40% of row width); the middle `meta` section
is `flex: 1` with `overflow: hidden` and absorbs whatever room is left.

The cwd-segment was removed from the session row (2026-07): in the cramped
line it was squeezed to the point of being unreadable, defeating its
purpose. Full paths live on the expanded window rows instead, right-aligned
so the informative tail (current folder) is always visible, horizontally
scrollable for the rest.

Team rows (`tmm-team-*`, see "Teams grouping" below) show no meta at all —
just dot + workspace label + a trailing chat glyph.

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
- The Terminal page does not duplicate the MRU strip. Its expanded switcher
  starts with a text-only active `AgentChip` for the current session; tapping
  that chip opens the all-session pane picker. The scrollable remainder holds
  only windows from the current session. This keeps cross-session navigation
  in one discoverable control and leaves the Terminal strip focused on window
  switching. The collapsed state remains the small floating current-window
  chip in the top-right.

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

**Exception — Teams grouping (added 2026-07).** Team sessions
(`tmm-team-<room>`, created by the team bus) ARE split into a labelled
"Teams" group above "Sessions". This does not reopen the rejection above:
the prefix here is not a user naming heuristic but an app-owned protocol
(the server creates these names, `src-tauri/src/team/workspace.rs`), classification
is additionally gated on the server actually having the team bus
(`teamState.available` in `src/lib/core/team.svelte.ts` — shared with
PanePicker so all surfaces agree), and the rows behave differently (tap →
Team chat, not a terminal), so mixing them into the flat list would mislead.
False groupings are impossible short of a user hand-naming a session
`tmm-team-*` on a bus-enabled server.

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

### Collapse is compression, not "become a different widget"
First pass of the horizontal bar kept the old floating glass blur button
as the collapsed state. Users correctly pointed out that the collapsed
state looked like a *different* widget (different shape, different
material, different corner treatment) from the expanded bar — as if
tapping collapsed an *entire component* and revealed *another one*. That
breaks the mental model: users should feel they are working with one
thing that has two sizes, not two unrelated surfaces.

Fix: the collapsed state is now just a single `.win-chip` (same class as
every other chip in the expanded bar), absolute-positioned to the top
right. Same border, same shape, same colors. Visually the switcher has
been compressed to the right end of an invisible bar — which matches the
"collapse to the right" direction that feels natural for a horizontal
strip.

General rule for collapse/expand animations: the collapsed state should
look like a minimum-size version of the expanded state, not a separate
affordance. Chevron icons should point in the direction of the movement
— a horizontal bar that collapses to the right uses `chevron-right` for
collapse and `chevron-left` for expand (pointing toward where the bar
will grow from). Do NOT use up/down chevrons on a horizontal primitive
just because dropdowns use that convention; match the axis of motion.

### One chip, one component
After the first horizontal-bar draft, chip styling was duplicated between
Sessions.svelte and Terminal.svelte with slightly different sizing rules.
Inevitable drift: one page's chip looked noticeably bigger than the
other, and Terminal's session-name tag had a different font from the
Sessions-page chip. Extracted `src/lib/ui/AgentChip.svelte` — one component
that holds all chip visuals (size, padding, border, font, agent icon,
optional chevron, optional label). Consumer pages pick a variant and
pass props; they cannot accidentally diverge on spacing.

This also makes future tweaks cheap: moving chip size down from 28 px to
24 px touched one file instead of three.

### Do not hand-roll pull-to-refresh on a scrollable list
The original Sessions page had a hand-written touchstart/move/end pull-
to-refresh. It mis-fired whenever the user tried to scroll up near the
top of the list. The gesture was ambiguous — "scroll a bit past the top"
vs "pull to refresh" — and the 60 px / 10 px threshold we chose worked
most of the time but produced the ~30 % "the indicator is dangling
halfway, is it going to refresh or not?" bug the user noticed.

Removed entirely. The explicit refresh button in the bottom bar is
reliable, discoverable, and takes one tap. If a future version wants
pull-to-refresh back, it should come from a tested library — not hand-
written gesture detection. On a list that fits on one screen the button
is the right primitive anyway.

### Chips never toggle inline rows
First draft had MRU chips call the same `activateSession(s)` handler as
the session row itself, which (for a multi-window session) toggled the
row's inline pane list. Click a chip at the top, a row somewhere down in
the list silently changed shape. Classic action-at-a-distance bug. Fixed
by giving chips their own handler (`chipOpen`) that always navigates to
a concrete pane — preferring a pane that has an agent running, falling
back to the first pane. A chip tap should ALWAYS take the user
somewhere; never "do something quietly".

### Secondary surfaces on demand, not always-visible
First pass had the search box as an always-visible row at the top — one
row for search, one for chips, stacked. Fine on paper, looked bulky in
practice (two strips of chrome before the actual list). Collapsed search
into a round icon button sharing the same row as the chip strip. Tapping
the button swaps the row into full-width input mode. Chips let the user
switch with one tap without search; search is a click away when needed.

Rule: if a secondary UI is accessed occasionally, prefer "collapsed on
demand" over "always shown". Mobile viewports are too narrow to
permanently spend a row on every potentially-useful action.

## Motion

The list follows [motion.md](../features/motion.md). Every `{#each}` of rows
and MRU chips is keyed by session name and its one child carries
`animate:flip={{ duration: moveMs() }}`, so a session that changes rank (an
activation re-sorts by recency) moves to its new place instead of every row
repainting. The pane list under an expanded row `.appear-rise`s — the row's
height change itself is a cut, never a slide (principle 3). The search bar
fades in when it replaces the chip strip; the press scale on `.session:active`
transitions on `--t-fast` with the row's other colours. There is no
disclosure caret on a session row (the row has no such glyph, and adding one
would be a new element), so the expand has nothing to turn. A split `.cell`
cross-fades only its border colour and ring — never a transform or a flip: it
is an xterm ancestor (principle 9).

Hover / unfold / highlight (#86, 2026-09-04): a session row's hover card
(`use:hoverInfo`, principle 16) shows windows, attached/detached, last
activity (tmux's `session_activity` — that is what the `created` field
carries, see `tmux.rs`), last opened and the summary command; a pane row's
shows command, cols×rows, title and path. Both go through `relTime` and
`sessionSummary`, never a second formatter, and the native `title`s on the
name and the ⋯ buttons are gone (`aria-label` stays). The FIRST list load
shows four `.skel` rows in a `.skel-wrap` (invisible for 150ms) and the rows
then unfold — `.reveal` on a `display: contents` wrapper around the untracked
rows, gated by `listReady` as the first-fill flag; the periodic refresh keeps
the keyed rows' nodes, so nothing replays, and the Projects section above has
its own unfold. The MRU chips are AgentChips in a scrolling strip, not a
segmented control, so no highlight travels there.
