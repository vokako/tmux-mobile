# Guidance: UI, Interaction and Motion

> Tenets: 10 (both screens complete; beauty and usability are goals),
> 11 (restraint), 9 (one mechanism).
> The normative specifications are `docs/design-docs/features/design-language.md`
> for tokens/controls/menus and `motion.md` for motion vocabulary. This document
> provides review tradeoffs and checklists rather than repeating their values.
> Review questions: **Are both layouts complete? Does this belong to the
> existing design language? What does the motion guide? Could a person
> misclick, misread or need another unnecessary decision?**
> Draft · 2026-09-09.

## 1. Principles

1. **Both screens are equally complete:** phone and desktop have distinct
   optimal forms of one language. At 760px the form changes, not the standard.
2. **Consistency comes first:** headers, drawers, sidebars, buttons,
   confirmations, to-tail controls, status dots, popovers and vocabulary
   stay consistent across the app (`@all` is called all). Use app controls,
   not native dropdowns or browser context menus.
3. **Restraint:** no fine-line borders; prefer icons, reveal text on hover,
   avoid excessive explanation/color, remove repetition and fold secondary
   content. Owner: "像一个系统状态的监控一样，不要过度占用人的注意力".
4. **Color is vocabulary:** accent means activity, `--status-ok` means a
   successful ending, `--status-warn` means human attention,
   `--status-danger` means failure/destruction, and gray means achromatic rest.
   Brand colors stay fixed; a hue has one meaning within a 5px mark.
5. **Motion guides:** direction conveys depth, return or horizontal travel.
   State changes move through 90-degree rotation, fading or flip reordering,
   instead of replacement. Exits are cuts; layout never animates.
   Avoid flashes, hurried motion and decoration.
6. **Usability has concrete rules:** confirm destructive actions, put
   confirmation at the far right, leave after success and remain after
   failure. Dismiss popovers on outside interaction and align their top-left
   edge with the trigger's left edge. Use native text selection. Back returns
   to the drawer, parent or jump source, never Terminal. Primary actions
   belong at the upper right.
7. **Use the available space:** fill the width, fold instead of crop,
   prioritize complete names, compress paths dynamically, use 1/2/4 columns
   rather than a 3+1 remainder, and align columns.
8. **Follow established app conventions:** iOS-style radii and WeChat/Alipay
   selection highlighting, using foreground color rather than a gray background.

## 2. Required and Forbidden

**Required**
- Use tokens for font sizes, colors, radii, durations and spacing.
  All mono surfaces use `var(--font-mono)`.
- Declare font features only for individual families in `@font-feature-values`;
  select Han glyph forms through `lang` and `hanLang()`.
- Define shared atoms only once in `app.css`: `.side-h`, `.side-row`,
  `.to-tail`, `.live-dot`, `.chev`, `.appear*`, `.state-ctl`, `sheet-up`,
  and `drill-in-*`.
- Give every `infinite` animation a `prefers-reduced-motion` rule.
  Intro animations change only transform/opacity.
- Position fixed popovers through `menuPlacement`, accounting for `--ui-zoom`;
  never place them inside horizontally scrolling containers.
- Measure through the `offsetParent` chain, not `getBoundingClientRect`,
  which includes press scaling, flip transforms and zoom.
- Give global geometry such as `will-change`, `zoom` and `env(safe-area-*)`
  one owner; safe areas use `--sab`.
- The feed's only anchoring guard is `overflow-anchor: none`; layout changes
  use `withReadingAnchor`. Preserve tail-following and return positions,
  fold by characters rather than lines, and keep complete backend records
  with lazy frontend loading.
- During page changes, the old page slides out, the new page slides in and
  content appears with it. Avoid "切过去后闪出来".
- Keep Chinese copy short and tutorials out of the interface. Configuration
  options show their own result, such as font names rendered in that font.

**Forbidden**
- Raw px font sizes, literal colors, a second sliding tempo or `svelte/transition`.
- Scoped CSS overriding shared classes: (0,2,0) silently beats (0,1,0).
- Hiding content with paint, sticky columns, translucent layers or
  pointer-event tricks instead of fixing structure.
- Opacity animation on status dots or color in resting states.
- Native `<select>`, browser context menus or custom text-selection menus.
- 3+1 grid remainders, cropped text or truncated names beside preserved decoration.
- Speculative three-dot buttons, redundant Bedrock/1M/stop suffixes, or
  secondary information expanded by default.
- Calling a UI change complete after checking only desktop or only mobile.

## 3. Review Checklist

- [ ] Were phone (<760px, one-handed) and desktop (rail, splits, resizing)
  checked with screenshots or a real device?
- [ ] Does this match existing controls, rather than adding a third button
  style or second drawer mechanism?
- [ ] Are colors tokens with the right meaning, including green only for successful endings?
- [ ] Does every animation guide something, in the right direction, and stop
  under reduced-motion?
- [ ] Are destructive actions confirmed on the right, with navigation after confirmation?
- [ ] Do popovers dismiss outside, anchor correctly and stay outside scrolling containers?
- [ ] Do Back buttons and gestures return to the origin?
- [ ] Is anything cropped, unnecessarily elided, left in a remainder row or wasting space?
- [ ] Which word, button or background color could be removed without causing user errors?
- [ ] Do `tokens.source.test`, `motion.source.test`, `statusdot.source.test`
  and `sidebar.source.test` pass? Were changed assertions deliberate?

## 4. Lessons

- 2026-08-20: sidebar titles drifted three times, with the third bypassing
  a `.dense`-only check. On 2026-08-30 scoped `relative` overrode the shared
  `.side-sheet` `position:fixed`, leaving half the screen black.
  Two to-tail styles existed on 2026-09-01.
- 2026-08-25: persistent `.page` `will-change` became a containing block,
  offsetting every popover by 46px. The sheet repeated this on 2026-08-30.
  `vh` does not follow zoom; three `env()` writers doubled offsets or read 0 in the APK.
- Three toolbar paint fixes on 2026-08-20 led to "structure beats paint".
  Vitals `pointer-events` fixes were rejected three times before moving
  the footer into normal flow on 2026-09-01.
- A palette pass changed brand colors on 2026-08-21 and was reverted.
  Board done colors were reconsidered three times on 2026-09-01:
  conflicting green, then red/orange/yellow/purple, then red/blue/yellow/purple.
  On 2026-09-08: "绿色好像是有点丑了…颜色调回以前的".
- Running/idle dots were indistinguishable on 2026-08-26 and 08-29;
  achromatic rest and a `.live-dot` halo resolved it. `s-pulse` opacity
  made running darker than idle for half its cycle.
- 2026-09-08: six rounds of unusual Chinese glyphs traced to Inter
  `cv05/cv08` leaking into PingFang SC and selecting traditional forms.
  Owner: "这可是一个深藏的 bug，你一定记好".
- 2026-09-04: a 180-degree rotation was "相当于没有变化", so use 90 degrees.
  Another report was "切换过去然后看到东西闪出来" (#86/#93).
- Four rounds of mobile bubble selection menus on 2026-09-01/04/07 ended
  with native text selection and tap-only Copy/Raw.
- Four Back-gesture fixes from 2026-08-24 to 09-01 established return to
  the drawer, parent or jump source. Files is the reference behavior.
- Board #26/#37: 1/2/4 columns, no 3+1 remainder, left-side indicators aligned.
  #96 fills the width; #92 folds non-agent windows.
- Board #11 uses phone drawers instead of another page and upper-right
  primary actions. #15/#28/#48/#77 require confirmation with the checkmark on the right.
- 2026-08-25: "注意当前我整体比较满意，不要大变样".
  2026-09-03: "按钮尺寸我觉得还好，不用
  调整太大". Standardize; do not redesign.
