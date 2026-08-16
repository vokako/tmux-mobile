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
  import { stateDotColor, mergeMessages, statuslineWindows, backendColor, feedBlocks, systemLine } from './hub.ts';
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
  let dmTarget = $state('');        // managed agent name the composer addresses
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
    dmTarget = '';
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
      if (dmTarget && !agents.some((a) => a.managed && a.name === dmTarget)) dmTarget = '';
    } catch { agents = []; }
  }

  function scrollFeed() {
    requestAnimationFrame(() => { if (feedEl) feedEl.scrollTop = feedEl.scrollHeight; });
  }

  async function send() {
    let text = composerText.trim();
    if (!text || !selected) return;
    // The DM target makes "talk to THIS agent" one tap: auto-address unless
    // the user already @-addressed someone explicitly.
    if (dmTarget && !text.includes('@')) text = `@${dmTarget} ${text}`;
    composerText = '';
    try {
      await hubPost(selected, text);
      await loadFeed();
    } catch (e) { console.warn('hub post failed', e); }
  }

  function toggleDm(a) {
    dmTarget = dmTarget === a.name ? '' : a.name;
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
  async function doSpawn() {
    if (!spawnAgent || !selected) return;
    const brief = spawnBrief.trim();
    spawnOpen = false;
    spawnBrief = '';
    try {
      await hubSpawn(selected, spawnAgent, brief);
      await Promise.all([reload(), loadAgents(), loadFeed()]);
    } catch (e) { console.warn('spawn failed', e); }
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
        <div class="cards">
          {#each managedAgents as a (a.window)}
            <button class="acard" class:sel={dmTarget === a.name} onclick={() => toggleDm(a)}>
              <div class="a-top">
                <span class="ava" style:background={backendColor(a.agent)}>{a.name.slice(0, 1).toUpperCase()}</span>
                {a.name}
                <span class="a-peek" role="button" tabindex="-1" title={t('hubWatch')}
                  onclick={(e) => { e.stopPropagation(); openDrawer(a); }}
                  onkeydown={(e) => { if (e.key === 'Enter') { e.stopPropagation(); openDrawer(a); } }}>
                  <Icon name="terminal" size={12} />
                </span>
              </div>
              <div class="a-state">
                <span class="st" style:background={stateDotColor(a.state)}></span>{a.state}
              </div>
              {#if a.detail}<div class="a-note">{a.detail}</div>{/if}
            </button>
          {/each}
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
          <div class="empty">{managedAgents.length ? t('hubEmpty') : t('hubEmptyNoAgents')}</div>
        {/if}
      </div>

      <div class="composer">
        {#if dmTarget}
          <button class="dm-chip" onclick={() => dmTarget = ''} title={t('hubDmClear')}>
            @{dmTarget} <Icon name="x" size={11} />
          </button>
        {/if}
        <input placeholder={dmTarget ? t('hubComposerDm').replace('{name}', dmTarget) : t('hubComposer')}
          bind:value={composerText} onkeydown={(e) => e.key === 'Enter' && send()} />
        <button class="send-btn" onclick={send}>{t('hubSend')}</button>
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

  {#if createOpen}
    <!-- ── New project: path + name + WHICH AGENTS ── -->
    <div class="dlg-backdrop" onclick={() => createOpen = false} role="presentation"></div>
    <div class="dlg">
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
  .dm-chip { display: flex; align-items: center; gap: 4px; flex: none; background: var(--accent-bg); color: var(--accent); border: none; border-radius: 8px; padding: 6px 10px; font-size: 12px; font-weight: 600; cursor: pointer; font-family: ui-monospace, Menlo, monospace; }
  .composer input { flex: 1; min-width: 0; background: var(--input-bg); border: 1px solid var(--input-border); border-radius: 10px; color: var(--text); padding: 8px 12px; font-size: 13px; outline: none; transition: border-color 160ms; }
  .composer input:focus { border-color: var(--accent); }
  .send-btn { background: var(--accent-bg); color: var(--accent); border: none; border-radius: 10px; padding: 8px 16px; cursor: pointer; font-weight: 600; font-size: 13px; }

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
  .dlg h2 { margin: 0 0 4px; font-size: 15px; }
  .dlg input { background: var(--input-bg); border: 1px solid var(--input-border); border-radius: 9px; color: var(--text); padding: 8px 12px; font-size: 13px; outline: none; }
  .dlg input:focus { border-color: var(--accent); }
  .dlg-h { font-family: ui-monospace, Menlo, monospace; font-size: 10px; text-transform: uppercase; letter-spacing: 1.4px; color: var(--text3); margin-top: 4px; }
  .dlg-agents { display: flex; flex-direction: column; gap: 5px; }
  .agent-pick { display: flex; align-items: center; gap: 8px; background: var(--surface); border: 1px solid var(--border); border-radius: 9px; color: var(--text2); padding: 8px 11px; font-size: 12.5px; cursor: pointer; text-align: left; }
  .agent-pick.sel { border-color: var(--accent); color: var(--text); background: var(--accent-bg); }
  .agent-pick :global(svg) { margin-left: auto; color: var(--accent); }
  .dlg-actions { display: flex; justify-content: flex-end; gap: 8px; margin-top: 6px; }
</style>
