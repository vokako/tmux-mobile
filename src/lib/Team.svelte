<script>
  // Team tab — the team multi-agent group chat.
  //
  // Talks to the in-process team bus on the desktop server (RPC methods
  // team_history / team_roster / team_post + the team_message push). The human
  // operator ("you") is just another participant: type to broadcast, or tap an
  // agent to @mention it. Tapping an agent's roster chip jumps to the tmux pane
  // that agent runs in (per-workspace session tmm-team-<slug>, window named
  // after the agent), so you can preview its live execution state.
  //
  // Availability: a server without the bus (mobile, or desktop with team
  // disabled) makes the team_* RPCs reject with method-not-found; we surface
  // that as an "unavailable" state and the App hides the tab.
  import Icon from './Icon.svelte';
  import AgentGrid from './AgentGrid.svelte';
  import CollabGraph from './CollabGraph.svelte';
  import DirPicker from './DirPicker.svelte';
  import { marked } from 'marked';
  import { t } from './i18n.svelte.js';
  import { layout } from './layout.svelte.js';
  import {
    teamHistory, teamRoster, teamPost, teamStatus, teamStartTeam,
    teamCloseTeam, teamEmployees, teamTemplateSave, teamTemplateDelete,
    teamSystemPromptSave,
    addTeamMessageListener, removeTeamMessageListener,
    listSessionsWithPanes, fsCwd,
  } from './ws.js';
  import TeamTemplates from './TeamTemplates.svelte';

  let {
    visible = false,
    currentSession = '',     // the open terminal session, used to default the workspace
    fontSize = 14,           // app standard size; the grid renders 2 notches smaller
    openTerminal = () => {}, // (session, target, command) — preview an agent's pane
  } = $props();

  // Desktop + wide → show the split layout (agent grid | chat). Mobile/narrow
  // keeps the chat-only view (roster chips already give pane preview via tab).
  const SPLIT_MIN_WIDTH = 900;
  let wideEnough = $state(typeof window !== 'undefined' && window.innerWidth >= SPLIT_MIN_WIDTH);
  let splitEligible = $derived(!layout.isTouchDevice && (layout.forceDesktop || wideEnough));
  $effect(() => {
    const onResize = () => { wideEnough = window.innerWidth >= SPLIT_MIN_WIDTH; };
    onResize();
    window.addEventListener('resize', onResize);
    return () => window.removeEventListener('resize', onResize);
  });

  // Grid pane width as a fraction (left = agent grid). Draggable splitter.
  let gridFrac = $state(parseFloat(localStorage.getItem('tmux_team_gridfrac') || '0.6'));
  // Latest live message drives the collaboration graph.
  let lastEvent = $state(null);
  // Swap the desktop split's left/right regions (some prefer the chat on the left).
  let swapSides = $state(localStorage.getItem('tmux_team_swap') === '1');
  function toggleSwap() {
    swapSides = !swapSides;
    localStorage.setItem('tmux_team_swap', swapSides ? '1' : '0');
  }
  // Mobile-only: the graph rides in a collapsible panel above the chat (there's
  // no preview grid on phones, so this is the only place it can show). Desktop
  // shows it as a cell in the preview grid instead.
  let collabOpen = $state(localStorage.getItem('tmux_team_collab') === '1');
  function toggleCollab() {
    collabOpen = !collabOpen;
    localStorage.setItem('tmux_team_collab', collabOpen ? '1' : '0');
  }
  let collabHeight = $state(parseInt(localStorage.getItem('tmux_team_collabh') || '210', 10));
  function startCollabResize(e) {
    e.preventDefault();
    const startY = e.touches ? e.touches[0].clientY : e.clientY;
    const startH = collabHeight;
    const move = (ev) => {
      const y = ev.touches ? ev.touches[0].clientY : ev.clientY;
      collabHeight = Math.max(140, Math.min(640, startH + (y - startY)));
    };
    const end = () => {
      localStorage.setItem('tmux_team_collabh', String(Math.round(collabHeight)));
      window.removeEventListener('mousemove', move); window.removeEventListener('mouseup', end);
      window.removeEventListener('touchmove', move); window.removeEventListener('touchend', end);
    };
    window.addEventListener('mousemove', move); window.addEventListener('mouseup', end);
    window.addEventListener('touchmove', move, { passive: false }); window.addEventListener('touchend', end);
  }

  // ─── Multiple teams ──────────────────────────────────────────────────────
  // Each team is an isolated room. `teams` is the live list; `activeRoom` is
  // the one in view. All chat data is scoped to activeRoom; pushes are filtered
  // by their message's room. `newTeam` toggles the start-a-new-team panel.
  let teams = $state([]);        // [{ room, workspace, session, started, agents }]
  let activeRoom = $state('');   // '' = none selected yet
  let newTeam = $state(false);   // true = showing the "new team" workspace picker
  let switcherOpen = $state(false);
  let employees = $state([]);    // active team's desired roster (grid cells)

  let messages = $state([]);     // active room Message[] (oldest first)
  let roster = $state([]);       // active room AgentRow[]
  let available = $state(true);  // false when the server has no team bus
  let starting = $state(false);
  let startError = $state('');   // surfaced when start_team is refused (e.g. bad template)
  let loading = $state(true);
  let draft = $state('');
  let sending = $state(false);
  let listEl = $state(null);
  let inputEl = $state(null);
  // Workspace for a NEW team. Defaulted (current session cwd > server default).
  let workspace = $state('');
  let showPicker = $state(false);   // folder-browser open in the new-team panel
  // Roster templates (named). `templates` = [{name, agents}]; `selectedTemplate`
  // is the one a new team will use. `showTemplates` opens the editor panel.
  let templates = $state([]);
  let selectedTemplate = $state('default');
  let showTemplates = $state(false);
  let tplOpen = $state(false);   // template dropdown open
  let systemPrompt = $state(''); // global system prompt, shared across all teams
  let selectedTplAgents = $derived(templates.find(x => x.name === selectedTemplate)?.agents?.length ?? null);

  let activeTeam = $derived(teams.find(x => x.room === activeRoom) || null);
  // team session for the active team (window_name → agent for pane preview).
  let teamSession = $derived(activeRoom ? `tmm-team-${activeRoom}` : '');

  // Roster entries that are present (not offline). The human posts as "human";
  // never show it as an addressable agent (you can't @ yourself usefully).
  let agents = $derived(roster.filter(a => a.status !== 'offline' && a.name !== 'human'));

  // Current live status for a message author (for the dot on its name label).
  function statusOf(name) {
    return roster.find(a => a.name === name)?.status || 'offline';
  }

  function scrollToBottom() {
    requestAnimationFrame(() => { if (listEl) listEl.scrollTop = listEl.scrollHeight; });
  }

  // Refresh the team list + default workspace. Picks an active room if none.
  async function refreshTeams() {
    const s = await teamStatus();
    teams = s?.teams || [];
    templates = s?.templates || [];
    systemPrompt = s?.system_prompt || '';
    // Keep selectedTemplate valid (default if the chosen one vanished).
    if (!templates.some(x => x.name === selectedTemplate)) {
      selectedTemplate = templates[0]?.name || 'default';
    }
    available = true;
    if (!workspace) {
      let ws = '';
      if (currentSession) {
        try { ws = (await fsCwd(currentSession))?.path || ''; } catch {}
      }
      workspace = ws || s?.default_workspace || '';
    }
    // Auto-select: keep current if still present, else first team, else none.
    if (!teams.some(x => x.room === activeRoom)) {
      activeRoom = teams[0]?.room || '';
    }
    if (!activeRoom) newTeam = teams.length === 0; // no teams → straight to new-team panel
  }

  // Full load for the active room: history + roster + employees.
  async function refresh() {
    try {
      await refreshTeams();
      if (activeRoom) {
        const [h, r, e] = await Promise.all([
          teamHistory(activeRoom, 200), teamRoster(activeRoom), teamEmployees(activeRoom),
        ]);
        messages = h?.messages || [];
        roster = r?.roster || [];
        employees = e?.employees || [];
      } else {
        messages = []; roster = []; employees = [];
      }
      scrollToBottom();
    } catch (e) {
      available = false;
    } finally {
      loading = false;
    }
  }

  // Lightweight poll: team list + active room's roster/employees. Cheap; keeps
  // the status bar + switcher live. No history reload.
  let pollInFlight = false;
  async function refreshRoster() {
    if (pollInFlight) return;
    pollInFlight = true;
    try {
      const s = await teamStatus();
      teams = s?.teams || [];
      available = true;
      if (!teams.some(x => x.room === activeRoom)) activeRoom = teams[0]?.room || '';
      if (activeRoom) {
        const [r, e] = await Promise.all([teamRoster(activeRoom), teamEmployees(activeRoom)]);
        roster = r?.roster || [];
        employees = e?.employees || [];
      }
    } catch { /* transient; next tick retries */ }
    finally { pollInFlight = false; }
  }

  // Switch the active team: reload its chat. Clears the current view first so
  // we never show team A's messages under team B's header.
  async function selectTeam(room) {
    switcherOpen = false;
    newTeam = false;
    if (room === activeRoom) return;
    activeRoom = room;
    messages = []; roster = []; employees = [];
    await refresh();
  }

  // Splitter drag: adjust the grid/chat width ratio (desktop only).
  let splitRow = $state(null);
  function startDrag(e) {
    e.preventDefault();
    const rect = splitRow?.getBoundingClientRect();
    if (!rect) return;
    const onMove = (ev) => {
      const x = (ev.touches ? ev.touches[0].clientX : ev.clientX) - rect.left;
      const f = Math.min(0.8, Math.max(0.2, x / rect.width));
      // When swapped, the LEFT pane is the chat, so the grid fraction is mirrored.
      gridFrac = swapSides ? (1 - f) : f;
    };
    const onUp = () => {
      localStorage.setItem('tmux_team_gridfrac', String(gridFrac));
      window.removeEventListener('mousemove', onMove);
      window.removeEventListener('mouseup', onUp);
    };
    window.addEventListener('mousemove', onMove);
    window.addEventListener('mouseup', onUp);
  }

  // Start a NEW team for the chosen workspace; its room = the workspace slug.
  async function startTeam() {
    if (starting || !workspace.trim()) return;
    starting = true;
    startError = '';
    showPicker = false;
    try {
      const res = await teamStartTeam(workspace.trim(), selectedTemplate);
      // The backend refuses (started:false + error) when the roster is missing/
      // empty — surface it instead of dropping into a room with no agents.
      if (res?.error) { startError = res.error; return; }
      newTeam = false;
      if (res?.room) {
        activeRoom = res.room;
        messages = []; roster = []; employees = [];
      }
      await refresh();
    } catch (e) {
      startError = String(e?.message || e || 'failed to start team');
    } finally {
      starting = false;
    }
  }

  // Close the active team (kill its agents); chat log persists server-side.
  async function closeActiveTeam() {
    if (!activeRoom) return;
    switcherOpen = false;
    const room = activeRoom;
    try { await teamCloseTeam(room); } catch {}
    activeRoom = '';
    await refresh();
  }

  // Live push: append messages for the ACTIVE room only (each Message carries
  // its room). join/leave/system → refresh presence immediately. Messages for
  // other rooms still bump the team list via the poll.
  function onTeamMessage(m) {
    if (!m?.id) return;
    if (m.room && activeRoom && m.room !== activeRoom) return; // other team
    lastEvent = m; // drive the collaboration graph (it de-dupes by id)
    if (m.kind === 'join' || m.kind === 'leave' || m.kind === 'system') {
      refreshRoster();
    }
    if (messages.some(x => x.id === m.id)) return;
    messages = [...messages, m];
    scrollToBottom();
  }

  $effect(() => {
    addTeamMessageListener(onTeamMessage);
    return () => removeTeamMessageListener(onTeamMessage);
  });

  // While visible: full refresh once, then poll on a tight interval so the
  // status bar + team list stay live. Stops when the tab hides.
  const ROSTER_POLL_MS = 1000;
  $effect(() => {
    if (!visible) return;
    refresh();
    const id = setInterval(refreshRoster, ROSTER_POLL_MS);
    return () => clearInterval(id);
  });

  async function send() {
    const body = draft.trim();
    if (!body || sending || !activeRoom) return;
    sending = true;
    try {
      await teamPost(activeRoom, body);
      draft = '';
      if (inputEl) inputEl.style.height = 'auto'; // collapse back to one row
      // The post echoes back via the team_message push, so we don't append
      // locally (avoids a duplicate).
    } catch {
      // Leave the draft in place so the user can retry.
    } finally {
      sending = false;
    }
  }

  function onKeydown(e) {
    // Cmd/Ctrl+Enter sends. A bare Enter inserts a newline (multi-line input),
    // and crucially does NOTHING while an IME is composing — a Chinese IME uses
    // Enter to confirm a candidate, and intercepting it would both eat that
    // confirmation and fire a premature send. `isComposing` (and the legacy
    // keyCode 229) guard against that.
    if (e.isComposing || e.keyCode === 229) return;
    if (e.key === 'Enter' && (e.metaKey || e.ctrlKey)) {
      e.preventDefault();
      send();
    }
    // bare Enter → default behavior (newline); textarea auto-grows via autogrow.
  }

  // Grow the textarea with its content up to a max, then scroll internally.
  function autogrow(el) {
    if (!el) return;
    el.style.height = 'auto';
    el.style.height = Math.min(el.scrollHeight, 160) + 'px';
  }

  function mention(name) {
    // Insert "@name " into the draft so the next post addresses that agent
    // (the bus turns an @mention into a reply obligation).
    const sep = draft && !draft.endsWith(' ') ? ' ' : '';
    draft = `${draft}${sep}@${name} `;
    // Keep the keyboard up: the chip tap must not steal focus from the input.
    inputEl?.focus();
  }

  // Jump to the tmux pane an agent runs in. The team session names each agent's
  // window after the agent, so we find the pane whose window_name matches in
  // our per-workspace session.
  async function previewAgent(name) {
    try {
      const { panes } = await listSessionsWithPanes();
      const p = (panes || []).find(p =>
        p.session === teamSession && (p.window_name === name)
      ) || (panes || []).find(p => p.window_name === name);
      if (!p) return;
      const target = `${p.session}:${p.window}.${p.pane}`;
      openTerminal(p.session, target, p.current_command || '');
    } catch {}
  }

  function fmtTime(ts) {
    try {
      return new Date(ts).toLocaleTimeString('en', { hour12: false, hour: '2-digit', minute: '2-digit' });
    } catch { return ''; }
  }

  function isSystem(m) { return m.kind === 'join' || m.kind === 'leave' || m.kind === 'system'; }
  function isMine(m) { return m.kind === 'msg' && m.from === 'human'; }

  // Render a message body as markdown. Agent output is UNTRUSTED, and marked
  // (v17) no longer sanitizes, so we escape HTML first — markdown syntax (**,
  // #, ```fences```, tables, links) still renders, but any raw <script>/<img
  // onerror> becomes inert literal text. Memoized by body (chat re-renders on
  // every poll/push). Links open in the system browser via App's global
  // a[href] handler.
  const _mdCache = new Map();
  function renderMarkdown(body) {
    const src = body || '';
    const hit = _mdCache.get(src);
    if (hit !== undefined) return hit;
    const escaped = src.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
    let html;
    try { html = marked.parse(escaped, { gfm: true, breaks: true }); }
    catch { html = escaped; }
    if (_mdCache.size > 500) _mdCache.clear(); // bound the cache
    _mdCache.set(src, html);
    return html;
  }
</script>

{#snippet teamSwitcher()}
  <!-- Header: active-team dropdown + close, then the live agent status chips on
       the SAME row (wrapping to the next line when they overflow). -->
  <div class="team-header">
    <div class="team-pick">
      <button class="team-pick-btn" onclick={() => switcherOpen = !switcherOpen}>
        <span class="team-pick-name">{activeTeam ? activeTeam.room : (newTeam ? t('teamNew') : t('teamNone'))}</span>
        <Icon name="chevron-down" size={10} />
      </button>
      {#if switcherOpen}
        <button class="team-pick-backdrop" aria-label="close" onclick={() => switcherOpen = false}></button>
        <div class="team-pick-menu">
          {#each teams as tm}
            <button class="team-pick-item" class:active={tm.room === activeRoom} onclick={() => selectTeam(tm.room)} title={tm.workspace}>
              <span class="tp-dot" class:on={tm.agents > 0}></span>
              <span class="tp-name">{tm.room}</span>
              <span class="tp-count">{tm.agents}</span>
            </button>
          {/each}
          {#if teams.length === 0}
            <div class="team-pick-empty">{t('teamNone')}</div>
          {/if}
          <button class="team-pick-new" onclick={() => { newTeam = true; switcherOpen = false; }}>
            <Icon name="plus" size={12} /> {t('teamNew')}
          </button>
        </div>
      {/if}
    </div>
    {#if activeTeam}
      {#if !splitEligible}
        <button class="team-hbtn" class:on={collabOpen} onclick={toggleCollab} title={t('teamCollab')} aria-label={t('teamCollab')}>
          <Icon name="collab" size={14} />
        </button>
      {/if}
      {#if splitEligible}
        <button class="team-swap" onclick={toggleSwap} class:on={swapSides} title={t('teamSwap')} aria-label={t('teamSwap')}>
          <Icon name="swap-h" size={14} />
        </button>
      {/if}
      <button class="team-close" onclick={closeActiveTeam} title={t('teamClose')} aria-label={t('teamClose')}>
        <Icon name="x" size={13} />
      </button>
    {/if}
    <!-- Agent status chips: dot + name only; tap to preview the agent's pane.
         flex-wrap drops overflow onto the next line. -->
    {#if !newTeam && activeRoom}
      {#each agents as a}
        <button class="roster-chip" onclick={() => previewAgent(a.name)} title={a.role || a.name}>
          <span class="roster-dot status-{a.status}"></span>
          <span class="roster-name">{a.name}</span>
        </button>
      {/each}
    {/if}
  </div>
{/snippet}

{#snippet newTeamPanel()}
  <div class="team-start-panel">
    <div class="start-row">
      <span class="start-ws-label">{t('teamWorkspace')}</span>
      <!-- Editable path + a folder-browse button (same UX as new-session). -->
      <input class="start-ws-input" bind:value={workspace} placeholder="/path/to/project" autocapitalize="off" />
      <button class="start-browse" class:on={showPicker} onclick={() => showPicker = !showPicker} aria-label={t('teamBrowse')} title={t('teamBrowse')}>
        <Icon name="folder" size={14} />
      </button>
    </div>
    {#if showPicker}
      <DirPicker start={workspace || undefined}
        onNavigate={(p) => { workspace = p; }}
        onPick={(p) => { workspace = p; showPicker = false; }}
        onClose={() => showPicker = false} />
    {/if}
    <!-- Roster template picker: which named roster (A/B/…) this team uses. A
         custom dropdown (not a native <select>, which pops a separate OS menu
         on desktop WKWebView) to match the rest of the in-app UI. -->
    <div class="start-row">
      <span class="start-ws-label">{t('teamTemplate')}</span>
      <div class="start-tpl-pick">
        <button class="start-tpl" onclick={() => tplOpen = !tplOpen}>
          <span class="start-tpl-name">{selectedTemplate}{#if selectedTplAgents != null} ({selectedTplAgents}){/if}</span>
          <Icon name="chevron-down" size={10} />
        </button>
        {#if tplOpen}
          <button class="start-tpl-backdrop" aria-label="close" onclick={() => tplOpen = false}></button>
          <div class="start-tpl-menu">
            {#each templates as tpl}
              <button class="start-tpl-item" class:active={tpl.name === selectedTemplate}
                onclick={() => { selectedTemplate = tpl.name; tplOpen = false; }}>
                <span class="stt-name">{tpl.name}</span>
                <span class="stt-count">{tpl.agents?.length ?? 0}</span>
              </button>
            {/each}
          </div>
        {/if}
      </div>
      <button class="start-browse" onclick={() => showTemplates = true} aria-label={t('teamEditTemplates')} title={t('teamEditTemplates')}>
        <Icon name="edit" size={14} />
      </button>
    </div>
    <div class="start-actions">
      <button class="team-start" disabled={starting || !workspace.trim()} onclick={startTeam}>
        {#if starting}<span class="reconnect-spinner-sm"></span>{:else}<Icon name="bot" size={14} />{/if}
        {t('teamStart')}
      </button>
      {#if teams.length > 0}
        <button class="team-start-cancel" onclick={() => { newTeam = false; showPicker = false; }}>{t('cancel')}</button>
      {/if}
    </div>
    {#if startError}
      <div class="start-error" style="margin-top:8px;color:#e5484d;font-size:13px;line-height:1.4;word-break:break-word;">{startError}</div>
    {/if}
  </div>
{/snippet}

{#snippet chatPane()}
  {@render teamSwitcher()}

  {#if newTeam || !activeRoom}
    {@render newTeamPanel()}
  {:else}
    <!-- Agent chips live in the header now. Show a "coming online" hint until
         the first agent appears. -->
    {#if agents.length === 0}
      <div class="team-start-panel">
        <span class="reconnect-spinner-sm"></span>
        <span class="start-hint">{t('teamStarting')}</span>
      </div>
    {/if}

    <!-- Mobile-only graph (toggled from the header button; phones have no preview grid). -->
    {#if !splitEligible && collabOpen}
      <div class="collab-wrap" style="height:{collabHeight}px">
        <CollabGraph {agents} event={lastEvent} />
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="collab-resize" onmousedown={startCollabResize} ontouchstart={startCollabResize} title="Drag to resize"></div>
      </div>
    {/if}

    <!-- Message log -->
    <div class="team-log" bind:this={listEl}>
      {#if messages.length === 0}
        <div class="team-empty">{t('teamNoMessages')}</div>
      {:else}
        {#each messages as m (m.id)}
          {#if isSystem(m)}
            <div class="msg-system">{m.body}</div>
          {:else}
            <div class="msg-row" class:mine={isMine(m)}>
              <div class="msg-bubble" class:mine={isMine(m)}>
                {#if !isMine(m)}<div class="msg-from"><span class="roster-dot status-{statusOf(m.from)}"></span>{m.from}</div>{/if}
                <!-- eslint-disable-next-line svelte/no-at-html-tags -->
                <div class="msg-body md">{@html renderMarkdown(m.body)}</div>
                <div class="msg-time">{fmtTime(m.ts)}</div>
              </div>
            </div>
          {/if}
        {/each}
      {/if}
    </div>

    <!-- Compose -->
    <div class="team-compose">
      {#if agents.length}
        <div class="compose-mentions">
          <button class="mention-chip mention-all" onmousedown={(e) => e.preventDefault()} onclick={() => mention('all')}>@all</button>
          {#each agents as a}
            <button class="mention-chip" onmousedown={(e) => e.preventDefault()} onclick={() => mention(a.name)}>@{a.name}</button>
          {/each}
        </div>
      {/if}
      <div class="compose-row">
        <textarea
          class="compose-input"
          bind:this={inputEl}
          bind:value={draft}
          onkeydown={onKeydown}
          oninput={(e) => autogrow(e.currentTarget)}
          placeholder={t('teamMessage')}
          rows="1"
        ></textarea>
        <button class="compose-send" disabled={!draft.trim() || sending} onclick={send} aria-label={t('teamSend')} title={t('teamSendHint')}>
          <Icon name="send" size={16} />
        </button>
      </div>
    </div>
  {/if}
{/snippet}

<div class="team">
  {#if loading}
    <div class="team-empty">…</div>
  {:else if !available}
    <div class="team-empty team-unavail">
      <Icon name="bot" size={28} />
      <p>{t('teamUnavailable')}</p>
    </div>
  {:else if splitEligible && activeRoom && !newTeam && employees.length > 0}
    <!-- Desktop split: agent grid (left) | draggable splitter | chat (right). -->
    <div class="team-split" bind:this={splitRow}>
      {#if swapSides}
        <div class="team-chat-pane" style="flex: {1 - gridFrac} 1 0;">
          {@render chatPane()}
        </div>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="team-splitter" onmousedown={startDrag} title="Drag to resize"></div>
        <div class="team-grid-pane" style="flex: {gridFrac} 1 0;">
          {#key activeRoom}
            <AgentGrid {teamSession} {employees} {fontSize} {visible} collab={true} collabAgents={agents} collabEvent={lastEvent} />
          {/key}
        </div>
      {:else}
        <div class="team-grid-pane" style="flex: {gridFrac} 1 0;">
          {#key activeRoom}
            <AgentGrid {teamSession} {employees} {fontSize} {visible} collab={true} collabAgents={agents} collabEvent={lastEvent} />
          {/key}
        </div>
        <!-- svelte-ignore a11y_no_static_element_interactions -->
        <div class="team-splitter" onmousedown={startDrag} title="Drag to resize"></div>
        <div class="team-chat-pane" style="flex: {1 - gridFrac} 1 0;">
          {@render chatPane()}
        </div>
      {/if}
    </div>
  {:else}
    {@render chatPane()}
  {/if}

  {#if showTemplates}
    <TeamTemplates
      {templates}
      {systemPrompt}
      onSave={async (name, agents) => { await teamTemplateSave(name, agents); await refresh(); }}
      onDelete={async (name) => { await teamTemplateDelete(name); await refresh(); }}
      onSaveSystemPrompt={async (text) => { await teamSystemPromptSave(text); await refresh(); }}
      onClose={() => showTemplates = false} />
  {/if}
</div>

<style>
  /* Desktop split: grid pane | splitter | chat pane. */
  .team-split { display: flex; height: 100%; min-height: 0; width: 100%; }
  .team-grid-pane { min-width: 0; min-height: 0; overflow: hidden; position: relative; }
  .team-chat-pane {
    min-width: 0; min-height: 0;
    display: flex; flex-direction: column;
    border-left: 1px solid var(--border);
  }
  .team-splitter {
    flex: 0 0 6px; cursor: col-resize; background: var(--border);
    transition: background 0.15s ease;
  }
  .team-splitter:hover { background: var(--accent); }

  .team {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    background: var(--bg);
  }

  /* Team switcher header */
  .team-header {
    display: flex; align-items: center; gap: 6px; flex-wrap: wrap;
    padding: 6px 10px; flex-shrink: 0;
    border-bottom: 1px solid var(--border);
  }
  .team-pick { position: relative; flex-shrink: 0; min-width: 0; }
  .team-pick-btn {
    display: inline-flex; align-items: center; gap: 6px; max-width: 100%;
    padding: 5px 10px; border: 1px solid var(--border2); border-radius: 8px;
    background: var(--input-bg); color: var(--text2);
    font-size: 12px; font-weight: 600; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .team-pick-btn:active { border-color: var(--accent); color: var(--accent); }
  .team-pick-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .team-pick-backdrop { position: fixed; inset: 0; z-index: 30; background: transparent; border: none; }
  .team-pick-menu {
    position: absolute; top: 34px; left: 0; z-index: 31;
    min-width: 200px; max-width: 280px; max-height: 50vh; overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: 10px;
    box-shadow: 0 12px 40px rgba(0,0,0,0.4); padding: 4px;
  }
  .team-pick-item, .team-pick-new {
    display: flex; align-items: center; gap: 8px; width: 100%;
    padding: 7px 9px; border: none; border-radius: 7px; background: transparent;
    color: var(--text2); font-size: 12px; cursor: pointer; text-align: left;
    -webkit-tap-highlight-color: transparent;
  }
  .team-pick-item:active, .team-pick-new:active { background: var(--surface2); }
  .team-pick-item.active { background: var(--accent-bg); color: var(--accent); }
  .tp-dot { width: 7px; height: 7px; border-radius: 50%; background: var(--text3); flex-shrink: 0; }
  .tp-dot.on { background: var(--status-ok); }
  .tp-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; font-family: var(--font-ui); }
  .tp-count { color: var(--text3); font-size: 11px; }
  .team-pick-empty { padding: 8px 9px; color: var(--text3); font-size: 12px; }
  .team-pick-new { color: var(--accent); border-top: 1px solid var(--border2); border-radius: 0 0 7px 7px; margin-top: 2px; }
  .team-close, .team-swap, .team-hbtn {
    flex-shrink: 0; width: 28px; height: 28px; padding: 0;
    border: 1px solid var(--border2); border-radius: 8px;
    background: var(--input-bg); color: var(--text3);
    cursor: pointer; display: flex; align-items: center; justify-content: center;
    -webkit-tap-highlight-color: transparent;
  }
  .team-close:active { color: var(--danger); border-color: var(--danger); }
  .team-swap:active, .team-swap.on,
  .team-hbtn:active, .team-hbtn.on { color: var(--accent); border-color: var(--accent); }
  .team-start-cancel {
    padding: 6px 12px; border: 1px solid var(--border2); border-radius: 8px;
    background: transparent; color: var(--text3); font-size: 12px; cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .team-empty {
    flex: 1;
    display: flex;
    flex-direction: column;
    align-items: center;
    justify-content: center;
    gap: 10px;
    color: var(--text3);
    font-size: 13px;
    padding: 24px;
    text-align: center;
  }
  .team-unavail p { margin: 0; max-width: 260px; line-height: 1.5; }

  /* Roster strip */
  .team-roster {
    display: flex;
    gap: 6px;
    overflow-x: auto;
    padding: 8px 10px;
    border-bottom: 1px solid var(--border);
    flex-shrink: 0;
    scrollbar-width: none;
  }
  .team-roster::-webkit-scrollbar { display: none; }
  .team-roster-empty { color: var(--text3); font-size: 12px; padding: 4px 2px; }
  .team-start-panel {
    display: flex; flex-direction: column; gap: 8px;
    padding: 10px 12px; border-bottom: 1px solid var(--border); flex-shrink: 0;
  }
  .start-hint { color: var(--text3); font-size: 12px; }
  .start-row { display: flex; align-items: center; gap: 8px; min-width: 0; }
  .start-ws-label {
    font-size: 10px; font-weight: 600; color: var(--text3);
    text-transform: uppercase; letter-spacing: 0.5px; white-space: nowrap;
  }
  .start-ws-input {
    flex: 1; min-width: 0;
    padding: 6px 10px; border: 1px solid var(--input-border); border-radius: 8px;
    background: var(--input-bg); color: var(--text);
    font-family: var(--font-mono);
    font-size: 12px; outline: none;
  }
  .start-ws-input:focus { border-color: var(--accent); }
  .start-browse {
    flex-shrink: 0; width: 30px; height: 30px; padding: 0;
    border: 1px solid var(--border2); border-radius: 8px;
    background: var(--input-bg); color: var(--text3);
    cursor: pointer; display: flex; align-items: center; justify-content: center;
    -webkit-tap-highlight-color: transparent;
  }
  .start-browse:active, .start-browse.on { color: var(--accent); border-color: var(--accent); background: var(--accent-bg); }
  .start-tpl-pick { position: relative; flex: 1; min-width: 0; }
  .start-tpl {
    width: 100%; display: flex; align-items: center; justify-content: space-between; gap: 6px;
    padding: 6px 10px; border: 1px solid var(--input-border); border-radius: 8px;
    background: var(--input-bg); color: var(--text); font-size: 12px; cursor: pointer;
    font-family: var(--font-ui);
    -webkit-tap-highlight-color: transparent;
  }
  .start-tpl:active { border-color: var(--accent); }
  .start-tpl-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .start-tpl-backdrop { position: fixed; inset: 0; z-index: 30; background: transparent; border: none; }
  .start-tpl-menu {
    position: absolute; top: 36px; left: 0; right: 0; z-index: 31;
    max-height: 40vh; overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: 10px;
    box-shadow: 0 12px 40px rgba(0,0,0,0.4); padding: 4px;
  }
  .start-tpl-item {
    display: flex; align-items: center; justify-content: space-between; gap: 8px; width: 100%;
    padding: 7px 9px; border: none; border-radius: 7px; background: transparent;
    color: var(--text2); font-size: 12px; cursor: pointer; text-align: left;
    font-family: var(--font-ui);
    -webkit-tap-highlight-color: transparent;
  }
  .start-tpl-item:active { background: var(--surface2); }
  .start-tpl-item.active { background: var(--accent-bg); color: var(--accent); }
  .stt-count { color: var(--text3); font-size: 11px; }
  .start-actions { display: flex; align-items: center; gap: 8px; }
  .team-start {
    display: inline-flex; align-items: center; gap: 6px;
    padding: 5px 12px; height: 28px;
    border: 1px solid var(--accent); border-radius: 999px;
    background: var(--accent-bg); color: var(--accent);
    font-size: 12px; font-weight: 600; cursor: pointer; flex-shrink: 0;
    -webkit-tap-highlight-color: transparent;
  }
  .team-start:active { background: var(--accent); color: var(--bg); }
  .team-start:disabled { opacity: 0.5; cursor: default; }
  .reconnect-spinner-sm {
    width: 12px; height: 12px; border: 2px solid var(--border);
    border-top-color: var(--accent); border-radius: 50%;
    animation: team-spin 0.6s linear infinite;
  }
  @keyframes team-spin { to { transform: rotate(360deg); } }
  .roster-chip {
    display: inline-flex; align-items: center; gap: 6px;
    padding: 3px 9px; height: 26px;
    border: 1px solid var(--border2); border-radius: 999px;
    background: var(--input-bg); color: var(--text2);
    font-size: 12px; font-weight: 500; cursor: pointer; flex-shrink: 0;
    white-space: nowrap; -webkit-tap-highlight-color: transparent;
    transition: border-color 0.15s ease, color 0.15s ease;
  }
  .roster-chip:active { border-color: var(--accent); color: var(--accent); }
  .roster-name { max-width: 120px; overflow: hidden; text-overflow: ellipsis; }
  .roster-dot { width: 7px; height: 7px; border-radius: 50%; flex-shrink: 0; background: var(--text3); }
  .roster-dot.status-working { background: var(--status-warn); }
  .roster-dot.status-waiting { background: var(--status-ok); }
  .roster-dot.status-online { background: var(--accent); }

  /* Collaboration graph panel (mobile only — toggled from the header button). */
  .collab-wrap { position: relative; flex-shrink: 0; background: var(--surface); border-bottom: 1px solid var(--border); }
  /* When the soft keyboard is open there isn't room for the fixed-height graph
     above the chat — the browser would scroll the input into view and push the
     top nav off-screen. Hide it while typing; it returns when the keyboard closes. */
  :global(html.keyboard-open) .collab-wrap { display: none; }
  .collab-resize {
    position: absolute; left: 0; right: 0; bottom: 0; height: 9px;
    cursor: ns-resize; touch-action: none;
  }
  .collab-resize::after {
    content: ''; position: absolute; left: 50%; bottom: 3px;
    width: 40px; height: 3px; transform: translateX(-50%);
    border-radius: 2px; background: var(--border2);
  }
  .collab-resize:active::after { background: var(--accent); }

  /* Message log */
  .team-log {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 12px 10px;
    display: flex;
    flex-direction: column;
    gap: 8px;
    -webkit-overflow-scrolling: touch;
  }
  .msg-system {
    align-self: center;
    color: var(--text3);
    font-size: 11px;
    padding: 2px 10px;
    background: var(--surface);
    border-radius: 999px;
    max-width: 90%;
    text-align: center;
  }
  .msg-row { display: flex; }
  .msg-row.mine { justify-content: flex-end; }
  .msg-bubble {
    max-width: 90%;
    padding: 7px 11px;
    border-radius: 14px;
    background: var(--surface2);
    border: 1px solid var(--border2);
    border-bottom-left-radius: 4px;
  }
  .msg-bubble.mine {
    background: var(--accent-bg);
    border-color: var(--accent);
    border-bottom-left-radius: 14px;
    border-bottom-right-radius: 4px;
  }
  .msg-from {
    display: inline-flex; align-items: center; gap: 5px;
    font-size: 11px; font-weight: 600; color: var(--accent);
    margin-bottom: 2px;
  }
  .msg-body {
    font-size: 13px; line-height: 1.45; color: var(--text);
    word-break: break-word; overflow-wrap: anywhere;
    user-select: text; -webkit-user-select: text;
  }
  /* Markdown element styling inside a message bubble. Tight margins so a
     one-line message looks like one line, not a padded document. */
  .msg-body.md :global(p) { margin: 0 0 6px; }
  .msg-body.md :global(p:last-child) { margin-bottom: 0; }
  .msg-body.md :global(ul),
  .msg-body.md :global(ol) { margin: 4px 0; padding-left: 20px; }
  .msg-body.md :global(li) { margin: 1px 0; }
  .msg-body.md :global(h1),
  .msg-body.md :global(h2),
  .msg-body.md :global(h3),
  .msg-body.md :global(h4) { font-size: 13px; font-weight: 700; margin: 6px 0 3px; }
  .msg-body.md :global(code) {
    font-family: var(--font-mono);
    font-size: 12px; background: var(--code-bg); padding: 1px 4px; border-radius: 4px;
  }
  .msg-body.md :global(pre) {
    background: var(--code-bg); border: 1px solid var(--border2); border-radius: 8px;
    padding: 8px 10px; margin: 6px 0; overflow-x: auto; -webkit-overflow-scrolling: touch;
  }
  .msg-body.md :global(pre code) { background: none; padding: 0; font-size: 12px; line-height: 1.4; }
  .msg-body.md :global(a) { color: var(--accent); text-decoration: underline; }
  .msg-body.md :global(blockquote) {
    margin: 4px 0; padding-left: 10px; border-left: 3px solid var(--border);
    color: var(--text2);
  }
  .msg-body.md :global(table) { border-collapse: collapse; margin: 6px 0; font-size: 12px; }
  .msg-body.md :global(th),
  .msg-body.md :global(td) { border: 1px solid var(--border2); padding: 3px 7px; text-align: left; }
  .msg-body.md :global(hr) { border: none; border-top: 1px solid var(--border2); margin: 8px 0; }
  .msg-body.md :global(img) { max-width: 100%; border-radius: 6px; }
  .msg-time { font-size: 9px; color: var(--text3); margin-top: 3px; text-align: right; }

  /* Compose */
  .team-compose {
    flex-shrink: 0;
    border-top: 1px solid var(--border);
    padding: 8px 10px calc(8px + var(--sab));
    background: var(--nav-bg);
  }
  .compose-mentions {
    display: flex; gap: 5px; overflow-x: auto; padding-bottom: 7px;
    scrollbar-width: none;
  }
  .compose-mentions::-webkit-scrollbar { display: none; }
  .mention-chip {
    flex-shrink: 0;
    padding: 3px 9px; border: 1px solid var(--border2); border-radius: 999px;
    background: transparent; color: var(--text2); font-size: 11px; font-weight: 500;
    cursor: pointer; -webkit-tap-highlight-color: transparent; white-space: nowrap;
  }
  .mention-chip:active { color: var(--accent); border-color: var(--accent); }
  .compose-row { display: flex; align-items: flex-end; gap: 8px; }
  .compose-input {
    flex: 1; min-height: 38px; max-height: 160px;
    padding: 9px 12px; border: 1px solid var(--input-border); border-radius: 8px;
    background: var(--input-bg); color: var(--text); font-size: 14px;
    font-family: inherit; resize: none; line-height: 1.4;
    overflow-y: auto;
    outline: none;
  }
  .compose-input:focus { border-color: var(--accent); }
  .compose-send {
    width: 38px; height: 38px; flex-shrink: 0;
    border: none; border-radius: 8px;
    background: var(--accent-bg); color: var(--accent);
    cursor: pointer; display: flex; align-items: center; justify-content: center;
    -webkit-tap-highlight-color: transparent;
    transition: opacity 0.15s ease;
  }
  .compose-send:disabled { opacity: 0.4; cursor: default; }
  .compose-send:not(:disabled):active { background: var(--accent); color: var(--bg); }
</style>
