# Message Notifications — design

The implementation lives in `src/lib/hub/notifications.ts`; `Hub.svelte` has two call sites — the `team_message` push handler (primary) and the `hub_log` poll (fallback).

## Where the check runs (board #72)

The first cut checked only after a `hub_log` poll batch. That poll runs inside an effect gated on the Hub page being visible, so the "another page is on screen" half of the away verdict could never fire — a phone user who switched to Terminal got nothing, which is the case the feature exists for. The `team_message` push arrives for every project room whatever page is showing (the listener effect is not gated), so `onPush` is now the primary site:

- a message in the SELECTED room asks the live document (`isAway(visible)`: hidden, unfocused, or the Hub not on screen);
- a message in any OTHER room is away by definition — the reader is not looking at that conversation — and the title names that project (`roomProjectName` over the sidebar rows, recorded `room` first, then `proj:<session>`);
- a push is never "first": it is by construction newer than any loaded page.

The poll keeps its call as the fallback. Both carry the server message id, so `sift` makes push-then-poll alert exactly once. What is still not covered: a suspended mobile webview (app backgrounded) receives no pushes; the missed messages arrive on the next poll, which runs only when the Hub is visible again, i.e. while looking — no alert. Remote push for a closed or backgrounded app is a separate task (native `tauri-plugin-notification` on Android).

## Remember first, then decide

`hub_log(since_ts)` is inclusive, cache-restored rooms re-pull around their cursor, and equal-timestamp messages can return together. Therefore a raw poll batch is not proof of novelty. `sift()` records every message key before applying any alert policy:

1. use the server message id when present;
2. otherwise use the same `(from, ts, body)` identity as feed merging;
3. bound insertion-ordered memory to `SEEN_CAP`;
4. pass only never-seen rows to the news gate.

Recording happens even for initial, watched, or muted batches. This is what prevents later backfill.

## Effects are downstream of pure gates

`notifiable()` filters first-load/watched batches, own messages, lifecycle narration, and — by LEVEL — replies and ambient status updates. The level (owner, 2026-09-02: "只有完成才通知，还是中间状态都通知") is three nested rungs, `done ⊂ replies ⊂ all`: a finished task (`[tmm done]`, or a board move to review/done) is news at every rung; a plain reply from `replies` up; a `[tmm status]` progress note only at `all`. `notifyNews` reads it through an injectable `effects.level`, so the gate stays pure and the default (`replies`) is what a chat app does. `notifyNews()` then emits at most one audio cue and one Web Notification for the remaining batch. Effects are injectable in tests, so deduplication, permission-independent behavior, and race handling do not require a real OS notification daemon.

The cue claims its cooldown before calling `Audio.play()` to prevent same-tick double playback. Rejection rolls the claim back only while that attempt still owns it (`lastCueAt === attemptTime`); an old deferred rejection cannot overwrite a newer successful claim.

## User gesture and platform boundary

The Settings toggle (its own **Notifications** category right after Appearance — owner, 2026-09-02: "应该在一个单独的 notification 二级页面"; On/Off in the segmented dialect) is the only permission and audio-unlock gesture; it persists `tmux_notify`. The category also carries a **Test** row: cue plus one notification on demand, because the real alert fires only while the reader is not looking, so a phone has no other way to confirm the tray shows one. It stood in the Hub header as a bell until board #72: a header keeps no spare switches, and on a phone the extra button was part of what let a long project name push the toggles off-screen (owner, 2026-09-02). Its caption reads the platform back through `notifyPermission()`: `denied` says the site is blocked and only the sound plays; `unsupported` (no `Notification` API — the Tauri Android webview) says the sound is the whole channel. Sending, interrupting, polling, and initial loading never prompt.

### Native tray on Android and the desktop app

The Android and macOS webviews have no Web Notification API of their own. `tauri-plugin-notification` (Cargo `gui` feature, `notification:default` capability, registered in `lib.rs`) injects a `window.Notification` shim into the webview: the constructor becomes `plugin:notification|notify` (the OS tray — the plugin creates Android's default channel itself and its manifest declares `POST_NOTIFICATIONS`), and `requestPermission` becomes the runtime prompt on Android 13+. So the one web path below is the native path inside Tauri; the JS side imports nothing from the plugin (its npm `sendNotification` is literally `new window.Notification`). Two shim facts the code respects: `Notification.permission` reads `denied`, not `default`, before the first ask — so `ensurePermission` asks whenever it is not `granted`, which a browser also tolerates (a re-ask on a denied site returns without prompting) — and the shim settles asynchronously at startup, so a caption read at mount may lag a tick.

What the phone says is what the owner asked for ("什么 project 谁完成了什么任务"): title `<agent> · <project>`, body the excerpt — an agent's reply, its `[tmm done]` summary, or a finished task. `taskFinished()` admits an agent's `[tmm] board #N … → review|done` line through the news gate (a move to doing, a spawn, or the human's own move stay narration) and `excerpt` renders it `#N → review · title`.

`systemNotify` prefers `ServiceWorkerRegistration.showNotification` and falls back to `new Notification`. The order matters on a phone: Android Chrome refuses the page-level constructor (`TypeError: Illegal constructor`), so the PWA showed nothing and the fail-soft catch hid it; `main.ts` already registers `/sw.js` on every non-Tauri secure origin, and the worker path works there. The tag is `tmm:<project>`, so a burst from one room replaces its tray card instead of stacking. The environment (`permission`, `construct`, `registration`) is injectable, and the tests pin all three branches.

Web Notification support varies by browser/webview and normally requires a running page. The design deliberately falls back to the cue and does not present this as service-worker remote push. The placeholder WAV is isolated from policy: the owner can replace the file without touching code.

## Rules and their reasons

Each entry is a decision with the reason it was made; treat them as normative. They lived in the root `CLAUDE.md` until 2026-09-02 (board #73), when that file became an index and the rules moved next to the design they belong to.

### New-message alerts remember BEFORE they notify

(board #57): `hub/notifications.ts` keys every observed message row by id (fallback `from/ts/body`) in a bounded seen set before the first/history/watched/muted/news gates, so inclusive polls and cache restores never replay or backfill alerts. Only an away running client alerts; own/sys/ambient-status rows drop, `[tmm done]` remains news.

**The check runs on the PUSH, not only the poll** (board #72): the `hub_log` poll effect stops with the page (`if (!visible) return`), so a poll-only notifier could never fire for "another page is on screen" — the phone user's whole case. `Hub.onPush` is the primary site: the `team_message` push arrives for EVERY room on every page; the selected room asks `isAway(visible)`, any other room is away by definition and the title names that project (`roomProjectName`); the poll stays as fallback and `sift` makes the pair alert once.

**The switch lives in Settings, in its OWN category, not the Hub header** (owner, 2026-09-02, twice: "放到设置里" then "应该在一个单独的 notification 二级页面"): Settings → Notifications (right after Appearance) holds the On/Off segmented switch — the only permission/audio-unlock gesture — plus a TEST row (cue + one notification on demand, because the real alert only fires while you are NOT looking, so a phone has no other way to check the tray); its caption reads `notifyPermission()` back (not permitted → sound only; no Notification API → sound only).

**Native on the phone and the desktop app**: `tauri-plugin-notification` (in the `gui` feature, `notification:default` capability, `.plugin(...)` in lib.rs) INJECTS a `window.Notification` shim into the webview that routes the constructor to the OS tray and `requestPermission` to Android's runtime prompt — so the one web path IS the native path inside Tauri, with no plugin import on the JS side (the npm package was added and removed the same hour: its `sendNotification` is literally `new window.Notification`). Two shim facts the code respects: its `permission` reads `denied` (not `default`) before the first ask, so `ensurePermission` asks whenever it is not `granted` (a browser answers a re-ask on a denied site without prompting, so this is safe everywhere), and it settles asynchronously at startup.

**The LEVEL is three nested rungs** (owner, same day: "能设置通知的级别，比如只有完成才通知，还是中间状态都通知"): Settings → Notifications → `done` (a `[tmm done]` summary or an agent's board move to review/done) ⊂ `replies` (default; plus every agent reply) ⊂ `all` (plus `[tmm status]` progress notes); `tmux_notify_level`, unknown values read as the default, `notifyNews` takes it through injectable `effects.level`.

**"谁完成了什么任务" is news**: `taskFinished()` — an agent's `[tmm] board #N … → review|done` line — passes the news gate alongside replies and `[tmm done]`, and `excerpt` renders it `#N → review · title`; a move to doing, a spawn, or the human's own move stay narration. The header lost the bell, and `.title-group` went from `flex: none` to `flex: 0 1 auto; min-width: 0` with `.path` at `flex-shrink: 1000` — the path yields first, the NAME second, the buttons never (a long name on a phone had pushed them off-screen). `systemNotify` prefers `ServiceWorkerRegistration.showNotification` (Android Chrome throws `Illegal constructor` on the page-level `new Notification`, so the PWA showed nothing), falls back to the constructor, tags per project so one room's burst replaces its tray card. Mute persists, one batch makes one cue + one Web Notification, and a rejected cue rolls back only its own cooldown claim (`lastCueAt === attemptTime`). This is running-page browser/PWA notification, not closed-app remote push; a backgrounded mobile webview receives no pushes; missing APIs fail soft.
