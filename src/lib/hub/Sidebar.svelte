<script lang="ts">
  import SideHandle from '../ui/SideHandle.svelte';
  import Icon from '../ui/Icon.svelte';
  import { scrollFade } from '../core/scrollFade.ts';
  import { t } from '../core/i18n.svelte.ts';
  import { projectAgeLabel, type ProjectRow } from '../projects/projects.ts';
  import { rowAgentCounts, rowAgents, type SidebarPane } from './sidebar.ts';
  import { stateDotColor, stateIsLive } from './hub.ts';
  import { anchorOf } from '../ui/placement.ts';
  import { longpress } from '../ui/longpress.ts';
  import { hoverInfo } from '../ui/hover.ts';
  import type { HoverInfo } from '../ui/hover.svelte.ts';
  import { flip } from 'svelte/animate';
  import { moveMs } from '../ui/motion.ts';

  type MenuPosition = { x: number; y: number }
    | { anchor: ReturnType<typeof anchorOf>; trigger: Element };
  interface Props {
    compact?: boolean;
    open?: boolean;
    rows?: ProjectRow[];
    trash?: ProjectRow[];
    rowsBase?: ReadonlySet<string> | null;
    selected?: string;
    panes?: SidebarPane[];
    agentStates?: Record<string, string>;
    talkMap?: Record<string, number>;
    tick?: number;
    unreadCount?: number;
    onselect?: (session: string) => void;
    oncreate?: () => void;
    onclose?: () => void;
    onmenu?: (row: ProjectRow, at: MenuPosition) => void;
    onrestore?: (row: ProjectRow) => void;
    onpurge?: (row: ProjectRow) => void;
  }
  let {
    compact = false, open = false, rows = [], trash = [], rowsBase = null,
    selected = '', panes = [], agentStates = {}, talkMap = {}, tick = Date.now(),
    unreadCount = 0, onselect = () => {}, oncreate = () => {}, onclose = () => {},
    onmenu = () => {}, onrestore = () => {}, onpurge = () => {},
  }: Props = $props();
  let trashOpen = $state(false);

  function rowInfo(row: ProjectRow): HoverInfo {
    const n = rowAgentCounts(row, panes);
    const lines: NonNullable<HoverInfo['lines']> = [{ label: t('hubHoverPath'), value: row.project.path }];
    if (n.live || n.stopped) {
      lines.push({ label: t('hubHoverAgents'), value: t('hubHoverAgentsCount').replace('{live}', String(n.live)).replace('{stopped}', String(n.stopped)), tone: n.live ? 'accent' : undefined });
    }
    const age = projectAgeLabel(row, talkMap, tick);
    if (age) lines.push({ label: t('hoverActivity'), value: age });
    if (row.project.session === selected && unreadCount) lines.push({ label: t('hubHoverUnread'), value: String(unreadCount), tone: 'accent' });
    return { title: row.project.name, lines };
  }
</script>

<!-- The same list is a desktop column and the shared compact sheet. -->
{#if compact && open}
  <div class="side-scrim" onclick={onclose} role="presentation"></div>
{/if}
<aside class="sidebar" class:side-sheet={compact} class:sheet={compact} class:open={compact && open}>
  {#if !compact}<SideHandle />{/if}
  <div class="side-scroll subtle-scroll" use:scrollFade>
    <div class="side-h side-head"><span>{t('hubProjects')}</span></div>
    {#each rows as row (row.project.id)}
      <div class="side-row proj-row" role="group" aria-label={row.project.name} class:open={row.project.session === selected}
        class:appear={!!rowsBase && !rowsBase.has(row.project.id)}
        animate:flip={{ duration: moveMs() }}
        oncontextmenu={(e) => { e.preventDefault(); onmenu(row, { x: e.clientX ?? 0, y: e.clientY ?? 0 }); }}
        use:longpress={{ onlongpress: (pt) => onmenu(row, pt) }}
        use:hoverInfo={() => rowInfo(row)}>
        <button class="proj-pick" onclick={() => onselect(row.project.session)}>
          <span class="dot" class:off={!row.live}></span>
          <span class="p-main">
            <span class="p-top">
              <span class="p-name">{row.project.name}</span>
              <span class="side-age">{projectAgeLabel(row, talkMap, tick)}</span>
            </span>
            {#if rowAgents(row, panes, agentStates).length}
              <span class="side-wins" class:dim={!row.live}>
                {#each rowAgents(row, panes, agentStates) as a (a.name)}
                  <span class="side-win">
                    {#if a.icon}<img src={a.icon} alt="" width="11" height="11" />{/if}
                    <span class="side-win-name">{a.name}</span>
                    {#if a.state}<span class="side-win-dot" class:live-dot={stateIsLive(a.state)} style:background={stateDotColor(a.state)}></span>{/if}
                  </span>
                {/each}
              </span>
            {/if}
          </span>
        </button>
        <button class="icon-btn row-menu" aria-label={t('hubProjectMenu')}
          onclick={(e) => {
            e.stopPropagation();
            onmenu(row, { anchor: anchorOf(e.currentTarget), trigger: e.currentTarget });
          }}>
          <Icon name="dots" size={13} />
        </button>
      </div>
    {/each}
    <button class="side-row add" onclick={oncreate}>
      <Icon name="plus" size={13} />{t('projectNew')}
    </button>
    {#if trash.length}
      <button class="side-row add trash-bar" onclick={() => trashOpen = !trashOpen}>
        <Icon name={trashOpen ? 'chevron-down' : 'trash'} size={13} />
        {t('hubTrashBar').replace('{n}', String(trash.length))}
      </button>
      {#if trashOpen}
        {#each trash as r (r.project.id)}
          <div class="side-row trash-row appear" title={r.project.path}>
            <span class="p-name trash-name">{r.project.name}</span>
            <button class="t-act" title={t('hubRestore')} aria-label={t('hubRestore')}
              onclick={() => onrestore(r)}>
              <Icon name="refresh" size={12} />
            </button>
            <button class="t-act danger" title={t('hubPurge')} aria-label={t('hubPurge')}
              onclick={() => onpurge(r)}>
              <Icon name="trash" size={12} />
            </button>
          </div>
        {/each}
      {/if}
    {/if}
  </div>
</aside>

<style>
  .proj-pick {
    display: flex; align-items: flex-start; gap: 8px; flex: 1; min-width: 0;
    padding: 0; border: 0; background: none; color: inherit; text-align: left;
    font: inherit; cursor: pointer;
  }
  .row-menu { width: 24px; height: 24px; padding: 0; flex: none; align-self: center; color: var(--text3); }

  .sidebar { position: relative; background: var(--bg2); border-right: 1px solid var(--border); display: flex; flex-direction: column; min-height: 0; }
  /* Sheet geometry/motion and project-row atoms remain in app.css. */
  .sidebar.sheet .side-row { min-height: 44px; }
  .side-scroll { flex: 1; overflow-y: auto; padding: 8px; }
  /* The head is a row; the Hub's riding collapse toggle overlays its right end
     (#197). The look (face, size, tracking, padding) stays app.css's .side-h. */
  .side-head { display: flex; align-items: center; justify-content: space-between; gap: 6px; }
  .trash-row { cursor: default; color: var(--text3); }
  .trash-row:hover { background: var(--surface); }
  .trash-name { font-weight: 450; }
  .t-act {
    display: grid; place-items: center; width: 24px; height: 24px; flex: none;
    background: none; border: none; border-radius: var(--ui-radius-control);
    color: var(--text3); cursor: pointer;
  }
  .t-act:hover { background: var(--surface2); color: var(--text); }
  .t-act.danger:hover { color: var(--danger); }
</style>
