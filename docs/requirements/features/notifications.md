# Message Notifications — requirements

Board #57. New project-chat messages can alert a reader who is not currently looking at that conversation.

## Channels

- Play the bundled placeholder cue (`/assets/notify.wav`). The owner will choose the final sound later; replacing the asset must not change notification logic.
- When the Web Notification API exists and permission is granted, also post a system notification — through the service worker where one is registered (Android Chrome refuses the page constructor), else the page constructor. One tray card per project (`tag`), replaced on each burst.
- Inside the Tauri app (Android, macOS) the same path reaches the OS notification tray through `tauri-plugin-notification`'s `window.Notification` shim; Android 13+ asks its runtime permission when the setting is turned on.
- Permission is requested only by the Settings toggle's own click (Settings → Notifications, its own category), never as a side effect of sending a message. The category also offers a Test row that plays the cue and posts one notification on demand. The toggle persists enabled/muted state and its caption says when only the sound can play (site blocked, or no Notification API as in the Android webview). The Hub header carries no notification switch (board #72).
- Unsupported APIs, denied permission, and blocked audio all fail soft.

This is a **running-client** notification path for browser/PWA/webview. It does not claim remote push after the page or app has been closed.

## Level

Settings → Notifications carries a level, persisted to `tmux_notify_level`, three nested rungs:

- **Finished** (`done`): `[tmm done]` summaries and an agent's board move to review/done only.
- **Replies** (`replies`, default): Finished plus every agent reply.
- **Everything** (`all`): Replies plus ambient `[tmm status working|waiting|blocked]` progress notes.

App narration and the human's own messages are never news at any level. An unknown stored value reads as the default.

## What counts as news

A message may alert only when the reader is away from that conversation: the document is hidden, the window is unfocused, another app page is visible, or the message belongs to a project other than the selected one (its notification names that project).

- The check runs on the live `team_message` push, which arrives for every room on every page; the visible-only `hub_log` poll is the fallback. A message seen by both alerts once.

- Initial room loads, history pages, cache restores, and inclusive-poll replays never alert.
- Every observed message is remembered by server id, falling back to its `(from, ts, body)` identity. The same message alerts at most once.
- Human-authored messages, nameless rows, `[tmm]` lifecycle narration, and ambient `[tmm status working|blocked]` updates do not alert.
- Agent replies, `[tmm done]` summaries, and an agent's board move to review or done ("who finished what": `#N → review · title`) are news. Moves to doing, spawns, and the human's own moves are not.
- Batches produce at most one cue and one system notification; the cue has a short cooldown.
- Messages observed while looking or muted are still remembered, so leaving the page or unmuting cannot backfill old alerts.

## Safety and races

A failed audio play rolls back only its own cooldown claim. A delayed rejection from an older play must never reopen the cooldown claimed by a newer successful cue. The seen-key set is bounded so a long-running client cannot grow notification memory without limit.
