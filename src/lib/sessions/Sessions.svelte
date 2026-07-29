<script lang="ts">
  import { listSessions, listPanes, listSessionsWithPanes, newSession, killSession, newWindow, killWindow, fsList } from '../core/ws.ts';
  import type { TmuxSession, TmuxPane } from '../core/ws.ts';
  import Icon from '../ui/Icon.svelte';
  import AgentChip from '../ui/AgentChip.svelte';
  import Projects from '../projects/Projects.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { sessionHasAgent, paneAgent, AGENTS } from '../core/agents.ts';
  // Team-mode sessions (`tmm-team-<room>`) are grouped apart from regular
  // sessions and their clicks route to the Team chat instead of a raw terminal.
  // isTeamSession is gated on the shared teamState.available, so on a server
  // without the team bus these fall back to ordinary sessions (consistently
  // with PanePicker and the Team tab).
  import { isTeamSession, teamRoomOf, teamLabel } from '../core/team.svelte.ts';
  import { agentNotifications, notificationForWindow, sessionHasNotification } from '../core/agent-notifications.svelte.ts';

  let { openTerminal, openTeam = () => {}, activeTarget = '', visible = false }: {
    openTerminal: (session: string, target: string, command?: string) => void;
    openTeam?: (room: string) => void;
    activeTarget?: string;
    visible?: boolean;
  } = $props();

  const isMobile = 'ontouchstart' in window || navigator.maxTouchPoints > 0;

  // ─── State ─────────────────────────────────────────────
  let sessions = $state<TmuxSession[]>([]);
  let panes = $state<Record<string, TmuxPane[]>>({});
  let expanded = $state<Record<string, boolean>>({}); // manually opened in multi-window view
  let error = $state('');
  let query = $state('');
  let searchOpen = $state(false);
  let searchInputEl = $state<HTMLInputElement | null>(null);
  let sessionsEl: HTMLElement | undefined;

  // New session form
  let showNew = $state(false);
  let newName = $state('');
  let newPath = $state('');
  let newCmd = $state('');

  // Folder picker (inside new form)
  type DirEntry = { name: string; type: string; path: string };
  let showPicker = $state(false);
  let pickerPath = $state('');
  let pickerEntries = $state<DirEntry[]>([]);
  let pickerPathEl = $state<HTMLElement | null>(null);

  // Confirm-to-kill gates
  let confirmKill = $state<string | null>(null);       // session name
  let confirmKillWindow = $state<string | null>(null); // "session:window"

  let refreshing = $state(false);

  // ─── Helpers ───────────────────────────────────────────
  const AGENT_BY_TAG = new Map(AGENTS.map(a => [a.tag, a] as const));
  function aiIcon(tag: string) { return AGENT_BY_TAG.get(tag)?.icon || ''; }
  function sessionAgents(sessionName: string) {
    const counts = new Map<string, number>();
    for (const pane of panes[sessionName] || []) {
      const agent = paneAgent(pane);
      if (agent) counts.set(agent.tag, (counts.get(agent.tag) || 0) + 1);
    }
    // Keep the global AGENTS order stable instead of letting pane/window order
    // reshuffle icons whenever tmux adds or removes a window.
    return AGENTS
      .filter(agent => counts.has(agent.tag))
      .map(agent => ({ agent, count: counts.get(agent.tag)! }));
  }
  function relTime(unixSec: number | undefined) {
    if (!unixSec) return '';
    const d = Math.max(0, Math.floor(Date.now() / 1000) - unixSec);
    if (d < 45) return t('justNow');
    if (d < 3600) return `${Math.round(d / 60)}${t('minAbbr')}`;
    if (d < 86400) return `${Math.round(d / 3600)}${t('hourAbbr')}`;
    if (d < 86400 * 7) return `${Math.round(d / 86400)}${t('dayAbbr')}`;
    const date = new Date(unixSec * 1000);
    return `${date.getMonth() + 1}/${date.getDate()}`;
  }
  // Summary derived from panes of a session — the "what is this session"
  // one-liner. Picks the most informative signal:
  //   attached pane > first pane with AI tag > first pane.
  function sessionSummary(s: TmuxSession) {
    const ps = panes[s.name];
    if (!ps || !ps.length) return { ai: '', cmd: '', agents: [] };
    // Prefer pane that matches activeTarget; else first with AI tag; else first.
    const act = ps.find(p => activeTarget === `${p.session}:${p.window}.${p.pane}`);
    const tagged = ps.find(p => paneAgent(p));
    const p = (act || tagged || ps[0])!; // ps checked non-empty above
    // Detect on the full pane signal (command + title + child argv). The
    // session row's big icon comes from this — title-only matching missed
    // interpreter-launched agents (codex = "node", claude = "2.1.141").
    return {
      ai: paneAgent(p)?.tag || '',
      cmd: p.current_command || '',
      agents: sessionAgents(s.name),
    };
  }

  // ─── Data loading ──────────────────────────────────────
  $effect(() => { if (visible) refresh(); });

  // After a reconnect, the cached sessions/panes likely went stale (process
  // exited, new windows, …). Re-pull when we hear the app's reconnect-success
  // event. Only acts if the page is currently visible — invisible pages get
  // their refresh from the visibility $effect above the next time they show.
  $effect(() => {
    const onReconn = () => { if (visible) refresh(); };
    window.addEventListener('ws-reconnected', onReconn);
    return () => window.removeEventListener('ws-reconnected', onReconn);
  });

  async function refresh() {
    try {
      // Single round-trip: server returns sessions[] + panes[] (across all
      // sessions). We group panes by session_name client-side to populate
      // the same shape the rest of the page expects.
      const { sessions: list, panes: allPanes } = await listSessionsWithPanes();
      const activeSession = activeTarget.split(':')[0];
      sessions = list.sort((a, b) => {
        if (a.name === activeSession) return -1;
        if (b.name === activeSession) return 1;
        const la = a.last_opened || 0, lb = b.last_opened || 0;
        if (la !== lb) return lb - la;
        return 0;
      });
      const grouped: Record<string, TmuxPane[]> = {};
      for (const p of allPanes) {
        (grouped[p.session] ||= []).push(p);
      }
      panes = grouped;
      error = '';
    } catch (e) {
      error = (e as Error).message;
    }
  }

  async function doRefresh() {
    refreshing = true;
    await refresh();
    setTimeout(() => refreshing = false, 600);
  }

  // ─── Open / navigation ────────────────────────────────
  // Single-click entry: open the session at its "primary" pane.
  // Multi-window sessions toggle expansion so user can choose.
  function activateSession(s: TmuxSession) {
    // Team session → open the team's chat instead of a raw terminal.
    if (isTeamSession(s.name)) { openTeam(teamRoomOf(s.name)); return; }
    const ps = panes[s.name] || [];
    if (s.windows > 1 && ps.length > 1) {
      expanded[s.name] = !expanded[s.name];
      return;
    }
    const p = ps[0];
    if (!p) return;
    openTerminal(s.name, `${p.session}:${p.window}.${p.pane}`, p.current_command);
  }
  // Chip click is always a direct-open, even for multi-window sessions.
  // The chip is an MRU fast-switch surface — toggling a row's expansion
  // elsewhere on the page would leave the user wondering what happened.
  // We pick the most-informative pane: first AI pane, else first pane.
  function chipOpen(s: TmuxSession) {
    // Team session → chat (chips normally exclude these; guard anyway).
    if (isTeamSession(s.name)) { openTeam(teamRoomOf(s.name)); return; }
    // If this chip represents the currently-active session, return to the
    // exact pane the user was viewing (not whichever AI pane the summary
    // picked). Tapping the active chip = "go back to Terminal".
    if (activeTarget.startsWith(s.name + ':')) {
      const parts = activeTarget.split(':')[1]?.split('.') || [];
      const win = parseInt(parts[0] ?? '', 10);
      const pane = parseInt(parts[1] ?? '', 10);
      const ps = panes[s.name] || [];
      const p = ps.find(x => x.window === win && x.pane === pane) || ps[0];
      if (!p) return;
      openTerminal(s.name, activeTarget, p.current_command);
      return;
    }
    const ps = panes[s.name] || [];
    const aiPane = ps.find(p => paneAgent(p));
    const p = aiPane || ps[0];
    if (!p) return;
    openTerminal(s.name, `${p.session}:${p.window}.${p.pane}`, p.current_command);
  }
  function openPane(s: TmuxSession, p: TmuxPane) {
    openTerminal(s.name, `${p.session}:${p.window}.${p.pane}`, p.current_command);
  }

  // ─── Kill with tap-to-confirm ─────────────────────────
  async function removeSession(name: string, e?: Event) {
    e?.stopPropagation();
    if (confirmKill !== name) {
      confirmKill = name;
      setTimeout(() => { if (confirmKill === name) confirmKill = null; }, 3000);
      return;
    }
    confirmKill = null;
    try {
      await killSession(name);
      await refresh();
    } catch (err) {
      error = (err as Error).message;
    }
  }
  async function removeWindow(target: string, session: string, e?: Event) {
    e?.stopPropagation();
    if (confirmKillWindow !== target) {
      confirmKillWindow = target;
      setTimeout(() => { if (confirmKillWindow === target) confirmKillWindow = null; }, 3000);
      return;
    }
    confirmKillWindow = null;
    try {
      await killWindow(target);
      panes[session] = await listPanes(session);
      if ((panes[session] || []).length === 0) await refresh();
    } catch (err) { error = (err as Error).message; }
  }

  // ─── Create session ───────────────────────────────────
  async function createSession() {
    if (!newName.trim()) return;
    try {
      await newSession(newName.trim(), newPath.trim() || undefined, newCmd.trim() || undefined);
      const name = newName.trim();
      newName = ''; newPath = ''; newCmd = ''; showNew = false;
      const ps = await listPanes(name);
      if (ps.length) {
        const p = ps[0]!;
        openTerminal(name, `${p.session}:${p.window}.${p.pane}`, p.current_command);
      }
      await refresh();
    } catch (e) { error = (e as Error).message; }
  }

  // ─── Folder picker ────────────────────────────────────
  async function openPicker() {
    showPicker = true;
    await loadPicker(newPath || '~');
  }
  async function loadPicker(path: string) {
    try {
      const r = await fsList(path, false);
      pickerPath = path;
      pickerEntries = r.entries.filter((e: DirEntry) => e.type === 'dir').sort((a: DirEntry, b: DirEntry) => a.name.localeCompare(b.name));
    } catch {}
  }
  function pickerUp() {
    const parent = pickerPath.replace(/\/[^/]+\/?$/, '') || '/';
    loadPicker(parent);
  }
  function pickerSelect() { newPath = pickerPath; showPicker = false; }
  let pickerBreadcrumbs = $derived.by(() => {
    if (!pickerPath) return [];
    const parts = pickerPath.split('/').filter(Boolean);
    return parts.map((name, i) => ({ name, path: '/' + parts.slice(0, i + 1).join('/') }));
  });
  $effect(() => {
    pickerPath;
    setTimeout(() => { if (pickerPathEl) pickerPathEl.scrollLeft = pickerPathEl.scrollWidth; }, 0);
  });

  // ─── Derived: filtered + MRU chips ────────────────────
  // MRU chips: up to N AI sessions (Kiro/Claude/…) by last_opened,
  // INCLUDING the currently-active one — the active session is pinned
  // first and highlighted so the user can see "I am here". Tapping it
  // returns to the Terminal view. AI-only filter: plain zsh/node/vim
  // sessions clutter the fast-switch surface and are still reachable
  // via the search bar or the full list.
  const MRU_CHIP_MAX = 5;
  let mruChips = $derived.by(() => {
    const activeName = activeTarget.split(':')[0];
    const eligible = sessions.filter(s =>
      !isTeamSession(s.name) &&            // team sessions live in their own group + the Team tab
      sessionHasAgent(panes[s.name]) &&
      (s.name === activeName || s.last_opened)
    );
    const active = eligible.find(s => s.name === activeName);
    const rest = eligible
      .filter(s => s.name !== activeName)
      .slice(0, MRU_CHIP_MAX - (active ? 1 : 0));
    return active ? [active, ...rest] : rest;
  });

  // Filter list by query. Matches against: session name, window name,
  // current_command, current_path, AI tag. Case-insensitive.
  function sessionMatches(s: TmuxSession, q: string) {
    if (!q) return true;
    const ql = q.toLowerCase();
    if (s.name.toLowerCase().includes(ql)) return true;
    const ps = panes[s.name] || [];
    return ps.some(p =>
      (p.current_command || '').toLowerCase().includes(ql) ||
      (p.window_name || '').toLowerCase().includes(ql) ||
      (p.current_path || '').toLowerCase().includes(ql) ||
      (paneAgent(p)?.tag || '').toLowerCase().includes(ql)
    );
  }
  function paneMatches(p: TmuxPane, q: string) {
    if (!q) return true;
    const ql = q.toLowerCase();
    return (
      (p.current_command || '').toLowerCase().includes(ql) ||
      (p.window_name || '').toLowerCase().includes(ql) ||
      (p.current_path || '').toLowerCase().includes(ql) ||
      (paneAgent(p)?.tag || '').toLowerCase().includes(ql)
    );
  }
  let filtered = $derived(sessions.filter(s => sessionMatches(s, query)));

  // Split the (filtered) list into team-mode sessions and the rest. When team
  // sessions are present we render the two as labelled groups; otherwise the
  // list stays a flat, headerless list exactly as before.
  let teamGroup = $derived(filtered.filter(s => isTeamSession(s.name)));
  let regularGroup = $derived(filtered.filter(s => !isTeamSession(s.name)));
  let grouped = $derived(teamGroup.length > 0);

  // Auto-expand during search so panes matching the query are visible.
  let isSearching = $derived(!!query.trim());

  // When the user opens the search box, focus the input (next microtask
  // so the input is rendered first).
  $effect(() => {
    if (searchOpen && searchInputEl) {
      setTimeout(() => searchInputEl?.focus(), 0);
    }
  });
  function closeSearch() {
    query = '';
    searchOpen = false;
  }

  function scrollIntoView(el: HTMLElement) {
    setTimeout(() => el.scrollIntoView({ behavior: 'smooth', block: 'end' }), 50);
  }

  // Svelte action: after the element mounts, scroll its horizontal content
  // to the right end so the most-informative tail of a long path is
  // visible without the user having to scroll. Re-runs when content changes.
  function scrollEndIntoView(el: HTMLElement) {
    const update = () => { if (el) el.scrollLeft = el.scrollWidth; };
    update();
    return { update };
  }
</script>

<div class="sessions" bind:this={sessionsEl}>
  <!--
    Top row: chip strip on the left, search button on the right.
    Tapping search swaps this row into full-width input mode; closing it
    restores chips. The chip strip itself already hides while searching,
    so there's no fight for horizontal space.
  -->
  <div class="top-row">
    {#if searchOpen}
      <div class="search-bar">
        <Icon name="search" size={14} />
        <input
          bind:this={searchInputEl}
          type="text"
          bind:value={query}
          placeholder={t('searchSessions')}
          autocapitalize="off"
          autocomplete="off"
          spellcheck="false"
          onkeydown={(e) => { if (e.key === 'Escape') closeSearch(); }}
        />
        <button class="icon-btn" onclick={closeSearch} aria-label="Close search">
          <Icon name="x" size={12} />
        </button>
      </div>
    {:else}
      {#if mruChips.length > 0}
        <div class="chips-row">
          {#each mruChips as s}
            {@const sum = sessionSummary(s)}
            {@const isActive = activeTarget.startsWith(s.name + ':')}
            {@const chipNotice = sessionHasNotification(s.name)}
            {@const chipUrgent = agentNotifications.unread.some(item => item.session === s.name && item.kind !== 'completed')}
            <AgentChip
              attention={chipNotice}
              urgent={chipUrgent}
              agent={AGENT_BY_TAG.get(sum.ai)}
              agents={sum.agents}
              label={s.name}
              variant={isActive ? 'active' : 'default'}
              onclick={() => chipOpen(s)}
            />
          {/each}
        </div>
      {:else}
        <!-- Keep the row's height stable when there are no chips. Uses its own
             modifier (NOT `empty`, which is the tall centered "no matches"
             message style and would add 32px padding here). -->
        <div class="chips-row chips-empty"></div>
      {/if}
      <button
        class="icon-btn search-btn"
        onclick={() => (searchOpen = true)}
        aria-label={t('searchSessions')}
      >
        <Icon name="search" size={14} />
      </button>
    {/if}
  </div>

  <div class="content">
    {#if error}
      <div class="error">{error}</div>
    {/if}

  <!-- Session row template — shared by both groups (team + regular). The
       `team` flag flips the leading icon, the displayed name (room vs the raw
       tmm-team-* session), the trailing affordance (chat hint vs kill), and
       disables pane expansion (a team row always opens the chat). -->
  {#snippet sessionItem(s: TmuxSession)}
    {@const team = isTeamSession(s.name)}
    {@const sum = sessionSummary(s)}
    {@const isActive = activeTarget.startsWith(s.name + ':')}
    {@const hasNotice = sessionHasNotification(s.name)}
    {@const urgentNotice = agentNotifications.unread.some(item => item.session === s.name && item.kind !== 'completed')}
    {@const isExpanded = !team && ((isSearching && s.windows > 1) || expanded[s.name])}
    {@const ps = panes[s.name] || []}
    {@const visiblePanes = isSearching ? ps.filter(p => paneMatches(p, query)) : ps}
    <div class="session" class:active={isActive} class:team-session={team}>
      <div
        class="session-row"
        role="button"
        tabindex="0"
        onclick={() => activateSession(s)}
        onkeydown={(e) => e.key === 'Enter' && activateSession(s)}
      >
        <span class="dot" class:attached={s.attached}></span>
        <span class="name" class:name-grow={team} title={team ? s.name : null}>{team ? teamLabel(s.name) : s.name}</span>
        <!-- Team rows show only the title. Regular rows keep a short cmd/AI
             marker, but NOT the cwd path — in the cramped row it was squeezed
             to the point of being unreadable. The full path lives on the
             window rows below (right-aligned, scrollable). -->
        {#if !team}
          <span class="meta">
            {#if sum.agents.length}
              <span class="session-agents" aria-label={sum.agents.map(item => `${item.agent.tag}${item.count > 1 ? ` ×${item.count}` : ''}`).join(', ')}>
                {#each sum.agents as item (item.agent.tag)}
                  <span class="session-agent-icon">
                    <img class="ai-icon" src={item.agent.icon} alt={item.agent.tag} />
                    {#if item.count > 1}<span class="agent-count">×{item.count}</span>{/if}
                  </span>
                {/each}
              </span>
            {:else if sum.cmd}
              <span class="cmd">{sum.cmd}</span>
            {/if}
          </span>
        {/if}
        <span class="trailing">
          {#if hasNotice}<span class="attention-dot" aria-label="Agent needs attention"></span>{/if}
          {#if team}
            <span class="go-chat" aria-hidden="true"><Icon name="chat" size={13} /></span>
          {:else}
            {#if s.last_opened}
              <span class="ago">{relTime(s.last_opened)}</span>
            {/if}
            {#if s.windows > 1}
              <span class="w-badge">{s.windows}w</span>
            {/if}
            <button
              class="kill"
              class:confirm={confirmKill === s.name}
              onclick={(e) => removeSession(s.name, e)}
              aria-label="Kill session"
            >
              {#if confirmKill === s.name}
                <span class="kill-text">{t('tapToKill')}</span>
              {:else}
                <Icon name="trash" size={12} />
              {/if}
            </button>
          {/if}
        </span>
      </div>

      {#if isExpanded && visiblePanes.length}
        <div class="pane-list">
          {#each visiblePanes as p}
            {@const pAi = paneAgent(p)?.tag || ''}
            {@const isPaneActive = activeTarget === `${p.session}:${p.window}.${p.pane}`}
            {@const paneNotice = notificationForWindow(p.session, p.window)}
            <div class="pane-row" class:active-pane={isPaneActive}>
              <button class="pane" onclick={() => openPane(s, p)}>
                <span class="pane-id">{p.window}.{p.pane}</span>
                <span class="pane-cmd">{p.current_command}</span>
                {#if p.current_path}
                  <span class="pane-cwd" use:scrollEndIntoView>{p.current_path}</span>
                {/if}
                {#if paneNotice}<span class="attention-dot" aria-label="Agent needs attention"></span>{/if}
                {#if pAi}
                  <img class="pane-ai-icon" src={aiIcon(pAi)} alt={pAi} />
                {/if}
              </button>
              <button
                class="pane-kill"
                class:confirm={confirmKillWindow === `${s.name}:${p.window}`}
                onclick={(e) => removeWindow(`${s.name}:${p.window}`, s.name, e)}
              >
                {#if confirmKillWindow === `${s.name}:${p.window}`}
                  <span class="kill-text">{t('del')}</span>
                {:else}
                  <Icon name="trash" size={11} />
                {/if}
              </button>
            </div>
          {/each}
          <button class="pane-add" onclick={async () => {
            try {
              await newWindow(s.name);
              const ps2 = await listPanes(s.name);
              panes[s.name] = ps2;
              const p = ps2[ps2.length - 1];
              if (p) openTerminal(s.name, `${p.session}:${p.window}.${p.pane}`, p.current_command);
            } catch (e) { error = (e as Error).message; }
          }}>
            <Icon name="plus" size={12} /> {t('window')}
          </button>
        </div>
      {/if}
    </div>
  {/snippet}

  <!-- Session list. When team sessions are present we split into two labelled
       groups (Teams first, then Sessions); otherwise it's the flat list. -->
  <div class="list">
    <!-- Declarative projects sit above the raw session list: a project is the
         thing you keep, a session is only its current projection. The section
         hides itself on a server without project support. -->
    <Projects {visible} {openTerminal} />
    {#if grouped}
      <div class="group-label">
        <Icon name="bot" size={12} />
        {t('groupTeams')}
        <span class="group-count">{teamGroup.length}</span>
      </div>
      {#each teamGroup as s (s.name)}
        {@render sessionItem(s)}
      {/each}
      {#if regularGroup.length > 0}
        <div class="group-label">
          <Icon name="terminal" size={12} />
          {t('groupSessions')}
          <span class="group-count">{regularGroup.length}</span>
        </div>
        {#each regularGroup as s (s.name)}
          {@render sessionItem(s)}
        {/each}
      {/if}
    {:else}
      {#each filtered as s (s.name)}
        {@render sessionItem(s)}
      {/each}
    {/if}

    {#if filtered.length === 0}
      <div class="empty">
        {#if isSearching}
          {t('noMatches')} "<span class="empty-q">{query}</span>"
        {:else}
          {t('noSessions')}
        {/if}
      </div>
    {/if}
  </div>

  {#if showNew}
    <div class="new-form" use:scrollIntoView>
      <input type="text" bind:value={newName} placeholder={t('sessionName')} onkeydown={(e) => e.key === 'Enter' && !e.isComposing && e.keyCode !== 229 && createSession()} autocapitalize="off" />
      <div class="cmd-row-new">
        <input type="text" bind:value={newPath} placeholder={t('workingDir')} autocapitalize="off" />
        <button class="preset-btn" onclick={openPicker}><Icon name="folder" size={14} /></button>
      </div>
      {#if showPicker}
        <div class="picker">
          <div class="picker-header">
            <button class="picker-btn" onclick={pickerUp}><Icon name="folder-up" size={13} /></button>
            <div class="picker-path" bind:this={pickerPathEl}>
              <button class="picker-seg" onclick={() => loadPicker('/')}>/</button>
              {#each pickerBreadcrumbs as bc}
                <button class="picker-seg" onclick={() => loadPicker(bc.path)}>{bc.name}</button>
                <span class="picker-sep">/</span>
              {/each}
            </div>
            <button class="picker-btn pick-ok" onclick={pickerSelect}><Icon name="check" size={13} /></button>
          </div>
          <div class="picker-list">
            {#each pickerEntries as e}
              <button class="picker-item" onclick={() => loadPicker(e.path)}>
                <Icon name="folder" size={13} /> {e.name}
              </button>
            {/each}
            {#if !pickerEntries.length}
              <div class="picker-empty">{t('noSubdirs')}</div>
            {/if}
          </div>
        </div>
      {/if}
      <div class="cmd-row-new">
        <input type="text" bind:value={newCmd} placeholder={t('commandOpt')} autocapitalize="off" />
        <button class="preset-btn" class:active={newCmd === 'kiro-cli-chat chat -a'} onclick={() => newCmd = newCmd === 'kiro-cli-chat chat -a' ? '' : 'kiro-cli-chat chat -a'}><img src="/assets/kiro.svg" alt="Kiro" width="16" height="16" /></button>
        <button class="preset-btn" class:active={newCmd === 'claude --dangerously-skip-permissions'} onclick={() => newCmd = newCmd === 'claude --dangerously-skip-permissions' ? '' : 'claude --dangerously-skip-permissions'}><img src="/assets/claude.svg" alt="Claude" width="18" height="18" /></button>
      </div>
      <button class="create-btn" onclick={createSession} disabled={!newName.trim()}>{t('create')}</button>
    </div>
  {/if}

  <div class="bottom-bar">
    <button class="new-btn" onclick={() => showNew = !showNew}>
      <Icon name="plus" size={16} /> {t('newSession')}
    </button>
    <button class="refresh-icon" class:spinning={refreshing} onclick={doRefresh} aria-label="Refresh">
      <Icon name="refresh" size={16} />
    </button>
  </div>
  </div>
</div>

<style>
  .sessions {
    display: flex;
    flex-direction: column;
    flex: 1;
    min-height: 0;
    -webkit-overflow-scrolling: touch;
  }

  /* Scrollable content below the top bar. Padding/gap moved here so the
     top bar can span edge-to-edge at the page top (matching Terminal /
     Files top strips). */
  .content {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 10px 12px 14px;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    -webkit-overflow-scrolling: touch;
  }

  /* Top bar flush against the page top, matches Terminal win-bar and
     Files toolbar height (24 px content + 3 px padding + 1 px border). */
  .top-row {
    display: flex;
    align-items: center;
    gap: 6px;
    min-height: var(--ui-bar-height);
    padding: var(--ui-bar-padding);
    box-sizing: border-box;
    border-bottom: 1px solid var(--border2);
    background: var(--surface);
    flex-shrink: 0;
  }
  .icon-btn {
    flex-shrink: 0;
    width: var(--ui-control-height);
    height: var(--ui-control-height);
    padding: 0;
    border: 1px solid var(--border2);
    border-radius: var(--ui-radius-pill);
    background: var(--input-bg);
    color: var(--text3);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    -webkit-tap-highlight-color: transparent;
    transition: color var(--ui-motion-fast), border-color var(--ui-motion-fast), background var(--ui-motion-fast);
  }
  .icon-btn:active {
    color: var(--accent);
    border-color: var(--accent);
    background: var(--accent-bg);
  }

  /* ─── Search bar (expanded state) ─────────────────── */
  .search-bar {
    flex: 1;
    min-width: 0;
    position: relative;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 0 4px 0 10px;
    height: var(--ui-control-height);
    background: var(--input-bg);
    border: 1px solid var(--border2);
    border-radius: var(--ui-radius-pill);
    color: var(--text3);
    transition: border-color 0.15s ease;
  }
  .search-bar:focus-within {
    border-color: var(--accent);
    color: var(--accent);
  }
  .search-bar input {
    flex: 1;
    min-width: 0;
    border: none;
    outline: none;
    background: transparent;
    padding: 0;
    font-size: 13px;
    color: var(--text);
    -webkit-appearance: none;
    appearance: none;
    line-height: 1;
  }
  .search-bar input::placeholder { color: var(--text3); }
  .search-bar .icon-btn {
    width: 20px;
    height: 20px;
    border: none;
    background: transparent;
  }
  .search-bar .icon-btn:active {
    background: var(--surface2);
    color: var(--text);
  }

  /* ─── MRU chips ──────────────────────────────────────── */
  .chips-row {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    overflow-x: auto;
    scrollbar-width: none;
    -webkit-overflow-scrolling: touch;
  }
  .chips-row::-webkit-scrollbar { display: none; }
  .chips-row.chips-empty { min-height: 24px; }

  /* ─── Session list ───────────────────────────────────── */
  .list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .session {
    border: 1px solid transparent;
    border-radius: var(--ui-radius-panel);
    background: transparent;
    overflow: hidden;
    transition: border-color var(--ui-motion-fast), background var(--ui-motion-fast);
  }
  .session:active { transform: scale(0.996); }
  .session.active {
    border-color: var(--accent);
    background: var(--accent-bg);
  }

  /* ─── Group headers (team vs regular sessions) ─────── */
  /* Both headers share this one style. Accent-highlighted text + icon (the
     Icon inherits the colour via currentColor) so the two section dividers
     read identically and stand out from the rows. */
  .group-label {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 8px 8px 2px;
    color: var(--accent);
    font-size: 11px;
    font-weight: 700;
    text-transform: uppercase;
    letter-spacing: 0.6px;
  }
  /* Tighten the very first header against the top of the list. */
  .group-label:first-child { padding-top: 2px; }
  .group-count {
    font-weight: 600;
    color: var(--accent);
    background: var(--accent-bg);
    border-radius: 999px;
    padding: 0 6px;
    font-size: 10px;
    letter-spacing: 0;
    font-variant-numeric: tabular-nums;
  }
  /* Team rows reuse the same status dot + title style as regular rows (no
     leading bot glyph); only the trailing chat glyph hints that a tap opens
     the conversation. The "Teams" group header is what marks the section. */
  .go-chat {
    display: inline-flex;
    align-items: center;
    padding: 6px;
    color: var(--text3);
  }
  .session.team-session .session-row:hover .go-chat { color: var(--accent); }

  .session-row {
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 10px 10px 10px 12px;
    cursor: pointer;
    min-width: 0;
    -webkit-tap-highlight-color: transparent;
  }
  .session-row:hover { background: var(--surface2); }
  .session.active .session-row:hover { background: transparent; }

  .dot {
    width: 7px; height: 7px;
    border-radius: 50%;
    background: var(--text3);
    flex-shrink: 0;
    transition: all 0.2s ease;
  }
  .dot.attached {
    background: var(--accent);
    box-shadow: 0 0 6px var(--accent-glow);
  }
  .attention-dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--danger);
    flex-shrink: 0;
  }

  .name {
    font-weight: 600;
    font-size: 14px;
    color: var(--text);
    white-space: nowrap;
    flex-shrink: 0;
    max-width: 40%;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  /* Team rows carry no meta sub-text, so let the title use the full row width
     (the 40% cap would otherwise leave an odd empty gap and clip the name). */
  .name.name-grow { max-width: none; flex: 1; min-width: 0; }

  .meta {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: 6px;
    color: var(--text3);
    font-size: 12px;
    white-space: nowrap;
    overflow: hidden;
  }
  .meta .ai-icon {
    width: 13px; height: 13px;
    flex-shrink: 0;
  }
  .session-agents {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    padding: 2px 3px 2px 1px;
    overflow: visible;
  }
  .session-agent-icon {
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: 1px;
    flex-shrink: 0;
  }
  .agent-count {
    display: inline-flex;
    align-items: center;
    color: var(--text3);
    font-size: 8px;
    font-weight: 600;
    line-height: 1;
    font-variant-numeric: tabular-nums;
  }
  .meta .cmd {
    font-family: var(--font-mono);
    font-size: 11px;
    color: var(--text2);
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }

  .trailing {
    display: flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
  }
  .ago {
    font-size: 10px;
    color: var(--text3);
    font-variant-numeric: tabular-nums;
  }
  .w-badge {
    font-size: 10px;
    font-weight: 600;
    color: var(--text2);
    background: var(--surface2);
    padding: 1px 6px;
    border-radius: 5px;
    font-variant-numeric: tabular-nums;
  }

  .kill {
    padding: 6px;
    background: transparent;
    border: none;
    color: var(--text3);
    cursor: pointer;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: color 0.15s ease;
    -webkit-tap-highlight-color: transparent;
  }
  .kill:active, .kill.confirm { color: var(--danger); }
  .kill-text { font-size: 10px; font-weight: 600; white-space: nowrap; }

  /* ─── Pane list (expanded) ──────────────────────────── */
  .pane-list {
    margin: 2px 0 6px;
    border-top: 1px solid var(--border2);
    display: flex;
    flex-direction: column;
  }
  .pane-row {
    display: flex;
    align-items: stretch;
    border-radius: 8px;
    transition: background 0.15s ease;
  }
  .pane-row.active-pane { background: var(--accent-bg); }

  .pane {
    display: flex;
    align-items: center;
    gap: 8px;
    flex: 1;
    /* left padding = session-row's 12 + 8 indent visually subordinates panes
       to their session without wasting horizontal space */
    padding: 7px 10px 7px 20px;
    background: none;
    border: none;
    color: var(--text);
    cursor: pointer;
    text-align: left;
    font-size: 13px;
    min-width: 0;
    -webkit-tap-highlight-color: transparent;
  }
  .pane:active { background: var(--surface2); border-radius: 8px; }
  .pane-id {
    font-family: var(--font-mono);
    color: var(--accent);
    font-weight: 500;
    font-size: 11px;
    min-width: 22px;
    flex-shrink: 0;
  }
  .pane-cmd {
    font-family: var(--font-mono);
    color: var(--text2);
    font-size: 12px;
    flex-shrink: 0;
  }
  .pane-cwd {
    display: block;
    color: var(--text3);
    font-size: 11px;
    font-family: var(--font-mono);
    flex: 1;
    min-width: 0;
    white-space: nowrap;
    /* Right-align so the current folder (the informative tail of the path)
       always sits flush against the right edge. Long paths still scroll
       horizontally — scrollEndIntoView parks them at the right end on mount,
       and the user can swipe left to reveal the full path. */
    text-align: right;
    overflow-x: auto;
    overflow-y: hidden;
    scrollbar-width: none;
    -webkit-overflow-scrolling: touch;
    /* Allow horizontal pan gestures here without triggering vertical scroll
       on the parent list. */
    touch-action: pan-x;
  }
  .pane-cwd::-webkit-scrollbar { display: none; }
  .pane-ai-icon {
    width: 13px; height: 13px;
    flex-shrink: 0;
  }
  .pane-kill {
    padding: 6px;
    border: none;
    border-radius: 6px;
    background: transparent;
    color: var(--text3);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: color 0.15s ease;
    -webkit-tap-highlight-color: transparent;
  }
  .pane-kill:active, .pane-kill.confirm { color: var(--danger); }
  .pane-add {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    width: 100%;
    padding: 6px;
    margin-top: 2px;
    border: 1px dashed var(--border2);
    border-radius: 7px;
    background: none;
    color: var(--text3);
    font-size: 11px;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
    transition: all 0.15s ease;
  }
  .pane-add:active {
    color: var(--accent);
    border-color: var(--accent);
    background: var(--accent-bg);
  }

  /* ─── Empty / error ──────────────────────────────────── */
  .empty {
    padding: 32px 12px;
    text-align: center;
    color: var(--text3);
    font-size: 13px;
  }
  .empty-q {
    color: var(--text);
    font-family: var(--font-ui);
  }
  .error {
    color: var(--danger);
    font-size: 13px;
    padding: 10px 14px;
    background: var(--danger-bg);
    border: 1px solid var(--danger);
    border-radius: 10px;
  }

  /* ─── Bottom bar (new / refresh) ───────────────────── */
  .bottom-bar {
    display: flex;
    gap: 8px;
    margin-top: auto;
    padding-top: 4px;
  }
  .new-btn {
    flex: 1;
    height: 36px; padding: 0 12px;
    border: 1px solid var(--border);
    border-radius: 9px;
    background: none;
    color: var(--text2);
    font-size: 13px;
    font-weight: 500;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    -webkit-tap-highlight-color: transparent;
    transition: color 0.15s ease, border-color 0.15s ease;
  }
  .new-btn:active { color: var(--accent); border-color: var(--accent); }
  .refresh-icon {
    width: 36px; height: 36px; flex-shrink: 0;
    border: 1px solid var(--border);
    border-radius: 9px;
    background: none;
    color: var(--text3);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    -webkit-tap-highlight-color: transparent;
    transition: color 0.2s, border-color 0.2s;
  }
  .refresh-icon:active { color: var(--accent); border-color: var(--accent); }
  .refresh-icon.spinning { color: var(--accent); animation: spin 0.6s ease; }
  @keyframes spin { to { transform: rotate(360deg); } }

  /* ─── New session form ────────────────────────────── */
  .new-form {
    display: flex;
    flex-direction: column;
    gap: 8px;
    padding: 12px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 14px;
  }
  .new-form input {
    padding: 10px 12px;
    border: 1px solid var(--border2);
    border-radius: 10px;
    background: var(--input-bg);
    color: var(--text);
    font-size: 14px;
    outline: none;
    -webkit-appearance: none;
    appearance: none;
  }
  .new-form input:focus { border-color: var(--accent); }
  .new-form input::placeholder { color: var(--text3); }
  .cmd-row-new { display: flex; gap: 8px; }
  .cmd-row-new input { flex: 1; min-width: 0; }
  .preset-btn {
    padding: 0 12px;
    border: 1px solid var(--border2);
    border-radius: 10px;
    background: var(--input-bg);
    color: var(--text2);
    font-size: 13px;
    font-weight: 600;
    cursor: pointer;
    white-space: nowrap;
    display: flex;
    align-items: center;
    -webkit-tap-highlight-color: transparent;
  }
  .preset-btn.active {
    background: var(--accent-bg);
    color: var(--accent);
    border-color: var(--accent);
  }

  .picker {
    border: 1px solid var(--border2);
    border-radius: 10px;
    overflow: hidden;
    background: var(--input-bg);
  }
  .picker-header {
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 6px 8px;
    border-bottom: 1px solid var(--border2);
  }
  .picker-path {
    flex: 1;
    display: flex;
    align-items: center;
    gap: 1px;
    overflow-x: auto;
    scrollbar-width: none;
    font-family: var(--font-mono);
    font-size: 12px;
    -webkit-overflow-scrolling: touch;
  }
  .picker-path::-webkit-scrollbar { display: none; }
  .picker-seg {
    padding: 2px 3px;
    border: none;
    background: none;
    color: var(--text2);
    cursor: pointer;
    white-space: nowrap;
    font-size: 12px;
    font-family: inherit;
    -webkit-tap-highlight-color: transparent;
  }
  .picker-seg:last-of-type { color: var(--accent); }
  .picker-seg:active { color: var(--accent); }
  .picker-sep { color: var(--text3); font-size: 11px; }
  .picker-btn {
    padding: 5px;
    border: none;
    border-radius: 6px;
    background: var(--surface2);
    color: var(--text2);
    cursor: pointer;
    display: flex;
    -webkit-tap-highlight-color: transparent;
  }
  .picker-btn:active { color: var(--accent); }
  .pick-ok { background: var(--accent-bg); color: var(--accent); }
  .picker-list { max-height: 180px; overflow-y: auto; -webkit-overflow-scrolling: touch; }
  .picker-item {
    display: flex;
    align-items: center;
    gap: 8px;
    width: 100%;
    padding: 10px 12px;
    border: none;
    border-bottom: 1px solid var(--border2);
    background: none;
    color: var(--accent);
    font-size: 13px;
    cursor: pointer;
    text-align: left;
    -webkit-tap-highlight-color: transparent;
  }
  .picker-item:active { background: var(--accent-bg); }
  .picker-item:last-child { border-bottom: none; }
  .picker-empty { padding: 12px; text-align: center; color: var(--text3); font-size: 12px; }
  .create-btn {
    padding: 10px;
    border: none;
    border-radius: 10px;
    background: var(--accent-bg);
    color: var(--accent);
    font-size: 14px;
    font-weight: 600;
    cursor: pointer;
    -webkit-tap-highlight-color: transparent;
  }
  .create-btn:active:not(:disabled) { opacity: 0.8; }
  .create-btn:disabled { opacity: 0.3; cursor: default; }
</style>
