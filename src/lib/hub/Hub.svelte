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
  import { t } from '../core/i18n.svelte.ts';
  import {
    projectList, projectUp, projectCreate, listSessionsWithPanes,
    hubPost, hubLog, hubAgents, hubSpawn, hubAgentStop, hubAgentRestart, hubActivity, registryList,
    addTeamMessageListener, removeTeamMessageListener,
  } from '../core/ws.ts';
  import { sortRows, shortPath } from '../projects/projects.ts';
  import { stateDotColor, mergeMessages, statuslineWindows, backendColor, feedBlocks, systemLine, pickLead, addressed, fmtElapsed, unreadSenders, splitImages, stoppedAgents, toolColor, STEPS_PREVIEW, pickAnchor, toolEventParts } from './hub.ts';
  import { hubPrefs } from './hub-prefs.svelte.ts';
  import { renderMarkdown } from '../core/markdown.ts';

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
  let createPath = $state('');
  let createName = $state('');
  let createAgents = $state([]);    // selected registry names
  let creating = $state(false);

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
      const [{ projects }, sp] = await Promise.all([projectList(), listSessionsWithPanes()]);
      rows = sortRows(projects);
      panes = sp.panes ?? [];
      if (!selected && rows.length) selectProject(rows[0].project.session);
    } catch { /* server without projects — the Hub tab is hidden anyway */ }
  }

  async function selectProject(session) {
    selected = session;
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
    await Promise.all([loadFeed(), loadAgents(), loadActivity()]);
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
    } catch { agents = []; }
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
  function syncAsk(direction = askDir, reset = false) {
    if (!feedEl) { askKey = ''; askEdge = ''; askHeld = false; return; }
    // Chromium's offsetTop for a sticky element is its HELD position. Read that
    // and the old anchor appears naturally visible, so the next anchor is never
    // selected. Neutralize the one current sticky element for this synchronous
    // layout read; the inline override is removed before the browser can paint.
    const stickies = [...feedEl.querySelectorAll('.ask-top, .ask-bottom')];
    for (const el of stickies) el.style.position = 'static';
    const items = [...feedEl.querySelectorAll('[data-ask]')].map((el) => ({
      key: el.dataset.ask ?? '',
      top: el.offsetTop,
      height: el.offsetHeight,
    }));
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
    // momentum) flips the prepared edge top<->bottom and held with it — the
    // one-line collapse blinking on and off was the reported flicker. While
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
    let text = composerText.trim();
    if (!text || !selected) return;
    // The recipient makes "talk to THIS agent" the default rather than a
    // gesture: addressed() prefixes @name unless the user @-addressed someone
    // by hand, and an empty recipient posts to the room.
    text = addressed(text, recipient);
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
  function growComposer() {
    const el = composerEl;
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = `${el.scrollHeight}px`;
    // The composer taking space is the feed losing it — the same way the keyboard
    // does — so keep the tail parked while it grows.
    if (following) scrollFeed(true);
  }
  $effect(() => {
    void composerText;   // includes the reset to '' after sending
    void toChipW;        // the indent changes wrapping, so height must re-measure
    growComposer();
  });

  /** Enter sends where there is a keyboard with modifiers, and inserts a newline
   * on a touch device — where the return key is the ONLY way to get one and the
   * send button is right there. Shift+Enter is always a newline. */
  function onComposerKey(e) {
    if (e.key !== 'Enter' || e.shiftKey || e.isComposing) return;
    if (compact) return;      // let the newline through; tap send
    e.preventDefault();
    send();
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
    termOpen = true;
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

  // New project: create → up → spawn each chosen agent. Client-side
  // orchestration on purpose: each step is an existing, observable RPC.
  async function createProject() {
    const path = createPath.trim();
    if (!path || creating) return;
    creating = true;
    try {
      const r = await projectCreate(path, createName.trim() ? { name: createName.trim() } : {});
      const proj = r.project ?? r;
      await projectUp(proj.id);
      for (const name of createAgents) {
        try { await hubSpawn(proj.session, name); } catch (e) { console.warn('spawn failed', name, e); }
      }
      createOpen = false;
      createPath = '';
      createName = '';
      createAgents = [];
      await reload();
      await selectProject(proj.session);
    } catch (e) {
      console.warn('create failed', e);
    } finally {
      creating = false;
    }
  }

  function toggleCreateAgent(name) {
    createAgents = createAgents.includes(name)
      ? createAgents.filter((n) => n !== name)
      : [...createAgents, name];
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
  let pendingAct = $state(null);   // { kind: 'stop', name }
  let acting = $state(false);
  const askAction = (kind, name) => { pendingAct = { kind, name }; };

  async function runAction() {
    if (!pendingAct || acting) return;
    const { name } = pendingAct;
    acting = true;
    try {
      await hubAgentStop(selected, name);
      await Promise.all([reload(), loadAgents(), loadFeed()]);
    } catch (e) {
      console.warn('stop failed', e);
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
    const onKey = (e) => { if (e.key === 'Escape') { termOpen = false; e.stopPropagation(); } };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  });

  const blocks = $derived(feedBlocks(feed, activity, hubPrefs.feedLevel));
  const windowName = (w) => agents.find((a) => a.window === w)?.name ?? `#${w}`;

  // Disclosure lives outside the row: `undefined` means "nobody chose", so the
  // group follows the agent (open while it works, closed when it is done) and
  // an explicit choice sticks. Keyed by group so re-renders can't lose it.
  let stepsChoice = $state({});
  let stepsAll = $state({});        // group key → show every step, not the tail
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
    b.key === newestSteps[b.window] && agents.find((a) => a.window === b.window)?.state === 'working';
  const stepsOpen = (b) => stepsChoice[b.key] ?? isRunning(b);
  const toggleSteps = (b, open) => { stepsChoice[b.key] = open; };
  /** Per window, the key of its LAST step group: only that one can be running. */
  const newestSteps = $derived.by(() => {
    const last = {};
    for (const b of blocks) if (b.type === 'steps') last[b.window] = b.key;
    return last;
  });
  const blockKey = (b, i) =>
    b.type === 'msg' ? (b.msg.id ?? `m${b.ts}-${i}`) : b.type === 'steps' ? b.key : `${b.type}${b.ts}-${i}`;

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
  let tick = $state(Date.now());
  $effect(() => {
    if (!visible) return;
    const id = setInterval(() => { tick = Date.now(); }, 1000);
    return () => clearInterval(id);
  });

  const winsForStatusline = $derived(statuslineWindows(agents, termTarget));
  const fmtTime = (ts) => {
    const d = new Date(ts);
    return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  };
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
          <button class="side-row" class:open={row.project.session === selected} onclick={() => { selectProject(row.project.session); sideOpen = false; }}>
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
        <h1>{selectedRow?.project.name ?? ''}</h1>
        {#if !compact}<span class="path">{shortPath(selectedRow?.project.path ?? '')}</span>{/if}
        <span class="spacer"></span>
        {#if selected && !liveSelected}
          <button class="chip-btn" onclick={bringUp}>{t('projectOpen')}</button>
        {/if}
        <!-- THE terminal affordance: a button, not a permanent pane. Adding an
             agent belongs to the roster row, and chat detail belongs to
             Settings — a header is not a place to keep spare switches. -->
        <button class="chip-btn term-toggle" class:on={termOpen} title={t('hubTerminal')} onclick={() => termOpen && !compact ? termOpen = false : openDrawer()}>
          <Icon name="terminal" size={14} />{#if !compact}<span>{t('hubTerminal')}</span>{/if}
        </button>
      </div>

      {#if managedAgents.length || stopped.length}
        <!-- The roster. Tapping an agent makes it the recipient (and this
             project's lead) — the phone gets chips, the desktop gets cards. -->
        <div class="cards" class:chips={compact}>
          {#each managedAgents as a (a.window)}
            <!-- A div, not a button: the dot menu inside contains real buttons,
                 and a button inside a button is invalid HTML the browser
                 silently reshuffles. -->
            <div class="acard" class:sel={recipient === a.name} role="button" tabindex="0"
              title={`${a.name} · ${stateLabel(a.state)}${a.detail ? ' · ' + a.detail : ''}`}
              onclick={() => setRecipient(a.name)}
              onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); setRecipient(a.name); } }}>
              <span class="ava" style:background={backendColor(a.agent)}>{a.name.slice(0, 1).toUpperCase()}</span>
              <span class="a-name">{a.name}</span>
              <span class="st" class:live={a.state === 'running'} style:background={stateDotColor(a.state)}></span>
              {#if a.since}<span class="s-age">{fmtElapsed(a.since, tick)}</span>{/if}
              {#if unread.has(a.name)}<span class="unread" title={t('hubUnread')}></span>{/if}
              <!-- Destructive and secondary actions stay behind a dot menu: a
                   roster is for seeing who is here, not a row of hazards. -->
              <span class="a-more" role="button" tabindex="-1" title={t('hubMore')}
                onclick={(e) => { e.stopPropagation(); menuFor = menuFor === a.name ? '' : a.name; }}
                onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); menuFor = menuFor === a.name ? '' : a.name; } }}>
                <Icon name="dots" size={13} />
              </span>
            </div>
          {/each}
          <!-- Stopped agents: declared by the project, no window right now.
               Starting one resumes its conversation, so it stays on the roster
               instead of vanishing from the room it belongs to. -->
          {#each stopped as name (name)}
            <button class="acard off" disabled={acting} onclick={() => startAgent(name)} title={t('hubStartAgain')}>
              <span class="ava dim">{name.slice(0, 1).toUpperCase()}</span>
              <span class="a-name">{name}</span>
              <span class="s-age">{t('hubStopped')}</span>
              <Icon name="refresh" size={11} />
            </button>
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
        <!-- Actions for one agent. A bar rather than a popover inside the chip:
             the roster scrolls horizontally, and a scroll container clips
             anything absolutely positioned inside it. -->
        <div class="a-bar">
          <span class="ab-who">{menuFor}</span>
          <button onclick={() => { const a = managedAgents.find((x) => x.name === menuFor); menuFor = ''; if (a) openDrawer(a); }}>
            <Icon name="terminal" size={12} />{t('hubWatch')}
          </button>
          <button class="danger" onclick={() => { const n = menuFor; menuFor = ''; askAction('stop', n); }}>
            <Icon name="stop" size={12} />{t('hubStop')}
          </button>
          <span class="spacer"></span>
          <button class="ab-x" onclick={() => menuFor = ''} title={t('cancel')}><Icon name="x" size={12} /></button>
        </div>
      {/if}

      <div class="feed-wrap">
      <div class="feed subtle-scroll" bind:this={feedEl} onscroll={onFeedScroll}>
        {#each blocks as b, i (blockKey(b, i))}
          {#if b.type === 'msg'}
            {@const m = b.msg}
            {@const sys = systemLine(m.body)}
            {@const parts = splitImages(m.body)}
            {#if sys !== null}
              <div class="sysline"><span class="sys-who">{m.from}</span>{sys}</div>
            {:else}
              <!-- Every user message can become the landmark, but exactly ONE
                   does. The real bubble enters with the feed, then that SAME
                   element catches the edge as it is about to leave; there is no
                   duplicate and no invisible midpoint swap. -->
              {@const key = blockKey(b, i)}
              {@const isAsk = m.from === 'human' && sys === null}
              <div class="msg" class:me={m.from === 'human'}
                class:ask-top={isAsk && askKey === key && askEdge === 'top'}
                class:ask-bottom={isAsk && askKey === key && askEdge === 'bottom'}
                class:held={isAsk && askKey === key && askHeld}
                data-ask={isAsk ? key : undefined}>
                <!-- Sender, timestamp and content are one visual object. This is
                     also the exact element that sticky moves — no detached label
                     that could make the anchor look duplicated. -->
                <!-- svelte-ignore a11y_no_static_element_interactions -->
                <div class="bubble md" role="button" tabindex="0"
                  onclick={() => { msgOpen = msgOpen === key ? '' : key; }}
                  onkeydown={(e) => { if (e.key === 'Enter') msgOpen = msgOpen === key ? '' : key; }}>
                  <div class="m-head">
                    {#if m.from !== 'human'}<span class="who">{m.from}</span>{/if}
                    <span class="m-time">{fmtTime(m.ts)}</span>
                    <!-- The agent's own prompt hook echoed this line back, so it
                         reached the CLI's input — not merely typed at the pane. -->
                    {#if b.delivered}
                      <span class="ok-chip" title={t('hubDeliveredHint')}><Icon name="check" size={9} />{t('hubDelivered')}</span>
                    {/if}
                  </div>
                  {#if parts.text}
                    <div class="m-body">
                      {#if rawOpen === key}
                        <pre class="raw">{m.body}</pre>
                      {:else}
                        {@html renderMarkdown(parts.text)}
                      {/if}
                    </div>
                  {/if}
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
                {#if parts.images.length}
                  <div class="shots">
                    {#each parts.images as src, k (`${k}-${src}`)}
                      <ChatImage {src} alt={m.from} />
                    {/each}
                  </div>
                {/if}
              </div>
            {/if}
          {:else if b.type === 'prompt'}
            <!-- The input half: what this agent was asked, which only the
                 userPromptSubmit hook can tell us. -->
            <div class="prompt">
              <div class="p-head"><span class="p-who">{windowName(b.window)}</span><span class="p-tag">{t('hubPromptIn')}</span><span>{fmtTime(b.ts)}</span></div>
              <div class="p-body">{b.text}</div>
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
                {@const shown = stepsAll[b.key] ? b.events : b.events.slice(-STEPS_PREVIEW)}
                <div class="s-body">
                  {#if shown.length < b.events.length}
                    <button class="s-all" onclick={() => { stepsAll[b.key] = true; }}>
                      {t('hubStepsAll').replace('{n}', String(b.events.length))}
                    </button>
                  {/if}
                  {#each shown as e, j (`${e.ts}-${j}`)}
                    {@const ep = toolEventParts(e)}
                    <div class="step">
                      <!-- The tool NAME is the scannable half: its own colour by
                           what the tool does. toolEventParts splits the name off
                           legacy events that glued it onto the text — those were
                           the "still grey" rows. -->
                      {#if ep.tool}<span class="tname" style:color={toolColor(ep.tool)}>{ep.tool}</span>{/if}
                      <span class="st-text">{ep.text}</span>
                      <span class="st-ts">{fmtTime(e.ts)}</span>
                    </div>
                  {/each}
                </div>
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
        <div class="compose-shell">
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
          <Icon name="send" size={16} />
        </button>
        </div>
      </div>
    </main>

    {#if termOpen && !compact}
    <!-- ── Terminal drawer: where terminal things live ── -->
    <section class="drawer">
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
        <button class="icon-btn" title={t('hubOpenFull')} onclick={() => { const m = /^(.+):(\d+)\.(\d+)$/.exec(termTarget); if (m) openTerminal(selected, termTarget, termCommand); }}>
          <Icon name="maximize" size={14} />
        </button>
        <button class="icon-btn" title="Esc" onclick={() => termOpen = false}>
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
      <footer class="statusline">
        <span class="sess">{selected || '—'}</span>
        <div class="wlist">
          {#each winsForStatusline as w (w.window)}
            <button class="w" class:cur={w.current} onclick={() => pickWindow(w)}>{w.label}</button>
          {/each}
        </div>
        <div class="right"><span>{managedAgents.length} agents · {working} working</span></div>
      </footer>
    </section>
    {/if}
  </div>

  {#if pendingAct}
    <!-- ── Stop one agent ── -->
    <div class="dlg-backdrop" onclick={() => pendingAct = null} role="presentation"></div>
    <div class="dlg" class:sheet={compact}>
      <h2>{t('hubStopTitle').replace('{name}', pendingAct.name)}</h2>
      <p class="dlg-note">{t('hubStopNote')}</p>
      <div class="dlg-actions">
        <button class="chip-btn" onclick={() => pendingAct = null}>{t('cancel')}</button>
        <button class="chip-btn primary danger" disabled={acting} onclick={runAction}>
          {acting ? '…' : t('hubStop')}
        </button>
      </div>
    </div>
  {/if}

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
    <!-- ── New project: path + name + WHICH AGENTS ── -->
    <div class="dlg-backdrop" onclick={() => createOpen = false} role="presentation"></div>
    <div class="dlg" class:sheet={compact}>
      <h2>{t('projectNew')}</h2>
      <input placeholder={t('hubCreatePath')} bind:value={createPath} />
      <input placeholder={t('hubCreateName')} bind:value={createName} />
      <div class="dlg-h">{t('hubCreateAgents')}</div>
      <div class="dlg-agents">
        {#each registry as r (r.name)}
          <button class="agent-pick" class:sel={createAgents.includes(r.name)} onclick={() => toggleCreateAgent(r.name)}>
            <span class="ava" style:background={backendColor(r.backend)}>{r.name.slice(0, 1).toUpperCase()}</span>
            {r.name} · {r.backend}
            {#if createAgents.includes(r.name)}<Icon name="check" size={13} />{/if}
          </button>
        {/each}
      </div>
      <div class="dlg-actions">
        <button class="chip-btn" onclick={() => createOpen = false}>{t('cancel')}</button>
        <button class="chip-btn primary" disabled={!createPath.trim() || creating} onclick={createProject}>
          {creating ? '…' : t('hubCreateGo')}
        </button>
      </div>
    </div>
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
  }
  .cols { flex: 1; display: grid; grid-template-columns: var(--sidebar-w) minmax(0, 1fr); min-height: 0; }
  .hub-root.compact .cols { grid-template-columns: minmax(0, 1fr); }
  /* Phone shape: tighter gutters, thumb-sized controls, no horizontal
     overflow. The page head wraps instead of pushing the chips off-screen. */
  .hub-root.compact .page-head { flex-wrap: wrap; row-gap: 6px; padding: 8px 12px; }
  .hub-root.compact .page-head h1 { font-size: 15px; }
  .hub-root.compact .feed { padding: 14px 10px 18px; gap: 9px; }
  .hub-root.compact .msg, .hub-root.compact .prompt { max-width: 91%; }
  .hub-root.compact .composer { padding: 8px 9px calc(8px + env(safe-area-inset-bottom)); }
  .hub-root.compact .compose-shell { padding: 6px 48px 6px 9px; border-radius: 21px; }
  .hub-root.compact .to-chip { max-width: 110px; height: 28px; }
  .hub-root.compact .to-label { display: none; }
  .hub-root.compact .c-input { min-height: 30px; font-size: 14px; max-height: 40vh; }
  .hub-root.compact .send-btn { width: 38px; height: 38px; right: 5px; bottom: 4px; }
  .hub-root.compact .chip-btn { min-height: 34px; }
  .hub-root.compact .s-head { min-height: 34px; }
  /* Drawer open: the conversation yields but stays present. */
  .hub-root.drawer-open .cols { grid-template-columns: var(--sidebar-w) minmax(280px, 0.8fr) minmax(360px, 1.2fr); }
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
  .path { font-family: ui-monospace, Menlo, monospace; font-size: 11px; color: var(--text3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .spacer { flex: 1; }
  .term-toggle.on { border-color: var(--accent); color: var(--accent); background: var(--accent-bg); }

  /* The roster: one line per agent, on every screen size. It answers "who is
     here and are they busy" — anything more was a wall of cards. */
  .cards { display: flex; gap: 6px; padding: 8px 14px; overflow-x: auto; border-bottom: 1px solid var(--border2); scrollbar-width: none; }
  .cards::-webkit-scrollbar { display: none; }
  .acard {
    position: relative; flex: none; display: flex; align-items: center; gap: 6px;
    min-height: 34px; background: var(--surface); border: 1px solid var(--border);
    border-radius: 999px; padding: 4px 10px 4px 5px; cursor: pointer; text-align: left;
    font-size: 12.5px; color: var(--text2); transition: border-color 160ms, color 160ms;
    -webkit-tap-highlight-color: transparent;
  }
  .acard:hover { border-color: var(--input-border); color: var(--text); }
  .acard.sel { border-color: var(--accent); background: var(--accent-bg); color: var(--text); }
  .acard.add { color: var(--text3); padding-right: 12px; }
  .acard.add:hover { color: var(--accent); }
  .acard.off { opacity: 0.55; }
  .acard.off:hover { opacity: 1; border-color: var(--accent); }
  .a-name { font-family: ui-monospace, Menlo, monospace; font-weight: 600; max-width: 12ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .s-age { color: var(--text3); font-size: 10.5px; font-variant-numeric: tabular-nums; font-family: ui-monospace, Menlo, monospace; }
  .st { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .st.live { animation: s-pulse 1.4s ease-in-out infinite; }
  .unread { width: 7px; height: 7px; border-radius: 50%; background: var(--status-danger); flex: none; }
  .ava.dim { background: var(--surface2) !important; color: var(--text3); }
  /* Secondary and destructive actions hide until asked for. */
  .a-more { display: grid; place-items: center; width: 20px; height: 22px; border-radius: 6px; color: var(--text3); flex: none; }
  .a-more:hover { color: var(--text); background: var(--surface2); }
  .a-bar {
    display: flex; align-items: center; gap: 6px; padding: 6px 14px;
    border-bottom: 1px solid var(--border2); background: var(--bg2);
  }
  .ab-who { font-family: ui-monospace, Menlo, monospace; font-weight: 600; font-size: 12px; color: var(--text2); }
  .a-bar button {
    display: inline-flex; align-items: center; gap: 5px; min-height: 32px;
    background: var(--surface); border: 1px solid var(--border); border-radius: 8px;
    color: var(--text3); padding: 4px 10px; font-size: 12px; cursor: pointer;
  }
  .a-bar button:hover { color: var(--text); border-color: var(--input-border); }
  .a-bar button.danger:hover { color: var(--status-danger); border-color: var(--status-danger); }
  .a-bar .ab-x { border: none; background: none; padding: 4px 6px; }

  /* Chat canvas: restrained depth rather than a flat admin-panel grey. The two
     broad glows work in both themes and never compete with message text. */
  .feed-wrap { flex: 1; position: relative; display: flex; min-height: 0; background: var(--chat-canvas); }
  .feed {
    flex: 1; overflow-y: auto; padding: 18px clamp(18px, 4vw, 64px) 24px;
    display: flex; flex-direction: column; gap: 10px;
    background:
      radial-gradient(ellipse at 12% 0%, var(--accent-glow), transparent 38%),
      radial-gradient(ellipse at 88% 100%, var(--surface2), transparent 42%);
  }
  /* The active anchor is the message itself. It enters and moves with the feed;
     only when that SAME element reaches an edge does sticky hold it there. */
  .msg.ask-top { position: sticky; top: 0; z-index: 6; }
  .msg.ask-bottom { position: sticky; bottom: 0; z-index: 6; }
  /* Floating treatment begins only after the bubble is actually held. Normal
     and held use the SAME opaque surface, so catching the edge changes depth,
     never identity or colour. */
  /* The held collapse is PAINT-ONLY, never layout. The first version shrank
     the bubble to one line with max-height, which changed its flow height;
     the browser's scroll anchoring compensated scrollTop, which flipped the
     boundary condition back, and the anchor blinked in an infinite layout
     feedback loop (measured: setting scrollTop=2261 landed on 2221↔2298).
     clip-path clips rendering and hit-testing but occupies identical space,
     so holding the edge can never move the scroll position. A short bubble
     yields a negative inset, which simply does not clip. */
  .msg.held { -webkit-backdrop-filter: blur(10px); backdrop-filter: blur(10px); }
  /* Both edges show the SAME preview — head (with its time) + first line —
     inside a 53px window. Geometry (measured): head bottom 25px, first line
     box 29..49px, the NEXT line's glyphs start ≈51px. Everything here is
     paint-only: clip-path clips, transform moves painting, neither touches
     layout (the scroll-anchoring feedback loop is documented above).

     · Top edge: clip the message to its top 53px.
     · Bottom edge: same clip ON THE BUBBLE plus translateY(100% − 53px), so
       the bubble's HEAD slides down into the bottom window — without it the
       window showed the bubble's tail and the time was cut off (owner
       report). The outer .msg clip bounds anything else (image refs).
     · The bare cut had no bottom edge ("少了一个小边边"): ::after paints a
       fake floor — 3px of the bubble's own colour capped by its 1px border
       line — which also covers the next line's 2px glyph sliver. */
  .msg.ask-top.held { clip-path: inset(0 0 calc(100% - 53px) 0 round 17px); }
  .msg.ask-bottom.held { clip-path: inset(calc(100% - 53px) 0 0 0 round 17px); }
  .msg.ask-bottom.held .bubble {
    clip-path: inset(0 0 calc(100% - 53px) 0 round 17px);
    transform: translateY(calc(100% - 53px));
  }
  /* The frame of the held mini-bubble is DRAWN, not inherited. The real
     bubble border cannot survive the clip: its bottom edge lies below the
     window, and its side strokes are eaten by the window's corner rounding —
     the first fake floor put its 1px line at border-y 53..54 and the 53px
     clip removed exactly that line (pseudo-element `top` is padding-box
     relative, 1px lower than the border box the clip measures — the owner
     saw the missing stroke). So while held: hide the real border
     (border-color is paint-only) and let ::before draw the complete frame,
     outer edge exactly on the 53px window, stroke safely INSIDE the clip.
     ::after remains an opaque bar that hides the next line's glyph sliver. */
  .msg.held .bubble, .msg.held.me .bubble { border-color: transparent; }
  .msg.held .bubble::before {
    content: ''; position: absolute; left: -1px; right: -1px; top: -1px; height: 53px;
    border: 1px solid var(--bubble-line); border-radius: 17px;
    pointer-events: none;
  }
  .msg.held.me .bubble::before {
    border-color: color-mix(in srgb, var(--accent) 18%, transparent);
  }
  .msg.held .bubble::after {
    content: ''; position: absolute; left: 0; right: 0; top: 48px; height: 3px;
    background: var(--bubble-in); pointer-events: none;
  }
  .msg.held.me .bubble::after { background: var(--bubble-out); }
  .msg.held .m-head { opacity: 0.72; }

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
  .msg.me { align-self: flex-end; }
  .bubble {
    position: relative;
    background: var(--bubble-in); border: 1px solid var(--bubble-line);
    border-radius: 17px 17px 17px 6px; padding: 8px 12px 9px;
    color: var(--text); font-size: 13.5px; line-height: 1.48;
    word-break: break-word; overflow-wrap: anywhere; cursor: pointer;
    box-shadow: 0 1px 2px rgba(0,0,0,0.10);
    transition: border-color 140ms ease, box-shadow 140ms ease;
    -webkit-tap-highlight-color: transparent;
  }
  .bubble:hover { border-color: var(--input-border); }
  .msg.me .bubble {
    background: var(--bubble-out); border-color: color-mix(in srgb, var(--accent) 18%, transparent);
    border-radius: 17px 17px 6px 17px;
  }
  .m-head {
    display: flex; align-items: center; gap: 7px; min-height: 16px;
    margin: 0 0 4px; color: var(--text3); font-size: 10.5px; line-height: 1;
  }
  .m-head .who { color: var(--accent); font-weight: 650; font-size: 11.5px; letter-spacing: 0.1px; }
  .msg.me .m-head { justify-content: flex-end; }
  .msg.me .m-head .who { color: color-mix(in srgb, var(--accent) 82%, var(--text)); }
  .m-time { font-variant-numeric: tabular-nums; opacity: 0.78; }
  .m-body { min-width: 0; }
  .bubble .raw { margin: 0; font-family: var(--font-mono); font-size: 11.5px; line-height: 1.5; white-space: pre-wrap; overflow-wrap: anywhere; color: var(--text2); }
  /* What you can DO with a message, revealed by tapping it. An OVERLAY on the
     bubble's top edge, out of the flow: opening it must not push the feed
     around or change the scroll height the anchor math depends on. */
  .m-acts {
    position: absolute; z-index: 8; top: -13px; right: 10px;
    display: flex; gap: 5px; margin: 0;
  }
  .msg:not(.me) .m-acts { right: auto; left: 10px; }
  .m-acts button {
    display: inline-flex; align-items: center; gap: 4px; min-height: 26px;
    background: var(--bubble-in); border: 1px solid var(--border); border-radius: 999px;
    color: var(--text2); padding: 3px 10px; font-size: 10.5px; cursor: pointer;
    box-shadow: 0 2px 8px rgba(0,0,0,0.18);
  }
  .m-acts button:hover, .m-acts button.on { color: var(--accent); border-color: color-mix(in srgb, var(--accent) 45%, transparent); }
  /* Referenced images, under the text they came with. */
  .shots { display: flex; flex-direction: column; gap: 6px; margin-top: 6px; border-radius: 16px; overflow: hidden; }
  .sysline {
    align-self: center; display: flex; align-items: baseline; gap: 7px;
    max-width: min(92%, 620px); padding: 4px 13px; border-radius: 999px;
    color: var(--text3); background: color-mix(in srgb, var(--bubble-in) 88%, transparent);
    border: 1px solid var(--border2); box-shadow: 0 1px 2px rgba(0,0,0,0.06);
    font-size: 10.5px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;
    -webkit-backdrop-filter: blur(8px); backdrop-filter: blur(8px);
  }
  .sysline .sys-who { font-weight: 650; color: var(--text2); }
  /* Delivery receipt: the agent's prompt hook echoed our line back. */
  .ok-chip { display: inline-flex; align-items: center; gap: 3px; margin-left: auto; color: var(--status-ok); font-size: 9.5px; letter-spacing: 0.1px; }

  /* The input half of a turn — what the agent was asked. */
  .prompt { align-self: flex-start; max-width: min(76%, 760px); border-left: 2px solid var(--border); padding-left: 9px; margin: 1px 6px; }
  .p-head { display: flex; align-items: baseline; gap: 7px; font-size: 10.5px; color: var(--text3); margin-bottom: 2px; }
  .p-head .p-who { font-family: ui-monospace, Menlo, monospace; font-weight: 600; color: var(--text2); }
  .p-tag { text-transform: uppercase; letter-spacing: 0.8px; font-size: 9px; color: var(--text3); border: 1px solid var(--border); border-radius: 4px; padding: 0 4px; }
  .p-body { font-size: 12.5px; color: var(--text2); white-space: pre-wrap; word-break: break-word; overflow-wrap: anywhere; max-height: 7.5em; overflow: hidden; }

  /* A single observed fact: status declaration, lifecycle hook, warning. */
  .note {
    display: flex; align-items: baseline; gap: 8px; width: min(76%, 760px);
    font-family: var(--font-mono); font-size: 10.5px; color: var(--text3);
    padding: 1px 8px; max-width: 100%;
  }
  .note .n-who { flex: none; font-weight: 600; }
  .note .n-text { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .note .n-ts { flex: none; margin-left: auto; opacity: 0.6; }
  .note.warn { color: var(--status-warn); }
  .note.warn :global(svg) { flex: none; align-self: center; }

  /* Collapsible run of tool calls between two replies. */
  .steps { display: flex; flex-direction: column; width: min(76%, 760px); max-width: 100%; }
  .s-head {
    display: flex; align-items: center; gap: 7px; width: 100%; text-align: left;
    background: var(--surface); border: 1px solid var(--border2); border-radius: 9px;
    padding: 5px 10px; cursor: pointer; color: var(--text3);
    font-family: ui-monospace, Menlo, monospace; font-size: 11px;
    transition: border-color 160ms, color 160ms;
  }
  .s-head:hover { border-color: var(--input-border); color: var(--text2); }
  .steps.open .s-head { border-bottom-left-radius: 0; border-bottom-right-radius: 0; }
  .chev { display: inline-flex; flex: none; transition: transform 150ms; }
  .chev.open { transform: rotate(90deg); }
  .s-live { flex: none; width: 7px; height: 7px; border-radius: 50%; background: var(--status-ok); animation: s-pulse 1.4s ease-in-out infinite; }
  @keyframes s-pulse { 0%, 100% { opacity: 1; } 50% { opacity: 0.35; } }
  @media (prefers-reduced-motion: reduce) { .s-live { animation: none; } }
  .s-who { flex: none; font-weight: 600; color: var(--text2); }
  .s-count { flex: none; }
  .s-peek { min-width: 0; opacity: 0.7; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .s-body {
    display: flex; flex-direction: column; gap: 2px;
    margin-left: 11px; padding: 5px 0 3px 11px; border-left: 1px solid var(--border);
  }
  .s-all {
    align-self: flex-start; background: none; border: none; color: var(--text3);
    font-size: 10.5px; padding: 0 0 3px; cursor: pointer; font-family: ui-monospace, Menlo, monospace;
  }
  .s-all:hover { color: var(--accent); }
  .step { display: flex; align-items: baseline; gap: 8px; font-family: ui-monospace, Menlo, monospace; font-size: 11px; color: var(--text3); }
  /* The tool name: the part the eye scans down a column. */
  .tname { flex: none; color: var(--accent); font-weight: 650; }
  .step .tname { min-width: 6.5em; max-width: 12em; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .hub-root.compact .step .tname { min-width: 0; }
  .s-peek .tname { min-width: 0; margin-right: 5px; }
  .step .st-text { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; color: var(--text2); }
  .step .st-ts { flex: none; margin-left: auto; opacity: 0.55; }
  .empty { color: var(--text3); font-size: 12.5px; text-align: center; margin: auto; padding: 0 24px; line-height: 1.6; }

  .composer {
    display: flex; align-items: flex-end; gap: 9px; padding: 10px clamp(12px, 3vw, 28px);
    border-top: 1px solid var(--border2); background: color-mix(in srgb, var(--bg) 92%, transparent);
    box-shadow: 0 -8px 28px rgba(0,0,0,0.05);
    -webkit-backdrop-filter: blur(14px); backdrop-filter: blur(14px);
  }
  .compose-shell {
    flex: 1; min-width: 0; position: relative;
    padding: 6px 52px 6px 10px; border: 1px solid var(--input-border); border-radius: 23px;
    background: var(--bubble-in); box-shadow: 0 1px 3px rgba(0,0,0,0.10);
    transition: border-color 150ms ease, box-shadow 150ms ease;
  }
  .compose-shell:focus-within { border-color: color-mix(in srgb, var(--accent) 55%, transparent); box-shadow: 0 2px 8px rgba(0,0,0,0.12); }
  /* Recipient control: who this message goes to, with a menu that opens
     UPWARD so the on-screen keyboard never covers it. */
  /* Pinned to the capsule's top-left; the textarea's first line is indented
     past it and later lines reclaim the full width beneath. */
  .to-wrap { position: absolute; top: 7px; left: 8px; z-index: 2; width: max-content; }
  .to-chip {
    display: flex; align-items: center; gap: 4px; height: 26px;
    background: var(--accent-bg); color: var(--accent); border: 1px solid transparent;
    border-radius: 999px; padding: 0 9px; font-size: 11px; font-weight: 650;
    cursor: pointer; max-width: min(34vw, 220px);
  }
  /* Broadcast and room-note are NOT the default state, so they do not wear the
     accent: one interrupts everyone, the other reaches nobody live. */
  .to-chip.all { background: var(--surface); color: var(--status-warn); border-color: var(--status-warn); }
  .to-chip.note { background: var(--surface); color: var(--text2); border-color: var(--border); }
  .to-label { font-weight: 500; opacity: 0.7; font-size: 10.5px; text-transform: uppercase; letter-spacing: 0.5px; }
  .to-name { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .to-sep { height: 1px; background: var(--border2); margin: 4px 6px; }
  .to-opt { display: flex; flex-direction: column; gap: 1px; min-width: 0; }
  .to-opt small { font-size: 10px; opacity: 0.65; }
  .note-dot { border: 1px dashed var(--text3); background: none; }
  .to-menu {
    position: absolute; bottom: calc(100% + 6px); left: 0; z-index: 12;
    min-width: 168px; max-height: 46vh; overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: 11px;
    box-shadow: 0 12px 34px rgba(0,0,0,0.45); padding: 5px; display: flex; flex-direction: column; gap: 2px;
  }
  .to-menu button {
    display: flex; align-items: center; gap: 7px; min-height: 38px; width: 100%; text-align: left;
    background: none; border: none; border-radius: 8px; color: var(--text2);
    padding: 7px 10px; font-size: 13px; cursor: pointer; font-family: ui-monospace, Menlo, monospace;
  }
  .to-menu button:hover { background: var(--surface2); color: var(--text); }
  .to-menu button.sel { color: var(--accent); background: var(--accent-bg); }
  .all-dot { border: 1px solid var(--text3); background: none; }
  .c-input {
    display: block; width: 100%; min-height: 28px; max-height: 34vh;
    padding: 5px 0 4px; background: transparent; border: none; outline: none;
    color: var(--text); font-size: 13.5px; line-height: 1.5;
    resize: none; overflow-y: auto;
  }
  .c-input::placeholder { color: var(--text3); opacity: 0.82; }
  .send-btn {
    position: absolute; right: 6px; bottom: 5px;
    width: 36px; height: 36px; display: grid; place-items: center;
    padding: 0; border: none; border-radius: 50%; cursor: pointer;
    background: var(--accent); color: white;
    box-shadow: 0 3px 10px color-mix(in srgb, var(--accent) 30%, transparent);
    transition: transform 130ms ease, box-shadow 130ms ease, opacity 130ms ease;
  }
  .send-btn:hover:not(:disabled) { transform: translateY(-1px); box-shadow: 0 5px 14px color-mix(in srgb, var(--accent) 38%, transparent); }
  .send-btn:active:not(:disabled) { transform: scale(0.96); }
  .send-btn:disabled { opacity: 0.35; cursor: default; box-shadow: none; }

  /* Empty room: start from a preset — one agent, or a team. */
  .start { margin: auto; display: flex; flex-direction: column; gap: 8px; width: min(420px, 100%); }
  .start-h { font-size: 12.5px; color: var(--text2); text-align: center; margin-bottom: 2px; }
  .start-list { display: flex; flex-direction: column; gap: 5px; }
  .start-row {
    display: flex; align-items: center; gap: 8px; min-height: 44px; width: 100%; text-align: left;
    background: var(--surface); border: 1px solid var(--border); border-radius: 11px;
    color: var(--text); padding: 8px 11px; font-size: 13px; cursor: pointer;
  }
  .start-row:hover { border-color: var(--accent); background: var(--accent-bg); }
  .start-row:disabled { opacity: 0.5; }
  .sr-name { font-family: ui-monospace, Menlo, monospace; font-weight: 600; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .sr-backend { font-family: ui-monospace, Menlo, monospace; font-size: 11px; color: var(--text3); margin-left: auto; }
  .sr-cap { font-size: 9px; color: var(--accent); border: 1px solid var(--accent); border-radius: 4px; padding: 0 3px; opacity: 0.75; }

  .drawer { display: flex; flex-direction: column; min-width: 0; min-height: 0; background: #000; border-left: 1px solid var(--border); }
  .drawer-head { display: flex; align-items: center; gap: 8px; padding: 6px 10px; background: var(--bg2); border-bottom: 1px solid var(--border); }
  .win-list { display: flex; gap: 5px; overflow-x: auto; scrollbar-width: none; }
  .win-list::-webkit-scrollbar { display: none; }
  .win-pill { display: flex; align-items: center; gap: 5px; flex: none; background: var(--surface); border: 1px solid var(--border); border-radius: 7px; color: var(--text2); padding: 4px 9px; font-family: ui-monospace, Menlo, monospace; font-size: 11.5px; cursor: pointer; }
  .win-pill.cur { border-color: var(--accent); color: var(--accent); background: var(--accent-bg); }
  .direct-tag { font-size: 9px; color: var(--text3); border: 1px solid var(--border); border-radius: 4px; padding: 0 4px; margin-left: 3px; }
  .term-body { flex: 1; min-width: 0; min-height: 0; position: relative; display: flex; flex-direction: column; }

  .statusline { display: flex; align-items: center; height: 25px; background: var(--bg3); border-top: 1px solid var(--border); font-family: ui-monospace, Menlo, monospace; font-size: 11.5px; color: var(--text2); user-select: none; flex: none; }
  .statusline .sess { background: var(--accent); color: #06232b; font-weight: 700; padding: 0 10px; height: 100%; display: flex; align-items: center; }
  .wlist { display: flex; height: 100%; overflow-x: auto; scrollbar-width: none; }
  .statusline .w { display: flex; align-items: center; padding: 0 9px; color: var(--text3); background: none; border: none; cursor: pointer; font: inherit; transition: color 160ms; }
  .statusline .w:hover { color: var(--text); }
  .statusline .w.cur { background: var(--surface2); color: var(--accent); }
  .statusline .right { margin-left: auto; padding: 0 12px; color: var(--text3); white-space: nowrap; }

  .dlg-backdrop { position: fixed; inset: 0; z-index: 30; background: rgba(0,0,0,0.45); }
  .dlg {
    position: fixed; z-index: 31; left: 50%; top: 50%; transform: translate(-50%, -50%);
    width: min(440px, calc(100vw / var(--ui-zoom, 1) - 32px)); max-height: calc(100vh / var(--ui-zoom, 1) - 48px); overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: 14px;
    box-shadow: 0 18px 60px rgba(0,0,0,0.5); padding: 18px; display: flex; flex-direction: column; gap: 10px;
  }
  .dlg h2 { margin: 0 0 4px; font-size: 15px; }  /* Phone: dialogs become bottom sheets — reachable with a thumb, and they
     never fight the on-screen keyboard for the middle of the screen. */
  .dlg.sheet {
    left: 0; top: auto; bottom: 0; transform: none;
    width: 100%; max-width: none; max-height: 82vh;
    border-radius: 16px 16px 0 0; border-left: none; border-right: none; border-bottom: none;
    padding: 16px 14px calc(16px + env(safe-area-inset-bottom));
  }
  .dlg.sheet .dlg-agents { max-height: 46vh; overflow-y: auto; }
  .dlg.sheet .agent-pick, .dlg.sheet input, .dlg.sheet .dlg-actions button { min-height: 44px; }
  .dlg input { background: var(--input-bg); border: 1px solid var(--input-border); border-radius: 9px; color: var(--text); padding: 8px 12px; font-size: 13px; outline: none; }
  .dlg input:focus { border-color: var(--accent); }
  .dlg-h { font-family: ui-monospace, Menlo, monospace; font-size: 10px; text-transform: uppercase; letter-spacing: 1.4px; color: var(--text3); margin-top: 4px; }
  .dlg-agents { display: flex; flex-direction: column; gap: 5px; }
  .agent-pick { display: flex; align-items: center; gap: 8px; background: var(--surface); border: 1px solid var(--border); border-radius: 9px; color: var(--text2); padding: 8px 11px; font-size: 12.5px; cursor: pointer; text-align: left; }
  .agent-pick.sel { border-color: var(--accent); color: var(--text); background: var(--accent-bg); }
  .agent-pick :global(svg) { margin-left: auto; color: var(--accent); }
  .dlg-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 6px; }
</style>
