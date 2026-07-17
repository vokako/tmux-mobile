<script>
  // Shared pane picker popover: lists every tmux pane grouped by session and
  // calls onPick(pane) when one is chosen. Used by SplitView (assign a cell)
  // and by Terminal's single-pane window switcher (jump to any pane without
  // returning to the Sessions page). The caller owns open/close and positions
  // this via a wrapping element; we just render the panel + backdrop.
  import AgentChip from './AgentChip.svelte';
  import Icon from './Icon.svelte';
  import { t } from './i18n.svelte.js';
  import { listSessionsWithPanes, newWindow } from './ws.js';
  import { paneAgent, paneChipLabel } from './agents.js';
  import { sessionHasNotification, terminalNotificationForWindow } from './agent-notifications.svelte.js';
  // Team sessions (tmm-team-<room>) are grouped apart from regular sessions and
  // labelled by their workspace basename. Shared helpers, gated on the server
  // actually having the team bus — consistent with the Sessions page.
  import { isTeamSession, teamLabel } from './team.svelte.js';

  let {
    currentTarget = '',   // highlight the pane matching this target
    onPick = () => {},    // (pane) — pane has {session, window, pane, current_command, ...}
    onClose = () => {},
    align = 'left',       // 'left' | 'right' — which edge the panel anchors to
  } = $props();

  let loading = $state(true);
  let sessions = $state([]); // [{ name, panes: [pane] }]
  let busySession = $state(''); // session whose "+" is mid-create (disable it)

  let teamSessions = $derived(sessions.filter(s => isTeamSession(s.name)));
  let regularSessions = $derived(sessions.filter(s => !isTeamSession(s.name)));
  let grouped = $derived(teamSessions.length > 0);

  let currentMatch = $derived(/^(.+):(\d+)\./u.exec(currentTarget));
  let currentSession = $derived(currentMatch?.[1] || '');

  async function load() {
    const { sessions: list, panes } = await listSessionsWithPanes();
    const bySession = new Map();
    for (const p of panes) {
      const arr = bySession.get(p.session);
      if (arr) arr.push(p); else bySession.set(p.session, [p]);
    }
    return list
      .map(s => ({ name: s.name, panes: bySession.get(s.name) || [] }))
      .filter(s => s.panes.length);
  }

  // Fetch on mount (the caller only renders this when open).
  $effect(() => {
    let cancelled = false;
    load()
      .then(next => { if (!cancelled) { sessions = next; loading = false; } })
      .catch(() => { if (!cancelled) { sessions = []; loading = false; } });
    return () => { cancelled = true; };
  });

  // Create a new window in `sessionName`, then pick its newest pane — mirrors
  // Terminal's in-bar "+" so opening a fresh window and jumping to it is one
  // tap. We reload to find the just-created window (new-window returns no id).
  async function addWindow(sessionName) {
    if (busySession) return;
    busySession = sessionName;
    try {
      await newWindow(sessionName);
      const next = await load();
      sessions = next;
      const s = next.find(x => x.name === sessionName);
      const fresh = s?.panes?.length
        ? s.panes.reduce((a, b) => (b.window > a.window ? b : a))
        : null;
      if (fresh) onPick(fresh);
    } catch {} finally {
      busySession = '';
    }
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="picker-backdrop" onclick={(e) => { e.stopPropagation(); onClose(); }}></div>
<div class="picker" class:align-right={align === 'right'}>
  {#if loading}
    <div class="picker-empty">…</div>
  {:else if sessions.length === 0}
    <div class="picker-empty">{t('noSessions')}</div>
  {:else if grouped}
    <div class="picker-group">{t('groupTeams')}</div>
    {#each teamSessions as s}
      {@render sessionBlock(s)}
    {/each}
    {#if regularSessions.length > 0}
      <div class="picker-group">{t('groupSessions')}</div>
      {#each regularSessions as s}
        {@render sessionBlock(s)}
      {/each}
    {/if}
  {:else}
    {#each sessions as s}
      {@render sessionBlock(s)}
    {/each}
  {/if}
</div>

{#snippet sessionBlock(s)}
  {@const team = isTeamSession(s.name)}
  <div class="picker-session">
    <span class="picker-session-name" title={team ? s.name : null}>{team ? teamLabel(s.name) : s.name}</span>
    {#if !team && s.name !== currentSession && sessionHasNotification(s.name)}<span class="picker-attention" aria-label="Agent needs attention"></span>{/if}
  </div>
  <div class="picker-panes">
    {#each s.panes as p}
      {@const isCur = currentTarget === `${p.session}:${p.window}.${p.pane}`}
      {@const notice = terminalNotificationForWindow(p.session, p.window)}
      {@const pAgent = paneAgent(p)}
      <AgentChip
        attention={!!notice}
        urgent={notice && notice.kind !== 'completed'}
        agent={pAgent}
        label={paneChipLabel(p, `${p.window}.${p.pane}`)}
        variant={isCur ? 'active' : 'default'}
        title={`${p.session}:${p.window}.${p.pane} · ${p.current_command || p.window_name || ''}`}
        onclick={(e) => { e.stopPropagation(); onPick(p); }}
      />
    {/each}
    <button
      class="picker-add"
      title={t('newWindow')}
      aria-label={t('newWindow')}
      disabled={busySession === s.name}
      onclick={(e) => { e.stopPropagation(); addWindow(s.name); }}
    >
      <Icon name="plus" size={12} />
    </button>
  </div>
{/snippet}

<style>
  .picker-backdrop { position: fixed; inset: 0; z-index: 30; }
  .picker {
    position: absolute;
    top: 36px; left: 6px;
    z-index: 31;
    max-width: calc(100% - 12px);
    min-width: 200px;
    max-height: 60vh;
    overflow-y: auto;
    background: var(--bg);
    border: 1px solid var(--border);
    border-radius: 10px;
    box-shadow: 0 12px 40px rgba(0,0,0,0.4);
    padding: 6px;
  }
  .picker.align-right { left: auto; right: 6px; }
  /* Group dividers and per-session headers share one type treatment (size /
     weight / case / spacing); only colour marks the hierarchy — the
     Teams/Sessions dividers are accent-highlighted, individual session names
     are muted. All session names (team + regular) use the same style. */
  .picker-group {
    padding: 6px 6px 2px;
    font-size: 10px; font-weight: 600; color: var(--accent);
    text-transform: uppercase; letter-spacing: 0.5px;
  }
  .picker-group:first-child { padding-top: 2px; }
  .picker-session {
    display: flex; align-items: center; gap: 6px;
    font-size: 10px; font-weight: 600; color: var(--text3);
    text-transform: uppercase; letter-spacing: 0.5px;
    padding: 6px 6px 2px;
  }
  .picker-session-name { flex: 1; min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .picker-attention {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--danger);
    flex-shrink: 0;
  }
  .picker-add {
    flex-shrink: 0;
    width: 24px; height: 24px; padding: 0;
    border: 1px solid var(--border2); border-radius: 6px;
    background: var(--input-bg); color: var(--text3);
    cursor: pointer; display: flex; align-items: center; justify-content: center;
    -webkit-tap-highlight-color: transparent;
    transition: color 0.15s ease, border-color 0.15s ease, background 0.15s ease;
  }
  .picker-add:active { color: var(--accent); border-color: var(--accent); background: var(--accent-bg); }
  .picker-add:disabled { opacity: 0.4; cursor: default; }
  .picker-panes { display: flex; flex-wrap: wrap; align-items: center; gap: 4px; padding: 0 4px 6px; }
  .picker-empty { padding: 16px; text-align: center; color: var(--text3); font-size: 13px; }
</style>
