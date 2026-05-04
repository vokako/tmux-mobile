<script>
  import { listSessions, listPanes, newSession, killSession, newWindow, killWindow, fsList } from './ws.js';
  import Icon from './Icon.svelte';
  import { t } from './i18n.svelte.js';

  let { openTerminal, activeTarget = '', visible = false } = $props();

  const isMobile = 'ontouchstart' in window || navigator.maxTouchPoints > 0;

  // ─── State ─────────────────────────────────────────────
  let sessions = $state([]);
  let panes = $state({});           // session name → TmuxPane[]
  let expanded = $state({});        // session name → bool (manually opened in multi-window view)
  let error = $state('');
  let query = $state('');
  let sessionsEl;

  // New session form
  let showNew = $state(false);
  let newName = $state('');
  let newPath = $state('');
  let newCmd = $state('');

  // Folder picker (inside new form)
  let showPicker = $state(false);
  let pickerPath = $state('');
  let pickerEntries = $state([]);
  let pickerPathEl = $state(null);

  // Confirm-to-kill gates
  let confirmKill = $state(null);       // session name
  let confirmKillWindow = $state(null); // "session:window"

  // Pull-to-refresh
  let pullStartY = 0;
  let pullDist = $state(0);
  let pulling = $state(false);
  let refreshing = $state(false);
  let refreshDone = $state(false);
  let canPull = false;

  // ─── Helpers ───────────────────────────────────────────
  function aiTag(cmd) {
    if (!cmd) return '';
    if (/kiro/i.test(cmd)) return 'Kiro';
    if (/claude/i.test(cmd)) return 'Claude';
    if (/openclaw/i.test(cmd)) return 'OpenClaw';
    return '';
  }
  function aiIcon(tag) {
    if (tag === 'Kiro') return '/assets/kiro.svg';
    if (tag === 'Claude') return '/assets/claude.svg';
    if (tag === 'OpenClaw') return '/assets/openclaw.svg';
    return '';
  }
  // Trailing segment of a path, keeping `~` visible. E.g.:
  //   /Users/clawd/work/proj     → proj
  //   ~/work/project/260226_x    → 260226_x
  //   ~                          → ~
  function cwdShort(p) {
    if (!p) return '';
    if (p === '~' || p === '/') return p;
    const clean = p.replace(/\/$/, '');
    const parts = clean.split('/');
    return parts[parts.length - 1] || clean;
  }
  function relTime(unixSec) {
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
  function sessionSummary(s) {
    const ps = panes[s.name];
    if (!ps || !ps.length) return { ai: '', cmd: '', cwd: '', count: s.windows };
    // Prefer pane that matches activeTarget; else first with AI tag; else first.
    const act = ps.find(p => activeTarget === `${p.session}:${p.window}.${p.pane}`);
    const tagged = ps.find(p => aiTag(p.current_command + ' ' + (p.pane_title || '')));
    const p = act || tagged || ps[0];
    const rawCmd = (p.current_command || '') + ' ' + ((p.pane_title || '').split(/\s/)[0] || '');
    const ai = aiTag(rawCmd);
    return {
      ai,
      cmd: p.current_command || '',
      cwd: cwdShort(p.current_path),
      count: s.windows,
      pane: p,
    };
  }

  // ─── Data loading ──────────────────────────────────────
  $effect(() => { if (visible) refresh(); });

  async function refresh() {
    try {
      const list = await listSessions();
      const activeSession = activeTarget.split(':')[0];
      sessions = list.sort((a, b) => {
        if (a.name === activeSession) return -1;
        if (b.name === activeSession) return 1;
        const la = a.last_opened || 0, lb = b.last_opened || 0;
        if (la !== lb) return lb - la;
        return 0;
      });
      error = '';
      // Eagerly load panes for every session — we need them for inline summary
      // (AI tag, cwd, cmd). Cheap: one list-panes call per session.
      await Promise.all(sessions.map(async s => {
        try { panes[s.name] = await listPanes(s.name); }
        catch {}
      }));
    } catch (e) {
      error = e.message;
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
  function activateSession(s) {
    const ps = panes[s.name] || [];
    if (s.windows > 1 && ps.length > 1) {
      expanded[s.name] = !expanded[s.name];
      return;
    }
    const p = ps[0];
    if (!p) return;
    openTerminal(s.name, `${p.session}:${p.window}.${p.pane}`, p.current_command);
  }
  function openPane(s, p) {
    openTerminal(s.name, `${p.session}:${p.window}.${p.pane}`, p.current_command);
  }

  // ─── Kill with tap-to-confirm ─────────────────────────
  async function removeSession(name, e) {
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
      error = err.message;
    }
  }
  async function removeWindow(target, session, e) {
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
    } catch (err) { error = err.message; }
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
        const p = ps[0];
        openTerminal(name, `${p.session}:${p.window}.${p.pane}`, p.current_command);
      }
      await refresh();
    } catch (e) { error = e.message; }
  }

  // ─── Folder picker ────────────────────────────────────
  async function openPicker() {
    showPicker = true;
    await loadPicker(newPath || '~');
  }
  async function loadPicker(path) {
    try {
      const r = await fsList(path, false);
      pickerPath = path;
      pickerEntries = r.entries.filter(e => e.type === 'dir').sort((a, b) => a.name.localeCompare(b.name));
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
  // MRU chips: top N sessions by last_opened (excluding currently open),
  // used for "quick switch" bar above the list.
  const MRU_CHIP_MAX = 5;
  let mruChips = $derived.by(() => {
    const now = sessions.slice();
    const activeName = activeTarget.split(':')[0];
    return now
      .filter(s => s.last_opened && s.name !== activeName)
      .slice(0, MRU_CHIP_MAX);
  });

  // Filter list by query. Matches against: session name, window name,
  // current_command, current_path, AI tag. Case-insensitive.
  function sessionMatches(s, q) {
    if (!q) return true;
    const ql = q.toLowerCase();
    if (s.name.toLowerCase().includes(ql)) return true;
    const ps = panes[s.name] || [];
    return ps.some(p =>
      (p.current_command || '').toLowerCase().includes(ql) ||
      (p.window_name || '').toLowerCase().includes(ql) ||
      (p.current_path || '').toLowerCase().includes(ql) ||
      aiTag(p.current_command).toLowerCase().includes(ql)
    );
  }
  function paneMatches(p, q) {
    if (!q) return true;
    const ql = q.toLowerCase();
    return (
      (p.current_command || '').toLowerCase().includes(ql) ||
      (p.window_name || '').toLowerCase().includes(ql) ||
      (p.current_path || '').toLowerCase().includes(ql) ||
      aiTag(p.current_command).toLowerCase().includes(ql)
    );
  }
  let filtered = $derived(sessions.filter(s => sessionMatches(s, query)));

  // Auto-expand during search so panes matching the query are visible.
  let isSearching = $derived(!!query.trim());

  // ─── Pull-to-refresh ───────────────────────────────────
  function onPullStart(e) {
    pullStartY = e.touches[0].clientY; pulling = false; pullDist = 0;
    canPull = sessionsEl && sessionsEl.scrollTop <= 0;
  }
  function onPullMove(e) {
    if (!canPull || refreshing) return;
    const dy = e.touches[0].clientY - pullStartY;
    if (dy > 10) { pulling = true; pullDist = Math.min(100, dy * 0.5); }
    else if (dy < -5) { canPull = false; }
  }
  function onPullEnd() {
    if (pulling && pullDist >= 60) {
      refreshing = true;
      pullDist = 60;
      refresh().finally(() => {
        refreshing = false;
        refreshDone = true;
        setTimeout(() => { refreshDone = false; pullDist = 0; }, 600);
      });
    } else {
      pullDist = 0;
    }
    pulling = false;
  }

  function scrollIntoView(el) {
    setTimeout(() => el.scrollIntoView({ behavior: 'smooth', block: 'end' }), 50);
  }
</script>

<div class="sessions" bind:this={sessionsEl} ontouchstart={onPullStart} ontouchmove={onPullMove} ontouchend={onPullEnd}>
  {#if pullDist > 0}
    <div class="pull-indicator" style="height:{pullDist}px">
      {#if refreshDone}
        <svg class="pull-done" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round"><polyline points="20 6 9 17 4 12"/></svg>
      {:else}
        <svg class="pull-arrow" class:pull-spin={refreshing} style="transform:rotate({refreshing ? 0 : Math.min(pullDist / 60 * 180, 180)}deg);opacity:{Math.min(pullDist / 30, 1)}" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
          <polyline points="23 4 23 10 17 10"/><polyline points="1 20 1 14 7 14"/><path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"/>
        </svg>
      {/if}
    </div>
  {/if}

  <!-- Search bar -->
  <div class="search-bar">
    <Icon name="search" size={14} />
    <input
      type="text"
      bind:value={query}
      placeholder={t('searchSessions')}
      autocapitalize="off"
      autocomplete="off"
      spellcheck="false"
    />
    {#if query}
      <button class="search-clear" onclick={() => query = ''} aria-label="Clear">
        <Icon name="x" size={12} />
      </button>
    {/if}
  </div>

  <!-- MRU chips (hidden while searching to keep focus on results) -->
  {#if mruChips.length > 0 && !isSearching}
    <div class="chips-row">
      {#each mruChips as s}
        {@const sum = sessionSummary(s)}
        <button class="chip" onclick={() => activateSession(s)}>
          {#if sum.ai}
            <img class="chip-ai" class:claude={sum.ai === 'Claude'} src={aiIcon(sum.ai)} alt={sum.ai} />
          {:else}
            <span class="chip-dot" class:attached={s.attached}></span>
          {/if}
          <span class="chip-name">{s.name}</span>
        </button>
      {/each}
    </div>
  {/if}

  {#if error}
    <div class="error">{error}</div>
  {/if}

  <!-- Session list -->
  <div class="list">
    {#each filtered as s (s.name)}
      {@const sum = sessionSummary(s)}
      {@const isActive = activeTarget.startsWith(s.name + ':')}
      {@const isExpanded = (isSearching && s.windows > 1) || expanded[s.name]}
      {@const ps = panes[s.name] || []}
      {@const visiblePanes = isSearching ? ps.filter(p => paneMatches(p, query)) : ps}
      <div class="session" class:active={isActive}>
        <div
          class="session-row"
          role="button"
          tabindex="0"
          onclick={() => activateSession(s)}
          onkeydown={(e) => e.key === 'Enter' && activateSession(s)}
        >
          <span class="dot" class:attached={s.attached}></span>
          <span class="name">{s.name}</span>
          <span class="meta">
            {#if sum.ai}
              <img class="ai-icon" class:claude={sum.ai === 'Claude'} src={aiIcon(sum.ai)} alt={sum.ai} />
            {:else if sum.cmd}
              <span class="cmd">{sum.cmd}</span>
            {/if}
            {#if sum.cwd}
              <span class="cwd">~/{sum.cwd}</span>
            {/if}
          </span>
          <span class="trailing">
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
                <Icon name="x" size={12} />
              {/if}
            </button>
          </span>
        </div>

        {#if isExpanded && visiblePanes.length}
          <div class="pane-list">
            {#each visiblePanes as p}
              {@const pAi = aiTag(p.current_command + ' ' + (p.pane_title || ''))}
              {@const isPaneActive = activeTarget === `${p.session}:${p.window}.${p.pane}`}
              <div class="pane-row" class:active-pane={isPaneActive}>
                <button class="pane" onclick={() => openPane(s, p)}>
                  <span class="pane-id">{p.window}.{p.pane}</span>
                  <span class="pane-cmd">{p.current_command}</span>
                  {#if p.current_path}
                    <span class="pane-cwd">{cwdShort(p.current_path)}</span>
                  {/if}
                  {#if pAi}
                    <img class="pane-ai-icon" class:claude={pAi === 'Claude'} src={aiIcon(pAi)} alt={pAi} />
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
                    <Icon name="x" size={10} />
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
              } catch (e) { error = e.message; }
            }}>
              <Icon name="plus" size={12} /> {t('window')}
            </button>
          </div>
        {/if}
      </div>
    {/each}

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

<style>
  .sessions {
    padding: 12px 14px 16px;
    display: flex;
    flex-direction: column;
    gap: 10px;
    overflow-y: auto;
    flex: 1;
    -webkit-overflow-scrolling: touch;
  }

  /* ─── Search bar ─────────────────────────────────────── */
  .search-bar {
    position: relative;
    display: flex;
    align-items: center;
    gap: 8px;
    padding: 0 12px;
    background: var(--input-bg);
    border: 1px solid var(--border2);
    border-radius: 12px;
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
    padding: 10px 0;
    font-size: 14px;
    color: var(--text);
    -webkit-appearance: none;
  }
  .search-bar input::placeholder { color: var(--text3); }
  .search-clear {
    padding: 4px;
    border: none;
    background: transparent;
    color: var(--text3);
    cursor: pointer;
    border-radius: 6px;
    display: flex;
    -webkit-tap-highlight-color: transparent;
  }
  .search-clear:active { color: var(--text); background: var(--surface2); }

  /* ─── MRU chips ──────────────────────────────────────── */
  .chips-row {
    display: flex;
    gap: 6px;
    overflow-x: auto;
    scrollbar-width: none;
    -webkit-overflow-scrolling: touch;
    padding: 2px 0;
    margin: -2px -14px 0;
    padding-left: 14px;
    padding-right: 14px;
  }
  .chips-row::-webkit-scrollbar { display: none; }
  .chip {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    padding: 6px 10px 6px 8px;
    border: 1px solid var(--border2);
    border-radius: 999px;
    background: var(--input-bg);
    color: var(--text2);
    font-size: 12px;
    font-weight: 500;
    cursor: pointer;
    flex-shrink: 0;
    white-space: nowrap;
    max-width: 140px;
    -webkit-tap-highlight-color: transparent;
    transition: all 0.15s ease;
  }
  .chip:active {
    background: var(--accent-bg);
    border-color: var(--accent);
    color: var(--accent);
  }
  .chip-ai {
    width: 14px; height: 14px;
    flex-shrink: 0;
  }
  .chip-ai.claude { width: 16px; height: 16px; }
  .chip-dot {
    width: 6px; height: 6px;
    border-radius: 50%;
    background: var(--text3);
    flex-shrink: 0;
  }
  .chip-dot.attached {
    background: var(--accent);
  }
  .chip-name {
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* ─── Session list ───────────────────────────────────── */
  .list {
    display: flex;
    flex-direction: column;
    gap: 4px;
  }
  .session {
    border: 1px solid transparent;
    border-radius: 12px;
    background: transparent;
    overflow: hidden;
    transition: border-color 0.15s ease, background 0.15s ease;
  }
  .session:active { transform: scale(0.996); }
  .session.active {
    border-color: var(--accent);
    background: var(--accent-bg);
  }

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
  .meta .ai-icon.claude { width: 15px; height: 15px; }
  .meta .cmd {
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    font-size: 11px;
    color: var(--text2);
    overflow: hidden;
    text-overflow: ellipsis;
    min-width: 0;
  }
  .meta .cwd {
    color: var(--text3);
    font-size: 11px;
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
    width: 24px; height: 24px;
    background: transparent;
    border: none;
    color: var(--text3);
    cursor: pointer;
    border-radius: 6px;
    display: flex;
    align-items: center;
    justify-content: center;
    transition: all 0.15s ease;
    -webkit-tap-highlight-color: transparent;
  }
  .kill:active { background: var(--danger-bg); color: var(--danger); }
  .kill.confirm {
    background: var(--danger-bg);
    color: var(--danger);
    width: auto;
    padding: 0 8px;
  }
  .kill-text { font-size: 10px; font-weight: 600; white-space: nowrap; }

  /* ─── Pane list (expanded) ──────────────────────────── */
  .pane-list {
    margin: 2px 8px 8px;
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
    padding: 8px 10px 8px 22px;
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
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    color: var(--accent);
    font-weight: 500;
    font-size: 11px;
    min-width: 28px;
    flex-shrink: 0;
  }
  .pane-cmd {
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
    color: var(--text2);
    font-size: 12px;
    flex-shrink: 0;
  }
  .pane-cwd {
    color: var(--text3);
    font-size: 11px;
    flex: 1;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    min-width: 0;
  }
  .pane-ai-icon {
    width: 13px; height: 13px;
    flex-shrink: 0;
  }
  .pane-ai-icon.claude { width: 15px; height: 15px; }
  .pane-kill {
    padding: 8px 10px;
    border: none;
    background: transparent;
    color: var(--text3);
    cursor: pointer;
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
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
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
    padding: 12px;
    border: 1px dashed var(--border);
    border-radius: 14px;
    background: none;
    color: var(--text2);
    font-size: 14px;
    font-weight: 500;
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    -webkit-tap-highlight-color: transparent;
  }
  .new-btn:active { background: var(--accent-bg); color: var(--accent); border-color: var(--accent); }
  .refresh-icon {
    width: 46px;
    border: 1px solid var(--border);
    border-radius: 14px;
    background: none;
    color: var(--text2);
    cursor: pointer;
    display: flex;
    align-items: center;
    justify-content: center;
    -webkit-tap-highlight-color: transparent;
    transition: color 0.2s;
  }
  .refresh-icon:active { color: var(--accent); }
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
    font-family: 'Maple Mono NF CN', 'Maple Mono', 'SF Mono', Menlo, 'Courier New', monospace;
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

  /* ─── Pull-to-refresh ──────────────────────────────── */
  .pull-indicator {
    display: flex;
    align-items: center;
    justify-content: center;
    color: var(--accent);
    flex-shrink: 0;
    overflow: hidden;
    transition: height 0.25s ease;
  }
  .pull-arrow { transition: opacity 0.15s; }
  .pull-done {
    color: var(--status-ok);
    animation: pull-pop 0.3s ease;
  }
  @keyframes pull-pop {
    0% { transform: scale(0.5); opacity: 0; }
    100% { transform: scale(1); opacity: 1; }
  }
  .pull-spin { animation: pull-rotate 0.6s linear infinite; }
  @keyframes pull-rotate { to { transform: rotate(360deg) !important; } }
</style>
