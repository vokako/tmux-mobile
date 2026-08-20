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
  import SideHandle from '../ui/SideHandle.svelte';
  import ChatImage from './ChatImage.svelte';
  import Icon from '../ui/Icon.svelte';
  import { tick as settled } from 'svelte';
  import { t, i18n } from '../core/i18n.svelte.ts';
  import {
    projectList, projectUp, projectDown, projectDelete, projectCreate, projectRename, listSessionsWithPanes,
    hubPost, hubCommand, modelsList, hubLog, hubMsgArchive, hubMsgRestore, hubMsgPurge, hubArchive, hubRooms, hubAgents, hubSpawn, hubAgentStop, hubAgentRestart, hubActivity, hubAgentRemove, hubAgentInterrupt, registryList,
    addTeamMessageListener, removeTeamMessageListener,
  } from '../core/ws.ts';
  import { sortRows } from '../projects/projects.ts';
  import { markLeadingMention, stateDotColor, mergeMessages, backendColor, feedBlocks, pickLead, addressed, fmtElapsed, unreadSenders, splitImages, stoppedAgents, toolColor, STEPS_ROWS, pickAnchor, toolEventParts, elideMiddle, slashCommand, commandPalette, ctxColor, statusNote, noteStateColor, sameDay, readlineEdit } from './hub.ts';
  import { anchorOf, menuPlacement, viewBox } from '../ui/placement.ts';
  import ContextMenu from '../ui/ContextMenu.svelte';
  import { longpress } from '../ui/longpress.ts';
  import { hubPrefs } from './hub-prefs.svelte.ts';
  import { renderMarkdown } from '../core/markdown.ts';
  import CreateProjectDialog from '../projects/CreateProjectDialog.svelte';
  import ConfirmDialog from '../ui/ConfirmDialog.svelte';

  let { visible = false, fontSize = 14, mobile = false, openTerminal = () => {} } = $props();

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
  let panes = $state([]);           // all tmux panes
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
  let feedEl = $state(null);
  let composerEl = $state(null);
  let toChipW = $state(0);        // measured recipient-chip width → first-line indent

  // Terminal drawer (closed by default — the whole point).
  let termOpen = $state(false);
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

  async function reload() {
    try {
      // The sidebar is ordered by CONVERSATION, so the list needs one more fact:
      // when each room last had a message. One grouped query server-side.
      const [{ projects }, sp, talk] = await Promise.all([
        projectList(),
        listSessionsWithPanes(),
        hubRooms().then((r) => r.rooms ?? {}).catch(() => ({})),
      ]);
      rows = sortRows(projects, talk);
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

  async function selectProject(session) {
    // An unsent line belongs to the conversation it was written for. Park it on
    // the project we are leaving and pick up whatever was waiting in the new one
    // — carrying the text across would put it in front of the wrong agents.
    if (selected) hubPrefs.setDraft(selected, composerText);
    selected = session;
    composerText = hubPrefs.draft(session);
    hubPrefs.setProject(session);
    feed = [];
    activity = [];
    lastActivityTs = 0;
    lastTs = 0;
    agents = [];
    recipient = '';
    recipientOpen = false;
    menuFor = '';
    msgOpen = '';
    rawOpen = '';
    termOpen = false;
    archiveOpen = false;
    archived = [];
    await Promise.all([loadFeed(), loadAgents(), loadActivity(), loadArchive()]);
  }

  async function loadFeed() {
    if (!selected) return;
    try {
      const { messages } = await hubLog(selected, lastTs, 200);
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
    try {
      const { events } = await hubActivity(selected, lastActivityTs);
      if (events?.length) {
        activity = [...activity, ...events].slice(-300);
        lastActivityTs = Math.max(lastActivityTs, ...events.map((e) => e.ts));
        // Telemetry rows are not "news": they extend the tail, so follow if we
        // were following, but they must not raise the new-messages dot.
        if (following) scrollFeed();
      }
    } catch { /* hub not available */ }
  }

  async function loadAgents() {
    if (!selected) return;
    try {
      agents = (await hubAgents(selected)).agents ?? [];
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

  function onFeedScroll() {
    following = atBottom();
    const top = feedEl?.scrollTop ?? 0;
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
  }

  /** One bubble, one continuous motion: select it while it is naturally inside
   * the viewport, then let CSS sticky catch that SAME element as it leaves in
   * the current scroll direction. In an empty stretch of a long reply, retain
   * it; never swap to another invisible message at an arbitrary midpoint. */
  /** The height a held ask should not exceed — a fifth of the conversation, the
   * owner's number. It is an ESTIMATE that feeds the line budget below, never a
   * cap on the box: the bubble is never clipped, the text is folded to fit. Taken
   * from the FEED rather than `20vh` because a head, a roster and a composer sit
   * around it. */
  let heldMax = $state(0);
  let heldLine = $state(20);        // measured line box of a bubble, px
  function measureHeld() {
    if (!feedEl) return;
    heldMax = Math.max(96, Math.round(feedEl.clientHeight * 0.2));
    const bubble = feedEl.querySelector('.bubble');
    const lh = bubble ? parseFloat(getComputedStyle(bubble).lineHeight) : NaN;
    if (Number.isFinite(lh) && lh > 6) heldLine = lh;
  }
  $effect(() => {
    void blocks; void visible;
    measureHeld();
    const onResize = () => { naturalH.clear(); measureHeld(); };
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });
  /** How many whole lines fit in that ceiling, minus the bubble's own padding
   * and the meta trailer's line. Never below three: head + marker + tail is the
   * floor at which an elision still says anything. */
  const heldLines = $derived(Math.max(3, Math.floor((heldMax - 26) / heldLine)));
  /** The body a held bubble shows. Identity when it already fits, so the common
   * case re-renders nothing. */
  const heldBody = (text) => elideMiddle(text, heldLines);
  /** The message the reader unfolded by hand. One at a time, and it resets when
   * the anchor moves on — an unfolded ask is a moment of attention, not a
   * setting. */
  let heldExpanded = $state('');
  $effect(() => { if (askKey !== heldExpanded) heldExpanded = ''; });
  /** Natural heights, by message key. A folded bubble is SHORTER than the
   * message it stands for, so the boundary test has to keep using the height it
   * had unfolded — otherwise folding shrinks the box, that unholds it, the text
   * comes back, and it holds again: the same blink, sourced from the text
   * instead of from a clip. The cache refreshes on every tick a bubble is NOT
   * folded, which includes the tick before it first folds. */
  const naturalH = new Map();

  function syncAsk(direction = askDir, reset = false) {
    if (!feedEl) { askKey = ''; askEdge = ''; askHeld = false; return; }
    // Chromium's offsetTop for a sticky element is its HELD position. Read that
    // and the old anchor appears naturally visible, so the next anchor is never
    // selected. Neutralize the one current sticky element for this synchronous
    // layout read; the inline override is removed before the browser can paint.
    const stickies = [...feedEl.querySelectorAll('.ask-top, .ask-bottom')];
    for (const el of stickies) el.style.position = 'static';
    const items = [...feedEl.querySelectorAll('[data-ask]')].map((el) => {
      const key = el.dataset.ask ?? '';
      const height = el.offsetHeight;
      if (el.dataset.folded === '1') {
        // Folded: this box is smaller than the message. Answer with the height
        // it has when unfolded (see naturalH) so the decision that folded it and
        // the decision that keeps it folded use the same number.
        return { key, top: el.offsetTop, height: naturalH.get(key) ?? height };
      }
      naturalH.set(key, height);
      return { key, top: el.offsetTop, height };
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
    if (!raw || !selected) return;
    // A SLASH COMMAND goes to the agent's CLI, not to its model, so it is typed
    // verbatim — no `[tmm chat …] human:` stamp, no @address, nothing the TUI
    // would read as prose. It needs a target: an explicit `@name`, else the
    // composer's recipient. With neither (a room note) there is nobody to run
    // it, so it stays an ordinary message rather than vanishing.
    const cmd = slashCommand(raw);
    const cmdTarget = cmd && (cmd.to || (recipient === ALL_TARGET ? 'all' : recipient));
    if (cmd && cmdTarget) {
      composerText = '';
      following = true;
      scrollFeed(true);
      try {
        await hubCommand(selected, cmdTarget, cmd.command);
        await loadFeed();
        scrollFeed(true);
      } catch (e) { console.warn('hub command failed', e); composerText = raw; }
      return;
    }
    // The recipient makes "talk to THIS agent" the default rather than a
    // gesture: addressed() prefixes @name unless the user @-addressed someone
    // by hand, and an empty recipient posts to the room.
    const text = addressed(raw, recipient);
    composerText = '';
    following = true;
    scrollFeed(true);
    try {
      await hubPost(selected, text);
      await loadFeed();
      scrollFeed(true);
    } catch (e) { console.warn('hub post failed', e); }
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
  const SEND_ZONE = 38; // button 30px + gaps, measured from the textarea's right edge
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
      el.style.paddingRight = '40px';
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
    // does — so keep the tail parked while it grows.
    if (following) scrollFeed(true);
  }
  $effect(() => {
    void composerText;   // includes the reset to '' after sending
    void toChipW;        // the indent changes wrapping, so height must re-measure
    void composerIsCmd;  // the mono flip changes metrics, so height re-measures too
    growComposer();
  });

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
  let cmdModels = $state([]);
  let paletteIdx = $state(0);
  let paletteOff = $state(false);      // Escape closes it until the text changes
  const palette = $derived(paletteOff ? null : commandPalette(composerText, cmdModels));
  // The open menu's agent, as its status line reads right now.
  const vitalsFor = $derived(vitalsLine(managedAgents.find((a) => a.name === menuFor)?.vitals));
  $effect(() => { void composerText; paletteOff = false; });
  // The draft survives a reload because a half-written message is work. Written
  // on every keystroke: one small JSON string, and the alternative (a debounce)
  // loses the last few characters exactly when the tab goes away.
  $effect(() => { hubPrefs.setDraft(selected, composerText); });
  $effect(() => { void palette; paletteIdx = 0; });
  // The model list is only needed once a command wants it, and the server caches
  // it for ten minutes — so this asks at most once per Hub visit.
  $effect(() => {
    if (!palette || cmdModels.length) return;
    modelsList('kiro').then((r) => { cmdModels = r.models ?? []; }).catch(() => {});
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
  // ── Deleting a message is two steps. Archive HIDES it (the message stays in
  // the room's store, so a restore costs nothing); deleting it in the archive is
  // what forgets it (owner, 2026-08-19). The archive is a VIEW of the same feed
  // area, not a page: it is a short list you visit to change your mind.
  let archiveOpen = $state(false);
  let archived = $state([]);
  let purgeAsk = $state(null);      // the message awaiting "yes, forget it"

  async function loadArchive() {
    if (!selected) return;
    try {
      const r = await hubArchive(selected);
      archived = r.messages ?? [];
    } catch (e) { console.warn('hub archive failed', e); }
  }

  /** Hide a message from the conversation. Reversible, so no confirmation: the
   * whole point of the first step is that it is the safe one. */
  async function archiveMsg(id) {
    if (!selected || !id) return;
    msgOpen = '';
    try {
      await hubMsgArchive(selected, [id]);
      feed = feed.filter((m) => m.id !== id);
      await loadArchive();
    } catch (e) { console.warn('archive failed', e); }
  }

  async function restoreMsg(id) {
    try {
      await hubMsgRestore(selected, [id]);
      archived = archived.filter((m) => m.id !== id);
      await loadFeed();
    } catch (e) { console.warn('restore failed', e); }
  }

  /** The irreversible half. Confirmed, and the copy says what is lost and what
   * survives — the app's rule for every destructive verb. */
  async function purgeMsg(id) {
    purgeAsk = null;
    try {
      await hubMsgPurge(selected, [id]);
      archived = archived.filter((m) => m.id !== id);
    } catch (e) { console.warn('purge failed', e); }
  }

  /** The card's share of the reading: what makes THIS agent different from its
   * neighbours. The branch and cwd are project-wide, so they stay in the tooltip
   * and the menu — repeating them on every card would be chrome, not data. */
  function cardVitals(v) {
    if (!v) return '';
    return [v.model, v.effort].filter(Boolean).join(' · ');
  }

  /** Choosing a recipient is also choosing this project's lead: it is the same
   * decision ("who am I working with here"), so it persists. */
  function setRecipient(name) {
    recipient = name;
    recipientOpen = false;
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
  }

  /** The one close path, so every trigger keeps the reader's place. */
  function closeDrawer() {
    withReadingAnchor(() => { termOpen = false; });
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
  let pendingAct = $state(null);   // { kind: keyof ACT_COPY, name }
  let acting = $state(false);
  const askAction = (kind, name) => { pendingAct = { kind, name }; };

  async function runAction() {
    if (!pendingAct || acting) return;
    const { kind, name } = pendingAct;
    acting = true;
    try {
      if (kind === 'down' || kind === 'delete') {
        const row = rows.find((r) => r.project.session === selected);
        if (row) await (kind === 'delete' ? projectDelete(row.project.id) : projectDown(row.project.id));
        if (kind === 'delete') {
          // The project is gone: land on whatever is left rather than an
          // empty conversation pointing at nothing.
          selected = '';
        }
      } else if (kind === 'remove') {
        await hubAgentRemove(selected, name);
      } else {
        await hubAgentStop(selected, name);
      }
      await Promise.all([reload(), loadAgents(), loadFeed()]);
    } catch (e) {
      console.warn(kind === 'down' ? 'close project failed' : 'stop failed', e);
    } finally {
      acting = false;
      pendingAct = null;
    }
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

  // Esc closes the drawer (capture so it wins over xterm).
  $effect(() => {
    if (!termOpen) return;
    const onKey = (e) => { if (e.key === 'Escape') { closeDrawer(); e.stopPropagation(); } };
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
  /** A pointer event as a plain client point. */
  const pointOf = (e) => ({ x: e.clientX ?? 0, y: e.clientY ?? 0 });

  /** An agent's verbs — the same ones its dot menu carries. */
  function agentItems(name) {
    if (stopped.includes(name)) {
      return [
        { label: t('hubStartAgain'), icon: 'refresh', onselect: () => startAgent(name) },
        { label: t('hubRemove'), icon: 'trash', danger: true, onselect: () => askAction('remove', name) },
      ];
    }
    const a = managedAgents.find((x) => x.name === name);
    return [
      { label: t('hubWatch'), icon: 'terminal', onselect: () => { if (a) openDrawer(a); } },
      { label: t('hubInterrupt'), icon: 'x', onselect: () => interrupt(name) },
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
    return [
      row?.live
        ? { label: t('projectDown'), icon: 'stop', danger: true, onselect: () => askAction('down', name) }
        : { label: t('projectUp'), icon: 'zap', onselect: () => { selectProject(session); setTimeout(bringUp, 0); } },
      { label: t('projectRename'), icon: 'edit',
        onselect: () => { selectProject(session); setTimeout(startRename, 0); } },
      { label: t('projectDelete'), icon: 'trash', danger: true,
        onselect: () => askAction('delete', name) },
    ];
  }

  /** A message's verbs: the same three its own overlay offers. */
  function msgItems(m) {
    return [
      { label: t('hubCopy'), icon: 'copy', onselect: () => copyMsg(m.body) },
      { label: t('hubRaw'), icon: 'command', onselect: () => { rawOpen = rawOpen === m.id ? '' : m.id; } },
      { label: t('hubArchive'), icon: 'trash', onselect: () => archiveMsg(m.id) },
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
    const onDown = (e) => { if (!e.target?.closest?.('.a-menu, .a-more')) close(); };
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

  // `windowOf` is what lets a reply close the lane it belongs to, so two agents
  // working at once keep ONE growing group each instead of interleaving.
  const blocks = $derived(
    feedBlocks(feed, activity, hubPrefs.feedLevel, (from) => agents.find((a) => a.name === from)?.window),
  );
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
      setTimeout(() => { if (copied === body) copied = ''; }, 1500);
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
    {#if !compact || sideOpen}
    {#if compact}
      <div class="side-scrim" onclick={() => sideOpen = false} role="presentation"></div>
    {/if}
    <aside class="sidebar" class:sheet={compact}>
      {#if !compact}<SideHandle />{/if}
      <div class="side-scroll">
        <div class="side-h">{t('hubProjects')}</div>
        {#each rows as row (row.project.id)}
          <!-- Right-click (desktop) and long press (phone) open the project's own
               verbs where the pointer is. The row's normal job — open this
               conversation — is unchanged. -->
          <button class="side-row" class:open={row.project.session === selected}
            onclick={() => { selectProject(row.project.session); sideOpen = false; }}
            oncontextmenu={(e) => { e.preventDefault(); openCtx(pointOf(e), row.project.name, projectItems(row)); }}
            use:longpress={{ onlongpress: (pt) => openCtx(pt, row.project.name, projectItems(row)) }}>
            <span class="dot" class:off={!row.live}></span>
            <span class="p-name">{row.project.name}</span>
          </button>
        {/each}
        <button class="side-row add" onclick={() => { createOpen = true; sideOpen = false; }}>
          <Icon name="plus" size={13} />{t('projectNew')}
        </button>
      </div>
    </aside>
    {/if}

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
            <span class="h1-text">{selectedRow?.project.name ?? ''}</span>
            {#if selected}
              <button class="h1-pen" title={t('projectRenameHint')}
                aria-label={t('projectRename')} onclick={startRename}>
                <Icon name="edit" size={13} />
              </button>
            {/if}
          </h1>
        {/if}
        <!-- The FULL path, not a middle-elided stub: it renders whole when it
             fits, and when it doesn't the box scrolls (wheel included) instead
             of ellipsizing — "不要直接用省略号" (owner, 2026-08-20). Desktop
             only, same as before. -->
        {#if !compact}<span class="path" use:wheelX title={selectedRow?.project.path ?? ''}>{selectedRow?.project.path ?? ''}</span>{/if}
        <span class="spacer"></span>
        {#if selected}
          <!-- Delete is offered whether or not the session is live: it is the
               "this project should stop existing" verb, so it does the closing
               itself. Confirmed like every consequential action. -->
          <button class="chip-btn danger" title={t('projectDeleteHint')}
            onclick={() => askAction('delete', selectedRow?.project.name ?? '')}>
            <Icon name="trash" size={13} />{#if !compact}{t('projectDelete')}{/if}
          </button>
        {/if}
        {#if selected && !liveSelected}
          <button class="chip-btn" onclick={bringUp}>{t('projectOpen')}</button>
        {:else if selected}
          <!-- Closing is the counterpart of Open, in the same slot: it kills
               the tmux session and keeps the project, so the header shows
               exactly one of the two depending on what is true now. -->
          <button class="chip-btn danger" title={t('projectDownHint')}
            onclick={() => askAction('down', selectedRow?.project.name ?? '')}>
            {t('projectDown')}
          </button>
        {/if}
        <!-- THE terminal affordance: a button, not a permanent pane. Adding an
             agent belongs to the roster row, and chat detail belongs to
             Settings — a header is not a place to keep spare switches. -->
        <button class="chip-btn term-toggle" class:on={termOpen} title={t('hubTerminal')} onclick={() => termOpen && !compact ? closeDrawer() : openDrawer()}>
          <Icon name="terminal" size={14} />{#if !compact}<span>{t('hubTerminal')}</span>{/if}
        </button>
      </div>

      {#if managedAgents.length || stopped.length}
        <!-- The roster. Tapping an agent makes it the recipient (and this
             project's lead) — the phone gets chips, the desktop gets cards. -->
        <div class="cards" class:chips={compact} bind:this={cardsEl}>
          {#each managedAgents as a (a.window)}
            <!-- A div, not a button: the dot menu inside contains real buttons,
                 and a button inside a button is invalid HTML the browser
                 silently reshuffles. -->
            <div class="acard" class:sel={recipient === a.name} role="button" tabindex="0"
              title={[`${a.name} · ${stateLabel(a.state)}`, a.detail, vitalsLine(a.vitals)].filter(Boolean).join(' · ')}
              onclick={() => setRecipient(a.name)}
              oncontextmenu={(e) => { e.preventDefault(); openCtx(pointOf(e), a.name, agentItems(a.name)); }}
              use:longpress={{ onlongpress: (pt) => openCtx(pt, a.name, agentItems(a.name)) }}
              onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setRecipient(a.name); } }}>
              <div class="ac-top">
                <span class="ava" style:background={backendColor(a.agent)}>{a.name.slice(0, 1).toUpperCase()}</span>
                <span class="a-name">{a.name}</span>
                <span class="st" class:live={a.state === 'running'} style:background={stateDotColor(a.state)}></span>
                {#if a.since}<span class="s-age">{fmtElapsed(a.since, tick)}</span>{/if}
                {#if unread.has(a.name)}<span class="unread" title={t('hubUnread')}></span>{/if}
                <!-- Destructive and secondary actions stay behind a dot menu: a
                     roster is for seeing who is here, not a row of hazards. -->
                <span class="a-more" role="button" tabindex="-1" title={t('hubMore')}
                  onclick={(e) => { e.stopPropagation(); toggleAgentMenu(a.name, e.currentTarget); }}
                  onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); toggleAgentMenu(a.name, e.currentTarget); } }}>
                  <Icon name="dots" size={13} />
                </span>
              </div>
              <!-- What the agent's own status line says, kept ON the card rather
                   than behind the menu ("这个直接常驻显示吧 可以字号小一点"). Model
                   and effort only: the branch is the same for every agent in a
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
                 holds real buttons. Clicking the card starts it; removing it
                 lives in the menu, because a stopped agent you are done with
                 has to be ejectable — the slot is what keeps `up` recreating
                 it (owner, 2026-08-19). -->
            <div class="acard off" class:busy={acting} role="button" tabindex="0" title={t('hubStartAgain')}
              oncontextmenu={(e) => { e.preventDefault(); openCtx(pointOf(e), name, agentItems(name)); }}
              use:longpress={{ onlongpress: (pt) => openCtx(pt, name, agentItems(name)) }}
              onclick={() => !acting && startAgent(name)}
              onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); if (!acting) startAgent(name); } }}>
              <div class="ac-top">
                <span class="ava dim">{name.slice(0, 1).toUpperCase()}</span>
                <span class="a-name">{name}</span>
                <span class="s-age">{t('hubStopped')}</span>
                <Icon name="refresh" size={11} />
                <span class="a-more" role="button" tabindex="-1" title={t('hubMore')}
                  onclick={(e) => { e.stopPropagation(); toggleAgentMenu(name, e.currentTarget); }}
                  onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); toggleAgentMenu(name, e.currentTarget); } }}>
                  <Icon name="dots" size={13} />
                </span>
              </div>
            </div>
          {/each}
          <!-- Ad hoc: add an agent to a conversation already in progress. -->
          {#if liveSelected}
            <button class="acard add" onclick={() => openPicker('add')} title={t('hubSpawn')}>
              <Icon name="plus" size={14} /><span>{t('hubSpawn')}</span>
            </button>
          {/if}
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
          {#if vitalsFor}<div class="am-vitals">{vitalsFor}</div>{/if}
          {#if stopped.includes(menuFor)}
            <button role="menuitem" disabled={acting} onclick={() => { const n = menuFor; menuFor = ''; startAgent(n); }}>
              <Icon name="refresh" size={12} />{t('hubStartAgain')}
            </button>
          {:else}
            <button role="menuitem" onclick={() => { const a = managedAgents.find((x) => x.name === menuFor); menuFor = ''; if (a) openDrawer(a); }}>
              <Icon name="terminal" size={12} />{t('hubWatch')}
            </button>
            <button role="menuitem" title={t('hubInterruptHint')} onclick={() => { const n = menuFor; menuFor = ''; interrupt(n); }}>
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
      {#if archived.length}
        <!-- The way in and out of the archive. It only exists when something is
             in there: an empty archive is not a place to visit. -->
        <button class="arch-bar" class:on={archiveOpen} onclick={() => { archiveOpen = !archiveOpen; if (archiveOpen) loadArchive(); }}>
          <Icon name={archiveOpen ? 'chevron-down' : 'trash'} size={11} />
          {archiveOpen ? t('hubArchiveClose') : t('hubArchiveOpen').replace('{n}', String(archived.length))}
        </button>
      {/if}
      {#if archiveOpen}
        <div class="arch subtle-scroll">
          {#each archived as a (a.id)}
            <div class="arow">
              <div class="ar-head">
                <span class="ar-who">{a.from}</span>
                <span class="ar-ts">{fmtTime(a.ts)}</span>
              </div>
              <div class="ar-body">{a.body}</div>
              <div class="ar-acts">
                <button onclick={() => restoreMsg(a.id)}>
                  <Icon name="refresh" size={11} />{t('hubRestore')}
                </button>
                <!-- The only irreversible step in the whole flow, so it is the
                     only one that asks. -->
                <button class="danger" onclick={() => (purgeAsk = a)}>
                  <Icon name="trash" size={11} />{t('hubPurge')}
                </button>
              </div>
            </div>
          {/each}
        </div>
      {/if}
      <div class="feed subtle-scroll" class:hidden={archiveOpen} bind:this={feedEl} onscroll={onFeedScroll}>
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
                 a pane), folded: consecutive lifecycle lines are one fact each,
                 not one row each. Hidden entirely at the chat-only level
                 (feedBlocks drops them). Readable ink, not fine print — and a
                 /command keeps the composer's monospace so it reads as the thing
                 that was typed ("不要只用灰色小字", owner 2026-08-20). -->
            <div class="sysline">
              {#each b.items as item, j (`${j}-${item}`)}
                {#if j > 0}<span class="sys-sep" aria-hidden="true">·</span>{/if}
                <span class="sys-item" class:cmd={item.startsWith('/')}>{item}</span>
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
              <!-- Folding is a property of the TEXT: while this ask is the held
                   anchor and the reader has not unfolded it, the bubble renders a
                   middle-elided body. `data-folded` tells syncAsk that this box
                   is smaller than its message (see naturalH). -->
              {@const folded = isAsk && askKey === key && askHeld && heldExpanded !== key
                && heldBody(parts.text) !== parts.text}
              <!-- Unfolded while STILL HELD: the whole message is now inside a
                   bubble that is pinned to an edge, so a message taller than the
                   screen would have an unreachable bottom half — sticky ignores
                   the feed's scrolling ("我展开看的时候，应该可以上下滑动，而不是死
                   死钉住", owner 2026-08-20). The BODY becomes its own scroller
                   for exactly this state; back in the normal flow the cap is off
                   and the message reads full-length again. -->
              {@const heldScroll = isAsk && askKey === key && askHeld && heldExpanded === key}
              <div class="msg" class:me={m.from === 'human'}
                class:ask-top={isAsk && askKey === key && askEdge === 'top'}
                class:ask-bottom={isAsk && askKey === key && askEdge === 'bottom'}
                class:held={isAsk && askKey === key && askHeld}
                data-ask={isAsk ? key : undefined}
                data-folded={folded ? '1' : undefined}>
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
                  <div class="m-body" class:held-scroll={heldScroll}>
                    {#if parts.text}
                      {#if rawOpen === key}
                        <pre class="raw">{m.body}</pre>
                      {:else}
                        <!-- Folded: both ends of the ask, with the middle
                             replaced by ……. Raw view and every other message
                             render in full. -->
                        {@html markLeadingMention(renderMarkdown(folded ? heldBody(parts.text) : parts.text))}
                      {/if}
                      {#if folded}
                        <!-- The way back to the whole message. A button, because
                             this is the one thing you might want from a folded
                             ask; the bubble's own click still opens copy/raw. -->
                        <button class="m-unfold" onclick={(e) => { e.stopPropagation(); heldExpanded = key; }}>
                          <Icon name="chevron-down" size={11} />{t('hubUnfold')}
                        </button>
                      {:else if heldScroll}
                        <button class="m-unfold" onclick={(e) => { e.stopPropagation(); heldExpanded = ''; }}>
                          <Icon name="chevron-up" size={11} />{t('hubRefold')}
                        </button>
                      {/if}
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
                    <!-- One click hides it; forgetting it takes a second decision
                         in the archive. So this is not a `danger` action — it is
                         the reversible one. -->
                    <button onclick={() => archiveMsg(m.id)} title={t('hubArchiveHint')}>
                      <Icon name="trash" size={11} />{t('hubArchive')}
                    </button>
                  </div>
                {/if}
                {#if parts.images.length}
                  <div class="shots">
                    {#each parts.images as src, k (`${k}-${src}`)}
                      <ChatImage {src} alt={m.from} />
                    {/each}
                  </div>
                {/if}
              </div>
          {:else if b.type === 'prompt'}
            <!-- The input half: what this agent was asked, which only the
                 userPromptSubmit hook can tell us. -->
            <div class="prompt">
              <div class="p-head"><span class="p-who">{windowName(b.window)}</span><span class="p-tag">{t('hubPromptIn')}</span><span>{fmtTime(b.ts)}</span></div>
              <div class="p-body">{b.text}</div>
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
                  <span class="s-live" aria-hidden="true"></span>
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
                {@const capped = !stepsAll[b.key] && b.events.length > STEPS_ROWS}
                <!-- Every call is in the DOM; the CAP is a viewport on it, so a
                     live run stops growing the conversation after ten rows and
                     the tail stays where the eye already is. -->
                <div class="s-body" class:capped style:--steps-rows={STEPS_ROWS}
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
                {#if b.events.length > STEPS_ROWS}
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
        {#if !blocks.length}
          {#if liveSelected && !managedAgents.length && registry.length}
            <!-- Nothing to talk to yet: start from a preset. One tap = that
                 agent becomes the lead; "several" starts a team in one go. -->
            <div class="start">
              <div class="start-h">{t('hubStartTitle')}</div>
              <div class="start-list">
                {#each registry as r (r.name)}
                  <button class="start-row" disabled={starting} onclick={() => addAgents([r.name], '', 'start')}>
                    <span class="ava" style:background={backendColor(r.backend)}>{r.name.slice(0, 1).toUpperCase()}</span>
                    <span class="sr-name">{r.name}</span>
                    <span class="sr-backend">{r.backend}</span>
                    {#if r.can_hire}<span class="sr-cap">{t('agentsCanHire')}</span>{/if}
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
                    <span class="st" style:background={stateDotColor(a.state)}></span>{a.name}
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
          onfocus={() => { following = true; scrollFeed(true); setTimeout(() => scrollFeed(true), 300); }}
        ></textarea>
        <!-- Send lives INSIDE the capsule, bottom-right, out of the flow: it
             stopped costing the composer a whole column. -->
        <button class="send-btn" onclick={send} title={t('hubSend')} aria-label={t('hubSend')}
          disabled={!composerText.trim() || !selected}>
          <Icon name="send-up" size={15} />
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
        <button class="icon-btn" title="Esc" onclick={closeDrawer}>
          <Icon name="x" size={14} />
        </button>
      </div>
      <div class="term-body">
        {#if termTarget}
          {#key termTarget}
            <Terminal target={termTarget} session={selected} command={termCommand} {fontSize} embedded chromeless active={visible} />
          {/key}
        {:else}
          <div class="empty">{t('hubNoPane')}</div>
        {/if}
      </div>
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

  <!-- Forgetting a message for good. The copy names what is lost AND what
       survives, which is the rule for every destructive verb in this app. -->
  <ConfirmDialog open={!!purgeAsk} compact={compact}
    title={t('hubPurgeTitle')}
    note={purgeAsk ? t('hubPurgeNote').replace('{who}', purgeAsk.from) : ''}
    confirmLabel={t('hubPurgeGo')}
    onconfirm={() => purgeMsg(purgeAsk?.id)} oncancel={() => (purgeAsk = null)} />

  {#if pickerOpen}
    <!-- ── Start a team: several agents at once ── -->
    <div class="dlg-backdrop" onclick={() => pickerOpen = false} role="presentation"></div>
    <div class="dlg" class:sheet={compact}>
      <h2>{t('hubStartTeam')}</h2>
      <div class="dlg-agents">
        {#each registry as r (r.name)}
          <button class="agent-pick" class:sel={startPick.includes(r.name)}
            onclick={() => { startPick = startPick.includes(r.name) ? startPick.filter((n) => n !== r.name) : [...startPick, r.name]; }}>
            <span class="ava" style:background={backendColor(r.backend)}>{r.name.slice(0, 1).toUpperCase()}</span>
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
</div>

<style>
  .hub-root {
    height: 100%; display: flex; flex-direction: column; min-height: 0;
    background: var(--bg); position: relative;
    --chat-canvas: color-mix(in srgb, var(--bg) 62%, var(--bg2));
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
  .hub-root.compact .page-head { flex-wrap: wrap; row-gap: 6px; padding: 8px 12px; }
  .hub-root.compact .page-head h1 { font-size: var(--fs-title); }
  .hub-root.compact .h1-edit { font-size: var(--fs-title); }
  .hub-root.compact .feed { padding: 14px 10px 18px; gap: 9px; }
  .hub-root.compact .msg, .hub-root.compact .prompt { max-width: 91%; }
  .hub-root.compact .composer { padding: 8px 9px calc(8px + env(safe-area-inset-bottom)); }
  .hub-root.compact .compose-shell { padding: 6px 9px; border-radius: 10px; }
  .hub-root.compact .to-chip { max-width: 110px; height: 28px; }
  .hub-root.compact .to-label { display: none; }
  .hub-root.compact .c-input { min-height: 30px; font-size: var(--fs-body); max-height: 40vh; }
  .hub-root.compact .send-btn { width: 32px; height: 32px; right: 6px; bottom: 4.5px; border-radius: 10px; }
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
  /* The global `.page-head h1` rule ellipsizes its own text; with a second child
     it has to be a flex row, or the pencil is pushed out and clipped by the
     heading's own `overflow: hidden` as soon as the name is long. */
  h1 { display: flex; align-items: center; min-width: 0; }
  /* The name is selectable text that ellipsizes; the pencil never shrinks with
     it, and it is the only thing that renames. */
  .h1-text { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .h1-pen {
    flex: none; display: inline-grid; place-items: center; background: none;
    border: 0; padding: 2px; margin-left: 5px; border-radius: 6px; cursor: pointer;
    color: var(--meta-ink); transition: color var(--t-fast), background var(--t-fast);
    /* A 13px glyph is not a touch target: the hit area is padded out to 44px
       without moving the layout, the same trick the primary actions use. */
    position: relative;
  }
  .h1-pen::after { content: ''; position: absolute; inset: -14px; }
  .h1-pen:hover { color: var(--text); background: var(--surface2); }
  .h1-pen:focus-visible { outline: 2px solid var(--accent-line); outline-offset: 2px; }
  .h1-edit {
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    font-size: var(--fs-title); font-weight: 600; color: var(--text);
    min-width: 0; flex: 0 1 auto; width: 22ch; max-width: 100%;
    background: var(--bg2); border: 1px solid var(--accent-line); border-radius: 7px;
    padding: 2px 6px; box-sizing: border-box;
  }
  .h1-edit:focus { outline: none; }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--status-ok); flex: none; }
  .dot.off { background: var(--text3); }

  .sidebar { position: relative; background: var(--bg2); border-right: 1px solid var(--border); display: flex; flex-direction: column; min-height: 0; }
  /* Phone: the project list slides over the conversation instead of taking a
     column from it. */
  .sidebar.sheet {
    position: fixed; z-index: 26; inset: 0 auto 0 0; width: min(280px, 82vw);
    box-shadow: 0 0 44px rgba(0,0,0,0.5);
    padding-top: env(safe-area-inset-top);
  }
  .sidebar.sheet .side-row { min-height: 44px; }
  .side-scrim { position: fixed; inset: 0; z-index: 25; background: rgba(0,0,0,0.45); }
  .side-scroll { flex: 1; overflow-y: auto; padding: 8px; }
  .p-name { flex: 1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-weight: 550; }

  .mid { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  /* Fits → shows whole; doesn't fit → pans (wheelX action), NEVER an ellipsis.
     `flex: 0 1 auto` + min-width lets the header's buttons take their space
     first while the path yields, and the hidden scrollbar keeps the header one
     quiet line. */
  .path {
    font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub); color: var(--text3);
    white-space: nowrap; overflow-x: auto; overflow-y: hidden;
    min-width: 0; flex: 0 1 auto;
    scrollbar-width: none; -webkit-overflow-scrolling: touch;
  }
  .path::-webkit-scrollbar { display: none; }
  .spacer { flex: 1; }
  .term-toggle.on { border-color: var(--accent); color: var(--accent); background: var(--accent-bg); }

  /* The roster: one line per agent, on every screen size. It answers "who is
     here and are they busy" — anything more was a wall of cards. */
  /* The archive: a short review list, not a second conversation — so it borrows
     the feed's surface and none of its bubble language. */
  .arch-bar {
    display: flex; align-items: center; gap: 6px; width: 100%;
    background: var(--surface); border: none; border-bottom: 1px solid var(--border2);
    color: var(--text3); font-size: var(--fs-meta); padding: 6px 14px; cursor: pointer;
  }
  .arch-bar:hover, .arch-bar.on { color: var(--text); }
  .arch { flex: 1; min-height: 0; overflow-y: auto; padding: 10px 14px; display: flex; flex-direction: column; gap: 8px; }
  .arow { border: 1px solid var(--border); border-radius: 10px; padding: 8px 10px; background: var(--surface); }
  .ar-head { display: flex; gap: 8px; align-items: baseline; font-size: var(--fs-meta); color: var(--text3); }
  .ar-who { font-family: ui-monospace, Menlo, monospace; color: var(--text2); }
  /* An archived body is EVIDENCE, not prose: it keeps its newlines and is clamped
     rather than rendered, so what you are about to forget is what you see. */
  .ar-body {
    font-size: var(--fs-sub); color: var(--text2); margin: 4px 0 6px;
    white-space: pre-wrap; overflow: hidden; display: -webkit-box;
    -webkit-box-orient: vertical; -webkit-line-clamp: 4; line-clamp: 4;
  }
  .ar-acts { display: flex; gap: 6px; }
  .ar-acts button {
    display: flex; align-items: center; gap: 4px; background: none; cursor: pointer;
    border: 1px solid var(--border); border-radius: 7px; color: var(--text3);
    font-size: var(--fs-meta); padding: 3px 8px;
  }
  .ar-acts button:hover { color: var(--text); border-color: var(--input-border); }
  .ar-acts button.danger:hover { color: var(--danger); border-color: var(--danger); }
  .feed.hidden { display: none; }
  .cards { display: flex; gap: 6px; padding: 8px 14px; overflow-x: auto; border-bottom: 1px solid var(--border2); scrollbar-width: none; }
  .cards::-webkit-scrollbar { display: none; }
  .acard {
    position: relative; flex: none; display: flex; flex-direction: column;
    align-items: stretch; justify-content: center; gap: 1px; overflow: hidden;
    min-height: 34px; background: var(--surface); border: 1px solid var(--border);
    border-radius: 9px; padding: 4px 10px 4px 5px; cursor: pointer; text-align: left;
    font-size: var(--fs-ui); color: var(--text2); transition: border-color var(--t-fast), color var(--t-fast);
    -webkit-tap-highlight-color: transparent;
  }
  /* The identity row — what the card used to be in its entirety. */
  .ac-top { display: flex; align-items: center; gap: 6px; }
  /* The sniffed reading: the smallest step on the scale, in monospace so a model
     id and a percentage do not reflow as they change. */
  .ac-vitals {
    font-size: var(--fs-micro); color: var(--meta-ink); line-height: 1.35;
    font-family: ui-monospace, Menlo, monospace; padding: 0 1px 1px 5px;
    max-width: 22ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
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
  /* The `+` card holds one row of two items — it never grew a second line. */
  .acard.add { flex-direction: row; align-items: center; gap: 6px; color: var(--text3); padding-right: 12px; }
  .acard.add:hover { color: var(--accent); }
  .acard.off { opacity: 0.55; }
  .acard.off:hover { opacity: 1; border-color: var(--accent); }
  /* An action is in flight: the card stops taking clicks (the handler guards
     too — this is the visible half of that). */
  .acard.busy { opacity: 0.35; pointer-events: none; }
  .a-name { font-family: ui-monospace, Menlo, monospace; font-weight: 600; max-width: 12ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .s-age { color: var(--text3); font-size: var(--fs-meta); font-variant-numeric: tabular-nums; font-family: ui-monospace, Menlo, monospace; }
  .st { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .st.live { animation: s-pulse 1.4s ease-in-out infinite; }
  .unread { width: 7px; height: 7px; border-radius: 50%; background: var(--status-danger); flex: none; }
  .ava.dim { background: var(--surface2) !important; color: var(--text3); }
  /* Secondary and destructive actions hide until asked for. */
  .a-more { display: grid; place-items: center; width: 20px; height: 22px; border-radius: 6px; color: var(--text3); flex: none; }
  .a-more:hover { color: var(--text); background: var(--surface2); }
  /* The agent action menu: a fixed popover, positioned in JS from the trigger's
     rect (see toggleAgentMenu). It speaks the same dialect as .to-menu — same
     surface, radius, shadow and row metrics — because this file should have ONE
     popover language, not one per feature. Invisible until measured so the
     clamp/flip cannot be seen happening. */
  .a-menu {
    position: fixed; z-index: 24; min-width: 176px; max-width: min(76vw, 280px);
    background: var(--bg); border: 1px solid var(--border); border-radius: 11px;
    box-shadow: 0 12px 34px rgba(0,0,0,0.45); padding: 5px;
    display: flex; flex-direction: column; gap: 2px;
    opacity: 0; transition: opacity var(--t-fast) ease;
  }
  .a-menu.ready { opacity: 1; }
  .am-who {
    font-family: ui-monospace, Menlo, monospace; font-weight: 600;
    font-size: var(--fs-meta); color: var(--text3);
    padding: 4px 10px 5px; border-bottom: 1px solid var(--border2); margin-bottom: 3px;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  /* Menu rows are CONTROLS: --ui-font-control (= --fs-sub) is the app's size
     for those, and it is what the action bar used before this became a menu.
     --fs-ui read as oversized for a menu (owner, 2026-08-19). */
  .a-menu button {
    display: flex; align-items: center; gap: 8px; min-height: 36px; width: 100%; text-align: left;
    background: none; border: none; border-radius: 8px; color: var(--text2);
    padding: 6px 10px; font-size: var(--ui-font-control); cursor: pointer; font-family: ui-monospace, Menlo, monospace;
  }
  /* Touch contract: a menu row is a tap target, so the phone keeps 44px rows
     even though the type got smaller. */
  .hub-root.compact .a-menu button, .hub-root.compact .to-menu button { min-height: 44px; }
  .a-menu button:hover { background: var(--surface2); color: var(--text); }
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
     what shrinks is the TEXT, folded by elideMiddle before it is rendered (owner,
     2026-08-19: "我希望是消息内容自己内部折叠 不是框截断 … 气泡什么的都要完整的不要
     任何裁切"). A cap or a clip would cut the bubble itself, which is exactly the
     thing that read as broken in the two earlier attempts. */
  .msg.held { -webkit-backdrop-filter: blur(10px); backdrop-filter: blur(10px); }
  .msg.held .bubble { box-shadow: 0 6px 20px rgba(0, 0, 0, 0.28); }
  /* EXPANDED while held: the one state where the body scrolls itself. A pinned
     bubble ignores the feed's scrolling, so a message taller than the screen had
     an unreachable bottom half. The cap is on the BODY — border, radius and meta
     trailer stay whole — and only in this state: folded bubbles keep the
     no-clipping rule, and back in the normal flow the message reads full-length.
     The blink hazard that banned max-height on `.held` was flow-height feedback
     through scroll anchoring; the feed has `overflow-anchor: none` now, and this
     cap is entered only by an explicit click, not by the boundary test. */
  .msg.held .m-body.held-scroll {
    max-height: min(62vh, 560px);
    overflow-y: auto; overscroll-behavior: contain; scrollbar-width: thin;
    /* Room for the scrollbar: the trailer floats at the text's right edge, and a
       bar drawn over it reads as clutter. */
    padding-right: 6px;
  }

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
  .msg { position: relative; display: flex; flex-direction: column; max-width: min(76%, 760px); }
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
    border-radius: 12px 12px 12px 4px; padding: 8px 12px 9px;
    color: var(--text); font-size: var(--fs-body); line-height: 1.48;
    word-break: break-word; overflow-wrap: anywhere; cursor: text;
    box-shadow: 0 1px 2px rgba(0,0,0,0.10);
    transition: border-color var(--t-fast) ease, box-shadow var(--t-fast) ease;
    -webkit-tap-highlight-color: transparent;
  }
  .bubble:hover { border-color: var(--input-border); }
  .msg.me .bubble {
    background: var(--bubble-out); border-color: color-mix(in srgb, var(--accent) 18%, transparent);
    border-radius: 12px 12px 4px 12px;
  }
  /* Agent name heads the bubble (your own carries none — the right-aligned
     accent bubble already says "yours"). */
  /* Agent name heads the bubble (your own carries none — the right-aligned
     accent bubble already says "yours"). A flex row so the status badge can
     sit at the bubble's TOP-RIGHT ("放到这个消息的右侧 往右上角放", owner
     2026-08-20) while the name keeps the left edge. */
  .m-head {
    display: flex; align-items: center; gap: 8px;
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
    background: var(--bubble-in); border: 1px solid var(--border); border-radius: 7px;
    color: var(--text2); padding: 3px 10px; font-size: var(--fs-meta); cursor: pointer;
    box-shadow: 0 2px 8px rgba(0,0,0,0.18);
  }
  .m-acts button:hover, .m-acts button.on { color: var(--accent); border-color: color-mix(in srgb, var(--accent) 45%, transparent); }
  /* Referenced images, under the text they came with. */
  .shots { display: flex; flex-direction: column; gap: 6px; margin-top: 6px; border-radius: 16px; overflow: hidden; }
  /* Lifecycle lines are the app narrating real actions (an agent stopped, a
     /command typed into a pane) — reading ink and body-adjacent size, not fine
     print the reader has to squint at ("不要只用灰色小字，让我看不太清", owner
     2026-08-20). Still a centred capsule: it is narration, not a speaker. */
  .sysline {
    align-self: center; display: flex; align-items: baseline; gap: 6px;
    max-width: min(92%, 620px); padding: 4px 13px; border-radius: 8px;
    color: var(--text2); background: color-mix(in srgb, var(--bubble-in) 88%, transparent);
    border: 1px solid var(--border2); box-shadow: 0 1px 2px rgba(0,0,0,0.06);
    font-size: var(--fs-sub); white-space: nowrap; overflow: hidden;
    -webkit-backdrop-filter: blur(8px); backdrop-filter: blur(8px);
  }
  .sysline .sys-sep { color: var(--text3); flex: none; }
  .sysline .sys-item { min-width: 0; overflow: hidden; text-overflow: ellipsis; }
  /* A /command wears the composer's command dialect — monospace with an accent
     lean — so the record reads as the thing that was typed. */
  .sysline .sys-item.cmd {
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
    color: color-mix(in srgb, var(--accent) 62%, var(--text));
  }

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
  .prompt { align-self: flex-start; max-width: min(76%, 760px); border-left: 2px solid var(--border); padding-left: 9px; margin: 1px 6px; }
  .p-head { display: flex; align-items: baseline; gap: 7px; font-size: var(--fs-meta); color: var(--text3); margin-bottom: 2px; }
  .p-head .p-who { font-family: ui-monospace, Menlo, monospace; font-weight: 600; color: var(--text2); }
  .p-tag { text-transform: uppercase; letter-spacing: 0.8px; font-size: var(--fs-micro); color: var(--text3); border: 1px solid var(--border); border-radius: 4px; padding: 0 4px; }
  .p-body { font-size: var(--fs-ui); color: var(--text2); white-space: pre-wrap; word-break: break-word; overflow-wrap: anywhere; max-height: 7.5em; overflow: hidden; }

  /* A single observed fact: status declaration, lifecycle hook, warning. */
  .note {
    display: flex; align-items: baseline; gap: 8px; width: min(76%, 760px);
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
    background: var(--lane-bg); border: 1px solid var(--border2); border-radius: 9px;
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
     still going. */
  .s-live { flex: none; width: 7px; height: 7px; border-radius: 50%; background: var(--accent); animation: s-pulse 1.4s ease-in-out infinite; }
  @keyframes s-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }
  @media (prefers-reduced-motion: reduce) { .s-live { animation: none; } }
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
     rows and lines are the same thing here. */
  .s-body.capped {
    max-height: calc(var(--steps-rows) * (1.5em + 2px) + 11px);
    overflow-y: auto; overscroll-behavior: contain;
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
  }
  .compose-shell {
    flex: 1; min-width: 0; position: relative;
    padding: 6px 10px; border: 1px solid var(--input-border); border-radius: 10px;
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
    border-radius: 7px; padding: 0 9px; font-size: var(--fs-sub); font-weight: 650;
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
    min-width: 168px; max-height: 46vh; overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: 11px;
    box-shadow: 0 12px 34px rgba(0,0,0,0.45); padding: 5px; display: flex; flex-direction: column; gap: 2px;
  }
  .to-menu button {
    display: flex; align-items: center; gap: 7px; min-height: 36px; width: 100%; text-align: left;
    background: none; border: none; border-radius: 8px; color: var(--text2);
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
    max-height: 44vh; overflow-y: auto; scrollbar-width: thin;
    background: var(--bg); border: 1px solid var(--border); border-radius: 11px;
    box-shadow: 0 12px 34px rgba(0,0,0,0.45); padding: 5px;
    display: flex; flex-direction: column; gap: 2px;
  }
  .cmd-opt {
    display: flex; align-items: baseline; gap: 10px; width: 100%; text-align: left;
    background: none; border: none; border-radius: 8px; color: var(--text2);
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
    display: block; width: 100%; min-height: 28px; max-height: 34vh;
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
  .send-btn {
    position: absolute; right: 7px; bottom: 5.5px;
    width: 30px; height: 30px; display: grid; place-items: center;
    padding: 0; border: none; border-radius: 9px; cursor: pointer;
    background: var(--accent-fill);
    color: var(--accent-fill-ink);
    transition: filter var(--t-fast) ease, background var(--t-fast) ease, color var(--t-fast) ease, transform var(--t-fast) ease;
  }
  .send-btn:hover:not(:disabled) { filter: brightness(1.07); }
  .send-btn:active:not(:disabled) { transform: scale(0.93); }
  .send-btn:disabled { background: var(--surface2); color: var(--text3); cursor: default; }
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
    background: var(--surface); border: 1px solid var(--border); border-radius: 11px;
    color: var(--text); padding: 8px 11px; font-size: var(--fs-ui); cursor: pointer;
  }
  .start-row:hover { border-color: var(--accent); background: var(--accent-bg); }
  .start-row:disabled { opacity: 0.5; }
  .sr-name { font-family: ui-monospace, Menlo, monospace; font-weight: 600; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sr-backend { font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub); color: var(--text3); margin-left: auto; }
  .sr-cap { font-size: var(--fs-micro); color: var(--accent); border: 1px solid var(--accent); border-radius: 4px; padding: 0 3px; opacity: 0.75; }

  .drawer { position: relative; }
  .drawer { display: flex; flex-direction: column; min-width: 0; min-height: 0; background: #000; border-left: 1px solid var(--border); }
  .drawer-head { display: flex; align-items: center; gap: 8px; padding: 6px 10px; background: var(--bg2); border-bottom: 1px solid var(--border); }
  .win-list { display: flex; gap: 5px; overflow-x: auto; scrollbar-width: none; }
  .win-list::-webkit-scrollbar { display: none; }
  .win-pill { display: flex; align-items: center; gap: 5px; flex: none; background: var(--surface); border: 1px solid var(--border); border-radius: 7px; color: var(--text2); padding: 4px 9px; font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-sub); cursor: pointer; }
  .win-pill.cur { border-color: var(--accent); color: var(--accent); background: var(--accent-bg); }
  .direct-tag { font-size: var(--fs-micro); color: var(--text3); border: 1px solid var(--border); border-radius: 4px; padding: 0 4px; margin-left: 3px; }
  .term-body { flex: 1; min-width: 0; min-height: 0; position: relative; display: flex; flex-direction: column; }

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
    background: var(--bg); border: 1px solid var(--border); border-radius: 14px;
    box-shadow: 0 18px 60px rgba(0,0,0,0.5); padding: 18px; display: flex; flex-direction: column; gap: 10px;
  }
  .dlg h2 { margin: 0 0 4px; font-size: var(--fs-title); }  /* Phone: dialogs become bottom sheets — reachable with a thumb, and they
     never fight the on-screen keyboard for the middle of the screen. */
  .dlg.sheet {
    left: 0; top: auto; bottom: 0; transform: none;
    width: 100%; max-width: none; max-height: 82vh;
    border-radius: 16px 16px 0 0; border-left: none; border-right: none; border-bottom: none;
    padding: 16px 14px calc(16px + env(safe-area-inset-bottom));
  }
  .dlg.sheet .dlg-agents { max-height: 46vh; overflow-y: auto; }
  .dlg.sheet .agent-pick, .dlg.sheet input, .dlg.sheet .dlg-actions button { min-height: 44px; }
  .dlg input { background: var(--input-bg); border: 1px solid var(--input-border); border-radius: 9px; color: var(--text); padding: 8px 12px; font-size: var(--fs-ui); outline: none; }
  .dlg input:focus { border-color: var(--accent); }
  .dlg-agents { display: flex; flex-direction: column; gap: 5px; }
  .agent-pick { display: flex; align-items: center; gap: 8px; background: var(--surface); border: 1px solid var(--border); border-radius: 9px; color: var(--text2); padding: 8px 11px; font-size: var(--fs-ui); cursor: pointer; text-align: left; }
  .agent-pick.sel { border-color: var(--accent); color: var(--text); background: var(--accent-bg); }
  .agent-pick :global(svg) { margin-left: auto; color: var(--accent); }
  .dlg-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 6px; }
</style>
