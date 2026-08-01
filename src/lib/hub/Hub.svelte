<script>
  // Hub — the agents-v2 desktop three-column view (docs/exec-plans/agents-v2.md
  // §4.7, prototype agents-v2-prototype.html): sidebar (projects → windows +
  // registry) / project chat + agent state cards / embedded terminal, with a
  // tmux-notation status line across the bottom and a full-screen terminal
  // escape hatch. Desktop-only; mobile keeps the tab layout untouched.
  import Terminal from '../terminal/Terminal.svelte';
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import {
    projectList, projectUp, listSessionsWithPanes,
    hubPost, hubLog, hubAgents, hubSpawn, registryList,
    addTeamMessageListener, removeTeamMessageListener,
  } from '../core/ws.ts';
  import { agentByBackend, sortRows } from '../projects/projects.ts';
  import { stateDotColor, mergeMessages, statuslineWindows, backendColor } from './hub.ts';

  let { visible = false, fontSize = 14, mobile = false, openTerminal = () => {} } = $props();

  // A desktop browser squeezed to phone width must not overflow: the layout
  // follows the VIEWPORT, not the device class. `mobile` (touch) still
  // decides behavior defaults, but the single-column shape kicks in for any
  // client narrower than the two-column minimum.
  let narrow = $state(typeof window !== 'undefined' && window.matchMedia('(max-width: 760px)').matches);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 760px)');
    const onChange = () => { narrow = mq.matches; };
    mq.addEventListener('change', onChange);
    return () => mq.removeEventListener('change', onChange);
  });
  const compact = $derived(mobile || narrow);

  let rows = $state([]);            // ProjectRow[] (projects + live flag)
  let panes = $state([]);           // all tmux panes (for window lists)
  let selected = $state('');        // selected project session
  let agents = $state([]);          // HubAgent[] for selected session
  let feed = $state([]);            // chat messages, oldest first
  let registry = $state([]);        // RegAgent[]
  let termTarget = $state('');      // selected pane target for the right column
  let termCommand = $state('');
  let termFull = $state(false);
  let composerText = $state('');
  let spawnOpen = $state(false);
  let spawnBrief = $state('');
  let spawnAgent = $state('');
  let feedEl = $state(null);

  const room = (session) => `proj:${session}`;
  let lastTs = 0;

  const selectedRow = $derived(rows.find((r) => r.project.session === selected) ?? null);
  const liveSelected = $derived(!!selectedRow?.live);
  const working = $derived(agents.filter((a) => a.agent && a.state === 'working').length);
  const agentCount = $derived(agents.filter((a) => a.agent).length);

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
    lastTs = 0;
    agents = [];
    await Promise.all([loadFeed(), loadAgents()]);
    // Default the terminal to the first agent window, else the first window.
    const first = agents.find((a) => a.agent) ?? agents[0];
    if (first) selectWindow(first);
  }

  function selectWindow(a) {
    const p = panes.find((p) => p.session === selected && p.window === a.window && p.active)
      ?? panes.find((p) => p.session === selected && p.window === a.window);
    if (!p) return;
    termTarget = `${p.session}:${p.window}.${p.pane}`;
    termCommand = p.current_command || '';
  }

  // Compact: tapping a card is "go watch it" — there is no embedded column.
  function tapWindow(a) {
    if (!compact) { selectWindow(a); return; }
    const p = panes.find((p) => p.session === selected && p.window === a.window && p.active)
      ?? panes.find((p) => p.session === selected && p.window === a.window);
    if (p) openTerminal(p.session, `${p.session}:${p.window}.${p.pane}`, p.current_command || '');
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

  async function loadAgents() {
    if (!selected) return;
    try {
      agents = (await hubAgents(selected)).agents ?? [];
    } catch { agents = []; }
  }

  function scrollFeed() {
    requestAnimationFrame(() => { if (feedEl) feedEl.scrollTop = feedEl.scrollHeight; });
  }

  async function send() {
    const text = composerText.trim();
    if (!text || !selected) return;
    composerText = '';
    try {
      await hubPost(selected, text);
      await loadFeed();
    } catch (e) { console.warn('hub post failed', e); }
  }

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

  async function bringUp() {
    if (!selectedRow) return;
    try {
      await projectUp(selectedRow.project.id);
      await reload();
      await loadAgents();
    } catch (e) { console.warn('up failed', e); }
  }

  // Live pushes: every bus message carries its room; append ours.
  const onPush = (m) => {
    if (!selected || m?.room !== room(selected)) return;
    feed = mergeMessages(feed, [m]);
    lastTs = Math.max(lastTs, m.ts ?? 0);
    scrollFeed();
    loadAgents(); // a message often means a state change
  };

  $effect(() => {
    addTeamMessageListener(onPush);
    return () => removeTeamMessageListener(onPush);
  });

  // Poll while visible: agents every 5s (cheap derive), feed cursor every 10s
  // (push fallback), projects every 20s.
  $effect(() => {
    if (!visible) return;
    reload();
    registryList().then((r) => { registry = r.agents ?? []; }).catch(() => {});
    const ai = setInterval(loadAgents, 5000);
    const fi = setInterval(loadFeed, 10000);
    const pi = setInterval(reload, 20000);
    return () => { clearInterval(ai); clearInterval(fi); clearInterval(pi); };
  });

  // Esc leaves full-screen (the prototype's contract). Window-level and
  // capture-phase so it wins over xterm's own key handling.
  $effect(() => {
    if (!termFull) return;
    const onKey = (e) => { if (e.key === 'Escape') { termFull = false; e.stopPropagation(); } };
    window.addEventListener('keydown', onKey, true);
    return () => window.removeEventListener('keydown', onKey, true);
  });

  const winsForStatusline = $derived(statuslineWindows(agents, termTarget));
  const fmtTime = (ts) => {
    const d = new Date(ts);
    return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`;
  };
</script>

<div class="hub-root" class:term-full={termFull} class:compact>
  <div class="cols">
    {#if !compact}
    <!-- ── Sidebar ─────────────────────────── -->
    <aside class="sidebar">
      <div class="side-scroll">
        <div class="side-h">{t('hubProjects')}</div>
        {#each rows as row (row.project.id)}
          <div class="proj" class:open={row.project.session === selected}>
            <button class="p-row" onclick={() => selectProject(row.project.session)}>
              <span class="dot" class:off={!row.live}></span>
              <span class="p-name">{row.project.name}</span>
            </button>
            {#if row.project.session === selected}
              <div class="wins">
                {#each agents as a (a.window)}
                  {@const ag = agentByBackend(a.agent)}
                  <button class="win" class:sel={termTarget.startsWith(`${selected}:${a.window}.`)} onclick={() => selectWindow(a)}>
                    <span class="idx">{a.window}:</span>
                    <span class="st" style:background={stateDotColor(a.agent ? a.state : 'shell')}></span>
                    {#if ag}<span class="ava" style:background={backendColor(a.agent)}>{a.name.slice(0, 1).toUpperCase()}</span>{/if}
                    <span class="w-name">{a.name}</span>
                  </button>
                {/each}
                {#if liveSelected}
                  <button class="win add" onclick={() => { spawnOpen = !spawnOpen; }}>
                    <span class="idx"></span>＋ {t('hubSpawn')}
                  </button>
                {/if}
              </div>
            {/if}
          </div>
        {/each}

        <div class="side-h">{t('hubRegistry')}</div>
        {#each registry as r (r.name)}
          {@const ag = agentByBackend(r.backend)}
          <div class="reg">
            {#if ag}<span class="ava" style:background={backendColor(r.backend)}>{r.name.slice(0, 1).toUpperCase()}</span>{/if}
            {r.name} · {r.backend}{r.can_hire ? ' ⚡' : ''}
          </div>
        {/each}
      </div>
    </aside>
    {/if}

    <!-- ── Middle: chat + agent cards ──────── -->
    <main class="mid">
      {#if compact}
        <!-- Compact: the project picker is a chip row — the sidebar's job in
             one thumb-scrollable line. -->
        <div class="proj-chips">
          {#each rows as row (row.project.id)}
            <button class="pchip" class:sel={row.project.session === selected} onclick={() => selectProject(row.project.session)}>
              <span class="dot" class:off={!row.live}></span>{row.project.name}
            </button>
          {/each}
        </div>
      {/if}
      <div class="mid-head">
        <h1>{selectedRow?.project.name ?? ''}</h1>
        {#if !compact}<span class="path">{selectedRow?.project.path ?? ''}</span>{/if}
        <span class="spacer"></span>
        {#if selected && !liveSelected}
          <button class="chip-btn" onclick={bringUp}>{t('projectOpen')}</button>
        {/if}
        {#if liveSelected}
          <button class="chip-btn" onclick={() => { spawnOpen = !spawnOpen; }}>＋ {t('hubSpawn')}</button>
        {/if}
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

      {#if agents.some((a) => a.agent)}
        <div class="cards">
          {#each agents.filter((a) => a.agent) as a (a.window)}
            {@const ag = agentByBackend(a.agent)}
            <button class="acard" class:sel={!compact && termTarget.startsWith(`${selected}:${a.window}.`)} onclick={() => tapWindow(a)}>
              <div class="a-top">
                {#if ag}<span class="ava" style:background={backendColor(a.agent)}>{a.name.slice(0, 1).toUpperCase()}</span>{/if}
                {a.name}
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
        {#each feed as m (m.id ?? `${m.ts}-${m.from}`)}
          {#if (m.body ?? '').startsWith('⚡')}
            <div class="sysline">{m.from}: {m.body}</div>
          {:else}
            <div class="msg" class:me={m.from === 'human'}>
              <div class="m-head"><span class="who">{m.from === 'human' ? t('hubYou') : m.from}</span><span>{fmtTime(m.ts)}</span></div>
              <div class="bubble">{m.body}</div>
            </div>
          {/if}
        {/each}
        {#if !feed.length}
          <div class="empty">{t('hubEmpty')}</div>
        {/if}
      </div>

      <div class="composer">
        <input placeholder={t('hubComposer')} bind:value={composerText} onkeydown={(e) => e.key === 'Enter' && send()} />
        <button onclick={send}>{t('hubSend')}</button>
      </div>
    </main>

    {#if !compact}
    <!-- ── Right: embedded terminal ────────── -->
    <section class="termcol">
      <div class="term-head">
        <span class="t-title">{termTarget || t('hubNoPane')}</span>
        <span class="spacer"></span>
        <button class="icon-btn" title={t('hubOpenFull')} onclick={() => { const m = /^(.+):(\d+)\.(\d+)$/.exec(termTarget); if (m) openTerminal(m[1], termTarget, termCommand); }}>
          <Icon name="terminal" size={14} />
        </button>
        <button class="icon-btn" title={t('hubFullscreen')} onclick={() => termFull = !termFull}>
          <Icon name={termFull ? 'minimize' : 'maximize'} size={14} />
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

  {#if !compact}
  <!-- ── Signature: the tmux status line ──── -->
  <footer class="statusline">
    <span class="sess">{selected || '—'}</span>
    <div class="wlist">
      {#each winsForStatusline as w (w.window)}
        <button class="w" class:cur={w.current} onclick={() => selectWindow(w)}>{w.label}</button>
      {/each}
    </div>
    <div class="right">
      <span>{agentCount} agents · {working} working</span>
    </div>
  </footer>
  {/if}
</div>

<style>
  .hub-root { height: 100%; display: flex; flex-direction: column; min-height: 0; background: var(--bg); }
  .hub-root.compact .cols { grid-template-columns: minmax(0, 1fr); }
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
  .pchip .dot { width: 6px; height: 6px; border-radius: 50%; background: var(--status-ok); }
  .pchip .dot.off { background: var(--text3); }
  .cols { flex: 1; display: grid; grid-template-columns: 240px minmax(320px, 1fr) minmax(380px, 1.2fr); min-height: 0; }
  .term-full .cols { grid-template-columns: 1fr; }
  .term-full .sidebar, .term-full .mid { display: none; }

  .sidebar { background: var(--bg2); border-right: 1px solid var(--border); display: flex; flex-direction: column; min-height: 0; }
  .side-scroll { flex: 1; overflow-y: auto; padding: 8px; }
  .side-h { font-family: var(--mono, ui-monospace, Menlo, monospace); font-size: 10px; text-transform: uppercase; letter-spacing: 1.4px; color: var(--text3); padding: 10px 10px 5px; }
  .p-row { display: flex; align-items: center; gap: 8px; width: 100%; text-align: left; background: none; border: none; padding: 7px 10px; border-radius: 9px; color: var(--text); cursor: pointer; font-size: 13px; }
  .p-row:hover { background: var(--surface2); }
  .proj.open .p-row { background: var(--accent-bg); }
  .dot { width: 7px; height: 7px; border-radius: 50%; background: var(--status-ok); flex: none; }
  .dot.off { background: var(--text3); }
  .p-name { flex: 1; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; font-weight: 550; }
  .wins { padding: 2px 0 6px 10px; }
  .win { display: flex; align-items: center; gap: 7px; width: 100%; text-align: left; background: none; border: none; padding: 4px 10px; border-radius: 8px; color: var(--text2); cursor: pointer; font-size: 12.5px; }
  .win:hover { background: var(--surface2); color: var(--text); }
  .win.sel { background: var(--surface2); color: var(--text); box-shadow: inset 2px 0 0 var(--accent); }
  .win .idx { font-family: ui-monospace, Menlo, monospace; font-size: 11px; color: var(--text3); width: 22px; flex: none; text-align: right; }
  .win.sel .idx { color: var(--accent); }
  .st { width: 6px; height: 6px; border-radius: 50%; flex: none; }
  .w-name { font-family: ui-monospace, Menlo, monospace; font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .win.add { color: var(--text3); font-size: 12px; }
  .win.add:hover { color: var(--accent); }
  .ava { width: 16px; height: 16px; border-radius: 5px; flex: none; display: grid; place-items: center; font-family: ui-monospace, Menlo, monospace; font-size: 9px; font-weight: 700; color: var(--bg); }
  .reg { display: flex; align-items: center; gap: 8px; padding: 4px 10px; color: var(--text2); font-size: 12px; }

  .mid { display: flex; flex-direction: column; min-width: 0; min-height: 0; }
  .mid-head { display: flex; align-items: center; gap: 10px; padding: 10px 16px; border-bottom: 1px solid var(--border); }
  .mid-head h1 { font-family: ui-monospace, Menlo, monospace; font-size: 15px; margin: 0; font-weight: 600; }
  .path { font-family: ui-monospace, Menlo, monospace; font-size: 11px; color: var(--text3); white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .spacer { flex: 1; }
  .chip-btn { display: flex; align-items: center; gap: 5px; background: var(--surface); border: 1px solid var(--border); color: var(--text2); border-radius: 8px; padding: 5px 11px; font-size: 12.5px; cursor: pointer; transition: border-color 160ms, color 160ms; }
  .chip-btn:hover { border-color: var(--accent); color: var(--accent); }
  .chip-btn:disabled { opacity: 0.4; cursor: default; }

  .spawn-form { display: flex; gap: 8px; padding: 10px 16px; border-bottom: 1px solid var(--border2); }
  .spawn-form select, .spawn-form input { background: var(--input-bg); border: 1px solid var(--input-border); border-radius: 8px; color: var(--text); padding: 6px 10px; font-size: 12.5px; }
  .spawn-form input { flex: 1; }

  .cards { display: flex; gap: 8px; padding: 10px 16px; overflow-x: auto; border-bottom: 1px solid var(--border2); }
  .acard { flex: none; width: 150px; background: var(--surface); border: 1px solid var(--border); border-radius: 11px; padding: 9px 11px; cursor: pointer; text-align: left; transition: border-color 160ms; }
  .acard:hover { border-color: var(--input-border); }
  .acard.sel { border-color: var(--accent); background: var(--accent-bg); }
  .a-top { display: flex; align-items: center; gap: 6px; font-family: ui-monospace, Menlo, monospace; font-weight: 600; font-size: 12.5px; color: var(--text); }
  .a-state { font-family: ui-monospace, Menlo, monospace; font-size: 10.5px; color: var(--text2); margin-top: 5px; display: flex; align-items: center; gap: 5px; }
  .a-note { font-size: 11px; color: var(--text3); margin-top: 3px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  .feed { flex: 1; overflow-y: auto; padding: 14px 18px; display: flex; flex-direction: column; gap: 12px; }
  .msg { max-width: 84%; }
  .msg.me { align-self: flex-end; }
  .m-head { font-size: 11px; color: var(--text3); margin-bottom: 3px; display: flex; gap: 7px; align-items: baseline; }
  .m-head .who { color: var(--text2); font-family: ui-monospace, Menlo, monospace; font-weight: 600; font-size: 11.5px; }
  .bubble { background: var(--surface); border: 1px solid var(--border2); border-radius: 12px; padding: 8px 12px; font-size: 13px; color: var(--text); white-space: pre-wrap; word-break: break-word; }
  .msg.me .bubble { background: var(--accent-bg); border-color: transparent; }
  .sysline { align-self: center; font-size: 11px; color: var(--text3); background: var(--surface); border-radius: 999px; padding: 3px 13px; max-width: 92%; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .empty { color: var(--text3); font-size: 12.5px; text-align: center; margin: auto; }

  .composer { display: flex; gap: 8px; padding: 10px 16px; border-top: 1px solid var(--border); }
  .composer input { flex: 1; background: var(--input-bg); border: 1px solid var(--input-border); border-radius: 10px; color: var(--text); padding: 8px 12px; font-size: 13px; outline: none; transition: border-color 160ms; }
  .composer input:focus { border-color: var(--accent); }
  .composer button { background: var(--accent-bg); color: var(--accent); border: none; border-radius: 10px; padding: 0 16px; cursor: pointer; font-weight: 600; font-size: 13px; }

  .termcol { display: flex; flex-direction: column; min-width: 0; min-height: 0; background: #000; border-left: 1px solid var(--border); }
  .term-head { display: flex; align-items: center; gap: 10px; padding: 6px 10px; background: var(--bg2); border-bottom: 1px solid var(--border); }
  .t-title { font-family: ui-monospace, Menlo, monospace; font-size: 12px; color: var(--text2); }
  .icon-btn { display: grid; place-items: center; background: none; border: 1px solid var(--border); color: var(--text2); border-radius: 7px; width: 28px; height: 25px; cursor: pointer; transition: border-color 160ms, color 160ms; }
  .icon-btn:hover { border-color: var(--accent); color: var(--accent); }
  /* Flex column so the embedded <Terminal> (.terminal { flex: 1 }) is
     constrained to the column height — without it xterm grows to its natural
     row count and the live TUI chrome is clipped below the fold (see
     AgentGrid's identical rule). */
  .term-body { flex: 1; min-width: 0; min-height: 0; position: relative; display: flex; flex-direction: column; }

  .statusline { display: flex; align-items: center; height: 25px; background: var(--bg3); border-top: 1px solid var(--border); font-family: ui-monospace, Menlo, monospace; font-size: 11.5px; color: var(--text2); user-select: none; flex: none; }
  .statusline .sess { background: var(--accent); color: #06232b; font-weight: 700; padding: 0 10px; height: 100%; display: flex; align-items: center; }
  .wlist { display: flex; height: 100%; overflow-x: auto; }
  .statusline .w { display: flex; align-items: center; padding: 0 9px; color: var(--text3); background: none; border: none; cursor: pointer; font: inherit; transition: color 160ms; }
  .statusline .w:hover { color: var(--text); }
  .statusline .w.cur { background: var(--surface2); color: var(--accent); }
  .statusline .right { margin-left: auto; padding: 0 12px; color: var(--text3); white-space: nowrap; }

  /* Narrow desktop: the hub and terminal become either/or — chat keeps the
     room, the Terminal tab (or full-screen) covers watching. */
  @media (max-width: 1100px) {
    .hub-root:not(.compact):not(.term-full) .cols { grid-template-columns: 220px minmax(0, 1fr); }
    .hub-root:not(.compact):not(.term-full) .termcol { display: none; }
  }
</style>
