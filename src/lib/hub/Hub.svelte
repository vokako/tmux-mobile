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
  import { stateDotColor, mergeMessages, statuslineWindows, backendColor, feedBlocks, systemLine, pickLead, addressed, fmtElapsed, unreadSenders, splitImages, stoppedAgents } from './hub.ts';
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
        scrollFeed();
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
        scrollFeed();
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

  function scrollFeed() {
    requestAnimationFrame(() => {
      if (!feedEl) return;
      feedEl.scrollTop = feedEl.scrollHeight;
      // Scrolled to the newest message means the user has seen it.
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

  async function send() {
    let text = composerText.trim();
    if (!text || !selected) return;
    // The recipient makes "talk to THIS agent" the default rather than a
    // gesture: addressed() prefixes @name unless the user @-addressed someone
    // by hand, and an empty recipient posts to the room.
    text = addressed(text, recipient);
    composerText = '';
    try {
      await hubPost(selected, text);
      await loadFeed();
    } catch (e) { console.warn('hub post failed', e); }
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

  // Stopping or restarting an agent kills a process that may be mid-task, and
  // on a phone the button is a thumb away from the chip you meant to tap — so
  // it asks first, naming what it will do.
  let pendingAct = $state(null);   // { kind: 'stop' | 'restart', name }
  let acting = $state(false);
  const askAction = (kind, name) => { pendingAct = { kind, name }; };

  async function runAction() {
    if (!pendingAct || acting) return;
    const { kind, name } = pendingAct;
    acting = true;
    try {
      if (kind === 'stop') await hubAgentStop(selected, name);
      else await hubAgentRestart(selected, name);
      await Promise.all([reload(), loadAgents(), loadFeed()]);
    } catch (e) {
      console.warn(`${kind} failed`, e);
    } finally {
      acting = false;
      pendingAct = null;
    }
  }

  // Live pushes + polling while visible.
  const onPush = (m) => {
    if (!selected || m?.room !== room(selected)) return;
    feed = mergeMessages(feed, [m]);
    lastTs = Math.max(lastTs, m.ts ?? 0);
    scrollFeed();
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
            <button class="acard" class:sel={recipient === a.name} onclick={() => setRecipient(a.name)}>
              <div class="a-top">
                <span class="ava" style:background={backendColor(a.agent)}>{a.name.slice(0, 1).toUpperCase()}</span>
                {a.name}
                <!-- It replied and you have not looked yet. -->
                {#if unread.has(a.name)}<span class="unread" title={t('hubUnread')}></span>{/if}
                {#if recipient === a.name}<span class="lead-tag">{t('hubLead')}</span>{/if}
                <span class="a-peek" role="button" tabindex="-1" title={t('hubWatch')}
                  onclick={(e) => { e.stopPropagation(); openDrawer(a); }}
                  onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); openDrawer(a); } }}>
                  <Icon name="terminal" size={12} />
                </span>
                <!-- Life controls, on the agent you are talking to: an agent's
                     window IS its life, and stopping keeps the declaration so it
                     can come back to the same conversation. -->
                {#if recipient === a.name}
                  <span class="a-act" role="button" tabindex="-1" title={t('hubRestart')}
                    onclick={(e) => { e.stopPropagation(); askAction('restart', a.name); }}
                    onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); askAction('restart', a.name); } }}>
                    <Icon name="refresh" size={12} />
                  </span>
                  <span class="a-act danger" role="button" tabindex="-1" title={t('hubStop')}
                    onclick={(e) => { e.stopPropagation(); askAction('stop', a.name); }}
                    onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); askAction('stop', a.name); } }}>
                    <Icon name="stop" size={12} />
                  </span>
                {/if}
              </div>
              <div class="a-state">
                <span class="st" class:live={a.state === 'running'} style:background={stateDotColor(a.state)}></span>
                <span class="s-word">{stateLabel(a.state)}</span>
                <!-- How long this state has held: running 2m14s. -->
                {#if a.since}<span class="s-age">{fmtElapsed(a.since, tick)}</span>{/if}
              </div>
              {#if a.detail && !compact}<div class="a-note">{a.detail}</div>{/if}
            </button>
          {/each}
          <!-- Stopped agents: declared by the project, no window right now.
               Starting one resumes its conversation, so it stays on the roster
               instead of vanishing from the room it belongs to. -->
          {#each stopped as name (name)}
            <button class="acard off" onclick={() => askAction('restart', name)} title={t('hubStartAgain')}>
              <div class="a-top">
                <span class="ava dim">{name.slice(0, 1).toUpperCase()}</span>
                {name}
              </div>
              <div class="a-state">
                <span class="st" style:background={stateDotColor('idle')}></span>
                <span class="s-word">{t('hubStopped')}</span>
                <Icon name="refresh" size={11} />
              </div>
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

      <div class="feed" bind:this={feedEl}>
        {#each blocks as b, i (blockKey(b, i))}
          {#if b.type === 'msg'}
            {@const m = b.msg}
            {@const sys = systemLine(m.body)}
            {@const parts = splitImages(m.body)}
            {#if sys !== null}
              <div class="sysline"><span class="sys-who">{m.from}</span>{sys}</div>
            {:else}
              <div class="msg" class:me={m.from === 'human'}>
                <div class="m-head">
                  <span class="who">{m.from === 'human' ? t('hubYou') : m.from}</span>
                  <span>{fmtTime(m.ts)}</span>
                  <!-- The agent's own prompt hook echoed this line back, so it
                       reached the CLI's input — not merely typed at the pane. -->
                  {#if b.delivered}
                    <span class="ok-chip" title={t('hubDeliveredHint')}><Icon name="check" size={9} />{t('hubDelivered')}</span>
                  {/if}
                </div>
                <!-- Markdown-rendered (agents write md); renderMarkdown
                     escapes HTML first, so raw tags stay inert text. Image
                     references are pulled out and resolved separately — a local
                     path is not a URL a webview can load. -->
                {#if parts.text}
                  <div class="bubble md">{@html renderMarkdown(parts.text)}</div>
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
                  <span class="s-peek"><span class="tname">{last?.tool ?? ''}</span> {last?.text ?? ''}</span>
                {/if}
              </button>
              {#if open}
                <div class="s-body">
                  {#each b.events as e, j (`${e.ts}-${j}`)}
                    <div class="step">
                      <!-- The tool NAME is the scannable half: fixed column,
                           accent colour. Its argument is secondary. -->
                      <span class="tname">{e.tool ?? ''}</span>
                      <span class="st-text">{e.text}</span>
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

      <div class="composer">
        <!-- WHO this goes to, always visible: the lead by default, one tap to
             retarget or to broadcast. No @ typing required. -->
        {#if managedAgents.length}
          <div class="to-wrap">
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
        <input placeholder={recipient === ALL_TARGET ? t('hubComposerAll') : recipient ? t('hubComposerDm').replace('{name}', recipient) : t('hubComposerRoom')}
          bind:value={composerText} onkeydown={(e) => e.key === 'Enter' && send()} />
        <button class="send-btn" onclick={send} title={t('hubSend')}>
          {#if compact}<Icon name="send" size={16} />{:else}{t('hubSend')}{/if}
        </button>
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
    <!-- ── Stop / restart one agent ── -->
    <div class="dlg-backdrop" onclick={() => pendingAct = null} role="presentation"></div>
    <div class="dlg" class:sheet={compact}>
      <h2>{(pendingAct.kind === 'stop' ? t('hubStopTitle') : t('hubRestartTitle')).replace('{name}', pendingAct.name)}</h2>
      <p class="dlg-note">{pendingAct.kind === 'stop' ? t('hubStopNote') : t('hubRestartNote')}</p>
      <div class="dlg-actions">
        <button class="chip-btn" onclick={() => pendingAct = null}>{t('cancel')}</button>
        <button class="chip-btn primary" class:danger={pendingAct.kind === 'stop'} disabled={acting} onclick={runAction}>
          {acting ? '…' : pendingAct.kind === 'stop' ? t('hubStop') : t('hubRestart')}
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
  .hub-root { height: 100%; display: flex; flex-direction: column; min-height: 0; background: var(--bg); position: relative; }
  .cols { flex: 1; display: grid; grid-template-columns: var(--sidebar-w) minmax(0, 1fr); min-height: 0; }
  .hub-root.compact .cols { grid-template-columns: minmax(0, 1fr); }
  /* Phone shape: tighter gutters, thumb-sized controls, no horizontal
     overflow. The page head wraps instead of pushing the chips off-screen. */
  .hub-root.compact .page-head { flex-wrap: wrap; row-gap: 6px; padding: 8px 12px; }
  .hub-root.compact .page-head h1 { font-size: 15px; }
  .hub-root.compact .feed { padding: 12px 12px; gap: 10px; }
  .hub-root.compact .msg, .hub-root.compact .prompt { max-width: 92%; }
  .hub-root.compact .composer { padding: 8px 10px calc(8px + env(safe-area-inset-bottom)); gap: 6px; }
  .hub-root.compact .composer input { min-height: 40px; font-size: 14px; }
  .hub-root.compact .send-btn { min-width: 44px; min-height: 40px; padding: 8px 12px; }
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

  .cards { display: flex; gap: 8px; padding: 10px 16px; overflow-x: auto; border-bottom: 1px solid var(--border2); }
  .acard { flex: none; width: 158px; background: var(--surface); border: 1px solid var(--border); border-radius: 11px; padding: 9px 11px; cursor: pointer; text-align: left; transition: border-color 160ms; }
  .acard:hover { border-color: var(--input-border); }
  .acard.sel { border-color: var(--accent); background: var(--accent-bg); }
  .acard.add { width: auto; display: flex; align-items: center; gap: 6px; color: var(--text3); font-size: 12px; }
  /* A stopped agent: present, not running. */
  .acard.off { opacity: 0.6; }
  .acard.off:hover { opacity: 1; border-color: var(--accent); }
  .ava.dim { background: var(--surface2) !important; color: var(--text3); }
  .acard.add:hover { color: var(--accent); }
  .lead-tag { font-size: 8.5px; letter-spacing: 0.6px; text-transform: uppercase; color: var(--accent); border: 1px solid var(--accent); border-radius: 4px; padding: 0 3px; }
  /* Phone: the roster is a scrollable chip row, not a wall of cards. */
  .cards.chips { gap: 6px; padding: 8px 12px; }
  .cards.chips .acard { width: auto; display: flex; align-items: center; gap: 6px; min-height: 40px; border-radius: 999px; padding: 6px 11px; }
  .cards.chips .a-top { gap: 5px; }
  .cards.chips .a-state { margin-top: 0; }
  .cards.chips .a-state :global(span) { margin: 0; }
  .a-top { display: flex; align-items: center; gap: 6px; font-family: ui-monospace, Menlo, monospace; font-weight: 600; font-size: 12.5px; color: var(--text); }
  .a-peek { margin-left: auto; display: grid; place-items: center; width: 22px; height: 20px; border-radius: 6px; color: var(--text3); }
  .a-peek:hover { color: var(--accent); background: var(--surface2); }
  /* Life controls for the agent you are addressing. */
  .a-act { display: grid; place-items: center; width: 22px; height: 20px; border-radius: 6px; color: var(--text3); flex: none; }
  .a-act:hover { color: var(--accent); background: var(--surface2); }
  .a-act.danger:hover { color: var(--status-danger); }
  .dlg-note { margin: 0; font-size: 12.5px; color: var(--text2); line-height: 1.55; }
  .chip-btn.danger { color: var(--status-danger); border-color: var(--status-danger); }
  .a-state { font-family: ui-monospace, Menlo, monospace; font-size: 10.5px; color: var(--text2); margin-top: 5px; display: flex; align-items: center; gap: 5px; }
  .s-word { text-transform: lowercase; }
  .s-age { color: var(--text3); font-variant-numeric: tabular-nums; }
  .st { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  /* Running is the only state that moves. */
  .st.live { animation: s-pulse 1.4s ease-in-out infinite; }
  /* An agent replied and you have not looked yet. */
  .unread { width: 7px; height: 7px; border-radius: 50%; background: var(--status-danger); flex: none; }
  .a-note { font-size: 11px; color: var(--text3); margin-top: 3px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .feed { flex: 1; overflow-y: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 12px; }
  .msg { max-width: 84%; }
  .msg.me { align-self: flex-end; }
  .m-head { font-size: 11px; color: var(--text3); margin-bottom: 3px; display: flex; gap: 7px; align-items: baseline; }
  .m-head .who { color: var(--text2); font-family: ui-monospace, Menlo, monospace; font-weight: 600; font-size: 11.5px; }
  .bubble { background: var(--surface); border: 1px solid var(--border2); border-radius: 12px; padding: 8px 12px; font-size: 13px; color: var(--text); word-break: break-word; overflow-wrap: anywhere; }
  .msg.me .bubble { background: var(--accent-bg); border-color: transparent; }
  /* Referenced images, under the text they came with. */
  .shots { display: flex; flex-direction: column; gap: 6px; margin-top: 6px; }
  .sysline { align-self: center; display: flex; align-items: baseline; gap: 7px; font-size: 11px; color: var(--text3); background: var(--surface); border-radius: 999px; padding: 3px 13px; max-width: 92%; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .sysline .sys-who { font-family: ui-monospace, Menlo, monospace; font-weight: 600; color: var(--text2); }
  /* Delivery receipt: the agent's prompt hook echoed our line back. */
  .ok-chip { display: inline-flex; align-items: center; gap: 3px; margin-left: auto; color: var(--status-ok); font-size: 10px; letter-spacing: 0.2px; }

  /* The input half of a turn — what the agent was asked. */
  .prompt { align-self: flex-start; max-width: 84%; border-left: 2px solid var(--border); padding-left: 9px; }
  .p-head { display: flex; align-items: baseline; gap: 7px; font-size: 10.5px; color: var(--text3); margin-bottom: 2px; }
  .p-head .p-who { font-family: ui-monospace, Menlo, monospace; font-weight: 600; color: var(--text2); }
  .p-tag { text-transform: uppercase; letter-spacing: 0.8px; font-size: 9px; color: var(--text3); border: 1px solid var(--border); border-radius: 4px; padding: 0 4px; }
  .p-body { font-size: 12.5px; color: var(--text2); white-space: pre-wrap; word-break: break-word; overflow-wrap: anywhere; max-height: 7.5em; overflow: hidden; }

  /* A single observed fact: status declaration, lifecycle hook, warning. */
  .note {
    display: flex; align-items: baseline; gap: 8px;
    font-family: ui-monospace, Menlo, monospace; font-size: 11px; color: var(--text3);
    padding: 0 4px; max-width: 100%;
  }
  .note .n-who { flex: none; font-weight: 600; }
  .note .n-text { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .note .n-ts { flex: none; margin-left: auto; opacity: 0.6; }
  .note.warn { color: var(--status-warn); }
  .note.warn :global(svg) { flex: none; align-self: center; }

  /* Collapsible run of tool calls between two replies. */
  .steps { display: flex; flex-direction: column; }
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
  .step { display: flex; align-items: baseline; gap: 8px; font-family: ui-monospace, Menlo, monospace; font-size: 11px; color: var(--text3); }
  /* The tool name: the part the eye scans down a column. */
  .tname { flex: none; color: var(--accent); font-weight: 650; }
  .step .tname { min-width: 6.5em; }
  .s-peek .tname { min-width: 0; }
  .step .st-text { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; color: var(--text2); }
  .step .st-ts { flex: none; margin-left: auto; opacity: 0.55; }
  .empty { color: var(--text3); font-size: 12.5px; text-align: center; margin: auto; padding: 0 24px; line-height: 1.6; }

  .composer { display: flex; align-items: center; gap: 8px; padding: 10px 16px; border-top: 1px solid var(--border); }
  /* Recipient control: who this message goes to, with a menu that opens
     UPWARD so the on-screen keyboard never covers it. */
  .to-wrap { position: relative; flex: none; }
  .to-chip { display: flex; align-items: center; gap: 5px; min-height: 34px; background: var(--accent-bg); color: var(--accent); border: 1px solid transparent; border-radius: 9px; padding: 6px 9px; font-size: 12px; font-weight: 600; cursor: pointer; font-family: ui-monospace, Menlo, monospace; max-width: 42vw; }
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
  .composer input { flex: 1; min-width: 0; background: var(--input-bg); border: 1px solid var(--input-border); border-radius: 10px; color: var(--text); padding: 8px 12px; font-size: 13px; outline: none; transition: border-color 160ms; }
  .composer input:focus { border-color: var(--accent); }
  .send-btn { background: var(--accent-bg); color: var(--accent); border: none; border-radius: 10px; padding: 8px 16px; cursor: pointer; font-weight: 600; font-size: 13px; display: grid; place-items: center; }

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
