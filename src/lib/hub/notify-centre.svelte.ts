// The notification centre (board #322, owner 2026-10-09: "我经常听到「嘟嘟」的
// 通知了，但是都不知道是哪个窗口的提示…在整个界面设置一个通知中心，让我能看到到底
// 提示了什么…方便我快速定位到回复我的那条具体消息").
//
// A BOUNDED INDEX of alerts, not a second chat history: the room is the one
// record of what was said, and each entry here only names a message in it
// (room + seq + id) with a one-line excerpt to recognise it by. So:
// - Clearing removes entries only; it says nothing about whether any room is
//   read (the room's read mark is hubPrefs.seen, moved only by reading).
// - Viewing one entry never moves a room's read mark.
// - An entry is viewed when its jump landed on the message, or when its room
//   was read past it (markRoomRead), so a read room never leaves unviewed
//   alerts behind; a message that arrived while the reader was at that
//   room's visible tail is recorded already viewed.
// - The 50-entry cap evicts the oldest ENTRIES; a room's unread state is
//   unaffected (that is hub_unread's job).
// - It knows only what this running client received: nothing arrives while
//   the app is closed (notifications.md, the running-client limit).
//
// Per server: the list lives in `tmux_hub_alerts`, one of servers.ts
// PARKED_KEYS, so a switch parks it with the rest of the server's Hub state
// and `reload()` reads the target's. Entries also carry the server id, and a
// jump request carries it too, so a late jump cannot act on another server.
import type { FeedMsg, NewsKind } from './notifications.ts';

export const ALERTS_KEY = 'tmux_hub_alerts';
export const CENTRE_CAP = 50;

export interface Alert {
  /** `room|id` — one entry per message. */
  key: string;
  server: string;
  room: string;
  session: string;
  project: string;
  seq: number;
  id: string;
  ts: number;
  from: string;
  /** `to` names the human (#289): emphasised, and the To-me filter's set. */
  toHuman: boolean;
  kind: NewsKind;
  excerpt: string;
  viewed: boolean;
  /** Why the last jump did not land (shown on the row; cleared on success). */
  failed?: string;
}

export interface JumpRequest { alert: Alert; n: number }

type Store = Pick<Storage, 'getItem' | 'setItem'>;

function read(storage: Store | null): Alert[] {
  try {
    const raw = JSON.parse(storage?.getItem(ALERTS_KEY) ?? '[]');
    return Array.isArray(raw) ? raw.filter((a) => a && typeof a.key === 'string' && typeof a.room === 'string').slice(0, CENTRE_CAP) : [];
  } catch { return []; }
}

/** Pure: the list after recording `alert` — newest first, one per key (a
 * second record of the same message keeps the first's viewed state unless the
 * new one is viewed), capped. Returns the same array when nothing changed. */
export function withAlert(list: readonly Alert[], alert: Alert, cap = CENTRE_CAP): Alert[] {
  const prev = list.find((a) => a.key === alert.key);
  if (prev && (prev.viewed || !alert.viewed)) return list as Alert[];
  const next = [{ ...alert, viewed: alert.viewed || !!prev?.viewed }, ...list.filter((a) => a.key !== alert.key)];
  next.sort((a, b) => b.ts - a.ts || b.seq - a.seq);
  return next.slice(0, cap);
}

/** Pure: every entry of `session` at or below `seq` is viewed. */
export function readThrough(list: readonly Alert[], session: string, seq: number): Alert[] {
  if (!list.some((a) => a.session === session && !a.viewed && a.seq > 0 && a.seq <= seq)) return list as Alert[];
  return list.map((a) => (a.session === session && !a.viewed && a.seq > 0 && a.seq <= seq ? { ...a, viewed: true, failed: undefined } : a));
}

export function alertOf(m: FeedMsg & { seq?: number; room?: string; to?: unknown }, ctx: {
  server: string; room: string; session: string; project: string; kind: NewsKind; viewed: boolean; excerpt: string;
}): Alert {
  const id = String(m.id ?? `${m.from ?? ''}:${m.ts ?? 0}`);
  return {
    key: `${ctx.room}|${id}`, server: ctx.server, room: ctx.room, session: ctx.session, project: ctx.project,
    seq: Number(m.seq) || 0, id, ts: m.ts ?? Date.now(), from: m.from ?? '',
    toHuman: Array.isArray(m.to) && m.to.includes('human'), kind: ctx.kind, excerpt: ctx.excerpt, viewed: ctx.viewed,
  };
}

const storage = (): Store | null => (typeof localStorage === 'undefined' ? null : localStorage);
const state = $state({ items: read(storage()), open: null as HTMLElement | null, jump: null as JumpRequest | null });
let jumps = 0;

function save(next: Alert[]) {
  if (next === state.items) return;
  state.items = next;
  try { storage()?.setItem(ALERTS_KEY, JSON.stringify(next)); } catch { /* private mode */ }
}

export const centre = {
  get items(): readonly Alert[] { return state.items; },
  get unviewed(): number { return state.items.filter((a) => !a.viewed).length; },
  record(alert: Alert) { save(withAlert(state.items, alert)); },
  markRoomRead(session: string, seq: number) { save(readThrough(state.items, session, seq)); },
  viewed(key: string) { save(state.items.map((a) => (a.key === key ? { ...a, viewed: true, failed: undefined } : a))); },
  failed(key: string, reason: string) { save(state.items.map((a) => (a.key === key ? { ...a, failed: reason } : a))); },
  clear() { save([]); },
  /** After a server switch pointed the live key at another server's list. */
  reload() { state.items = read(storage()); state.jump = null; state.open = null; },
  /** The popover's anchor: whichever entry opened it (rail bell, Hub header). */
  get anchor() { return state.open; },
  toggle(trigger: HTMLElement) { state.open = state.open === trigger ? null : trigger; },
  close() { state.open = null; },
  /** A request for the Hub to open this entry's message; `n` makes a newer
   * request supersede an older one still in flight. */
  get jump(): JumpRequest | null { return state.jump; },
  requestJump(alert: Alert) { state.jump = { alert, n: ++jumps }; },
  isCurrent(n: number) { return state.jump?.n === n; },
  /** The request `n` is done (landed, failed, or invalidated): it is never
   * replayed. A newer request is left alone. */
  consume(n: number) { if (state.jump?.n === n) state.jump = null; },
};
