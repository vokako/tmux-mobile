// New-message notifications (board #57, owner: "当收到新的消息后 手机推送通知
// 或者播放提示音提示，尤其桌面端播放提示音，看能不能和系统的消息通知结合上").
//
// Two channels, both fail-soft: a short audio cue (the PLACEHOLDER chime at
// `/assets/notify.wav` — the owner will pick the real sound later, so swapping
// the file is the whole change) and a system notification through the Web
// Notification API where it exists and is permitted (browser/PWA; a webview
// without the API silently keeps the sound, which is the half the owner called
// out for desktop).
//
// The GATE is the design, in two layers (lead review, board #57):
//
// 1. SEEN KEYS, not batch trust: `hub_log`'s since_ts poll is inclusive, a
//    cache-restored room re-pulls around its cursor, and same-ts messages
//    come back again — mergeMessages dedups the FEED, but a notifier that
//    eats raw batches would re-alert on every replay. So EVERY batch (first
//    page, history, looking or away, muted or not) marks its keys seen, and
//    only never-seen messages can alert. That is also why the first page and
//    a mute period never BACKFILL: their keys are recorded, silently.
// 2. The news gate: your own words are not news, app narration (`[tmm] ` sys
//    lines) is not news, ambient `[tmm status]` progress notes are not news —
//    they exist precisely so nobody is interrupted (a `[tmm done]` note IS
//    news: something finished). And nothing fires while the reader is looking
//    at the room — "away" means the tab is hidden, the window is unfocused,
//    or the Hub page is not the one on screen.
//
// Enabling is EXPLICIT: the permission prompt and the autoplay unlock ride the
// bell control's own click (a real user gesture that also previews the cue) —
// never an unrelated send. The choice persists.
import { systemLine, statusNote } from './hub.ts';

export type FeedMsg = { id?: number | string; ts?: number; from?: string; body?: string };
export type NotifyState = { seen: Set<string>; lastCueAt: number };

/** The placeholder cue (owner will supply options later — see board #57). */
export const CUE_SRC = '/assets/notify.wav';

/** One cue per burst: a poll batch plays ONE sound, and bursts closer than
 * this stay silent — a busy agent must not machine-gun the speaker. */
export const CUE_COOLDOWN_MS = 3000;

/** Seen-key memory bound — enough for hours of chat, small enough to forget. */
export const SEEN_CAP = 800;

const ENABLED_KEY = 'tmux_notify';

/** A message's replay-stable identity: the id when the server gave one, else
 * the from/ts/body triple (what mergeMessages itself dedups by). */
export function msgKey(m: FeedMsg): string {
  return m.id != null ? `#${m.id}` : `${m.from ?? ''}\u0000${m.ts ?? 0}\u0000${m.body ?? ''}`;
}

/** Split a batch into the never-seen part, and REMEMBER the whole batch.
 * Insertion order is the prune order, so the memory stays bounded. */
export function sift(msgs: readonly FeedMsg[], seen: Set<string>, cap = SEEN_CAP): FeedMsg[] {
  const fresh: FeedMsg[] = [];
  for (const m of msgs) {
    const k = msgKey(m);
    if (!seen.has(k)) { fresh.push(m); seen.add(k); }
  }
  if (seen.size > cap) {
    for (const k of seen) { if (seen.size <= cap) break; seen.delete(k); }
  }
  return fresh;
}

/** Which of a batch's NEVER-SEEN messages deserve the reader's attention.
 * `first` marks a room's initial page (history, never news); `away` is the
 * reader-not-looking verdict computed by the caller from the live document. */
export function notifiable(msgs: readonly FeedMsg[], opts: { first: boolean; away: boolean }): FeedMsg[] {
  if (opts.first || !opts.away) return [];
  return msgs.filter((m) => {
    const from = m.from ?? '';
    if (!from || from === 'human') return false;         // your own words
    if (systemLine(m.body) !== null) return false;       // app narration
    const note = statusNote(m.body);
    if (note && note.state !== 'done') return false;     // ambient progress
    return true;
  });
}

/** Title + body for the system notification. Composed from NAMES only —
 * no invented prose, so it reads the same in every UI language. */
export function notifyText(items: readonly FeedMsg[], project: string): { title: string; body: string } {
  const names = [...new Set(items.map((m) => m.from ?? ''))].filter(Boolean);
  const title = `${names.join(', ')} · ${project}`;
  return { title, body: excerpt(items[items.length - 1]?.body ?? '') };
}

/** A one-line reading of a message body: the status marker gives way to its
 * text, image refs give way to their prose, whitespace collapses, 120 chars. */
export function excerpt(body: string, max = 120): string {
  const note = statusNote(body);
  let text = note ? note.text : body;
  text = text.replace(/!\[[^\]]*\]\([^)]*\)/gu, '').replace(/\s+/gu, ' ').trim();
  return text.length > max ? text.slice(0, max - 1).trimEnd() + '…' : text;
}

/** Pure cooldown verdict, so the timing rule is testable without an Audio.
 * `lastCueAt <= 0` means "never played" (also what a rollback restores). */
export function cueDue(lastCueAt: number, now: number, cooldown = CUE_COOLDOWN_MS): boolean {
  return lastCueAt <= 0 || now - lastCueAt >= cooldown;
}

// ─── Effect layer (browser only, every path fail-soft) ──────────────────────

const state: NotifyState = { seen: new Set(), lastCueAt: 0 };

/** The persisted mute switch. Sound defaults ON (the owner's ask); the system
 * notification additionally needs the permission the bell click requests. */
export function notifyEnabled(): boolean {
  try { return localStorage.getItem(ENABLED_KEY) !== 'off'; } catch { return true; }
}

export function setNotifyEnabled(on: boolean): void {
  try { localStorage.setItem(ENABLED_KEY, on ? 'on' : 'off'); } catch { /* private mode */ }
}

const defaultPlay = (): Promise<void> => {
  const a = new Audio(CUE_SRC);
  a.volume = 0.6;
  return a.play();
};

/** Play the cue if the cooldown allows. The window is claimed up front (two
 * same-tick batches must not double-fire) but a REJECTED play rolls it back:
 * autoplay commonly blocks before the user's first gesture, and a failure
 * that still consumed the window would swallow the first REAL cue for
 * 3 seconds after the user unlocks (lead review, board #57). The rollback is
 * IDENTITY-GUARDED (lead follow-up): a rejection lands whenever the browser
 * pleases, so an OLD attempt's late catch must not reopen the window a NEWER
 * successful cue has claimed — it may only roll back the claim it made, i.e.
 * while `st.lastCueAt` still equals its own `now`. */
export function playCue(now = Date.now(), st: NotifyState = state, play: () => Promise<void> = defaultPlay): boolean {
  if (!cueDue(st.lastCueAt, now)) return false;
  const before = st.lastCueAt;
  st.lastCueAt = now;
  try {
    play().catch(() => { if (st.lastCueAt === now) st.lastCueAt = before; });
    return true;
  } catch {
    if (st.lastCueAt === now) st.lastCueAt = before;
    return false;
  }
}

/** The bell's own preview: a plain play on the user's gesture — it doubles as
 * the autoplay unlock, and a preview is never subject to the cooldown. */
export function previewCue(play: () => Promise<void> = defaultPlay): void {
  try { void play().catch(() => {}); } catch { /* no Audio */ }
}

/** Show a system notification when the API exists and is already granted —
 * never prompts here (prompting belongs to the bell's gesture, see below).
 * `silent: true` because the cue is ours. */
export function systemNotify(text: { title: string; body: string }): boolean {
  try {
    if (typeof Notification === 'undefined' || Notification.permission !== 'granted') return false;
    new Notification(text.title, { body: text.body, silent: true });
    return true;
  } catch {
    return false;
  }
}

/** Ask for notification permission — call from the BELL's click (a real user
 * gesture), because browsers ignore or penalize unprompted requests. */
export function ensurePermission(): void {
  try {
    if (typeof Notification !== 'undefined' && Notification.permission === 'default') {
      void Notification.requestPermission();
    }
  } catch {
    /* no Notification API — the sound remains */
  }
}

/** The one call sites use: remember the batch, gate the remainder, fire both
 * channels. Marking ALWAYS happens — first pages, watched batches and muted
 * periods are remembered so nothing backfills later. */
export function notifyNews(
  msgs: readonly FeedMsg[],
  ctx: { first: boolean; away: boolean; project: string },
  st: NotifyState = state,
  effects: { cue: typeof playCue; sys: typeof systemNotify; enabled: () => boolean } = { cue: playCue, sys: systemNotify, enabled: notifyEnabled },
): boolean {
  const fresh = sift(msgs, st.seen);
  const news = notifiable(fresh, ctx);
  if (!news.length || !effects.enabled()) return false;
  effects.cue(Date.now(), st);
  effects.sys(notifyText(news, ctx.project));
  return true;
}
