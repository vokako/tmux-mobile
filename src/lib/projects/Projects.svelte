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
    projectAdopt,
    projectArchive,
    projectCreate,
    projectDown,
    projectList,
    projectRestore,
    projectSnapshots,
    projectUp,
  } from '../core/ws.ts';
  import type { ProjectRow, SnapshotMeta } from './projects.ts';
  import { ageLabel, shortPath, sortRows, windowChips } from './projects.ts';
  import { AGENTS } from '../core/agents.ts';
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';

  let { visible = false, openTerminal }: {
    visible?: boolean;
    openTerminal: (session: string, target: string, command?: string) => void;
  } = $props();

  let rows = $state<ProjectRow[]>([]);
  let unmanaged = $state<string[]>([]);
  let supported = $state(true);
  let error = $state('');
  let busy = $state<Record<string, boolean>>({});
  let menuFor = $state<string | null>(null);
  let snapshots = $state<SnapshotMeta[]>([]);
  let addOpen = $state(false);
  let addPath = $state('');
  let collapsed = $state(false);
  let adoptOpen = $state(false);

  const sorted = $derived(sortRows(rows));

  function agentIcon(backend: string | null) {
    if (!backend) return null;
    return AGENTS.find((a) => a.tag.toLowerCase() === backend.toLowerCase()) ?? null;
  }

  async function load() {
    try {
      const res = await projectList();
      rows = res?.projects ?? [];
      unmanaged = res?.unmanaged ?? [];
      supported = true;
      error = '';
    } catch (e) {
      const code = (e as { code?: number })?.code;
      // -32601: this server has no project support. Not an error to show.
      if (code === -32601) { supported = false; return; }
      error = (e as Error)?.message || String(e);
    }
  }

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

  async function open(row: ProjectRow) {
    if (!row.live) {
      await run(row.project.id, () => projectUp(row.project.id));
    }
    const panes = await listPanes(row.project.session).catch(() => []);
    const pane = panes.find((p) => p.active) ?? panes[0];
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

{#if supported && (sorted.length > 0 || unmanaged.length > 0)}
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
        {@const chips = windowChips(row.slots)}
        <div class="proj" class:live={row.live}>
          <button class="proj-main" onclick={() => open(row)} title={row.project.path}>
            <span class="dot" class:on={row.live}></span>
            <span class="body">
              <span class="line">
                <span class="name">{row.project.name}</span>
                {#if row.project.adopted}<span class="tag">{t('projectAdopted')}</span>{/if}
                <span class="age">{ageLabel(row.project.last_seen_at ?? row.project.last_up_at)}</span>
              </span>
              <span class="line sub">
                <span class="path">{shortPath(row.project.path)}</span>
              </span>
              {#if chips.length}
                <span class="chips">
                  {#each chips as chip (chip.name)}
                    {@const icon = agentIcon(chip.agent)}
                    <span class="chip" class:agent={!!icon}>
                      {#if icon}<img src={icon.icon} alt="" width="11" height="11" />{/if}
                      {chip.name}
                    </span>
                  {/each}
                </span>
              {:else}
                <span class="line sub muted">{t('projectNoWindows')}</span>
              {/if}
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

      {#if unmanaged.length > 0}
        <!-- Collapsed by default: every one of these already appears in the
             session list below, so expanding this group is an explicit "I want
             to start tracking one of them" gesture rather than a second copy of
             the list. -->
        <div class="group-label sub-label">
          <button class="group-toggle" onclick={() => adoptOpen = !adoptOpen} aria-expanded={adoptOpen}>
            <Icon name={adoptOpen ? 'chevron-down' : 'chevron-right'} size={12} />
            {t('projectAdoptable')}
            <span class="group-count">{unmanaged.length}</span>
          </button>
        </div>
        {#if adoptOpen}
          {#each unmanaged as session (session)}
            <div class="proj adopt">
              <span class="body">
                <span class="line"><span class="name">{session}</span></span>
              </span>
              <div class="acts">
                <button class="act" disabled={busy[session]} onclick={() => run(session, () => projectAdopt(session))}>
                  {t('projectAdopt')}
                </button>
              </div>
            </div>
          {/each}
        {/if}
      {/if}

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
  .sub-label { margin-top: 6px; }
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
  .tag {
    font-size: 9px; letter-spacing: 0.04em; text-transform: uppercase;
    color: var(--text3); border: 1px solid var(--border); border-radius: 4px; padding: 0 3px;
  }
  .age { margin-left: auto; font-size: 10px; color: var(--text3); }
  .sub { font-family: var(--font-mono); font-size: 11px; color: var(--text2); }
  .muted { color: var(--text3); }
  .path { white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .chips { display: flex; flex-wrap: wrap; gap: 4px; margin-top: 2px; }
  .chip {
    display: inline-flex; align-items: center; gap: 3px;
    font-family: var(--font-mono); font-size: 10px; color: var(--text2);
    background: var(--surface2); border-radius: 4px; padding: 1px 5px;
  }
  .chip.agent { color: var(--text); }

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

  .adopt { align-items: center; }
  .adopt .body { flex: 1; }
  .err {
    color: var(--danger); background: var(--danger-bg);
    border-radius: 6px; padding: 5px 8px; font-size: 11px;
  }
</style>
