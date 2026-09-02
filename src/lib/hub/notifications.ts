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
// Settings toggle's own click (a real user gesture that also previews the cue)
// — never an unrelated send. The choice persists. The toggle lived in the Hub
// header until board #72: a header is not a place to keep spare switches, and
// on a phone the switch cost the row a button (owner, 2026-09-02).
//
// WHERE the check runs matters as much as the gate (board #72): the feed POLL
// only runs while the Hub page is on screen, so a poll-only notifier could
// never fire for "another page is visible" — the very case a phone user is
// in after switching to Terminal. The `team_message` PUSH arrives for every
// room whatever page is showing, so `Hub.onPush` is the primary call site and
// the poll is the fallback; `sift` makes push-then-poll alert exactly once
// because both carry the server's message id.
//
// NATIVE on the phone and the desktop app (board #72, owner: "接入到安卓的消息
// 通知里…手机提示什么 project 谁完成了什么任务"): the Android and macOS webviews
// have no Web Notification API of their own, so `tauri-plugin-notification`
// (registered in lib.rs, `notification:default` capability) INJECTS a
// `window.Notification` shim that routes the constructor to the OS tray and
// `requestPermission` to the runtime prompt (Android 13+ POST_NOTIFICATIONS).
// So the one web path below IS the native path inside Tauri — no plugin
// import, no second channel. Two shim facts the code has to respect: its
// `permission` reads `denied` (not `default`) before the user was ever asked,
// so `ensurePermission` asks whenever it is not `granted`; and it settles
// asynchronously at startup, so a caption read at mount may lag one tick.
import { systemLine, statusNote, boardLine } from './hub.ts';

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

/** The reader-not-looking verdict for the SELECTED room, from the live
 * document: tab hidden, window unfocused, or the Hub page not on screen.
 * A message in any OTHER room is away by definition — the caller passes
 * `true` without asking. */
export function isAway(visible: boolean, doc: { hidden: boolean; hasFocus(): boolean } = document): boolean {
  return doc.hidden || !doc.hasFocus() || !visible;
}

/** The project NAME a bus room belongs to, for the notification title: the
 * row whose recorded `room` (fallback `proj:<session>`) matches, else the
 * session spelled by the room itself — never an empty title. */
export function roomProjectName(
  rows: readonly { project: { name?: string; session?: string; room?: string | null } }[],
  room: string,
): string {
  const hit = rows.find((r) => (r.project.room ?? `proj:${r.project.session ?? ''}`) === room);
  return hit?.project.name || room.replace(/^proj:/u, '');
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

/** A board line that says a TASK WAS FINISHED: an agent moved an issue to
 * review (handoff — a person is waited on) or done. Every other lifecycle
 * line (spawned, started, a move to doing…) stays app narration. */
export function taskFinished(body: string | null | undefined): { id: string; to: string; title: string } | null {
  const b = boardLine(systemLine(body));
  return b && (b.to === 'review' || b.to === 'done') ? { id: b.id, to: b.to, title: b.title } : null;
}

/** Which of a batch's NEVER-SEEN messages deserve the reader's attention.
 * `first` marks a room's initial page (history, never news); `away` is the
 * reader-not-looking verdict computed by the caller from the live document.
 * News is what the owner asked the phone to say — "谁完成了什么任务": an
 * agent's reply, its `[tmm done]` summary, and a board move to review/done. */
export function notifiable(msgs: readonly FeedMsg[], opts: { first: boolean; away: boolean }): FeedMsg[] {
  if (opts.first || !opts.away) return [];
  return msgs.filter((m) => {
    const from = m.from ?? '';
    if (!from || from === 'human') return false;         // your own words
    if (systemLine(m.body) !== null) return taskFinished(m.body) !== null; // narration, unless a task finished
    const note = statusNote(m.body);
    if (note && note.state !== 'done') return false;     // ambient progress
    return true;
  });
}

/** Title + body for the system notification. Composed from NAMES only —
 * no invented prose, so it reads the same in every UI language. The `tag`
 * is per project, so one room's burst collapses to one tray card. */
export function notifyText(items: readonly FeedMsg[], project: string): { title: string; body: string; tag: string } {
  const names = [...new Set(items.map((m) => m.from ?? ''))].filter(Boolean);
  const title = `${names.join(', ')} · ${project}`;
  return { title, body: excerpt(items[items.length - 1]?.body ?? ''), tag: `tmm:${project}` };
}

/** A one-line reading of a message body: the status marker gives way to its
 * text, a finished-task line to `#N → review · title`, image refs give way
 * to their prose, whitespace collapses, 120 chars. */
export function excerpt(body: string, max = 120): string {
  const note = statusNote(body);
  const task = taskFinished(body);
  let text = task ? `#${task.id} → ${task.to}${task.title ? ` · ${task.title}` : ''}` : note ? note.text : body;
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
 * notification additionally needs the permission the Settings toggle requests. */
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

/** The toggle's own preview: a plain play on the user's gesture — it doubles as
 * the autoplay unlock, and a preview is never subject to the cooldown. */
export function previewCue(play: () => Promise<void> = defaultPlay): void {
  try { void play().catch(() => {}); } catch { /* no Audio */ }
}

/** What the platform can do, for the Settings row's caption: `unsupported`
 * means no Notification API at all (a plain webview without the plugin —
 * the cue is the whole channel there), `denied` means the site or the OS
 * refused (or, inside Tauri, that nobody has asked yet — see the shim note
 * at the top; turning the setting on asks). */
export type NotifyPermission = 'granted' | 'denied' | 'default' | 'unsupported';
export function notifyPermission(): NotifyPermission {
  try {
    if (typeof Notification === 'undefined') return 'unsupported';
    const p = Notification.permission;
    return p === 'granted' || p === 'denied' ? p : 'default';
  } catch {
    return 'unsupported';
  }
}

export type NotifyEnv = {
  /** `Notification.permission`, or null where the API is missing. */
  permission: string | null;
  /** `new Notification(...)` — REFUSED on Android Chrome ("Illegal
   * constructor": a page may not construct one, only a service worker may
   * show one), which is why it is never the only path. */
  construct: (title: string, opts: NotificationOptions) => unknown;
  /** The service-worker registration, when the page has one (main.ts
   * registers `/sw.js` on every non-Tauri secure origin). */
  registration: () => Promise<{ showNotification(title: string, opts: NotificationOptions): Promise<void> } | undefined>;
};

const defaultEnv = (): NotifyEnv => ({
  permission: typeof Notification === 'undefined' ? null : Notification.permission,
  construct: (title, opts) => new Notification(title, opts),
  registration: async () => {
    const sw = typeof navigator === 'undefined' ? undefined : navigator.serviceWorker;
    if (!sw) return undefined;
    const reg = await sw.getRegistration();
    return reg?.active ? reg : undefined;
  },
});

/** Show a system notification when the API exists and is already granted —
 * never prompts here (prompting belongs to the Settings toggle's gesture, see
 * below). `silent: true` because the cue is ours. `tag` is the project, so a
 * burst from one room REPLACES its earlier card in the tray instead of
 * stacking twenty (over-push was half the owner's worry, board #72).
 *
 * Prefers `ServiceWorkerRegistration.showNotification`: on Android Chrome the
 * page-level constructor throws, so the PWA on a phone showed NOTHING and the
 * catch below quietly said so. The worker path is fire-and-forget; when there
 * is no registration (desktop browser without the SW, or the SW not yet
 * active) the constructor is the fallback. Returns whether a channel was
 * attempted. */
export function systemNotify(text: { title: string; body: string; tag?: string }, env: NotifyEnv = defaultEnv()): boolean {
  if (env.permission !== 'granted') return false;
  const opts: NotificationOptions = { body: text.body, silent: true, ...(text.tag ? { tag: text.tag } : {}) };
  const construct = (): boolean => {
    try { env.construct(text.title, opts); return true; } catch { return false; }
  };
  try {
    env.registration()
      .then((reg) => { if (reg) return reg.showNotification(text.title, opts).catch(() => { construct(); }); construct(); })
      .catch(() => { construct(); });
    return true;
  } catch {
    return construct();
  }
}

/** Ask for notification permission — call from the Settings toggle's click
 * (a real user gesture), because browsers ignore or penalize unprompted
 * requests. Resolves once the prompt settles (or at once when there is
 * nothing to ask), so the caller can re-read `notifyPermission()`. */
export async function ensurePermission(): Promise<void> {
  try {
    // `!== 'granted'`, not `=== 'default'`: the Tauri shim reports `denied`
    // before the first ask, and a browser answers a re-ask on a denied site
    // immediately without prompting — so asking is always safe and only
    // this form ever reaches the Android runtime prompt.
    if (typeof Notification !== 'undefined' && Notification.permission !== 'granted') {
      await Notification.requestPermission();
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
