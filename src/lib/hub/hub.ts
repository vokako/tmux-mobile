// Pure display logic for the Hub view — testable with node --test, no Svelte.
import type { HubAgent, HubActivityEvent } from '../core/ws.ts';

/**
 * THE status colour language — one progression, read at a glance (owner,
 * 2026-08-20: "不同的颜色应该是渐进式的 … 设计好颜色所表达的含义"):
 *
 *   accent (cyan)  = MOTION      — a turn is open, work is happening now
 *                                  (running/working; the same colour the
 *                                  progress row's lane bar and the tool
 *                                  lane's pulse already use)
 *   ok (green)     = SUCCESS     — finished well (done)
 *   warn (amber)   = NEEDS YOU   — paused on a person (waiting/blocked)
 *   danger (red)   = FAILED      — the only distress signal
 *   sleep (grey)   = AT REST     — nothing happening, nothing wrong (idle)
 *
 * The progression start → running → done reads accent → green, which matches
 * the CI intuition (a spinner is never green; green means it ENDED well).
 * `working` used to be green, which made every busy agent look already
 * finished. Both readers below speak this one language; do not fork it.
 *
 * Derived-state → dot color (theme token values are resolved in CSS; these
 * are the fallback literals used inline for dynamic dots).
 */
export function stateDotColor(state: string): string {
  switch (state) {
    case 'running':
    case 'working': return 'var(--accent)';       // 'working' = pre-2026-08 name
    case 'waiting':
    case 'blocked': return 'var(--status-warn)';  // paused on a person
    case 'stuck':
    case 'failed': return 'var(--status-danger)';
    case 'shell': return 'var(--text3)';          // outside the vocabulary
    default: return 'var(--status-sleep)'; // idle
  }
}

/**
 * Which states wear the RUNNING cue — app.css `.live-dot`: full-strength fill,
 * an accent halo, and the presence breathe. ONE definition so the sidebar
 * chip, the roster card and the recipient picker cannot disagree about what
 * "in motion" looks like (the colour alone was not enough at 5–7px; see the
 * .live-dot comment for the measurements). `working` is the pre-2026-08 name
 * for the same thing, exactly as in stateDotColor above.
 */
export function stateIsLive(state: string): boolean {
  return state === 'running' || state === 'working';
}

/**
 * The colour of the context-usage bar, as a THEME EXPRESSION rather than a
 * colour: every stop is one of the app's four status tokens, so the ramp is
 * correct in both themes and stays correct if the palette changes.
 *
 * The two anchors are kiro's own: its status line paints context green until 20%
 * and treats 60% as the warning threshold. Past that we continue into our `hot`
 * and `danger` tokens, because a context above 85% is about to force a compact —
 * which is a thing the user should see coming rather than discover.
 */
export function ctxColor(pct: number): string {
  const n = Math.max(0, Math.min(100, Number.isFinite(pct) ? pct : 0));
  const ramp = (from: string, to: string, t: number) =>
    `color-mix(in srgb, var(--status-${to}) ${Math.round(t * 100)}%, var(--status-${from}))`;
  if (n <= 20) return 'var(--status-ok)';
  if (n <= 60) return ramp('ok', 'warn', (n - 20) / 40);
  if (n <= 85) return ramp('warn', 'hot', (n - 60) / 25);
  return ramp('hot', 'danger', (n - 85) / 15);
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

/** Merge activity events without losing history (board #9): id-keyed dedupe
 * (the durable log's row id; pre-paging rows fall back to a content key),
 * sorted by (ts, id) so a prepended older page and an appended poll both land
 * in log order. Returns the EXISTING array when nothing new arrived, so the
 * poll path can skip a re-render. No length cap: the initial page is bounded
 * by the server, and everything past it was loaded on purpose. */
export function mergeEvents<T extends { ts: number; id?: number; window?: number; kind?: string; text?: string }>(
  existing: T[],
  incoming: T[],
): T[] {
  const key = (e: T) => (e.id != null ? `#${e.id}` : `${e.ts}|${e.window}|${e.kind}|${e.text}`);
  const seen = new Set(existing.map(key));
  const fresh = incoming.filter((e) => !seen.has(key(e)));
  if (!fresh.length) return existing;
  return [...existing, ...fresh].sort((a, b) => (a.ts - b.ts) || ((a.id ?? 0) - (b.id ?? 0)));
}

/** Backend identity colors (the prototype's palette): consistent across
 * sidebar, cards and registry so a glance identifies who is who. */
export function backendColor(backend: string | null | undefined): string {
  switch (backend) {
    case 'kiro': return '#a78bfa';
    case 'claude': return '#fb923c';
    case 'codex': return '#94a3b8';
    case 'kimi': return '#4ade80';
    case 'grok': return '#e2e8f0';
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
  if (!body || !to) return body;
  // The chip ALWAYS addresses the recipient (owner, 2026-08-26: "前边的永远
  // 保留，后边的at如果存在某人的话也会发送过去") — it used to stand down when
  // the body contained ANY `@`, so an email address or a quoted @all defeated
  // the chosen recipient silently. Extra @mentions in the body still deliver:
  // the server scans the WHOLE body for mentions, not just the head. Only a
  // body already LEADING with this same recipient skips the prefix (no
  // `@dev @dev`).
  const esc = to.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
  if (new RegExp(`^@${esc}(?:[\\s,:;.!?]|$)`, 'u').test(body)) return body;
  return `@${to} ${body}`;
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

/** How long ago, at sidebar precision: "now", "5m", "2h", "3d". One unit only —
 * the row is a summary, and `fmtElapsed`'s two-unit form ("2h14m") is running-
 * timer language, not last-reply language (owner, 2026-08-24: the project list
 * should show "上次回复的时间"). Both arguments in ms; 0/absent → ''. */
export function agoShort(ts: number, now: number): string {
  if (!ts) return '';
  const s = Math.max(0, Math.floor((now - ts) / 1000));
  if (s < 60) return 'now';
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m`;
  const h = Math.floor(m / 60);
  if (h < 24) return `${h}h`;
  return `${Math.floor(h / 24)}d`;
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
/** A bare image reference in prose: an http(s) URL or an absolute/`~` path
 * ending in an image extension. Agents rarely write markdown image syntax —
 * they answer "截图在 /tmp/shot.png" (owner, 2026-08-26: "如果有图片的返回
 * 可以返回图片路径或者 url，然后你正确读图渲染出来"). */
const BARE_IMG = String.raw`(?:https?:\/\/[^\s<>()"']+?|(?<=^|[\s"'【（(:：=])~?\/[^\s<>()"':]+?)\.(?:png|jpe?g|webp|gif|bmp|svg)(?:\?[^\s<>()"']*)?`;

export function splitImages(body: string | null | undefined): { text: string; images: string[] } {
  const images: string[] = [];
  let text = (body ?? '')
    .replace(/!\[[^\]]*\]\(\s*([^)\s]+)[^)]*\)/g, (_m, src: string) => {
      images.push(src);
      return '';
    })
    // A bare ref ALONE on its line folds into the image strip; one inside a
    // sentence keeps the prose readable and STILL renders below.
    .replace(new RegExp(`^[ \\t]*(${BARE_IMG})[ \\t]*$`, 'gim'), (_m, src: string) => {
      images.push(src);
      return '';
    });
  for (const m of text.matchAll(new RegExp(BARE_IMG, 'gi'))) images.push(m[0]);
  const seen = new Set<string>();
  const deduped = images.filter((s) => !seen.has(s) && (seen.add(s), true));
  text = text.replace(/\n{3,}/g, '\n\n').trim();
  return { text, images: deduped };
}

/** Where a composer attachment lands: a flat temp space under the project's
 * own `.tmm` (owner, 2026-08-26: "上传到项目下…创建临时目录，随机图片 id，
 * 并且转 webp"). The extension is the encoder's verdict — webp where the
 * webview can (Chromium), jpeg where it cannot (WebKit). */
export function uploadImagePath(ws: string, id: string, ext: string): string {
  return `${ws.replace(/\/+$/, '')}/.tmm/uploads/${id}.${ext}`;
}

/** A non-image attachment keeps its own name (an agent told about
 * `report.pdf` should see `report.pdf`), uniqued by the same id and stripped
 * of separators and whitespace — a space would break the bare-ref grammar
 * and read ambiguously when typed into a pane. */
export function uploadFilePath(ws: string, id: string, name: string): string {
  const safe = (name.split(/[\\/]/).pop() || '').trim()
    .replace(/\s+/g, '_')
    .replace(/[<>:"'|?*\x00-\x1f]/g, '')
    .slice(-80) || 'file';
  return `${ws.replace(/\/+$/, '')}/.tmm/uploads/${id}-${safe}`;
}

/** Collision-safe enough for a temp dir, sortable by time, no ceremony. */
export function imageId(): string {
  const rand = (globalThis.crypto?.randomUUID?.() ?? Math.random().toString(36).slice(2)).slice(0, 8);
  return `${Date.now().toString(36)}-${rand}`;
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

/** How many tool rows a group shows before its body starts scrolling — the
 * DEFAULT for the `hubPrefs.stepsRows` setting (owner, 2026-08-24: "最大显示的
 * 行数应该也变成一个可配置的参数。现在默认把这个参数配置成 5 行"). Five rows
 * show what the agent is doing now and where it has just been; the group's
 * OUTER height stops growing there, which is what keeps a live run from
 * shoving the conversation around. "Expand all" lifts the cap for one group. */
export const STEPS_ROWS = 5;

/** The setting's guard: any stored/typed value becomes a usable cap. Not a
 * free number — 1–2 rows cannot show a run's shape, and past ~30 the cap no
 * longer caps anything a screen can hold. Non-numbers fall to the default. */
export function clampStepsRows(v: unknown): number {
  const n = Math.round(Number(v));
  if (!Number.isFinite(n)) return STEPS_ROWS;
  return Math.min(30, Math.max(3, n));
}

/** The marker that stands in for what a folded bubble is not showing. Full-width
 * ellipsis: this is a Chinese-first UI and `...` reads as three periods. */
export const ELIDE = '……';

/**
 * Shorten `text` from the TAIL — a plain rear truncation, with the marker
 * glued INLINE to the last kept content, never on a line of its own.
 *
 * Every long USER message renders folded by default (owner, 2026-08-27:
 * "用户如果发送过长消息 直接后截断的形式 最后文末不用换行三个点 中间不要了，
 * 默认用户消息都截断 不要显示太多"), which retired the middle elision the held
 * anchor used (head + …… + tail): the fold is no longer a landmark's reminder
 * of both ends of an ask but the feed's default economy, and a default wants
 * the simplest shape — the start of the message, cut where the budget runs out.
 *
 * Two shapes need cutting:
 *  · MANY LINES → keep the first whole lines (markdown stays parseable),
 *    marker appended to the last one.
 *  · ONE LONG PARAGRAPH → line counting cannot help, so cut characters, on a
 *    word boundary rather than mid-word (CJK has no spaces; a run without one
 *    is cut where asked).
 *
 * `maxLines` is derived by the caller from the height it may occupy and the
 * measured line height, so a big screen shows more lines than a small one.
 * Returns the text unchanged when it already fits — the common case, and the
 * caller relies on identity to skip re-rendering.
 */
/** A folded user message's line budget on a COMPACT screen: a constant.
 * On the phone the old screen-derived budget (20% of the chat column) gave
 * 6–8 lines — too tall for a screen that fits three bubbles (board #4) — and
 * it MOVED: the on-screen keyboard resizes the viewport, so opening it
 * re-cut every folded message mid-read. One line above the 3-line floor:
 * the floor is where "a shorter fold no longer says anything", and a fold
 * must stay distinguishable from it while letting several messages share a
 * phone screen. Constant on purpose — no basis term means neither the
 * keyboard nor the composer can change it. */
export const PHONE_FOLD_LINES = 4;

/** How many whole lines a folded user message may show — the ONE mapping
 * from (compact, chat-column height, bubble line-height) to a budget
 * (board #4, testable without a DOM):
 * - compact: `PHONE_FOLD_LINES`, flat (see above);
 * - desktop: a fifth of the column (the owner's number), floored at 96px so
 *   a tiny window still folds meaningfully, minus the bubble's own padding
 *   and meta line (26px), in whole line boxes — never below the 3-line
 *   floor. The basis is the chat COLUMN, which holds still while the
 *   composer grows; a real window resize re-enters through the reading
 *   anchor. */
export function foldLines(compact: boolean, basis: number, lineHeight: number): number {
  if (compact) return PHONE_FOLD_LINES;
  const maxPx = Math.max(96, Math.round(basis * 0.2));
  const lh = Number.isFinite(lineHeight) && lineHeight > 6 ? lineHeight : 20;
  return Math.max(3, Math.floor((maxPx - 26) / lh));
}

export function elideTail(text: string, maxLines: number, perLine = 80): string {
  const lines = (text ?? '').split('\n');
  const budget = Math.max(1, Math.floor(maxLines));
  if (lines.length > budget) {
    const out = lines.slice(0, budget).join('\n').trimEnd();
    return closeFences(`${out}${ELIDE}`);
  }
  // Short enough in lines, but a single paragraph can still be pages long once
  // it wraps.
  const cap = budget * perLine;
  if (text.length <= cap) return text;
  return closeFences(`${cutAt(text, cap)}${ELIDE}`);
}

/** Cut `s` down to its first `n` characters, backing up to the nearest space
 * so a word is not sliced in half. */
function cutAt(s: string, n: number): string {
  const slice = s.slice(0, n);
  const at = slice.lastIndexOf(' ');
  return (at > n * 0.7 ? slice.slice(0, at) : slice).trimEnd();
}

/** An elision can drop the closing half of a fenced block, which would swallow
 * the rest of the bubble in a code block. Balance it. */
function closeFences(s: string): string {
  const fences = (s.match(/^```/gmu) ?? []).length;
  return fences % 2 === 0 ? s : `${s}\n\`\`\``;
}

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

/**
 * A composer line that is a SLASH COMMAND for the agent's CLI rather than a
 * message for its model — `/model`, `/clear`, `/compact`, `/tools`.
 *
 * Those are interpreted by the TUI and only when they are the whole line, so
 * they cannot go through the normal delivery, which prefixes
 * `[tmm chat <time>] human:` — the model would just read them as prose (owner,
 * 2026-08-19). Returns the target (an explicit leading `@name`, else '' for
 * "whoever the composer is addressing") and the command text, or null when this
 * is an ordinary message.
 *
 * A PATH is not a command: the first token must be `/word` with no second
 * slash, so `/tmp/foo` and `/usr/bin/env x` stay messages while `/model
 * claude-opus-5` does not.
 */
export function slashCommand(text: string): { to: string; command: string } | null {
  const body = (text ?? '').trim();
  const addressed = /^@([\w][\w.-]*)\s+([\s\S]+)$/u.exec(body);
  const to = addressed ? addressed[1]! : '';
  const rest = (addressed ? addressed[2]! : body).trim();
  if (!/^\/[A-Za-z][\w-]*(\s|$)/u.test(rest)) return null;
  const first = rest.split(/\s/u)[0] ?? '';
  if (first.slice(1).includes('/')) return null;    // a path, not a command
  return { to, command: rest };
}

/** One slash command of an agent's CLI, for the composer's completion palette.
 *
 * The table is TRANSCRIBED from kiro-cli's own TUI (its command palette carries
 * the same names, descriptions and subcommands), not invented here: a made-up
 * command is worse than no completion, because it looks authoritative in the
 * list and then does nothing in the pane. Cloud-only and hidden entries are
 * left out; `/quit` is last because it ends the agent.
 */
export interface SlashCmd {
  name: string;
  desc: string;
  /** Fixed sub-commands, when the CLI defines them. */
  args?: string[];
  /** Values that have to be fetched — the model ids come from `models_list`. */
  dynamic?: 'models';
  /** The command opens an interactive VIEW in the agent's TUI rather than doing
   * something: kiro marks these `inputType: "panel"` (a list/table that takes
   * over the pane and needs a key to dismiss) or opens $EDITOR / a recorder.
   * Sending one from the chat parks the agent inside a panel nobody here can
   * see, so they are kept in this table — with the reason — and filtered OUT of
   * the palette until there is a way to show them ("有一些命令输入后是交互式查看
   * 的 这种就先去掉吧 以后再想办法支持", owner 2026-08-19). Re-enabling one is
   * deleting a flag. */
  view?: true;
}

export const KIRO_COMMANDS: readonly SlashCmd[] = [
  // ── Act immediately, or act once an argument is chosen.
  { name: 'model', desc: 'List or switch models', args: ['set-current-as-default'], dynamic: 'models' },
  { name: 'compact', desc: 'Compact conversation history to reduce context usage' },
  { name: 'clear', desc: 'Clear the conversation and start a fresh session' },
  { name: 'effort', desc: 'Set the reasoning effort level', args: ['low', 'medium', 'high', 'xhigh', 'max', 'set-current-as-default'] },
  // `create`/`edit` open $EDITOR in the pane, so only `swap` is offered.
  { name: 'agent', desc: 'Switch to a different agent', args: ['swap'] },
  { name: 'chat', desc: 'Save the conversation, load one, or start fresh', args: ['new', 'save', 'load'] },
  { name: 'spec', desc: 'List specs, switch to spec mode, or run spec tasks', args: ['new', 'run', 'view', 'analyze_requirements'] },
  { name: 'plan', desc: 'Switch to plan mode to break ideas into a plan' },
  { name: 'paste', desc: 'Paste image from clipboard' },
  { name: 'quit', desc: 'Exit the agent CLI' },
  // ── Interactive views: kept for the record, filtered out of the palette.
  { name: 'tools', desc: 'List available tools', view: true },
  { name: 'help', desc: 'Show available commands', view: true },
  { name: 'usage', desc: 'Show plan usage and billing information', view: true },
  { name: 'context', desc: 'Show or manage context files', args: ['add', 'remove', 'clear'], view: true },
  { name: 'mcp', desc: 'Show MCP server status', view: true },
  { name: 'hooks', desc: 'View configured hooks', view: true },
  { name: 'code', desc: 'Code intelligence status, init, codebase overview', args: ['status', 'init', 'overview'], view: true },
  { name: 'knowledge', desc: 'Manage knowledge bases', view: true },
  { name: 'memories', desc: 'Manage repo-scoped memories from previous sessions', view: true },
  { name: 'tangent', desc: 'Go back, switch to, or create a conversation tangent', args: ['ls', 'root'], view: true },
  { name: 'rewind', desc: 'Fork the session at an earlier turn', view: true },
  { name: 'goal', desc: 'Work toward a goal in a loop until done', view: true },
  { name: 'workflow', desc: 'Browse and manage workflows or run a recipe', args: ['run', 'list', 'new', 'retry'], view: true },
  { name: 'prompts', desc: 'Select or list available prompts', view: true },
  { name: 'feedback', desc: 'Submit feedback, request features, or report issues', view: true },
  { name: 'upgrade-agent', desc: 'Upgrade V2 agent configs to universal form', view: true },
  { name: 'reply', desc: 'Reply to the last assistant message in $EDITOR', view: true },
  { name: 'voice', desc: 'Record voice input', view: true },
];

/** The commands the palette offers: everything that is not an interactive view. */
export const OFFERED_COMMANDS: readonly SlashCmd[] = KIRO_COMMANDS.filter((c) => !c.view);

/** grok 1.0.5 — transcribed from its own docs (`~/.grok/docs/user-guide/
 * 04-slash-commands.md`), same contract as KIRO_COMMANDS: a made-up command
 * looks authoritative in the list and then does nothing in the pane. `view`
 * entries open a modal/picker/pane nobody here can see or dismiss and are
 * filtered from the palette; they stay in the table WITH the reason. */
export const GROK_COMMANDS: readonly SlashCmd[] = [
  { name: 'model', desc: 'Switch models (name, then optional effort)', dynamic: 'models' },
  { name: 'effort', desc: 'Set reasoning effort on the current model', args: ['low', 'medium', 'high', 'xhigh'] },
  { name: 'compact', desc: 'Compress history to reclaim context; a note says what to keep' },
  { name: 'new', desc: 'Start a fresh session (alias /clear)' },
  { name: 'plan', desc: 'Enter plan mode, optionally with a description' },
  { name: 'fork', desc: 'Branch the session into a new agent, keeping history' },
  { name: 'remember', desc: 'Save a note to memory immediately' },
  { name: 'btw', desc: 'Send an aside without interrupting the current task' },
  { name: 'goal', desc: 'Set or manage an autonomous goal', args: ['status', 'pause', 'resume', 'clear'] },
  { name: 'deep-research', desc: 'Kick off a background research workflow' },
  { name: 'workflow', desc: 'Launch or manage a saved workflow', args: ['pause', 'resume', 'stop', 'save'] },
  { name: 'loop', desc: 'Run a prompt on a recurring interval' },
  { name: 'imagine', desc: 'Generate an image from a description' },
  { name: 'always-approve', desc: 'Toggle skip-all-permission-prompts mode' },
  { name: 'auto', desc: 'Toggle classifier-approved permission mode' },
  { name: 'quit', desc: 'Quit the application (alias /exit)' },
  // ── Interactive views / pickers / modals: kept for the record, filtered.
  { name: 'resume', desc: 'Open the session picker', view: true },
  { name: 'dashboard', desc: 'Open the live agent dashboard', view: true },
  { name: 'context', desc: 'Show the context-window breakdown', view: true },
  { name: 'session-info', desc: 'Show session details (alias /status)', view: true },
  { name: 'rewind', desc: 'Roll back to an earlier turn (alias /undo)', view: true },
  { name: 'memory', desc: 'Browse and manage saved memories', view: true },
  { name: 'hooks', desc: 'Extensions modal, Hooks tab', view: true },
  { name: 'plugins', desc: 'Extensions modal, Plugins tab', view: true },
  { name: 'skills', desc: 'Extensions modal, Skills tab', view: true },
  { name: 'mcps', desc: 'MCP servers management modal', view: true },
  { name: 'theme', desc: 'Switch the color theme (picker)', view: true },
  { name: 'feedback', desc: 'Bare form opens a report pane', view: true },
  { name: 'settings', desc: 'Configuration UI', view: true },
  { name: 'usage', desc: 'Account usage view', view: true },
  { name: 'delete', desc: 'Deletes the session — destructive, never offered', view: true },
];

/** codex-cli 0.148.0 — transcribed live from its own `/` popup (2026-08-22).
 * `/model` and friends are PICKERS in codex (they park the TUI at a selection
 * UI), so unlike kiro's they are views here. */
export const CODEX_COMMANDS: readonly SlashCmd[] = [
  { name: 'new', desc: 'Start a new chat during a conversation' },
  { name: 'clear', desc: 'Clear the terminal and start a new chat' },
  { name: 'compact', desc: 'Summarize conversation to prevent hitting the context limit' },
  { name: 'init', desc: 'Create an AGENTS.md file with instructions for Codex' },
  { name: 'plan', desc: 'Switch to Plan mode' },
  { name: 'goal', desc: 'Set or view the goal for a long-running task', args: ['edit', 'pause', 'resume', 'clear'] },
  { name: 'fork', desc: 'Fork the current chat' },
  { name: 'diff', desc: 'Show git diff (including untracked files)' },
  { name: 'status', desc: 'Show current session configuration and token usage' },
  { name: 'mcp', desc: 'List configured MCP tools' },
  { name: 'ps', desc: 'List background terminals' },
  { name: 'stop', desc: 'Stop all background terminals' },
  { name: 'approve', desc: 'Approve one retry of a recent auto-review denial' },
  { name: 'archive', desc: 'Archive this session and exit' },
  { name: 'exit', desc: 'Exit Codex' },
  // ── Pickers and views: kept for the record, filtered from the palette.
  { name: 'model', desc: 'Model picker — a view in codex, not an inline arg', view: true },
  { name: 'permissions', desc: 'Approval-mode picker', view: true },
  { name: 'review', desc: 'Review-preset picker', view: true },
  { name: 'resume', desc: 'Saved-chat picker', view: true },
  { name: 'agent', desc: 'Thread switcher', view: true },
  { name: 'mention', desc: 'File picker', view: true },
  { name: 'skills', desc: 'Skills toggle view', view: true },
  { name: 'memories', desc: 'Memory settings view', view: true },
  { name: 'hooks', desc: 'Lifecycle hooks view', view: true },
  { name: 'keymap', desc: 'Shortcut remapping view', view: true },
  { name: 'feedback', desc: 'Send logs to maintainers (report view)', view: true },
  { name: 'personality', desc: 'Communication-style picker', view: true },
  { name: 'delete', desc: 'Permanently deletes the session — destructive, never offered', view: true },
];

/** The palette's table for a recipient's backend. kiro is the default dialect
 * (and the empty string is "backend unknown", which historically meant kiro).
 * claude returns NOTHING on purpose: the CLI is not installed on this machine,
 * so its command table cannot be transcribed, and offering kiro's table to a
 * claude agent shows commands that do not exist there (owner, 2026-08-22 对齐).
 */
export function offeredCommands(backend?: string | null): readonly SlashCmd[] {
  switch (backend ?? '') {
    case 'grok': return GROK_COMMANDS.filter((c) => !c.view);
    case 'codex': return CODEX_COMMANDS.filter((c) => !c.view);
    // kiro is the default dialect; the empty string is "backend unknown",
    // which historically meant kiro and keeps the old behavior.
    case 'kiro': case '': return OFFERED_COMMANDS;
    // claude (not installed here — table untranscribable), a mixed @all
    // roster, kimi, anything else: no palette beats a wrong one.
    default: return [];
  }
}

export interface PaletteItem { value: string; hint: string }
export interface Palette {
  /** 'command' completes `/mo` → `/model`; 'arg' completes what follows it. */
  stage: 'command' | 'arg';
  items: PaletteItem[];
  /** Replace `text.slice(from)` with the chosen value. */
  from: number;
  /** True when accepting an item should keep the palette open for its argument. */
  more: boolean;
}

/**
 * How well a candidate matches what was typed: `0` prefix, `1` substring,
 * `2` subsequence, `-1` no match. The palette must not demand the first
 * characters ("不一定从第一个字符开始匹配，可以模糊匹配", owner 2026-08-24) —
 * the discriminating part of a model id like `claude-sonnet-4.5` never comes
 * first — but a looser match must never outrank a tighter one: Enter accepts
 * the TOP item, so `/co` has to keep meaning `/compact`, not whatever else
 * happens to contain a c and an o. Case-insensitive; the empty query matches
 * everything as a prefix.
 */
export function fuzzyRank(query: string, candidate: string): number {
  const q = (query ?? '').toLowerCase();
  if (!q) return 0;
  const c = (candidate ?? '').toLowerCase();
  if (c.startsWith(q)) return 0;
  if (c.includes(q)) return 1;
  let i = 0;
  for (const ch of c) {
    if (ch === q[i] && ++i === q.length) return 2;
  }
  return -1;
}

/** Filter + order by fuzzyRank, keeping the table's own order within a tier
 * (sort is stable): the table order is the CLI's own, and it is what makes the
 * list predictable when several candidates tie. */
function fuzzyPick<T>(query: string, all: readonly T[], key: (t: T) => string): T[] {
  return all
    .map((t) => ({ t, r: fuzzyRank(query, key(t)) }))
    .filter((x) => x.r >= 0)
    .sort((a, b) => a.r - b.r)
    .map((x) => x.t);
}

/**
 * What to offer for the composer's current text — the two-stage completion the
 * owner asked for: "比如我打/ 就会出现compact之类的让我选，还有model 如果支持两个
 * 参数的，可以多次选择".
 *
 * Only a line that IS a slash command gets a palette (an optional leading
 * `@name ` is allowed, since that is how you aim one), and only its LAST token is
 * completed. Returns null when there is nothing to offer, which is also how the
 * caller knows to stay out of the way.
 */
export function commandPalette(text: string, models: readonly string[] = [], backend?: string | null): Palette | null {
  const table = offeredCommands(backend);
  if (!table.length) return null;
  const at = /^(\s*@[\w][\w.-]*\s+)?/u.exec(text ?? '')?.[0]?.length ?? 0;
  const line = (text ?? '').slice(at);
  if (!line.startsWith('/')) return null;
  const parts = line.split(/(\s+)/u);          // keeps the separators
  const head = parts[0]!.slice(1);              // the command, without the slash
  // Still typing the command itself: `/`, `/mo`, `/mdl` with no space yet.
  if (parts.length === 1) {
    const items = fuzzyPick(head, table, (c) => c.name).map((c) => ({
      value: `/${c.name}`,
      hint: c.desc,
    }));
    return items.length ? { stage: 'command', items, from: at, more: true } : null;
  }
  const cmd = table.find((c) => c.name === head.toLowerCase());
  if (!cmd) return null;
  const values = [...(cmd.dynamic === 'models' ? models : []), ...(cmd.args ?? [])];
  if (!values.length) return null;
  // Completing the argument: everything after the last whitespace run.
  const lastGap = line.search(/\s+\S*$/u);
  const typed = lastGap >= 0 ? line.slice(lastGap).trimStart() : '';
  const from = at + line.length - typed.length;
  // Only the FIRST argument is completable — these commands take one, and what
  // follows it (a path, a prompt, a free-text name) is not ours to guess. So a
  // filled argument ends the palette instead of re-offering the same list.
  const tokens = line.split(/\s+/u).filter(Boolean);
  if (tokens.length > 2 || (tokens.length === 2 && typed === '')) return null;
  const items = fuzzyPick(typed, values, (v) => v)
    .map((v) => ({ value: v, hint: cmd.dynamic === 'models' && !cmd.args?.includes(v) ? 'model' : 'option' }));
  return items.length ? { stage: 'arg', items, from, more: false } : null;
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

/** The lifecycle verbs the server narrates (`hub_rpc.rs`). A line that does not
 * start with one of them is left whole — an unknown shape must render as its own
 * text, never as a guessed verb plus a truncated remainder. */
const SYS_VERBS = new Set(['spawned', 'started', 'stopped', 'restarted', 'removed', 'interrupted', 'done']);

/**
 * ONE structure for every line the app narrates: WHO it is about, WHAT
 * happened, and the DETAIL — `{ who, verb, text }` (owner, 2026-08-24: "slash
 * 命令，以及 agent 的 remove 等状态更新，都用统一的 ui 来展示…包括 agent 的名字，
 * 状态，或者发送的指令").
 *
 * - `removed k`               → who `k`,   verb `removed`
 * - `spawned dev — fix bug`   → who `dev`, verb `spawned`, text `fix bug`
 * - `done`                    → verb `done` (nobody named)
 * - `/model x → dev, rev`     → who `dev, rev`, verb `/model`, text `x`, cmd
 *
 * The command splits at the LAST ` → ` because that is the one `hub_command`
 * appended (`[tmm] {text} → {targets}`) — the typed text may contain an arrow,
 * the slug-joined target list cannot. An unknown shape keeps only `text`: the
 * whole line renders as itself, because a guessed verb or name would truncate
 * the rest.
 */
export function sysParts(item: string | null | undefined): { verb: string; who: string; text: string; cmd: boolean } {
  const line = (item ?? '').trim();
  if (line.startsWith('/')) {
    const cut = line.lastIndexOf(' → ');
    const typed = cut < 0 ? line : line.slice(0, cut);
    const who = cut < 0 ? '' : line.slice(cut + 3).trim();
    const gap = typed.search(/\s/u);
    return {
      verb: gap < 0 ? typed : typed.slice(0, gap),
      who,
      text: gap < 0 ? '' : typed.slice(gap).trim(),
      cmd: true,
    };
  }
  const gap = line.search(/\s/u);
  const head = (gap < 0 ? line : line.slice(0, gap)).toLowerCase();
  if (!SYS_VERBS.has(head)) return { verb: '', who: '', text: line, cmd: false };
  const rest = gap < 0 ? '' : line.slice(gap).trim();
  // The subject is one token (spawned window names are registry slugs); what
  // follows the em-dash is the spawn brief.
  const wgap = rest.search(/\s/u);
  const who = wgap < 0 ? rest : rest.slice(0, wgap);
  const text = wgap < 0 ? '' : rest.slice(wgap).replace(/^\s*—\s*/u, '').trim();
  return { verb: head, who, text, cmd: false };
}

/**
 * The colour of a lifecycle verb's badge — the SAME progressive language as
 * `noteStateColor` / `stateDotColor`, so one vocabulary covers every coloured
 * state word in the Hub: accent = something started moving, green = ended well,
 * grey = at rest, red = destructive (the token the Close/Remove buttons already
 * wear). An unknown verb takes ordinary reading ink rather than a colour we
 * would be inventing a meaning for.
 */
export function sysVerbColor(verb: string): string {
  switch (verb) {
    case 'spawned':
    case 'started':
    case 'restarted': return 'var(--accent)';
    case 'done': return 'var(--status-ok)';
    case 'interrupted': return 'var(--status-warn)';
    case 'removed': return 'var(--status-danger)';
    case 'stopped': return 'var(--text3)';
    default: return 'var(--text2)';
  }
}

/**
 * A note posted by an agent about its own work: `[tmm status working] compiling
 * the server`, or `[tmm done] shipped the palette`.
 *
 * `tmm status` used to be a telemetry event, which meant it vanished on restart
 * while the messages around it survived. It is now a MESSAGE from the agent
 * ("status要用agent发送消息的形式显示", 2026-08-19), so it is persisted like any
 * other and it reads as the agent speaking — because it is. This is the reader:
 * the marker comes off, the declared state comes out, and the note is rendered as
 * an ordinary bubble that happens to be dimmer.
 *
 * The marker is NOT `[tmm] ` on purpose: that prefix means "the app is narrating",
 * folds into a grey sys row, and is dropped entirely at the chat-only level — so
 * the text disappeared exactly where a reader looks. That is the treatment both of
 * these were moved OUT of (owner, 2026-08-19, twice: "status要用agent发送消息的形
 * 式显示" and "返回的状态信息要用消息的形式展示在对话里").
 *
 * A `done` with no summary never becomes a message: there is nothing to read.
 */
export function statusNote(body: string | null | undefined): { state: string; text: string } | null {
  const m = /^\[tmm (?:status ([a-z]+)|(done))\]\s*([\s\S]*)$/u.exec(body ?? '');
  if (!m) return null;
  const text = m[3]!.trim();
  return text ? { state: m[1] ?? m[2]!, text } : null;
}

/**
 * The colour of a status-note's declared state, worn by the badge in the
 * bubble header. SAME language as `stateDotColor` (see the table there):
 * accent = in motion, green = done well, amber = needs a person, red =
 * failed, grey = a word we do not know (quiet, never wrong). The only case
 * this reader adds is `done`, which is a note-only state — a roster dot never
 * shows it because a finished agent goes back to idle.
 */
export function noteStateColor(state: string): string {
  switch (state) {
    case 'working':
    case 'running': return 'var(--accent)';
    case 'waiting':
    case 'blocked': return 'var(--status-warn)';
    case 'failed':
    case 'stuck': return 'var(--status-danger)';
    case 'done': return 'var(--status-ok)';
    default: return 'var(--text3)';
  }
}

/**
 * Same calendar day in LOCAL time — the rule behind the feed's date
 * separators. UTC arithmetic would flip the divider at 08:00 for a UTC+8
 * reader, which is exactly the reader who asked for it ("我希望在有新日期的
 * 地方能有一个标注", owner 2026-08-20).
 */
export function sameDay(a: number, b: number): boolean {
  const da = new Date(a);
  const db = new Date(b);
  return da.getFullYear() === db.getFullYear()
    && da.getMonth() === db.getMonth()
    && da.getDate() === db.getDate();
}

/** A draft is a convenience, not a document: capped so a pasted file cannot fill
 * localStorage and take the rest of the Hub's prefs down with it. */
export const DRAFT_MAX = 8000;

/**
 * The next draft map after typing in a project's composer. Pure so the two rules
 * that would regress silently are testable: an EMPTY draft removes its key (else
 * every project ever visited leaves a row behind), and the text is capped.
 *
 * Returns the SAME object when nothing changes, which is the caller's signal to
 * skip the write — this runs on every keystroke.
 */
export function draftUpdate(
  map: Record<string, string>,
  session: string,
  text: string,
): Record<string, string> {
  if (!session) return map;
  const keep = (text ?? '').slice(0, DRAFT_MAX);
  if (keep === (map[session] ?? '')) {
    // A stored empty string is junk — we never write one — so clean it up if an
    // older build or a hand-edited value left one behind. Everything else is a
    // genuine no-op, which is the common case on a keystroke.
    if (map[session] === '') {
      const cleaned = { ...map };
      delete cleaned[session];
      return cleaned;
    }
    return map;
  }
  const next = { ...map };
  if (keep) next[session] = keep;
  else delete next[session];
  return next;
}

/** One row of the conversation. `msg` and `prompt` are things that were said,
 * `note` is a single observed fact, `steps` is a collapsible run of tool calls
 * (the "what it did between two replies" pane). */
export type FeedBlock =
  | { type: 'msg'; ts: number; msg: any; delivered: boolean }
  | { type: 'sys'; ts: number; key: string; items: string[] }
  | { type: 'prompt'; ts: number; window: number; text: string }
  | { type: 'progress'; ts: number; window: number; state: string; text: string }
  | { type: 'note'; ts: number; window: number; event: HubActivityEvent }
  | { type: 'steps'; ts: number; window: number; key: string; events: HubActivityEvent[] };

/** The two halves of a `tmm status` event: what the agent DECLARED and the note
 * it wrote. Older servers glued them into one string (`"working — 重写状态机"`)
 * and sent no `state`; rooms and rings outlive a build, so both shapes parse. */
export function statusParts(e: HubActivityEvent): { state: string; text: string } {
  if (e.state) return { state: e.state, text: e.text === e.state ? '' : e.text };
  const m = /^(working|waiting|blocked)(?:\s+—\s+([\s\S]*))?$/.exec(e.text.trim());
  return m ? { state: m[1]!, text: m[2] ?? '' } : { state: '', text: e.text };
}

/** Internal: a tool call before consecutive ones are folded into a group. */
type ToolItem = { type: 'tool'; ts: number; window: number; event: HubActivityEvent };

/** ONE truth per agent dot (board #8): the sidebar's project rows colour
 * their chips from the `hub_rooms` snapshot (all projects, 20s poll) while
 * the roster reads `hub_agents` (selected project, 5s poll) — same server
 * derivation, different ages, so the two UIs disagreed for up to a poll
 * cycle. This overlays the SELECTED project's roster states onto the
 * snapshot map, so whichever response lands LAST, the freshest source wins
 * for the project on screen and the cheap snapshot keeps speaking for the
 * rest:
 * - called with the roster after a `hub_agents` poll, it refreshes that
 *   project's keys in place;
 * - called with the roster over a fresh `hub_rooms` response, it stops the
 *   (older-cadence) snapshot from rolling the selected project's dots back;
 * - only MANAGED agents with a non-empty state write keys (a shell window
 *   has no state to assert), and a stopped agent's snapshot key passes
 *   through untouched — absence is the sidebar's own "idle" default, never
 *   invented here. Pure; returns a new map. */
export function mergeStates(
  snapshot: Record<string, string>,
  session: string,
  agents: readonly { window: number; state?: string; managed?: boolean }[],
): Record<string, string> {
  const out: Record<string, string> = { ...snapshot };
  for (const a of agents) {
    if (!a.managed || !a.state) continue;
    out[`${session}:${a.window}`] = a.state;
  }
  return out;
}

/** Does this body ADDRESS the agent — an `@name` (or `@all`) token, parsed
 * exactly the way the server's `deliver_mentions` does (token after `@` up to
 * whitespace, trailing punctuation trimmed, exact name equality)? Used by the
 * roster's double-click filter: "发给它的用户消息" means addressed, and a
 * substring match would collect `@builder-2` for `builder` (board #3). */
export function mentionsAgent(body: string, name: string): boolean {
  return body
    .split('@')
    .slice(1)
    .map((rest) => rest.split(/\s/, 1)[0]?.replace(/[,:;.!?]+$/u, '') ?? '')
    .some((token) => token === name || token === 'all');
}

/** The feed narrowed to ONE agent (board #3: 双击卡片 "筛选跟某个agent有关的
 * 消息"). "Related" is defined here, once, and tested:
 * - a message FROM the agent (its replies, its `[tmm status]`/`[tmm done]`
 *   bubbles), or ANY message ADDRESSED to it (`@name` / `@all` — from the
 *   human or a teammate);
 * - telemetry blocks (prompt / progress / note / steps) of ITS window and no
 *   other — two agents working concurrently must not leak into each other's
 *   filter (`window` undefined keeps none: no window, no telemetry);
 * - sys capsules keep only the lifecycle LINES that name the agent as a whole
 *   word (`spawned builder-2` must not surface for `builder`), and disappear
 *   when none do.
 * Pure so the rules are testable without a DOM. */
export function filterBlocks(blocks: FeedBlock[], name: string, window?: number): FeedBlock[] {
  const word = new RegExp(`(?<![\\w-])${name.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}(?![\\w-])`, 'u');
  const out: FeedBlock[] = [];
  for (const b of blocks) {
    if (b.type === 'msg') {
      const from = b.msg?.from ?? '';
      const body = b.msg?.body ?? '';
      if (from === name || mentionsAgent(body, name)) out.push(b);
      continue;
    }
    if (b.type === 'sys') {
      const items = b.items.filter((line) => word.test(line));
      if (items.length) out.push({ ...b, items });
      continue;
    }
    // prompt / progress / note / steps — all carry the window they belong to.
    if (window != null && b.window === window) out.push(b);
  }
  return out;
}

/** ALL whitespace removed — the delivery-echo match runs on this form (the
 * server's `strip_ws` twin; a test pins the two to the same behavior via the
 * same cases). Squashing to ONE space was not enough: tmux in extended-keys
 * mode dropped the raw \n byte outright, so the echo came back with the lines
 * GLUED and "a b" could never contain "ab" (owner, 2026-08-24). */
export function squashWs(s: string): string {
  return s.replace(/\s+/gu, '');
}

/** Internal: a turn boundary that produces NO row — a delivery echo. The echo
 * is consumed as a receipt (rule 1) so it never renders, but it is still the
 * moment `userPromptSubmit` opened a new turn, and a new turn must not pour its
 * tool calls into the previous turn's group. */
type TurnMark = { type: 'turn'; ts: number; window: number };

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
 * 2b. **A `tmm status` note is a spoken line.** Hooks see that a turn is open,
 *    never what it is about; the note is the only account of the work in
 *    progress, so it renders at every level and never breaks a tool lane (the
 *    agent wrote it in the middle of the run it describes).
 * 3. **Tool calls collapse per AGENT, replies do not.** A window's tool events
 *    fold into one `steps` group that stays open for that window's whole turn.
 *    Only that SAME window ends its own run — its reply, its local prompt, the
 *    echo of a line delivered to it, a note about it. Another agent's rows are a
 *    different lane and must not break it: with two agents working, a rule that
 *    folded only CONSECUTIVE events produced one group per call
 *    (w1, w2, w1, w2 …) and the feed read as churn (owner report, 2026-08-19).
 *    Duplicate consecutive tool lines are dropped per window first (pre+post
 *    fire for one call).
 *
 * `windowOf` maps a message's sender to its window, which is how a reply is
 * attributed to the lane it ends. Without it a reply cannot be attributed, so
 * it conservatively ends every open run — the pre-aggregation behavior.
 */
export function feedBlocks(
  feed: any[],
  activity: readonly HubActivityEvent[],
  level: FeedLevel,
  windowOf?: (from: string) => number | undefined,
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
  const turns: TurnMark[] = [];
  for (const e of activity) {
    if (e.kind !== 'prompt' || e.via !== 'app') continue;
    // EVERY message at or before the echo whose body it contains. The typed line
    // is `[tmm chat] <from>: <body>`, an agent that was mid-typing submits it with
    // its own leftover text attached — and a busy agent can submit SEVERAL queued
    // lines in one prompt, in which case each of them was delivered. Marking only
    // the newest left the earlier ones hollow for ever.
    // Whitespace-canonical, same rule as the server's record_prompt: the line
    // rides tmux send-keys and the TUI's composer before it echoes back, and a
    // newline in the body does not survive that byte-for-byte (owner,
    // 2026-08-22: multi-line messages never confirmed).
    let hit = false;
    const canonEcho = squashWs(e.text);
    for (const m of msgs) {
      if (m.type !== 'msg' || m.delivered) continue;
      const body = squashWs(m.msg?.body ?? '');
      if (body && m.ts <= e.ts && canonEcho.includes(body)) {
        m.delivered = true;
        hit = true;
      }
    }
    if (hit) {
      consumed.add(e);
      // Invisible, but still a turn boundary (rule 3).
      turns.push({ type: 'turn', ts: e.ts, window: e.window });
    }
  }

  const stream: (FeedBlock | ToolItem | TurnMark)[] = [...msgs, ...turns];
  const lastTool = new Map<number, string>();
  for (const e of activity) {
    if (consumed.has(e)) continue;
    // A line that never came back is about the message, not about telemetry:
    // it survives even the chat-only level.
    if (e.kind === 'warn') {
      stream.push({ type: 'note', ts: e.ts, window: e.window, event: e });
      continue;
    }
    // A `tmm status` NOTE is the agent saying what it is doing — the one thing
    // hooks cannot observe. It is deliberate, it is prose, and it is the answer
    // to "what is happening right now", so it renders as a spoken line at every
    // detail level. A note-less claim is dropped: the derived state already says
    // running/idle better than a word the agent typed.
    if (e.kind === 'status') {
      const { state, text } = statusParts(e);
      if (text.trim()) stream.push({ type: 'progress', ts: e.ts, window: e.window, state, text: text.trim() });
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
  // can land in the same millisecond. A turn boundary sorts ahead of everything
  // at its timestamp, because it OPENS what follows.
  const rank = (i: FeedBlock | ToolItem | TurnMark) => (i.type === 'turn' ? -1 : i.type === 'msg' ? 1 : 0);
  stream.sort((a, b) => a.ts - b.ts || rank(a) - rank(b));

  // Rule 3: one group per window per turn — see the doc comment. `open` holds
  // the group each window is still adding to; a row from that window closes it,
  // a row from any other window is a different lane and is ignored.
  const out: FeedBlock[] = [];
  const open = new Map<number, Extract<FeedBlock, { type: 'steps' }>>();
  for (const item of stream) {
    if (item.type === 'tool') {
      const group = open.get(item.window);
      if (group) {
        group.events.push(item.event);
        continue;
      }
      const created: Extract<FeedBlock, { type: 'steps' }> = {
        type: 'steps',
        ts: item.ts,
        window: item.window,
        key: `w${item.window}-${item.ts}`,
        events: [item.event],
      };
      open.set(item.window, created);
      out.push(created);
      continue;
    }
    if (item.type === 'turn') {
      open.delete(item.window);
      continue;                      // a receipt renders as a mark on its message
    }
    if (item.type === 'msg') {
      // Which lane does this reply end? Its sender's, when we can tell; every
      // lane when we cannot (no map = no attribution, so stay conservative).
      const from = item.msg?.from ?? '';
      const w = from ? windowOf?.(from) : undefined;
      if (!windowOf) open.clear();
      else if (w !== undefined) open.delete(w);
      out.push(item);
      continue;
    }
    if (item.type === 'prompt' || item.type === 'note') {
      open.delete(item.window);
      out.push(item);
      continue;
    }
    // A progress note is NOT a boundary, on purpose: the agent wrote it in the
    // middle of the work it describes, so closing its lane there would chop one
    // run into a group per report — the churn this whole rule exists to avoid.
    if (item.type === 'progress') {
      out.push(item);
      continue;
    }
    // Lifecycle lines are the app's record, not a turn boundary: consecutive
    // ones fold into ONE row.
    const prev = out[out.length - 1];
    if (item.type === 'sys' && prev?.type === 'sys') {
      prev.items.push(...item.items);
      continue;
    }
    out.push(item);
  }
  return out;
}

/** One readline edit against the composer's text, as pure data.
 *
 * The composer talks to terminal people: their fingers already know Ctrl-A/E/
 * U/K/W/Y from every shell they have ever typed into, and on macOS the system
 * text views honour half of that set natively — so its ABSENCE here is what
 * reads as broken, and the half macOS lacks (U, W, Y) is the half a Linux
 * browser lacks entirely (owner, 2026-08-20: "支持 ctrl a/e/u/y 等等快捷键，
 * 其中有一些 mac 系统好像就已经支持了 … 适用性更好一些"). Implementing the set
 * OURSELVES, everywhere, is what makes it uniform: the mac keeps the bindings
 * it had, every other platform gains them, and none of them drift apart.
 *
 * Pure — the component owns the textarea and the kill buffer; this owns the
 * arithmetic. Returns null for any key the table does not know, so the caller
 * falls through to the browser (Ctrl-C/V/X/Z are deliberately NOT here).
 *
 * Kill semantics are readline's: U/K/W save what they delete into ONE buffer,
 * consecutive kills accumulate (backward kills PREPEND, forward kills APPEND —
 * so Ctrl-W Ctrl-W Ctrl-Y restores the words in their original order), and any
 * other edit breaks the chain. Ctrl-D deletes without saving, as in readline.
 * A/E move within the LINE, not the whole text — the composer is a textarea
 * and a multi-line draft has lines.
 */
export interface ReadlineReq {
  key: string;      // e.key, lowercased; caller has checked ctrl && !meta && !alt
  text: string;
  start: number;    // selectionStart
  end: number;      // selectionEnd
  kill: string;     // the kill buffer as of this keystroke
  killing: boolean; // the PREVIOUS edit was a kill (accumulation chain)
}
export interface ReadlineEdit {
  text: string;
  caret: number;    // collapsed selection
  kill: string;
  killing: boolean;
}
export function readlineEdit(r: ReadlineReq): ReadlineEdit | null {
  const { text, start, end } = r;
  const lineStart = (i: number) => text.lastIndexOf('\n', i - 1) + 1;
  const lineEnd = (i: number) => { const n = text.indexOf('\n', i); return n === -1 ? text.length : n; };
  const keep = (caret: number): ReadlineEdit => ({ text, caret, kill: r.kill, killing: false });
  const cut = (from: number, to: number, prepend: boolean): ReadlineEdit => {
    const del = text.slice(from, to);
    return {
      text: text.slice(0, from) + text.slice(to),
      caret: from,
      kill: del ? (r.killing ? (prepend ? del + r.kill : r.kill + del) : del) : r.kill,
      killing: true,
    };
  };
  switch (r.key) {
    case 'a': return keep(lineStart(start));
    case 'e': return keep(lineEnd(end));
    case 'b': return keep(start !== end ? start : Math.max(start - 1, 0));
    case 'f': return keep(start !== end ? end : Math.min(end + 1, text.length));
    case 'u': return cut(lineStart(start), start, true);
    case 'k': {
      // At the end of a line, Ctrl-K eats the newline — that is how readline
      // joins lines, and without it the key dies exactly where you reach for it.
      const from = end;
      const to = lineEnd(end) === from && from < text.length ? from + 1 : lineEnd(end);
      return cut(from, to, false);
    }
    case 'w': {
      let p = start;
      while (p > 0 && /\s/u.test(text[p - 1]!)) p--;
      while (p > 0 && !/\s/u.test(text[p - 1]!)) p--;
      return cut(p, start, true);
    }
    case 'y': {
      // Empty buffer still returns handled: the browser's own Ctrl-Y is redo
      // (Chromium), and half-yanking half-redoing would be worse than a no-op.
      if (!r.kill) return keep(start);
      return {
        text: text.slice(0, start) + r.kill + text.slice(end),
        caret: start + r.kill.length,
        kill: r.kill,
        killing: false,
      };
    }
    case 'd': {
      if (start !== end) return { text: text.slice(0, start) + text.slice(end), caret: start, kill: r.kill, killing: false };
      if (start >= text.length) return keep(start);
      return { text: text.slice(0, start) + text.slice(start + 1), caret: start, kill: r.kill, killing: false };
    }
    case 'h': {
      if (start !== end) return { text: text.slice(0, start) + text.slice(end), caret: start, kill: r.kill, killing: false };
      if (start === 0) return keep(0);
      return { text: text.slice(0, start - 1) + text.slice(start), caret: start - 1, kill: r.kill, killing: false };
    }
    case 't': {
      // Drag the char before point over the char at point; at the end of a
      // line, transpose the last two. Readline's caret lands after the pair.
      const atEol = start >= text.length || text[start] === '\n';
      const i = atEol ? start - 2 : start - 1;
      if (i < 0 || text[i] === '\n' || text[i + 1] === '\n' || i + 1 >= text.length) return keep(start);
      return {
        text: text.slice(0, i) + text[i + 1] + text[i] + text.slice(i + 2),
        caret: i + 2,
        kill: r.kill,
        killing: false,
      };
    }
    default: return null;
  }
}
