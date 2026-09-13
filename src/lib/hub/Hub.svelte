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
  import Sidebar from './Sidebar.svelte';
  import Roster from './Roster.svelte';
  import Composer from './Composer.svelte';
  import Feed from './Feed.svelte';
  import Drawer from './Drawer.svelte';
  import { copyText } from '../core/clipboard.ts';
  import OperationFeedback from '../ui/OperationFeedback.svelte';
  import { createFeedbackLifetime } from '../ui/feedback-lifetime.ts';
  import { feedbackPosition } from '../ui/feedback-position.ts';
  import Lightbox from '../ui/Lightbox.svelte';
  import CommandButton from '../ui/CommandButton.svelte';
  import './hub-atoms.css';
  import Icon from '../ui/Icon.svelte';
  import { onDestroy, tick as settled } from 'svelte';
  import { t } from '../core/i18n.svelte.ts';
  import {
    projectList, projectUp, projectDown, projectDelete, projectArchive, projectCreate, projectRename, listSessionsWithPanes,
    hubPost, hubCommand, modelsList, hubLog, hubRooms, hubAgents, fsMkdir, fsUpload, fsCwd, hubSpawn, hubSpawnTeam, teamsList, hubAgentStop, hubAgentRestart, hubActivity, hubAgentRemove, hubAgentInterrupt, registryList,
    addTeamMessageListener, removeTeamMessageListener,
  } from '../core/ws.ts';
  import { sortRows } from '../projects/projects.ts';
  import { stateDotColor, mergeMessages, mergeEvents, backendColor, feedBlocks, filterBlocks, mergeStates, pickLead, addressed, unreadSenders, stoppedAgents, slashCommand, uploadImagePath, uploadFilePath, imageId } from './hub.ts';
  import { resolvePathRef } from '../core/path-links.ts';
  import { ALL_TARGET, attachmentBody, attachToken, busyTargetsFor } from './hub-composer.ts';
  import { walkFeedGap } from './hub-history.ts';
  import { createHubBackRegistry } from './hub-back.ts';
  import { notifyNews, isAway, roomProjectName } from './notifications.ts';
  import { backendIcon } from '../core/agents.ts';
  import { anchorOf } from '../ui/placement.ts';
  import ContextMenu from '../ui/ContextMenu.svelte';
  import { revealMs } from '../ui/motion.ts';
  import { hubPrefs } from './hub-prefs.svelte.ts';
  import { pinTrack, moveTrack } from './reveal.ts';
  import CreateProjectDialog from '../projects/CreateProjectDialog.svelte';
  import ConfirmDialog from '../ui/ConfirmDialog.svelte';
  import { activeModal } from '../ui/modal.ts';

  let { visible = false, fontSize = 14, mobile = false, openTerminal = () => {}, onSelectSession = (_s) => {}, onGoBack = null, openAgentConfig = null, openFilesTab = null, openBoardTab = null } = $props();
  const drawerId = $props.id();

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
  /** Project ids present when the sidebar first filled (motion.md principle
   * 13): the first fill is not news, a project that joins later fades in. */
  let rowsBase = $state(null);
  // The recycle bin: archived projects, folded at the sidebar's bottom.
  let trash = $state([]);           // ProjectRow[] (archived)
  let trashAsk = $state(null);      // row pending PERMANENT delete (the only irreversible step)
  let panes = $state([]);           // all tmux panes
  let talkMap = $state({});         // room -> newest message ts (ms) — sidebar row times
  let agentStates = $state({});     // "<session>:<window>" -> derived state, all projects
  let selected = $state('');        // selected project session
  let selectionGeneration = 0, alive = true;
  onDestroy(() => { alive = false; selectionGeneration++; });
  let agents = $state([]);          // HubAgent[] for selected session (all windows)
  let feed = $state([]);            // chat messages, oldest first
  let activity = $state([]);        // telemetry events (in-memory ring on the server)
  let lastActivityTs = 0;
  let registry = $state([]);        // RegAgent[]
  let teams = $state([]);           // RegTeam[] — configured agent teams (board #74)
  // Three ways a message can land, and they are NOT variations of one thing:
  //   a name    → typed into that agent's input; exactly one agent is
  //               interrupted and starts a turn. The default (the lead).
  //   ALL_TARGET→ `@all`: typed into EVERY managed agent's input. Every agent
  //               starts a turn at once, so this is a deliberate act, not a
  //               casual default — it is the expensive one.
  //   ''        → recorded in the room and delivered to NOBODY. Agents read it
  //               when they next call `tmm log`, which is how you leave context
  //               without interrupting anyone mid-task.
  let composerText = $state('');
  // Who the composer addresses. Defaults to the project's lead (pickLead), so
  // talking to your lead agent needs no @ ceremony.
  let recipient = $state('');
  let composer = $state(null);
  let feedView = $state(null);
  let feedActions = $state.raw(null);
  function registerFeedActions(handlers) {
    feedActions = handlers;
    return () => { if (feedActions === handlers) feedActions = null; };
  }

  // Terminal drawer (closed by default — the whole point).
  let termOpen = $state(false);
  // The drawer REVEALS and withdraws instead of appearing (board #174; the
  // technique is reveal.ts): while it closes, the Drawer stays mounted at its
  // pinned width and the grid track shrinks over it, so `drawerShown` — the
  // mount gate — is the intent OR a close in flight. `.drawer-open`, the rest
  // class the track factor follows, is the intent alone.
  let drawerClosing = $state(false);
  const drawerShown = $derived(termOpen || drawerClosing);
  let colsEl = $state(null);
  let drawerTrackEl = $state(null);
  // The project sidebar collapses and expands the same way (owner, board
  // #174: "左侧侧边栏可以加一个折叠展开的按钮 … 折叠展开最好是有动画，不是直接
  // 跳"). The state IS the app-wide preference; the desktop-only class puts
  // the track factor at 0 and hides the content at rest.
  let sideTrackEl = $state(null);
  const sideCollapsed = $derived(hubPrefs.sidebarCollapsed);
  /** Collapse: pin at the current width, rest state, move from 1. Expand:
   * rest state, flush, pin at the FINAL width, move from 0 — placed first,
   * then revealed (reveal.ts). Both through the reading anchor: the chat
   * column changes width either way. */
  async function setSidebar(collapsed) {
    if (compact || collapsed === sideCollapsed) return;
    if (!colsEl || !sideTrackEl) { hubPrefs.setSidebarCollapsed(collapsed); return; }
    if (collapsed) {
      const unpin = pinTrack(sideTrackEl, 'end');
      await withReadingAnchor(() => hubPrefs.setSidebarCollapsed(true));
      await moveTrack(colsEl, '--side-open', 1);
      unpin();
    } else {
      await withReadingAnchor(() => hubPrefs.setSidebarCollapsed(false));
      const unpin = pinTrack(sideTrackEl, 'end');
      await moveTrack(colsEl, '--side-open', 0);
      unpin();
    }
  }
  // What the drawer SHOWS: the terminal, or the file browser (owner,
  // 2026-08-28: "右侧边栏，可以展开文件浏览器的分区，类似展示 terminal 面板
  // 一样的逻辑"). One drawer, one width handle, two bodies — the hidden one
  // keeps visibility:hidden (never display:none: a re-laid-out terminal
  // would resize the pane and make the agent repaint, the .keep-rows story).
  let drawerView = $state('term');
  let drawerIssueReq = $state(null); // a feed board-line tap on desktop: open the issue in the drawer
  let drawerFilesReq = $state(null); // a feed path-reference tap: preview the file in the drawer (board #99)
  let drawerFilesBack = null;

  async function routePathRef(raw) {
    const target = selected;
    let file = raw;
    // A relative reference is relative to the PROJECT — the same base the
    // agents' own paths mean. (~ passes through; the server expands it.) The
    // declared path is that base (board #181); only a session without a
    // declaration falls back to asking where its active pane is.
    if (!file.startsWith('/') && !file.startsWith('~')) {
      const base = projectPath;
      if (base) file = resolvePathRef(base, file);
      else { try { const r = await fsCwd(target); if (r.path) file = resolvePathRef(r.path, file); } catch {} }
    }
    if (selected !== target) return;
    if (mobile || compact) { openFilesTab?.(target, '', file); return; }
    drawerFilesReq = { file, n: (drawerFilesReq?.n ?? 0) + 1 };
    drawerView = 'files'; openDrawer();
  }
  let drawerBoardNew = $state(null); // the drawer head's + (board #23): new issue, requested into the embedded Board
  let drawerFilesDir = $state(''); // where the drawer's Files is — the jump hands it over
  let termTarget = $state('');
  let termCommand = $state('');
  // The switcher bar folds non-agent windows (board #92: "只 filter 出当前有效
  // 的 agent window，其他 window 可以帮我折叠起来"): shells and other windows
  // hide behind a +N pill so they never push the agent pills out of the bar.
  // The one exception is the window the terminal is SHOWING — the bar may
  // never hide the current pane, so it stays a pill even when folded-class.
  let winsExpanded = $state(false);

  // New-project dialog.
  let createOpen = $state(false);


  const room = (session) => `proj:${session}`;
  let lastTs = 0;

  const selectedRow = $derived(rows.find((r) => r.project.session === selected) ?? null);
  /** The project's DECLARED path — where Files opened from this room starts
   * and what a relative path reference resolves against (board #181, owner
   * 2026-09-12: "你应该默认从我们选定的项目路径去跑"). The active pane's cwd is
   * an accident of what was last touched in tmux; it stays the explicit
   * Session-directory tool and the fallback for a session with no project. */
  const projectPath = $derived(selectedRow?.project.path ?? '');
  const liveSelected = $derived(!!selectedRow?.live);
  const managedAgents = $derived(agents.filter((a) => a.managed));
  // The names `deliver_mentions` can type into — the roster the room-note
  // verdict and the composer chip's `+@name` read against.
  const managedNames = $derived(managedAgents.map((a) => a.name));
  // Declared but not running — a stopped agent still belongs to the room.
  const stopped = $derived(stoppedAgents(selectedRow?.slots, managedAgents));

  let projectReadSequence = 0, rosterReadSequence = 0;
  async function reload(report = false) {
    const request = ++projectReadSequence;
    try {
      // The sidebar is ordered by CONVERSATION, so the list needs one more fact:
      // when each room last had a message. One grouped query server-side.
      const [{ projects }, sp, roomsRes] = await Promise.all([
        projectList(true),
        listSessionsWithPanes(),
        hubRooms().catch(() => ({ rooms: {}, states: {} })),
      ]);
      if (!alive || request !== projectReadSequence) return;
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
      if (!rowsBase && rows.length) rowsBase = new Set(rows.map((r) => r.project.id));
      panes = sp.panes ?? [];
      // First load: go back to the conversation that was open, and only fall
      // back to the top row when that project is gone.
      if (!selected && rows.length) {
        const remembered = rows.some((r) => r.project.session === hubPrefs.project)
          ? hubPrefs.project
          : rows[0].project.session;
        selectProject(remembered);
      }
    } catch (e) { if (report && alive && request === projectReadSequence) throw e; }
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
  /** Motion gates (motion.md principle 13: a whole list mounting at once never
   * animates). `openedAt` is the newest block timestamp once the room's first
   * load has landed — only a block NEWER than it rises in (`.appear-rise`);
   * history, an older page and a cached room are already there. `rosterBase`
   * is the set of agent names present at that same moment — a card that
   * JOINS the roster later pops in, a project switch replays nothing. Both
   * are reset to "nothing is fresh" while a room is loading. */
  let openedAt = $state(Infinity);
  let rosterBase = $state(null);
  /** A load unfolds (motion.md principle 15, wave 8): while an UNCACHED room
   * loads (`!roomReady`) the feed and roster show skeletons of the coming
   * shape (`.skel-wrap` keeps them invisible for 150ms, so a fast answer never
   * flashes one); when the room settles, `justLoaded` puts `.reveal-tail` on
   * the feed and `.reveal` on the roster for ONE unfold. It is cleared by a
   * timer rather than left on: the atoms animate every child that MOUNTS
   * while the class is present, and an older page prepended by `loadOlder`
   * (or the empty-room panel) must not rise (principle 13) — the timer runs
   * one --t-move plus the atom's longest stagger (210ms) and a margin, and is
   * zero-plus-margin under reduced motion, where the atom is inert anyway.
   * A cached room never sets it: its content is already there. */
  let justLoaded = $state(false);
  let justLoadedTimer = null;
  function unfold() {
    clearTimeout(justLoadedTimer);
    justLoaded = true;
    justLoadedTimer = setTimeout(() => { justLoaded = false; }, revealMs());
  }

  async function selectProject(session) {
    headerCopyLifetime.clear();
    selectionGeneration++;
    actionReadError = ''; actionRefreshing = false;
    // An unsent line belongs to the conversation it was written for. Park it on
    // the project we are leaving and pick up whatever was waiting in the new one
    // — carrying the text across would put it in front of the wrong agents.
    if (selected) hubPrefs.setDraft(selected, composerText);
    // Park the room too, so switching back is instant.
    if (selected) roomCache.set(selected, { feed, lastTs, activity, lastActivityTs, agents, histSeq, histMore, actCursor, actMore });
    selected = session;
    openedAt = Infinity; rosterBase = null; // nothing is fresh until this room has settled
    clearTimeout(justLoadedTimer); justLoaded = false; // an unfold belongs to the room that loaded
    composerText = hubPrefs.draft(session);
    clearAttachments(); // staged for one room; must not ride into another
    feedView?.resetForRoom();
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
    filterAgent = ''; // a filter is a reading choice, scoped to its room
    // The drawer follows the project (board #23, owner: "chat的右侧边栏打开
    // 哪个的状态前端帮我记住，这样我切换不同的 project 回来原来的视图还在"):
    // whichever partition was open when the user LEFT this room reopens on
    // return, and a room where it was closed comes back closed. No reading
    // anchor here — selectProject lands at the tail regardless.
    const dv = compact ? '' : hubPrefs.drawer(session);
    if (dv) drawerView = dv;
    termOpen = !!dv;
    // The old room's pane must never leak into this one's terminal partition
    // — and the unfold is a transient reading of THAT room's bar (board #92).
    termTarget = ''; termCommand = ''; winsExpanded = false;
    if (dv === 'term') {
      const pick = agents.find((x) => x.managed) ?? agents[0];
      if (pick) pickWindow(pick);
    }
    // Entering a room lands at its tail, cached or not — a parked scrollTop
    // from the LAST room would point at arbitrary content in this one.
    following = true;
    if (feed.length) scrollFeed(true);
    await Promise.all([loadFeed(), loadAgents(), loadActivity()]);
    if (selected === session) {
      // An uncached room's first fill unfolds (the skeleton has been standing
      // in for it); a cached one was already on screen and stays a cut.
      if (!c) unfold();
      roomReady = true;
      // The room has settled: from here on, what arrives is news and moves.
      openedAt = blocks.reduce((m, b) => Math.max(m, b.ts), 0);
      rosterBase = new Set([...managedAgents.map((a) => a.name), ...stopped]);
    }
    // A restored terminal partition may have had NO roster to pick from (a
    // first visit after reload restores before any cache exists) — seat it
    // once the fresh roster is in.
    if (selected === session && termOpen && drawerView === 'term' && !termTarget) {
      const pick = agents.find((x) => x.managed) ?? agents[0];
      if (pick) pickWindow(pick);
    }
  }

  async function loadFeed(report = false) {
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
      const floorTs = lastTs;
      const res = await hubLog(s, lastTs, 100);
      if (selected !== s) return;
      const messages = res.messages;
      if (first) {
        histSeq = res.oldest_seq ?? 0;
        histMore = (res.has_more ?? false) && histSeq > 0;
      } else if (res.has_more && (res.oldest_seq ?? 0) > 0) {
        // More than a page arrived since the cursor (a room parked in
        // roomCache while the team talked): the poll's page is the NEWEST 100
        // and `has_more` says newer-than-cursor rows lie behind it. Walk them
        // in before the cursor moves past them — the hole was permanent
        // otherwise (review C, 2026-09-03). Bounded: 50 pages is 5000 messages
        // in one absence, past which the first-load page is the honest answer.
        const walked = await walkFeedGap({ session: s, floorTs, cursor: res.oldest_seq }, {
          readPage: (session, cursor) => hubLog(session, 0, 100, cursor),
          stillCurrent: (session) => selected === session,
          mergePage: (newer) => { feed = mergeMessages(feed, newer); },
        });
        // The helper return adds an await boundary before adopting the poll batch.
        if (!walked || selected !== s) return;
      }
      if (messages?.length) {
        feed = mergeMessages(feed, messages);
        lastTs = Math.max(lastTs, ...messages.map((m) => m.ts ?? 0));
        // New-message notification (board #57): a poll batch that lands while
        // the reader is AWAY — tab hidden or window unfocused — plays the cue
        // and raises a system notification. The first page is history, never
        // news; the gate drops the rest. This is the FALLBACK site: the poll
        // only runs while this page is visible, so the push handler below is
        // where an off-screen reader is actually reached (board #72); sift's
        // seen keys make the two alert once.
        notifyNews(messages, { first, away: isAway(visible), project: s });
        if (following) scrollFeed(); else newBelow = true;
      }
    } catch (e) { if (report) throw e; /* hub not available */ }
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

  async function loadAgents(report = false) {
    if (!selected) return;
    const s = selected;
    const request = ++rosterReadSequence;
    try {
      const got = (await hubAgents(s)).agents ?? [];
      // A stale success is as wrong as a stale feed: the OLD project's roster
      // must not dress the NEW project (and then re-pick its recipient).
      if (selected !== s) return;
      if (!alive || request !== rosterReadSequence) return;
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
      if (!alive || request !== rosterReadSequence || selected !== s) return;
      if (report) throw e;
      // "I could not ask" is not "there is nobody". Emptying the roster on a
      // failed poll is what made the cards — and with them the model and context
      // readings — blink away whenever the socket hiccuped or an RPC timed out
      // (owner, 2026-08-19: "有时候会闪没了，是不是中间心跳失败了"). The roster is
      // a last-known state; `selectProject` is the one place that clears it,
      // because that is the one time it is genuinely unknown.
      console.warn('hub agents poll failed, keeping the last roster', e);
    }
  }

  const scrollFeed = (force = false) => feedView?.scrollToTail(force);

  /** The red dot means "an agent replied and you have not looked". So it clears
   * when the newest message is on screen — the bottom of the feed — and when
   * you send, since you are plainly looking then. */
  function markSeen() {
    if (!selected || !visible) return;
    const newest = feed.reduce((max, m) => Math.max(max, m.ts ?? 0), 0);
    if (newest > hubPrefs.seen(selected)) hubPrefs.setSeen(selected, newest);
  }
  const unread = $derived(unreadSenders(feed, hubPrefs.seen(selected)));

  let following = $state(true);   // the feed is parked at the tail
  let newBelow = $state(false);   // something arrived while it was not
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

  async function send() {
    const raw = composerText.trim();
    if ((!raw && !pending.length) || !selected) return;
    // While a stage job is in flight the message is NOT whole yet: sending
    // now would post the text without its attachment, and the attachment
    // would land in an emptied composer as an orphaned token (lead review,
    // board #25). The button is disabled too — this entry guard covers the
    // keyboard's Enter and any future caller.
    if (attaching) return;
    // A failed attachment stays a visible chip until the user removes it: a
    // message that quietly left without the file it showed is the failure
    // mode this exists to prevent (review, 2026-09-03).
    if (failed) return;
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
    const body = attachmentBody(raw, atts);
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
  // A FAILED attachment is a chip too (review, 2026-09-03: an oversized file
  // or a failed upload only console.warned, so the user could not tell what
  // the message would carry): `{ key, name, kind, error }` with no path, no
  // token and no number — rendered in the error state with its reason,
  // removable, and it BLOCKS send until removed. Nothing about an attachment
  // is ever silent. `key` is the each-key: a path for a staged one, a fresh id
  // for a failed one (two failures of the same file are two chips).
  let pending = $state([]);
  let attachSeq = 1;
  const failedAttachment = (f, error) => ({
    key: `err-${imageId()}`, path: '', kind: f.type?.startsWith('image/') ? 'image' : 'file',
    name: f.name, n: 0, thumb: '', error,
  });
  const errText = (err) => String(err?.message ?? err ?? '');

  function removeAttachment(i) {
    const a = pending[i];
    if (!a) return;
    if (a.n) {
      const tok = attachToken(a);
      // Strip the token (and one adjacent space) wherever the user left it.
      composerText = composerText.replace(new RegExp(`\\s?${tok.replace(/[[\\]]/g, '\\$&')}`), '');
    }
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
  // A failed chip is not content: it does not make the composer sendable, and
  // while one is staged nothing sends (send() and the button agree, the same
  // two-gate rule as `attaching`) — the user removes it or re-attaches.
  const failed = $derived(pending.some((a) => a.error));
  const sendable = $derived(!!composerText.trim() || pending.some((a) => !a.error));
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

  async function stageFiles(files) {
    const ws = selectedRow?.project.path;
    if (!ws || !files.length) return;
    // Where the tokens land: the caret's last position (the file dialog
    // blurs the box but the selection survives; a paste's caret is live),
    // else the end.
    let at = composer?.caret() ?? composerText.length;
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
        // Per FILE: one bad file must not take the others down with it, and
        // its failure must land as a chip the user can see and remove — never
        // only in the console (review, 2026-09-03).
        try {
          if (f.type.startsWith('image/')) {
            // Images are re-encoded (webp, capped long edge) — 2(c).
            const { b64, ext } = await encodeImage(f);
            if (stale()) return;
            const path = uploadImagePath(ws, imageId(), ext);
            await fsUpload(path, b64);
            if (stale()) return;
            item = { key: path, path, kind: 'image', name: f.name, n: attachSeq++, thumb: URL.createObjectURL(f) };
          } else {
            // Everything else lands BYTE-IDENTICAL under its own name — 2(a)/3(a).
            if (f.size > FILE_CAP) {
              // No await since the last check, so the verdict still holds.
              pending = [...pending, failedAttachment(f, t('hubAttachTooLarge').replace('{mb}', String(FILE_CAP / 1024 / 1024)))];
              continue;
            }
            const b64 = toB64(new Uint8Array(await f.arrayBuffer()));
            if (stale()) return;
            const path = uploadFilePath(ws, imageId(), f.name);
            await fsUpload(path, b64);
            if (stale()) return;
            item = { key: path, path, kind: 'file', name: f.name, n: attachSeq++, thumb: '' };
          }
        } catch (err) {
          // The throw came out of an await, so the room may have changed under
          // it: a guard, not a return — the loop goes on to the next file.
          if (!stale()) pending = [...pending, failedAttachment(f, t('hubAttachFailed').replace('{err}', errText(err)))];
          continue;
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
      composer?.focus();
    } catch (err) {
      // The uploads dir itself could not be prepared: every file of this job
      // failed, and each says so as a chip (same staleness guard as above).
      if (!stale()) pending = [...pending, ...files.map((f) => failedAttachment(f, t('hubAttachFailed').replace('{err}', errText(err))))];
    } finally {
      // Remove exactly THIS job's entry — never a blanket reset: a stale
      // job's finally must not unlock a job the new room started.
      const i = jobGens.indexOf(gen);
      if (i >= 0) jobGens = [...jobGens.slice(0, i), ...jobGens.slice(i + 1)];
    }
  }

  // The draft survives a reload because a half-written message is work. Written
  // on every keystroke: one small JSON string, and the alternative (a debounce)
  // loses the last few characters exactly when the tab goes away.
  $effect(() => { hubPrefs.setDraft(selected, composerText); });

  // Roster owns card interaction; Hub owns the resulting reading filter.
  let filterAgent = $state('');
  /** Filtering is a ContextMenu verb, independent of the delivery recipient. */
  function toggleFilter(name) {
    filterAgent = filterAgent === name ? '' : name;
  }
  const filterItem = (name) => ({
    label: filterAgent === name ? t('hubFilterExit') : t('hubFilterItem'),
    icon: 'search',
    onselect: () => toggleFilter(name),
  });

  /** Choosing a recipient is also choosing this project's lead: it is the same
   * decision ("who am I working with here"), so it persists. That includes
   * choosing the ROOM (`''`): hubPrefs stores it as a real value, and pickLead
   * keeps it — before, '' was "unset" and the next roster poll re-seated a lead
   * the user had just dismissed (review C, 2026-09-03). */
  function setRecipient(name) {
    recipient = name;
    if (selected) hubPrefs.setLead(selected, name);
    // An OPEN terminal partition follows the selection (board #91): choosing
    // an agent is also choosing whose pane you are watching — the same reading
    // openDrawer makes when it seats the recipient's window (board #76). The
    // lookup is by roster name, so @all and the room skip naturally; a closed
    // drawer stays closed — this retargets, it never opens.
    if (termOpen && drawerView === 'term') {
      const a = agents.find((x) => x.name === name);
      if (a) pickWindow(a);
    }
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
  let interruptJobs = $state.raw([]);
  const interrupting = $derived(interruptJobs.filter((job) => job.session === selected).map((job) => job.name));
  const busyNames = $derived(busyTargetsFor(ALL_TARGET, agents));
  const interruptible = $derived(busyTargetsFor(recipient, agents).length > 0
    && !busyTargetsFor(recipient, agents).some((name) => interrupting.includes(name)));
  const rosterExpanded = $derived(hubPrefs.rosterExpanded(selected));

  async function toggleRoster() {
    const session = selected;
    if (!session) return;
    const next = !rosterExpanded;
    await withReadingAnchor(() => {
      if (selected === session) hubPrefs.setRosterExpanded(session, next);
    });
  }

  async function interrupt(target, session = selected) {
    if (!session || session !== selected) return;
    // Snapshot before the first await. A peer Stop never selects its card,
    // changes the reading anchor, or reaches an idle/unmanaged window.
    const targets = busyTargetsFor(target, agents);
    if (targets.some((name) => interruptJobs.some((job) => job.session === session && job.name === name))) return;
    const jobs = targets.map((name) => ({ session, name }));
    interruptJobs = [...interruptJobs, ...jobs];
    await Promise.all(jobs.map(async (job) => {
      try {
        await hubAgentInterrupt(job.session, job.name);
      } catch (e) {
        console.warn('interrupt failed', job.name, e);
      } finally {
        interruptJobs = interruptJobs.filter((pending) => pending !== job);
      }
    }));
    // The existing push/poll path reads the server's reset-first status and
    // [tmm] interrupted line; acknowledging a request does not invent either.
  }

  async function withReadingAnchor(mutate) {
    if (!feedView) { mutate(); return; }
    return feedView.withReadingAnchor(mutate);
  }

  function watchAgent(agent) {
    drawerView = 'term';
    openDrawer(agent);
  }

  async function openDrawer(a = null) {
    // No explicit agent → the one you are TALKING TO (board #76: "应该优先跳转到
    // 当前所选的 agent 的 terminal window"), then the first managed, then anything.
    const pick = a
      ?? agents.find((x) => x.managed && x.name === recipient)
      ?? agents.find((x) => x.managed)
      ?? agents[0];
    if (pick) {
      const p = panes.find((p) => p.session === selected && p.window === pick.window && p.active)
        ?? panes.find((p) => p.session === selected && p.window === pick.window);
      if (p) {
        termTarget = `${p.session}:${p.window}.${p.pane}`;
        termCommand = p.current_command || '';
      } else {
        termTarget = '';
        termCommand = '';
      }
    }
    if (mobile || (compact && drawerView === 'term')) {
      // Compact layouts have no terminal drawer; use the whole Terminal tab.
      const m = /^(.+):(\d+)\.(\d+)$/.exec(termTarget);
      if (m) openTerminal(selected, termTarget, termCommand);
      return;
    }
    if (termOpen) return; // switching partitions while open is a content swap, not a move
    // Placed first, then revealed (reveal.ts): the rest state lands and the
    // reader is re-anchored at the FINAL layout, all before a paint; only
    // then is the content pinned at that width and the track moved from 0.
    await withReadingAnchor(() => { termOpen = true; });
    hubPrefs.setDrawer(selected, drawerView);
    await revealDrawer(0);
  }

  /** Move the drawer track between 0 and its rest value with the Drawer's
   * content pinned at the width it has now (the measured final width on
   * open, the current width on close) — xterm sees ONE size. */
  async function revealDrawer(from) {
    if (!colsEl || !drawerTrackEl) return;
    const unpin = pinTrack(drawerTrackEl, 'start');
    await moveTrack(colsEl, '--drawer-open', from);
    unpin();
  }

  /** The one close path, so every trigger keeps the reader's place. The close
   * moves too (lead, #174: a cut beside an animated open reads as a stutter):
   * pinned at its current width, the Drawer withdraws under the shrinking
   * track and unmounts after the move — one size until the end. */
  async function closeDrawer() {
    if (!termOpen) return;
    const unpin = drawerTrackEl ? pinTrack(drawerTrackEl, 'start') : null;
    drawerClosing = true;
    await withReadingAnchor(() => { termOpen = false; });
    hubPrefs.setDrawer(selected, '');
    if (colsEl) await moveTrack(colsEl, '--drawer-open', 1);
    drawerClosing = false;
    unpin?.();
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

  /** Member names of a configured team, for the preset rows. */
  function teamMembers(tm) {
    try { return (JSON.parse(tm.members) ?? []).map((m) => (m.team ? `+${m.team}` : m.name)).filter(Boolean); } catch { return []; }
  }

  /** Start a configured TEAM (board #74): one RPC spawns every member with its
   * role; the first member that landed becomes the recipient (the team's own
   * order is its hierarchy — the lead is listed first). */
  async function startTeam(name, brief = '') {
    if (!selected || !name || starting) return;
    starting = true;
    pickerOpen = false;
    try {
      const r = await hubSpawnTeam(selected, name, brief);
      for (const e of r?.errors ?? []) console.warn('team member spawn failed', e.name, e.error);
      await Promise.all([reload(), loadAgents(), loadFeed()]);
      const first = r?.spawned?.[0]?.window_name;
      if (first && (pickerMode === 'start' || !recipient)) setRecipient(first);
    } catch (e) {
      console.warn('team spawn failed', name, e);
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
    stop:   { title: 'hubStopTitle',       note: 'hubStopNote',       go: 'hubStop', icon: 'stop' },
    remove: { title: 'hubRemoveTitle',     note: 'hubRemoveNote',     go: 'hubRemove', icon: 'trash' },
    down:   { title: 'projectDownTitle',   note: 'projectDownNote',   go: 'projectDown', icon: 'stop' },
    delete: { title: 'projectDeleteTitle', note: 'projectDeleteNote', go: 'projectDelete', icon: 'trash' },
  };
  let pendingAct = $state(null);   // { kind: keyof ACT_COPY, name, session }
  let acting = $state(false);
  let actionError = $state(''), purgeError = $state(''), purging = $state(false);
  let actionReadError = $state(''), actionRefreshing = $state(false);
  let actionRefreshSequence = 0;
  /** Freeze the TARGET at ask time. `name` is what the dialog shows; `session`
   * is what the action runs on. The context menu opens on ANY row, so the verb
   * must carry that row's identity — resolving `selected` at confirm time
   * closed whichever project happened to be open, not the one long-pressed
   * (owner, 2026-08-24: "关的不是我选中的 是其他的"). Agent verbs keep the
   * default: the roster only shows the selected project's agents. */
  const askAction = (kind, name, session = selected) => {
    if (acting || purging) return;
    const row = rows.find(r => r.project.session === session);
    actionError = '';
    pendingAct = { kind, name, session, projectId: row?.project.id, closed: !row?.live };
  };
  async function refreshActionView(afterMutation = false) {
    if (actionRefreshing && !afterMutation) return;
    const generation = selectionGeneration;
    const request = ++actionRefreshSequence;
    const current = () => alive && generation === selectionGeneration && request === actionRefreshSequence;
    actionRefreshing = true;
    try {
      await Promise.all([reload(true), loadAgents(true), loadFeed(true)]);
      if (current()) actionReadError = '';
    }
    catch (e) {
      if (current())
        actionReadError = t('operationRefreshFailed').replace('{error}', String(e?.message ?? e));
    } finally { if (current()) actionRefreshing = false; }
  }

  async function runAction() {
    if (!pendingAct || acting) return;
    const act = pendingAct, generation = selectionGeneration;
    const { kind, name, session, projectId } = act;
    let step = t(ACT_COPY[kind].go);
    acting = true; actionError = '';
    try {
      if (kind === 'down' || kind === 'delete') {
        if (!projectId) throw new Error(t('operationUnavailable'));
        // Delete is close then archive, not purge. Remember the completed
        // step on this captured intent so a retry never closes it again.
        if (kind === 'down' || !act.closed) {
          step = t('projectDown');
          await projectDown(projectId);
          act.closed = true;
        }
        if (kind === 'delete') {
          step = t('projectArchive');
          await projectArchive(projectId, true);
        }
      } else if (kind === 'remove') {
        await hubAgentRemove(session, name);
      } else {
        await hubAgentStop(session, name);
      }
    } catch (e) {
      if (alive && pendingAct === act)
        actionError = t('operationStepFailed').replace('{action}', step).replace('{error}', String(e?.message ?? e));
      return;
    } finally { if (alive) acting = false; }
    if (!alive || pendingAct !== act) return;
    pendingAct = null;
    if (generation !== selectionGeneration) return;
    if (kind === 'delete' && session === selected) selected = '';
    await refreshActionView(true);
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
    if (!row || purging) return;
    const generation = selectionGeneration;
    purging = true; purgeError = '';
    try {
      await projectDelete(row.project.id);
    } catch (e) {
      if (alive && trashAsk === row) purgeError = String(e?.message ?? e);
      return;
    } finally { if (alive) purging = false; }
    if (!alive || trashAsk !== row) return;
    trashAsk = null;
    if (generation === selectionGeneration) await refreshActionView(true);
  }

  /** Restart a live agent, or bring a stopped one back. The RPC tolerates there
   * being no window, so both labels share one implementation and both resume
   * the agent's own conversation rather than opening a blank prompt. */
  async function restartAgent(name, failure = 'restart failed') {
    if (!selected || acting) return;
    acting = true;
    try {
      await hubAgentRestart(selected, name);
      await Promise.all([reload(), loadAgents(), loadFeed()]);
      setRecipient(name);
    } catch (e) {
      console.warn(failure, e);
    } finally {
      acting = false;
    }
  }
  const startAgent = (name) => restartAgent(name, 'start failed');

  // Live pushes + polling while visible.
  const onPush = (m) => {
    // New-message notification, PRIMARY site (board #72): a push arrives for
    // EVERY project room and whatever page is on screen — the poll above does
    // not (it stops with the page). A message in another project's room is
    // away by definition (the reader is not looking at THAT conversation);
    // the selected room asks the live document. Messages are never "first"
    // here: a push is by construction newer than any loaded page.
    if (m?.room) {
      notifyNews([m], {
        first: false,
        away: !selected || m.room !== room(selected) || isAway(visible),
        project: roomProjectName(rows, m.room),
      });
    }
    if (!selected || m?.room !== room(selected)) return;
    feed = mergeMessages(feed, [m]);
    lastTs = Math.max(lastTs, m.ts ?? 0);
    if (following) scrollFeed(); else newBelow = true;
    // A message is a turn edge — the sender's state just changed — so the
    // roster is refreshed ahead of the 5 s poll; but a burst of 30 pushes was
    // 30 hub_agents calls (review C, 2026-09-03). One trailing 300 ms window
    // keeps the immediacy and collapses the burst into one call.
    clearTimeout(agentsRefresh);
    agentsRefresh = setTimeout(() => { agentsRefresh = 0; loadAgents(); }, 300);
  };
  let agentsRefresh = 0;
  $effect(() => {
    addTeamMessageListener(onPush);
    return () => { removeTeamMessageListener(onPush); clearTimeout(agentsRefresh); agentsRefresh = 0; };
  });
  $effect(() => {
    if (!visible) return;
    reload();
    registryList().then((r) => { registry = r.agents ?? []; }).catch(() => {});
    teamsList().then((r) => { teams = r.teams ?? []; }).catch(() => {});
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
      if (activeModal(document)) return;
      if (e.target?.closest?.('[role="menu"]')) return;
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

  function activateAll(event) {
    const session = selected;
    if (!session) return;
    if (recipient !== ALL_TARGET) { setRecipient(ALL_TARGET); return; }
    if (ctxAt?.allSession === session) { closeCtx(); return; }
    const trigger = event.currentTarget;
    const allMenuId = Symbol('all-menu');
    openCtx({ anchor: anchorOf(trigger), trigger, keepTriggerClear: true, allSession: session, allMenuId },
      t('hubEveryone'), allItems(session, allMenuId));
  }
  function allItems(session, menuId) {
    const current = () => selected === session && recipient === ALL_TARGET && ctxAt?.allMenuId === menuId;
    return [
      { label: t('hubRecordOnly'), icon: 'chat', onselect: () => {
        if (current()) setRecipient('');
      } },
      ...(busyNames.length ? [{
        label: t('hubInterrupt'), icon: 'stop', warn: true, disabled: !interruptible,
        onselect: () => { if (current()) return interrupt(ALL_TARGET, session); },
      }] : []),
    ];
  }
  const visibleCtxItems = $derived(ctxAt?.allSession ? allItems(ctxAt.allSession, ctxAt.allMenuId) : ctxItems);
  $effect(() => {
    if (ctxAt?.allSession && (ctxAt.allSession !== selected || recipient !== ALL_TARGET)) closeCtx();
  });

  // ── The phone's BACK GESTURE, the Files page's contract (owner, 2026-08-24:
  // "chat…对于返回手势适配不太好 像是网页刷新了。像文件管理页面就很好"): App
  // routes a history pop here, and a `true` means it was CONSUMED by peeling
  // the topmost layer in the fixed Back priority (separate from key listeners).
  // Only when nothing is left to peel does it fall through to App's re-push,
  // so a back never looks like the browser leaving. On a phone the project
  // LIST is the level above the conversation (the Files analogy: cwd = '/'
  // is the floor); with the list open, back has reached the floor.
  // The lightbox is the topmost transient layer: back peels it first.
  let shotView = $state('');
  const backLayers = createHubBackRegistry();
  $effect(() => {
    if (!onGoBack) return;
    const disposers = [
      backLayers.register('lightbox', () => { if (shotView) { shotView = ''; return true; } return false; }),
      backLayers.register('contextMenu', () => { if (ctxAt) { closeCtx(); return true; } return false; }),
      backLayers.register('action', () => { if (!pendingAct) return false; if (!acting) pendingAct = null; return true; }),
      backLayers.register('trash', () => { if (!trashAsk) return false; if (!purging) trashAsk = null; return true; }),
      backLayers.register('picker', () => { if (pickerOpen) { pickerOpen = false; return true; } return false; }),
      backLayers.register('create', () => { if (createOpen) { createOpen = false; return true; } return false; }),
      backLayers.register('rename', () => { if (renaming) { renaming = false; return true; } return false; }),
      backLayers.register('filter', () => { if (filterAgent) { filterAgent = ''; return true; } return false; }),
      backLayers.register('files', () => { if (termOpen && drawerView === 'files' && drawerFilesBack?.()) return true; return false; }),
      backLayers.register('drawer', () => { if (termOpen) { closeDrawer(); return true; } return false; }),
      backLayers.register('sidebar', () => { if (compact && !sideOpen) { sideOpen = true; return true; } return false; }),
    ];
    onGoBack(backLayers.back);
    return () => { for (const dispose of disposers) dispose(); };
  });

  /** An agent's verbs — shared by its card and context menus. */
  /** One order for every agent menu — rising consequence, destructive last
   * (owner, 2026-08-25: "停止删除应该靠后"), interrupt in the warn tone. */
  function agentItems(name) {
    const session = selected;
    const config = openAgentConfig
      ? [{ label: t('hubAgentConfig'), icon: 'gear', onselect: () => openAgentConfig(name) }]
      : [];
    if (stopped.includes(name)) {
      return [
        { label: t('hubStartAgain'), icon: 'refresh', onselect: () => startAgent(name) },
        filterItem(name),
        ...config,
        { label: t('hubRemove'), icon: 'trash', danger: true, onselect: () => askAction('remove', name) },
      ];
    }
    const a = managedAgents.find((x) => x.name === name);
    return [
      // The current recipient's card offers to stop addressing it (Record
      // only leads, as for All); any other card offers to talk to it (#196).
      recipient === name
        ? { label: t('hubRecordOnly'), icon: 'chat', onselect: () => { if (recipient === name) setRecipient(''); } }
        : { label: t('hubTalkTo'), icon: 'chat', onselect: () => setRecipient(name) },
      { label: t('hubWatch'), icon: 'terminal', onselect: () => { if (a) watchAgent(a); } },
      filterItem(name),
      ...config,
      ...(busyTargetsFor(name, agents).length
        ? [{ label: t('hubInterrupt'), icon: 'stop', warn: true, disabled: interrupting.includes(name), onselect: () => interrupt(name, session) }]
        : []),
      { label: t('hubRestart'), icon: 'refresh', onselect: () => restartAgent(name) },
      { label: t('hubStop'), icon: 'stop', danger: true, onselect: () => askAction('stop', name) },
      { label: t('hubRemove'), icon: 'trash', danger: true, onselect: () => askAction('remove', name) },
    ];
  }

  /** A project's verbs. Open/Close mirrors the header's single button, so the two
   * cannot disagree about which one applies. Rename selects the project first,
   * because the editor it opens is the chat header's own title. */
  function projectItems(row, withView = false) {
    const session = row?.project?.session ?? '';
    const name = row?.project?.name ?? '';
    // Constructive verbs lead, destructive close the menu (owner, 2026-08-25):
    // Open (when closed) → Rename → [view] → Close (when live) → Delete.
    return [
      ...(row?.live ? [] : [{ label: t('projectUp'), icon: 'zap', onselect: () => { selectProject(session); setTimeout(bringUp, 0); } }]),
      { label: t('projectRename'), icon: 'edit',
        onselect: () => { selectProject(session); setTimeout(startRename, 0); } },
      ...(withView ? feedLevelItems() : []),
      ...(row?.live ? [{ label: t('projectDown'), icon: 'stop', danger: true, onselect: () => askAction('down', name, session) }] : []),
      { label: t('projectDelete'), icon: 'trash', danger: true,
        onselect: () => askAction('delete', name, session) },
    ];
  }

  /** The feed's detail level as three menu rows — chat / + status / + tools —
   * the current one ticked. It lives in the project TITLE's menu (the one
   * menu about "this conversation") rather than as a header switch: the
   * header keeps no spare switches (board #72), and Settings was the only
   * other way to reach it (review, 2026-09-03: dead code in the Hub, a
   * setting three pages away). Radio semantics through the two icons the menu
   * already has — a tick on the chosen level, a hollow circle on the rest. */
  function feedLevelItems() {
    const levels = [['chat', 'hubFeedChat'], ['status', 'hubFeedStatus'], ['tools', 'hubFeedTools']];
    return levels.map(([level, key]) => ({
      label: `${t('hubFeedLevel')} · ${t(key)}`,
      icon: hubPrefs.feedLevel === level ? 'check' : 'circle',
      onselect: () => hubPrefs.setFeedLevel(level),
    }));
  }


  // The same dismissal order for Copy/Raw and the command palette. Both used to
  // stay up until something happened to replace them (owner, 2026-08-22:
  // "在其他操作之后应该自动隐藏 不应该一直常驻显示"). A tap anywhere outside
  // the layer (or Escape) closes them; the toggles themselves and clicks
  // INSIDE the layer are excluded so choosing an option is not also
  // "outside". Raw view is not a popup — an opened raw source stays until
  // retoggled or the project changes.
  $effect(() => {
    if (!feedActions?.isOpen() && !composer?.hasTransient()) return;
    const onDown = (e) => {
      feedActions?.outside(e);
      composer?.dismissOutside(e);
    };
    const onKey = (e) => {
      if (e.key !== 'Escape') return;
      feedActions?.escape(e);
      composer?.dismissEscape(e);
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onKey, true);
    return () => {
      window.removeEventListener('pointerdown', onDown, true);
      window.removeEventListener('keydown', onKey, true);
    };
  });

  // Messages remain a record: Copy/Raw belong under the bubble, never in
  // an action menu with delete (hub-feed.md).
  // `windowOf` is what lets a reply close the lane it belongs to, so two agents
  // working at once keep ONE growing group each instead of interleaving.
  const blocks = $derived.by(() => {
    const all = feedBlocks(feed, activity, hubPrefs.feedLevel, (from) => agents.find((a) => a.name === from)?.name);
    if (!filterAgent) return all;
    // Replies, addressed messages and this agent's telemetry lane:
    // the reading rule lives in filterBlocks.
    return filterBlocks(all, filterAgent, agents.find((a) => a.name === filterAgent)?.name);
  });

  /** Four states, one word each. Anything unexpected shows itself rather than
   * being silently relabelled. */
  function stateLabel(state) {
    const label = t('hubState_' + state);
    return label.startsWith('hubState_') ? state : label;
  }

  // ── Hover explains (motion.md principle 16, wave 9). ONE shared card
  // (`use:hoverInfo`, ui/HoverCard) on a pointer device only — the action
  // ignores touch, where the long-press menu is the "more" gesture. Every
  // getter runs at OPEN time, so the card reads the current state; every
  // value goes through the formatter the page already uses (stateLabel,
  // modelLabel, fmtElapsed for the epoch-seconds `since`, agoShort for ms
  // timestamps) — a second formatter would be a second clock.
  /** The state's tone in the card is the SAME family stateDotColor paints. */
  function stateTone(state) {
    switch (stateDotColor(state)) {
      case 'var(--accent)': return 'accent';
      case 'var(--status-warn)': return 'warn';
      case 'var(--status-danger)': return 'danger';
      default: return undefined;
    }
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

  /** The header path remains prose: drag selects it, while a double-click
   * copies the complete value (including the off-screen part) through the
   * app's clipboard helper. An action keeps the span's read-only semantics —
   * it is not a fake button — and lets the browser perform its normal text
   * selection before we expand the highlight to the whole copied path. */
  let headerCopyFeedback = $state(null), headerCopyAnchor = $state(null);
  const headerCopyLifetime = createFeedbackLifetime(value => { headerCopyFeedback = value; });
  $effect(() => { if (!visible) headerCopyLifetime.clear(); });
  onDestroy(() => headerCopyLifetime.dispose());
  function doubleClickCopy(el, initialValue) {
    let value = initialValue;
    const onDoubleClick = () => {
      if (!value || !visible) return;
      const attempt = headerCopyLifetime.begin(); headerCopyAnchor = el;
      void copyText(value).then(ok => headerCopyLifetime.update(attempt,
        { kind: ok ? 'success' : 'error', message: ok ? t('copied') : t('copyFailed') }));
      const selection = window.getSelection();
      if (!selection || !el.isConnected) return;
      const range = document.createRange();
      range.selectNodeContents(el);
      selection.removeAllRanges();
      selection.addRange(range);
    };
    el.addEventListener('dblclick', onDoubleClick);
    return {
      update(nextValue) { if (value !== nextValue) headerCopyLifetime.clear(); value = nextValue; },
      destroy() { el.removeEventListener('dblclick', onDoubleClick); headerCopyLifetime.clear(); if (headerCopyAnchor === el) headerCopyAnchor = null; },
    };
  }

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

<!-- The ONE sidebar collapse/expand control (board #174): a snippet, so the
     sidebar's head (open) and the header's left edge (collapsed) render the
     same button — one glyph that turns (motion.md 4), never two icons. -->
{#snippet sideToggle()}
  <CommandButton variant="icon" icon="chevron-right" expanded={!sideCollapsed} inside controls="hub-sidebar"
    label={sideCollapsed ? t('hubSidebarExpand') : t('hubSidebarCollapse')}
    onclick={() => setSidebar(!sideCollapsed)} />
{/snippet}
<div class="hub-root" class:compact class:drawer-open={termOpen && !compact} class:side-collapsed={sideCollapsed && !compact}>
  <div class="cols" bind:this={colsEl}>
    <!-- The .track is the grid item the reveal pins; the Sidebar inside never
         changes size while the track moves (board #174). -->
    <div class="track side" bind:this={sideTrackEl}>
    <Sidebar {compact} open={sideOpen} {rows} {trash} {rowsBase} {selected}
      {panes} {agentStates} {talkMap} {tick} unreadCount={unread.size} collapse={compact ? null : sideToggle}
      onselect={(session) => { selectProject(session); sideOpen = false; }}
      oncreate={() => { createOpen = true; sideOpen = false; }}
      onclose={() => { sideOpen = false; }}
      onmenu={(row, at) => openCtx(at, row.project.name, projectItems(row))}
      onrestore={restoreProject} onpurge={(row) => { if (!purging) { purgeError = ''; trashAsk = row; } }} />
    </div>

    <!-- ── Main: the conversation ─────────── -->
    <main class="mid">
      <div class="page-head chat-head">
        <!-- The phone reaches the project list here, as a drawer. No chip strip:
             separate conversations are chosen deliberately, not flicked past. -->
        {#if compact}
          <CommandButton variant="icon" icon="menu" label={t('hubProjects')}
            expanded={sideOpen} controls="hub-sidebar" onclick={() => sideOpen = true} />
        {/if}
        <!-- Collapsed, the way back to the project list is the same control
             at the header's left edge — where the phone's menu button
             stands (board #174). -->
        {#if !compact && sideCollapsed}{@render sideToggle()}{/if}
        <!-- Name and menu stay one tight group (board #32). The name is
             selectable prose; its sibling command owns the full hit target. -->
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
              <!-- Rename remains an explicit project-menu action. -->
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
                 出的下拉标签选项，这样对于人类更好理解").
                 Opens the SAME projectItems menu the sidebar row's
                 long-press/right-click speaks. -->
            <CommandButton variant="icon" icon="chevron-down" label={t('hubProjectMenu')} hasPopup="menu"
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
                  selectedRow?.project.name ?? '', projectItems(selectedRow, true));
              }} />
          {/if}
        </div>
        <!-- The FULL path, not a middle-elided stub: it renders whole when it
             fits, and when it doesn't the box scrolls (wheel included) instead
             of ellipsizing — "不要直接用省略号" (owner, 2026-08-20). Desktop
             only, same as before. -->
        {#if !compact}<span class="path" use:wheelX use:doubleClickCopy={selectedRow?.project.path ?? ''} title={selectedRow?.project.path ?? ''}>{selectedRow?.project.path ?? ''}</span>{/if}
        <span class="spacer"></span>
        <!-- Shared commands own paint, accessible names and hover. Desktop
             discloses a partition; compact layouts navigate to its page. -->
        <!-- The message-notification switch that stood here moved to Settings
             (board #72): a header keeps no spare switches, and on a phone it
             cost this row a button. -->
        <!-- The task board: on the phone this jumps to the board PAGE (owner,
             2026-08-29: "board单独作为一个独立的功能的页面"); on desktop it is
             the drawer's THIRD partition ("或者右侧边栏有这个任务侧边栏", same
             day) — exactly the files toggle's split. -->
        <CommandButton variant="icon" icon="layout" label={t('board')}
          expanded={mobile || compact ? undefined : termOpen && drawerView === 'board'}
          controls={mobile || compact || !drawerShown ? undefined : drawerId}
          onclick={() => {
            if (mobile || compact) { openBoardTab?.(selected); return; }
            if (termOpen && drawerView === 'board') { closeDrawer(); } else { drawerView = 'board'; openDrawer(); }
          }} />
        <!-- The drawer's second partition. On the phone (no drawer) the same
             button JUMPS to the Files tab — exactly what the terminal toggle
             does with the Terminal tab (owner, 2026-08-28: "手机上好像没有
             打开文件侧边栏的按钮"). -->
        <CommandButton variant="icon" icon="files" label={t('files')}
          expanded={mobile || compact ? undefined : termOpen && drawerView === 'files'}
          controls={mobile || compact || !drawerShown ? undefined : drawerId}
          onclick={() => {
            if (mobile || compact) { openFilesTab?.(selected, drawerFilesDir || projectPath); return; }
            if (termOpen && drawerView === 'files') { closeDrawer(); } else { drawerView = 'files'; openDrawer(); }
          }} />
        <!-- THE terminal affordance: a button, not a permanent pane. Adding an
             agent belongs to the roster row, and chat detail belongs to the
             title's menu (and Settings) — a header is not a place to keep
             spare switches. -->
        <CommandButton variant="icon" icon="terminal" label={t('hubTerminal')}
          expanded={mobile || compact ? undefined : termOpen && drawerView === 'term'}
          controls={mobile || compact || !drawerShown ? undefined : drawerId}
          onclick={() => termOpen && drawerView === 'term' && !compact ? closeDrawer() : (drawerView = 'term', openDrawer())} />
        {#if headerCopyFeedback && headerCopyAnchor}
          <div class="header-copy-feedback pop-layer" use:feedbackPosition={{ trigger: headerCopyAnchor, bounds: headerCopyAnchor.closest('main'), keepClear: headerCopyAnchor.closest('.chat-head') }}>
            <OperationFeedback value={headerCopyFeedback} ondismiss={headerCopyFeedback.kind === 'error' ? headerCopyLifetime.clear : undefined} />
          </div>
        {/if}
      </div>

      {#if actionReadError}
        <div class="config-error action-read-error" role="alert">
          <span>{actionReadError}</span>
          <CommandButton icon="refresh" label={t('refresh')} pending={actionRefreshing} onclick={() => refreshActionView()} />
        </div>
      {/if}
      {#snippet emptyFeed()}
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
                <!-- Configured TEAMS (board #74): one tap starts every member
                     with its role. -->
                {#each teams as tm (tm.name)}
                  <button class="start-row team" disabled={starting} onclick={() => startTeam(tm.name)}>
                    <span class="ava tava"><Icon name="collab" size={12} /></span>
                    <span class="sr-name">{tm.name}</span>
                    <span class="sr-backend">{teamMembers(tm).join(' · ')}</span>
                  </button>
                {/each}
              </div>
              <CommandButton icon="collab" label={t('hubStartTeam')} disabled={starting}
                onclick={() => openPicker('start')} />
            </div>
          {:else}
            <div class="empty">{managedAgents.length ? t('hubEmpty') : t('hubEmptyNoAgents')}</div>
          {/if}
      {/snippet}
      <Feed bind:this={feedView} {blocks} {agents} {managedNames} {selected} {visible} {compact}
        {roomReady} {justLoaded} {openedAt} {filterAgent} {loadingOlder} {histMore} {actMore}
        stepsRows={hubPrefs.stepsRows} {stateLabel} {emptyFeed} bind:following bind:newBelow
        onseen={markSeen} onolder={loadOlder} onpath={routePathRef}
        onclearfilter={() => { filterAgent = ''; }} onimage={(url) => { shotView = url; }}
        onboard={(id) => {
          if (mobile || compact) { openBoardTab?.(selected, id); return; }
          drawerIssueReq = { session: selected, id, n: (drawerIssueReq?.n ?? 0) + 1 };
          drawerView = 'board'; openDrawer();
        }}
        registerActions={registerFeedActions} />

      <Roster {selected} {compact} {managedAgents} {stopped} {selectedRow}
        {recipient} {filterAgent} {composerText} {managedNames} {busyNames} {interrupting}
        {unread} {acting} {tick} {roomReady} {justLoaded} {rosterBase}
        expanded={rosterExpanded} onexpand={toggleRoster}
        {stateLabel} {stateTone} onselect={setRecipient} oninterrupt={interrupt}
        onfilter={(name) => { closeCtx(); toggleFilter(name); }}
        onadd={() => openPicker('add')}
        oncontext={(at, name) => openCtx(at, name, agentItems(name))} />

      <Composer bind:this={composer} bind:composerText {selected} {compact} {recipient}
        {roomReady} allMenuOpen={!!ctxAt?.allSession && ctxAt.allSession === selected} onall={activateAll}
        {agents} {pending} {attaching} {failed} {sendable} {interruptible}
        onsend={send} onstage={stageFiles} onremove={removeAttachment}
        onmodels={modelsList} oninterrupt={interrupt} onpreview={(path) => { shotView = path; }}
        onfocus={() => { following = true; scrollFeed(true); setTimeout(() => scrollFeed(true), 300); }}
        onheightchange={() => { if (following) scrollFeed(true); }}
        registerBack={onGoBack ? backLayers.register : null} />
    </main>

    {#if drawerShown && !compact}
    <!-- ── Terminal drawer: where terminal things live. The .track is the
         grid item the reveal pins; the Drawer inside never changes size
         while the track moves (board #174). ── -->
    <div class="track drawer-track" id={drawerId} bind:this={drawerTrackEl}>
    <Drawer {compact} {visible} {fontSize} {selected} {projectPath} {termTarget} {termCommand}
      {drawerView} {drawerFilesReq} {drawerIssueReq} {drawerBoardNew}
      {agents} {panes} {managedAgents} {winsExpanded} {stateLabel} {stateTone}
      bind:drawerFilesDir onpick={pickWindow} onclose={closeDrawer}
      onexpand={() => (winsExpanded = !winsExpanded)}
      onterminal={() => { const m = /^(.+):(\d+)\.(\d+)$/.exec(termTarget); if (m) openTerminal(selected, termTarget, termCommand); }}
      onfiles={() => openFilesTab?.(selected, drawerFilesDir || projectPath)}
      onboard={() => openBoardTab?.(selected)}
      onnewissue={() => (drawerBoardNew = { n: (drawerBoardNew?.n ?? 0) + 1 })}
      onfilesback={(back) => { drawerFilesBack = back; }} />
    </div>
    {/if}
  </div>

  <!-- Confirm: stop one agent, close or delete the whole project. The dialog is
       the shared one (src/lib/ui/ConfirmDialog.svelte) — this page had the only
       good version of it, so it was lifted out rather than copied. -->
  <ConfirmDialog open={!!pendingAct} busy={acting} compact={compact}
    error={actionError} confirmIcon={pendingAct ? ACT_COPY[pendingAct.kind].icon : 'check'}
    title={pendingAct ? t(ACT_COPY[pendingAct.kind].title).replace('{name}', pendingAct.name) : ''}
    note={pendingAct ? t(ACT_COPY[pendingAct.kind].note) : ''}
    confirmLabel={pendingAct ? t(ACT_COPY[pendingAct.kind].go) : ''}
    onconfirm={runAction} oncancel={() => (pendingAct = null)} />

  <!-- One context menu for every subject above: right-click on the desktop, long
       press on a phone. -->
  <ContextMenu at={ctxAt} items={visibleCtxItems} who={ctxWho} oncancel={closeCtx} />

  <!-- Forgetting a PROJECT for good — only reachable from the recycle bin,
       the two-step rule: hide first, destroy there. -->
  <ConfirmDialog open={!!trashAsk} compact={compact} busy={purging} error={purgeError} confirmIcon="trash"
    title={trashAsk ? t('projectPurgeTitle').replace('{name}', trashAsk.project.name) : ''}
    note={t('projectPurgeNote')}
    confirmLabel={t('hubPurgeGo')}
    onconfirm={purgeProject} oncancel={() => (trashAsk = null)} />

  {#if pickerOpen}
    <!-- ── Start a team: several agents at once ── -->
    <div class="dlg-backdrop" onclick={() => pickerOpen = false} role="presentation"></div>
    <div class="dlg" class:sheet={compact}>
      <h2>{t('hubStartTeam')}</h2>
      {#if teams.length}
        <!-- Configured teams first (board #74): a tap starts the whole team
             with the brief below; the ad-hoc pick stays underneath. -->
        <div class="dlg-agents">
          {#each teams as tm (tm.name)}
            <button class="agent-pick team" disabled={starting} onclick={() => startTeam(tm.name, startBrief.trim())}>
              <span class="ava tava"><Icon name="collab" size={12} /></span>
              {tm.name} · {teamMembers(tm).join(', ')}
              <span class="ap-go">{t('hubStartTeamNamed')}</span>
            </button>
          {/each}
        </div>
      {/if}
      <div class="dlg-agents">
        {#each registry as r (r.name)}
          <button class="agent-pick" class:sel={startPick.includes(r.name)} aria-pressed={startPick.includes(r.name)}
            onclick={() => { startPick = startPick.includes(r.name) ? startPick.filter((n) => n !== r.name) : [...startPick, r.name]; }}>
            {#if backendIcon(r.backend)}<img class="ava" src={backendIcon(r.backend)} alt={r.backend} />{:else}<span class="ava" style:background={backendColor(r.backend)}>{r.name.slice(0, 1).toUpperCase()}</span>{/if}
            {r.name} · {r.backend}
            {#if startPick.includes(r.name)}<Icon name="check" size={13} />{/if}
          </button>
        {/each}
      </div>
      <input placeholder={t('hubBrief')} bind:value={startBrief} />
      <div class="dlg-actions">
        <CommandButton label={t('cancel')} onclick={() => pickerOpen = false} />
        <CommandButton variant="primary" icon="zap" label={t('hubStartGo').replace('{n}', String(startPick.length))}
          disabled={!startPick.length} pending={starting} onclick={() => addAgents(startPick, startBrief.trim())} />
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
  .header-copy-feedback { position: fixed; z-index: 8; }
  .action-read-error { display: flex; align-items: center; gap: var(--tool-gap); padding: calc(2 * var(--ui-gap)) calc(3 * var(--ui-gap)); }
  .action-read-error span { min-width: 0; overflow-wrap: anywhere; }
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
    /* --bubble-in moved to :root in app.css with the Board note's shared
       action atoms (board #46) — one definition, every wearer. */
    --bubble-out: color-mix(in srgb, var(--bg) 84%, var(--accent) 16%);
    --bubble-line: color-mix(in srgb, var(--border) 72%, var(--text3) 28%);
  
    /* Design tokens (--fs-*, --meta-ink, --t-*) come from :root in app.css —
       promoted app-wide 2026-08-18. Contract: tmm-cli.md "Design tokens". */
  }
  /* REVEAL, DO NOT RESIZE (board #174; the helpers and the reasoning are in
     reveal.ts). Each side track is its requested width times an OPEN FACTOR
     — a registered numeric custom property, so it interpolates — and the
     three tracks are always declared (a closed side is a 0 track). The
     factor is the only thing that ever transitions, and only under the
     `.moving` gate the helper raises for the duration of one move; the
     content inside a `.track` is pinned at its final width (inline, by
     pinTrack) and anchored to the edge that stays put, so the track merely
     uncovers it and an xterm inside is fitted once. What the track has not
     uncovered yet is clipped by the grid. Engines without @property
     interpolation fall back to the cut this always was. */
  @property --side-open { syntax: '<number>'; inherits: false; initial-value: 1; }
  @property --drawer-open { syntax: '<number>'; inherits: false; initial-value: 0; }
  .cols { flex: 1; display: grid; min-height: 0; --side-open: 1; --drawer-open: 0; }
  .hub-root:not(.compact) .cols { grid-template-columns: minmax(0, calc(var(--sidebar-w) * var(--side-open))) minmax(280px, 1fr) minmax(0, calc(var(--hub-drawer-w, 520px) * var(--drawer-open))); overflow: hidden; }
  .hub-root.compact .cols { grid-template-columns: minmax(0, 1fr); }
  /* :global — these classes are raised by reveal.ts, not by the markup. */
  .cols:global(.moving) { transition: --side-open var(--t-move) ease-out, --drawer-open var(--t-move) ease-out; }
  /* A track is a single-cell grid so its partition stretches to it at rest;
     pinned, it keeps the inline width and hugs the edge that does not move. */
  .track { display: grid; min-width: 0; min-height: 0; }
  .track > :global(*) { min-width: 0; } /* a partition follows its track at rest (yield, #158) instead of overflowing it */
  .track:global(.pin-start) { justify-self: start; }
  .track:global(.pin-end) { justify-self: end; }
  .hub-root.compact .track { display: contents; }
  /* Phone shape: tighter gutters, thumb-sized controls, no horizontal
     overflow. The page head wraps instead of pushing the chips off-screen. */
  /* ONE row, always: the app-wide phone rule lets a busy page-head wrap under
     the title (skill editor), but this header has few, icon-sized actions and
     wrapping put the Terminal toggle on a second line (owner, 2026-08-21:
     "打开terminal的按钮给换行到第二行了"). The title is the flexible child —
     .h1-text ellipsizes — and the buttons refuse to shrink. */
  .hub-root.compact .page-head { flex-wrap: nowrap; }
  .hub-root.compact .page-head h1 { font-size: var(--fs-title); }
  .hub-root.compact .h1-edit { font-size: var(--fs-title); }

  /* SideHandle widths are requested maxima: both side tracks yield when the
     container cannot fit them, without rewriting the saved preferences.
     Chat keeps its reading floor and takes any remaining space. */
  .hub-root.drawer-open .cols { --drawer-open: 1; }
  /* Collapsed = the track is 0 and the content is UNREACHABLE (not merely
     narrowed): hidden from sight, tab order and assistive tech at rest, and
     visible only while the move uncovers or withdraws it. */
  .hub-root.side-collapsed .cols { --side-open: 0; }
  .hub-root.side-collapsed .cols:not(:global(.moving)) > .track.side { visibility: hidden; }
  /* The NAME displays WHOLE, with priority over the PATH (owner, 2026-08-30:
     "名字还是优先要显示全的，尽量不要省略") — but NEVER over the buttons
     (owner, 2026-09-02, board #72: "按钮应该优先出现，避免项目名把按钮挤不见").
     `flex: none` made the group refuse to shrink outright, and in this nowrap
     header a long name on a phone overflowed the row and pushed the icon
     buttons off-screen. So the group CAN shrink (min-width: 0 lets the
     .h1-text ellipsis engage), but at flex-shrink 1 against the path's 1000:
     negative space is distributed by shrink × basis, so the path absorbs
     effectively all of it and the name only starts to ellipsize once the
     path is gone. The buttons are `flex: none` and always keep their box. */
  .title-group { display: flex; align-items: center; gap: 1px; flex: 0 1 auto; min-width: 0; max-width: 60%; }
  /* The title yields width before any native command target does. */
  h1 { display: flex; align-items: center; min-width: 0; }
  .h1-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .h1-edit {
    font-family: var(--font-mono);
    font-size: var(--fs-title); font-weight: 600; color: var(--text);
    min-width: 0; flex: 0 1 auto; width: 22ch; max-width: 100%;
    background: var(--bg2); border: 1px solid var(--accent-line); border-radius: var(--ui-radius-control);
    padding: 2px 6px; box-sizing: border-box;
  }
  .h1-edit:focus { outline: none; }
  .mid { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  /* Fits → shows whole; doesn't fit → pans (wheelX action), NEVER an ellipsis.
     `flex: 0 1 auto` + min-width lets the header's buttons take their space
     first while the path yields, and the hidden scrollbar keeps the header one
     quiet line. */
  .path {
    font-family: var(--font-mono); font-size: var(--fs-sub); color: var(--text3);
    white-space: nowrap; overflow-x: auto; overflow-y: hidden;
    /* THE dynamic region: shrinks below its content first (min-width: 0 +
       scroll) while the buttons stay whole; the shrink weight of 1000 against
       the title group's 1 is what makes it yield FIRST and the name only
       after it (see .title-group); the spacer still owns the leftover,
       keeping the toggles right-aligned. */
    min-width: 0; flex: 0 1000 auto;
    scrollbar-width: none; -webkit-overflow-scrolling: touch;
    user-select: text; -webkit-user-select: text; cursor: text;
  }
  .path::-webkit-scrollbar { display: none; }
  .tava { display: inline-grid; place-items: center; background: var(--accent-bg); color: var(--accent); }
  .ap-go { margin-left: auto; font-size: var(--fs-micro); color: var(--accent); }



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
  .sr-name { font-family: var(--font-mono); font-weight: 600; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sr-backend { font-family: var(--font-mono); font-size: var(--fs-sub); color: var(--text3); margin-left: auto; }
  /* The Manager atom (board #7) — same declaration as AgentsPage's, pinned. */
  .m-badge { flex: none; display: inline-flex; align-items: center; justify-content: center; width: 15px; height: 15px; border: 1px solid var(--accent); border-radius: 4px; color: var(--accent); font-size: var(--fs-micro); font-weight: 700; line-height: 1; }

  .dlg-backdrop { position: fixed; inset: 0; z-index: 30; background: rgba(0,0,0,0.45); animation: fade-in var(--t-move) ease-out; }
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
    /* Sheets rise with a scrim (design-language §1; motion.md): the resting
       transform is none, so the intro can own it. Exit is a cut. */
    animation: sheet-up var(--t-move) ease-out;
  }
  @media (prefers-reduced-motion: reduce) { .dlg-backdrop, .dlg.sheet { animation: none; } .cols:global(.moving) { transition: none; } }
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
