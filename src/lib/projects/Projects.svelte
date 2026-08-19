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
    projectDown,
    projectList,
    projectUp,
  } from '../core/ws.ts';
  import type { TmuxPane } from '../core/ws.ts';
  import type { ProjectRow } from './projects.ts';
  import { ageLabel, declaredWindowChips, liveWindowChips, shortPath, sortRows } from './projects.ts';
  import { notificationForWindow } from '../core/agent-notifications.svelte.ts';
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';

  let { visible = false, openTerminal, panes = {}, onTracked = () => {}, onReady = () => {}, dense = false }: {
    visible?: boolean;
    openTerminal: (session: string, target: string, command?: string) => void;
    /** Live panes per session, already loaded by the Sessions page. */
    panes?: Record<string, TmuxPane[]>;
    /** Session names that are tracked, so the session list can stop repeating them. */
    onTracked?: (sessions: string[]) => void;
    /** Hands the reload function out, so creating a project refreshes us. */
    onReady?: (reload: () => Promise<void>) => void;
    /** Sidebar mode: rows in the shared side-row language instead of cards
     * (ui-unification "Page skeleton"). The Chat sidebar set that style; the
     * Terminal sidebar has to match it (owner, 2026-08-19). */
    dense?: boolean;
  } = $props();

  let rows = $state<ProjectRow[]>([]);
  let supported = $state(true);
  let error = $state('');
  let busy = $state<Record<string, boolean>>({});
  let confirmRemove = $state<string | null>(null);
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
      onTracked(rows.map((r) => r.project.session));
    } catch (e) {
      const code = (e as { code?: number })?.code;
      // -32601: this server has no project support. Not an error to show.
      if (code === -32601) { supported = false; onTracked([]); return; }
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



</script>

{#if supported && sorted.length > 0}
  <section class="projects" class:dense>
    <div class="group-label">
      {#if dense}
        <!-- A sidebar section header is a LABEL, not a control: the Chat
             sidebar has no chevron to collapse its projects, and a list this
             short never needed one (owner: "projects 上边还是折叠的，这个和
             chat 里不太一样"). -->
        <span class="side-h-inline">{t('projects')}<span class="group-count">{sorted.length}</span></span>
      {:else}
        <button class="group-toggle" onclick={() => collapsed = !collapsed} aria-expanded={!collapsed}>
          <Icon name={collapsed ? 'chevron-right' : 'chevron-down'} size={12} />
          {t('projects')}
          <span class="group-count">{sorted.length}</span>
        </button>
      {/if}
    </div>

    {#if !collapsed}
      {#each sorted as row (row.project.id)}
        {@const chips = chipsFor(row)}
        <div class="proj" class:live={row.live}>
          <div class="proj-top">
            <button class="proj-main" onclick={() => open(row)} title={row.project.path}>
              <span class="dot" class:on={row.live}></span>
              <span class="body">
                <span class="line">
                  <span class="name">{row.project.name}</span>
                  <span class="age">{ageLabel(row.project.last_seen_at ?? row.project.last_up_at)}</span>
                </span>
                {#if !dense}
                  <!-- The path is what tells two same-named folders apart, so a
                       full-page card shows it. A sidebar row is one line like
                       every other sidebar row; the path lives in the row's
                       title instead. -->
                  <span class="line sub">
                    <span class="path">{shortPath(row.project.path)}</span>
                  </span>
                {/if}
              </span>
            </button>
            <div class="acts">
              {#if row.live}
                <button class="act" disabled={busy[row.project.id]} onclick={() => run(row.project.id, () => projectDown(row.project.id))}>{t('projectDown')}</button>
              {:else}
                <button class="act primary" disabled={busy[row.project.id]} onclick={() => run(row.project.id, () => projectUp(row.project.id))}>{t('projectUp')}</button>
              {/if}
              <!-- Two taps, because there is no un-remove in the UI and the
                   server never auto-tracks a removed session again. The session
                   itself is left alone — this forgets the declaration, it does
                   not kill anything. -->
              <button
                class="act icon"
                class:confirm={confirmRemove === row.project.id}
                disabled={busy[row.project.id]}
                aria-label={t('projectArchive')}
                title={t('projectArchive')}
                onclick={() => {
                  if (confirmRemove !== row.project.id) { confirmRemove = row.project.id; return; }
                  confirmRemove = null;
                  void run(row.project.id, () => projectArchive(row.project.id, true));
                }}>
                {#if confirmRemove === row.project.id}
                  <span class="confirm-text">{t('projectArchiveConfirm')}</span>
                {:else}
                  <Icon name="x" size={13} />
                {/if}
              </button>
            </div>
          </div>

          <!-- Windows belong INSIDE the project card: they are what the project
               is made of, not a separate list under it. Each one is tappable —
               jumping to the window you want is the whole point. A down project
               lists what `up` would restore, and the tap brings it up first. -->
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
        </div>

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

  /* ── Sidebar (dense) mode ───────────────────────────────────────────────
     The Chat sidebar defined the house style — bg2 surface, one uppercase
     mono section header, borderless rows that only highlight when active —
     and the Terminal sidebar now shows the SAME component, so the cards
     become rows here instead of a second visual language living next door.
     Card chrome off, section header in the shared .side-h idiom, actions
     revealed on hover so the row is a name and a state at rest. */
  .projects.dense { gap: 1px; margin-bottom: 6px; }
  .projects.dense .group-label { font-size: var(--fs-meta); letter-spacing: 0.1em; padding: 8px 6px 4px; }
  .projects.dense .group-count { background: none; color: var(--text3); padding: 0; }
  .projects.dense .proj {
    background: none; border: none; border-radius: 9px; padding: 2px 4px; gap: 2px;
  }
  .projects.dense .proj:hover { background: var(--surface); }
  .projects.dense .proj.live { border: none; }
  .projects.dense .proj-main { padding: 4px 4px; }
  .projects.dense .age, .projects.dense .path { color: var(--text3); }
  /* At rest a row shows what it IS; what you can DO to it appears on hover
     (or focus, for keyboards) — the same restraint the Chat rows have. */
  .projects.dense .acts { opacity: 0; transition: opacity var(--t-fast) ease; }
  .projects.dense .proj:hover .acts,
  .projects.dense .proj:focus-within .acts { opacity: 1; }
  .projects.dense .side-h-inline {
    display: inline-flex; align-items: baseline; gap: 6px;
    font-family: ui-monospace, "SF Mono", Menlo, monospace;
  }
  /* Windows stay — they are what the project is made of, and picking one is
     why this sidebar exists — but as quiet text under the row, not a tray of
     bordered pills (that tray is what still read as a "card"). */
  .projects.dense .wins { padding: 0 4px 3px 18px; gap: 2px 10px; }
  .projects.dense .win {
    background: none; border: none; border-radius: 6px; padding: 2px 5px;
    color: var(--text3); font-size: var(--fs-meta);
  }
  .projects.dense .win:hover { background: var(--surface2); color: var(--text); }
  .projects.dense .win-name { max-width: 16ch; }

  .proj {
    display: flex; flex-direction: column; gap: 6px;
    background: var(--surface); border: 1px solid var(--border);
    border-radius: var(--ui-radius-panel, 8px); padding: 7px 8px;
  }
  .proj.live { border-color: var(--accent-bg); }
  .proj-top { display: flex; align-items: stretch; gap: 6px; }
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

  /* Window row: one tappable button per window, inside the project card and
     indented to line up under the project name (past the status dot). */
  .wins {
    display: flex; flex-wrap: wrap; gap: 4px;
    padding-left: 15px;
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

  .act.icon.confirm {
    color: var(--danger); border-color: var(--danger);
    padding: 0 8px;
  }
  .confirm-text { font-family: var(--font-ui); font-size: 10.5px; white-space: nowrap; }

  .err {
    color: var(--danger); background: var(--danger-bg);
    border-radius: 6px; padding: 5px 8px; font-size: 11px;
  }
</style>
