# The design language

One normative reference for every surface — layout, type, colour, controls,
menus, motion, touch. Written on owner request (2026-08-25: "你重新梳理我们ui
的设计语言…设定好一套规范 让以后的新功能也能完美和谐统一。注意当前我整体比
较满意，不要大变样") — so this codifies what the app already is, and new work
matches it instead of inventing. Deep rationale lives with each feature
(`tmm-cli.md` §Conversation visual language, `ui-unification.md`, `fonts.md`);
this file is the contract. `src/lib/ui/tokens.source.test.ts` and
`src/lib/ui/sidebar.source.test.ts` enforce the mechanizable parts.

## 1 · Tokens (app.css `:root` — never restate a value)

- **Type scale, six chrome steps**: `--fs-micro 9 · --fs-meta 10.5 · --fs-sub
  11.5 · --fs-ui 12.5 · --fs-body 13.5 · --fs-title 15`. Connect card only:
  `--fs-hero/--fs-display`. `--ui-font-control: var(--fs-sub)` for menu rows,
  steppers, segments. A raw px font-size anywhere is a regression (guarded).
  `--fs-input-touch: 16px` is a BEHAVIOUR (iOS focus auto-zoom), not a step,
  and is gated `@supports (-webkit-touch-callout: none)`.
- **Three font roles** (`fonts.md`, all three user-overridable per device):
  `--font-ui` (Inter Variable) = content: prose, inputs, documents.
  `--font-display` (Space Grotesk Variable) = identity: page titles, `.side-h`
  headers, names, the brand, and every `button` that is chrome.
  `--font-mono` = data: terminal, code, paths, ids, readings, window chips.
  A data-carrying button (file row, dropdown value) opts back out of the
  button rule with an explicit font-family.
- **Radius scale**: `--ui-radius-control 10` (buttons, inputs, selects) ·
  `--ui-radius-row 12` (cards, rows) · `--ui-radius-panel 14` (menus, panels)
  · specials: bubbles 18/6, composer 16/15, dialogs 18 · true pills
  (`--ui-radius-pill`) ONLY for micro tags ≤ `--fs-micro` and stadium chips
  (`.pick`, day pill). Controls are never pills.
- **Colour**: theme tokens only — `--bg*`, `--surface*`, `--text*`, `--border*`,
  `--accent*` (`--accent-fill/-ink` for solid CTAs, `--accent-line` for
  selection borders, `--accent-bg` for washes), `--danger/-bg`. ONE progressive
  status language everywhere a state shows: accent = in motion, `--status-ok`
  = ended well, `--status-warn` = needs a person / a turn cut short,
  `--status-danger` = failed/destructive, grey = at rest. A literal colour is
  wrong; compute expressions with `color-mix` over tokens (`ctxColor`).
- **Motion**: `--t-fast 120ms` = micro feedback (hover, border, colour);
  `--t-move 200ms` = things that move or resize (drawer, bars, width).
  Spinner tempos are semantic, not tokens: 0.6s = loading, 2.2s = "a turn is
  open" (send button), breathe ≈ 1.3–5s (presence). Every looping animation
  stills under `prefers-reduced-motion`.
- **Navigation motion grammar** (owner, 2026-08-25: "对于交互动效也应该有
  规范，大家都共同遵守"): ONE slide language, 120ms linear translateX(40%),
  for every navigation — no fades, no scales, no second tempo. Direction is
  meaning: going DEEPER (opening Settings, a drill-down category, an editor)
  enters from the RIGHT; going BACK (back gesture, back button, closing
  Settings) enters from the LEFT; lateral tab switches slide in the
  direction of travel. Page-level slides are TOUCH-ONLY (`slidePage` in App
  — desktop tabs are a rail with no motion behind them, owner rule);
  compact drill-downs animate under 760px via the shared `drill-in-right`/
  `drill-in-left` keyframe pair (Settings, Agents). SHEETS (sidebar
  slide-overs, phone dialogs) instead slide on `--t-move` with a scrim.
  POPOVERS (menus, selects) do not animate — they are placed, measured,
  shown. A transform hint (`will-change`) may exist only WHILE a slide runs:
  a resting transform turns the page into a containing block and breaks
  every fixed popover.
- **History discipline for drill layers**: a navigation that OPENS a layer
  (entering Settings, a compact drill-down) pushes its history entry AT OPEN
  TIME, and the pop that peels it SPENDS that entry (no re-push). Reason:
  the browser's predictive-back preview slides in a SCREENSHOT of the entry
  being returned to, captured when it was last current — entries pushed only
  at tab switches made backing out of a Settings level flash an unrelated
  page for a beat (owner, 2026-08-25: "疑似闪一下其他页面"). Pushed at open,
  every preview shows the true destination. Two fallbacks keep the stack
  honest: an open that pushed nothing (reload landed there; drill opened
  outside compact) closes DIRECTLY instead of calling history.back() into
  the stack-bottom re-push guard. Layers that only PEEL state without a
  matching open-time push (Hub menus, dialogs) keep the consume-and-re-push
  model.

## 2 · Layout skeletons

- A page = optional LEFT SIDEBAR (`var(--sidebar-w)` grid column, `--bg2`,
  1px `--border` right) + main column headed by `.page-head` (h1 in
  `--font-display` at `--fs-title`; actions as `.icon-btn`, right-aligned).
- The ONE compact breakpoint is **760px**. Under it a page picks ONE of two
  patterns, never a third: the sidebar becomes a slide-over SHEET
  (Chat, Terminal — scrim + `--t-move` slide), or the page DRILLS DOWN
  (Agents, Settings — the list is the first screen, the opened thing takes
  the whole screen, a chevron-left `.icon-btn` and the back gesture return).
- Every page registers `onGoBack` (Files' contract) and peels its layers in
  tap-outside order; a consumed pop re-pushes. A page never installs its own
  popstate listener.
- Sidebar internals are the SHARED atoms in app.css (`.side-h`, `.side-row`,
  `.side-age`, `.side-win*`): components may position them, never restyle
  them (guarded).
- `.page` must never become a containing block (no resting transform/
  will-change/filter): every fixed popover assumes the VIEWPORT. Any fixed
  sheet pads with `var(--sat)/var(--sab)`, never raw `env()` (0 in the APK).
  Overlay vh/vw sizes divide by `--ui-zoom`.

## 3 · Control dialects (reuse, never invent)

- `.chip-btn` — bordered text chip; `.primary` accent wash; lone `.danger`
  quiet until hover. `.chip-btn.primary.danger` = SOLID red with white ink,
  reserved for the confirming button of a destructive dialog.
- `.icon-btn` — 28×25 bordered icon square; the page-head action dialect;
  label goes in `title` + `aria-label`. `.danger` variant reds on hover.
- `.side-row` — sidebar list row; hover `--surface2`, open/selected
  `--accent-bg`.
- `.pick` / `.agent-pick` / `.pchip` — stadium toggle chips for MEMBERSHIP
  (skills, roster picks); selected = `--accent-line` border + `--accent-bg`.
- Inputs — the dense field dialect: `--input-bg`, 1px `--input-border`,
  `--ui-radius-control`, `--fs-ui`, padding 6px 9px; focus = accent border.
  `ui/Select` is the ONE dropdown (its `editable` mode is the one combobox);
  a native `<select>`/`<datalist>` is a regression.
- Segmented rows / steppers (Preferences) — `--ui-control-height`,
  `--ui-radius-control`, `--ui-font-control`; active = accent border + wash.
- Dialogs — `ui/ConfirmDialog` for every confirm; phone = bottom sheet with
  44px buttons. Solid red confirm per above.

## 4 · Hover / active (desktop), two families only

- CONTROLS (chips, icon buttons, inputs, select triggers): border →
  `--accent`, text → `--accent` (danger controls red instead). No fills.
- ROWS & MENU ITEMS (side rows, menu buttons, list rows): background →
  `--surface2`, text → `--text`; toned verbs keep their tone and tint their
  wash (`color-mix` 14%).
- Solid CTAs brighten (`filter: brightness(1.07)`); pressing scales 0.93–0.95.
  All at `--t-fast`. Hover is never the only affordance (phones exist): a
  hover-revealed control must have a tap/long-press route.

## 5 · Menus & popovers

- ONE popover mechanism: `position: fixed` layer placed by `menuPlacement`
  (`anchorOf`/`pointAnchor` divide by `--ui-zoom`), styled `--bg` surface,
  1px `--border`, `--ui-radius-panel`, shadow, `--ui-font-control` rows;
  invisible until measured. Dismissal set: outside pointerdown, Escape, any
  ancestor scroll, resize — and every transient layer auto-hides after its
  job (owner rule, 2026-08-22).
- Right-click and long-press are ONE gesture (`ui/ContextMenu` +
  `ui/longpress`), offering the verbs the surface already has elsewhere.
- Menu ORDER = rising consequence: read/constructive verbs first, configure
  next, amber interrupt-class verbs after, red destructive verbs LAST.
  Tones sit on the verb at rest, not only on hover.

## 6 · Touch

- Primary actions ≥ 44px hit area — small visual boxes grow an invisible
  `::before/::after` overlay. Menu rows ≥ 40–44px on compact. Keyboard
  equivalents (Enter/Space) on anything with role=button.

## 7 · Data honesty (the "verdict" rule)

"Empty" and states are VERDICTS, not defaults: render nothing until the first
answer arrives (`roomReady`, `listReady`), keep last-known data on a failed
poll ("could not ask" ≠ "there is nothing"), and cache-restore on switches so
nothing flashes.

## 8 · New-feature checklist

sizes from `--fs-*` · fonts by role · colours by token · radius from the scale
· controls from §3 · hover from §4 · popovers from §5 · 760 compact + onGoBack
· 44px touch · transitions from `--t-*` · reduced-motion for loops · no new
species without retiring the old one — and update THIS file when a rule earns
an exception.
