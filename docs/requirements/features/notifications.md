# Message Notifications — requirements

Board #57. New project-chat messages can alert a reader who is not currently looking at that conversation.

## Channels

- Play the bundled placeholder cue (`/assets/notify.wav`). The owner will choose the final sound later; replacing the asset must not change notification logic.
- When the Web Notification API exists and permission is granted, also post a system notification. Permission is requested only by the notification bell's own click, never as a side effect of sending a message.
- The bell persists enabled/muted state. Unsupported APIs, denied permission, and blocked audio all fail soft.

This is a **running-client** notification path for browser/PWA/webview. It does not claim remote push after the page or app has been closed.

## What counts as news

A message may alert only when the Hub is away: the document is hidden, the window is unfocused, or another app page is visible.

- Initial room loads, history pages, cache restores, and inclusive-poll replays never alert.
- Every observed message is remembered by server id, falling back to its `(from, ts, body)` identity. The same message alerts at most once.
- Human-authored messages, nameless rows, `[tmm]` lifecycle narration, and ambient `[tmm status working|blocked]` updates do not alert.
- Agent replies and `[tmm done]` summaries are news.
- Batches produce at most one cue and one system notification; the cue has a short cooldown.
- Messages observed while looking or muted are still remembered, so leaving the page or unmuting cannot backfill old alerts.

## Safety and races

A failed audio play rolls back only its own cooldown claim. A delayed rejection from an older play must never reopen the cooldown claimed by a newer successful cue. The seen-key set is bounded so a long-running client cannot grow notification memory without limit.
