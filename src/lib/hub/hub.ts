// Pure display logic for the Hub view — testable with node --test, no Svelte.
import type { HubAgent, HubActivityEvent } from '../core/ws.ts';

/** Derived-state → dot color (theme token values are resolved in CSS; these
 * are the fallback literals used inline for dynamic dots). */
export function stateDotColor(state: string): string {
  switch (state) {
    case 'running':
    case 'working': return 'var(--status-ok)';   // 'working' = pre-2026-08 name
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

/** "2m14s" / "1h03m" / "12s" — how long the current state has held. Compact on
 * purpose: it sits inside an agent chip, and the point is the order of
 * magnitude, not the precision. `since` is epoch SECONDS (what the server
 * reports); `now` is epoch ms so the caller can pass a ticking clock. */
export function fmtElapsed(since: number, now: number): string {
  if (!since) return '';
  const s = Math.max(0, Math.floor(now / 1000) - since);
  if (s < 60) return `${s}s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m${String(s % 60).padStart(2, '0')}s`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h${String(m % 60).padStart(2, '0')}m`;
  return `${Math.floor(h / 24)}d${String(h % 24).padStart(2, '0')}h`;
}

/** Agents whose newest message the user has not seen yet — the red-dot rule.
 * Keyed by sender name, so a room where three agents replied marks all three.
 * `seenTs` is the newest message timestamp the user has looked at (ms).
 * Lifecycle lines (`[tmm] stopped dev`) are posted under the agent's name but
 * are not replies, so they never raise the dot. */
export function unreadSenders(feed: readonly { ts?: number; from?: string; body?: string }[], seenTs: number): Set<string> {
  const out = new Set<string>();
  for (const m of feed) {
    const from = m.from ?? '';
    if (!from || from === 'human') continue;
    if (systemLine(m.body) !== null) continue;
    if ((m.ts ?? 0) > seenTs) out.add(from);
  }
  return out;
}

/** Markdown image references in a message body, split from the prose.
 *
 * `tmm send --image` appends `![](src)` lines, and agents write the same syntax
 * by hand. They are pulled OUT of the markdown rather than left to the renderer
 * because a local path is not a URL a webview can load: the src has to go
 * through the file service first, which means the client needs the list, not an
 * `<img>` tag it would have to rewrite afterwards. Text keeps its markdown. */
export function splitImages(body: string | null | undefined): { text: string; images: string[] } {
  const images: string[] = [];
  const text = (body ?? '')
    .replace(/!\[[^\]]*\]\(\s*([^)\s]+)[^)]*\)/g, (_m, src: string) => {
      images.push(src);
      return '';
    })
    .replace(/\n{3,}/g, '\n\n')
    .trim();
  return { text, images };
}

/** True when a reference is already something a webview can load directly; a
 * filesystem path has to be fetched through the file service instead. */
export function isDirectUrl(src: string): boolean {
  return /^(https?:|data:|blob:)/i.test(src);
}

/** Agent slots the project DECLARES that have no live window right now — a
 * stopped agent. They belong in the roster (greyed, with a start action)
 * because an agent you stopped has not left the project: its isolated home and
 * its conversation id are still on disk, so starting it again resumes it. */
export function stoppedAgents(
  slots: readonly { window_name: string; kind?: string }[] | undefined,
  live: readonly HubAgent[],
): string[] {
  const running = new Set(live.map((a) => a.name));
  return (slots ?? [])
    .filter((s) => String(s.kind ?? '').toLowerCase() === 'agent' && !running.has(s.window_name))
    .map((s) => s.window_name);
}

/** A colour for a tool NAME, by what the tool does. A wall of grey monospace is
 * hard to read; the same four buckets in the same colours make a run of steps
 * scannable without a legend. Names differ per backend (`fs_read`, `Read`,
 * `execute_bash`, `Bash`, `web_search`…), so match on substrings rather than an
 * exhaustive table — an unknown tool falls back to neutral rather than to a
 * misleading colour. */
export function toolColor(tool: string | null | undefined): string {
  const t = (tool ?? '').toLowerCase();
  if (!t) return 'var(--text3)';
  if (/(write|edit|create|insert|replace|delete|remove|rename|mv|patch|apply)/.test(t)) {
    return 'var(--status-warn)';       // it changes something
  }
  if (/(bash|shell|exec|command|run|terminal|process)/.test(t)) {
    return 'var(--status-ok)';         // it runs something
  }
  if (/(search|grep|find|fetch|web|http|browse|url|query)/.test(t)) {
    return 'var(--accent)';            // it looks something up
  }
  if (/(read|list|stat|cat|view|ls|glob|tree|show)/.test(t)) {
    return 'var(--tool-read, #7aa2f7)'; // it reads something
  }
  return 'var(--text2)';
}

/** How many steps a group shows before "show all". A run of forty tool calls is
 * a wall; the last handful is what tells you where the agent is. */
export const STEPS_PREVIEW = 5;

/** Advance the ONE user-message anchor without inventing a second component.
 *
 * The active message is selected while its real bubble is naturally visible:
 * scrolling down chooses the newest visible message and gives that SAME element
 * a top sticky edge; scrolling up chooses the oldest visible one and gives it a
 * bottom edge. It therefore moves with the feed first and only catches when it
 * is about to leave. While a long reply contains no user message, keep the same
 * active element — never switch at an invisible midpoint.
 *
 * `current` is also the seed across direction reversals in an empty gap. If the
 * page opens/jumps directly into a gap, select only the message already passed
 * in that direction. */
export function pickAnchor(
  items: readonly { key: string; top: number; height: number }[],
  scrollTop: number,
  clientHeight: number,
  scrollHeight: number,
  direction: 'up' | 'down',
  current: { key: string; edge: 'top' | 'bottom' | '' } = { key: '', edge: '' },
): { key: string; edge: 'top' | 'bottom' | '' } {
  const none = { key: '', edge: '' } as const;
  if (scrollHeight <= clientHeight + 1 || !items.length) return none;
  const viewBottom = scrollTop + clientHeight;
  const visible = items.filter((it) => it.top < viewBottom - 1 && it.top + it.height > scrollTop + 1);
  if (visible.length) {
    const picked = direction === 'down' ? visible[visible.length - 1]! : visible[0]!;
    return { key: picked.key, edge: direction === 'down' ? 'top' : 'bottom' };
  }
  // No new bubble entered naturally: the current one keeps holding its edge.
  if (current.key && items.some((it) => it.key === current.key)) return current;
  if (direction === 'down') {
    const passed = items.filter((it) => it.top + it.height <= scrollTop + 1).at(-1);
    return passed ? { key: passed.key, edge: 'top' } : none;
  }
  const ahead = items.find((it) => it.top >= viewBottom - 1);
  return ahead ? { key: ahead.key, edge: 'bottom' } : none;
}

export type FeedLevel = 'chat' | 'status' | 'tools';

/** Lifecycle lines the server posts into the room (a spawn, a `tmm done`) are
 * events, not prose, and render as a centered system line rather than a chat
 * bubble. The marker is `[tmm] `; the two glyph prefixes are the pre-2026-08
 * spelling and stay recognized because the room is persisted — old messages
 * must not regress into bubbles. Returns the text without its marker, or null
 * when the body is ordinary prose. */
/// Wrap a message's leading @recipient — the address, the composer's own
/// prefix — in a span the bubble can set apart. Only the FIRST mention and
/// only at the very start of the first paragraph: a mention mid-text is
/// content, not an address. Works on rendered markdown HTML so the span
/// stays inline in the first line box.
export function markLeadingMention(html: string): string {
  return html.replace(/^(\s*<p>)(@[\w][\w.-]*)(?=[\s<,，:：、!！?？]|$)/, '$1<span class="m-to">$2</span>');
}

/** The spawn kick's echo: `[YYYY-MM-DD HH:MM] (session start)`. Also matches
 * the pre-2026-08-18 instruction kick, because rooms are persisted. */
export function isSessionStart(text: string | null | undefined): boolean {
  const t = (text ?? '').trim();
  return /^\[[^\]]+\]\s*\(session start\)$/.test(t) || /^\[[^\]]+\]\s*Start now:/.test(t);
}

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
  | { type: 'sys'; ts: number; key: string; items: string[] }
  | { type: 'prompt'; ts: number; window: number; text: string }
  | { type: 'note'; ts: number; window: number; event: HubActivityEvent }
  | { type: 'steps'; ts: number; window: number; key: string; events: HubActivityEvent[] };

/** Internal: a tool call before consecutive ones are folded into a group. */
type ToolItem = { type: 'tool'; ts: number; window: number; event: HubActivityEvent };

/** `tmm` subcommands whose EFFECT is already a row in this timeline: the
 * message, the status change, the completion, the spawn notice. Showing the
 * tool call that produced them would print the same event twice — once as the
 * thing the agent said, once as the mechanics of saying it. `tmm log` is
 * filtered for the same reason inverted: polling the room produces nothing to
 * see. Everything else (`tmm task`, `project`, `agent`, `skill`…) has no other
 * trace in the chat, so it stays visible. */
const TMM_SELF_REPORT = new Set(['send', 'status', 'done', 'log', 'spawn']);

/** Normalize a tool event across server generations. An older server had no
 * `tool` field and spelled the event as one string, `"shell tmm send …"` — the
 * tool name glued onto the front of the text. Measured live (2026-08-18): that
 * shape is what made every tool name grey (the coloured column only renders
 * when `tool` is set) AND what defeated the self-report filter (the first
 * token was `shell`, not `tmm`). Split the known lead-in words back out;
 * events that already carry `tool` pass through untouched. */
const LEGACY_TOOL_PREFIX = /^(shell|bash|exec)\s+(\S[\s\S]*)$/;
export function toolEventParts(e: HubActivityEvent): { tool: string; text: string } {
  if (e.tool) return { tool: e.tool, text: e.text };
  const m = LEGACY_TOOL_PREFIX.exec(e.text.trim());
  return m ? { tool: m[1]!, text: m[2]! } : { tool: '', text: e.text };
}

/** True when this tool call is the agent reporting through `tmm` — the call
 * whose own output is already shown as a message or a note. Agents chain the
 * report onto one shell line (`tmm send "…" 2>&1; tmm status working "…"`), so
 * the whole command is a self-report only if EVERY chained segment is one —
 * `tmm send "done" && make deploy` still has something to show. */
export function isSelfReport(e: HubActivityEvent): boolean {
  if (e.kind !== 'tool') return false;
  const { text } = toolEventParts(e);
  const segments = text.split(/(?:&&|\|\||;)+/);
  let sawTmm = false;
  for (const seg of segments) {
    const parts = seg.trim().split(/\s+/);
    if (!parts[0]) continue;                        // empty tail after a ';'
    const cmd = parts[0].split('/').pop() ?? '';
    if (cmd !== 'tmm' || !TMM_SELF_REPORT.has(parts[1] ?? '')) return false;
    sawTmm = true;
  }
  return sawTmm;
}

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
  // Lifecycle lines ("[tmm] spawned dev") are the app's record, not the
  // conversation: at the chat-only level they disappear, and elsewhere they
  // become 'sys' rows so CONSECUTIVE ones fold into one line — a stop
  // followed by a restart is one fact, not two rows (owner report).
  const msgs: Extract<FeedBlock, { type: 'msg' | 'sys' }>[] = feed.flatMap((m): Extract<FeedBlock, { type: 'msg' | 'sys' }>[] => {
    const sys = systemLine(m.body);
    if (sys === null) {
      return [{ type: 'msg' as const, ts: m.ts ?? 0, msg: m, delivered: false }];
    }
    if (level === 'chat') return [];
    return [{ type: 'sys' as const, ts: m.ts ?? 0, key: `sys${m.id ?? m.ts}`, items: [sys] }];
  });

  // Rule 1: pair echoes with the messages that produced them.
  const consumed = new Set<HubActivityEvent>();
  for (const e of activity) {
    if (e.kind !== 'prompt' || e.via !== 'app') continue;
    // The newest message at or before the echo whose body it contains. The
    // typed line is `[tmm chat] <from>: <body>`, and an agent that was
    // mid-typing submits it with its own leftover text attached.
    let hit: Extract<FeedBlock, { type: 'msg' }> | undefined;
    for (const m of msgs) {
      if (m.type !== 'msg') continue;
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
      // The agent's own `tmm send/status/done` is already a row of its own.
      if (isSelfReport(e)) continue;
      // Dedup on the whole call, not the argument: two different tools acting
      // on one file are two facts, and pre+post of one call are one.
      const sig = `${e.tool ?? ''}\u0000${e.text}`;
      if (lastTool.get(e.window) === sig) continue;
      lastTool.set(e.window, sig);
      stream.push({ type: 'tool', ts: e.ts, window: e.window, event: e });
      continue;
    }
    if (e.kind === 'prompt') {
      // The spawn kick is machinery, not conversation: it is a marker
      // (`[time] (session start)`) that only exists because a CLI does
      // nothing until spoken to. Showing it made the app look like it had
      // typed instructions at the agent in the operator's name.
      if (isSessionStart(e.text)) continue;
      stream.push({ type: 'prompt', ts: e.ts, window: e.window, text: e.text });
      continue;
    }
    // A finished turn is not news: the agent's reply is right there as a
    // message, and its chip goes idle. A row saying "finished a turn" after
    // every answer was just noise in the transcript (owner call, 2026-08-16).
    // The other lifecycle events all mean something is WAITING on a human.
    if (e.kind === 'notif' && e.text === 'completed') continue;
    stream.push({ type: 'note', ts: e.ts, window: e.window, event: e });
  }
  // Chronological, and when two things share a timestamp the OBSERVATION comes
  // first: a reply is what ends a turn, so the tool calls of that turn happened
  // before it. Ties are not hypothetical — the server stamps a hook event when
  // it consumes the file, so a turn's last tool call and its auto-posted reply
  // can land in the same millisecond.
  const rank = (i: FeedBlock | ToolItem) => (i.type === 'msg' ? 1 : 0);
  stream.sort((a, b) => a.ts - b.ts || rank(a) - rank(b));

  // Rule 3: fold consecutive same-window tool calls.
  const out: FeedBlock[] = [];
  for (const item of stream) {
    if (item.type === 'sys') {
      const prev = out[out.length - 1];
      if (prev?.type === 'sys') {
        prev.items.push(...item.items);
        continue;
      }
      out.push(item);
      continue;
    }
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
