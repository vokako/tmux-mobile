<script lang="ts">
  // Projects: the declarative half of the Sessions page.
  //
  // A project is a directory plus the windows it is made of; the tmux session
  // is a projection we can throw away and rebuild (`up` / `down`). Everything
  // here is thin: the server owns the declaration, this file owns the taps.
  //
  // Hidden entirely on a server without project support (mobile builds), the
  // same method-not-found contract the Team tab uses.
  import {
    listPanes,
    projectArchive,
    projectCreate,
    projectDown,
    projectList,
    projectRestore,
    projectSnapshots,
    projectUp,
  } from '../core/ws.ts';
  import type { TmuxPane } from '../core/ws.ts';
  import type { ProjectRow, SnapshotMeta } from './projects.ts';
  import { ageLabel, declaredWindowChips, liveWindowChips, shortPath, sortRows } from './projects.ts';
  import { notificationForWindow } from '../core/agent-notifications.svelte.ts';
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';

  let { visible = false, openTerminal, panes = {}, onTracked = () => {}, onReady = () => {} }: {
    visible?: boolean;
    openTerminal: (session: string, target: string, command?: string) => void;
    /** Live panes per session, already loaded by the Sessions page. */
    panes?: Record<string, TmuxPane[]>;
    /** Session names that are tracked, so the session list can stop repeating them. */
    onTracked?: (sessions: string[], supported: boolean) => void;
    /** Hands the reload function out, so adopting from a session row refreshes us. */
    onReady?: (reload: () => Promise<void>) => void;
  } = $props();

  let rows = $state<ProjectRow[]>([]);
  let supported = $state(true);
  let error = $state('');
  let busy = $state<Record<string, boolean>>({});
  let menuFor = $state<string | null>(null);
  let snapshots = $state<SnapshotMeta[]>([]);
  let addOpen = $state(false);
  let addPath = $state('');
  let collapsed = $state(false);

  const sorted = $derived(sortRows(rows));

  // Live windows beat the declaration while the session exists: they are what
  // you can actually tap into, including a window that has not settled yet.
  function chipsFor(row: ProjectRow) {
    if (!row.live) return declaredWindowChips(row.slots);
    const live = panes[row.project.session] ?? [];
    return live.length ? liveWindowChips(live) : declaredWindowChips(row.slots);
  }

  async function load() {
    try {
      const res = await projectList();
      rows = res?.projects ?? [];
      supported = true;
      error = '';
      onTracked(rows.map((r) => r.project.session), true);
    } catch (e) {
      const code = (e as { code?: number })?.code;
      // -32601: this server has no project support. Not an error to show.
      if (code === -32601) { supported = false; onTracked([], false); return; }
      error = (e as Error)?.message || String(e);
    }
  }

  // Hand our reload out once, so tracking a session from the list below can
  // refresh this section. Inside an effect because reading a prop at component
  // init only captures its initial value.
  $effect(() => {
    onReady(load);
  });

  // Reload whenever the section becomes visible: another client (or the
  // capturer) may have changed the declaration while we were away.
  $effect(() => {
    if (visible) void load();
  });

  async function run(id: string, fn: () => Promise<unknown>) {
    busy = { ...busy, [id]: true };
    error = '';
    try {
      await fn();
      await load();
    } catch (e) {
      error = (e as Error)?.message || String(e);
    } finally {
      const next = { ...busy };
      delete next[id];
      busy = next;
    }
  }

  // A window's live pane target. The panes prop comes from the Sessions page's
  // own poll, so right after `up` it does not know about the new session yet —
  // ask tmux directly in that case.
  async function paneTargets(session: string): Promise<TmuxPane[]> {
    const known = panes[session] ?? [];
    if (known.length) return known;
    return await listPanes(session).catch(() => []);
  }

  async function open(row: ProjectRow) {
    if (!row.live) {
      await run(row.project.id, () => projectUp(row.project.id));
    }
    const live = await paneTargets(row.project.session);
    const pane = live.find((p) => p.active) ?? live[0];
    if (pane) openTerminal(row.project.session, `${pane.session}:${pane.window}.${pane.pane}`, pane.current_command);
  }

  /// Tap a window chip: jump straight into that window. A closed project has to
  /// come up first, and then we look the window up by name.
  async function openWindow(row: ProjectRow, name: string, target: string | null) {
    if (target) {
      openTerminal(row.project.session, target);
      return;
    }
    await run(row.project.id, () => projectUp(row.project.id));
    const live = await paneTargets(row.project.session);
    const pane = live.find((p) => p.window_name === name && p.active)
      ?? live.find((p) => p.window_name === name);
    if (pane) openTerminal(row.project.session, `${pane.session}:${pane.window}.${pane.pane}`, pane.current_command);
  }

  async function toggleMenu(id: string) {
    if (menuFor === id) { menuFor = null; return; }
    menuFor = id;
    snapshots = [];
    try {
      snapshots = (await projectSnapshots(id)) ?? [];
    } catch (e) {
      error = (e as Error)?.message || String(e);
    }
  }

  async function addProject() {
    const path = addPath.trim();
    if (!path) return;
    await run('__add', () => projectCreate(path));
    addPath = '';
    addOpen = false;
  }
</script>

{#if supported && sorted.length > 0}
  <section class="projects">
    <div class="group-label">
      <button class="group-toggle" onclick={() => collapsed = !collapsed} aria-expanded={!collapsed}>
        <Icon name={collapsed ? 'chevron-right' : 'chevron-down'} size={12} />
        {t('projects')}
        <span class="group-count">{sorted.length}</span>
      </button>
      <button class="add-btn" onclick={() => addOpen = !addOpen} aria-label={t('projectAdd')}>
        <Icon name="plus" size={13} />
      </button>
    </div>

    {#if !collapsed}
      {#if addOpen}
        <div class="add-row">
          <input
            class="add-input"
            bind:value={addPath}
            placeholder={t('projectPathPlaceholder')}
            onkeydown={(e) => { if (e.key === 'Enter') void addProject(); }} />
          <button class="act" disabled={!addPath.trim() || busy['__add']} onclick={addProject}>{t('projectAdd')}</button>
        </div>
      {/if}

      {#each sorted as row (row.project.id)}
        {@const chips = chipsFor(row)}
        <div class="proj" class:live={row.live}>
          <button class="proj-main" onclick={() => open(row)} title={row.project.path}>
            <span class="dot" class:on={row.live}></span>
            <span class="body">
              <span class="line">
                <span class="name">{row.project.name}</span>
                <span class="age">{ageLabel(row.project.last_seen_at ?? row.project.last_up_at)}</span>
              </span>
              <span class="line sub">
                <span class="path">{shortPath(row.project.path)}</span>
              </span>
            </span>
          </button>
          <div class="acts">
            {#if row.live}
              <button class="act" disabled={busy[row.project.id]} onclick={() => run(row.project.id, () => projectDown(row.project.id))}>{t('projectDown')}</button>
            {:else}
              <button class="act primary" disabled={busy[row.project.id]} onclick={() => run(row.project.id, () => projectUp(row.project.id))}>{t('projectUp')}</button>
            {/if}
            <button class="act icon" aria-label={t('projectHistory')} onclick={() => toggleMenu(row.project.id)}>
              <Icon name="clock" size={13} />
            </button>
          </div>
        </div>

        <!-- Windows are their own row so each one is tappable: a project is a
             set of windows, and jumping to the one you want is the whole point.
             Down projects list what `up` would restore; the tap brings the
             project up and lands you in that window. -->
        {#if chips.length}
          <div class="wins" class:dim={!row.live}>
            {#each chips as chip (chip.name + (chip.window ?? ''))}
              {@const notice = row.live && chip.window != null
                ? notificationForWindow(row.project.session, chip.window)
                : null}
              <button class="win" onclick={() => openWindow(row, chip.name, chip.target)}>
                {#if chip.agentIcon}<img src={chip.agentIcon} alt={chip.agentTag} width="11" height="11" />{/if}
                <span class="win-name">{chip.name}</span>
                {#if notice}<span class="attention-dot" aria-label={t('newOutput')}></span>{/if}
              </button>
            {/each}
          </div>
        {:else}
          <div class="wins"><span class="win-empty">{t('projectNoWindows')}</span></div>
        {/if}

        {#if menuFor === row.project.id}
          <div class="menu">
            <div class="menu-title">{t('projectHistory')}</div>
            {#if snapshots.length === 0}
              <div class="menu-empty">{t('projectNoHistory')}</div>
            {:else}
              {#each snapshots as snap (snap.id)}
                <button
                  class="snap"
                  onclick={() => run(row.project.id, async () => { await projectRestore(row.project.id, snap.id); menuFor = null; })}>
                  <span class="snap-age">{ageLabel(snap.at)}</span>
                  <span class="snap-windows">{snap.windows.join(' · ') || '—'}</span>
                  <span class="snap-action">{t('projectRestore')}</span>
                </button>
              {/each}
            {/if}
            <button
              class="menu-danger"
              onclick={() => run(row.project.id, async () => { await projectArchive(row.project.id, true); menuFor = null; })}>
              {t('projectArchive')}
            </button>
          </div>
        {/if}
      {/each}

      {#if error}
        <div class="err">{error}</div>
      {/if}
    {/if}
  </section>
{/if}

<style>
  .projects { display: flex; flex-direction: column; gap: 4px; margin-bottom: 10px; }
  .group-label {
    display: flex; align-items: center; gap: 6px;
    font-size: 11px; letter-spacing: 0.06em; text-transform: uppercase;
    color: var(--text3); padding: 2px 2px 2px 4px;
  }
  .group-toggle {
    display: flex; align-items: center; gap: 6px;
    background: none; border: 0; padding: 2px 0; cursor: pointer;
    font: inherit; letter-spacing: inherit; text-transform: inherit; color: inherit;
  }
  .group-count {
    background: var(--surface2); color: var(--text2);
    border-radius: 8px; padding: 0 5px; font-size: 10px; letter-spacing: 0;
  }
  .add-btn {
    margin-left: auto; display: flex; align-items: center;
    background: none; border: 0; color: var(--text3); cursor: pointer; padding: 2px 4px;
  }
  .add-btn:hover { color: var(--accent); }
  .add-row { display: flex; gap: 6px; padding: 0 2px 4px; }
  .add-input {
    flex: 1; min-width: 0; height: var(--ui-control-height, 30px);
    background: var(--input-bg); color: var(--text);
    border: 1px solid var(--border); border-radius: 6px;
    padding: 0 8px; font-family: var(--font-mono); font-size: 12px;
  }

  .proj {
    display: flex; align-items: stretch; gap: 6px;
    background: var(--surface); border: 1px solid var(--border);
    border-radius: var(--ui-radius-panel, 8px); padding: 7px 8px;
  }
  .proj.live { border-color: var(--accent-bg); }
  .proj-main {
    flex: 1; min-width: 0; display: flex; align-items: flex-start; gap: 8px;
    background: none; border: 0; padding: 0; cursor: pointer; text-align: left; color: var(--text);
  }
  .dot {
    flex-shrink: 0; width: 7px; height: 7px; border-radius: 50%; margin-top: 5px;
    background: var(--border2);
  }
  .dot.on { background: var(--accent); box-shadow: 0 0 6px var(--accent-glow); }
  .body { min-width: 0; display: flex; flex-direction: column; gap: 2px; }
  .line { display: flex; align-items: baseline; gap: 6px; min-width: 0; }
  .name { font-size: 13px; font-weight: 600; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .age { margin-left: auto; font-size: 10px; color: var(--text3); }
  .sub { font-family: var(--font-mono); font-size: 11px; color: var(--text2); }
  .path { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }

  /* Window row: one tappable button per window. */
  .wins {
    display: flex; flex-wrap: wrap; gap: 4px;
    padding: 0 8px 2px 25px;   /* aligns under the project name, past the dot */
  }
  .wins.dim { opacity: 0.65; }
  .win {
    display: inline-flex; align-items: center; gap: 4px;
    background: var(--surface2); color: var(--text2);
    border: 1px solid var(--border); border-radius: 5px;
    padding: 2px 7px; cursor: pointer;
    font-family: var(--font-mono); font-size: 10.5px;
    transition: color var(--ui-motion-fast, 0.12s) ease, border-color var(--ui-motion-fast, 0.12s) ease;
  }
  .win:hover { color: var(--text); border-color: var(--border2); }
  .win-name { max-width: 13ch; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .win-empty { font-family: var(--font-mono); font-size: 10.5px; color: var(--text3); }
  .attention-dot {
    width: 6px; height: 6px; border-radius: 50%;
    background: var(--accent); box-shadow: 0 0 5px var(--accent-glow);
  }

  .acts { display: flex; align-items: center; gap: 4px; }
  .act {
    height: var(--ui-control-height, 28px); padding: 0 9px;
    background: var(--surface2); color: var(--text2);
    border: 1px solid var(--border); border-radius: 6px;
    font-family: var(--font-ui); font-size: 11px; cursor: pointer;
    transition: background var(--ui-motion-fast, 0.12s) ease;
  }
  .act:hover:not(:disabled) { color: var(--text); }
  .act:disabled { opacity: 0.5; cursor: default; }
  .act.primary { color: var(--accent); border-color: var(--accent-bg); }
  .act.icon { padding: 0 7px; display: flex; align-items: center; }

  .menu {
    display: flex; flex-direction: column; gap: 2px;
    background: var(--surface2); border: 1px solid var(--border);
    border-radius: var(--ui-radius-panel, 8px); padding: 6px;
  }
  .menu-title, .menu-empty { font-size: 10px; color: var(--text3); padding: 0 2px 2px; }
  .snap {
    display: flex; align-items: center; gap: 8px;
    background: none; border: 0; padding: 4px 2px; cursor: pointer;
    color: var(--text2); font-family: var(--font-mono); font-size: 11px; text-align: left;
  }
  .snap:hover { color: var(--text); }
  .snap-age { flex-shrink: 0; width: 34px; color: var(--text3); }
  .snap-windows { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .snap-action { flex-shrink: 0; font-size: 10px; color: var(--accent); }
  .menu-danger {
    margin-top: 4px; align-self: flex-start;
    background: none; border: 0; padding: 4px 2px; cursor: pointer;
    color: var(--danger); font-family: var(--font-ui); font-size: 11px;
  }

  .err {
    color: var(--danger); background: var(--danger-bg);
    border-radius: 6px; padding: 5px 8px; font-size: 11px;
  }
</style>
