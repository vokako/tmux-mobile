# Hub — the composer

Who a message goes to, the agent strip, `/commands`, readline editing and
per-agent interruption. The feed is `hub-feed.md`.

## Rules and their reasons

Each entry is a decision with the reason it was made; treat them as normative. They lived in the root `CLAUDE.md` until 2026-09-02 (board #73), when that file became an index and the rules moved next to the design they belong to.

### Comparable avatar meters and content-sized cards (#180, 2026-09-12)

Owner, 12:13, requested a narrower card and a meter integrated with it:
> Context window 百分比进度条：这个进度条现在看起来还是有点丑，感觉像在边缘框上多了一条线，这条线和原来的框没有任何关联。

After comparing card-ground fill and an avatar ring, the owner superseded
the temporary fill choice at 14:26:
> 背景色有一个不好的是有可能卡片宽度不一样，大家对进度感知不一样，要不用圆环的方案吧，注意头像一定用圆形，圆环刚好包括头像大一圈。然后 agent 卡片现在右边的空白太多了，就自适应卡片宽度，不要 撑开这么多。agent 卡片现在高度有点太低了，已经贴近头像边边了，可以稍微留一点边冗余，包括给进度圆环留冗余，还有颜色，就绿黄橙红，不要有中间插值不好看的过渡色，看着颜色怪怪的

Use one 26px outer ring around a circular 20px avatar: 1px of ground
separates the image from a 2px stroke. The card paints 30px high for pointers
and 34px for coarse input, leaving 2px/4px outside the ring. Its container
includes the existing paint inset; native commands remain 28/44px. Avatar
and ring stay round through the one app.css corner-policy owner.
The existing meter exposes its bounded value and exact reading through ARIA;
expanded cards also show the exact percentage. Unknown means no ring,
zero has an empty track, and over-capacity values fill one circle while
retaining their exact text.

`ctxColor` is the single capacity classifier: green below 50%, yellow at
50-69%, orange at 70-84%, red at 85% and above. These four existing status
tokens replace the interpolated CLI-statusline ramp. The 85% imminent-
compaction anchor remains; neither CSS nor JS interpolates between hues.
The ring has fixed geometry, independent of name/card width; the old edge
bar and the rejected ground-fill prototype are not production mechanisms.

The newer auto-fit request supersedes the 2026-09-11 08:41 hover-reveal
decision quoted under #173 below. Cards no longer reserve invisible Watch
or Stop slots. Busy cards carry their real Stop; idle cards carry no action.
Watch remains in the same right-click/long-press ContextMenu on both input
types. Hover never expands a card or covers its name with actions. The
expanded, height-bounded view wraps the same content-width keyed cards
instead of stretching them into fixed columns. Recency ordering, input-
held order, disclosure persistence, reading transaction and status dots
retain their existing owners.
During the existing pointer press lifecycle only, rendered Stop membership
is held so a turn ending cannot remove the pressed target or shift its
neighbors. Current busy membership still disables an ended Stop; release
on pointer-up/cancel adopts current layout, without a timer or another
classifier. Expanded percentage text reserves four monospace characters
(the wire value is u8), so changes from 0% to 100% do not move targets.
Chromium 152.0.7977.64 verifies 26/20px concentric geometry, circular
avatars, 30/34px paint, native target floors, content widths, hover stability,
menu Watch and the four colour boundaries across desktop/390px light/dark,
wide touch, reduced motion and 360px. A forced zero-fraction ring fails the
computed visual-fraction check and recovers after removing the override.
The mounted press regression proves slot retention with live disabling;
native-device gesture and owner-build acceptance remain separate gates.

### Compact card paint and secondary All (board #176, 2026-09-12)

Historical geometry: #180 above replaces the edge meter, reserved action
slots and 24/32px card paint, not the native scrolling/selection mechanisms.

Owner, 00:02 (repeating the 2026-09-11 screenshot report), verbatim:
> 这个card行太高了，看着太难看，你要做的精致一点，还有类似检查一下，很多卡片按钮什么的，不要这么傻大的，上下文的进度条稍微粗一点明显一些。还有这个展开箭头和卡片，中间直接硬截断了，不好看，要融合完整一些。
> 还有everyone按钮太大了，而且占据了主要位置，不好用

The strip had kept full-height card backgrounds after commands adopted inset
paint. Cards now use the same `--control-paint-inset`: 24px pointer / 32px
coarse paint inside unchanged 28/44px native targets. Remove the extra 4px
outer vertical padding; the ordinary strip becomes 32/48px, including its
2px scrollport padding. Skeletons use the same paint inset. Selection and
filter outlines follow the painted boundary rather than crossing the
20px avatar in the compact interior; focus and sibling Stop/Watch targets
remain separate. The existing `ctxColor` meter becomes 3px and straddles
the painted lower edge by 1px to clear the avatar, with rounded ends; percentage/ARIA and
missing-versus-zero semantics are unchanged.
Expanded names retain wrapping, with padding for the paint inset and meter.
This leaves ordinary single-line geometry unchanged while letting long names
grow their row instead of crossing the 3px meter.

All is secondary, after the named and stopped identities, before Add. It is
an icon-only choice in the collapsed strip, labelled in the expanded list
and always named in hover/ARIA and the selected-destination placeholder.
Its resting paint is neutral/unframed, not a permanently accented first
card; selected All retains the accent capsule. This deliberately supersedes
the first-position/broadcast-emphasis treatment from #168/#173, not their
selection or interruption contracts. The same busy-only All Stop remains,
including the double-Ctrl+C equivalence; no new destination sentinel.

The single native horizontal scroller meets the disclosure without an extra
gap. `ui/scroll-edges.ts` measures which physical ends still hide content;
the shared `.edge-fade` alpha mask softens only those ends. It is an overflow
cue, not an overlay covering actions or a substitute for fitting controls.
At an end, that end is fully visible; a fitting or expanded list has no
horizontal mask. Keyboard focus also suspends the mask: native Tab and
menu focus return otherwise leave an edge glyph/focus ring visibly faded.
One action observes scrollport/items and direct-child
changes, cleans up on unmount and adds no timer, custom scrolling or
selection listener. The same keyed list, recency hold, disclosure preference
and parent reading transaction remain authoritative.

### Card refinements (board #173, 2026-09-11)

Owner, 08:39: "卡片显示优化，context 长度比例还是直接帮我可视化出来，还有 everyon 可以换一个样式，不然我以为还是和 agent 会话一样。还有终止按钮，不要圆形的阴影了，还有这个黑方块看着不知道是停止的意思，是不是换个颜色？让我能更容易够理解这个按钮的含义。还有缺少了双击show 单独过滤某个 agent 消息的能力，是不是可以双击后，卡片样式给我一些改变。"

Originally 2px (strengthened by #176 above), `.ac-bar` restores context usage at the card's
bottom edge, in both strip and expanded views. It uses `ctxColor`, not a new
threshold or colour family; zero is a reading, missing data has no bar, and
over-capacity readings saturate the fill while hover/ARIA retain the exact
percentage. The track sits between the rounded corners, costs no layout
height and never intercepts clicks or clips the command focus ring.
Chromium 152 desktop/390px light/dark, collapsed/expanded comparisons keep
card and roster boxes identical across 16 captures; fill/track ratios match
the reported percentage. A zero-fill negative control breaks that ratio.

The everyone choice is a broadcast capsule: no avatar tile, a group glyph
and accent-ink label, with the same accent fill/border for selected state.
It is deliberately distinct from a named partner, not another agent. Its
round-corner exception stays in app.css, the existing corner-policy owner.

**Double-click focuses an agent** (lead clarification, 08:48). There is no
single-click delay or rollback: clicks act immediately, and a fine-pointer
double-click explicitly selects the live agent and toggles the existing feed
filter. The second click in that native sequence does not toggle selection
off. Another double-click clears the filter and leaves the agent selected.
Stopped slots only filter; their click menu opts into the shared
`menuHeightLimit` cap before `menuPlacement` measures/flips it. A viewport-only
clamp could cover the trigger when neither side fits the full menu; limiting
height to the larger side keeps the second click reachable and scrolls the
menu internally. Other menu callers retain their existing placement.
Coarse pointers and keyboard users retain the ContextMenu filter command,
which remains a reading-only choice. A filtered card carries a neutral dashed
inner outline and an accessible label; recipient selection keeps its
separate accent fill/border. The feed's existing filter indicator remains.
Tests execute the complete click/click/dblclick sequence, not a lone
synthetic dblclick. Chromium also verifies repeat-to-clear, stopped-card
menu exclusion and the coarse-pointer long-press filter path.

Owner, 08:41: "还有我觉得交互可以优化，比如应该打断方块可以不一直显示，可以鼠标移到卡片上后，可以显示打断按钮，还有 show terminal 的快捷按钮也要显示"

Fine-pointer cards reveal Stop (busy only) and Watch on hover/focus-within,
without moving their reserved native targets. Pending Stop remains visible
so the existing keyboard interrupt has feedback. On coarse pointers Stop
stays visible while busy and Watch stays in the long-press menu. Stop uses
the shared plain `warn` command: the same glyph, amber mixed with foreground
ink to meet the 3:1 glyph floor, no circular ground/shadow. Watch and the menu
share one parent adapter that selects the Terminal view before using the
existing pane resolver; a compact terminal
request uses the existing full-page Terminal callback. A missing requested
pane clears the stale terminal target rather than watching the previous agent.
Neither action changes the recipient. The Watch slot stays fixed; an idle
selection reclaims the
vacant Stop slot, keeping target regions separate. Chromium 152 verifies
18 desktop/390px, light/dark action states, stable reveal boxes, keyboard
reach/pending feedback and coarse fallback. Mounted tests exercise phone
and compact desktop routing including a vanished pane; the wide-browser
Files-to-Watch check stubs only the two renderer boundaries, proving routing
and identity rather than either renderer's internals.

### One roster above the input, one Stop operation (board #168, 2026-09-11)

Owner, 03:32: "我看到这个设计挺好看的，因为我们有多个 agent，可以在上边去展示哪些agent 在工作，并且我可以对某一个 agent 单独点停止。可以把原来我们的 agent 擦片就放到发送框上边的位置，然后我们消息输入框里，就不用 to 谁了"

The single `Roster` sits between Feed and Composer. Its native selection
button chooses `@name`; selecting it again chooses no implicit recipient.
An explicit everyone card chooses `@all`. The placeholder names that
destination, and the selected card's existing hover card explains delivery.
Body mentions still deliver through the existing parser; `chipExtras` uses
`mentionTokens` to mark reached cards with one `@` glyph, never a second
parser, dashed ring or selection change. The drawer's window selector has
a different purpose and stays. The owner-selected ordering and disclosure
below supersede the prototype's team-group adjacency.

**Final density and ordering, owner 07:09:** "用单行的形式吧，而且不要进行中这种文字描述了，最好有一个展开的小按钮，展开一个多行的 list ，但是限制高度，我可以一下子看到很多，而且 agent 卡片排序，一般最后活跃的放到最前边"

Cards have one line: avatar, name, the existing status dot, optional mention/
unread marks and a separate Stop. State words remain in hover/ARIA, not the
visible row. `sortAgentsForRoster` in `hub.ts` puts the existing busy membership
first, then sorts each busy/nonbusy partition by `since` descending and window
index ascending. For hooked agents this is turn-level recency: `since` is the
prompt start while running, ask time while waiting, and end time while
idle/failed, not the latest tool call. The server's existing no-hook fallback
is unchanged; the client adds no pane reading, clock, telemetry field or
history scan.
Global order replaces `groupRoster`; team paths remain in hover and accessible
names, rather than forcing members to remain adjacent.

One shared icon-only CommandButton expands the same keyed list in normal
flow. The default is one horizontal row; expansion uses 1/2/4 columns and
internal scrolling, capped at `min(240px, 32dvh / --ui-zoom)`. Long names wrap
in expanded cells instead of clipping. Both selection and Stop retain native
28px pointer / 44px coarse targets; the Stop track remains reserved when idle.
The disclosure chevron uses the existing `.flip` atom through controlled
`expanded`, never a second glyph or animation. `hubPrefs.rosterExpanded` stores
the mode per project and follows project rename like the drawer preference.
Hub captures project and next mode before its existing `withReadingAnchor`
transaction; this is not a popup or new Back/history layer.

While a card is pointed at, pressed or focused, hold the display order but
read current agent objects for state and actions. Remove departed names and
append new members; adopt the latest turn order on pointerleave/blur/release,
not a timer. An idle expanded view reorders live: freezing until collapse
would hide new activity (lead clarification, 08:04). Keyed `flip` uses
`moveMs()` for that reorder, not a new timing curve. A new Stop must not move
under an active gesture. Pointer-up/lost capture releases the press lock even
when a disabled target emits no click.
Relevant DOM updates recheck actual focus because removing a Stop or agent
does not guarantee a `focusout` event.

Each busy card exposes a sibling Stop button, not a nested selection button.
`busyTargetsFor` in `hub-composer.ts` is the sole membership decision:
running/working/waiting/blocked managed agents only; everyone includes only
busy members, no selection includes none. Both that button and empty-composer
double Ctrl+C call the same Hub dispatcher. The dispatcher captures room and
names before awaiting, prevents overlapping requests for a member, and cleans
up only its own jobs. A peer Stop never changes recipient, opens/retargets a
drawer or forces the feed to its tail. Existing server push/poll reads the
reset-first state and `[tmm] interrupted <name>` line; no optimistic status
store is added. Process kill/remove remain separate consequential menu verbs.

The keyboard can do no more than the visible Stop: two non-repeated Ctrl+C
presses within three seconds, only with an empty composer and no text
selection. Text, room or recipient changes and Escape reset that sequence;
native copy and IME composition remain untouched. It has no send-button
arming state, timer or caption. Send only sends and is disabled while empty,
uploading or blocked by a failed attachment.

The delayed 260ms card menu, double-click filter shortcut, recipient popup
and two-beat send-button mechanism are removed whole. Secondary agent verbs
use `ContextMenu` with `longpress`; stopped slots open that menu, never select
an undeliverable destination. Only the command palette remains a Composer
Back layer. The textarea and attach/send row are normal flow, so the old
recipient indent, last-line mirror and corner-collision padding are gone.
Actual shell-height changes still notify Hub through the existing reading
boundary; no animated height, new scroll mechanism or lifecycle remount.

The pre-#168 descriptions below of the chip, mirror, recipient menu and
two-beat button are historical evidence, not alternate supported controls.
Delivery parsing, attachment gates and readline rules are unchanged.

Verification on Node 22.23.2 / Svelte 5.55.5 / Vite 6.4.2: pure target
vectors plus the mounted real Hub pin selection, body mentions, busy-only
Stop, keyboard parity, duplicate requests and room identity across awaits.
Negative controls remove the managed gate (reaches a shell) and make Stop
select its target (changes Alice to Bob); both fail their tests. Seven retained
Hub transport/staging/reading functions are AST-identical to the #171 base.
Chromium 152 checks desktop and 390px light/dark, explicit coarse media,
long names, nested teams, menu/Stop reach, reload, reduced motion and a
reduced-height multiline input. Settled history typing and peer Stop preserve
the reference row within 1px. Tests wait for the existing 300ms focus-tail
callback before beginning a history transaction; racing that separate callback
is not evidence of a composer-growth defect. Synthetic touch and key checks
do not replace native Android keyboard/IME acceptance.

### The room has a default recipient

**Explicit broadcast restoration** (board #171, 2026-09-11): a stored
`ALL_TARGET` is a destination mode, not an agent name to look up. `pickLead`
preserves it before inspecting the roster, exactly like the stored empty
string. This deliberately keeps broadcast selected through an empty/direct-only
roster; it neither delivers nor interrupts anything. With null/absent preference,
the existing managed-lead rule still applies, and an empty roster yields `''`.
The bug was in this classification, not localStorage: a stored `"all"` failed
the agent-name lookup and silently became the first lead on reload or revisit.
The single `ALL_TARGET` constant comes from `hub-composer.ts`; no new sentinel
or preference migration. Unit vectors cover zero/one/many/direct-only rosters;
a mounted real Hub covers fresh-client restore, cached-room return and polling
with post/command/interrupt RPCs forbidden. The chip's former "three states"
description below omits broadcast; this dated rule corrects that omission.

`pickLead()` (pure, client) = remembered choice while present → the only managed agent → one whose registry def `can_hire` → lowest window; choosing a recipient IS choosing the project lead, so it persists per session — and the ROOM is a choice too: `hubPrefs.lead` has three states (a name, `""` = the user chose "no recipient, record only", `null` = nobody chose), `pickLead` keeps an explicit `""`, and only `null` seats a lead; before, `""` was stored as an absent key and the next 5 s roster poll re-seated a lead the user had just dismissed (review C, 2026-09-03; `hub-prefs.test.ts` + `pickLead` tests pin it) — and `addressed()` ALWAYS prefixes the chip's `@name` (owner, 2026-08-26: "前边的永远保留，后边的at如果存在某人的话也会发送过去" — an `@` in the body used to stand the chip down, so an email address or quoted @all silently defeated the chosen recipient); body @mentions deliver TOO because `deliver_mentions` scans the whole body, and only a body already leading with the same recipient skips the prefix (empty recipient = the whole room). **The chip SAYS so, live** (review, 2026-09-03: it read `to: alice` while `@bob` in the body was delivered as well, with no hint): `chipExtras(text, recipient, managedNames)` (pure + tested, on the same `mentionTokens` tokenizer as `mentionsAgent` and the feed's room-note verdict — never a second parser) appends `+@bob` for every managed agent the body addresses beyond the recipient, in roster order; an `@all` in the body collapses to `+@all` rather than listing the roster, `@all` as the recipient appends nothing (already everyone), and a name that resolves to nobody on the roster — an email address, a removed agent, a direct window — is not shown because it is not delivered. The extras wear the accent even on the grey room-note chip, because that part IS a delivery, and the chip's tooltip switches to name them. An empty room shows a preset start (one agent, or several as a team) rather than a composer with nobody to talk to — but "empty" is a VERDICT, not a default: switching projects parks the leaving room in an in-memory per-session cache (`roomCache`: feed, activity, roster, cursors) and restores it instantly on return, and a room without a cache entry renders NOTHING until its first `hub_log` answers (`roomReady`); judging emptiness from the cleared arrays made every switch flash the "add an agent" panel in front of rooms full of history (owner, 2026-08-25: "先看到添加 agent 一个 agent list那个页面闪了一下，然后再出来消息"). The pollers merge on top of the restored state (cached `lastTs` makes the refresh incremental), a cached roster seats the recipient immediately via the same `pickLead`, and entering a room always lands at its tail. The verdict rule swept the other pages too (owner, 2026-08-25: "不同页面切换会不会有类似的这种问题"): the Terminal sidebar says "No sessions" only after its first refresh answered (`listReady`) and renders the untracked group only after Projects reported `onTracked` once (`trackedReady` — before that every tracked session flashed as untracked and migrated; `Projects.load` now reports on its failure path too, so the gate cannot stay shut), and the Agents page keeps last-known defs/skills/MCP on a failed reload instead of wiping them. Files (`!entries.length && !loading`) and Team (`loading` gate) were already correct. DirPicker navigates the same way (owner, 2026-08-28: "每次点击一个路径…先清空再重新刷新…要交互更流畅"): a tap KEEPS the current list and swaps it atomically when `fs_list` answers (`seq` makes the newest tap win; scrollTop resets on the swap because a new directory starts at its top); the `…` placeholder shows only before the FIRST answer, a failed listing keeps the old rows with the error line above, and the busy cue is a 150ms-delayed opacity dim so a fast local listing never blinks.

### Three ways a message lands, and they are not shades of one thing

a name (the default lead) types into ONE agent's input; `@all` types into EVERY managed agent's input, so every agent starts a turn at once; no recipient records it in the room and interrupts NOBODY (agents see it at their next `tmm log`). The third was once labelled "everyone", which was exactly backwards — it is the one that reaches nobody live.

### A `/command` goes to the CLI, not to the model

`/model`, `/clear`, `/compact` are interpreted by the agent's TUI and only as a whole line, so `hub_command` types them VERBATIM into the pane — no `[tmm chat …] human:` stamp, no @address (owner, 2026-08-19). `slashCommand()` (pure + tested) requires the first token to be `/word` with NO second slash, so `/tmp/foo` and `/usr/bin/env node` stay messages; it needs a target (explicit `@name`, else the composer's recipient, `@all` = every managed agent) and falls back to an ordinary message when there is none; managed windows only (a `/clear` typed into a SHELL would run as a path); and the room records it as a `[tmm] ` lifecycle line, never a message, so the mention scanner cannot feed it back.

### The `/` palette is transcribed, not invented

`KIRO_COMMANDS` (`hub.ts`) carries kiro-cli's own names/descriptions/sub-commands copied from its TUI table — a made-up command looks authoritative in the list and then does nothing in the pane — minus cloud-only/hidden entries, with `/quit` last.

**Only commands that DO something are offered**: kiro's `inputType: "panel"` ones (plus those that open $EDITOR or a recorder) would park the agent inside a view nobody here can see or dismiss, so `/tools`, `/help`, `/mcp`, `/context`, `/code`… carry `view: true` — they stay in the table WITH the reason and `OFFERED_COMMANDS` filters them out (owner, 2026-08-19); re-enabling one is deleting a flag. Ten remain, and `/agent` offers only `swap` because create/edit open an editor.

**The palette speaks the ADDRESSEE's dialect** (2026-08-22): `offeredCommands(backend)` picks the table by the explicit `@name`'s backend, else the recipient — `GROK_COMMANDS` (transcribed from grok 1.0.5's own docs; its `/model` completes inline since grok models enumerate) and `CODEX_COMMANDS` (transcribed live from codex 0.148.0's `/` popup; there `/model`/`/permissions`/`/review` are PICKERS = views, and `/delete` is flagged destructive, never offered); claude gets NO palette yet (its `/` popup is still untranscribed) and `@all` over a mixed roster none either. `commandPalette()` (pure + tested) decides stage/items/replace-slice: `/` → commands, `/model ` → its values (fetched via `models_list` PER BACKEND, the same call the agent editor uses), and only the LAST token + FIRST argument are completed because what follows one is a path or free text. Matching is FUZZY in three strict tiers — prefix, substring, subsequence (`fuzzyRank`, pure + tested; stable table order within a tier) — so `/mdl` finds `/model` and `son` finds `claude-sonnet-4.5` (owner, 2026-08-24: "不一定从第一个字符开始匹配"), while Enter stays safe because a looser match never outranks a tighter one (`/co` still means `/compact`). The palette owns ↑↓/Tab/Enter while open, Escape dismisses until the text changes, hover and the keyboard cursor share ONE highlight. The composer SPEAKS READLINE: Ctrl-A/E/U/K/W/Y/D/H/T/F/B via `readlineEdit()` (pure + tested) — one kill buffer, consecutive kills accumulate (backward prepends), any other key breaks the chain, A/E are line-scoped, Ctrl-K at line end joins, empty-buffer Ctrl-Y is a handled no-op (Chromium's default is REDO), Ctrl-C/V/X/Z fall through to the browser; the caret is set after Svelte writes the value back. A command-shaped draft STYLES the composer (accent-tinted capsule, tool-lane monospace) via `composerIsCmd`, which mirrors send()'s branch exactly; the measuring mirror flips font with the input in ONE rule, and `growComposer` re-measures on the flip (a font change rewraps).

### Pure composer decisions (board #117, 2026-09-09)

`hub-composer.ts` owns palette-backend selection, `ALL_TARGET`, attachment
token spelling and body substitution; #168 adds the pure busy-target
selector described above. The original helpers were closure-local calculations;
parameterizing them adds executing boundary tests without changing behavior.
The selector still requires a managed target, keeps unknown targets empty,
and offers a shared dialect only when the managed roster has exactly one.
Substitution replaces the first matching token, appends missing-token
references in attachment order, and retains `String.replace` semantics.
The existing command/mention/readline parsers and command tables stay in
`hub.ts`; Hub keeps `send()`, staging and both send gates. Board #133 moves
the view-owned state and DOM as described below.

The mounted Hub test holds a file upload unresolved, presses Enter and
observes no post; after upload it observes the complete addressed body.
Removing the actual `attaching` guard is the negative control. This proves
the event-to-send path, not native clipboard behavior or layout.

### Historical extraction characterization (board #133, 2026-09-09)

Before moving the view, the mounted Hub characterizes readline caret after
settling, kill-buffer and draft lifetime across rooms, palette focus, all
three delivery destinations, compact/Shift/IME Enter, the mixed button/key
interrupt pair and its 3-second expiry/disarm rules. Deferred RPC replies
cover overlapping upload generations, failed chips and post/command rollback
across a room switch. The existing upload/Enter and Back tests stay unchanged.
These execute the real handlers but do not simulate clipboard insertion or
layout. Chromium 152.0.7977.64 separately captures the real view at 1440x900
and 390x844, light/dark and reduced-motion spots: short and overflowing text,
command mirror font, measured recipient indent, both menus and file chips.

The receipt and recipient menu share the dashed `.note-dot` in
`hub-atoms.css`, beside their shared `.st` box. Its former scoped declaration
is relocated once, with the same two-class specificity and restricted parent
selectors, so extracting the Composer does not duplicate it or restyle
embedded Files/Terminal/Board content.

### Historical view ownership (board #133, 2026-09-09; superseded by #168)

`Composer.svelte` owns the capsule, textarea/mirror sizing, paste/key adapters,
palette/model cache, readline kill buffer, recipient popover and interrupt
arm/timer. It is unkeyed and lives as long as Hub: changing rooms does not
reset the kill buffer or model cache. Private CSS moves with its markup;
compact ancestor selectors cross the Svelte boundary at the same specificity.

Hub still owns the draft binding, recipient/preference writes, attachment
generation/jobs/pending set, staging, `send()` and every RPC. Composer receives
explicit state and commands, not a Hub store or feed/roster DOM references.
The named `caret`/`focus` methods replace staging's two textarea accesses;
height/focus intents retain the original parent tail behavior. Thumbnail
preview passes the original local object URL to Hub's existing Lightbox.

Composer registers only recipient/palette/interrupt with the existing fixed
Back registry and disposes those registrations on unmount. The capture-phase
listeners remain in Hub in their original order; their Composer portions
delegate to live child methods, not copied open flags. Parent recipient
selection still closes the picker and disarms synchronously. No browser
history listener, new dismissal mechanism or RPC race fix is part of this move.

Verification on Svelte 5.53.5 / Vite 6.4.1 / Node 22.23.2: seven moved
handlers and `send()` are AST-identical; staging/growth differ only by their
named boundary calls; all 73 moved CSS rule bodies match. Chromium
152.0.7977.64 produced 54 identical text/rect/computed-style signatures
across the six variants. The font-mirror negative control removed only the
mirror's command-font selector: its tail marker moved from (552.58, 849.5)
to (983.13, 809), failing the geometry assertion. The selector was restored.
The restored build passes that assertion again. Two additional desktop/compact
long-recipient signatures also match; real PNG paste stages a thumbnail whose
original local URL opens in the existing Lightbox in both layouts.

### Words beside a picture of the words: the text is the paste (2026-09-08)

The paste door's rule was "files win over co-riding text" (board #25) — right for a Finder/Explorer file copy, whose text is the file's own path, and for a screenshot, which has no text. It was wrong for the paste the owner met daily: PowerPoint (and Word, Excel, Keynote, Numbers, every browser) puts a **PNG rendering of the selection** on the clipboard beside `text/plain`/`text/html`, and the composer staged the picture and threw the words away ("从 ppt 上粘贴过来的文字，总是被粘贴为了一个图片"). `textIsThePaste(text, files)` (hub.ts, pure, tested) now decides before `preventDefault`: the text wins when it is non-empty AND every file is an image AND the text is not one token that is a URL, a path, or the name of one of the files. Every legitimate file paste fails that test — a screenshot carries no text, a file copy carries its own name/path, a web "Copy image" carries at most the image URL, and any non-image file (a pdf beside a caption) is a file paste whatever the text says. When the text wins the handler simply returns and the textarea's default insertion runs; the rendering is dropped, not staged — nobody wants a picture of a bullet list next to the bullet list. The HTML flavour is ignored on purpose: the composer is plain text and markdown, and Office HTML is a wall of styling.

### An attachment failure is a chip, never a console line

The staging pipeline (`stageFiles` — the `+` button and paste share it; the reference rule and the upload layout are in `hub-feed.md` §An image is a reference) used to `console.warn` an oversized file (`FILE_CAP`, 32 MB per RPC) or a failed upload and carry on, so the user could not tell what the message was about to carry (review, 2026-09-03). Now every failure is a `pending` entry like any other attachment — `failedAttachment(file, reason)`: no path, no token, no number, an `error` — rendered in the file chip's own clothes turned to the danger tone (`.pend-chip.err`: `--status-danger` ink + border + 8% wash, name + reason, tooltip with the whole reason, ✕ removes it). Three failures land there: too large (before any upload — with the hint to point the agent at the path instead), a per-file throw (the loop is per FILE now, so one bad file does not take the others down), and the uploads dir itself failing (every file of that job). A failed chip is not content (`sendable` counts only staged ones) and it BLOCKS send while it stands — `send()` returns and the button is disabled with `Remove the failed attachment first` — the same two-gate rule `attaching` follows, because a message that quietly left without the file it showed is the failure mode this exists to prevent. The staleness discipline holds: a throw comes out of an await, so the catch checks `stale()` before touching the room (as a guard, not a return, so the loop goes on); the source test pins one check per await unchanged, plus this test's own three chips and the two gates.

### Interrupt is a third verb, and only the pane can carry it

typing `Escape` into the agent's own pane (`hub_agent_interrupt` server-side so the UI and CLI share ONE implementation; `send_keys`, NAMED key — with `extended-keys on` tmux drops raw C0 bytes) is the only way to cancel a turn that is ALREADY running; a `tmm`/chat message is read between turns by definition, and stop/restart kill or bounce the process.

**The derived state is RESET FIRST, then the key is typed** (`telemetry::record_interrupt`: `end = ("completed", now)`, `ask` and the explicit claim cleared → `derive` says `idle` immediately), and the order is the whole point (owner, 2026-08-29): a turn cancelled from OUTSIDE has no edge of its own — no stop hook fires for it — so the newest fact would stay the `userPromptSubmit` that opened it and the card would read `running` for as long as the agent lived; and an interrupted agent usually gets something else to do within seconds, whose own turn re-derives `running`, so a reset racing that new turn is indistinguishable from no reset at all — an interrupt that looks like it never landed. Clearing `explicit` is part of it: a `blocked` claim from the dead turn outranks the end in `derive_from` when they share a second. So the Hub's agent action bar carries Watch / Interrupt / Stop, in rising order of consequence.

**The composer carries it too, behind two beats** (owner, 2026-08-24): with the box EMPTY the grey send button is still clickable — the first tap ARMS it ("send interrupt": amber, a caption pill naming the target since a phone has no hover) and the second fires; double Ctrl+C on the empty composer is the same arm/fire pair (with text present Ctrl+C stays the browser's copy, which `readlineEdit` deliberately falls through to).

**While the recipient is mid-turn the resting button says so**: a stop square inside a slowly circling arc (2.2s — a fast spin says "loading", this says "a turn is open"; accent on the resting grey, `prefers-reduced-motion` stills it), the glyph every chat product speaks — the armed state keeps the same glyph on the amber ground, because the earlier `zap` bolt read as nothing ("我看打断是闪电，看着好像不是那么容易理解", owner 2026-08-25). Busy means the hook-derived running/working/waiting/blocked of the recipient (any managed agent for `@all`); idle and failed show the plain grey arrow — an ended turn has nothing to interrupt. The interrupt reaches whoever the composer reaches — the recipient, or every managed agent for `@all`; an unaddressed room note arms nothing. Armed state stands down by itself: 3 s, typing, Escape, or switching projects. And the room now RECORDS the act — `hub_agent_interrupt` posts `[tmm] interrupted <name>` (the sys grammar already spoke it: amber, a turn cut short), so the feed shows what the app did ("发送 interrupt 的状态在消息列表里也要展示出来"); a real-tmux test pins window-survives + room-line.

### Historical composer motion (before #168)

Per [motion.md](motion.md). The recipient chip's arrow is ONE chevron in `.flip` — up while closed (the menu opens upward), turned down while open — never two glyphs swapped (principle 4, the owner's own example). `+@bob` fades in (`.appear`); attachment chips and thumbs pop in (`.appear-pop`; the list is keyed by `a.key`, but its three chip species sit under an `{#if}`, so there is no `animate:flip`); the interrupt caption rises (`.appear-rise`); the send glyph pops when the `{#if}` swaps arrow ↔ stop-square — unrelated glyphs may swap. The recipient menu and the `/` palette are placed popovers like every other (principle 8, wave 6): each wears `.pop-layer` with `bind:clientHeight` as its `.ready` gate and `--pop-origin: bottom left` — the corner touching the chip, since both open upward — so it is measured invisible, then grows from the chip on `--t-fast`; the height resets when the menu closes so every opening is measured (and animated) again, and the atom touches only opacity/transform/pointer-events, so their own `position: absolute; bottom: calc(100% + 6px)` placement is untouched and the rest state is `transform: none`. Exits are cuts. **Hover** (principle 16): resting on the recipient chip opens the one hover card with a sentence for the CURRENT destination in the three-destination model — `@name` types into one pane, `@all` into every managed pane, no recipient records only — with the body's `+@` extras as its note; the chip carries no native `title` any more. The composer's height is JS-measured per keystroke and never animated.
