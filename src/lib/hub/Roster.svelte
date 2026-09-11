<script>
  import Icon from '../ui/Icon.svelte';
  import CommandButton from '../ui/CommandButton.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { groupRoster } from './roster.ts';
  import { ALL_TARGET } from './hub-composer.ts';
  import { backendIcon } from '../core/agents.ts';
  import { backendColor, stateDotColor, stateIsLive, stateNeedsYou, chipExtras, fmtElapsed, modelLabel } from './hub.ts';
  import { hoverInfo } from '../ui/hover.ts';
  import { longpress } from '../ui/longpress.ts';
  import { flip } from 'svelte/animate';
  import { moveMs } from '../ui/motion.ts';

  let {
    selected = '', compact = false, managedAgents = [], stopped = [], selectedRow = null,
    recipient = '', unread = new Set(), acting = false, tick = Date.now(),
    roomReady = false, justLoaded = false, rosterBase = null,
    composerText = '', managedNames = [], busyNames = [], interrupting = [],
    stateLabel = (state) => state, stateTone = () => undefined,
    onselect: setRecipient = () => {}, oninterrupt: interrupt = () => {},
    onadd = () => {}, oncontext = () => {},
  } = $props();

  // Same-team cards stay at their first member's position, including nested teams.
  const rosterGroups = $derived(groupRoster(managedAgents));
  const extras = $derived(chipExtras(composerText, recipient, managedNames));
  const allPending = $derived(busyNames.some((name) => interrupting.includes(name)));
  const slotBackend = (name) => (selectedRow?.slots ?? []).find((s) => s.window_name === name)?.command;

  function selectTarget(name) {
    setRecipient(recipient === name ? '' : name);
  }

  function destinationNote(name) {
    if (recipient !== name) return '';
    const destination = name === ALL_TARGET ? t('hubToAllLong') : t('hubToDmLong').replace('{name}', `@${name}`);
    const also = extras.length ? t('hubToAlsoHint').replace('{names}', extras.map((n) => `@${n}`).join(', ')) : '';
    return [destination, also].filter(Boolean).join('\n');
  }

  function vitalsLine(v) {
    if (!v) return '';
    const parts = [];
    if (v.model) parts.push(modelLabel(v.model));
    if (v.context_pct != null) parts.push(`${v.context_pct}% ctx`);
    if (v.effort) parts.push(v.effort);
    if (v.branch) parts.push(v.branch);
    return parts.join(' · ');
  }

  function cardInfo(a) {
    const lines = [{ label: t('hubHoverState'), value: [stateLabel(a.state), a.detail].filter(Boolean).join(' · '), tone: stateTone(a.state) }];
    const model = [a.agent, a.vitals?.model ? modelLabel(a.vitals.model) : ''].filter(Boolean).join(' · ');
    if (model) lines.push({ label: t('hubHoverModel'), value: model });
    if (a.vitals?.context_pct != null) lines.push({ label: t('hubHoverCtx'), value: `${a.vitals.context_pct}%` });
    if (a.since) lines.push({ label: t('hubHoverSince'), value: fmtElapsed(a.since, tick) });
    lines.push({ label: t('hubHoverTarget'), value: `${selected}:${a.window}` });
    if (selectedRow?.project.path) lines.push({ label: t('hubHoverPath'), value: selectedRow.project.path });
    const text = [a.vitals?.effort, a.vitals?.branch].filter(Boolean).join(' · ');
    return { title: a.name, lines, text, note: destinationNote(a.name) };
  }

  function offCardInfo(name) {
    const slot = (selectedRow?.slots ?? []).find((x) => x.window_name === name);
    const lines = [{ label: t('hubHoverState'), value: t('hubStopped') }];
    if (slot?.command) lines.push({ label: t('hubHoverModel'), value: slot.command });
    if (selectedRow?.project.path) lines.push({ label: t('hubHoverPath'), value: selectedRow.project.path });
    return { title: name, lines };
  }

  // Native keyboard clicks have no pointer point; keep their context menu at the card.
  function pointOf(e) {
    const trigger = e.currentTarget;
    if (!e.clientX && !e.clientY) {
      const rect = trigger.getBoundingClientRect();
      return { x: rect.left, y: rect.bottom, trigger };
    }
    return { x: e.clientX, y: e.clientY, trigger };
  }
</script>

{#if selected}
  <div class="roster" class:compact>
    <div class="cards" class:reveal={justLoaded}>
      {#if !roomReady}
        <div class="skel-wrap sk-cards" aria-hidden="true">
          <span class="skel sk-card"></span><span class="skel sk-card"></span><span class="skel sk-card"></span>
        </div>
      {:else}
        <div class="acard all" data-agent={ALL_TARGET} class:sel={recipient === ALL_TARGET}>
          <button type="button" class="agent-select" aria-pressed={recipient === ALL_TARGET}
            aria-label={[t('hubEveryone'), '@all', extras.includes(ALL_TARGET) ? t('hubToAlsoHint').replace('{names}', '@all') : ''].filter(Boolean).join(' · ')}
            use:hoverInfo={() => ({ title: t('hubEveryone'), note: destinationNote(ALL_TARGET) })}
            onclick={() => selectTarget(ALL_TARGET)}>
            <span class="ava all-ava"><Icon name="collab" size={18} /></span>
            <span class="agent-facts">
              <span class="agent-name-row"><span class="a-name">{t('hubEveryone')}</span></span>
              <span class="agent-state">@all</span>
            </span>
            <span class="agent-marks">
              {#if extras.includes(ALL_TARGET)}<span class="agent-mention" aria-hidden="true">@</span>{/if}
            </span>
          </button>
          {#if busyNames.length}
            <span class="agent-stop">
              <CommandButton label={`${t('hubInterrupt')} ${t('hubEveryone')}`} icon="stop" variant="secondary" iconOnly
                pending={allPending} disabled={allPending}
                onclick={(e) => { e.stopPropagation(); interrupt(ALL_TARGET); }} />
            </span>
          {/if}
        </div>
      {/if}

      {#snippet card(a)}
        {@const mentioned = extras.includes(a.name) || extras.includes(ALL_TARGET)}
        {@const pending = interrupting.includes(a.name)}
        <!-- Selection and interruption are sibling native targets, never nested buttons. -->
        <div class="acard" data-agent={a.name} class:sel={recipient === a.name}
          class:appear-pop={!!rosterBase && !rosterBase.has(a.name)}>
          <button type="button" class="agent-select"
            aria-pressed={recipient === a.name}
            aria-label={[`${a.name} · ${stateLabel(a.state)}`, a.detail, vitalsLine(a.vitals), unread.has(a.name) ? t('hubUnread') : '', mentioned ? t('hubToAlsoHint').replace('{names}', `@${a.name}`) : ''].filter(Boolean).join(' · ')}
            use:hoverInfo={() => cardInfo(a)}
            onclick={() => selectTarget(a.name)}
            oncontextmenu={(e) => { e.preventDefault(); oncontext(pointOf(e), a.name); }}
            use:longpress={{ onlongpress: (pt) => oncontext(pt, a.name) }}>
            {#if backendIcon(a.agent)}<img class="ava" src={backendIcon(a.agent)} alt={a.agent} />{:else}<span class="ava" style:background={backendColor(a.agent)}>{a.name.slice(0, 1).toUpperCase()}</span>{/if}
            <span class="agent-facts">
              <span class="agent-name-row"><span class="a-name">{a.name}</span></span>
              <span class="agent-state ac-top" class:needs={stateNeedsYou(a.state)}>
                <span class="st" class:live-dot={stateIsLive(a.state)} style:background={stateDotColor(a.state)}></span>
                <span>{stateLabel(a.state)}</span>
              </span>
            </span>
            <span class="agent-marks">
              {#if mentioned}<span class="agent-mention" aria-hidden="true">@</span>{/if}
              {#if unread.has(a.name)}<span class="unread appear-pop" aria-hidden="true"></span>{/if}
            </span>
          </button>
          {#if busyNames.includes(a.name)}
            <span class="agent-stop">
              <CommandButton label={`${t('hubInterrupt')} ${a.name}`} icon="stop" variant="secondary" iconOnly
                {pending} disabled={pending}
                onclick={(e) => { e.stopPropagation(); interrupt(a.name); }} />
            </span>
          {/if}
        </div>
      {/snippet}
      {#snippet group(g)}
        <div class="tgroup" role="group" aria-label={t('hubTeamGroup').replace('{name}', g.path)}>
          <span class="tg-label"><Icon name="collab" size={10} />{g.team}</span>
          <div class="tg-cards">
            {#each g.items as x (x.path ?? `w${x.window}`)}
              {#if x.items}{@render group(x)}{:else}{@render card(x)}{/if}
            {/each}
          </div>
        </div>
      {/snippet}
      {#each rosterGroups as x (x.path ?? `w${x.window}`)}
        {#if x.items}{@render group(x)}{:else}{@render card(x)}{/if}
      {/each}
      {#each stopped as name (name)}
        {@const backend = slotBackend(name)}
        <!-- A stopped identity offers context actions, never a card-wide restart.
             Owner, 2026-09-05: "头像应该使用我们正常设定的 Agent 头像，并且变成灰色". -->
        <div class="acard off" data-agent={name} class:appear-pop={!!rosterBase && !rosterBase.has(name)}
          animate:flip={{ duration: moveMs() }}>
          <button type="button" class="agent-select" disabled={acting}
            aria-label={`${name} · ${t('hubStopped')}`} aria-haspopup="menu"
            use:hoverInfo={() => offCardInfo(name)}
            onclick={(e) => oncontext(pointOf(e), name)}
            oncontextmenu={(e) => { e.preventDefault(); oncontext(pointOf(e), name); }}
            use:longpress={{ onlongpress: (pt) => oncontext(pt, name) }}>
            {#if backendIcon(backend)}<img class="ava dim" src={backendIcon(backend)} alt={backend} />{:else}<span class="ava dim">{name.slice(0, 1).toUpperCase()}</span>{/if}
            <span class="agent-facts">
              <span class="agent-name-row"><span class="a-name">{name}</span></span>
              <span class="agent-state">{t('hubStopped')}</span>
            </span>
          </button>
        </div>
      {/each}
      <!-- Spawning opens a closed project too; keep its entry even in an empty room. -->
      <button type="button" class="acard add" class:mini={managedAgents.length > 0 || stopped.length > 0}
        onclick={onadd} aria-label={t('hubSpawn')} use:hoverInfo={() => ({ title: t('hubSpawn') })}>
        <Icon name="plus" size={16} />{#if !managedAgents.length && !stopped.length}<span>{t('hubSpawn')}</span>{/if}
      </button>
    </div>
  </div>
{/if}

<style>
  /* Density is undecided: two lines remain the default. A one-line choice only
     changes these local dimensions/flow, not the DOM or interaction contract. */
  .roster {
    --roster-facts-flow: column;
    --roster-facts-gap: 2px;
    --roster-card-width: 180px;
    --roster-all-width: 142px;
    --roster-select-height: 54px;
    --roster-avatar-size: 25px;
    --roster-control-gap: 8px;
    display: flex; flex: none; min-width: 0; padding: 6px 14px;
  }
  @media (any-pointer: coarse) {
    .roster { --roster-card-width: 184px; --roster-all-width: 150px; --roster-select-height: 58px; }
  }
  .roster.compact { padding-inline: 10px; }
  .cards {
    display: flex; align-items: end; gap: 6px; overflow-x: auto; scrollbar-width: none;
    flex: 1 1 auto; min-width: 0; padding: 2px;
  }
  .cards::-webkit-scrollbar { display: none; }
  .sk-cards { display: flex; gap: 6px; flex: none; }
  .sk-card { width: var(--roster-card-width); min-height: var(--roster-select-height); border-radius: var(--ui-radius-row); }
  .tgroup { flex: none; display: flex; flex-direction: column; gap: 2px; }
  .tg-label { display: inline-flex; align-items: center; gap: 3px; padding: 0 4px; font-family: var(--font-mono); font-size: var(--fs-micro); color: var(--text2); line-height: 1.4; }
  .tg-cards { display: flex; align-items: end; gap: 6px; }
  .acard {
    display: flex; align-items: center; flex: none; min-width: var(--roster-card-width);
    border: 0; border-radius: var(--ui-radius-row); background: var(--surface);
    box-shadow: inset 0 0 0 1px var(--border); color: var(--text);
    transition: background var(--t-fast), box-shadow var(--t-fast);
  }
  .acard:hover { box-shadow: inset 0 0 0 1px var(--input-border); }
  .acard.sel { background: var(--accent-bg); box-shadow: inset 0 0 0 1px var(--accent-line); }
  .acard.all { min-width: var(--roster-all-width); }
  .agent-select {
    display: flex; align-items: center; flex: 1 0 auto; gap: var(--roster-control-gap);
    min-height: max(var(--control-height), var(--roster-select-height)); min-width: var(--control-height);
    border: 0; border-radius: var(--ui-radius-row); background: transparent; color: inherit;
    padding: 0 10px; text-align: left; cursor: pointer; font-size: var(--fs-ui);
    -webkit-tap-highlight-color: transparent;
  }
  .agent-select:focus-visible { outline-color: var(--accent-ink); outline-offset: -2px; }
  .agent-facts { display: flex; flex-direction: var(--roster-facts-flow); gap: var(--roster-facts-gap); }
  .agent-name-row { display: flex; align-items: center; }
  .a-name { font-family: var(--font-display); font-size: var(--fs-ui); font-weight: 600; white-space: nowrap; }
  .agent-state { display: flex; align-items: center; gap: 6px; color: var(--text2); font-family: var(--font-mono); font-size: var(--fs-meta); line-height: 1.4; white-space: nowrap; }
  .agent-state.needs { color: var(--status-warn); font-weight: 600; }
  .agent-marks { display: flex; flex-direction: column; align-items: center; justify-content: center; gap: 3px; min-width: 1em; }
  .agent-mention { color: var(--accent-ink); font-family: var(--font-mono); font-size: var(--fs-meta); font-weight: 600; }
  .agent-stop { display: flex; align-items: center; flex: none; }
  .ava { width: var(--roster-avatar-size); height: var(--roster-avatar-size); flex: none; }
  .all-ava { display: grid; place-items: center; background: var(--surface2); color: var(--text2); }
  .unread { width: 7px; height: 7px; border-radius: 50%; background: var(--status-danger); flex: none; }
  .off { color: var(--text2); }
  .ava.dim { background: var(--surface2); color: var(--text3); }
  img.ava.dim { background: none !important; filter: grayscale(1); opacity: 0.55; }
  .acard.add {
    position: sticky; right: 0; z-index: 1; justify-content: center; gap: 6px; min-width: var(--control-height);
    min-height: max(var(--control-height), var(--roster-select-height)); padding: 0 10px;
    background: var(--bg); box-shadow: none; color: var(--text2); cursor: pointer; font-size: var(--fs-ui);
  }
  .acard.add:hover { background: var(--surface2); color: var(--accent-ink); }
  .acard.add.mini { width: var(--control-height); padding: 0; }
</style>
