# Message Notifications — design

The implementation lives in `src/lib/hub/notifications.ts`; `Hub.svelte` has one call site immediately after a successful `hub_log` batch.

## Remember first, then decide

`hub_log(since_ts)` is inclusive, cache-restored rooms re-pull around their cursor, and equal-timestamp messages can return together. Therefore a raw poll batch is not proof of novelty. `sift()` records every message key before applying any alert policy:

1. use the server message id when present;
2. otherwise use the same `(from, ts, body)` identity as feed merging;
3. bound insertion-ordered memory to `SEEN_CAP`;
4. pass only never-seen rows to the news gate.

Recording happens even for initial, watched, or muted batches. This is what prevents later backfill.

## Effects are downstream of pure gates

`notifiable()` filters first-load/watched batches, own messages, lifecycle narration, and ambient status updates. `notifyNews()` then emits at most one audio cue and one Web Notification for the remaining batch. Effects are injectable in tests, so deduplication, permission-independent behavior, and race handling do not require a real OS notification daemon.

The cue claims its cooldown before calling `Audio.play()` to prevent same-tick double playback. Rejection rolls the claim back only while that attempt still owns it (`lastCueAt === attemptTime`); an old deferred rejection cannot overwrite a newer successful claim.

## User gesture and platform boundary

The Hub bell is the only permission and audio-unlock gesture. It uses shared `Icon` glyphs (`bell` / `bell-off`) and persists `tmux_notify`. Sending, interrupting, polling, and initial loading never prompt.

Web Notification support varies by browser/webview and normally requires a running page. The design deliberately falls back to the cue and does not present this as service-worker remote push. The placeholder WAV is isolated from policy: the owner can replace the file without touching code.
