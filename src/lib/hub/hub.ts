// Pure display logic for the Hub view — testable with node --test, no Svelte.
import type { HubAgent, HubActivityEvent } from '../core/ws.ts';

/** Derived-state → dot color (theme token values are resolved in CSS; these
 * are the fallback literals used inline for dynamic dots). */
export function stateDotColor(state: string): string {
  switch (state) {
    case 'working': return 'var(--status-ok)';
    case 'waiting': return 'var(--status-warn)';
    case 'blocked':
    case 'stuck':
    case 'failed': return 'var(--status-danger)';
    case 'shell': return 'var(--text3)';
    default: return 'var(--status-sleep)'; // idle
  }
}

/** Merge new chat messages into the feed without duplicates, oldest first.
 * Pushes and cursor polls overlap; identity is the message id when present,
 * else (ts, from, body). */
export function mergeMessages<T extends { id?: string; ts?: number; from?: string; body?: string }>(
  existing: T[],
  incoming: T[],
): T[] {
  const key = (m: T) => m.id ?? `${m.ts}|${m.from}|${m.body}`;
  const seen = new Set(existing.map(key));
  const merged = existing.slice();
  for (const m of incoming) {
    if (!seen.has(key(m))) {
      seen.add(key(m));
      merged.push(m);
    }
  }
  merged.sort((a, b) => (a.ts ?? 0) - (b.ts ?? 0));
  return merged;
}

/** Backend identity colors (the prototype's palette): consistent across
 * sidebar, cards and registry so a glance identifies who is who. */
export function backendColor(backend: string | null | undefined): string {
  switch (backend) {
    case 'kiro': return '#a78bfa';
    case 'claude': return '#fb923c';
    case 'codex': return '#94a3b8';
    case 'kimi': return '#4ade80';
    default: return '#818cf8';
  }
}

export interface StatuslineWindow {
  window: number;
  label: string;
  current: boolean;
}

/** tmux's own notation: `2:reviewer*` marks the current window. Windows are
 * listed in index order; the `*` suffix goes to the window the terminal
 * column is showing. */
export function statuslineWindows(agents: HubAgent[], termTarget: string): StatuslineWindow[] {
  const m = /^.+:(\d+)\.\d+$/.exec(termTarget || '');
  const cur = m ? Number(m[1]) : -1;
  return agents
    .slice()
    .sort((a, b) => a.window - b.window)
    .map((a) => ({
      window: a.window,
      label: `${a.window}:${a.name}${a.window === cur ? '*' : ''}`,
      current: a.window === cur,
    }));
}

/** Who the composer talks to when the user has not chosen anyone.
 *
 * A conversation has ONE default recipient — the lead. Typing `@name` to reach
 * the only agent in the room is ceremony, so the room picks a lead and the
 * composer addresses it silently. Order of preference:
 *   1. the stored choice for this project, while that agent is still present;
 *   2. the only managed agent, when there is exactly one;
 *   3. an agent whose registry definition can hire (that IS the lead role);
 *   4. the lowest window index, so the answer is stable rather than arbitrary.
 * Returns '' when the project has no managed agent to talk to. */
export function pickLead(
  agents: readonly HubAgent[],
  registry: readonly { name: string; can_hire?: boolean }[],
  stored?: string | null,
): string {
  const managed = agents.filter((a) => a.managed);
  if (!managed.length) return '';
  if (stored && managed.some((a) => a.name === stored)) return stored;
  if (managed.length === 1) return managed[0]!.name;
  const canHire = new Set(registry.filter((r) => r.can_hire).map((r) => r.name));
  const lead = managed.find((a) => canHire.has(a.name));
  if (lead) return lead.name;
  return managed.slice().sort((a, b) => a.window - b.window)[0]!.name;
}

/** The body to post for `text` addressed at `to`. `''` means everyone (the
 * room's broadcast), and an explicit `@` anywhere means the user is addressing
 * people by hand — never rewrite that. */
export function addressed(text: string, to: string): string {
  const body = text.trim();
  if (!body || body.includes('@')) return body;
  return to ? `@${to} ${body}` : body;
}

export type FeedLevel = 'chat' | 'status' | 'tools';

/** Lifecycle lines the server posts into the room (a spawn, a `tmm done`) are
 * events, not prose, and render as a centered system line rather than a chat
 * bubble. The marker is `[tmm] `; the two glyph prefixes are the pre-2026-08
 * spelling and stay recognized because the room is persisted — old messages
 * must not regress into bubbles. Returns the text without its marker, or null
 * when the body is ordinary prose. */
export function systemLine(body: string | null | undefined): string | null {
  for (const marker of ['[tmm] ', '⚡ ', '✔ ']) {
    if (body?.startsWith(marker)) return body.slice(marker.length);
  }
  return null;
}

/** One row of the conversation. `msg` and `prompt` are things that were said,
 * `note` is a single observed fact, `steps` is a collapsible run of tool calls
 * (the "what it did between two replies" pane). */
export type FeedBlock =
  | { type: 'msg'; ts: number; msg: any; delivered: boolean }
  | { type: 'prompt'; ts: number; window: number; text: string }
  | { type: 'note'; ts: number; window: number; event: HubActivityEvent }
  | { type: 'steps'; ts: number; window: number; key: string; events: HubActivityEvent[] };

/** Internal: a tool call before consecutive ones are folded into a group. */
type ToolItem = { type: 'tool'; ts: number; window: number; event: HubActivityEvent };

/**
 * Build the conversation from chat messages plus observed telemetry.
 *
 * Three rules carry the design:
 *
 * 1. **A delivery receipt is not telemetry.** `deliver_mentions` types a line
 *    into an agent's pane; the agent's `userPromptSubmit` hook echoing that line
 *    back is the only proof it was accepted as a prompt. Such an echo arrives as
 *    a `prompt` event with `via: 'app'`, and it is consumed here to mark the
 *    message that caused it as delivered rather than shown as a separate row —
 *    the text would otherwise appear twice. This runs at EVERY feed level,
 *    because "did what I just sent arrive" is not a detail the user opted into.
 * 2. **A local prompt is the input half of the transcript.** Text typed at the
 *    agent's own keyboard exists in no other channel, so an unmatched `prompt`
 *    event renders as its own row.
 * 3. **Tool calls collapse, replies do not.** Consecutive tool events from the
 *    same window fold into one `steps` group; anything else (a message, a status
 *    declaration, a lifecycle notification) ends the run, which is what makes a
 *    group mean "between these two replies". Duplicate consecutive tool lines
 *    are dropped per window first (pre+post fire for one call).
 */
export function feedBlocks(
  feed: any[],
  activity: readonly HubActivityEvent[],
  level: FeedLevel,
): FeedBlock[] {
  const msgs: Extract<FeedBlock, { type: 'msg' }>[] = feed.map((m) => ({
    type: 'msg',
    ts: m.ts ?? 0,
    msg: m,
    delivered: false,
  }));

  // Rule 1: pair echoes with the messages that produced them.
  const consumed = new Set<HubActivityEvent>();
  for (const e of activity) {
    if (e.kind !== 'prompt' || e.via !== 'app') continue;
    // The newest message at or before the echo whose body it contains. The
    // typed line is `[tmm chat] <from>: <body>`, and an agent that was
    // mid-typing submits it with its own leftover text attached.
    let hit: (typeof msgs)[number] | undefined;
    for (const m of msgs) {
      const body = m.msg?.body ?? '';
      if (body && m.ts <= e.ts && e.text.includes(body)) hit = m;
    }
    if (hit) {
      hit.delivered = true;
      consumed.add(e);
    }
  }

  const stream: (FeedBlock | ToolItem)[] = [...msgs];
  const lastTool = new Map<number, string>();
  for (const e of activity) {
    if (consumed.has(e)) continue;
    // A line that never came back is about the message, not about telemetry:
    // it survives even the chat-only level.
    if (e.kind === 'warn') {
      stream.push({ type: 'note', ts: e.ts, window: e.window, event: e });
      continue;
    }
    if (level === 'chat') continue;
    if (e.kind === 'tool') {
      if (level !== 'tools') continue;
      if (lastTool.get(e.window) === e.text) continue;
      lastTool.set(e.window, e.text);
      stream.push({ type: 'tool', ts: e.ts, window: e.window, event: e });
      continue;
    }
    if (e.kind === 'prompt') {
      stream.push({ type: 'prompt', ts: e.ts, window: e.window, text: e.text });
      continue;
    }
    stream.push({ type: 'note', ts: e.ts, window: e.window, event: e });
  }
  stream.sort((a, b) => a.ts - b.ts);

  // Rule 3: fold consecutive same-window tool calls.
  const out: FeedBlock[] = [];
  for (const item of stream) {
    if (item.type !== 'tool') {
      out.push(item);
      continue;
    }
    const prev = out[out.length - 1];
    if (prev?.type === 'steps' && prev.window === item.window) {
      prev.events.push(item.event);
      continue;
    }
    out.push({
      type: 'steps',
      ts: item.ts,
      window: item.window,
      key: `w${item.window}-${item.ts}`,
      events: [item.event],
    });
  }
  return out;
}
