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
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import {
    projectList, projectUp, projectCreate, listSessionsWithPanes,
    hubPost, hubLog, hubAgents, hubSpawn, hubActivity, registryList,
    addTeamMessageListener, removeTeamMessageListener,
  } from '../core/ws.ts';
  import { sortRows, shortPath } from '../projects/projects.ts';
  import { stateDotColor, mergeMessages, statuslineWindows, backendColor, feedBlocks, systemLine, pickLead, addressed } from './hub.ts';
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
  let composerText = $state('');
  // Who the composer addresses. '' = the whole room. Defaults to the project's
  // lead (pickLead), so talking to your lead agent needs no @ ceremony; the
  // user can retarget or broadcast at any time.
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
      // without the user choosing one.
      if (recipient && !agents.some((a) => a.managed && a.name === recipient)) recipient = '';
      if (!recipient) recipient = pickLead(agents, registry, hubPrefs.lead(selected));
    } catch { agents = []; }
  }

  function scrollFeed() {
    requestAnimationFrame(() => { if (feedEl) feedEl.scrollTop = feedEl.scrollHeight; });
  }

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
  let spawnOpen = $state(false);
  let spawnAgent = $state('');
  let spawnBrief = $state('');
  // Preset start: pick one agent (tap) or several (a team) for an empty room.
  let startOpen = $state(false);
  let startPick = $state([]);
  let startBrief = $state('');
  let starting = $state(false);
  async function doSpawn() {
    if (!spawnAgent || !selected) return;
    const brief = spawnBrief.trim();
    spawnOpen = false;
    spawnBrief = '';
    const name = spawnAgent;
    spawnAgent = '';
    try {
      await hubSpawn(selected, name, brief);
      await Promise.all([reload(), loadAgents(), loadFeed()]);
      // First agent in an empty room becomes the one you are talking to.
      if (!recipient) setRecipient(name);
    } catch (e) { console.warn('spawn failed', e); }
  }

  /** Start a conversation from a preset: one agent, or several at once (a
   * team). Each is an existing `hub_spawn`, run in order so the roster appears
   * in the order it was picked; the lead is the first that can hire, else the
   * first picked — the same rule pickLead applies to a room already running. */
  async function startWith(names, brief = '') {
    if (!selected || !names.length || starting) return;
    starting = true;
    startOpen = false;
    try {
      for (const name of names) {
        try { await hubSpawn(selected, name, brief); }
        catch (e) { console.warn('spawn failed', name, e); }
      }
      await Promise.all([reload(), loadAgents(), loadFeed()]);
      const lead = names.find((n) => registry.find((r) => r.name === n)?.can_hire) ?? names[0];
      setRecipient(lead);
    } finally {
      starting = false;
      startPick = [];
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

  const winsForStatusline = $derived(statuslineWindows(agents, termTarget));
  const fmtTime = (ts) => {
    const d = new Date(ts);
    return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  };
</script>

<div class="hub-root" class:compact class:drawer-open={termOpen && !compact}>
  <div class="cols">
    {#if !compact}
    <!-- ── Sidebar: projects only ─────────── -->
    <aside class="sidebar">
      <SideHandle />
      <div class="side-scroll">
        <div class="side-h">{t('hubProjects')}</div>
        {#each rows as row (row.project.id)}
          <button class="side-row" class:open={row.project.session === selected} onclick={() => selectProject(row.project.session)}>
            <span class="dot" class:off={!row.live}></span>
            <span class="p-name">{row.project.name}</span>
          </button>
        {/each}
        <button class="side-row add" onclick={() => { createOpen = true; }}>
          <Icon name="plus" size={13} />{t('projectNew')}
        </button>
      </div>
    </aside>
    {/if}

    <!-- ── Main: the conversation ─────────── -->
    <main class="mid">
      {#if compact}
        <div class="proj-chips">
          {#each rows as row (row.project.id)}
            <button class="pchip" class:sel={row.project.session === selected} onclick={() => selectProject(row.project.session)}>
              <span class="dot" class:off={!row.live}></span>{row.project.name}
            </button>
          {/each}
          <button class="pchip" onclick={() => { createOpen = true; }} title={t('projectNew')}><Icon name="plus" size={12} /></button>
        </div>
      {/if}

      <div class="page-head">
        <h1>{selectedRow?.project.name ?? ''}</h1>
        {#if !compact}<span class="path">{shortPath(selectedRow?.project.path ?? '')}</span>{/if}
        <span class="spacer"></span>
        {#if selected && !liveSelected}
          <button class="chip-btn" onclick={bringUp}>{t('projectOpen')}</button>
        {/if}
        {#if liveSelected}
          <button class="chip-btn" onclick={() => { spawnOpen = !spawnOpen; }}><Icon name="plus" size={12} /> {t('hubSpawn')}</button>
        {/if}
        <!-- Chat detail, reachable where the feed is: cycles chat → status →
             tools. The full control with labels lives in Settings. -->
        <button class="chip-btn lvl" title={t('hubFeedLevel')} onclick={() => hubPrefs.cycleFeedLevel()}>
          {hubPrefs.feedLevel === 'chat' ? t('hubFeedChat') : hubPrefs.feedLevel === 'status' ? t('hubFeedStatus') : t('hubFeedTools')}
        </button>
        <!-- THE terminal affordance: a button, not a permanent pane. -->
        <button class="chip-btn term-toggle" class:on={termOpen} title={t('hubTerminal')} onclick={() => termOpen && !compact ? termOpen = false : openDrawer()}>
          <Icon name="terminal" size={14} />{#if !compact}<span>{t('hubTerminal')}</span>{/if}
        </button>
      </div>

      {#if spawnOpen}
        <div class="spawn-form">
          <select bind:value={spawnAgent}>
            <option value="" disabled selected>{t('hubPickAgent')}</option>
            {#each registry as r (r.name)}<option value={r.name}>{r.name} · {r.backend}</option>{/each}
          </select>
          <input placeholder={t('hubBrief')} bind:value={spawnBrief} onkeydown={(e) => e.key === 'Enter' && doSpawn()} />
          <button class="chip-btn" disabled={!spawnAgent} onclick={doSpawn}>{t('hubSpawn')}</button>
        </div>
      {/if}

      {#if managedAgents.length}
        <!-- The roster. Tapping an agent makes it the recipient (and this
             project's lead) — the phone gets chips, the desktop gets cards. -->
        <div class="cards" class:chips={compact}>
          {#each managedAgents as a (a.window)}
            <button class="acard" class:sel={recipient === a.name} onclick={() => setRecipient(a.name)}>
              <div class="a-top">
                <span class="ava" style:background={backendColor(a.agent)}>{a.name.slice(0, 1).toUpperCase()}</span>
                {a.name}
                {#if recipient === a.name}<span class="lead-tag">{t('hubLead')}</span>{/if}
                <span class="a-peek" role="button" tabindex="-1" title={t('hubWatch')}
                  onclick={(e) => { e.stopPropagation(); openDrawer(a); }}
                  onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); openDrawer(a); } }}>
                  <Icon name="terminal" size={12} />
                </span>
              </div>
              <div class="a-state">
                <span class="st" style:background={stateDotColor(a.state)}></span>{a.state}
              </div>
              {#if a.detail && !compact}<div class="a-note">{a.detail}</div>{/if}
            </button>
          {/each}
          <!-- Ad hoc: add an agent to a conversation already in progress. -->
          {#if liveSelected}
            <button class="acard add" onclick={() => { spawnOpen = true; }} title={t('hubSpawn')}>
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
                     escapes HTML first, so raw tags stay inert text. -->
                <div class="bubble md">{@html renderMarkdown(m.body)}</div>
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
                {#if !open}<span class="s-peek">{b.events[b.events.length - 1]?.text ?? ''}</span>{/if}
              </button>
              {#if open}
                <div class="s-body">
                  {#each b.events as e, j (`${e.ts}-${j}`)}
                    <div class="step"><span class="st-text">{e.text}</span><span class="st-ts">{fmtTime(e.ts)}</span></div>
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
                  <button class="start-row" disabled={starting} onclick={() => startWith([r.name])}>
                    <span class="ava" style:background={backendColor(r.backend)}>{r.name.slice(0, 1).toUpperCase()}</span>
                    <span class="sr-name">{r.name}</span>
                    <span class="sr-backend">{r.backend}</span>
                    {#if r.can_hire}<span class="sr-cap">{t('agentsCanHire')}</span>{/if}
                  </button>
                {/each}
              </div>
              <button class="chip-btn" disabled={starting} onclick={() => { startPick = []; startOpen = true; }}>
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
            <button class="to-chip" class:all={!recipient} onclick={() => recipientOpen = !recipientOpen}>
              <span class="to-label">{t('hubTo')}</span>
              <span class="to-name">{recipient || t('hubEveryone')}</span>
              <Icon name={recipientOpen ? 'chevron-down' : 'chevron-up'} size={11} />
            </button>
            {#if recipientOpen}
              <div class="to-menu">
                {#each managedAgents as a (a.window)}
                  <button class:sel={recipient === a.name} onclick={() => setRecipient(a.name)}>
                    <span class="st" style:background={stateDotColor(a.state)}></span>{a.name}
                  </button>
                {/each}
                <button class:sel={!recipient} onclick={() => setRecipient('')}>
                  <span class="st all-dot"></span>{t('hubEveryone')}
                </button>
              </div>
            {/if}
          </div>
        {/if}
        <input placeholder={recipient ? t('hubComposerDm').replace('{name}', recipient) : t('hubComposer')}
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

  {#if startOpen}
    <!-- ── Start a team: several agents at once ── -->
    <div class="dlg-backdrop" onclick={() => startOpen = false} role="presentation"></div>
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
        <button class="chip-btn" onclick={() => startOpen = false}>{t('cancel')}</button>
        <button class="chip-btn primary" disabled={!startPick.length || starting}
          onclick={() => startWith(startPick, startBrief.trim())}>
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
  .hub-root.compact .spawn-form { flex-wrap: wrap; padding: 8px 12px; }
  .hub-root.compact .spawn-form select, .hub-root.compact .spawn-form input { min-height: 40px; flex: 1 1 100%; }
  .hub-root.compact .s-head { min-height: 34px; }
  /* Drawer open: the conversation yields but stays present. */
  .hub-root.drawer-open .cols { grid-template-columns: var(--sidebar-w) minmax(280px, 0.8fr) minmax(360px, 1.2fr); }

  .proj-chips {
    display: flex; gap: 6px; padding: 10px 12px 0; overflow-x: auto; flex: none;
    -webkit-overflow-scrolling: touch; scrollbar-width: none;
  }
  .proj-chips::-webkit-scrollbar { display: none; }
  .pchip {
    display: flex; align-items: center; gap: 6px; flex: none;
    background: var(--surface); border: 1px solid var(--border); border-radius: 999px;
    color: var(--text2); padding: 5px 12px; font-size: 12.5px; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .pchip.sel { border-color: var(--accent); color: var(--accent); background: var(--accent-bg); }
  .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--status-ok); flex: none; }
  .dot.off { background: var(--text3); }

  .sidebar { position: relative; background: var(--bg2); border-right: 1px solid var(--border); display: flex; flex-direction: column; min-height: 0; }
  .side-scroll { flex: 1; overflow-y: auto; padding: 8px; }
  .p-name { flex: 1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-weight: 550; }

  .mid { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .path { font-family: ui-monospace, Menlo, monospace; font-size: 11px; color: var(--text3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .spacer { flex: 1; }
  .term-toggle.on { border-color: var(--accent); color: var(--accent); background: var(--accent-bg); }
  .lvl { font-family: ui-monospace, Menlo, monospace; font-size: 11px; }

  .spawn-form { display: flex; gap: 8px; padding: 10px 16px; border-bottom: 1px solid var(--border2); }
  .spawn-form select, .spawn-form input { background: var(--input-bg); border: 1px solid var(--input-border); border-radius: 8px; color: var(--text); padding: 6px 10px; font-size: 12.5px; }
  .spawn-form input { flex: 1; }

  .cards { display: flex; gap: 8px; padding: 10px 16px; overflow-x: auto; border-bottom: 1px solid var(--border2); }
  .acard { flex: none; width: 158px; background: var(--surface); border: 1px solid var(--border); border-radius: 11px; padding: 9px 11px; cursor: pointer; text-align: left; transition: border-color 160ms; }
  .acard:hover { border-color: var(--input-border); }
  .acard.sel { border-color: var(--accent); background: var(--accent-bg); }
  .acard.add { width: auto; display: flex; align-items: center; gap: 6px; color: var(--text3); font-size: 12px; }
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
  .a-state { font-family: ui-monospace, Menlo, monospace; font-size: 10.5px; color: var(--text2); margin-top: 5px; display: flex; align-items: center; gap: 5px; }
  .st { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .a-note { font-size: 11px; color: var(--text3); margin-top: 3px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .feed { flex: 1; overflow-y: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 12px; }
  .msg { max-width: 84%; }
  .msg.me { align-self: flex-end; }
  .m-head { font-size: 11px; color: var(--text3); margin-bottom: 3px; display: flex; gap: 7px; align-items: baseline; }
  .m-head .who { color: var(--text2); font-family: ui-monospace, Menlo, monospace; font-weight: 600; font-size: 11.5px; }
  .bubble { background: var(--surface); border: 1px solid var(--border2); border-radius: 12px; padding: 8px 12px; font-size: 13px; color: var(--text); word-break: break-word; overflow-wrap: anywhere; }
  .msg.me .bubble { background: var(--accent-bg); border-color: transparent; }
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
  .step .st-text { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .step .st-ts { flex: none; margin-left: auto; opacity: 0.55; }
  .empty { color: var(--text3); font-size: 12.5px; text-align: center; margin: auto; padding: 0 24px; line-height: 1.6; }

  .composer { display: flex; align-items: center; gap: 8px; padding: 10px 16px; border-top: 1px solid var(--border); }
  /* Recipient control: who this message goes to, with a menu that opens
     UPWARD so the on-screen keyboard never covers it. */
  .to-wrap { position: relative; flex: none; }
  .to-chip { display: flex; align-items: center; gap: 5px; min-height: 34px; background: var(--accent-bg); color: var(--accent); border: 1px solid transparent; border-radius: 9px; padding: 6px 9px; font-size: 12px; font-weight: 600; cursor: pointer; font-family: ui-monospace, Menlo, monospace; max-width: 42vw; }
  .to-chip.all { background: var(--surface); color: var(--text2); border-color: var(--border); }
  .to-label { font-weight: 500; opacity: 0.7; font-size: 10.5px; text-transform: uppercase; letter-spacing: 0.5px; }
  .to-name { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
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
