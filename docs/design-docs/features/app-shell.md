# App Shell — one chrome per context

## Context

Until agents-v2 the app had ONE shell on every platform: a top bar with four
nav pills (Sessions / Terminal / Team / Files) plus a gear. The Hub made it
five, and the top bar started working against both platforms at once:

- On desktop the Hub already contains the projects list and an embedded
  terminal, so the top pills mostly duplicated what the sidebar shows — while
  eating ~45px of vertical space on the page where vertical space is the
  product (a terminal).
- On a phone the pills sit in the hardest-to-reach part of the screen, and
  every added page shrank the tap targets.

## Decision

Three shell shapes, chosen by context, never mixed:

| context | chrome | why |
|---|---|---|
| connected desktop | **left icon rail** (46px, fixed): Hub / Sessions / Terminal / Agents / Files, gear at the bottom | the VSCode/Slack pattern — switching stays one always-visible click, the top edge belongs to content, and the rail composes with the Hub's own sidebar instead of stacking a second horizontal bar over it |
| connected mobile | **bottom tab bar** (icons + labels, safe-area padded) | thumb reach; the top edge goes back to content. Hidden under `html.keyboard-open` so immersive typing (terminal, editor) costs nothing — the ONLY writer of that class is App's viewport handler, so there is no second source of truth |
| disconnected | the old **top brand bar** with the gear (both platforms) | before auth there is nothing to navigate; brand + settings is the whole story |

Desktop lands on the **Hub by default** (fresh state); a guard falls back to
Sessions once the bus probe answers negative (`hub`/`agents` need the bus).

The Hub is CHAT-FIRST (owner-directed rework 2026-08-01): a project's default
view is its conversation, full width; the terminal is a DRAWER behind a
button — terminal is terminal, project is project, never parallel equals.
Agent definitions live on their own Agents page (rail icon / bottom-bar tab).
The Team tab is retired — the Hub's per-project chat + spawn + telemetry
replaced it; the team_* backend stays (the bus IS the hub substrate) and
legacy tmm-team-* sessions appear as plain sessions in the list.

## Mechanics worth recording

- The rail is `position: fixed`; content clears it via `main.with-rail
  { padding-left: 46px }` rather than a flex re-nest, so the page-layer
  keep-alive structure (Team/Files/Terminal/Hub stay mounted) is untouched.
- Fixed overlays cannot assume a top bar anymore. `Preferences` used
  `inset: calc(49px + var(--sat)) 0 0`; it now reads
  `--shell-top` / `--shell-left`, which App sets per context (49px top when
  disconnected, 46px left on connected desktop, zero on connected mobile).
  Anything else `position: fixed` that wants to clear the chrome should use
  the same two vars.
- The mobile tab bar hides with CSS (`:global(html.keyboard-open) .tabbar
  { display: none }`), not JS — it inherits exactly the keyboard lifecycle
  the terminal already fights hard to get right, including the Android
  native-height path.
- Tab order for swipe navigation is unchanged (`tabs()`), and the gear is a
  toggle, not a page — it never participates in swipes.

## The rail has two groups: where you work, and what you configure

Top: Chat, Terminal, Files — the places work happens. Bottom, after the
spacer: Agents (agent/skill/MCP definitions) and Settings. Agents moved down
there on 2026-08-19 (owner ask): it is a configuration surface, visited when
setting something up rather than while working, and pairing it with the gear
says that without a label. Order within the pair puts Agents above the gear,
which stays the last item in the rail as the app-wide convention.

## The tab slide belongs to the swipe, not to the app

`switchTab` plays a one-shot `slide-in-left/right` on the page layer — but only
on a TOUCH layout (`layout.isTouchDevice`, 2026-08-19). It is the visual half
of the swipe gesture: content follows the finger, so the direction carries
meaning. A desktop switches tabs by clicking a rail button, where nothing moved
horizontally and the slide just made the page lurch (owner report) — most
visibly on the wide three-column pages, where a whole workspace slid. The
animation is not deleted, because on a phone removing it would leave a swipe
whose content does not follow the thumb.

## Verified

Desktop 1440x900: rail with Hub/Sessions/Terminal/Team/Files/Settings, no
top bar, `main` padding 46px, Hub as the fresh-state default, Files
switching, Preferences opening at `left: 46px`. Mobile 390x844 (touch
emulation): bottom bar with active states, tapping a project window lands in
a rendering terminal with the bar visible, `keyboard-open` computes
`display: none` for it. Disconnected: top brand bar, no tab bar.
