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

/** How many tool rows a group shows before its body starts scrolling. A run of
 * forty calls is a wall; ten rows is enough to see what the agent is doing now
 * and where it has been, and the group's OUTER height stops growing there —
 * which is what keeps a live run from shoving the conversation around. "Expand
 * all" lifts the cap for one group. */
export const STEPS_ROWS = 10;

/** The marker that stands in for what a held bubble is not showing. Full-width
 * ellipsis: this is a Chinese-first UI and `...` reads as three periods. */
export const ELIDE = '……';

/**
 * Shorten `text` from the MIDDLE, keeping the beginning and the end.
 *
 * A held user message is a reminder of what you asked, and the ask is usually
 * split between the two ends: the subject at the top, the actual request at the
 * bottom. Cutting the tail (what a line clamp does) throws away the half that
 * says what to do — hence "把文字中间内容可以跳过比较多用 ……省略, 但是少数几行可以
 * 完整展示" (owner, 2026-08-19).
 *
 * Two shapes need eliding and they need different cuts:
 *  · MANY LINES → keep whole lines, more from the head than the tail (the first
 *    lines carry the framing), with the marker on a line of its own. Whole lines
 *    keep the markdown parseable, which is why this is not a character cut.
 *  · ONE LONG PARAGRAPH → line counting cannot help, so cut characters, and cut
 *    on a word/CJK boundary rather than mid-word.
 *
 * `maxLines` is derived by the caller from the height it may occupy and the
 * measured line height, so a big screen shows more lines than a small one.
 * Returns the text unchanged when it already fits — the common case, and the
 * caller relies on identity to skip re-rendering.
 */
export function elideMiddle(text: string, maxLines: number, perLine = 80): string {
  const lines = (text ?? '').split('\n');
  const budget = Math.max(2, Math.floor(maxLines));
  if (lines.length > budget) {
    // One line of the budget goes to the marker itself.
    const keep = budget - 1;
    const head = Math.max(1, Math.ceil(keep * 0.6));
    const tail = Math.max(1, keep - head);
    const out = [...lines.slice(0, head), ELIDE, ...lines.slice(-tail)].join('\n');
    return closeFences(out);
  }
  // Short enough in lines, but a single paragraph can still be pages long once
  // it wraps.
  const cap = budget * perLine;
  if (text.length <= cap) return text;
  const head = Math.floor(cap * 0.6);
  const tail = Math.max(20, cap - head);
  return closeFences(`${cutAt(text, head, 'end')}\n${ELIDE}\n${cutAt(text, tail, 'start')}`);
}

/** Cut `n` characters off the start/end of `s`, backing up to the nearest space
 * so a word is not sliced in half. CJK has no spaces, so a run without one is
 * cut where asked. */
function cutAt(s: string, n: number, from: 'start' | 'end'): string {
  if (from === 'end') {
    const slice = s.slice(0, n);
    const at = slice.lastIndexOf(' ');
    return (at > n * 0.7 ? slice.slice(0, at) : slice).trimEnd();
  }
  const slice = s.slice(-n);
  const at = slice.indexOf(' ');
  return (at >= 0 && at < n * 0.3 ? slice.slice(at + 1) : slice).trimStart();
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
 * What to offer for the composer's current text — the two-stage completion the
 * owner asked for: "比如我打/ 就会出现compact之类的让我选，还有model 如果支持两个
 * 参数的，可以多次选择".
 *
 * Only a line that IS a slash command gets a palette (an optional leading
 * `@name ` is allowed, since that is how you aim one), and only its LAST token is
 * completed. Returns null when there is nothing to offer, which is also how the
 * caller knows to stay out of the way.
 */
export function commandPalette(text: string, models: readonly string[] = []): Palette | null {
  const at = /^(\s*@[\w][\w.-]*\s+)?/u.exec(text ?? '')?.[0]?.length ?? 0;
  const line = (text ?? '').slice(at);
  if (!line.startsWith('/')) return null;
  const parts = line.split(/(\s+)/u);          // keeps the separators
  const head = parts[0]!.slice(1);              // the command, without the slash
  // Still typing the command itself: `/`, `/mo`, `/model` with no space yet.
  if (parts.length === 1) {
    const items = OFFERED_COMMANDS.filter((c) => c.name.startsWith(head.toLowerCase())).map((c) => ({
      value: `/${c.name}`,
      hint: c.desc,
    }));
    return items.length ? { stage: 'command', items, from: at, more: true } : null;
  }
  const cmd = OFFERED_COMMANDS.find((c) => c.name === head.toLowerCase());
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
  const items = values
    .filter((v) => v.toLowerCase().startsWith(typed.toLowerCase()))
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

/**
 * A status note posted by an agent: `[tmm status working] compiling the server`.
 *
 * `tmm status` used to be a telemetry event, which meant it vanished on restart
 * while the messages around it survived. It is now a MESSAGE from the agent
 * ("status要用agent发送消息的形式显示", 2026-08-19), so it is persisted like any
 * other and it reads as the agent speaking — because it is. This is the reader:
 * the marker comes off, the declared state comes out, and the note is rendered as
 * an ordinary bubble that happens to be dimmer.
 *
 * The marker is NOT `[tmm] ` on purpose: that prefix means "the app is narrating"
 * and folds into a grey sys row, which is the treatment this note was moved OUT
 * of.
 */
export function statusNote(body: string | null | undefined): { state: string; text: string } | null {
  const m = /^\[tmm status ([a-z]+)\]\s*([\s\S]*)$/u.exec(body ?? '');
  if (!m) return null;
  const text = m[2]!.trim();
  return text ? { state: m[1]!, text } : null;
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
