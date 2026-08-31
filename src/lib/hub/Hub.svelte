<script>
  // Hub — chat-first project workspace (owner-directed IA rework 2026-08-01):
  //
  // · The DEFAULT view of a project is its CONVERSATION, full width. The
  //   terminal is a DRAWER opened by a button on the right — terminal is
  //   terminal, project is project; they are never presented as parallel
  //   equals (that layout implied the two panes were synced views of one
  //   thing, which they are not).
  // · Two kinds of windows, handled apart: MANAGED agents (spawned from the
  //   registry, tmm-wired, isolated home — the server marks them) live in
  //   the chat as cards and DM targets; DIRECT windows (shells, agents the
  //   user started by hand) exist only inside the terminal drawer's window
  //   list. Shells never get chat affordances.
  // · Tapping a managed agent's card selects it as the DM target: the
  //   composer auto-addresses @name, so "talk to THIS agent" is one tap.
  // · Agent DEFINITIONS are configured on their own page (AgentsPage), not
  //   in this sidebar. New projects pick their agents at creation time.
  import Terminal from '../terminal/Terminal.svelte';
  import Files from '../files/Files.svelte';
  import Board from './Board.svelte';
  import SideHandle from '../ui/SideHandle.svelte';
  import { scrollFade } from '../core/scrollFade.ts';
  import ChatImage from './ChatImage.svelte';
  import Lightbox from '../ui/Lightbox.svelte';
  import 'katex/dist/katex.min.css';
  import Icon from '../ui/Icon.svelte';
  import { tick as settled } from 'svelte';
  import { t, i18n } from '../core/i18n.svelte.ts';
  import {
    projectList, projectUp, projectDown, projectDelete, projectArchive, projectCreate, projectRename, listSessionsWithPanes,
    hubPost, hubCommand, modelsList, hubLog, hubRooms, hubAgents, fsMkdir, fsUpload, hubSpawn, hubAgentStop, hubAgentRestart, hubActivity, hubAgentRemove, hubAgentInterrupt, registryList,
    addTeamMessageListener, removeTeamMessageListener,
  } from '../core/ws.ts';
  import { sortRows } from '../projects/projects.ts';
  import { markLeadingMention, stateDotColor, stateIsLive, mergeMessages, mergeEvents, backendColor, feedBlocks, filterBlocks, mergeStates, pickLead, addressed, fmtElapsed, agoShort, unreadSenders, splitImages, stoppedAgents, toolColor, pickAnchor, toolEventParts, elideTail, foldLines, slashCommand, commandPalette, ctxColor, statusNote, noteStateColor, sysParts, sysVerbColor, boardLine, boardStatusColor, promptParts, sameDay, readlineEdit, uploadImagePath, uploadFilePath, imageId, pastedFiles } from './hub.ts';
  import { backendIcon, paneAgent } from '../core/agents.ts';
  import { anchorOf, menuPlacement, viewBox } from '../ui/placement.ts';
  import ContextMenu from '../ui/ContextMenu.svelte';
  import { longpress } from '../ui/longpress.ts';
  import { hubPrefs } from './hub-prefs.svelte.ts';
  import { renderMarkdown } from '../core/markdown.ts';
  import CreateProjectDialog from '../projects/CreateProjectDialog.svelte';
  import ConfirmDialog from '../ui/ConfirmDialog.svelte';

  let { visible = false, fontSize = 14, mobile = false, openTerminal = () => {}, onSelectSession = (_s) => {}, onGoBack = null, openAgentConfig = null, openFilesTab = null, openBoardTab = null } = $props();

  // Layout follows the viewport, not the device class (a squeezed desktop
  // window must not overflow). `mobile` still decides behavior defaults.
  let narrow = $state(typeof window !== 'undefined' && window.matchMedia('(max-width: 760px)').matches);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 760px)');
    const onChange = () => { narrow = mq.matches; };
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  });
  const compact = $derived(mobile || narrow);

  let rows = $state([]);            // ProjectRow[]
  // The recycle bin: archived projects, folded at the sidebar's bottom.
  let trash = $state([]);           // ProjectRow[] (archived)
  let trashOpen = $state(false);
  let trashAsk = $state(null);      // row pending PERMANENT delete (the only irreversible step)
  let panes = $state([]);           // all tmux panes
  let talkMap = $state({});         // room -> newest message ts (ms) — sidebar row times
  let agentStates = $state({});     // "<session>:<window>" -> derived state, all projects
  let selected = $state('');        // selected project session
  let agents = $state([]);          // HubAgent[] for selected session (all windows)
  let feed = $state([]);            // chat messages, oldest first
  let activity = $state([]);        // telemetry events (in-memory ring on the server)
  let lastActivityTs = 0;
  let registry = $state([]);        // RegAgent[]
  // Three ways a message can land, and they are NOT variations of one thing:
  //   a name    → typed into that agent's input; exactly one agent is
  //               interrupted and starts a turn. The default (the lead).
  //   ALL_TARGET→ `@all`: typed into EVERY managed agent's input. Every agent
  //               starts a turn at once, so this is a deliberate act, not a
  //               casual default — it is the expensive one.
  //   ''        → recorded in the room and delivered to NOBODY. Agents read it
  //               when they next call `tmm log`, which is how you leave context
  //               without interrupting anyone mid-task.
  const ALL_TARGET = 'all';
  let composerText = $state('');
  // Who the composer addresses. Defaults to the project's lead (pickLead), so
  // talking to your lead agent needs no @ ceremony.
  let recipient = $state('');
  let recipientOpen = $state(false);
  // Empty-composer interrupt (owner, 2026-08-24): with nothing typed, the grey
  // send button (or Ctrl+C) ARMS — it becomes a "send interrupt" button — and
  // a second activation fires. Two beats on purpose: interrupt cancels a
  // running turn, and a single stray tap on the button every message is sent
  // from should not be able to do that.
  let intArm = $state(false);
  let intTimer = 0;
  let feedEl = $state(null);
  let composerEl = $state(null);
  let toChipW = $state(0);        // measured recipient-chip width → first-line indent

  // Terminal drawer (closed by default — the whole point).
  let termOpen = $state(false);
  // What the drawer SHOWS: the terminal, or the file browser (owner,
  // 2026-08-28: "右侧边栏，可以展开文件浏览器的分区，类似展示 terminal 面板
  // 一样的逻辑"). One drawer, one width handle, two bodies — the hidden one
  // keeps visibility:hidden (never display:none: a re-laid-out terminal
  // would resize the pane and make the agent repaint, the .keep-rows story).
  let drawerView = $state('term');
  let drawerIssueReq = $state(null); // a feed board-line tap on desktop: open the issue in the drawer
  let drawerBoardNew = $state(null); // the drawer head's + (board #23): new issue, requested into the embedded Board
  let drawerFilesDir = $state(''); // where the drawer's Files is — the jump hands it over
  let termTarget = $state('');
  let termCommand = $state('');

  // New-project dialog.
  let createOpen = $state(false);


  const room = (session) => `proj:${session}`;
  let lastTs = 0;

  const selectedRow = $derived(rows.find((r) => r.project.session === selected) ?? null);
  const liveSelected = $derived(!!selectedRow?.live);
  const managedAgents = $derived(agents.filter((a) => a.managed));
  // Declared but not running — a stopped agent still belongs to the room.
  const stopped = $derived(stoppedAgents(selectedRow?.slots, managedAgents));
  const working = $derived(managedAgents.filter((a) => a.state === 'working').length);

  // ── Sidebar row summary ──────────────────────────────────────────────────
  // Each row answers two questions at a glance — "when did this project last
  // speak" (the same map that ORDERS the list, so the time explains the order)
  // and "who is in it, doing what" (owner, 2026-08-24: "上次回复的时间 …
  // 当前几个 Agent 的简单 logo 状态").
  const rowTalk = (row) => talkMap[row.project.room ?? `proj:${row.project.session}`] ?? 0;
  // A LIVE project reads its real windows — the same agent detection the
  // window switcher uses — each coloured by the hook-derived state from
  // hub_rooms (absence = idle: a window with no hook facts is at rest). A
  // CLOSED project shows its DECLARED agent slots dimmed, no state dot: the
  // roster `up` will bring back, not anything running now.
  function rowAgents(row) {
    if (row.live) {
      const out = [];
      const seen = new Set();
      for (const p of panes) {
        if (p.session !== row.project.session || !p.active || seen.has(p.window)) continue;
        seen.add(p.window);
        const agent = paneAgent(p);
        if (!agent) continue;
        out.push({
          icon: agent.icon, name: p.window_name,
          state: agentStates[`${row.project.session}:${p.window}`] ?? 'idle',
        });
      }
      return out.slice(0, 4);
    }
    return (row.slots ?? [])
      .filter((s) => s.kind === 'agent')
      .slice(0, 4)
      .map((s) => ({ icon: backendIcon(s.command), name: s.window_name, state: '' }));
  }

  async function reload() {
    try {
      // The sidebar is ordered by CONVERSATION, so the list needs one more fact:
      // when each room last had a message. One grouped query server-side.
      const [{ projects }, sp, roomsRes] = await Promise.all([
        projectList(true),
        listSessionsWithPanes(),
        hubRooms().catch(() => ({ rooms: {}, states: {} })),
      ]);
      const talk = roomsRes.rooms ?? {};
      // Kept for the rows themselves: the same map that orders the sidebar
      // answers "when did this project last say something" on each row, and
      // the states map colours the agent chips (owner, 2026-08-24: "上次回复
      // 的时间 … 当前几个 Agent 的简单 logo 状态").
      talkMap = talk;
      // Overlay the current roster BEFORE adopting the snapshot: hub_rooms
      // answers on a 20s cadence, so for the selected project this response
      // is usually OLDER than the 5s roster — adopting it raw rolled the
      // sidebar dots back and the two UIs disagreed again (board #8).
      agentStates = mergeStates(roomsRes.states ?? {}, selected, agents);
      // Archived projects are the RECYCLE BIN (owner, 2026-08-21: "相当于回收
      // 站的功能"): they leave the working list and wait, restorable, in the
      // collapsed section at the bottom of the sidebar.
      const all = projects ?? [];
      trash = all.filter((r) => r.project.archived);
      rows = sortRows(all.filter((r) => !r.project.archived), talk);
      panes = sp.panes ?? [];
      // First load: go back to the conversation that was open, and only fall
      // back to the top row when that project is gone.
      if (!selected && rows.length) {
        const remembered = rows.some((r) => r.project.session === hubPrefs.project)
          ? hubPrefs.project
          : rows[0].project.session;
        selectProject(remembered);
      }
    } catch { /* server without projects — the Hub tab is hidden anyway */ }
  }

  // A room you have visited before renders INSTANTLY from memory when you
  // return; a room you have not shows nothing until its first load answers.
  // Without both, every switch showed the empty-room PRESET panel for the
  // beat between `feed = []` and hub_log's answer — an "add an agent" pitch
  // flashing in front of rooms full of history (owner, 2026-08-25: "先看到
  // 添加 agent 一个 agent list那个页面闪了一下，然后再出来消息"). In-memory
  // only, per session; the pollers keep merging on top, so the cache is a
  // starting point, never a second source of truth.
  // ── History paging (board #9, frontend half): the first load is ONE page,
  // the poll stays incremental, and scrolling near the TOP walks backwards by
  // cursor — chat by `seq`, activity by the exact (ts, id) pair the server
  // hands back. `has_more=false` parks the walk; the cursors ride roomCache
  // so returning to a room never re-walks what it already loaded.
  let histSeq = $state(0);          // chat: oldest loaded seq (0 = no page yet)
  let histMore = $state(true);
  let actCursor = $state(null);     // activity: { ts, id } of the oldest loaded row
  let actMore = $state(true);
  let loadingOlder = $state(false); // one walk at a time — the dedupe
  const ACT_PAGE = 200;

  const roomCache = new Map();
  let roomReady = $state(false);

  async function selectProject(session) {
    // An unsent line belongs to the conversation it was written for. Park it on
    // the project we are leaving and pick up whatever was waiting in the new one
    // — carrying the text across would put it in front of the wrong agents.
    if (selected) hubPrefs.setDraft(selected, composerText);
    // Park the room too, so switching back is instant.
    if (selected) roomCache.set(selected, { feed, lastTs, activity, lastActivityTs, agents, histSeq, histMore, actCursor, actMore });
    selected = session;
    composerText = hubPrefs.draft(session);
    clearAttachments(); // staged for one room; must not ride into another
    expanded = {};      // an unfold is a reading choice, scoped to its room
    hubPrefs.setProject(session);
    // The chat's project is a working context like the terminal's: tell App,
    // so the Files tab follows whichever the user touched LAST (owner,
    // 2026-08-22: "chat里的路径没有刷新到文件 terminal好像就会刷新路径").
    onSelectSession(session);
    const c = roomCache.get(session);
    feed = c?.feed ?? [];
    activity = c?.activity ?? [];
    lastActivityTs = c?.lastActivityTs ?? 0;
    lastTs = c?.lastTs ?? 0;
    agents = c?.agents ?? [];
    histSeq = c?.histSeq ?? 0;
    histMore = c?.histMore ?? true;
    actCursor = c?.actCursor ?? null;
    actMore = c?.actMore ?? true;
    loadingOlder = false;
    // A cached room is ready NOW (its pollers refresh underneath); an unknown
    // one may not claim "empty" until its first load has actually answered.
    roomReady = !!c;
    recipient = '';
    // The cached roster can seat the recipient immediately — same rule as
    // loadAgents, which will confirm or correct it when the fresh roster lands.
    if (agents.length) recipient = pickLead(agents, registry, hubPrefs.lead(session));
    recipientOpen = false;
    menuFor = '';
    filterAgent = ''; // a filter is a reading choice, scoped to its room
    msgOpen = '';
    rawOpen = '';
    // The drawer follows the project (board #23, owner: "chat的右侧边栏打开
    // 哪个的状态前端帮我记住，这样我切换不同的 project 回来原来的视图还在"):
    // whichever partition was open when the user LEFT this room reopens on
    // return, and a room where it was closed comes back closed. No reading
    // anchor here — selectProject lands at the tail regardless.
    const dv = compact ? '' : hubPrefs.drawer(session);
    if (dv) drawerView = dv;
    termOpen = !!dv;
    // The old room's pane must never leak into this one's terminal partition.
    termTarget = ''; termCommand = '';
    if (dv === 'term') {
      const pick = agents.find((x) => x.managed) ?? agents[0];
      if (pick) pickWindow(pick);
    }
    // Entering a room lands at its tail, cached or not — a parked scrollTop
    // from the LAST room would point at arbitrary content in this one.
    following = true;
    if (feed.length) scrollFeed(true);
    await Promise.all([loadFeed(), loadAgents(), loadActivity()]);
    if (selected === session) roomReady = true;
    // A restored terminal partition may have had NO roster to pick from (a
    // first visit after reload restores before any cache exists) — seat it
    // once the fresh roster is in.
    if (selected === session && termOpen && drawerView === 'term' && !termTarget) {
      const pick = agents.find((x) => x.managed) ?? agents[0];
      if (pick) pickWindow(pick);
    }
  }

  async function loadFeed() {
    if (!selected) return;
    // The answer must still be about the question: every poller here freezes
    // the project it asked ABOUT and drops the reply if the user has switched
    // meanwhile — a late resolve was merging the OLD room's messages into the
    // NEW room's feed (same identity bug as the context-menu close: resolving
    // a live `selected` after the fact; owner, 2026-08-24).
    const s = selected;
    try {
      // First load asks for ONE page (the server's newest 100) and keeps its
      // cursor; every later call is the same incremental since_ts poll as
      // before (board #9).
      const first = lastTs === 0 && histSeq === 0;
      const res = await hubLog(s, lastTs, 100);
      if (selected !== s) return;
      const messages = res.messages;
      if (first) {
        histSeq = res.oldest_seq ?? 0;
        histMore = (res.has_more ?? false) && histSeq > 0;
      }
      if (messages?.length) {
        feed = mergeMessages(feed, messages);
        lastTs = Math.max(lastTs, ...messages.map((m) => m.ts ?? 0));
        if (following) scrollFeed(); else newBelow = true;
      }
    } catch { /* hub not available */ }
  }

  async function loadActivity() {
    // Polled at EVERY feed level: delivery receipts and undelivered-line
    // reports ride this channel, and those are about the message the user just
    // sent, not opt-in telemetry detail. feedBlocks does the level filtering.
    if (!selected) return;
    const s = selected;
    try {
      const first = lastActivityTs === 0 && !actCursor;
      const res = await hubActivity(s, lastActivityTs);
      if (selected !== s) return;
      const events = res.events;
      if (first) {
        actCursor = res.oldest ?? null;
        actMore = (res.has_more ?? false) && !!actCursor;
      }
      if (events?.length) {
        // mergeEvents, not concat-and-slice: the old -300 cap EVICTED the
        // history pages the user had walked to (board #9), and id-keyed
        // dedupe is what makes a prepend and a poll meet without doubles.
        activity = mergeEvents(activity, events);
        lastActivityTs = Math.max(lastActivityTs, ...events.map((e) => e.ts));
        // Telemetry rows are not "news": they extend the tail, so follow if we
        // were following, but they must not raise the new-messages dot.
        if (following) scrollFeed();
      }
    } catch { /* hub not available */ }
  }

  async function loadAgents() {
    if (!selected) return;
    const s = selected;
    try {
      const got = (await hubAgents(s)).agents ?? [];
      // A stale success is as wrong as a stale feed: the OLD project's roster
      // must not dress the NEW project (and then re-pick its recipient).
      if (selected !== s) return;
      agents = got;
      // The sidebar chips share this truth (board #8): the roster is the
      // freshest reading for THIS project, so its states overwrite the
      // 20s-cadence snapshot's keys instead of disagreeing beside them.
      agentStates = mergeStates(agentStates, s, got);
      // The recipient follows the room: an agent that left cannot be the
      // recipient, and a room that just gained its first agent gets a lead
      // without the user choosing one. ALL_TARGET is not a window, so it stays.
      if (recipient && recipient !== ALL_TARGET && !agents.some((a) => a.managed && a.name === recipient)) recipient = '';
      if (!recipient) recipient = pickLead(agents, registry, hubPrefs.lead(selected));
    } catch (e) {
      // "I could not ask" is not "there is nobody". Emptying the roster on a
      // failed poll is what made the cards — and with them the model and context
      // readings — blink away whenever the socket hiccuped or an RPC timed out
      // (owner, 2026-08-19: "有时候会闪没了，是不是中间心跳失败了"). The roster is
      // a last-known state; `selectProject` is the one place that clears it,
      // because that is the one time it is genuinely unknown.
      console.warn('hub agents poll failed, keeping the last roster', e);
    }
  }

  /** Follow the tail — but only while the user is AT the tail. Yanking someone
   * back down while they read history is worse than a missed autoscroll, so new
   * content only scrolls when `following`; sending forces it, because you plainly
   * want to see what you just sent. */
  function scrollFeed(force = false) {
    if (!force && !following) return;
    requestAnimationFrame(() => {
      if (!feedEl) return;
      feedEl.scrollTop = feedEl.scrollHeight;
      // Programmatic jumps have no continuous path to preserve. Seed from the
      // destination (latest passed message), and do not depend on a scroll event
      // — assigning the same scrollTop emits none.
      askScrollTop = feedEl.scrollTop;
      askDir = 'down';
      askDirTravel = 0;
      syncAsk('down', true);
      following = true;
      newBelow = false;
      markSeen();
    });
  }

  /** The red dot means "an agent replied and you have not looked". So it clears
   * when the newest message is on screen — the bottom of the feed — and when
   * you send, since you are plainly looking then. */
  function markSeen() {
    if (!selected || !visible) return;
    const newest = feed.reduce((max, m) => Math.max(max, m.ts ?? 0), 0);
    if (newest > hubPrefs.seen(selected)) hubPrefs.setSeen(selected, newest);
  }
  const atBottom = () => !feedEl || feedEl.scrollHeight - feedEl.scrollTop - feedEl.clientHeight < 40;
  const unread = $derived(unreadSenders(feed, hubPrefs.seen(selected)));

  let following = $state(true);   // the feed is parked at the tail
  let newBelow = $state(false);   // something arrived while it was not
  let askKey = $state('');        // the ONE user-message anchor on screen
  let askEdge = $state('');       // which edge that same bubble catches
  let askHeld = $state(false);    // true only after it has actually hit the edge
  let askScrollTop = 0;           // last observed position
  let askDir = 'down';            // committed direction — flips only after real travel
  let askDirTravel = 0;           // accumulated movement AGAINST the committed direction

  /** Walk one page BACK on each channel that still has one (board #9). One
   * walk at a time; the response is dropped if the room changed underneath;
   * the prepend re-enters through withReadingAnchor so the line being read
   * stays put — a jump to the tail here would throw the reader out of the
   * history they came for. */
  async function loadOlder() {
    if (loadingOlder || !selected || (!histMore && !actMore)) return;
    const s = selected;
    loadingOlder = true;
    try {
      const [older, olderAct] = await Promise.all([
        histMore && histSeq > 0 ? hubLog(s, 0, 100, histSeq) : Promise.resolve(null),
        actMore && actCursor ? hubActivity(s, 0, { limit: ACT_PAGE, before: actCursor }) : Promise.resolve(null),
      ]);
      if (selected !== s) return; // the room changed — this page is not ours
      // AWAITED: withReadingAnchor is async (it settles the DOM and then
      // compensates scrollTop). Releasing `loadingOlder` before that scroll
      // lands let the compensation itself re-enter onFeedScroll and walk the
      // SAME cursor twice (#9 review).
      await withReadingAnchor(() => {
        if (older) {
          if (older.messages?.length) feed = mergeMessages(feed, older.messages);
          histSeq = older.oldest_seq ?? histSeq;
          histMore = (older.has_more ?? false) && (older.oldest_seq ?? 0) > 0;
        }
        if (olderAct) {
          if (olderAct.events?.length) activity = mergeEvents(activity, olderAct.events);
          actCursor = olderAct.oldest ?? actCursor;
          actMore = (olderAct.has_more ?? false) && !!olderAct.oldest;
        }
      });
    } catch { /* keep the cursors — the next nudge retries */ }
    finally {
      if (selected === s) loadingOlder = false;
    }
  }

  function onFeedScroll() {
    following = atBottom();
    const top = feedEl?.scrollTop ?? 0;
    // Near the top: reach for the previous page. 120px of runway starts the
    // fetch before the reader actually hits the edge.
    if (top < 120 && roomReady) loadOlder();
    const delta = top - askScrollTop;
    askScrollTop = top;
    // Direction hysteresis: trackpad and touch momentum land 1–3px reversals at
    // rest, and at the held boundary a direction flip re-picks the anchor — the
    // reported flicker. A reversal only counts once it has travelled 16px.
    if (delta !== 0) {
      if ((delta > 0) === (askDir === 'down')) {
        askDirTravel = 0;
      } else {
        askDirTravel += Math.abs(delta);
        if (askDirTravel >= 16) {
          askDir = delta > 0 ? 'down' : 'up';
          askDirTravel = 0;
        }
      }
    }
    syncAsk(askDir);
    if (following) {
      newBelow = false;
      markSeen();
    }
    autoRefold();
  }

  /** One bubble, one continuous motion: select it while it is naturally inside
   * the viewport, then let CSS sticky catch that SAME element as it leaves in
   * the current scroll direction. In an empty stretch of a long reply, retain
   * it; never swap to another invisible message at an arbitrary midpoint. */
  /** The height a folded message should not exceed — a fifth of the
   * conversation, the owner's number. It is an ESTIMATE that feeds the line
   * budget below, never a cap on the box: the bubble is never clipped, the
   * text is folded to fit. Measured on the CHAT COLUMN (the feed's parent),
   * never the feed itself: the feed is the flex leftover after the composer,
   * so typing a multi-line message shrank it, the next `blocks` tick
   * re-measured, the budget dropped a line, and EVERY folded message re-cut —
   * heights shifting with no compensation, which read as the parked tail
   * drifting on its own (owner, 2026-08-27: "我打字输入，原来是最下方，但是一
   * 会儿又变化了"). The column holds still while the composer grows. */
  let heldBasis = $state(0);
  let heldLine = $state(20);        // measured line box of a bubble, px
  function measureHeld() {
    if (!feedEl) return;
    heldBasis = feedEl.parentElement?.clientHeight || feedEl.clientHeight;
    const bubble = feedEl.querySelector('.bubble');
    const lh = bubble ? parseFloat(getComputedStyle(bubble).lineHeight) : NaN;
    if (Number.isFinite(lh) && lh > 6) heldLine = lh;
  }
  $effect(() => {
    void blocks; void visible;
    measureHeld();
    // A REAL basis change (window resize) may re-cut every folded message, so
    // it keeps the reader's line still the same way the drawer toggle does.
    const onResize = () => withReadingAnchor(measureHeld);
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });
  /** How many whole lines a folded user message may show — the mapping is
   * `foldLines` in hub.ts (pure, tested; board #4): a flat small budget on
   * compact (the keyboard resize cannot move it), the screen-derived fifth
   * on desktop. */
  const heldLines = $derived(foldLines(compact, heldBasis, heldLine));
  /** The body a folded user message shows: a plain rear truncation (owner,
   * 2026-08-27: "直接后截断的形式 … 中间不要了，默认用户消息都截断"). Identity
   * when it already fits, so the common case re-renders nothing. */
  const foldBody = (text) => elideTail(text, heldLines);
  /** Messages the reader unfolded by hand, by key. Folding is the DEFAULT for
   * every long user message, so an unfold is a choice that stays until the
   * project changes — resetting it whenever the anchor moved would re-fold a
   * message the reader is still reading. */
  let expanded = $state({});
  /** Expanding releases the pin (see `pinned` in the markup), so the bubble
   * returns to its natural flow position — often pages away from the viewport
   * that was showing its pinned copy, which read as the message VANISHING
   * (owner, 2026-08-27: "现在点击展开 消息就不见了 应该展开跳转到那条消息的位
   * 置"). Jump the feed to the message's own position, unless its start is
   * already in view — then expansion just grows downward in place. */
  async function expandMsg(key) {
    expanded = { ...expanded, [key]: true };
    await settled();
    if (!feedEl) return;
    const el = feedEl.querySelector(`[data-ask="${CSS.escape(key)}"]`);
    if (!(el instanceof HTMLElement)) return;
    const top = feedEl.scrollTop;
    if (el.offsetTop >= top && el.offsetTop < top + feedEl.clientHeight - 60) {
      syncAsk();
      return;
    }
    feedEl.scrollTop = Math.max(0, el.offsetTop - 8);
    // Programmatic jump: seed the anchor path from the destination, the way
    // scrollFeed does — the scroll event this assignment fires then sees a
    // zero delta and invents no direction.
    askScrollTop = feedEl.scrollTop;
    askDirTravel = 0;
    syncAsk(askDir, true);
  }
  /** An expanded message the reader scrolled clean away from folds itself back
   * and rejoins the anchor pool (owner, 2026-08-27: "划走看不到以后 自动折叠
   * 并且钉住"). "Away" is the whole box out of the viewport by a margin, so a
   * pixel of overshoot does not snap it shut; the refold changes heights
   * outside the viewport, so it goes through the reading anchor. */
  function autoRefold() {
    if (!feedEl) return;
    const keys = Object.keys(expanded);
    if (!keys.length) return;
    const top = feedEl.scrollTop;
    const bottom = top + feedEl.clientHeight;
    const gone = keys.filter((k) => {
      const el = feedEl.querySelector(`[data-ask="${CSS.escape(k)}"]`);
      return el instanceof HTMLElement
        && (el.offsetTop + el.offsetHeight < top - 120 || el.offsetTop > bottom + 120);
    });
    if (!gone.length) return;
    const next = { ...expanded };
    for (const k of gone) delete next[k];
    withReadingAnchor(() => { expanded = next; });
  }

  function syncAsk(direction = askDir, reset = false) {
    if (!feedEl) { askKey = ''; askEdge = ''; askHeld = false; return; }
    // Chromium's offsetTop for a sticky element is its HELD position. Read that
    // and the old anchor appears naturally visible, so the next anchor is never
    // selected. Neutralize the one current sticky element for this synchronous
    // layout read; the inline override is removed before the browser can paint.
    const stickies = [...feedEl.querySelectorAll('.ask-top, .ask-bottom')];
    for (const el of stickies) el.style.position = 'static';
    const items = [...feedEl.querySelectorAll('[data-ask]')].map((el) => {
      // Folding is a default property of the TEXT now, never a held-state side
      // effect, so the box is the same height held or not and the real
      // offsetHeight is the right answer. (The old naturalH phantom-height
      // cache existed for fold-on-hold, where holding shrank the box, that
      // unheld it, and the blink came back sourced from the text.)
      const key = el.dataset.ask ?? '';
      return { key, top: el.offsetTop, height: el.offsetHeight };
    });
    for (const el of stickies) el.style.removeProperty('position');
    const picked = pickAnchor(
      items,
      feedEl.scrollTop,
      feedEl.clientHeight,
      feedEl.scrollHeight,
      direction,
      reset ? undefined : { key: askKey, edge: askEdge },
    );
    // Hysteresis on the HELD state, and it overrides direction re-edging: at
    // the edge the bubble is simultaneously "naturally visible" (within 1px)
    // and "touching the edge", so a micro scroll reversal (trackpad, touch
    // momentum) flips the prepared edge top<->bottom and held with it, which
    // made the held treatment blink on and off (the reported flicker). While
    // the SAME bubble is within 8px of the edge it already holds, it keeps
    // holding that edge regardless of instantaneous direction.
    const chosen = items.find((it) => it.key === picked.key);
    let edge = picked.edge;
    let held = false;
    if (chosen) {
      const top = feedEl.scrollTop;
      const bottom = top + feedEl.clientHeight;
      if (askHeld && askKey === picked.key && askEdge === 'top' && chosen.top <= top + 8) {
        edge = 'top'; held = true;
      } else if (askHeld && askKey === picked.key && askEdge === 'bottom'
        && chosen.top + chosen.height >= bottom - 8) {
        edge = 'bottom'; held = true;
      } else {
        held = edge === 'top' ? chosen.top <= top + 1
          : edge === 'bottom' ? chosen.top + chosen.height >= bottom - 1
          : false;
      }
    }
    askKey = picked.key;
    askEdge = edge;
    askHeld = held;
  }
  $effect(() => {
    void blocks;   // a new message changes both the set and the geometry
    requestAnimationFrame(syncAsk);
  });

  // The keyboard shrinks the visible viewport (App sets --app-height and fires
  // this), which otherwise leaves the tail below the fold: the feed keeps its
  // scrollTop while its box gets shorter. Re-park at the tail instead.
  $effect(() => {
    if (!visible) return;
    const onKb = () => { if (following) scrollFeed(true); };
    window.addEventListener('keyboard-shift', onKb);
    return () => window.removeEventListener('keyboard-shift', onKb);
  });

  async function send() {
    const raw = composerText.trim();
    if ((!raw && !pending.length) || !selected) return;
    // While a stage job is in flight the message is NOT whole yet: sending
    // now would post the text without its attachment, and the attachment
    // would land in an emptied composer as an orphaned token (lead review,
    // board #25). The button is disabled too — this entry guard covers the
    // keyboard's Enter and any future caller.
    if (attaching) return;
    // A SLASH COMMAND goes to the agent's CLI, not to its model, so it is typed
    // verbatim — no `[tmm chat …] human:` stamp, no @address, nothing the TUI
    // would read as prose. It needs a target: an explicit `@name`, else the
    // composer's recipient. With neither (a room note) there is nobody to run
    // it, so it stays an ordinary message rather than vanishing.
    const cmd = slashCommand(raw);
    const cmdTarget = cmd && (cmd.to || (recipient === ALL_TARGET ? 'all' : recipient));
    if (cmd && cmdTarget) {
      const room = selected; // same room-snapshot rule as the message path
      composerText = '';
      following = true;
      scrollFeed(true);
      try {
        await hubCommand(room, cmdTarget, cmd.command);
        if (selected !== room) return;
        await loadFeed();
        scrollFeed(true);
      } catch (e) {
        console.warn('hub command failed', e);
        if (selected === room) composerText = raw;
      }
      return;
    }
    // The recipient makes "talk to THIS agent" the default rather than a
    // gesture: addressed() prefixes @name unless the user @-addressed someone
    // by hand, and an empty recipient posts to the room.
    const atts = pending;
    let body = raw;
    const stragglers = [];
    for (const a of atts) {
      const ref = a.kind === 'image' ? `![](${a.path})` : a.path;
      const tok = attachToken(a);
      if (body.includes(tok)) body = body.replace(tok, ref); // position preserved
      else stragglers.push(ref);
    }
    body = [body, ...stragglers].filter(Boolean).join('\n');
    const text = addressed(body, recipient);
    // Room snapshot: everything after the await below must answer to the
    // room this message BELONGS to, never to whichever room is on screen
    // when the RPC returns (lead review, board #25, round 2 — the success
    // path reset attachSeq into a room that had meanwhile staged its own
    // attachments, colliding token numbers, and refreshed the wrong feed).
    const room = selected;
    composerText = '';
    pending = [];
    following = true;
    scrollFeed(true);
    try {
      await hubPost(room, text);
      // The thumbs belong to the delivered attachments — dead either way.
      for (const a of atts) if (a.thumb) URL.revokeObjectURL(a.thumb);
      if (selected !== room) return; // the new room's numbering/feed are not ours
      attachSeq = 1;
      await loadFeed();
      scrollFeed(true);
    } catch (e) {
      console.warn('hub post failed', e);
      // A post that fails AFTER the user switched projects must not restore
      // the old room's draft/attachments into the new one. The refs are
      // already uploaded; the draft is lost with the failed post — losing it
      // beats corrupting another room.
      if (selected === room) { pending = atts; composerText = raw; }
      else for (const a of atts) if (a.thumb) URL.revokeObjectURL(a.thumb);
    }
  }

  /** Grow to fit what is being typed, up to the CSS ceiling, then let it scroll.
   * Height has to be measured, not guessed: wrapping depends on the font, the
   * width and the text. Reset to `auto` first or the box can only ever grow. */
  /* The send button does NOT reserve a column (owner: text may run directly
     above it). The textarea is full width; a hidden mirror re-lays-out the
     value to find the LAST line's right edge, and only when that edge would
     collide with the button zone does the box gain one line of bottom
     padding — the same "share the last line, else drop below" semantics as
     the bubble's meta trailer. A textarea cannot flow around a float, so
     the mirror is the only honest way to know where the last line ends. */
  let mirrorEl = null;
  const SEND_ZONE = 74; // attach + send (30px each + gaps), from the textarea's right edge
  function lastLineCollides(el) {
    if (!el.value) return false;
    if (!mirrorEl) {
      mirrorEl = document.createElement('div');
      mirrorEl.className = 'c-mirror';
      el.parentElement?.appendChild(mirrorEl);
    }
    mirrorEl.style.width = `${el.clientWidth}px`;
    mirrorEl.style.textIndent = el.style.textIndent || '0';
    mirrorEl.textContent = el.value;
    const marker = document.createElement('span');
    marker.textContent = '\u200b';
    mirrorEl.appendChild(marker);
    return marker.offsetLeft > el.clientWidth - SEND_ZONE;
  }
  function growComposer() {
    const el = composerEl;
    if (!el) return;
    // Measure the natural height first, with no avoidance applied.
    el.style.paddingBottom = '';
    el.style.paddingRight = '';
    el.style.height = 'auto';
    const maxH = parseFloat(getComputedStyle(el).maxHeight) || Infinity;
    if (el.scrollHeight > maxH + 1) {
      // Scrolled state: the box is at max height and the button permanently
      // overlays its bottom-right corner — EVERY line scrolls past it, so
      // all lines shorten clear of the button zone while scrolling lasts.
      el.style.paddingRight = '76px'; // clear BOTH corner buttons
    } else if (lastLineCollides(el)) {
      // Tail collision: clear the button's full height (top edge sits
      // ~30px above the textarea's bottom; 34px keeps descenders clear),
      // not just one line box — a 24px pad still left the button's top
      // strip over the glyphs (owner report).
      el.style.paddingBottom = '34px';
    }
    el.style.height = 'auto';
    el.style.height = `${el.scrollHeight}px`;
    // The composer taking space is the feed losing it — the same way the keyboard
    // does — so keep the tail parked while it grows. ONLY when it actually
    // grew: this runs per KEYSTROKE, and re-parking an unchanged tail
    // re-assigns scrollTop (fractional clamping under --ui-zoom lands on a
    // different pixel each time — the measured 2261 → 2221↔2298 oscillation)
    // and re-seeds the ask anchor, so the whole feed shivered as you typed
    // (owner, 2026-08-26: "每打一个字符…内容就会上下闪烁"). The SHELL is what
    // takes space (chips rows count, not just the textarea).
    const shellH = el.parentElement?.offsetHeight ?? el.offsetHeight;
    if (shellH !== lastShellH) {
      lastShellH = shellH;
      if (following) scrollFeed(true);
    }
  }
  let lastShellH = 0;
  $effect(() => {
    void composerText;   // includes the reset to '' after sending
    void toChipW;        // the indent changes wrapping, so height must re-measure
    void composerIsCmd;  // the mono flip changes metrics, so height re-measures too
    growComposer();
  });

  // ── Attach an image (owner, 2026-08-26: "我发送图片时可以有一个小的+按钮，
  // 上传到项目下…创建临时目录，随机图片 id，并且转 webp…限制一下原图"). The
  // picked image is downscaled CLIENT-side to the models' effective ceiling —
  // Claude reads best at ≤1568px on the long edge and GPT caps at 2048, so
  // 1568 serves both and a 12 MB phone photo becomes a ~100 KB webp before it
  // crosses the wire. Encoding prefers webp; WebKit cannot ENCODE webp, so the
  // blob's own type decides the extension (jpeg there). The upload lands in
  // <ws>/.tmm/uploads/ via the same fs_upload the file browser uses, and the
  // composer gains a `![](path)` line — send() delivers the PATH into the
  // agent's pane (an image is a reference, never bytes), and the feed renders
  // it through ChatImage like any other ref.
  let fileEl = $state(null);
  // The staged set's generation: bumped whenever it is invalidated (project
  // switch, explicit clear). An async stage job snapshots it at entry and
  // refuses to touch pending/composerText once stale — without this, an
  // upload finishing AFTER a project switch refilled the NEW room's composer
  // with the OLD room's attachment (lead review, board #25).
  let attachGen = $state(0);
  // In-flight stage jobs, each remembering the GENERATION it belongs to. A
  // list and not a flag (paste + picker overlap; a flag dropped the gate when
  // the first job finished), and per-generation so `attaching` answers for
  // the room on screen: a stale job from the room the user LEFT neither
  // holds the new room's send closed nor — via its finally — unlocks a job
  // the new room started, because every job adds and removes only its OWN
  // entry (lead review, board #25: 旧 finally 不能解锁新 job).
  let jobGens = $state([]);
  const attaching = $derived(jobGens.includes(attachGen));
  // Uploaded, waiting to ride the next send. The composer never shows the
  // markdown path line (owner, 2026-08-26: "消息框内部不展示完整的上传图片的
  // markdown 格式路径，就用一个 Image 的 placeholder 代替") — each attachment
  // is a chip above the textarea; the ref joins the body at SEND time.
  // [{ path, kind: 'image'|'file', name, n, thumb }] — n is the token number
  // the composer text carries as `[img:n]` / `[file:n]` at the INSERTION
  // POINT (owner, 2026-08-26: "会有图片在文本里的相对位置信息吗…要让我能够
  // 看到图片插入的相对位置在哪里"): the token is the visible position marker
  // (a textarea cannot style spans), and send() swaps it for the real ref IN
  // PLACE, so the prompt keeps the image exactly where the words put it.
  // thumb is an object URL for the picked image — the chip shows the picture
  // itself, so "哪几张加上去了" is answered by looking.
  let pending = $state([]);
  let attachSeq = 1;
  const attachToken = (a) => `[${a.kind === 'image' ? 'img' : 'file'}:${a.n}]`;

  function removeAttachment(i) {
    const a = pending[i];
    if (!a) return;
    const tok = attachToken(a);
    // Strip the token (and one adjacent space) wherever the user left it.
    composerText = composerText.replace(new RegExp(`\\s?${tok.replace(/[[\\]]/g, '\\$&')}`), '');
    if (a.thumb) URL.revokeObjectURL(a.thumb);
    pending = pending.filter((_, j) => j !== i);
    if (!pending.length) attachSeq = 1;
  }
  function clearAttachments() {
    attachGen++; // any in-flight stage job is now stale — it must not refill
    for (const a of pending) if (a.thumb) URL.revokeObjectURL(a.thumb);
    pending = [];
    attachSeq = 1;
  }
  const sendable = $derived(!!composerText.trim() || !!pending.length);
  const IMG_EDGE = 1568;
  const FILE_CAP = 32 * 1024 * 1024; // base64 over one RPC; beyond this, point the agent at the original path instead

  async function encodeImage(file) {
    const bmp = await createImageBitmap(file);
    const k = Math.min(1, IMG_EDGE / Math.max(bmp.width, bmp.height));
    const w = Math.max(1, Math.round(bmp.width * k)), h = Math.max(1, Math.round(bmp.height * k));
    const canvas = document.createElement('canvas');
    canvas.width = w; canvas.height = h;
    canvas.getContext('2d').drawImage(bmp, 0, 0, w, h);
    bmp.close?.();
    const blob = await new Promise((res) => canvas.toBlob(res, 'image/webp', 0.85));
    const out = blob?.type === 'image/webp' ? blob
      : await new Promise((res) => canvas.toBlob(res, 'image/jpeg', 0.85));
    if (!out) throw new Error('encode failed');
    return { b64: toB64(new Uint8Array(await out.arrayBuffer())), ext: out.type === 'image/webp' ? 'webp' : 'jpg' };
  }

  // Chunked, never one big spread (Key Patterns: base64 large data).
  function toB64(bytes) {
    let bin = '';
    for (let i = 0; i < bytes.length; i += 8192) bin += String.fromCharCode(...bytes.subarray(i, i + 8192));
    return btoa(bin);
  }

  async function onPickFiles(e) {
    const files = [...(e.target.files || [])];
    e.target.value = ''; // same file re-pickable
    await stageFiles(files);
  }

  /** Paste is the second door into the SAME staging pipeline (board #25:
   * "chat 输入框，应该支持剪贴板粘贴图片或者文件等"): a screenshot or a copied
   * file lands exactly like the + button's pick — image re-encoded, file
   * uploaded byte-identical, chip above the box, [img:n]/[file:n] token at
   * the caret. Files WIN over text riding the same clipboard (a copied file
   * also carries its path as text — inserting it beside the staged chip
   * would say the same thing twice), so the default insertion is suppressed
   * exactly when there are files to stage; a plain text paste is untouched. */
  function onComposerPaste(e) {
    const files = pastedFiles(e.clipboardData);
    if (!files.length) return; // text-only paste: the textarea's own business
    e.preventDefault();
    stageFiles(files);
  }

  async function stageFiles(files) {
    const ws = selectedRow?.project.path;
    if (!ws || !files.length) return;
    // Where the tokens land: the caret's last position (the file dialog
    // blurs the box but the selection survives; a paste's caret is live),
    // else the end.
    let at = composerEl?.selectionStart ?? composerText.length;
    // Room snapshot + generation: every await below is a chance for the user
    // to switch projects (selectProject → clearAttachments bumps the gen). A
    // stale job may still finish its upload — a harmless orphan in the OLD
    // room's .tmm/uploads — but must never touch pending, the composer text
    // or the sequence counter again: those belong to the room on screen NOW.
    const gen = attachGen;
    const stale = () => gen !== attachGen;
    jobGens = [...jobGens, gen];
    try {
      await fsMkdir(`${ws}/.tmm/uploads`); // create_dir_all — idempotent
      if (stale()) return;
      // Self-gitignored like the other .tmm runtime dirs — a chat attachment
      // must never show up in the project's `git status`.
      await fsUpload(`${ws}/.tmm/uploads/.gitignore`, btoa('*\n'));
      if (stale()) return;
      for (const f of files) {
        let item;
        if (f.type.startsWith('image/')) {
          // Images are re-encoded (webp, capped long edge) — 2(c).
          const { b64, ext } = await encodeImage(f);
          if (stale()) return;
          const path = uploadImagePath(ws, imageId(), ext);
          await fsUpload(path, b64);
          if (stale()) return;
          item = { path, kind: 'image', name: f.name, n: attachSeq++, thumb: URL.createObjectURL(f) };
        } else {
          // Everything else lands BYTE-IDENTICAL under its own name — 2(a)/3(a).
          if (f.size > FILE_CAP) { console.warn('attach skipped (too large)', f.name, f.size); continue; }
          const b64 = toB64(new Uint8Array(await f.arrayBuffer()));
          if (stale()) return;
          const path = uploadFilePath(ws, imageId(), f.name);
          await fsUpload(path, b64);
          if (stale()) return;
          item = { path, kind: 'file', name: f.name, n: attachSeq++, thumb: '' };
        }
        // No await between the last check and these mutations — the commit
        // is atomic with the verdict that this job's room is still on screen.
        pending = [...pending, item];
        // The visible position marker, at the caret.
        const tok = attachToken(item);
        const pre = composerText.slice(0, at), post = composerText.slice(at);
        const sep = pre && !/\s$/.test(pre) ? ' ' : '';
        composerText = `${pre}${sep}${tok}${post}`;
        at += sep.length + tok.length;
      }
      composerEl?.focus();
    } catch (err) {
      console.warn('attach failed', err);
    } finally {
      // Remove exactly THIS job's entry — never a blanket reset: a stale
      // job's finally must not unlock a job the new room started.
      const i = jobGens.indexOf(gen);
      if (i >= 0) jobGens = [...jobGens.slice(0, i), ...jobGens.slice(i + 1)];
    }
  }

  // Is what is typed going to be RUN rather than SAID? Mirrors send()'s own
  // branch exactly — slashCommand() recognises the shape, and a target must
  // exist (explicit @name, else the recipient; a room note has nobody to run
  // it and stays a message) — so the composer's look never promises a command
  // that send() would deliver as prose (owner, 2026-08-20: "如果是指令的话在输
  // 入框里样式改变一下").
  const composerIsCmd = $derived.by(() => {
    const c = slashCommand(composerText.trim());
    return !!(c && (c.to || recipient));
  });

  /** Enter sends where there is a keyboard with modifiers, and inserts a newline
   * on a touch device — where the return key is the ONLY way to get one and the
   * send button is right there. Shift+Enter is always a newline. */
  // ── Slash-command completion. Typing `/` offers the agent CLI's commands;
  // choosing one that takes an argument offers ITS values next (the model ids
  // come from the server, which asks the CLI). Two stages, one palette.
  let cmdModels = $state({});          // backend → model ids (each fetched once)
  let paletteIdx = $state(0);
  let paletteOff = $state(false);      // Escape closes it until the text changes
  // The palette speaks the ADDRESSEE's dialect: each CLI has its own command
  // table (kiro's /tangent does not exist in codex; codex's /model is a
  // picker), so the table follows the explicit @name, else the composer's
  // recipient. @all with a mixed roster gets no palette — one command line
  // cannot be right in two dialects at once (owner, 2026-08-22 对齐).
  const paletteBackend = $derived.by(() => {
    const m = /^\s*@([\w][\w.-]*)\s/u.exec(composerText ?? '');
    const name = m ? m[1] : (recipient === ALL_TARGET ? null : recipient);
    if (name) return agents.find((a) => a.managed && a.name === name)?.agent ?? '';
    const backends = [...new Set(managedAgents.map((a) => a.agent ?? ''))];
    return backends.length === 1 ? backends[0] : 'mixed';
  });
  const palette = $derived(paletteOff ? null : commandPalette(composerText, cmdModels[paletteBackend] ?? [], paletteBackend));
  // The open menu's agent, as its status line reads right now.
  // The menu header's reading DROPS the model: the model belongs to the
  // agent's CONFIG, and the menu now links there instead of quoting it
  // (owner, 2026-08-25: "菜单里不应该有模型名，可以有一个跳转到模型配置的
  // 页面的选项"). Context/effort/branch stay — they are live state, not
  // configuration.
  const vitalsFor = $derived((() => {
    const v = managedAgents.find((a) => a.name === menuFor)?.vitals;
    if (!v) return '';
    const parts = [];
    if (v.context_pct != null) parts.push(`${v.context_pct}% ctx`);
    if (v.effort) parts.push(v.effort);
    if (v.branch) parts.push(v.branch);
    return parts.join(' · ');
  })());
  /** The menu's subject, for its header lines (state + elapsed). */
  const menuAgent = $derived(managedAgents.find((a) => a.name === menuFor));
  $effect(() => { void composerText; paletteOff = false; });
  // The draft survives a reload because a half-written message is work. Written
  // on every keystroke: one small JSON string, and the alternative (a debounce)
  // loses the last few characters exactly when the tab goes away.
  $effect(() => { hubPrefs.setDraft(selected, composerText); });
  $effect(() => { void palette; paletteIdx = 0; });
  // The model list is only needed once a command wants it, and the server caches
  // it for ten minutes — so this asks at most once per backend per Hub visit.
  // kiro and grok can enumerate their models; claude and codex return null.
  $effect(() => {
    const backend = paletteBackend;
    if (!palette || cmdModels[backend]) return;
    modelsList(backend || 'kiro').then((r) => { cmdModels = { ...cmdModels, [backend]: r.models ?? [] }; }).catch(() => {});
  });

  /** Put the chosen completion in the box. `more` keeps the palette alive for the
   * argument, which is what makes a two-part command one flow. */
  function acceptCompletion(item) {
    if (!palette) return;
    const head = composerText.slice(0, palette.from);
    composerText = `${head}${item.value}${palette.more ? ' ' : ''}`;
    paletteIdx = 0;
    composerEl?.focus();
    // Put the caret at the end; assigning `value` in Svelte leaves it wherever
    // it was, which on a re-render means before the text we just inserted.
    requestAnimationFrame(() => composerEl?.setSelectionRange(composerText.length, composerText.length));
  }

  // The composer's readline set (Ctrl-A/E/U/K/W/Y/D/H/T/F/B): the kill buffer
  // and the accumulation chain live here; the arithmetic is readlineEdit()
  // (pure, in hub.ts). Ours everywhere — macOS keeps the half it had natively,
  // every other platform gains the whole set, and none of them drift apart.
  let killBuf = '';
  let killChain = false;

  function onComposerKey(e) {
    if (e.ctrlKey && !e.metaKey && !e.altKey && !e.isComposing && composerEl) {
      const r = readlineEdit({
        key: e.key.toLowerCase(), text: composerEl.value,
        start: composerEl.selectionStart, end: composerEl.selectionEnd,
        kill: killBuf, killing: killChain,
      });
      if (r) {
        e.preventDefault();
        killBuf = r.kill;
        killChain = r.killing;
        composerText = r.text;
        // The caret AFTER Svelte writes the value back — setting it before
        // would let the value update reset it to the text's end.
        settled().then(() => composerEl?.setSelectionRange(r.caret, r.caret));
        return;
      }
    }
    // Any other keystroke breaks the kill chain: Ctrl-K Ctrl-K accumulates,
    // Ctrl-K <type> Ctrl-K replaces — readline's own rule.
    killChain = false;
    // Ctrl+C twice on an EMPTY composer sends an interrupt (owner, 2026-08-24)
    // — the terminal's own cancel gesture, aimed at the addressee. Only while
    // empty: with text present (or a selection) Ctrl+C stays the browser's
    // copy, and readlineEdit deliberately lets it fall through.
    if (e.ctrlKey && !e.metaKey && !e.altKey && !e.isComposing
        && e.key.toLowerCase() === 'c' && !composerText.trim()) {
      e.preventDefault();
      armInterrupt();
      return;
    }
    if (e.key === 'Escape' && intArm) { e.preventDefault(); intArm = false; return; }
    // The palette owns the arrows, Tab and Enter while it is open — it is a
    // menu, and a menu that ignores the keyboard is a menu you have to reach for
    // the mouse to use.
    if (palette?.items.length) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const step = e.key === 'ArrowDown' ? 1 : -1;
        paletteIdx = (paletteIdx + step + palette.items.length) % palette.items.length;
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey && !e.isComposing)) {
        e.preventDefault();
        acceptCompletion(palette.items[paletteIdx] ?? palette.items[0]);
        return;
      }
      if (e.key === 'Escape') { e.preventDefault(); paletteOff = true; return; }
    }
    if (e.key !== 'Enter' || e.shiftKey || e.isComposing) return;
    if (compact) return;      // let the newline through; tap send
    e.preventDefault();
    send();
  }

  /** What the agent's own status line says, as one line. Sniffed server-side
   * from the last lines of its pane (there is no API for a CLI's live state), so
   * every field is a maybe and a missing one is simply absent — never a zero, a
   * dash, or a guess. */
  function vitalsLine(v) {
    if (!v) return '';
    const parts = [];
    if (v.model) parts.push(v.model);
    if (v.context_pct != null) parts.push(`${v.context_pct}% ctx`);
    if (v.effort) parts.push(v.effort);
    if (v.branch) parts.push(v.branch);
    return parts.join(' · ');
  }
  // ── Messages are NOT deletable in the UI (owner, 2026-08-21: "没有消息删除
  // 不需要这个功能，彻底去掉吧" — the two-step archive tried on 08-19 read as
  // clutter). The room is the record; the server's hub_msg_* RPCs remain for
  // the API, the client just never calls them.
  /** The card's share of the reading: what makes THIS agent different from its
   * neighbours. The MODEL only — effort belongs behind the tap, in the agent
   * menu's header line, next to context and branch (owner, 2026-08-26: "只展示
   * 模型名字就可以了 … Effort 只要在我点击额外展开时再去展示"); branch and cwd
   * are project-wide, so they too stay in the tooltip and the menu — repeating
   * them on every card would be chrome, not data. */
  function cardVitals(v) {
    return v?.model ?? '';
  }

  // ── The card's interaction state machine (board #3, owner): a click on an
  // UNSELECTED card only selects it; a click on the SELECTED card opens the
  // menu; a DOUBLE click filters the feed to that agent. The menu waits one
  // double-click window so click-click never flashes it before the filter.
  let filterAgent = $state('');
  let cardTimer = null;
  let cardTimerFor = ''; // WHICH card the pending menu belongs to — a click on
                         // a DIFFERENT card within the window must still act
                         // (review of board #3: the global timer swallowed it)
  function cardClick(name, el) {
    if (cardTimer) {
      clearTimeout(cardTimer); cardTimer = null;
      if (cardTimerFor === name) return; // 2nd of a double on the SAME card — dblclick acts
      // another card's pending menu is stale; fall through and act on THIS one
    }
    if (recipient !== name) { setRecipient(name); return; }               // select first, options later
    cardTimerFor = name;
    cardTimer = setTimeout(() => { cardTimer = null; toggleAgentMenu(name, el); }, 260);
  }
  function cardDbl(name) {
    if (cardTimer) { clearTimeout(cardTimer); cardTimer = null; }
    menuFor = '';
    setRecipient(name);
    filterAgent = filterAgent === name ? '' : name; // double-click again = exit
  }

  /** Choosing a recipient is also choosing this project's lead: it is the same
   * decision ("who am I working with here"), so it persists. */
  function setRecipient(name) {
    recipient = name;
    recipientOpen = false;
    // An armed interrupt was armed FOR a target; changing the addressee is a
    // new intent, so the button stands down (same rule as switching projects).
    intArm = false;
    if (selected) hubPrefs.setLead(selected, name);
  }

  // Terminal drawer: pick a window (any window — this is where direct
  // windows and shells live) and show it.
  /* Interrupt: type Escape into the agent's own pane. An agent CLI reacts to
     what lands in its input (the same truth `deliver_mentions` rests on), and
     Escape is how every supported TUI cancels the turn it is in — a `tmm`
     message could not, because a busy agent reads chat only between turns.
     Escape must be the NAMED key: with extended-keys on, tmux drops raw C0
     bytes sent to a pane in extended mode (see CLAUDE.md). Stop/restart stay
     separate and heavier: this cancels output, it does not kill the agent. */
  async function interrupt(name) {
    // One implementation, server-side (`hub_agent_interrupt`), so the CLI's
    // `tmm agent interrupt` and this button cannot drift apart — and so the
    // managed-agent gate is enforced in the same place as stop/restart.
    try {
      await hubAgentInterrupt(selected, name);
    } catch (e) {
      console.warn('interrupt failed', name, e);
    }
  }

  // The composer's interrupt reaches whoever the composer reaches: the named
  // recipient, or every managed agent for @all. An unaddressed room note
  // interrupts NOBODY — same shape as message delivery, no third rule.
  const intTargets = $derived(
    recipient === ALL_TARGET ? managedAgents.map((a) => a.name)
    : recipient ? [recipient] : []);
  const intWho = $derived(recipient === ALL_TARGET ? '@all' : recipient ? `@${recipient}` : '');

  // The recipient's turn is OPEN (running a turn, or holding an ask) — the
  // send button wears a slow spinner around a stop square, the reader's cue
  // that "this one is mid-turn, tapping here is how you cut it" (owner,
  // 2026-08-25: the lightning bolt "看着好像不是那么容易理解"). Not for
  // idle/failed: an ended turn has nothing to interrupt.
  const BUSY_STATES = ['running', 'working', 'waiting', 'blocked'];
  const recipientBusy = $derived(
    recipient === ALL_TARGET
      ? managedAgents.some((a) => BUSY_STATES.includes(a.state))
      : managedAgents.some((a) => a.name === recipient && BUSY_STATES.includes(a.state)));

  /** First activation arms; the second (button or Ctrl+C, mixable) fires. */
  function armInterrupt() {
    if (!selected || !intTargets.length) return;
    if (intArm) { void fireInterrupt(); return; }
    intArm = true;
    clearTimeout(intTimer);
    // An armed cancel button should not lie in wait: unfired, it stands down.
    intTimer = setTimeout(() => { intArm = false; }, 3000);
  }
  async function fireInterrupt() {
    clearTimeout(intTimer);
    intArm = false;
    const targets = intTargets;
    if (!selected || !targets.length) return;
    following = true;
    try {
      await Promise.all(targets.map((n) => hubAgentInterrupt(selected, n)));
      // The room recorded `[tmm] interrupted <name>` — show it where the
      // owner asked to see it ("发送 interrupt 的状态在消息列表里也要展示").
      await loadFeed();
      scrollFeed(true);
    } catch (e) { console.warn('interrupt failed', e); }
  }
  // Typing disarms — the button means "send" again the moment there is text —
  // and so does switching projects: an armed cancel must not follow the user
  // into another room.
  $effect(() => { if (composerText.trim()) intArm = false; });
  $effect(() => { void selected; intArm = false; });

  /** Run a layout-changing mutation without losing the reader's place.
   *
   * Opening or closing the terminal drawer regrids the columns: the feed
   * narrows, every message rewraps to a new height, and the same scrollTop now
   * points at different content — the reader's message drifts away (owner,
   * 2026-08-20: "点击右侧 terminal 按钮后，当前消息变窄，导致当前消息位置漂移").
   * `overflow-anchor` cannot help — it is off on purpose (the held-ask blink) —
   * so this does the anchoring by hand: remember the topmost visible block and
   * its offset from the feed's top edge, mutate, then put the SAME element back
   * at the SAME offset. Svelte's keyed each preserves the DOM node, so identity
   * is the element itself; a pinned (sticky) ask is skipped as the reference
   * because its rect does not move with the flow. At the tail, just stay at the
   * tail — that is what "where I was" means there. */
  async function withReadingAnchor(mutate) {
    if (!feedEl) { mutate(); return; }
    if (following) {
      mutate();
      await settled();
      scrollFeed(true);
      return;
    }
    const feedTop = feedEl.getBoundingClientRect().top;
    const ref = [...feedEl.children].find((el) =>
      // Any sticky variant is a bad reference: its rect is its PINNED position,
      // which does not move with the flow the way the content does.
      !el.classList.contains('held') && !el.classList.contains('ask-top')
      && !el.classList.contains('ask-bottom')
      && el.getBoundingClientRect().bottom > feedTop + 1);
    const delta = ref ? ref.getBoundingClientRect().top - feedTop : 0;
    mutate();
    await settled();
    if (ref?.isConnected) {
      const now = ref.getBoundingClientRect().top - feedEl.getBoundingClientRect().top;
      feedEl.scrollTop += now - delta;
    }
    // The anchor machinery reads geometry; it must re-decide for the new widths.
    syncAsk();
  }

  function openDrawer(a = null) {
    const pick = a ?? agents.find((x) => x.managed) ?? agents[0];
    if (pick) {
      const p = panes.find((p) => p.session === selected && p.window === pick.window && p.active)
        ?? panes.find((p) => p.session === selected && p.window === pick.window);
      if (p) {
        termTarget = `${p.session}:${p.window}.${p.pane}`;
        termCommand = p.current_command || '';
      }
    }
    if (mobile) {
      // The phone has a whole Terminal tab — jump there instead of a drawer.
      const m = /^(.+):(\d+)\.(\d+)$/.exec(termTarget);
      if (m) openTerminal(selected, termTarget, termCommand);
      return;
    }
    withReadingAnchor(() => { termOpen = true; });
    // The drawer follows the project (board #23): remember which partition
    // this room has open, so returning to it restores the same view.
    hubPrefs.setDrawer(selected, drawerView);
  }

  /** The one close path, so every trigger keeps the reader's place. */
  function closeDrawer() {
    withReadingAnchor(() => { termOpen = false; });
    hubPrefs.setDrawer(selected, '');
  }

  function pickWindow(a) {
    const p = panes.find((p) => p.session === selected && p.window === a.window && p.active)
      ?? panes.find((p) => p.session === selected && p.window === a.window);
    if (p) {
      termTarget = `${p.session}:${p.window}.${p.pane}`;
      termCommand = p.current_command || '';
    }
  }

  async function bringUp() {
    if (!selectedRow) return;
    try {
      await projectUp(selectedRow.project.id);
      await reload();
      await loadAgents();
    } catch (e) { console.warn('up failed', e); }
  }

  // ── Renaming a project happens where you read its name: the chat header.
  // A project is named by its NAME and identified by its SESSION, so this
  // edits the label only — the room (`proj:<session>`) and the tmux session
  // stay put, which is why the conversation survives a rename.
  let renaming = $state(false);
  let renameDraft = $state('');
  let renameEl = $state(null);
  let titleNameEl = $state(null); // the h1's name span — the title menu's anchor (board #32)

  function startRename() {
    if (!selectedRow) return;
    renameDraft = selectedRow.project.name;
    renaming = true;
  }
  // Focus + select once the input exists, so typing replaces the old name.
  $effect(() => {
    if (renaming && renameEl) { renameEl.focus(); renameEl.select(); }
  });
  async function commitRename() {
    if (!renaming || !selectedRow) return;
    const name = renameDraft.trim();
    const id = selectedRow.project.id;
    const was = selectedRow.project.session;
    renaming = false;
    if (!name || name === selectedRow.project.name) return;
    try {
      const res = await projectRename(id, name);
      // The session may have followed the name. Everything keyed by it — the
      // remembered lead, the read marker, which project is open — follows too,
      // and the feed reloads under the new key.
      if (res?.session && res.session !== was) {
        hubPrefs.renameSession(was, res.session);
        // The Terminal may be pointing at `<old session>:<win>.<pane>`, which
        // stops resolving the moment tmux renames the session. Nothing here can
        // reach that state, so say it out loud and let App remap.
        window.dispatchEvent(new CustomEvent('project-renamed', {
          detail: { from: was, to: res.session },
        }));
        await reload();
        await selectProject(res.session);
        return;
      }
      await reload();
    } catch (e) { console.warn('rename failed', e); }
  }





  // Spawn into the CURRENT project (from the chat header).
  let sideOpen = $state(false);     // phone: the project list, as a drawer
  // ONE way to add agents, in two moods: 'start' also makes the first pick the
  // lead (an empty room), 'add' leaves the current lead alone (a running one).
  let pickerOpen = $state(false);
  let pickerMode = $state('start');
  let startPick = $state([]);
  let startBrief = $state('');
  let starting = $state(false);
  /** Add agents to the conversation: one, or several at once. Each is an
   * existing `hub_spawn`, run in order so the roster appears in the order it was
   * picked. In 'start' mode the new roster also gets a lead — the first that can
   * hire, else the first picked, the same rule pickLead applies to a live room.
   * In 'add' mode whoever you were talking to stays the recipient. */
  async function addAgents(names, brief = '', mode = pickerMode) {
    if (!selected || !names.length || starting) return;
    starting = true;
    pickerOpen = false;
    try {
      for (const name of names) {
        try { await hubSpawn(selected, name, brief); }
        catch (e) { console.warn('spawn failed', name, e); }
      }
      await Promise.all([reload(), loadAgents(), loadFeed()]);
      if (mode === 'start' || !recipient) {
        setRecipient(names.find((n) => registry.find((r) => r.name === n)?.can_hire) ?? names[0]);
      }
    } finally {
      starting = false;
      startPick = [];
      startBrief = '';
    }
  }

  function openPicker(mode) {
    pickerMode = mode;
    startPick = [];
    pickerOpen = true;
  }

  // Stopping an agent kills a process that may be mid-task, and on a phone the
  // button is a thumb away from the chip you meant to tap — so it asks first.
  // Starting one that is already stopped destroys nothing, so it just happens.
  // One confirmation dialog for the consequential actions: stopping ONE agent
  // and closing the WHOLE project (which kills every pane in its session).
  // Both are recoverable — the declaration survives, `Open` brings it back
  // and an agent resumes its conversation — but neither should happen from a
  // mis-tap.
  // Copy for the one confirmation dialog, keyed by action. Four consequential
  // verbs share it: stop/remove an agent, close/delete a project.
  const ACT_COPY = {
    stop:   { title: 'hubStopTitle',       note: 'hubStopNote',       go: 'hubStop' },
    remove: { title: 'hubRemoveTitle',     note: 'hubRemoveNote',     go: 'hubRemove' },
    down:   { title: 'projectDownTitle',   note: 'projectDownNote',   go: 'projectDown' },
    delete: { title: 'projectDeleteTitle', note: 'projectDeleteNote', go: 'projectDelete' },
  };
  let pendingAct = $state(null);   // { kind: keyof ACT_COPY, name, session }
  let acting = $state(false);
  /** Freeze the TARGET at ask time. `name` is what the dialog shows; `session`
   * is what the action runs on. The context menu opens on ANY row, so the verb
   * must carry that row's identity — resolving `selected` at confirm time
   * closed whichever project happened to be open, not the one long-pressed
   * (owner, 2026-08-24: "关的不是我选中的 是其他的"). Agent verbs keep the
   * default: the roster only shows the selected project's agents. */
  const askAction = (kind, name, session = selected) => { pendingAct = { kind, name, session }; };

  async function runAction() {
    if (!pendingAct || acting) return;
    const { kind, name, session } = pendingAct;
    acting = true;
    try {
      if (kind === 'down' || kind === 'delete') {
        const row = rows.find((r) => r.project.session === session);
        if (row) {
          if (kind === 'delete') {
            // "Delete" is the RECYCLE BIN, not destruction (owner, 2026-08-21:
            // "把project里删掉进入archive … 在archive里可以彻底删除"): close the
            // session if one is live, then archive the declaration. Everything
            // survives — restore is one tap in the trash section, and the only
            // irreversible verb lives THERE, behind its own confirmation.
            if (row.live) await projectDown(row.project.id).catch(() => {});
            await projectArchive(row.project.id, true);
          } else {
            await projectDown(row.project.id);
          }
        }
        if (kind === 'delete' && session === selected) {
          // The OPEN project left the working list: land on whatever is left
          // rather than an empty conversation pointing at nothing. Deleting a
          // NON-selected row from its context menu moves nothing.
          selected = '';
        }
      } else if (kind === 'remove') {
        await hubAgentRemove(session, name);
      } else {
        await hubAgentStop(session, name);
      }
      await Promise.all([reload(), loadAgents(), loadFeed()]);
    } catch (e) {
      console.warn(kind === 'down' ? 'close project failed' : 'stop failed', e);
    } finally {
      acting = false;
      pendingAct = null;
    }
  }

  /** Out of the recycle bin: un-archive. Destroys nothing, so it asks nothing
   * (the same rule as restoring a message from the archive). */
  async function restoreProject(row) {
    try {
      await projectArchive(row.project.id, false);
      await reload();
    } catch (e) { console.warn('restore project failed', e); }
  }

  /** The ONLY irreversible project verb, so the only one that confirms from
   * the trash: remove the managed agents' homes and forget the declaration.
   * User files and the chat history survive even this (server contract). */
  async function purgeProject() {
    const row = trashAsk;
    trashAsk = null;
    if (!row) return;
    try {
      await projectDelete(row.project.id);
      await reload();
    } catch (e) { console.warn('purge project failed', e); }
  }

  /** Bring a stopped agent back. Same RPC as a restart — it tolerates there
   * being no window — so "start again" and "restart" are one code path, and it
   * resumes the agent's own conversation rather than opening a blank prompt. */
  async function startAgent(name) {
    if (!selected || acting) return;
    acting = true;
    try {
      await hubAgentRestart(selected, name);
      await Promise.all([reload(), loadAgents(), loadFeed()]);
      setRecipient(name);
    } catch (e) {
      console.warn('start failed', e);
    } finally {
      acting = false;
    }
  }

  // Live pushes + polling while visible.
  const onPush = (m) => {
    if (!selected || m?.room !== room(selected)) return;
    feed = mergeMessages(feed, [m]);
    lastTs = Math.max(lastTs, m.ts ?? 0);
    if (following) scrollFeed(); else newBelow = true;
    loadAgents();
  };
  $effect(() => {
    addTeamMessageListener(onPush);
    return () => removeTeamMessageListener(onPush);
  });
  $effect(() => {
    if (!visible) return;
    reload();
    registryList().then((r) => { registry = r.agents ?? []; }).catch(() => {});
    const ai = setInterval(() => { loadAgents(); loadActivity(); }, 5000);
    const fi = setInterval(loadFeed, 10000);
    const pi = setInterval(reload, 20000);
    return () => { clearInterval(ai); clearInterval(fi); clearInterval(pi); };
  });

  // Esc closes the drawer — but ONLY when it is pressed OUTSIDE the terminal.
  // Escape is how every agent TUI cancels the turn it is running, so an Esc
  // typed INTO the focused pane belongs to the pane app; stealing it at window
  // capture closed the drawer instead of reaching the agent (owner,
  // 2026-08-26: "按键盘的 ESC 键 它就直接退出这个区域了 而不是发送 ESC 键").
  // The drawer still closes via its ✕, the back gesture, and an Esc pressed
  // while focus is anywhere else. Gated on `visible` too: pages stay mounted
  // while hidden, so a drawer left open used to eat the Terminal page's Esc.
  $effect(() => {
    if (!termOpen || !visible) return;
    const onKey = (e) => {
      if (e.key !== 'Escape') return;
      if (e.target?.closest?.('.xterm')) return; // focused terminal: the pane gets it
      // Same territory rule for the files partition: an Esc from inside it
      // (editor, rename field, preview) is the browser's own — closing the
      // drawer here would UNMOUNT an open editor mid-edit.
      if (e.target?.closest?.('.files-body')) return;
      // And the board partition: its Esc order (drawer → dirty-draft confirm →
      // detail) belongs to the Board itself.
      if (e.target?.closest?.('.board-body')) return;

      closeDrawer(); e.stopPropagation();
    };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  });

  // ── The agent action menu is a CONTEXT MENU next to its chip, not a row.
  // It used to open as a bar under the roster because the roster scrolls
  // horizontally and a popover positioned inside a scroll container gets
  // clipped by it. A fixed layer escapes that container entirely, so the menu
  // can sit where the click was (owner, 2026-08-19).
  let cardsEl = $state(null);
  let menuAnchor = $state(null);   // trigger rect, in CSS px
  let menuW = $state(0);
  let menuH = $state(0);

  // ── Right-click on the desktop, long press on a phone. One menu component, one
  // piece of state, three subjects (owner, 2026-08-20: "还有很多地方增加右键点击操
  // 作，和手机长按"). The ITEMS are built per subject, and each surface offers the
  // verbs it already has elsewhere — a context menu with its own set of actions is
  // a second source of truth waiting to disagree.
  let ctxAt = $state(null);        // { x, y } in client px, or null
  let ctxItems = $state([]);
  let ctxWho = $state('');

  function openCtx(at, who, items) {
    const usable = items.filter(Boolean);
    if (!usable.length) return;
    ctxWho = who;
    ctxItems = usable;
    ctxAt = at;
  }
  const closeCtx = () => { ctxAt = null; ctxItems = []; };

  // ── The phone's BACK GESTURE, the Files page's contract (owner, 2026-08-24:
  // "chat…对于返回手势适配不太好 像是网页刷新了。像文件管理页面就很好"): App
  // routes a history pop here, and a `true` means it was CONSUMED by peeling
  // the topmost layer — the same order a tap-outside or Escape would use.
  // Only when nothing is left to peel does it fall through to App's re-push,
  // so a back never looks like the browser leaving. On a phone the project
  // LIST is the level above the conversation (the Files analogy: cwd = '/'
  // is the floor); with the list open, back has reached the floor.
  // The lightbox is the topmost transient layer: back peels it first.
  let shotView = $state('');
  $effect(() => {
    if (!onGoBack) return;
    onGoBack(() => {
      if (shotView) { shotView = ''; return true; }
      if (ctxAt) { closeCtx(); return true; }
      if (menuFor) { menuFor = ''; return true; }
      if (recipientOpen) { recipientOpen = false; return true; }
      if (palette) { paletteOff = true; return true; }
      if (intArm) { intArm = false; return true; }
      if (pendingAct && !acting) { pendingAct = null; return true; }
      if (trashAsk) { trashAsk = null; return true; }
      if (pickerOpen) { pickerOpen = false; return true; }
      if (createOpen) { createOpen = false; return true; }
      if (renaming) { renaming = false; return true; }
      if (filterAgent) { filterAgent = ''; return true; }
      if (termOpen) { closeDrawer(); return true; }
      if (compact && !sideOpen) { sideOpen = true; return true; }
      return false;
    });
  });
  /** A pointer event as a plain client point. */
  const pointOf = (e) => ({ x: e.clientX ?? 0, y: e.clientY ?? 0 });

  /** An agent's verbs — the same ones its dot menu carries. */
  /** One order for every agent menu — rising consequence, destructive last
   * (owner, 2026-08-25: "停止删除应该靠后"), interrupt in the warn tone. */
  function agentItems(name) {
    const config = openAgentConfig
      ? [{ label: t('hubAgentConfig'), icon: 'gear', onselect: () => openAgentConfig(name) }]
      : [];
    if (stopped.includes(name)) {
      return [
        { label: t('hubStartAgain'), icon: 'refresh', onselect: () => startAgent(name) },
        ...config,
        { label: t('hubRemove'), icon: 'trash', danger: true, onselect: () => askAction('remove', name) },
      ];
    }
    const a = managedAgents.find((x) => x.name === name);
    return [
      { label: t('hubTalkTo'), icon: 'chat', onselect: () => setRecipient(name) },
      { label: t('hubWatch'), icon: 'terminal', onselect: () => { if (a) openDrawer(a); } },
      ...config,
      { label: t('hubInterrupt'), icon: 'x', warn: true, onselect: () => interrupt(name) },
      { label: t('hubStop'), icon: 'stop', danger: true, onselect: () => askAction('stop', name) },
      { label: t('hubRemove'), icon: 'trash', danger: true, onselect: () => askAction('remove', name) },
    ];
  }

  /** A project's verbs. Open/Close mirrors the header's single button, so the two
   * cannot disagree about which one applies. Rename selects the project first,
   * because the editor it opens is the chat header's own title. */
  function projectItems(row) {
    const session = row?.project?.session ?? '';
    const name = row?.project?.name ?? '';
    // Constructive verbs lead, destructive close the menu (owner, 2026-08-25):
    // Open (when closed) → Rename → Close (when live) → Delete.
    return [
      ...(row?.live ? [] : [{ label: t('projectUp'), icon: 'zap', onselect: () => { selectProject(session); setTimeout(bringUp, 0); } }]),
      { label: t('projectRename'), icon: 'edit',
        onselect: () => { selectProject(session); setTimeout(startRename, 0); } },
      ...(row?.live ? [{ label: t('projectDown'), icon: 'stop', danger: true, onselect: () => askAction('down', name, session) }] : []),
      { label: t('projectDelete'), icon: 'trash', danger: true,
        onselect: () => askAction('delete', name, session) },
    ];
  }

  /** A message's verbs: the same two its own overlay offers. No delete —
   * the room is the record (owner, 2026-08-21). */
  function msgItems(m) {
    return [
      { label: t('hubCopy'), icon: 'copy', onselect: () => copyMsg(m.body) },
      { label: t('hubRaw'), icon: 'command', onselect: () => { rawOpen = rawOpen === m.id ? '' : m.id; } },
    ];
  }

  function toggleAgentMenu(name, trigger) {
    if (menuFor === name) { menuFor = ''; return; }
    // anchorOf divides the client rect by --ui-zoom: a rect is in visual px
    // while a fixed child's `left` is in its own zoomed px.
    menuAnchor = anchorOf(trigger);
    menuW = 0; menuH = 0;          // re-measure for this opening
    menuFor = name;
  }

  /** Under the trigger, right-aligned to it, flipped above when the bottom of
   * the viewport is closer than the menu is tall, and always clamped into
   * view. Measured size arrives one frame after mount, which is why the menu
   * stays invisible until it has one. The math is `menuPlacement` in hub.ts,
   * unit-tested there. */
  const menuPos = $derived.by(() =>
    menuAnchor ? menuPlacement(menuAnchor, { w: menuW, h: menuH }, viewBox()) : { x: 0, y: 0 },
  );

  // Any click elsewhere, Escape, a scroll of the roster or a resize dismisses
  // it — a menu you have to close by hand is a menu you forget to close.
  $effect(() => {
    if (!menuFor) return;
    const close = () => { menuFor = ''; };
    // .acard is the trigger now (the dots are gone): a pointerdown on a card
    // must not pre-close the menu, or the click's toggle would reopen it —
    // the toggle itself owns same-card close and other-card switch.
    const onDown = (e) => { if (!e.target?.closest?.('.a-menu, .acard:not(.add)')) close(); };
    const onKey = (e) => { if (e.key === 'Escape') { close(); e.stopPropagation(); } };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('resize', close);
    cardsEl?.addEventListener('scroll', close, { passive: true });
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey, true);
      window.removeEventListener('resize', close);
      cardsEl?.removeEventListener('scroll', close);
    };
  });

  // The SAME rule for the other two transient layers — the message action row
  // (copy/raw under a tapped bubble) and the recipient picker. Both used to
  // stay up until something happened to replace them, so a tapped message
  // kept its buttons through scrolling, composing and sending (owner,
  // 2026-08-22: "在其他操作之后应该自动隐藏 不应该一直常驻显示"). A tap
  // anywhere outside the layer (or Escape) closes them; the toggles
  // themselves and clicks INSIDE the layer are excluded so choosing an
  // option is not also "outside". Raw view is not a popup — an opened raw
  // source stays until retoggled or the project changes.
  $effect(() => {
    if (!msgOpen && !recipientOpen && !palette) return;
    const onDown = (e) => {
      const t = e.target;
      if (msgOpen && !t?.closest?.('.m-acts, .bubble')) msgOpen = '';
      if (recipientOpen && !t?.closest?.('.to-wrap')) recipientOpen = false;
      // A tap outside the composer parks the palette exactly like Escape:
      // paletteOff resets on the next text change, so typing brings it back.
      if (palette && !t?.closest?.('.cmd-menu, .compose-shell')) paletteOff = true;
    };
    const onKey = (e) => {
      if (e.key !== 'Escape') return;
      if (msgOpen) { msgOpen = ''; e.stopPropagation(); }
      if (recipientOpen) { recipientOpen = false; e.stopPropagation(); }
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey, true);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey, true);
    };
  });

  // `windowOf` is what lets a reply close the lane it belongs to, so two agents
  // working at once keep ONE growing group each instead of interleaving.
  const blocks = $derived.by(() => {
    const all = feedBlocks(feed, activity, hubPrefs.feedLevel, (from) => agents.find((a) => a.name === from)?.window);
    if (!filterAgent) return all;
    // The double-click filter: one agent's world (its replies, what was
    // addressed to it, its own telemetry lane) — rules live in filterBlocks.
    return filterBlocks(all, filterAgent, agents.find((a) => a.name === filterAgent)?.window);
  });
  const windowName = (w) => agents.find((a) => a.window === w)?.name ?? `#${w}`;

  // Disclosure lives outside the row: `undefined` means "nobody chose", and the
  // default is OPEN — what an agent is doing is the thing you came to watch
  // (owner, 2026-08-19; it used to open only while the agent was working, so a
  // finished run needed a click to read). An explicit choice sticks. Keyed by
  // group so re-renders can't lose it.
  let stepsChoice = $state({});
  let stepsAll = $state({});        // group key → lift the 10-row cap
  let menuFor = $state('');         // agent name whose dot menu is open
  let msgOpen = $state('');         // message key whose action row is open
  let rawOpen = $state('');         // message key showing its raw source
  let copied = $state('');          // body just copied, for the button label

  /** Copy a message as the agent wrote it — markdown, image refs and all. */
  async function copyMsg(body) {
    try {
      await navigator.clipboard.writeText(body ?? '');
      copied = body;
      // Show the "Copied" confirmation, then put the row away — copying IS
      // the operation the row was opened for, so it should not stay resident
      // afterwards (owner, 2026-08-22: "在其他操作之后应该自动隐藏").
      setTimeout(() => { if (copied === body) { copied = ''; msgOpen = ''; } }, 1500);
    } catch (e) { console.warn('copy failed', e); }
  }
  const isRunning = (b) =>
    b.key === newestSteps[b.window] &&
    ['running', 'working'].includes(agents.find((a) => a.window === b.window)?.state);
  const stepsOpen = (b) => stepsChoice[b.key] ?? true;
  const toggleSteps = (b, open) => { stepsChoice[b.key] = open; };

  /** Keep a capped step list showing its NEWEST row, the way a log tail does —
   * unless the user has scrolled up inside it, which is a deliberate look at
   * history and must not be yanked back. `use:stickBottom={events.length}`
   * re-runs on every appended call. */
  function stickBottom(node) {
    let stick = true;
    const onScroll = () => { stick = node.scrollHeight - node.scrollTop - node.clientHeight < 24; };
    node.addEventListener('scroll', onScroll, { passive: true });
    const toBottom = () => { if (stick) requestAnimationFrame(() => { node.scrollTop = node.scrollHeight; }); };
    toBottom();
    return { update: toBottom, destroy: () => node.removeEventListener('scroll', onScroll) };
  }
  /** Per window, the key of its LAST step group: only that one can be running. */
  const newestSteps = $derived.by(() => {
    const last = {};
    for (const b of blocks) if (b.type === 'steps') last[b.window] = b.key;
    return last;
  });
  const blockKey = (b, i) =>
    b.type === 'msg' ? (b.msg.id ?? `m${b.ts}-${i}`) : b.type === 'steps' || b.type === 'sys' ? b.key : `${b.type}${b.ts}-${i}`;

  // Notification kinds get a human label; unknown kinds fall back to the raw
  // text (t() returns the key on a miss, so detect that).
  function activityLabel(e) {
    if (e.kind !== 'notif') return e.text;
    const label = t('hubNotif_' + e.text);
    return label.startsWith('hubNotif_') ? e.text : label;
  }

  /** Four states, one word each. Anything unexpected shows itself rather than
   * being silently relabelled. */
  function stateLabel(state) {
    const label = t('hubState_' + state);
    return label.startsWith('hubState_') ? state : label;
  }

  // A clock for the elapsed readouts. One timer for the whole page, and only
  // while the tab is on screen — a "running 2m14s" that ticks in a hidden tab
  // is pure wakeups.
  // The chat/terminal divider is draggable like the sidebar's, so --hub-drawer-w
  // has to be restored the same way App restores --sidebar-w: SideHandle is the
  // only other writer.
  $effect(() => {
    const saved = parseInt(localStorage.getItem('tmux_hub_drawer_w') || '', 10);
    if (saved >= 320 && saved <= 900) {
      document.documentElement.style.setProperty('--hub-drawer-w', saved + 'px');
    }
  });

  let tick = $state(Date.now());
  $effect(() => {
    if (!visible) return;
    const id = setInterval(() => { tick = Date.now(); }, 1000);
    return () => clearInterval(id);
  });

  const fmtTime = (ts) => {
    const d = new Date(ts);
    return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  };

  /** The date separator's label: Today / Yesterday in the app's own words,
   * otherwise a local-format date (year only when it differs — a chat is
   * mostly about this week). */
  const fmtDay = (ts) => {
    const d = new Date(ts);
    const now = new Date();
    const startOf = (x) => new Date(x.getFullYear(), x.getMonth(), x.getDate()).getTime();
    const days = Math.round((startOf(now) - startOf(d)) / 86400000);
    if (days === 0) return t('hubToday');
    if (days === 1) return t('hubYesterday');
    const opts = { month: 'short', day: 'numeric', weekday: 'short' };
    if (d.getFullYear() !== now.getFullYear()) opts.year = 'numeric';
    return d.toLocaleDateString(i18n.lang === 'zh' ? 'zh-CN' : 'en-US', opts);
  };

  /** Vertical wheel pans the header path horizontally — the path scrolls
   * instead of ellipsizing, and a mouse has no horizontal axis of its own.
   * An action (not `onwheel`) because preventDefault needs a non-passive
   * listener. */
  function wheelX(el) {
    const onWheel = (e) => {
      if (!e.deltaY || e.deltaX) return; // trackpads already pan natively
      if (el.scrollWidth <= el.clientWidth) return; // it fits — let the page have the wheel
      el.scrollLeft += e.deltaY;
      e.preventDefault();
    };
    el.addEventListener('wheel', onWheel, { passive: false });
    return { destroy: () => el.removeEventListener('wheel', onWheel) };
  }
</script>

<div class="hub-root" class:compact class:drawer-open={termOpen && !compact}>
  <div class="cols">
    <!-- ── Projects. A column on the desktop; on the phone the SAME list slides
         in from the left, because these are separate conversations you pick
         between, not tabs you flick through. ─────────── -->
    {#if compact && sideOpen}
      <div class="side-scrim" onclick={() => sideOpen = false} role="presentation"></div>
    {/if}
    <!-- side-sheet is the SHARED drawer dialect (app.css): inert on desktop,
         the parked-and-sliding sheet on the phone — one width, one shadow,
         one motion for Chat, Terminal and Board (owner, 2026-08-30). -->
    <aside class="sidebar" class:side-sheet={compact} class:sheet={compact} class:open={compact && sideOpen}>
      {#if !compact}<SideHandle />{/if}
      <div class="side-scroll subtle-scroll" use:scrollFade>
        <div class="side-h">{t('hubProjects')}</div>
        {#each rows as row (row.project.id)}
          <!-- Right-click (desktop) and long press (phone) open the project's own
               verbs where the pointer is. The row's normal job — open this
               conversation — is unchanged. -->
          <button class="side-row proj-row" class:open={row.project.session === selected}
            onclick={() => { selectProject(row.project.session); sideOpen = false; }}
            oncontextmenu={(e) => { e.preventDefault(); openCtx(pointOf(e), row.project.name, projectItems(row)); }}
            use:longpress={{ onlongpress: (pt) => openCtx(pt, row.project.name, projectItems(row)) }}>
            <span class="dot" class:off={!row.live}></span>
            <span class="p-main">
              <span class="p-top">
                <span class="p-name">{row.project.name}</span>
                {#if rowTalk(row)}<span class="side-age">{agoShort(rowTalk(row), tick)}</span>{/if}
              </span>
              {#if rowAgents(row).length}
                <span class="side-wins" class:dim={!row.live}>
                  {#each rowAgents(row) as a (a.name)}
                    <span class="side-win">
                      {#if a.icon}<img src={a.icon} alt="" width="11" height="11" />{/if}
                      <span class="side-win-name">{a.name}</span>
                      {#if a.state}<span class="side-win-dot" class:live-dot={stateIsLive(a.state)} style:background={stateDotColor(a.state)}></span>{/if}
                    </span>
                  {/each}
                </span>
              {/if}
            </span>
          </button>
        {/each}
        <button class="side-row add" onclick={() => { createOpen = true; sideOpen = false; }}>
          <Icon name="plus" size={13} />{t('projectNew')}
        </button>
        <!-- ── The recycle bin. "Delete" moves a project here (session closed,
             declaration kept); this folded section is the way back — restore
             is one tap and asks nothing — and the way OUT: permanent delete
             lives only here, behind the one confirmation that means it
             (owner, 2026-08-21: "相当于回收站的功能，在archive里可以彻底删除
             project"). Hidden entirely while empty: an empty bin is not a
             place to visit. -->
        {#if trash.length}
          <button class="side-row add trash-bar" onclick={() => trashOpen = !trashOpen}>
            <Icon name={trashOpen ? 'chevron-down' : 'trash'} size={13} />
            {t('hubTrashBar').replace('{n}', String(trash.length))}
          </button>
          {#if trashOpen}
            {#each trash as r (r.project.id)}
              <div class="side-row trash-row" title={r.project.path}>
                <span class="p-name trash-name">{r.project.name}</span>
                <button class="t-act" title={t('hubRestore')} aria-label={t('hubRestore')}
                  onclick={() => restoreProject(r)}>
                  <Icon name="refresh" size={12} />
                </button>
                <button class="t-act danger" title={t('hubPurge')} aria-label={t('hubPurge')}
                  onclick={() => trashAsk = r}>
                  <Icon name="trash" size={12} />
                </button>
              </div>
            {/each}
          {/if}
        {/if}
      </div>
    </aside>

    <!-- ── Main: the conversation ─────────── -->
    <main class="mid">
      <div class="page-head">
        <!-- The phone reaches the project list here, as a drawer. No chip strip:
             separate conversations are chosen deliberately, not flicked past. -->
        {#if compact}
          <button class="icon-btn" title={t('hubProjects')} onclick={() => sideOpen = true}>
            <Icon name="menu" size={17} />
          </button>
        {/if}
        <!-- The title IS the rename control: a project's name is the one thing
             in this header you might want to change, and a second pencil button
             would be a duplicate of the thing it edits. -->
        <!-- The title IS the rename control. It carries a visible pencil,
             because the first version relied on a hover underline and the owner
             could not find the feature at all (2026-08-19) — and hover does not
             exist on a phone, where this page mostly lives. The icon sits INSIDE
             the title button, so it is a hint on the thing it edits rather than
             a second control next to it. -->
        <!-- The ⋯ belongs TO the name, so the two are ONE group and the row
             gap falls OUTSIDE it: 3px between the name and its menu, the
             header's own spacing between this group and the path/spacer
             (owner, 2026-08-30: "离 project name 还是有点远，可以直接紧挨着
             name，让人觉得是可以点击操作的").

             A wrapper rather than the two alternatives: a negative margin
             cancelling the row gap would be wrong arithmetic the moment that
             gap changes (it is 10px here, 7px compact), and moving the button
             INSIDE the h1 would put it under the heading's `overflow: hidden`,
             which clips the invisible ~42px tap overlay the compact rule adds
             — the affordance would read closer and be harder to hit. The group
             also sidesteps the phone's shared `.page-head h1 { flex: 1 1 auto }`
             without fighting it: the h1 may still stretch, but only inside a
             group that is itself content-sized, so the ⋯ stays against the
             name instead of being parked at the far right. -->
        <div class="title-group">
          {#if renaming}
            <input class="h1-edit" bind:this={renameEl} bind:value={renameDraft}
              aria-label={t('projectRename')} maxlength="80"
              onkeydown={(e) => {
                if (e.key === 'Enter') { e.preventDefault(); commitRename(); }
                else if (e.key === 'Escape') { e.preventDefault(); renaming = false; }
              }}
              onblur={commitRename} />
          {:else}
            <h1>
              <!-- The NAME is text, not a control. Making the whole title clickable
                   meant every attempt to select it, or a stray tap on the way to
                   something else, opened an editor ("不应该点击名字都是改名，只有点击
                   右边小图标才是改名", owner 2026-08-20). The pencil beside it is the
                   rename affordance, and it is a real button so assistive tech and
                   Enter/Space come for free. -->
              <span class="h1-text" bind:this={titleNameEl}>{selectedRow?.project.name ?? ''}</span>
            </h1>
          {/if}
          {#if selected}
            <!-- The project's own verbs — Open/Rename/Close/Delete — as ONE
                 second-level menu (owner, 2026-08-29), glued to the name it
                 acts on. The affordance is a DOWNWARD CHEVRON, tight against
                 the name — it reads as "expand this name for its options",
                 the dropdown grammar everyone already knows (owner,
                 2026-08-30: "换成一个向下的直角箭头…有点像把这个名字展开给
                 出的下拉标签选项，这样对于人类更好理解"). A SIBLING of the
                 h1, inside the group: within the heading, its
                 overflow:hidden would clip the compact 42px tap overlay.
                 Opens the SAME projectItems menu the sidebar row's
                 long-press/right-click speaks. -->
            <button class="icon-btn title-caret" title={t('hubProjectMenu')} aria-label={t('hubProjectMenu')}
              onclick={(e) => {
                // The menu is the NAME expanding downward (board #32): its
                // LEFT edge sits on the name's own left edge, measured from
                // the name element's REAL rect (anchorOf carries the zoom
                // correction), so a wide menu near the right edge stops
                // clipping — the placement clamp still owns the viewport
                // edges. While renaming, the span is an input and the caret
                // itself anchors. Every other context menu keeps the
                // right-aligned pointer default.
                openCtx({ anchor: anchorOf(titleNameEl ?? e.currentTarget), align: 'left' },
                  selectedRow?.project.name ?? '', projectItems(selectedRow));
              }}>
              <Icon name="chevron-down" size={14} />
            </button>
          {/if}
        </div>
        <!-- The FULL path, not a middle-elided stub: it renders whole when it
             fits, and when it doesn't the box scrolls (wheel included) instead
             of ellipsizing — "不要直接用省略号" (owner, 2026-08-20). Desktop
             only, same as before. -->
        {#if !compact}<span class="path" use:wheelX title={selectedRow?.project.path ?? ''}>{selectedRow?.project.path ?? ''}</span>{/if}
        <span class="spacer"></span>
        <!-- Header actions are ONE dialect: icon-only .icon-btn, the label on
             hover via title (and aria-label for readers). Mixed text-and-icon
             chips read as three different kinds of control (owner, 2026-08-25:
             "删除 关闭 命令按钮 都不统一 有的文字 有的图案，可以改成图案 鼠标
             悬停显示按钮文字"). Icons match the project context menu's verbs:
             zap=up, stop=down, trash=delete. -->
        <!-- The task board: on the phone this jumps to the board PAGE (owner,
             2026-08-29: "board单独作为一个独立的功能的页面"); on desktop it is
             the drawer's THIRD partition ("或者右侧边栏有这个任务侧边栏", same
             day) — exactly the files toggle's split. -->
        <button class="icon-btn term-toggle" class:on={termOpen && drawerView === 'board' && !compact} title={t('board')} aria-label={t('board')}
          onclick={() => {
            if (mobile || compact) { openBoardTab?.(selected); return; }
            if (termOpen && drawerView === 'board') { closeDrawer(); } else { drawerView = 'board'; openDrawer(); }
          }}>
          <Icon name="layout" size={14} />
        </button>
        <!-- The drawer's second partition. On the phone (no drawer) the same
             button JUMPS to the Files tab — exactly what the terminal toggle
             does with the Terminal tab (owner, 2026-08-28: "手机上好像没有
             打开文件侧边栏的按钮"). -->
        <button class="icon-btn term-toggle" class:on={termOpen && drawerView === 'files' && !compact} title={t('files')} aria-label={t('files')}
          onclick={() => {
            if (mobile || compact) { openFilesTab?.(selected, drawerFilesDir); return; }
            if (termOpen && drawerView === 'files') { closeDrawer(); } else { drawerView = 'files'; openDrawer(); }
          }}>
          <Icon name="files" size={14} />
        </button>
        <!-- THE terminal affordance: a button, not a permanent pane. Adding an
             agent belongs to the roster row, and chat detail belongs to
             Settings — a header is not a place to keep spare switches. -->
        <button class="icon-btn term-toggle" class:on={termOpen && drawerView === 'term'} title={t('hubTerminal')} aria-label={t('hubTerminal')}
          onclick={() => termOpen && drawerView === 'term' && !compact ? closeDrawer() : (drawerView = 'term', openDrawer())}>
          <Icon name="terminal" size={14} />
        </button>
      </div>

      {#if selected}
        <!-- The roster. Tapping an agent makes it the recipient (and this
             project's lead) — the phone gets chips, the desktop gets cards. -->
        <!-- The roster row: one scrolling strip, the add button riding INSIDE
             it as the sticky last card. It renders for every SELECTED
             project, empty roster and closed session included, because `+ agent`
             is the only way into an empty room and both gates hid it: on a
             non-empty-roster gate it vanished with the last agent, and on a
             live-session gate a CLOSED project still had none (owner, 2026-08-24,
             twice — "test 这个 project"). A closed session is not a real
             constraint either: `hub_spawn` → `projects::spawn` calls
             `tmux::ensure_session` itself, so spawning into a project that is
             down OPENS it. -->
        <div class="roster">
        <div class="cards" class:chips={compact} bind:this={cardsEl}>
          {#each managedAgents as a (a.window)}
            <!-- A div, not a button: the dot menu inside contains real buttons,
                 and a button inside a button is invalid HTML the browser
                 silently reshuffles. -->
            <!-- ONE tap surface: the whole card opens the agent menu — the
                 dots duplicated it for no gain ("交互上就是直接点击卡片出来
                 选项就行，好像没必要单独点击三个点", owner 2026-08-25).
                 And the tap ALSO makes this agent the recipient: tapping a
                 card means "I want to talk to this one", so the conversation
                 switches without hunting for the menu's first item (owner,
                 2026-08-26: "每次点击 project 里的 Agent 小卡片时 能自动帮我
                 切换到跟当前 Agent 的对话"). -->
            <div class="acard" class:sel={recipient === a.name} role="button" tabindex="0"
              title={[`${a.name} · ${stateLabel(a.state)}`, a.detail, vitalsLine(a.vitals)].filter(Boolean).join(' · ')}
              onclick={(e) => cardClick(a.name, e.currentTarget)}
              ondblclick={() => cardDbl(a.name)}
              oncontextmenu={(e) => { e.preventDefault(); openCtx(pointOf(e), a.name, agentItems(a.name)); }}
              use:longpress={{ onlongpress: (pt) => openCtx(pt, a.name, agentItems(a.name)) }}
              onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); cardClick(a.name, e.currentTarget); } }}>
              <div class="ac-top">
                {#if backendIcon(a.agent)}<img class="ava" src={backendIcon(a.agent)} alt={a.agent} />{:else}<span class="ava" style:background={backendColor(a.agent)}>{a.name.slice(0, 1).toUpperCase()}</span>{/if}
                <span class="a-name">{a.name}</span>
                <span class="st" class:live-dot={stateIsLive(a.state)} style:background={stateDotColor(a.state)}></span>
                {#if unread.has(a.name)}<span class="unread" title={t('hubUnread')}></span>{/if}
              </div>
              <!-- What the agent's own status line says, kept ON the card rather
                   than behind the menu ("这个直接常驻显示吧 可以字号小一点"). The
                   MODEL only: effort shows on tap in the menu header (owner,
                   2026-08-26), and the branch is the same for every agent in a
                   project, so on a card it is noise. -->
              {#if cardVitals(a.vitals)}<div class="ac-vitals">{cardVitals(a.vitals)}</div>{/if}
              <!-- Context used as a thin colour-changing line at the card's own
                   bottom edge ("百分比用一个细长会变颜色的进度条示意 一个细横线就
                   行"): a percentage you read at a glance, costing no row and no
                   vertical space. The exact number stays in the tooltip and the
                   menu header, where a number is what you came for. -->
              {#if a.vitals?.context_pct != null}
                <div class="ac-bar" title={`${a.vitals.context_pct}% · ${t('hubCtxUsed')}`}>
                  <i style:width="{a.vitals.context_pct}%" style:background={ctxColor(a.vitals.context_pct)}></i>
                </div>
              {/if}
            </div>
          {/each}
          <!-- Stopped agents: declared by the project, no window right now.
               Starting one resumes its conversation, so it stays on the roster
               instead of vanishing from the room it belongs to. -->
          {#each stopped as name (name)}
            <!-- A div for the same reason as the live card: the dot menu inside
                 holds real buttons. The card SURFACE is inert — restarting is
                 the refresh button's job alone (owner, 2026-08-24: "已经停止的
                 agent我只要点击就自动重启了 并没有点到重启的那个圆圈箭头上" —
                 a card-wide click restarted agents by accident). Removing it
                 lives in the menu, because a stopped agent you are done with
                 has to be ejectable — the slot is what keeps `up` recreating
                 it (owner, 2026-08-19). -->
            <div class="acard off" class:busy={acting} role="button" tabindex="0" aria-label={name}
              onclick={(e) => toggleAgentMenu(name, e.currentTarget)}
              onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleAgentMenu(name, e.currentTarget); } }}
              oncontextmenu={(e) => { e.preventDefault(); openCtx(pointOf(e), name, agentItems(name)); }}
              use:longpress={{ onlongpress: (pt) => openCtx(pt, name, agentItems(name)) }}>
              <div class="ac-top">
                <span class="ava dim">{name.slice(0, 1).toUpperCase()}</span>
                <span class="a-name">{name}</span>
                <span class="s-age">{t('hubStopped')}</span>
                <button class="a-start" title={t('hubStartAgain')} aria-label={t('hubStartAgain')}
                  disabled={acting}
                  onclick={(e) => { e.stopPropagation(); startAgent(name); }}>
                  <Icon name="refresh" size={11} />
                </button>
              </div>
            </div>
          {/each}
        <!-- Ad hoc: add an agent to a conversation already in progress. It
             lives INSIDE the strip as its last card — one region, one family
             (owner, 2026-08-25: "加agent应该放到最后，和其他agent放到一起…
             不用强行一直占一个位置") — but STICKY at the right edge, because
             the strip hides its scrollbar and a plainly-scrolling last child
             was invisible with agents present (owner, 2026-08-21: "agent 不为
             空的情况下 我都看不到'加 Agent'的按钮"). Sticky is both at once:
             it sits after the last card when everything fits, and floats at
             the edge while the strip scrolls. Icon-only when agents exist;
             the label only when the roster is empty and the button IS the
             room's entry point (a closed session is not a reason to hide it —
             the spawn opens the session on its way in). -->
          <button class="acard add" class:mini={managedAgents.length > 0 || stopped.length > 0}
            onclick={() => openPicker('add')} title={t('hubSpawn')} aria-label={t('hubSpawn')}>
            <Icon name="plus" size={14} />{#if !managedAgents.length && !stopped.length}<span>{t('hubSpawn')}</span>{/if}
          </button>
        </div>
        </div>
      {/if}

      {#if menuFor}
        <!-- Actions for one agent, as a context menu beside its chip. It is a
             FIXED layer: the roster scrolls horizontally, and a popover inside
             that scroll container would be clipped by it (which is why this was
             a full-width bar under the roster until 2026-08-19). Same popover
             dialect as the recipient menu — one menu language in this file.

             A stopped agent gets the two verbs that mean anything to it: start
             it again, or eject it. Watch/Interrupt/Stop all need a live pane. -->
        <div class="a-menu" class:ready={menuH > 0} role="menu" tabindex="-1"
          style:left="{menuPos.x}px" style:top="{menuPos.y}px"
          bind:clientWidth={menuW} bind:clientHeight={menuH}>
          <div class="am-who">{menuFor}</div>
          <!-- State + running time live HERE, not on the card: the elapsed
               counter earned its glance-value in the tooltip and menu, and a
               ticking number on every card was permanent motion the roster
               did not need (owner, 2026-08-25: "运行时间…没太必要常显示，
               可以点击三个点显示就行"). -->
          {#if menuAgent}
            <div class="am-vitals">{stateLabel(menuAgent.state)}{menuAgent.since ? ` · ${fmtElapsed(menuAgent.since, tick)}` : ''}</div>
          {/if}
          {#if vitalsFor}<div class="am-vitals">{vitalsFor}</div>{/if}
          <!-- Rising order of consequence, colours saying which is which
               (owner, 2026-08-25: "停止 删除 打断等颜色不一样…停止删除应该
               靠后"): reading verbs first, then config, then amber interrupt
               (a turn cut short — the sys grammar's colour), and the red
               stop/remove close the menu. -->
          {#if stopped.includes(menuFor)}
            <button role="menuitem" disabled={acting} onclick={() => { const n = menuFor; menuFor = ''; startAgent(n); }}>
              <Icon name="refresh" size={12} />{t('hubStartAgain')}
            </button>
          {:else}
            <!-- Choosing the recipient used to be the card's own click; the
                 card opens this menu now, so the verb lives here, first. -->
            <button role="menuitem" onclick={() => { const n = menuFor; menuFor = ''; setRecipient(n); }}>
              <Icon name="chat" size={12} />{t('hubTalkTo')}
            </button>
            <button role="menuitem" onclick={() => { const a = managedAgents.find((x) => x.name === menuFor); menuFor = ''; if (a) openDrawer(a); }}>
              <Icon name="terminal" size={12} />{t('hubWatch')}
            </button>
          {/if}
          {#if openAgentConfig}
            <!-- The model's HOME is the config page — the menu links to it
                 instead of quoting the model name as dead text. -->
            <button role="menuitem" onclick={() => { const n = menuFor; menuFor = ''; openAgentConfig(n); }}>
              <Icon name="gear" size={12} />{t('hubAgentConfig')}
            </button>
          {/if}
          {#if !stopped.includes(menuFor)}
            <button role="menuitem" class="warn" title={t('hubInterruptHint')} onclick={() => { const n = menuFor; menuFor = ''; interrupt(n); }}>
              <Icon name="x" size={12} />{t('hubInterrupt')}
            </button>
            <button role="menuitem" class="danger" onclick={() => { const n = menuFor; menuFor = ''; askAction('stop', n); }}>
              <Icon name="stop" size={12} />{t('hubStop')}
            </button>
          {/if}
          <button role="menuitem" class="danger" title={t('hubRemoveHint')} onclick={() => { const n = menuFor; menuFor = ''; askAction('remove', n); }}>
            <Icon name="trash" size={12} />{t('hubRemove')}
          </button>
        </div>
      {/if}

      <div class="feed-wrap">
      <div class="feed subtle-scroll" bind:this={feedEl} onscroll={onFeedScroll}>
        <!-- The double-click filter is a MODE, so it says so (board #3, owner:
             "注意ui上体现我们现在的筛选状态，以及可以再退出"): a compact pill
             INSIDE the feed — as a feed-wrap sibling it became a full-height
             left COLUMN (feed-wrap is row flex; reopened #3). Sticky at the
             top so the mode stays visible while reading; content-width. -->
        {#if filterAgent}
          <div class="filter-pill">
            <Icon name="search" size={12} />
            <span class="f-label">{t('hubFilterOn')}</span>
            <span class="f-name">@{filterAgent}</span>
            <button class="icon-btn" title={t('hubFilterExit')} aria-label={t('hubFilterExit')} onclick={() => (filterAgent = '')}>
              <Icon name="x" size={13} />
            </button>
          </div>
        {/if}
        <!-- Low-presence paging feedback at the very top: fetching, or the
             confirmed beginning once both walks are parked (board #9). -->
        {#if roomReady && (loadingOlder || (!histMore && !actMore))}
          <div class="older-hint">{loadingOlder ? t('hubOlderLoading') : t('hubFeedStart')}</div>
        {:else if roomReady && (histMore || actMore)}
          <!-- A first page shorter than the viewport never fires a scroll
               event, so the walk needs a hand-hold too (#9 review). Same
               whisper voice; scrolling near the top still auto-loads. -->
          <button class="older-hint older-more" onclick={loadOlder}>{t('hubOlderMore')}</button>
        {/if}
        {#each blocks as b, i (blockKey(b, i))}
          <!-- A new calendar day gets a centred date pill before its first
               block — the times alone never said WHICH day a message was from
               (owner, 2026-08-20). Local-time days (sameDay), Today/Yesterday
               in the app's words. Not sticky: a pinned rect would fight the
               ask-anchor's edge math. -->
          {#if i === 0 || !sameDay(blocks[i - 1].ts, b.ts)}
            <div class="day-sep" aria-hidden="true"><span class="day-pill">{fmtDay(b.ts)}</span></div>
          {/if}
          {#if b.type === 'sys'}
            <!-- The app's own record (spawn/stop/restart, a /command typed into
                 a pane). Consecutive lines still fold into ONE capsule — a stop
                 plus its restart is one fact — and every line speaks ONE grammar:
                 who it is about, what happened, the detail ("都用统一的 ui 来展示
                 …agent 的名字，状态，或者发送的指令", owner 2026-08-24). The name
                 wears the bubble header's ink, the action wears the status-note
                 badge dialect (dot + word, sysVerbColor), and a /command's badge
                 is the command itself in the composer's monospace — badge + args
                 read back as exactly the line that was typed. Hidden entirely at
                 the chat-only level (feedBlocks drops them). -->
            <div class="sysline">
              {#each b.items as item, j (`${j}-${item}`)}
                {@const bl = boardLine(item)}
                {#if bl}
                  <!-- A board move in the sys grammar's own atoms (board #13):
                       the issue number is the WHO, the destination status the
                       coloured badge (one progressive status language — review
                       is amber because it WAITS for a person), the title the
                       detail. The FROM stays visible because the transition is
                       the message: done → todo reads as a REOPEN. The row is a
                       BUTTON: tapping it jumps to that issue on the board page
                       (same route the header's layout icon takes). -->
                  <button class="sys-item sys-jump" title={t('board')}
                    onclick={() => {
                      if (mobile || compact) { openBoardTab?.(selected, Number(bl.id)); return; }
                      drawerIssueReq = { session: selected, id: Number(bl.id), n: (drawerIssueReq?.n ?? 0) + 1 };
                      drawerView = 'board'; openDrawer();
                    }}>
                    <span class="sys-who">#{bl.id}</span>
                    <span class="sys-from">{t(`boardStatus_${bl.from}`)} →</span>
                    <span class="sys-verb" style:color={boardStatusColor(bl.to)}><span class="sv-dot" aria-hidden="true"></span>{t(`boardStatus_${bl.to}`)}</span>
                    {#if bl.title}<span class="sys-text">{bl.title}</span>{/if}
                  </button>
                {:else}
                {@const p = sysParts(item)}
                {@const c = sysVerbColor(p.verb)}
                <div class="sys-item">
                  {#if p.who}<span class="sys-who">{p.who}</span>{/if}
                  {#if p.cmd}
                    <!-- The typed line is ONE object in the composer's own
                         command costume — splitting it into a micro-pill name
                         plus loose args at another size broke it into fragments
                         ("带参数的渲染好像不是很好", owner 2026-08-24). -->
                    <span class="sys-cmd">{p.text ? `${p.verb} ${p.text}` : p.verb}</span>
                  {:else}
                    {#if p.verb}
                      <span class="sys-verb" style:color={c}><span class="sv-dot" aria-hidden="true"></span>{p.verb}</span>
                    {/if}
                    {#if p.text}<span class="sys-text">{p.text}</span>{/if}
                  {/if}
                </div>
                {/if}
              {/each}
            </div>
          {:else if b.type === 'msg'}
            {@const m = b.msg}
            <!-- An agent's note about its own work — a `tmm status` note or a
                 `tmm done` summary. Only the MARKER is client-side: it is a message
                 from the agent, so it wears exactly the same bubble as any other
                 ("status 消息的样式要和普通消息一样就行", owner 2026-08-19). A
                 second visual species for the same thing is what made it read as
                 telemetry in the first place. -->
            {@const note = statusNote(m.body)}
            {@const parts = splitImages(note ? note.text : m.body)}
              <!-- Every user message can become the landmark, but exactly ONE
                   does. The real bubble enters with the feed, then that SAME
                   element catches the edge as it is about to leave; there is no
                   duplicate and no invisible midpoint swap. -->
              {@const key = blockKey(b, i)}
              {@const isAsk = m.from === 'human'}
              <!-- Folding is a property of the TEXT, and the DEFAULT for every
                   long user message (owner, 2026-08-27: "默认用户消息都截断 不要
                   显示太多"): the bubble renders a rear-truncated body until the
                   reader unfolds it by hand. -->
              {@const foldable = isAsk && foldBody(parts.text) !== parts.text}
              {@const folded = foldable && !expanded[key]}
              <!-- An EXPANDED message is never pinned: sticky ignores the feed's
                   scrolling, so a pinned screen-tall message had an unreachable
                   bottom half (owner, 2026-08-27: "如果展开了消息 就要把钉住用户
                   消息关掉 不然展开就没法上下滑动了" — which retired the held-scroll
                   in-body scroller, 2026-08-20's answer to the same problem: the
                   feed itself is the scroller now). It rejoins the anchor pool
                   when folded again. -->
              {@const pinned = isAsk && askKey === key && !expanded[key]}
              <div class="msg" class:me={m.from === 'human'}
                class:ask-top={pinned && askEdge === 'top'}
                class:ask-bottom={pinned && askEdge === 'bottom'}
                class:held={pinned && askHeld}
                data-ask={isAsk ? key : undefined}>
                <!-- Telegram-style bubble: agent name heads the bubble; the
                     time — and on your own messages the delivery ring, right
                     of it — is an inline trailer FLOATED at the end of the
                     text, sharing the last line when it fits and dropping to
                     its own right-aligned line when it doesn't. Never a
                     separate row or column outside the bubble. -->
                <!-- The bubble is TEXT to assistive tech (role="button" made
                     every message announce as one giant button and Tab walk
                     the whole transcript); its click is a pointer convenience.
                     The accessible path to copy/raw is the meta-trailer
                     button below. -->
                <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
                <div class="bubble md"
                  oncontextmenu={(e) => { e.preventDefault(); openCtx(pointOf(e), m.from, msgItems(m)); }}
                  onclick={() => { msgOpen = msgOpen === key ? '' : key; }}>
                  {#if m.from !== 'human'}
                    <!-- A status note keeps the ordinary bubble, but its header
                         says what the words are ABOUT. The first cut was
                         `name → state`, and the arrow read as an ADDRESSEE —
                         "像是这个 Agent 给另外一个 working 的人发的" (owner,
                         2026-08-20) — so the state is now a BADGE in the app's
                         existing state-pill dialect (.pg-tag): a dot + the
                         state word in its own status colour, which reads as
                         "entered this state", not "sent to working". -->
                    <div class="m-head">{m.from}{#if note}<span class="m-note-state" style:color={noteStateColor(note.state)}><span class="mns-dot" aria-hidden="true"></span>{stateLabel(note.state)}</span>{/if}</div>
                  {/if}
                  <div class="m-body">
                    {#if parts.text}
                      {#if rawOpen === key}
                        <pre class="raw">{m.body}</pre>
                      {:else}
                        <!-- Folded: the start of the message, cut where the
                             budget runs out, …… glued to the last kept line.
                             Raw view and agent messages render in full. -->
                        {@html markLeadingMention(renderMarkdown(folded ? foldBody(parts.text) : parts.text))}
                      {/if}
                      {#if folded}
                        <!-- The way to the whole message. A button, because this
                             is the one thing you might want from a folded
                             message; the bubble's own click still opens copy/raw. -->
                        <button class="m-unfold" onclick={(e) => { e.stopPropagation(); expandMsg(key); }}>
                          <Icon name="chevron-down" size={11} />{t('hubUnfold')}
                        </button>
                      {:else if foldable}
                        <button class="m-unfold" onclick={(e) => { e.stopPropagation(); const { [key]: _gone, ...rest } = expanded; expanded = rest; }}>
                          <Icon name="chevron-up" size={11} />{t('hubRefold')}
                        </button>
                      {/if}
                    {/if}
                    {#if parts.images.length}
                      <!-- Inside the bubble (owner, 2026-08-26): part of the
                           message, clipped by the bubble's own radius. The
                           held anchor hides these — a pinned landmark is for
                           re-reading your words, not for a tall image. -->
                      <div class="shots">
                        {#each parts.images as src, k (`${k}-${src}`)}
                          <ChatImage {src} alt={m.from} onview={(u) => shotView = u} />
                        {/each}
                      </div>
                    {/if}
                    <button class="m-meta" aria-label={t('hubMsgActions')}
                      onclick={(e) => { e.stopPropagation(); msgOpen = msgOpen === key ? '' : key; }}>
                      <span class="m-time">{fmtTime(m.ts)}</span>
                      {#if m.from === 'human'}
                        <!-- Hollow does not mean failed: a busy agent QUEUES the
                             line and accepts it when its turn ends, so the two
                             states need two different explanations. -->
                        <span class="m-state" class:ok={b.delivered}
                          title={b.delivered ? t('hubDeliveredHint') : t('hubPendingHint')}>
                          <Icon name={b.delivered ? 'circle-check' : 'circle'} size={11} />
                        </span>
                      {/if}
                    </button>
                  </div>
                </div>
                {#if msgOpen === key}
                  <div class="m-acts">
                    <button onclick={() => copyMsg(m.body)}>
                      <Icon name="copy" size={11} />{copied === m.body ? t('hubCopied') : t('hubCopy')}
                    </button>
                    <button class:on={rawOpen === key} onclick={() => { rawOpen = rawOpen === key ? '' : key; }}>
                      <Icon name="command" size={11} />{t('hubRaw')}
                    </button>
                  </div>
                {/if}
              </div>
          {:else if b.type === 'prompt'}
            <!-- The input half: what this agent was asked, which only the
                 userPromptSubmit hook can tell us. -->
            {@const pp = promptParts(b.text)}
            <div class="prompt">
              <!-- The machine stamp comes OFF (owner, 2026-08-30): the sender
                   joins the head, and a board delivery wears the board
                   dialect — issue chip (+ review badge) — instead of raw
                   log text. -->
              <div class="p-head"><span class="p-who">{windowName(b.window)}</span><span class="p-tag">{t('hubPromptIn')}</span>{#if pp.from}<span class="p-from">{pp.from}</span>{/if}<span>{fmtTime(b.ts)}</span></div>
              <div class="p-body">
                {#if pp.board}<span class="p-chip">#{pp.board.id}</span>{#if pp.board.review}<span class="p-badge" style:color={boardStatusColor('review')}><span class="pb-dot" aria-hidden="true"></span>{t('boardStatus_review')}</span>{/if}{/if}{pp.text}
              </div>
            </div>
          {:else if b.type === 'progress'}
            <!-- What the agent says it is doing (`tmm status <state> "note"`).
                 Hooks can see that a turn is open, never what it is about, so
                 this is the only account of work in progress — it reads as a
                 line the agent spoke, not as a telemetry row, and a blocked or
                 waiting note carries the colour of something that needs a
                 human. -->
            <div class="prog" class:blocked={b.state === 'blocked' || b.state === 'waiting'}>
              <span class="pg-bar" aria-hidden="true"></span>
              <span class="pg-who">{windowName(b.window)}</span>
              {#if b.state && b.state !== 'working'}<span class="pg-tag">{stateLabel(b.state)}</span>{/if}
              <span class="pg-text">{b.text}</span>
              <span class="pg-ts">{fmtTime(b.ts)}</span>
            </div>
          {:else if b.type === 'note'}
            <div class="note" class:warn={b.event.kind === 'warn'}>
              {#if b.event.kind === 'warn'}<Icon name="info" size={11} />{/if}
              <span class="n-who">{windowName(b.window)}</span>
              <span class="n-text">{activityLabel(b.event)}</span>
              <span class="n-ts">{fmtTime(b.ts)}</span>
            </div>
          {:else}
            <!-- Tool calls between two replies: one collapsible run per window.
                 Open while the agent is working, closed once it is done, unless
                 the user has said otherwise for this group. -->
            {@const open = stepsOpen(b)}
            <div class="steps" class:open>
              <button class="s-head" aria-expanded={open} onclick={() => toggleSteps(b, !open)}>
                {#if isRunning(b)}
                  <span class="s-live live-dot" aria-hidden="true"></span>
                {:else}
                  <span class="chev" class:open><Icon name="chevron-right" size={12} /></span>
                {/if}
                <span class="s-who">{windowName(b.window)}</span>
                <span class="s-count">{t('hubStepsN').replace('{n}', String(b.events.length))}</span>
                {#if !open}
                  {@const last = b.events[b.events.length - 1]}
                  {@const lp = last ? toolEventParts(last) : { tool: '', text: '' }}
                  <span class="s-peek">{#if lp.tool}<span class="tname" style:color={toolColor(lp.tool)}>{lp.tool}</span> {/if}{lp.text}</span>
                {/if}
              </button>
              {#if open}
                {@const capped = !stepsAll[b.key] && b.events.length > hubPrefs.stepsRows}
                <!-- Every call is in the DOM; the CAP is a viewport on it, so a
                     live run stops growing the conversation after the configured
                     rows (hubPrefs.stepsRows) and the tail stays where the eye
                     already is. -->
                <div class="s-body" class:capped style:--steps-rows={hubPrefs.stepsRows}
                  use:stickBottom={capped ? b.events.length : 0}>
                  {#each b.events as e, j (`${e.ts}-${j}`)}
                    {@const ep = toolEventParts(e)}
                    <div class="step">
                      <!-- The tool NAME is the scannable half: its own colour by
                           what the tool does. toolEventParts splits the name off
                           legacy events that glued it onto the text — those were
                           the "still grey" rows. -->
                      {#if ep.tool}<span class="tname" style:color={toolColor(ep.tool)}>{ep.tool}</span>{/if}
                      <!-- The argument is the ONLY scrolling cell: the name and
                           the time are ordinary flex children BESIDE it, so the
                           panning text is clipped by this box and structurally
                           cannot show through them or slide past the lane's edge.
                           The sticky-column build could not guarantee that — a
                           sticky column covers its own box but not the lane
                           padding beside it, which is where the text bled through
                           (owner, 2026-08-20: "参数穿模到工具名左侧了"). -->
                      <span class="st-scroll" tabindex="-1"><span class="st-text">{ep.text}</span></span>
                      <span class="st-ts">{fmtTime(e.ts)}</span>
                    </div>
                  {/each}
                </div>
                {#if b.events.length > hubPrefs.stepsRows}
                  <!-- Outside the scroller on purpose: a control that scrolls
                       away is a control you cannot find. -->
                  <button class="s-all" onclick={() => { stepsAll[b.key] = !stepsAll[b.key]; }}>
                    {capped ? t('hubStepsAll').replace('{n}', String(b.events.length)) : t('hubStepsCap')}
                  </button>
                {/if}
              {/if}
            </div>
          {/if}
        {/each}
        {#if !blocks.length && roomReady}
          {#if selected && !managedAgents.length && registry.length}
            <!-- Nothing to talk to yet: start from a preset. One tap = that
                 agent becomes the lead; "several" starts a team in one go.
                 Offered for a CLOSED project too, for the same reason the +
                 button is: `projects::spawn` ensures the session, so "the
                 session is down" was never a reason to withhold the only way
                 to start one. -->
            <div class="start">
              <div class="start-h">{t('hubStartTitle')}</div>
              <div class="start-list">
                {#each registry as r (r.name)}
                  <button class="start-row" disabled={starting} onclick={() => addAgents([r.name], '', 'start')}>
                    {#if backendIcon(r.backend)}<img class="ava" src={backendIcon(r.backend)} alt={r.backend} />{:else}<span class="ava" style:background={backendColor(r.backend)}>{r.name.slice(0, 1).toUpperCase()}</span>{/if}
                    <span class="sr-name">{r.name}</span>
                    <span class="sr-backend">{r.backend}</span>
                    {#if r.can_hire}<span class="m-badge" title={t('agentsManagerHint')} aria-label={t('agentsManagerHint')}>M</span>{/if}
                  </button>
                {/each}
              </div>
              <button class="chip-btn" disabled={starting} onclick={() => openPicker('start')}>
                <Icon name="collab" size={13} /> {t('hubStartTeam')}
              </button>
            </div>
          {:else}
            <div class="empty">{managedAgents.length ? t('hubEmpty') : t('hubEmptyNoAgents')}</div>
          {/if}
        {/if}
      </div>
      <!-- Parked away from the tail: one tap back, with a dot when something
           arrived while you were reading. -->
      {#if !following}
        <button class="to-bottom" class:news={newBelow} title={t('hubToBottom')} onclick={() => scrollFeed(true)}>
          <Icon name="arrow-down" size={15} />
        </button>
      {/if}
      </div>

      <div class="composer">
        <div class="compose-shell" class:cmd={composerIsCmd}>
        <!-- WHO this goes to, always visible: the lead by default, one tap to
             retarget or to broadcast. No @ typing required. Pinned to the shell's
             top-left; the textarea's FIRST line starts beside it (measured
             text-indent) and wrapped lines run full width beneath it. -->
        {#if managedAgents.length}
          <div class="to-wrap" bind:clientWidth={toChipW}>
            <button class="to-chip" class:all={recipient === ALL_TARGET} class:note={!recipient}
              title={recipient === ALL_TARGET ? t('hubToAllHint') : recipient ? '' : t('hubToRoomHint')}
              onclick={() => recipientOpen = !recipientOpen}>
              <span class="to-label">{t('hubTo')}</span>
              <span class="to-name">{recipient === ALL_TARGET ? t('hubEveryone') : recipient || t('hubRoomNote')}</span>
              <Icon name={recipientOpen ? 'chevron-down' : 'chevron-up'} size={11} />
            </button>
            {#if recipientOpen}
              <div class="to-menu">
                {#each managedAgents as a (a.window)}
                  <button class:sel={recipient === a.name} onclick={() => setRecipient(a.name)}>
                    <span class="st" class:live-dot={stateIsLive(a.state)} style:background={stateDotColor(a.state)}></span>{a.name}
                  </button>
                {/each}
                <div class="to-sep"></div>
                <!-- Broadcast: every agent is interrupted. Labelled with what
                     it costs, not just with who it reaches. -->
                <button class:sel={recipient === ALL_TARGET} onclick={() => setRecipient(ALL_TARGET)}>
                  <span class="st all-dot"></span>
                  <span class="to-opt"><span>{t('hubEveryone')}</span><small>{t('hubToAllHint')}</small></span>
                </button>
                <button class:sel={!recipient} onclick={() => setRecipient('')}>
                  <span class="st note-dot"></span>
                  <span class="to-opt"><span>{t('hubRoomNote')}</span><small>{t('hubToRoomHint')}</small></span>
                </button>
              </div>
            {/if}
          </div>
        {/if}
        {#if palette?.items.length}
          <!-- Completion for a `/command`, opening UPWARD so the on-screen
               keyboard never covers it — the same rule as the recipient menu, and
               the same popover dialect. -->
          <div class="cmd-menu" role="listbox" tabindex="-1">
            {#each palette.items as it, i (it.value)}
              <button class="cmd-opt" class:cur={i === paletteIdx} role="option"
                aria-selected={i === paletteIdx}
                onpointerenter={() => (paletteIdx = i)}
                onclick={() => acceptCompletion(it)}>
                <span class="cmd-name">{it.value}</span>
                <span class="cmd-hint">{it.hint}</span>
              </button>
            {/each}
          </div>
        {/if}
        <!-- A textarea, not an input: a message you are still writing has to be
             readable. It grows with the text and then scrolls, so a long one is
             never a one-line peephole. -->
        <textarea class="c-input" rows="1" bind:this={composerEl} bind:value={composerText}
          style:text-indent={managedAgents.length ? `${toChipW + 8}px` : '0'}
          placeholder={recipient === ALL_TARGET ? t('hubComposerAll') : recipient ? t('hubComposerDm').replace('{name}', recipient) : t('hubComposerRoom')}
          onkeydown={onComposerKey}
          onpaste={onComposerPaste}
          onfocus={() => { following = true; scrollFeed(true); setTimeout(() => scrollFeed(true), 300); }}
        ></textarea>
        {#if pending.length}
          <div class="pend-row">
            {#each pending as a, i (a.path)}
              {#if a.kind === 'image'}
                <span class="pend-thumb" title={`[img:${a.n}] ${a.name}`}>
                  <!-- Tap the thumb → the in-app viewer (owner, 2026-08-27:
                       "文本输入框…显示的图片…我可以点击放大查看…在我应用内的"). -->
                  <button class="pend-view" aria-label={a.name}
                    onclick={() => shotView = a.thumb}>
                    <img src={a.thumb} alt={a.name} />
                  </button>
                  <span class="pend-n">{a.n}</span>
                  <button class="pend-x on-img" aria-label={t('hubRemoveAttachment')}
                    onclick={() => removeAttachment(i)}>
                    <Icon name="x" size={10} />
                  </button>
                </span>
              {:else}
                <span class="pend-chip" title={`[file:${a.n}] ${a.path}`}>
                  <Icon name="file" size={12} />
                  <span class="pend-name">{a.name}</span>
                  <button class="pend-x" aria-label={t('hubRemoveAttachment')}
                    onclick={() => removeAttachment(i)}>
                    <Icon name="x" size={11} />
                  </button>
                </span>
              {/if}
            {/each}
          </div>
        {/if}
        <!-- Send lives INSIDE the capsule, bottom-right, out of the flow: it
             stopped costing the composer a whole column. Empty, it is still
             CLICKABLE (grey, muted): the first tap arms it as a "send
             interrupt" button — amber, named — and the second fires. -->
        {#if intArm}
          <div class="int-pill" role="status">{t('hubIntArmed').replace('{who}', intWho)}</div>
        {/if}
        <input type="file" multiple hidden bind:this={fileEl} onchange={onPickFiles} />
        <button class="attach-btn" class:busy={attaching} title={t('hubAttach')} aria-label={t('hubAttach')}
          disabled={!selected || attaching} onclick={() => fileEl?.click()}>
          <svg class="plus-ring" viewBox="0 0 20 20" aria-hidden="true">
            <g fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round">
              <circle cx="10" cy="10" r="9.2" />
              <line x1="10" y1="6.4" x2="10" y2="13.6" />
              <line x1="6.4" y1="10" x2="13.6" y2="10" />
            </g>
          </svg>
        </button>
        <button class="send-btn" class:muted={!sendable && !intArm && !recipientBusy} class:arm={intArm}
          class:busy={recipientBusy && !sendable && !intArm}
          onclick={() => (sendable ? send() : armInterrupt())}
          title={intArm ? t('hubIntArmed').replace('{who}', intWho) : sendable ? t('hubSend') : t('hubIntHint')}
          aria-label={intArm ? t('hubIntArmed').replace('{who}', intWho) : sendable ? t('hubSend') : t('hubIntHint')}
          disabled={!selected || attaching || (!sendable && !intTargets.length)}>
          {#if !sendable && (intArm || recipientBusy)}
            <!-- A stop square inside a slowly circling arc: the "mid-turn, tap
                 to cut it" glyph every chat product speaks. Armed keeps the
                 same glyph on the amber ground — same object, hotter state —
                 instead of the lightning bolt nobody read as "interrupt". -->
            <svg class="stop-spin" viewBox="0 0 20 20" width="16" height="16" aria-hidden="true">
              <circle class="ss-ring" cx="10" cy="10" r="8" fill="none" stroke="currentColor"
                stroke-width="1.6" stroke-linecap="round" stroke-dasharray="37.7 12.6" />
              <rect x="6.6" y="6.6" width="6.8" height="6.8" rx="1.7" fill="currentColor" />
            </svg>
          {:else}
            <Icon name="send-up" size={15} />
          {/if}
        </button>
        </div>
      </div>
    </main>

    {#if termOpen && !compact}
    <!-- ── Terminal drawer: where terminal things live ── -->
    <section class="drawer">
      {#if !compact}
        <SideHandle varName="--hub-drawer-w" storeKey="tmux_hub_drawer_w"
          min={320} max={900} def={520} edge="left" label={t('hubTerminal')} />
      {/if}
      <div class="drawer-head">
        {#if drawerView === 'term'}
          <div class="win-list">
            {#each agents as a (a.window)}
              <button class="win-pill" class:cur={termTarget.startsWith(`${selected}:${a.window}.`)} onclick={() => pickWindow(a)}>
                <span class="st" style:background={stateDotColor(a.agent ? a.state : 'shell')}></span>
                {a.window}:{a.name}{#if a.agent && !a.managed}<span class="direct-tag">{t('hubDirect')}</span>{/if}
              </button>
            {/each}
          </div>
          <span class="spacer"></span>
          <!-- The roster count the retired statusline carried. Everything else it
               showed was a second copy of this bar. -->
          <span class="d-count">{managedAgents.length} · {working} {t('hubState_running')}</span>
          <button class="icon-btn" title={t('hubOpenFull')} onclick={() => { const m = /^(.+):(\d+)\.(\d+)$/.exec(termTarget); if (m) openTerminal(selected, termTarget, termCommand); }}>
            <Icon name="maximize" size={14} />
          </button>
        {:else if drawerView === 'files'}
          <!-- Files carries its own path bar and toolbar; the head only says
               which partition this is and keeps the one close affordance. -->
          <span class="d-files"><Icon name="files" size={13} />{t('files')} — {selected}</span>
          <span class="spacer"></span>
          <button class="icon-btn" title={t('hubFilesFull')} aria-label={t('hubFilesFull')}
            onclick={() => openFilesTab?.(selected, drawerFilesDir)}>
            <Icon name="maximize" size={14} />
          </button>
        {:else}
          <!-- The board partition: the head names it, maximize hands off to
               the board PAGE — the same translation the files head makes.
               New-issue lives HERE (board #23): the embedded Board renders no
               page-head of its own — that row only repeated the project name
               this head already carries. -->
          <span class="d-files"><Icon name="layout" size={13} />{t('board')} — {selected}</span>
          <span class="spacer"></span>
          <button class="icon-btn go" title={t('boardNew')} aria-label={t('boardNew')}
            onclick={() => (drawerBoardNew = { n: (drawerBoardNew?.n ?? 0) + 1 })}>
            <Icon name="plus" size={14} />
          </button>
          <button class="icon-btn" title={t('board')} aria-label={t('board')}
            onclick={() => openBoardTab?.(selected)}>
            <Icon name="maximize" size={14} />
          </button>
        {/if}
        <button class="icon-btn" title="Esc" onclick={closeDrawer}>
          <Icon name="x" size={14} />
        </button>
      </div>
      <div class="term-body" class:off={drawerView !== 'term'}>
        {#if termTarget}
          {#key termTarget}
            <Terminal target={termTarget} session={selected} command={termCommand} {fontSize} embedded chromeless active={visible && drawerView === 'term'} visible={visible && drawerView === 'term'} />
          {/key}
        {:else}
          <div class="empty">{t('hubNoPane')}</div>
        {/if}
      </div>
      {#if drawerView === 'files'}
        <!-- Per-project cwd is Files' own parked-position map (module-scoped,
             keyed by session), so each project wakes up where you left it. -->
        <div class="files-body">
          <Files session={selected} visible={visible} {fontSize} singlePane bind:currentDir={drawerFilesDir} />
        </div>
      {/if}
      {#if drawerView === 'board'}
        <!-- The task sidebar (board #13 follow-up): the REAL Board, embedded —
             no project sidebar, it follows this room's project. -->
        <div class="board-body">
          <Board session={selected} visible={visible && drawerView === 'board'} embedded issueRequest={drawerIssueReq} createRequest={drawerBoardNew} />
        </div>
      {/if}
    </section>
    {/if}
  </div>

  <!-- Confirm: stop one agent, close or delete the whole project. The dialog is
       the shared one (src/lib/ui/ConfirmDialog.svelte) — this page had the only
       good version of it, so it was lifted out rather than copied. -->
  <ConfirmDialog open={!!pendingAct} busy={acting} compact={compact}
    title={pendingAct ? t(ACT_COPY[pendingAct.kind].title).replace('{name}', pendingAct.name) : ''}
    note={pendingAct ? t(ACT_COPY[pendingAct.kind].note) : ''}
    confirmLabel={pendingAct ? t(ACT_COPY[pendingAct.kind].go) : ''}
    onconfirm={runAction} oncancel={() => (pendingAct = null)} />

  <!-- One context menu for every subject above: right-click on the desktop, long
       press on a phone. -->
  <ContextMenu at={ctxAt} items={ctxItems} who={ctxWho} oncancel={closeCtx} />

  <!-- Forgetting a PROJECT for good — only reachable from the recycle bin,
       the two-step rule: hide first, destroy there. -->
  <ConfirmDialog open={!!trashAsk} compact={compact}
    title={trashAsk ? t('projectPurgeTitle').replace('{name}', trashAsk.project.name) : ''}
    note={t('projectPurgeNote')}
    confirmLabel={t('hubPurgeGo')}
    onconfirm={purgeProject} oncancel={() => (trashAsk = null)} />

  {#if pickerOpen}
    <!-- ── Start a team: several agents at once ── -->
    <div class="dlg-backdrop" onclick={() => pickerOpen = false} role="presentation"></div>
    <div class="dlg" class:sheet={compact}>
      <h2>{t('hubStartTeam')}</h2>
      <div class="dlg-agents">
        {#each registry as r (r.name)}
          <button class="agent-pick" class:sel={startPick.includes(r.name)}
            onclick={() => { startPick = startPick.includes(r.name) ? startPick.filter((n) => n !== r.name) : [...startPick, r.name]; }}>
            {#if backendIcon(r.backend)}<img class="ava" src={backendIcon(r.backend)} alt={r.backend} />{:else}<span class="ava" style:background={backendColor(r.backend)}>{r.name.slice(0, 1).toUpperCase()}</span>{/if}
            {r.name} · {r.backend}
            {#if startPick.includes(r.name)}<Icon name="check" size={13} />{/if}
          </button>
        {/each}
      </div>
      <input placeholder={t('hubBrief')} bind:value={startBrief} />
      <div class="dlg-actions">
        <button class="chip-btn" onclick={() => pickerOpen = false}>{t('cancel')}</button>
        <button class="chip-btn primary" disabled={!startPick.length || starting}
          onclick={() => addAgents(startPick, startBrief.trim())}>
          {starting ? '…' : t('hubStartGo').replace('{n}', String(startPick.length))}
        </button>
      </div>
    </div>
  {/if}

  {#if createOpen}
    <!-- ONE New Project surface app-wide (CreateProjectDialog): the Terminal
         sidebar opens the same component, so they cannot drift apart. -->
    <CreateProjectDialog {compact}
      oncreated={async (proj) => {
        createOpen = false;
        await reload();
        await selectProject(proj.session);
      }}
      oncancel={() => createOpen = false} />
  {/if}
  {#if shotView}
    <Lightbox src={shotView} onclose={() => shotView = ''} />
  {/if}
</div>

<style>
  .hub-root {
    height: 100%; display: flex; flex-direction: column; min-height: 0;
    background: var(--bg); position: relative;
    --chat-canvas: color-mix(in srgb, var(--bg) 62%, var(--bg2));
    /* ONE bubble width budget for message, prompt row and tool lane — three
       rules sharing a literal is how they drift. The %-term is what rules on
       a wide screen: the old min(76%, 760px) let the 760px absolute cap win
       there, leaving bubbles at ~half of a wide chat column (owner,
       2026-08-28: "屏幕很宽的情况下，只占到了可能一半的宽度"). 84% keeps a
       readable gutter that says "a message, not a document"; 1360px still
       stops a full-screen ultrawide from producing 200-char prose lines. */
    --msg-max: min(84%, 1360px);
    --bubble-in: color-mix(in srgb, var(--bg) 92%, white 8%);
    --bubble-out: color-mix(in srgb, var(--bg) 84%, var(--accent) 16%);
    --bubble-line: color-mix(in srgb, var(--border) 72%, var(--text3) 28%);
  
    /* Design tokens (--fs-*, --meta-ink, --t-*) come from :root in app.css —
       promoted app-wide 2026-08-18. Contract: tmm-cli.md "Design tokens". */
  }
  .cols { flex: 1; display: grid; grid-template-columns: var(--sidebar-w) minmax(0, 1fr); min-height: 0; }
  .hub-root.compact .cols { grid-template-columns: minmax(0, 1fr); }
  /* Phone shape: tighter gutters, thumb-sized controls, no horizontal
     overflow. The page head wraps instead of pushing the chips off-screen. */
  /* ONE row, always: the app-wide phone rule lets a busy page-head wrap under
     the title (skill editor), but this header has few, icon-sized actions and
     wrapping put the Terminal toggle on a second line (owner, 2026-08-21:
     "打开terminal的按钮给换行到第二行了"). The title is the flexible child —
     .h1-text ellipsizes — and the buttons refuse to shrink. */
  .hub-root.compact .page-head { flex-wrap: nowrap; row-gap: 6px; padding: 8px 12px; gap: 7px; }
  /* The header actions are 28×25 icon squares; on the phone the VISUAL box
     stays small and the TAP target grows to ~42px via the invisible overlay
     (the token contract's hit rule). */
  .hub-root.compact .page-head :global(.icon-btn) { position: relative; }
  .hub-root.compact .page-head :global(.icon-btn)::before { content: ''; position: absolute; inset: -8px; }
  .hub-root.compact .page-head h1 { font-size: var(--fs-title); }
  .hub-root.compact .h1-edit { font-size: var(--fs-title); }
  /* Bottom padding tight against the composer: the capsule brings its own 8px
     (owner, 2026-08-21: "最后一个消息框，和发送框中间的高度也有点大"). */
  .hub-root.compact .feed { padding: 14px 10px 6px; gap: 9px; }
  .hub-root.compact .msg, .hub-root.compact .prompt { max-width: 91%; }
  /* No env(safe-area-inset-bottom) here: the composer does not sit at the
     screen's bottom — the TAB BAR below it does, and it already pads for the
     gesture bar (`--sab`). Adding the inset here too stacked it twice and
     opened a wide blank band between the composer and the tabs on Android
     (owner, 2026-08-21: "消息框和下边的选项标签中间的空白有点大"). With the
     keyboard open the tabbar hides, but the keyboard covers the inset then —
     8px is the right gap in both states. */
  .hub-root.compact .composer { padding: 8px 9px; }
  .hub-root.compact .compose-shell { padding: 6px 9px; border-radius: 15px; }
  .hub-root.compact .to-chip { max-width: 110px; height: 28px; }
  .hub-root.compact .to-label { display: none; }
  .hub-root.compact .c-input { min-height: 30px; font-size: var(--fs-body); max-height: calc(40vh / var(--ui-zoom, 1)); }
  .hub-root.compact .send-btn { width: 32px; height: 32px; right: 6px; bottom: 4.5px; border-radius: var(--ui-radius-control); }
  .hub-root.compact .attach-btn { right: 44px; bottom: 12.5px; }
  .hub-root.compact .chip-btn { min-height: 34px; }
  .hub-root.compact .s-head { min-height: 34px; }
  /* Drawer open: the conversation yields but stays present. */
  /* The terminal column is a DRAGGED width (SideHandle on its left edge), not a
     fraction: the owner reached for that divider and nothing moved. The chat
     column takes the rest and keeps a floor so it can never be squeezed away. */
  .hub-root.drawer-open .cols { grid-template-columns: var(--sidebar-w) minmax(280px, 1fr) var(--hub-drawer-w, 520px); }
  /* The project title, in its two states. The idle one carries a visible pencil
     and only underlines on hover — a permanent box would make the header look
     like a form, but relying on hover ALONE hid the feature (no hover on a
     phone). The edit state keeps the title's metrics so nothing in the row
     shifts when it appears. */
  /* The name and its ⋯ are ONE unit: content-sized so the group hugs the text,
     `min-width: 0` so a long name still ellipsizes inside it, and a 3px gap —
     close enough to read as an affordance ON the name (owner, 2026-08-30)
     while the button keeps its own 28×26 box and its enlarged tap overlay. */
  /* The NAME displays WHOLE, with priority (owner, 2026-08-30: "名字还是优
     先要显示全的，尽量不要省略"): the group refuses to shrink — the PATH is
     the flexible region that gives way (it scrolls), and the buttons never
     compress. */
  .title-group { display: flex; align-items: center; gap: 1px; flex: none; max-width: 60%; }
  /* The caret hugs the last letter: a narrow box (the icon-btn's 28px read
     as a detached control), quiet ink so the NAME stays the subject. The
     compact rule's invisible tap overlay still gives it a full target. */
  .title-caret { width: 20px; color: var(--text3); }
  .title-caret:hover { color: var(--text); }
  /* The global `.page-head h1` rule ellipsizes its own text; with a second child
     it has to be a flex row, or the pencil is pushed out and clipped by the
     heading's own `overflow: hidden` as soon as the name is long. */
  h1 { display: flex; align-items: center; min-width: 0; }
  /* The name is selectable text that ellipsizes; the pencil never shrinks with
     it, and it is the only thing that renames. */
  .h1-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* Glued to the name (2px, inside the h1's flex) and never shrinking with
     it — the retired rename pencil's exact geometry. */
  .h1-edit {
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    font-size: var(--fs-title); font-weight: 600; color: var(--text);
    min-width: 0; flex: 0 1 auto; width: 22ch; max-width: 100%;
    background: var(--bg2); border: 1px solid var(--accent-line); border-radius: var(--ui-radius-control);
    padding: 2px 6px; box-sizing: border-box;
  }
  .h1-edit:focus { outline: none; }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--status-ok); flex: none; }
  .dot.off { background: var(--text3); }

  .sidebar { position: relative; background: var(--bg2); border-right: 1px solid var(--border); display: flex; flex-direction: column; min-height: 0; }
  /* Phone: the project list slides over the conversation instead of taking a
     column from it. */
  /* The sheet geometry/motion is the SHARED .side-sheet dialect in app.css
     (owner, 2026-08-30: one drawer for Chat/Terminal/Board); only the touch
     row height is Hub's own. */
  .sidebar.sheet .side-row { min-height: 44px; }
  .side-scroll { flex: 1; overflow-y: auto; padding: 8px; }
  .p-name { flex: 1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-weight: 550; font-family: var(--font-display); }
  /* ── Sidebar row summary. The LOOK is the Terminal sidebar's, atom for atom
     ("应该和terminal侧边栏一样 … 这两个可以共用", owner 2026-08-24): the age
     and the agent chips wear the shared .side-age/.side-win classes from
     app.css — same quiet mono text the Terminal sidebar's window chips wear,
     with one chat-only garnish, the state dot. Only the two-line structure
     is local. */
  .p-main { flex: 1; min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .p-top { display: flex; align-items: baseline; gap: 6px; min-width: 0; }
  /* Two-line rows: the dot marks the PROJECT, so it rides the name's line
     instead of centering across the chips below. */
  .proj-row { align-items: flex-start; }
  .proj-row .dot { margin-top: 6px; }
  /* The recycle bin's rows: quieter than a live project (they are parked, not
     open-able), with the two verbs inline — restore free, purge confirmed. */
  .trash-row { cursor: default; color: var(--text3); }
  .trash-row:hover { background: var(--surface); }
  .trash-name { font-weight: 450; }
  .t-act {
    display: grid; place-items: center; width: 24px; height: 24px; flex: none;
    background: none; border: none; border-radius: var(--ui-radius-control);
    color: var(--text3); cursor: pointer;
  }
  .t-act:hover { background: var(--surface2); color: var(--text); }
  .t-act.danger:hover { color: var(--danger); }

  .mid { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  /* Fits → shows whole; doesn't fit → pans (wheelX action), NEVER an ellipsis.
     `flex: 0 1 auto` + min-width lets the header's buttons take their space
     first while the path yields, and the hidden scrollbar keeps the header one
     quiet line. */
  .path {
    font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub); color: var(--text3);
    white-space: nowrap; overflow-x: auto; overflow-y: hidden;
    /* THE dynamic region: shrinks below its content first (min-width: 0 +
       scroll) while the name group and the buttons stay whole; the spacer
       still owns the leftover, keeping the toggles right-aligned. */
    min-width: 0; flex: 0 1 auto;
    scrollbar-width: none; -webkit-overflow-scrolling: touch;
  }
  .path::-webkit-scrollbar { display: none; }
  .spacer { flex: 1; }
  .term-toggle.on { color: var(--accent); background: var(--accent-bg); }

  /* The roster: one line per agent, on every screen size. It answers "who is
     here and are they busy" — anything more was a wall of cards. Metrics run
     TIGHT (owner, 2026-08-25: "感觉整体占的空间不小"): the card is a chip
     with a reading, not a panel. */
  .roster { display: flex; align-items: stretch; padding: 6px 14px; border-bottom: 1px solid var(--border2); min-width: 0; }
  .cards { display: flex; gap: 5px; overflow-x: auto; scrollbar-width: none; flex: 1 1 auto; min-width: 0; }
  .cards::-webkit-scrollbar { display: none; }
  .acard {
    position: relative; flex: none; display: flex; flex-direction: column;
    align-items: stretch; justify-content: center; gap: 1px; overflow: hidden;
    min-height: 30px; background: var(--surface); border: 1px solid var(--border);
    border-radius: var(--ui-radius-row); padding: 3px 8px 3px 5px; cursor: pointer; text-align: left;
    font-size: var(--fs-ui); color: var(--text2); transition: border-color var(--t-fast), color var(--t-fast);
    -webkit-tap-highlight-color: transparent;
  }
  /* The identity row — what the card used to be in its entirety. */
  .ac-top { display: flex; align-items: center; gap: 6px; }
  /* The sniffed reading: the smallest step on the scale, in monospace so a model
     id and a percentage do not reflow as they change. The cap must FIT a real
     `model · effort` reading (26ch holds `claude-sonnet-4.5 · medium`): at 16ch
     the effort truncated into a trailing `…` that carried no information — the
     owner read it as decoration ("模型名 后面又有一个点点点 … 好像有点多余了",
     2026-08-26). Cards size to content, so short readings stay tight. */
  .ac-vitals {
    font-size: var(--fs-micro); color: var(--meta-ink); line-height: 1.35;
    font-family: ui-monospace, Menlo, monospace; padding: 0 1px 1px 5px;
    max-width: 26ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  /* One thin horizontal line at the card's bottom edge. Absolute, so it costs no
     height and cannot push the roster taller as it appears. */
  .ac-bar {
    position: absolute; left: 0; right: 0; bottom: 0; height: 2px;
    background: var(--pill-bg); border-radius: 0 0 8px 8px; overflow: hidden;
  }
  .ac-bar > i { display: block; height: 100%; transition: width var(--t-move), background var(--t-move); }
  .acard:hover { border-color: var(--input-border); color: var(--text); }
  .acard.sel { border-color: var(--accent-line); background: var(--accent-bg); color: var(--text); }
  /* The add card rides INSIDE the strip as its last member, STICKY at the
     right edge: after the last card when everything fits, floating at the
     edge while the strip scrolls (the strip hides its scrollbar, so a
     plainly-scrolling add button was invisible — owner, 2026-08-21). The
     opaque ground alone masks cards passing beneath — the left lift shadow
     read as "a strange shadow" between it and the cards, and the borderless
     icon-action grammar applies to a lone + as much as to any icon button
     (owner, 2026-08-28). */
  .acard.add {
    flex-direction: row; align-items: center; gap: 6px; color: var(--text3); padding-right: 12px;
    position: sticky; right: 0; z-index: 1; background: var(--bg);
    border-color: transparent;
    transition: border-color var(--t-fast), color var(--t-fast), background var(--t-fast);
  }
  .acard.add:hover { color: var(--accent); background: var(--surface2); }
  /* With agents present it is a small square +: it stands at the end of the
     family without holding a seat wider than it needs (owner, 2026-08-25:
     "不用强行一直占一个位置"). */
  .acard.add.mini { padding: 0 7px; }
  .acard.off { opacity: 0.55; cursor: default; }
  /* Waking on hover is fine; promising a click is not — the accent border was
     the card selling a card-wide restart it no longer has. */
  .acard.off:hover { opacity: 1; }
  /* An action is in flight: the card stops taking clicks (the handler guards
     too — this is the visible half of that). */
  .acard.busy { opacity: 0.35; pointer-events: none; }
  /* Identity layer: names wear the display face (--font-display), not mono. */
  .a-name { font-family: var(--font-display); font-weight: 600; max-width: 12ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .s-age { color: var(--text3); font-size: var(--fs-meta); font-variant-numeric: tabular-nums; font-family: ui-monospace, Menlo, monospace; }
  .st { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .unread { width: 7px; height: 7px; border-radius: 50%; background: var(--status-danger); flex: none; }
  .ava.dim { background: var(--surface2) !important; color: var(--text3); }
  /* The stopped card's ONE way back up: a real button in the .a-more dialect,
     accent-leaning so it reads as the card's action. The card surface around
     it is inert — see the stopped-card comment in the markup. */
  .a-start {
    display: grid; place-items: center; width: 20px; height: 22px; border-radius: 6px;
    background: none; border: none; padding: 0; cursor: pointer; color: var(--text3); flex: none;
  }
  .a-start:hover:not(:disabled) { color: var(--accent); background: var(--surface2); }
  .a-start:disabled { opacity: 0.5; cursor: default; }
  /* The agent action menu: a fixed popover, positioned in JS from the trigger's
     rect (see toggleAgentMenu). It speaks the same dialect as .to-menu — same
     surface, radius, shadow and row metrics — because this file should have ONE
     popover language, not one per feature. Invisible until measured so the
     clamp/flip cannot be seen happening. */
  .a-menu {
    position: fixed; z-index: 24; min-width: 176px; max-width: min(76vw, 280px);
    background: var(--bg); border: 1px solid var(--border); border-radius: var(--ui-radius-panel);
    box-shadow: 0 12px 34px rgba(0,0,0,0.45); padding: 5px;
    display: flex; flex-direction: column; gap: 2px;
    opacity: 0; transition: opacity var(--t-fast) ease;
  }
  .a-menu.ready { opacity: 1; }
  .am-who {
    font-family: var(--font-display); font-weight: 600;
    font-size: var(--fs-meta); color: var(--text3);
    padding: 4px 10px 5px; border-bottom: 1px solid var(--border2); margin-bottom: 3px;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  /* Menu rows are CONTROLS: --ui-font-control (= --fs-sub) is the app's size
     for those, and it is what the action bar used before this became a menu.
     --fs-ui read as oversized for a menu (owner, 2026-08-19). */
  .a-menu button {
    display: flex; align-items: center; gap: 8px; min-height: 36px; width: 100%; text-align: left;
    background: none; border: none; border-radius: var(--ui-radius-control); color: var(--text2);
    padding: 6px 10px; font-size: var(--ui-font-control); cursor: pointer; font-family: ui-monospace, Menlo, monospace;
  }
  /* Touch contract: a menu row is a tap target, so the phone keeps 44px rows
     even though the type got smaller. */
  .hub-root.compact .a-menu button, .hub-root.compact .to-menu button { min-height: 44px; }
  .a-menu button:hover { background: var(--surface2); color: var(--text); }
  /* Tones live on the verb itself (owner, 2026-08-25): amber = interrupt (a
     turn cut short, the sys grammar's colour), red = stop/remove. */
  .a-menu button.warn { color: var(--status-warn); }
  .a-menu button.warn:hover { background: color-mix(in srgb, var(--status-warn) 14%, transparent); color: var(--status-warn); }
  .a-menu button.danger { color: var(--status-danger); }
  .a-menu button.danger:hover { background: color-mix(in srgb, var(--status-danger) 14%, transparent); color: var(--status-danger); }
  .a-menu button:disabled { opacity: 0.45; cursor: default; background: none; }
  .a-menu button :global(svg) { flex: none; }

  /* Chat canvas: one quiet tone derived from the theme. It had two radial
     glows for "depth"; the accent one read as a faint blue shadow over the
     conversation and the owner asked for it gone — flat is calmer. */
  .feed-wrap { flex: 1; position: relative; display: flex; min-height: 0; background: var(--chat-canvas); }
  .feed {
    flex: 1; overflow-y: auto; padding: 18px clamp(18px, 4vw, 64px) 24px;
    display: flex; flex-direction: column; gap: 10px;
    /* THE reason a held bubble may change its own height. Chromium's scroll
       anchoring compensated `scrollTop` whenever the held bubble grew or shrank;
       that compensation moved the geometry the boundary test reads, which
       unheld it, which restored the height — the "一闪一闪" infinite blink
       (measured: assigning scrollTop 2261 landed on 2221↔2298). With anchoring
       off, a height change is just a height change. The feed follows its tail
       explicitly anyway (scrollFeed), so nothing here depended on it. */
    overflow-anchor: none;
  }
  /* Feed rows must NEVER flex-shrink. The feed always overflows, and a column
     flex container compresses shrinkable children before scrolling; children
     with `overflow: visible` are saved by their min-content height, but any
     row with `overflow: hidden` has a spec minimum of ZERO — the sysline was
     crushed to its padding (10px of 23px) and read as an empty little bar
     (owner report, measured live). One rule retires the whole bug class. */
  .feed > * { flex: none; }
  /* The active anchor is the message itself. It enters and moves with the feed;
     only when that SAME element reaches an edge does sticky hold it there. */
  .msg.ask-top { position: sticky; top: 0; z-index: 6; }
  .msg.ask-bottom { position: sticky; bottom: 0; z-index: 6; }
  /* Floating treatment begins only after the bubble is actually held. Normal
     and held use the SAME opaque surface, so catching the edge changes depth,
     never identity or colour.

     A held bubble shows ALL of itself. It used to be clipped to a 33px window
     — one line — which turned every multi-line question into a single truncated
     line at the edge (owner, 2026-08-19: "保留原始样式完整显示就好，不用截断").
     What replaced it is nothing at all: no clip, no transform, no drawn frame.
     That is also the safest possible change here, because the ONE hazard in
     this feature is layout, not paint. The first version collapsed the bubble
     with max-height, which changed its flow height; the browser's scroll
     anchoring compensated scrollTop, that flipped the boundary condition back,
     and the anchor blinked in an infinite feedback loop (measured: setting
     scrollTop=2261 landed on 2221↔2298). So a cap is not available even as a
     safety net: any max-height on `.held` brings the loop back, and a generous
     clip window would be truncation again. The bubble is as tall as the message
     — a deliberately long question held at an edge covers more of the feed, and
     that is the trade the owner asked for.

     Depth is the only thing `.held` adds: the backdrop blur plus a lifted
     shadow, both paint-only, so a bubble overlapping the scrolling content
     below it reads as floating rather than as a rendering glitch. */
  /* NOTHING is clipped or capped here. The bubble keeps its whole box — border,
     radius, padding, meta trailer — and stays as tall as the text it is showing;
     what shrinks is the TEXT, folded by elideTail before it is rendered (owner,
     2026-08-19: "我希望是消息内容自己内部折叠 不是框截断 … 气泡什么的都要完整的不要
     任何裁切"). A cap or a clip would cut the bubble itself, which is exactly the
     thing that read as broken in the two earlier attempts. */
  .msg.held { -webkit-backdrop-filter: blur(10px); backdrop-filter: blur(10px); }
  .msg.held .bubble { box-shadow: 0 6px 20px rgba(0, 0, 0, 0.28); }
  /* Back to the tail. */
  .to-bottom {
    position: absolute; right: 14px; bottom: 12px; z-index: 7;
    width: 38px; height: 38px; border-radius: 50%; display: grid; place-items: center;
    background: var(--surface); border: 1px solid var(--border); color: var(--text2);
    cursor: pointer; box-shadow: 0 6px 18px rgba(0,0,0,0.35);
  }
  .to-bottom:hover { color: var(--accent); border-color: var(--accent); }
  .to-bottom.news::after {
    content: ''; position: absolute; top: 1px; right: 1px; width: 9px; height: 9px;
    border-radius: 50%; background: var(--status-danger); border: 2px solid var(--bg);
  }
  .msg { position: relative; display: flex; flex-direction: column; max-width: var(--msg-max); }
  /* Both sides hug their content (default column-flex STRETCH made every
     agent bubble 76% wide, leaving a short line's inline time stranded at
     the far right). */
  .msg { align-self: flex-start; }
  .msg.me { align-self: flex-end; }
  /* iOS-style CONTINUOUS corners live in app.css (the app-wide
     `corner-shape: squircle` policy block): svelte-check's CSS service does
     not know the property yet and the house bar is zero warnings. */
  .bubble {
    position: relative;
    background: var(--bubble-in); border: 1px solid var(--bubble-line);
    border-radius: 18px 18px 18px 6px; padding: 8px 12px 9px;
    color: var(--text); font-size: var(--fs-body); line-height: 1.48;
    word-break: break-word; overflow-wrap: anywhere; cursor: text;
    box-shadow: 0 1px 2px rgba(0,0,0,0.10);
    transition: border-color var(--t-fast) ease, box-shadow var(--t-fast) ease;
    -webkit-tap-highlight-color: transparent;
  }
  .bubble:hover { border-color: var(--input-border); }
  .msg.me .bubble {
    background: var(--bubble-out); border-color: color-mix(in srgb, var(--accent) 18%, transparent);
    border-radius: 18px 18px 6px 18px;
  }
  /* Agent name heads the bubble (your own carries none — the right-aligned
     accent bubble already says "yours"). */
  /* Agent name heads the bubble (your own carries none — the right-aligned
     accent bubble already says "yours"). A flex row so the status badge can
     sit at the bubble's TOP-RIGHT ("放到这个消息的右侧 往右上角放", owner
     2026-08-20) while the name keeps the left edge. */
  .m-head {
    display: flex; align-items: center; gap: 8px;
    font-family: var(--font-display);
    color: var(--accent); font-weight: 650; font-size: var(--fs-ui);
    letter-spacing: 0.1px; line-height: 1.2; margin: 0 0 2px; user-select: none;
  }
  /* The status-note header's state BADGE: the .pg-tag pill dialect (micro,
     uppercase, bordered) plus a leading dot, coloured by noteStateColor via
     inline `color` — border and dot follow through currentColor. A pill with
     a dot reads as a state the agent ENTERED; the first cut's arrow read as
     an addressee (owner, 2026-08-20). Pushed to the row's right edge. */
  .m-head .m-note-state {
    display: inline-flex; align-items: center; gap: 4px;
    margin-left: auto; padding: 0 5px; border-radius: 4px;
    border: 1px solid color-mix(in srgb, currentColor 55%, transparent);
    font-size: var(--fs-micro); font-weight: 650;
    text-transform: uppercase; letter-spacing: 0.6px; line-height: 1.6;
  }
  .m-head .mns-dot {
    width: 5px; height: 5px; border-radius: 50%; flex: none;
    background: currentColor;
  }
  /* The Telegram inline trailer: time (+ delivery ring on your own messages,
     to its right) FLOATS at the end of the content. On the last line when it
     fits, its own right-aligned line when it doesn't — never a separate
     row/column. Two pieces make it work with rendered markdown: the last
     content element (when it is a <p>) turns inline so the float can share
     its line box, and .m-body is flow-root so the bubble's height contains
     the float. The 7px top margin bottoms the 10px trailer within the
     ~20px line box. */
  .m-body { min-width: 0; display: flow-root; }
  .m-body > :global(p:nth-last-child(2)) { display: inline; }
  /* The leading @recipient — the address — reads apart from the words
     without shouting: weight and a quiet accent lean, no chip, no box. */
  .m-body :global(.m-to) { font-weight: 600; color: color-mix(in srgb, var(--accent) 62%, var(--text)); }
  .m-body :global(.katex-display) { overflow-x: auto; overflow-y: hidden; margin: 8px 0; padding: 2px 0; }
  .m-body :global(.katex) { font-size: 1.06em; }
  /* A real <button>: the accessible route to copy/raw (the bubble itself is
     text). Styled to stay a quiet trailer. */
  .m-meta {
    float: right; display: inline-flex; align-items: center; gap: 3px;
    margin: 7px 0 0 8px; color: var(--meta-ink); font-size: var(--fs-meta); line-height: 1;
    user-select: none; background: none; border: none; padding: 0;
    font-family: inherit; cursor: pointer;
  }
  /* "Show the rest": a quiet inline control inside the bubble, not a chip on
     top of it — the bubble is complete, this is part of its content. */
  .m-unfold {
    display: inline-flex; align-items: center; gap: 4px; margin-top: 4px;
    background: none; border: none; padding: 2px 0; cursor: pointer;
    color: var(--accent); font-family: inherit; font-size: var(--fs-sub);
  }
  .m-unfold:hover { text-decoration: underline; text-underline-offset: 2px; }
  .m-unfold :global(svg) { flex: none; }
  .m-state { display: inline-flex; opacity: 0.55; }
  .m-state.ok { color: var(--status-ok); opacity: 1; }
  .m-time { font-variant-numeric: tabular-nums; }
  .bubble .raw { margin: 0; font-family: var(--font-mono); font-size: var(--fs-sub); line-height: 1.5; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--text2); }
  /* What you can DO with a message, revealed by tapping it. An OVERLAY on the
     bubble's bottom-right corner, out of the flow: opening it must not push
     the feed around or change the scroll height the anchor math depends on. */
  .m-acts {
    position: absolute; z-index: 8; bottom: -13px; right: 10px;
    display: flex; gap: 5px; margin: 0;
  }
  .m-acts button {
    display: inline-flex; align-items: center; gap: 4px; min-height: 26px;
    background: var(--bubble-in); border: 1px solid var(--border); border-radius: var(--ui-radius-control);
    color: var(--text2); padding: 3px 10px; font-size: var(--fs-meta); cursor: pointer;
    box-shadow: 0 2px 8px rgba(0,0,0,0.18);
  }
  .m-acts button:hover, .m-acts button.on { color: var(--accent); border-color: color-mix(in srgb, var(--accent) 45%, transparent); }
  /* Referenced images, under the text they came with. */
  .shots { display: flex; flex-direction: column; gap: 6px; margin-top: 6px; border-radius: var(--ui-radius-control); overflow: hidden; clear: both; }
  .m-body > .shots:first-child { margin-top: 2px; }
  .msg.held .shots { display: none; }
  /* Lifecycle lines are the app narrating real actions (an agent stopped, a
     /command typed into a pane) — reading ink and body-adjacent size, not fine
     print the reader has to squint at ("不要只用灰色小字，让我看不太清", owner
     2026-08-20). Still a centred capsule: it is narration, not a speaker. One
     ROW per line inside it, because a folded group joined by `·` read as one
     run-on string (owner, 2026-08-24). */
  .sysline {
    align-self: center; display: flex; flex-direction: column; align-items: flex-start; gap: 3px;
    max-width: min(92%, 620px); padding: 5px 12px; border-radius: var(--ui-radius-row);
    color: var(--text2); background: color-mix(in srgb, var(--bubble-in) 88%, transparent);
    border: 1px solid var(--border2); box-shadow: 0 1px 2px rgba(0,0,0,0.06);
    font-size: var(--fs-sub);
    -webkit-backdrop-filter: blur(8px); backdrop-filter: blur(8px);
  }
  .sysline .sys-item { display: flex; align-items: baseline; gap: 6px; max-width: 100%; min-width: 0; }
  /* The three atoms of a narrated line, each in a dialect the feed already
     speaks (owner, 2026-08-24: "都用统一的 ui 来展示…不要随意瞎写"):
     the NAME wears the bubble header's ink (.m-head — 650-weight accent), the
     ACTION wears the status-note badge (.m-note-state — dot + word in a
     currentColor pill, coloured by the one progressive status language), and a
     /command is the composer's command dialect (monospace, accent lean). */
  .sysline .sys-who { flex: none; font-weight: 650; color: var(--accent); letter-spacing: 0.1px; }
  .sysline .sys-verb {
    flex: none; display: inline-flex; align-items: center; gap: 4px;
    font-size: var(--fs-micro); font-weight: 650;
    text-transform: uppercase; letter-spacing: 0.6px; line-height: 1.6;
  }
  .sysline .sys-verb .sv-dot { width: 5px; height: 5px; border-radius: 50%; flex: none; background: currentColor; }
  /* A /command row: the typed line stays ONE object — name and arguments
     together ("带参数的渲染好像不是很好", owner 2026-08-24). It wears the
     rendered-markdown INLINE CODE dialect (.md code: soft --code-bg wash,
     radius, NO border) with the composer's accent lean — drawn frames on the
     inner atoms read as chrome, not content ("不用这种边框的", owner same
     day, which also stripped the verb badge down to dot + word). */
  .sysline .sys-cmd {
    min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    color: color-mix(in srgb, var(--accent) 62%, var(--text));
    background: var(--code-bg);
    border-radius: 4px; padding: 0.1em 0.45em;
  }
  .sysline .sys-text {
    min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; color: var(--text);
  }
  /* A board move's origin status: quiet reading ink — the destination badge
     carries the colour, the origin only situates the transition. */
  .sysline .sys-from { flex: none; font-size: var(--fs-micro); color: var(--text3); }
  /* The board row is tappable: bare button in the row-hover family (surface2
     wash, no border — it is a row, not a control), inheriting the capsule's
     type. */
  .sysline .sys-jump {
    background: none; border: none; font: inherit; color: inherit;
    padding: 1px 5px; margin: -1px -5px; border-radius: 6px;
    cursor: pointer; text-align: left;
    transition: background var(--t-fast);
  }
  .sysline .sys-jump:hover { background: var(--surface2); }

  /* The feed's date separators: a centred pill in the sysline's capsule
     dialect, marking where a new calendar day starts. */
  .day-sep { align-self: center; display: flex; justify-content: center; padding: 6px 0 2px; user-select: none; }
  .day-pill {
    font-size: var(--fs-meta); font-weight: 600; letter-spacing: 0.3px;
    color: var(--text2); background: color-mix(in srgb, var(--bubble-in) 88%, transparent);
    border: 1px solid var(--border2); border-radius: 999px; padding: 2px 11px;
    box-shadow: 0 1px 2px rgba(0,0,0,0.06);
    -webkit-backdrop-filter: blur(8px); backdrop-filter: blur(8px);
  }

  /* The input half of a turn — what the agent was asked. */
  /* The delivered line's atoms (owner, 2026-08-30): the sender in the name
     ink, the issue chip in the sys-who mono dialect, the review badge in the
     status-badge dialect — the same vocabulary the sysline speaks. */
  .prompt .p-from { color: var(--accent); font-weight: 650; }
  .prompt .p-chip { font-family: ui-monospace, Menlo, monospace; color: var(--accent); font-weight: 650; margin-right: 6px; }
  .prompt .p-badge {
    display: inline-flex; align-items: center; gap: 4px; margin-right: 6px;
    font-size: var(--fs-micro); font-weight: 650;
    text-transform: uppercase; letter-spacing: 0.6px;
  }
  .prompt .p-badge .pb-dot { width: 5px; height: 5px; border-radius: 50%; flex: none; background: currentColor; }

  .prompt { align-self: flex-start; max-width: var(--msg-max); border-left: 2px solid var(--border); padding-left: 9px; margin: 1px 6px; }
  .p-head { display: flex; align-items: baseline; gap: 7px; font-size: var(--fs-meta); color: var(--text3); margin-bottom: 2px; }
  .p-head .p-who { font-family: ui-monospace, Menlo, monospace; font-weight: 600; color: var(--text2); }
  .p-tag { text-transform: uppercase; letter-spacing: 0.8px; font-size: var(--fs-micro); color: var(--text3); border: 1px solid var(--border); border-radius: 4px; padding: 0 4px; }
  .p-body { font-size: var(--fs-ui); color: var(--text2); white-space: pre-wrap; word-break: break-word; overflow-wrap: anywhere; max-height: 7.5em; overflow: hidden; }

  /* A single observed fact: status declaration, lifecycle hook, warning. */
  .note {
    display: flex; align-items: baseline; gap: 8px; width: var(--msg-max);
    font-family: var(--font-mono); font-size: var(--fs-meta); color: var(--text3);
    padding: 1px 8px; max-width: 100%;
  }
  .note .n-who { flex: none; font-weight: 600; }
  .note .n-text { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .note .n-ts { flex: none; margin-left: auto; opacity: 0.6; }
  .note.warn { color: var(--status-warn); }
  .note.warn :global(svg) { flex: none; align-self: center; }

  /* A progress note: what the agent SAYS it is doing. Between a bubble and a
     telemetry row on purpose — it is prose the agent chose to write, so it gets
     the reading font and full-strength ink, but it is about work rather than
     addressed to anyone, so it wears a lane bar instead of a bubble. */
  .prog {
    display: flex; align-items: baseline; gap: 8px; width: 100%; max-width: 100%;
    font-size: var(--fs-sub); color: var(--text2);
    padding: 3px 10px 3px 0; position: relative;
  }
  .pg-bar {
    flex: none; align-self: stretch; width: 2px; min-height: 1em;
    background: var(--accent); opacity: 0.55; border-radius: 2px; margin-right: 2px;
  }
  .pg-who { flex: none; font-family: ui-monospace, Menlo, monospace; font-weight: 650; color: var(--text3); }
  .pg-tag {
    flex: none; font-size: var(--fs-micro); text-transform: uppercase; letter-spacing: 0.6px;
    color: var(--status-warn); border: 1px solid var(--status-warn); border-radius: 4px; padding: 0 3px;
  }
  .pg-text { min-width: 0; overflow-wrap: break-word; }
  .pg-ts { flex: none; margin-left: auto; font-size: var(--fs-meta); color: var(--meta-ink); font-variant-numeric: tabular-nums; }
  /* Waiting on a human is not the same colour as making progress. */
  .prog.blocked .pg-bar { background: var(--status-warn); opacity: 0.9; }
  .prog.blocked .pg-text { color: var(--text); }

  /* Collapsible run of tool calls between two replies. */
  /* Telemetry, not a bubble: the group spans the feed's full width so paths
     stop being truncated at 76% (owner: "整个宽度非常窄"), and it is ONE card —
     the head owns no border of its own, the body is separated by a line rather
     than indented with a margin+border guide. That guide is what made the left
     edge jog when the group opened: the body box started at 11px while the
     head's text started at 30px. */
  .steps {
    display: flex; flex-direction: column; width: 100%;
    /* The lane's painted colour as one value: the feed canvas underneath, the
       same 3% wash on top. Nothing needs to COVER anything since the middle cell
       became the only scroller, but the token stays — it is the lane's colour,
       and the next thing that must match it should have one name to reach for. */
    --lane-bg: linear-gradient(var(--surface), var(--surface)), var(--chat-canvas);
    /* 30px = the head's padding (10) + chevron (12) + gap (7): every row, the
       pinned name column and the "show all" button line up under the head's TEXT,
       which is the column the eye follows. ONE number, on the element they all
       inherit from — a second copy is how the column and the rows drift apart. */
    --lane-indent: 30px; --lane-pad-r: 10px;
    background: var(--lane-bg); border: 1px solid var(--border2); border-radius: var(--ui-radius-panel);
    overflow: hidden;
  }
  .s-head {
    display: flex; align-items: center; gap: 7px; width: 100%; text-align: left;
    background: none; border: none; border-radius: 0;
    padding: 5px 10px; cursor: pointer; color: var(--text3);
    font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub);
    transition: color var(--t-fast);
  }
  .steps:hover { border-color: var(--input-border); }
  .s-head:hover { color: var(--text2); }
  .chev { display: inline-flex; flex: none; transition: transform var(--t-move); }
  .chev.open { transform: rotate(90deg); }
  /* A run in motion pulses in the MOTION colour (the accent — see the status
     colour language in hub.ts): green would say "ended well" about something
     still going. The pulse itself is the app-wide `.live-dot` cue in app.css
     (halo + breathe, never an opacity fade — that fade is what made a running
     dot read dimmer than a resting one), worn alongside this class. */
  .s-live { flex: none; width: 7px; height: 7px; border-radius: 50%; background: var(--accent); }
  .s-who { flex: none; font-weight: 600; color: var(--text2); }
  .s-count { flex: none; }
  .s-peek { min-width: 0; opacity: 0.7; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .s-body {
    display: flex; flex-direction: column; gap: 2px;
    padding: 5px var(--lane-pad-r) 6px var(--lane-indent);
    border-top: 1px solid var(--border2);
    /* One em == one step row's font size, so the cap below is expressed in ROWS
       and follows the type scale instead of a magic pixel height. */
    font-size: var(--fs-sub);
    /* The ARGUMENT is the interesting half of a tool call and it is routinely
       wider than the lane — a path, a command, a heredoc. It used to be cut with
       an ellipsis, so the one thing you opened the lane to read was the one thing
       you could not (owner, 2026-08-20: "这些参数应该左右可以滑动，查看完整的参
       数"). Each row's MIDDLE CELL pans instead — see .st-scroll: the lane itself
       never scrolls horizontally, which is what makes bleed-through impossible. */
    overflow-x: hidden;
  }
  /* Ten rows, then scroll. Each step is exactly one line (rows never wrap), so
     rows and lines are the same thing here.
     overscroll-behavior stays AUTO on purpose: when the pointer/finger is over
     this inner scroller and it reaches its top or bottom, the scroll must
     CHAIN to the feed — `contain` trapped the gesture and the page "stuck"
     the moment you scrolled across a tool group (owner, 2026-08-21: "手势点在
     工具调用框框，就卡住了滚不上去了"). The feed itself never flings from
     this: chaining only starts at the lane's edge, which is exactly the owner's
     rule ("小窗口到顶，就继续滚外部的消息框"). */
  .s-body.capped {
    max-height: calc(var(--steps-rows) * (1.5em + 2px) + 11px);
    overflow-y: auto;
    scrollbar-width: thin;
  }
  .s-all {
    align-self: flex-start; background: none; border: none; color: var(--text3);
    font-size: var(--fs-meta); cursor: pointer; font-family: ui-monospace, Menlo, monospace;
    padding: 2px var(--lane-pad-r) 5px var(--lane-indent);
  }
  .s-all:hover { color: var(--accent); }
  .step {
    display: flex; align-items: baseline; gap: 8px; line-height: 1.5;
    font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub); color: var(--text3);
  }
  /* The tool name: the part the eye scans down a column. A plain flex child —
     it sits OUTSIDE the scroller, so the argument cannot be panned under it. */
  .tname { flex: none; color: var(--accent); font-weight: 650; }
  .step .tname { min-width: 6.5em; max-width: 12em; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hub-root.compact .step .tname { min-width: 0; }
  .s-peek .tname { min-width: 0; margin-right: 5px; }
  /* The middle column, and the ONLY scroller. It takes the leftover width, clips
     its own overflow, and pans on wheel/touch — so the full argument is reachable
     ("中间参数是可以左右滑动查看的") while the name and the time never move. Its
     scrollbar is hidden: ten rows each drawing one is a ruler collection, and the
     cut-off text itself is the affordance. */
  .step .st-scroll { flex: 1; min-width: 0; overflow-x: auto; scrollbar-width: none; }
  .step .st-scroll::-webkit-scrollbar { display: none; }
  /* No ellipsis: what does not fit is scrolled to, not cut. Still `nowrap`, NOT
     `pre`: a tool detail routinely contains real newlines (a heredoc, a multi-line
     command), and `pre` would turn one call into a three-line row — breaking both
     one-row-per-call and the 10-row cap, whose height is single-line math. */
  .step .st-text { display: inline-block; white-space: nowrap; color: var(--text2); }
  /* The right column: a plain flex child after the scroller, always at the lane's
     right edge no matter how far the argument is panned. */
  .step .st-ts { flex: none; opacity: 0.55; }
  .empty { color: var(--text3); font-size: var(--fs-ui); text-align: center; margin: auto; padding: 0 24px; line-height: 1.6; }

  .composer {
    display: flex; align-items: flex-end; gap: 9px; padding: 10px clamp(12px, 3vw, 28px);
    border-top: 1px solid var(--border2); background: color-mix(in srgb, var(--bg) 92%, transparent);
    box-shadow: 0 -8px 28px rgba(0,0,0,0.05);
    -webkit-backdrop-filter: blur(14px); backdrop-filter: blur(14px);
    /* ONE stacking context for the whole composer, above every layer the FEED
       makes (pinned .ask-top/.ask-bottom at 6, the action overlay at 8): its
       popovers — the recipient menu and the / palette — open UPWARD over the
       feed, and without this the .to-wrap wrapper's own level (2) CAPPED the
       menu below a pinned bubble (board #1, owner: "应该在钉住的消息图层上方…
       新弹出的选项都应该优先级高于已有的组件"). Inner levels keep ordering
       INSIDE the composer; this decides composer-vs-feed once. */
    position: relative; z-index: 15;
  }
  /* Paging feedback: a whisper at the very top of the scrollback, never a
     component — it shares the sys-line grey and costs one line. */
  .older-hint { text-align: center; color: var(--text3); font-size: var(--fs-micro); padding: 2px 0 6px; }
  .older-more { display: block; width: 100%; background: none; border: none; cursor: pointer; transition: color var(--t-fast); }
  .older-more:hover { color: var(--accent); }

  /* The filter mode's pill: a compact, content-width capsule pinned at the
     feed's top — accent-tinted so it reads as a STATE, not a message. Sticky
     (not a layout row): it must never own a column or squeeze the feed
     (reopened #3: as a feed-wrap sibling it displayed as the left half). */
  .filter-pill {
    position: sticky; top: 0; z-index: 9; /* above pinned bubbles (6) and the action overlay (8) */
    align-self: center;
    display: inline-flex; align-items: center; gap: 6px;
    padding: 3px 6px 3px 12px;
    border-radius: 999px;
    background: color-mix(in srgb, var(--accent) 10%, var(--bg));
    border: 1px solid color-mix(in srgb, var(--accent) 30%, transparent);
    color: var(--text2); font-size: var(--fs-meta);
    box-shadow: 0 2px 10px rgba(0, 0, 0, 0.08);
  }
  .filter-pill .f-name { color: var(--accent); font-weight: 650; font-family: ui-monospace, Menlo, monospace; }

  .compose-shell {
    flex: 1; min-width: 0; position: relative;
    padding: 6px 10px; border: 1px solid var(--input-border); border-radius: 16px;
    background: var(--bubble-in); box-shadow: 0 1px 3px rgba(0,0,0,0.10);
    transition: border-color var(--t-fast) ease, box-shadow var(--t-fast) ease;
  }
  .compose-shell:focus-within { border-color: color-mix(in srgb, var(--accent) 55%, transparent); box-shadow: 0 2px 8px rgba(0,0,0,0.12); }
  /* A line that will be RUN, not said: machine text wears the machine face —
     the same monospace the tool lane uses — plus an accent tint on the capsule,
     so "this goes to the CLI" is visible before send decides anything. The
     mirror MUST flip with it: it re-lays-out the text to find the last line,
     and measuring mono text with a proportional font misplaces the send
     button's collision zone. */
  .compose-shell.cmd { border-color: color-mix(in srgb, var(--accent) 45%, transparent); background: color-mix(in srgb, var(--accent) 6%, var(--bubble-in)); }
  .compose-shell.cmd .c-input, .compose-shell.cmd :global(.c-mirror) { font-family: ui-monospace, Menlo, monospace; }
  /* Recipient control: who this message goes to, with a menu that opens
     UPWARD so the on-screen keyboard never covers it. */
  /* Pinned to the capsule's top-left; the textarea's first line is indented
     past it and later lines reclaim the full width beneath. */
  .to-wrap { position: absolute; top: 7px; left: 8px; z-index: 2; width: max-content; }
  .to-chip {
    display: flex; align-items: center; gap: 4px; height: 26px;
    background: var(--accent-bg); color: var(--accent); border: 1px solid transparent;
    border-radius: var(--ui-radius-control); padding: 0 9px; font-size: var(--fs-sub); font-weight: 650;
    cursor: pointer; max-width: min(34vw, 220px);
  }
  /* Broadcast and room-note are NOT the default state, so they do not wear the
     accent: one interrupts everyone, the other reaches nobody live. */
  .to-chip.all { background: var(--surface); color: var(--status-warn); border-color: var(--status-warn); }
  .to-chip.note { background: var(--surface); color: var(--text2); border-color: var(--border); }
  .to-label { font-weight: 500; opacity: 0.7; font-size: var(--fs-meta); text-transform: uppercase; letter-spacing: 0.5px; }
  .to-name { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .to-sep { height: 1px; background: var(--border2); margin: 4px 6px; }
  .to-opt { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .to-opt small { font-size: var(--fs-meta); opacity: 0.65; }
  .note-dot { border: 1px dashed var(--text3); background: none; }
  .to-menu {
    position: absolute; bottom: calc(100% + 6px); left: 0; z-index: 12;
    min-width: 168px; max-height: calc(46vh / var(--ui-zoom, 1)); overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: var(--ui-radius-panel);
    box-shadow: 0 12px 34px rgba(0,0,0,0.45); padding: 5px; display: flex; flex-direction: column; gap: 2px;
  }
  .to-menu button {
    display: flex; align-items: center; gap: 7px; min-height: 36px; width: 100%; text-align: left;
    background: none; border: none; border-radius: var(--ui-radius-control); color: var(--text2);
    padding: 6px 10px; font-size: var(--ui-font-control); cursor: pointer; font-family: ui-monospace, Menlo, monospace;
  }
  .to-menu button:hover { background: var(--surface2); color: var(--text); }
  .am-vitals {
    padding: 0 9px 6px; margin-top: -3px; font-size: var(--fs-meta); color: var(--text3);
    font-family: ui-monospace, Menlo, monospace; max-width: 240px;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  /* The slash-command palette: the recipient menu's surface, full capsule width
     because a command list is read as rows of name + description. */
  .cmd-menu {
    position: absolute; bottom: calc(100% + 6px); left: 0; right: 0; z-index: 14;
    max-height: calc(44vh / var(--ui-zoom, 1)); overflow-y: auto; scrollbar-width: thin;
    background: var(--bg); border: 1px solid var(--border); border-radius: var(--ui-radius-panel);
    box-shadow: 0 12px 34px rgba(0,0,0,0.45); padding: 5px;
    display: flex; flex-direction: column; gap: 2px;
  }
  .cmd-opt {
    display: flex; align-items: baseline; gap: 10px; width: 100%; text-align: left;
    background: none; border: none; border-radius: var(--ui-radius-control); color: var(--text2);
    padding: 6px 10px; font-size: var(--ui-font-control); cursor: pointer;
    font-family: ui-monospace, Menlo, monospace;
  }
  /* Hover and the keyboard cursor are the SAME highlight — two would read as two
     selections. */
  .cmd-opt:hover, .cmd-opt.cur { background: var(--surface2); color: var(--text); }
  .cmd-name { flex: none; font-weight: 650; color: var(--accent); }
  .cmd-hint {
    min-width: 0; color: var(--text3); font-family: inherit; font-size: var(--fs-meta);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  .hub-root.compact .cmd-opt { min-height: 44px; align-items: center; }
  .to-menu button.sel { color: var(--accent); background: var(--accent-bg); }
  .all-dot { border: 1px solid var(--text3); background: none; }
  .c-input {
    display: block; width: 100%; min-height: 28px; max-height: calc(34vh / var(--ui-zoom, 1));
    padding: 5px 0 4px; background: transparent; border: none; outline: none;
    color: var(--text); font-size: var(--fs-body); line-height: 1.5;
    resize: none; overflow-y: auto;
  }
  /* growComposer's layout mirror: same metrics as .c-input, invisible.
     Created by JS, so it has no scope class — hence :global under the shell. */
  .compose-shell :global(.c-mirror) {
    position: absolute; left: 10px; top: 0; visibility: hidden; pointer-events: none;
    font-size: var(--fs-body); line-height: 1.5; white-space: pre-wrap;
    overflow-wrap: break-word; padding: 0; border: 0;
  }
  .c-input::placeholder { color: var(--text3); opacity: 0.82; }
  /* The send action: a bold up-arrow (the iMessage/ChatGPT shape — symmetric,
     so it optically centres where a diagonal plane always sat crooked) on a
     flat accent square that matches the capsule's radius. Light theme: full
     accent (a deep blue) + near-white ink. Dark theme: the accent is ELECTRIC
     CYAN (#00d4ff) — at full strength it read as a glowing block on the dark
     canvas (owner report), so the fill is toned to a 60% mix with the
     background and the ink flips to near-white, which also matches the
     recipient chip's quiet accent language. Disabled recedes into the
     surface instead of ghosting the accent. */
  /* Sized so the empty capsule centres it exactly (measured shell 43px, so
     the absolute `bottom` is measured from the PADDING box, 1px inside the
     border, so 5.5px yields symmetric 6.5px gaps). Bottom-anchored, so it
     stays put as the box grows into multiple lines. */
  .pend-row { display: flex; flex-wrap: wrap; gap: 5px; padding: 6px 88px 4px 4px; }
  .pend-chip {
    display: inline-flex; align-items: center; gap: 5px;
    height: 24px; padding: 0 4px 0 8px; max-width: 220px;
    border: 1px solid var(--border2); border-radius: var(--ui-radius-control);
    background: var(--surface2); color: var(--text2); font-size: var(--fs-micro);
  }
  .pend-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pend-view { display: block; width: 100%; height: 100%; padding: 0; margin: 0; border: 0; background: none; cursor: zoom-in; }
  .pend-thumb {
    position: relative; width: 44px; height: 44px; flex: none;
    border: 1px solid var(--border2); border-radius: var(--ui-radius-control);
    overflow: hidden; background: var(--surface2);
    cursor: zoom-in;
  }
  .pend-thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .pend-n {
    position: absolute; left: 0; bottom: 0; padding: 0 4px;
    font-size: var(--fs-micro); line-height: 14px; font-family: ui-monospace, Menlo, monospace;
    color: #fff; background: rgba(0,0,0,0.55); border-top-right-radius: 5px;
  }
  .pend-x.on-img {
    position: absolute; top: 0; right: 0; width: 15px; height: 15px;
    color: #fff; background: rgba(0,0,0,0.55); border-bottom-left-radius: 6px; border-radius: 0 0 0 6px;
  }
  .pend-x.on-img::after { inset: -6px; }
  .pend-x.on-img:hover { color: #fff; background: var(--status-danger); }
  .pend-x {
    display: grid; place-items: center; width: 16px; height: 16px; padding: 0;
    border: none; border-radius: 5px; background: none; color: var(--text3);
    cursor: pointer; position: relative;
  }
  .pend-x::after { content: ''; position: absolute; inset: -8px; }
  .pend-x:hover { color: var(--status-danger); }
  .attach-btn {
    position: absolute; right: 42px; bottom: 12.5px;
    width: 16px; height: 16px; display: grid; place-items: center;
    padding: 0; border: none; border-radius: 50%;
    background: transparent; color: color-mix(in srgb, var(--text3) 78%, transparent); cursor: pointer;
    transition: color var(--t-fast) ease;
  }
  .attach-btn .plus-ring { position: absolute; inset: 0; }
  .attach-btn::after { content: ''; position: absolute; inset: -12px; }
  /* The glyph is ONE drawing: hover lifts circle and plus together. */
  .attach-btn:hover:not(:disabled) { color: var(--accent); }
  .attach-btn:disabled { opacity: 0.55; cursor: default; }
  .attach-btn.busy { animation: attach-pulse 1s ease-in-out infinite; }
  @keyframes attach-pulse { 50% { opacity: 0.4; } }
  .send-btn {
    position: absolute; right: 7px; bottom: 5.5px;
    width: 30px; height: 30px; display: grid; place-items: center;
    padding: 0; border: none; border-radius: var(--ui-radius-control); cursor: pointer;
    background: var(--accent-fill);
    color: var(--accent-fill-ink);
    transition: filter var(--t-fast) ease, background var(--t-fast) ease, color var(--t-fast) ease, transform var(--t-fast) ease;
  }
  .send-btn:hover:not(:disabled) { filter: brightness(1.07); }
  .send-btn:active:not(:disabled) { transform: scale(0.93); }
  .send-btn:disabled { background: var(--surface2); color: var(--text3); cursor: default; }
  /* Empty composer: clickable but wearing the resting grey — the tap is an
     ARM, not a send, and the button must not advertise accent urgency. */
  .send-btn.muted { background: var(--surface2); color: var(--text3); }
  /* The recipient is mid-turn: same resting ground, but the glyph is the
     spinner-around-a-stop-square in accent — alive, not urgent. The button
     still ARMS first; this state only changes what it looks like at rest. */
  .send-btn.busy { background: var(--surface2); color: var(--accent); }
  /* Armed: the one attention colour — interrupt asks a person to confirm. */
  .send-btn.arm { background: var(--status-warn); color: var(--accent-fill-ink); }
  /* Deliberately unhurried (owner: "动画不用很快"): a fast spin says
     "loading", this says "a turn is open". The square stays put; only the
     arc travels. */
  .ss-ring { transform-origin: 50% 50%; animation: stop-spin 2.2s linear infinite; }
  @keyframes stop-spin { to { transform: rotate(360deg); } }
  @media (prefers-reduced-motion: reduce) { .ss-ring { animation: none; } }
  /* What the armed button will do, in words, above it — a phone has no hover
     for the title. pointer-events off: it is a caption, not a control. */
  .int-pill {
    position: absolute; right: 6px; bottom: 44px;
    font-size: var(--fs-micro); color: var(--text2);
    background: var(--surface2); border-radius: 6px; padding: 3px 8px;
    white-space: nowrap; pointer-events: none;
  }
  /* Phone-first hit areas (contract: primary actions ≥44px): the visual box
     stays small, the tap target grows via an invisible overlay. .to-bottom
     uses ::before — its ::after is the new-output dot. */
  .send-btn::after { content: ''; position: absolute; inset: -7px; }
  .to-bottom::before { content: ''; position: absolute; inset: -3px; }

  /* Empty room: start from a preset — one agent, or a team. */
  .start { margin: auto; display: flex; flex-direction: column; gap: 8px; width: min(420px, 100%); }
  .start-h { font-size: var(--fs-ui); color: var(--text2); text-align: center; margin-bottom: 2px; }
  .start-list { display: flex; flex-direction: column; gap: 5px; }
  .start-row {
    display: flex; align-items: center; gap: 8px; min-height: 44px; width: 100%; text-align: left;
    background: var(--surface); border: 1px solid var(--border); border-radius: var(--ui-radius-row);
    color: var(--text); padding: 8px 11px; font-size: var(--fs-ui); cursor: pointer;
  }
  .start-row:hover { border-color: var(--accent); background: var(--accent-bg); }
  .start-row:disabled { opacity: 0.5; }
  .sr-name { font-family: ui-monospace, Menlo, monospace; font-weight: 600; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sr-backend { font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub); color: var(--text3); margin-left: auto; }
  /* The Manager atom (board #7) — same declaration as AgentsPage's, pinned. */
  .m-badge { flex: none; display: inline-flex; align-items: center; justify-content: center; width: 15px; height: 15px; border: 1px solid var(--accent); border-radius: 4px; color: var(--accent); font-size: var(--fs-micro); font-weight: 700; line-height: 1; }

  .drawer { position: relative; }
  /* The drawer's GROUND is the app's, not the terminal's (board #23): a
     hardcoded #000 here leaked out as a black seam beside the chat column —
     the terminal element paints its own theme-adapted background, so in
     light theme every uncovered sliver of the drawer read as a black line
     that matched nothing. The dark surface belongs to the terminal BODY
     alone; files/board partitions already carry var(--bg). */
  .drawer { display: flex; flex-direction: column; min-width: 0; min-height: 0; background: var(--bg); border-left: 1px solid var(--border); }
  /* The head is the page-head's TWIN across the divider (board #23: the two
     top bars sat at different heights in different colors): same 42px
     min-height and border so the horizontal line runs THROUGH the divider,
     same transparent ground over the same var(--bg) as the chat column's. */
  .drawer-head { display: flex; align-items: center; gap: 8px; min-height: 42px; box-sizing: border-box; padding: 6px 10px; border-bottom: 1px solid var(--border); }
  .win-list { display: flex; gap: 5px; overflow-x: auto; scrollbar-width: none; }
  .win-list::-webkit-scrollbar { display: none; }
  .win-pill { display: flex; align-items: center; gap: 5px; flex: none; background: var(--surface); border: 1px solid var(--border); border-radius: var(--ui-radius-control); color: var(--text2); padding: 4px 9px; font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub); cursor: pointer; }
  .win-pill.cur { border-color: var(--accent); color: var(--accent); background: var(--accent-bg); }
  .direct-tag { font-size: var(--fs-micro); color: var(--text3); border: 1px solid var(--border); border-radius: 4px; padding: 0 4px; margin-left: 3px; }
  .term-body { flex: 1; min-width: 0; min-height: 0; position: relative; display: flex; flex-direction: column; }
  /* The files partition replaces the terminal VISUALLY only: the terminal
     stays laid out under visibility:hidden so its box never changes size —
     a display:none would re-fit cols×rows and make every agent TUI repaint
     (the .keep-rows lesson). */
  .term-body.off { visibility: hidden; position: absolute; inset: 0; }
  .files-body { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg); }
  .board-body { flex: 1; min-width: 0; min-height: 0; display: flex; flex-direction: column; background: var(--bg); }
  .board-body > :global(.board-root) { flex: 1; min-height: 0; }
  /* Parent-owned suppression (board #23, lead): the drawer head is the ONLY
     header this partition may have. The embedded Board renders no page-head
     of its own (its {#if !embedded} gate, pinned by the render test), but the
     drawer is the container that KNOWS the embedding — so it enforces the
     contract too: whatever a prop/HMR/child-path drift might leak, a second
     header can neither show nor keep its height here. */
  .board-body :global(.page-head) { display: none; }
  .d-files { display: flex; align-items: center; gap: 6px; font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub); color: var(--text2); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  /* ONE switcher for the drawer. It used to have two: these pills on top and
     a tmux-style statusline underneath, both listing the same windows and both
     calling pickWindow (owner: "上面和下面有两个 bar…可以把它们合并一下").
     The pills won — they carry the state dot, the direct-window tag and the
     actions — and the statusline's only unique content, the roster count,
     moved up here. */
  .d-count { font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-meta); color: var(--text3); white-space: nowrap; margin-right: 2px; }

  .dlg-backdrop { position: fixed; inset: 0; z-index: 30; background: rgba(0,0,0,0.45); }
  .dlg {
    position: fixed; z-index: 31; left: 50%; top: 50%; transform: translate(-50%, -50%);
    width: min(440px, calc(100vw / var(--ui-zoom, 1) - 32px)); max-height: calc(100vh / var(--ui-zoom, 1) - 48px); overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: 18px;
    box-shadow: 0 18px 60px rgba(0,0,0,0.5); padding: 18px; display: flex; flex-direction: column; gap: 10px;
  }
  .dlg h2 { margin: 0 0 4px; font-size: var(--fs-title); }  /* Phone: dialogs become bottom sheets — reachable with a thumb, and they
     never fight the on-screen keyboard for the middle of the screen. */
  .dlg.sheet {
    left: 0; top: auto; bottom: 0; transform: none;
    width: 100%; max-width: none; max-height: calc(82vh / var(--ui-zoom, 1));
    border-radius: 18px 18px 0 0; border-left: none; border-right: none; border-bottom: none;
    padding: 16px 14px calc(16px + var(--sab, 0px)); /* var(--sab): env() is 0 in the APK */
  }
  .dlg.sheet .dlg-agents { max-height: calc(46vh / var(--ui-zoom, 1)); overflow-y: auto; }
  .dlg.sheet .agent-pick, .dlg.sheet input, .dlg.sheet .dlg-actions button { min-height: 44px; }
  .dlg input { background: var(--input-bg); border: 1px solid var(--input-border); border-radius: var(--ui-radius-control); color: var(--text); padding: 8px 12px; font-size: var(--fs-ui); outline: none; }
  .dlg input:focus { border-color: var(--accent); }
  .dlg-agents { display: flex; flex-direction: column; gap: 5px; }
  .agent-pick { display: flex; align-items: center; gap: 8px; background: var(--surface); border: 1px solid var(--border); border-radius: var(--ui-radius-control); color: var(--text2); padding: 8px 11px; font-size: var(--fs-ui); cursor: pointer; text-align: left; }
  .agent-pick.sel { border-color: var(--accent); color: var(--text); background: var(--accent-bg); }
  .agent-pick :global(svg) { margin-left: auto; color: var(--accent); }
  .dlg-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 6px; }
</style>
