<script>
  import Icon from '../ui/Icon.svelte';
  import CommandButton from '../ui/CommandButton.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { untrack } from 'svelte';
  import { ALL_TARGET } from './hub-composer.ts';
  import { backendIcon } from '../core/agents.ts';
  import { backendColor, stateDotColor, stateIsLive, chipExtras, ctxColor, fmtElapsed, modelLabel, sortAgentsForRoster } from './hub.ts';
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
    expanded = false, onexpand = () => {}, onadd = () => {}, oncontext = () => {},
  } = $props();

  const cardsId = $props.id();
  let cardsEl = $state(null);
  let hovering = $state(false);
  let focused = $state(false);
  let pressing = $state(false);
  let heldOrder = $state.raw({ session: '', names: [] });
  const holdOrder = $derived(hovering || focused || pressing);
  const ranked = $derived(sortAgentsForRoster(managedAgents));
  function clearPress() {
    pressing = false;
  }
  function beginPress() {
    pressing = true;
  }
  $effect(() => {
    void managedAgents; void busyNames; void selected;
    focused = !!cardsEl?.contains(document.activeElement);
  });
  // Freeze identity order, not the live agent objects or their action state.
  $effect(() => {
    const names = ranked.map((a) => a.name);
    const previous = untrack(() => heldOrder);
    let next = names;
    if (holdOrder && previous.session === selected) {
      const present = new Set(names);
      const known = new Set(previous.names);
      next = [...previous.names.filter((name) => present.has(name)), ...names.filter((name) => !known.has(name))];
    }
    if (previous.session !== selected || next.length !== previous.names.length
        || next.some((name, index) => name !== previous.names[index])) {
      heldOrder = { session: selected, names: next };
    }
  });
  const orderedAgents = $derived.by(() => {
    if (!holdOrder || heldOrder.session !== selected) return ranked;
    const current = new Map(ranked.map((a) => [a.name, a]));
    const held = new Set(heldOrder.names);
    return [...heldOrder.names.map((name) => current.get(name)).filter(Boolean), ...ranked.filter((a) => !held.has(a.name))];
  });
  $effect(() => {
    void selected; void expanded;
    if (cardsEl) { cardsEl.scrollLeft = 0; cardsEl.scrollTop = 0; }
  });
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
    if (a.team) lines.push({ label: t('teamsTitle'), value: a.team });
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
    <div class="cards" class:expanded class:reveal={justLoaded} id={cardsId} bind:this={cardsEl}
      role="group" aria-label={t('agentsTitle')}
      onpointerenter={(e) => { hovering = e.pointerType !== 'touch'; }}
      onpointerleave={() => { hovering = false; clearPress(); }}
      onpointerdown={beginPress} onpointerup={clearPress} onlostpointercapture={clearPress}
      onpointercancel={clearPress} onclickcapture={clearPress}
      onfocusin={() => { focused = true; }}
      onfocusout={(e) => { focused = !!e.relatedTarget && e.currentTarget.contains(e.relatedTarget); if (!focused) clearPress(); }}>
      {#if !roomReady}
        <div class="skel-wrap sk-cards" aria-hidden="true">
          <span class="skel sk-card"></span><span class="skel sk-card"></span><span class="skel sk-card"></span>
        </div>
      {:else}
        <div class="acard all" data-agent={ALL_TARGET} class:sel={recipient === ALL_TARGET} class:has-stop={busyNames.length > 0}>
          <button type="button" class="agent-select" aria-pressed={recipient === ALL_TARGET}
            aria-label={[t('hubEveryone'), '@all', extras.includes(ALL_TARGET) ? t('hubToAlsoHint').replace('{names}', '@all') : ''].filter(Boolean).join(' · ')}
            use:hoverInfo={() => ({ title: t('hubEveryone'), note: destinationNote(ALL_TARGET) })}
            onclick={() => selectTarget(ALL_TARGET)}>
            <span class="broadcast-glyph"><Icon name="collab" size={18} /></span>
            <span class="a-name">{t('hubEveryone')}</span>
            <span class="agent-marks" class:unmarked={!extras.includes(ALL_TARGET)}>
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

      {#each orderedAgents as a (a.name)}
        {@const mentioned = extras.includes(a.name) || extras.includes(ALL_TARGET)}
        {@const pending = interrupting.includes(a.name)}
        <!-- Selection and interruption are sibling native targets, never nested buttons. -->
        <div class="acard" data-agent={a.name} class:sel={recipient === a.name} class:has-stop={busyNames.includes(a.name)}
          class:appear-pop={!!rosterBase && !rosterBase.has(a.name)} animate:flip={{ duration: moveMs() }}>
          <button type="button" class="agent-select"
            aria-pressed={recipient === a.name}
            aria-label={[`${a.name} · ${stateLabel(a.state)}`, a.team, a.detail, vitalsLine(a.vitals), unread.has(a.name) ? t('hubUnread') : '', mentioned ? t('hubToAlsoHint').replace('{names}', `@${a.name}`) : ''].filter(Boolean).join(' · ')}
            use:hoverInfo={() => cardInfo(a)}
            onclick={() => selectTarget(a.name)}
            oncontextmenu={(e) => { e.preventDefault(); oncontext(pointOf(e), a.name); }}
            use:longpress={{ onlongpress: (pt) => oncontext(pt, a.name) }}>
            {#if backendIcon(a.agent)}<img class="ava" src={backendIcon(a.agent)} alt={a.agent} />{:else}<span class="ava" style:background={backendColor(a.agent)}>{a.name.slice(0, 1).toUpperCase()}</span>{/if}
            <span class="a-name">{a.name}</span>
            <span class="ac-top">
              <span class="st" class:live-dot={stateIsLive(a.state)} style:background={stateDotColor(a.state)}></span>
            </span>
            <span class="agent-marks" class:unmarked={!mentioned && !unread.has(a.name)}>
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
          {#if a.vitals?.context_pct != null}
            {@const pct = Math.max(0, Math.min(100, a.vitals.context_pct))}
            <div class="ac-bar" role="meter" aria-label={t('hubCtxUsed')}
              aria-valuemin="0" aria-valuemax="100" aria-valuenow={pct} aria-valuetext={`${a.vitals.context_pct}%`}>
              <i style:width={`${pct}%`} style:background={ctxColor(a.vitals.context_pct)}></i>
            </div>
          {/if}
        </div>
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
            <span class="a-name">{name}</span>
          </button>
        </div>
      {/each}
      <!-- Spawning opens a closed project too; keep its entry even in an empty room. -->
      <div class="roster-add">
        <CommandButton icon="plus" variant="icon" label={t('hubSpawn')} onclick={onadd} />
      </div>
    </div>
    <div class="roster-toggle">
      <CommandButton icon="chevron-up" variant="icon" label={expanded ? t('hubRosterCollapse') : t('hubRosterExpand')}
        {expanded} controls={cardsId} disabled={!roomReady} onclick={onexpand} />
    </div>
  </div>
{/if}

<style>
  .roster {
    --roster-avatar-size: 20px;
    --roster-gap: 6px;
    --roster-control-gap: 4px;
    --roster-expanded-max: min(240px, calc(32dvh / var(--ui-zoom, 1)));
    display: grid; grid-template-columns: minmax(0, 1fr) var(--control-height);
    gap: var(--roster-gap); flex: 0 1 auto; min-width: 0; min-height: 0; padding: 4px 14px;
    container: roster / inline-size;
  }
  .roster.compact { padding-inline: 10px; }
  .cards {
    display: flex; align-items: start; gap: var(--roster-gap); overflow-x: auto; scrollbar-width: none;
    min-width: 0; min-height: 0; padding: 2px;
  }
  .cards:not(.expanded)::-webkit-scrollbar { display: none; }
  .cards.expanded {
    display: grid; grid-template-columns: minmax(0, 1fr); align-content: start; align-items: stretch;
    max-height: var(--roster-expanded-max); overflow-x: hidden; overflow-y: auto; scrollbar-width: thin;
  }
  @container roster (min-width: 360px) { .cards.expanded { grid-template-columns: repeat(2, minmax(0, 1fr)); } }
  @container roster (min-width: 720px) { .cards.expanded { grid-template-columns: repeat(4, minmax(0, 1fr)); } }
  .sk-cards { display: flex; gap: var(--roster-gap); flex: none; grid-column: 1 / -1; }
  .sk-card { width: calc(3 * var(--control-height)); height: var(--control-height); border-radius: var(--ui-radius-row); }
  .acard {
    position: relative;
    display: grid; grid-template-columns: minmax(0, 1fr) var(--control-height);
    align-items: center; flex: none; width: max-content; min-width: 0;
    border: 0; border-radius: var(--ui-radius-row); background: var(--surface);
    box-shadow: inset 0 0 0 1px var(--border); color: var(--text);
    transition: background var(--t-fast), box-shadow var(--t-fast);
  }
  .acard:hover { box-shadow: inset 0 0 0 1px var(--input-border); }
  .acard.sel { background: var(--accent-bg); box-shadow: inset 0 0 0 1px var(--accent-line); }
  .acard.all {
    border-radius: var(--ui-radius-pill); background: transparent; color: var(--accent-ink);
    box-shadow: inset 0 0 0 1px var(--accent-line);
  }
  .acard.all.sel { background: var(--accent-bg); }
  .acard.all .agent-select { border-radius: inherit; }
  .ac-bar {
    position: absolute; left: var(--ui-radius-row); right: var(--ui-radius-row); bottom: 0; height: 2px;
    background: var(--pill-bg); overflow: hidden; pointer-events: none;
  }
  .ac-bar > i { display: block; height: 100%; transition: width var(--t-move), background var(--t-move); }
  @media (prefers-reduced-motion: reduce) { .ac-bar > i { transition: none; } }
  .cards.expanded .acard { width: auto; }
  .agent-select {
    display: flex; align-items: center; gap: var(--roster-control-gap);
    min-height: var(--control-height); min-width: var(--control-height);
    border: 0; border-radius: var(--ui-radius-row); background: transparent; color: inherit;
    padding: 0 6px; text-align: left; cursor: pointer; font-size: var(--fs-ui);
    -webkit-tap-highlight-color: transparent;
  }
  .agent-select:focus-visible { outline-color: var(--accent-ink); outline-offset: -2px; }
  .acard:not(.has-stop):not(.off) .agent-select { grid-column: 1 / -1; padding-right: calc(6px + var(--control-height)); }
  .a-name { font-family: var(--font-display); font-size: var(--fs-ui); font-weight: 600; white-space: nowrap; }
  .cards.expanded .a-name { min-width: 0; white-space: normal; overflow-wrap: anywhere; }
  .ac-top { display: inline-flex; flex: none; }
  .agent-marks { display: inline-flex; flex-direction: column; align-items: center; justify-content: center; gap: 3px; min-width: 1em; flex: none; }
  .cards.expanded .agent-marks.unmarked { display: none; }
  .agent-mention { color: var(--accent-ink); font-family: var(--font-mono); font-size: var(--fs-meta); font-weight: 600; }
  .agent-stop { display: flex; align-items: center; justify-self: end; }
  .ava { width: var(--roster-avatar-size); height: var(--roster-avatar-size); flex: none; }
  .broadcast-glyph { display: grid; place-items: center; width: var(--roster-avatar-size); height: var(--roster-avatar-size); flex: none; }
  .unread { width: 7px; height: 7px; border-radius: 50%; background: var(--status-danger); flex: none; }
  .off { color: var(--text2); }
  .off .agent-select { grid-column: 1 / -1; }
  .ava.dim { background: var(--surface2); color: var(--text3); }
  img.ava.dim { background: none !important; filter: grayscale(1); opacity: 0.55; }
  .roster-add { display: flex; align-items: center; flex: none; min-height: var(--control-height); }
  .roster-toggle { display: flex; align-self: end; padding-block-end: 2px; }
</style>
