<script>
  import CommandButton from '../ui/CommandButton.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { untrack } from 'svelte';
  import { ALL_TARGET } from './hub-composer.ts';
  import { backendIcon } from '../core/agents.ts';
  import { backendColor, stateDotColor, stateIsLive, chipExtras, ctxColor, fmtElapsed, modelLabel, sortAgentsForRoster } from './hub.ts';
  import { hoverInfo } from '../ui/hover.ts';
  import { longpress } from '../ui/longpress.ts';
  import { anchorOf } from '../ui/placement.ts';
  import { flip } from 'svelte/animate';
  import { moveMs } from '../ui/motion.ts';
  import { scrollEdges } from '../ui/scroll-edges.ts';

  let {
    selected = '', compact = false, managedAgents = [], stopped = [], selectedRow = null,
    recipient = '', filterAgent = '', unread = new Set(), acting = false, tick = Date.now(),
    roomReady = false, justLoaded = false, rosterBase = null,
    composerText = '', managedNames = [], busyNames = [], interrupting = [],
    stateLabel = (state) => state, stateTone = () => undefined,
    onselect: setRecipient = () => {}, oninterrupt: interrupt = () => {}, onfilter = () => {},
    expanded = false, onexpand = () => {}, onadd = () => {}, oncontext = () => {},
  } = $props();

  const cardsId = $props.id();
  let cardsEl = $state(null);
  let hovering = $state(false);
  let focused = $state(false);
  let pressing = $state(false);
  let pressStops = $state.raw([]);
  // Only layout is held during a press; current busyNames still gates actions.
  const renderedStops = $derived(pressing ? pressStops : busyNames);
  let heldOrder = $state.raw({ session: '', names: [] });
  const holdOrder = $derived(hovering || focused || pressing);
  const ranked = $derived(sortAgentsForRoster(managedAgents));
  function clearPress() {
    pressing = false;
    pressStops = [];
  }
  function beginPress() {
    if (!pressing) pressStops = busyNames;
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
  const slotBackend = (name) => (selectedRow?.slots ?? []).find((s) => s.window_name === name)?.command;

  function selectTarget(name) {
    setRecipient(recipient === name ? '' : name);
  }
  const isAddressed = (name) => recipient === ALL_TARGET || recipient === name;
  const coarsePointer = () => window.matchMedia('(any-pointer: coarse)').matches;
  function clickAgent(event, name) {
    if (!coarsePointer() && event.detail > 1) return;
    selectTarget(name);
  }
  function focusAgent(event, name, stopped = false) {
    if (coarsePointer() || event.detail < 2) return;
    event.preventDefault();
    if (!stopped && recipient !== name) setRecipient(name);
    onfilter(name);
  }
  function stoppedMenu(event, name) {
    if (!coarsePointer() && event.detail > 1) return;
    const trigger = event.currentTarget;
    oncontext({ anchor: anchorOf(trigger), align: 'left', trigger, keepTriggerClear: true }, name);
  }

  function destinationNote(name) {
    if (!isAddressed(name)) return '';
    const destination = recipient === ALL_TARGET ? t('hubToAllLong') : t('hubToDmLong').replace('{name}', `@${name}`);
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
    <div class="cards edge-fade" class:expanded class:reveal={justLoaded} id={cardsId} bind:this={cardsEl} use:scrollEdges={!expanded}
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
      {/if}

      {#each orderedAgents as a (a.name)}
        {@const mentioned = extras.includes(a.name) || extras.includes(ALL_TARGET)}
        {@const pending = interrupting.includes(a.name)}
        <!-- Selection and interruption are sibling native targets, never nested buttons. -->
        <div class="acard" data-agent={a.name} class:sel={isAddressed(a.name)} class:filtered={filterAgent === a.name} class:has-stop={busyNames.includes(a.name)}
          class:appear-pop={!!rosterBase && !rosterBase.has(a.name)} animate:flip={{ duration: moveMs() }}>
          <button type="button" class="agent-select"
            aria-pressed={isAddressed(a.name)}
            aria-label={[`${a.name} · ${stateLabel(a.state)}`, a.team, a.detail, vitalsLine(a.vitals), unread.has(a.name) ? t('hubUnread') : '', mentioned ? t('hubToAlsoHint').replace('{names}', `@${a.name}`) : '', filterAgent === a.name ? t('hubFilterItem') : ''].filter(Boolean).join(' · ')}
            use:hoverInfo={() => cardInfo(a)}
            onclick={(e) => clickAgent(e, a.name)} ondblclick={(e) => focusAgent(e, a.name)}
            oncontextmenu={(e) => { e.preventDefault(); oncontext(pointOf(e), a.name); }}
            use:longpress={{ onlongpress: (pt) => oncontext(pt, a.name) }}>
            <span class="avatar-slot">
              {#if backendIcon(a.agent)}<img class="ava" src={backendIcon(a.agent)} alt={a.agent} />{:else}<span class="ava" style:background={backendColor(a.agent)}>{a.name.slice(0, 1).toUpperCase()}</span>{/if}
            </span>
            <span class="a-name">{a.name}<span class="ac-top"><span class="st" class:live-dot={stateIsLive(a.state)} style:background={stateDotColor(a.state)}></span></span></span>
            <span class="agent-marks" class:unmarked={!mentioned && !unread.has(a.name)}>
              {#if mentioned}<span class="agent-mention" aria-hidden="true">@</span>{/if}
              {#if unread.has(a.name)}<span class="unread appear-pop" aria-hidden="true"></span>{/if}
            </span>
            {#if expanded && a.vitals?.context_pct != null}<span class="ctx-value">{a.vitals.context_pct}%</span>{/if}
          </button>
          {#if renderedStops.includes(a.name)}
            <!-- A dense slot (compact-tools: 28/32px, board #192) and the plain icon ink:
                 the card's colour is its state dot; the stop is a quiet action beside it
                 (owner, 2026-09-13: "停止按钮，又大颜色也不好看"; board #195). -->
            <span class="agent-stop compact-tools" class:pending>
              <CommandButton label={`${t('hubInterrupt')} ${a.name}`} icon="stop" variant="icon" iconOnly
                {pending} disabled={pending || !busyNames.includes(a.name)}
                onclick={(e) => { e.stopPropagation(); interrupt(a.name); }} />
            </span>
          {/if}
          {#if a.vitals?.context_pct != null}
            {@const pct = Math.max(0, Math.min(100, a.vitals.context_pct))}
            <div class="ctx-ring" role="meter" aria-label={t('hubCtxUsed')}
              aria-valuemin="0" aria-valuemax="100" aria-valuenow={pct} aria-valuetext={`${a.vitals.context_pct}%`}
              style:--ctx-amount={`${pct}%`} style:--ctx-color={ctxColor(a.vitals.context_pct)}></div>
          {/if}
        </div>
      {/each}
      {#each stopped as name (name)}
        {@const backend = slotBackend(name)}
        <!-- A stopped identity offers context actions, never a card-wide restart.
             Owner, 2026-09-05: "头像应该使用我们正常设定的 Agent 头像，并且变成灰色". -->
        <div class="acard off" data-agent={name} class:filtered={filterAgent === name} class:appear-pop={!!rosterBase && !rosterBase.has(name)}
          animate:flip={{ duration: moveMs() }}>
          <button type="button" class="agent-select" disabled={acting}
            aria-label={[name, t('hubStopped'), filterAgent === name ? t('hubFilterItem') : ''].filter(Boolean).join(' · ')} aria-haspopup="menu"
            use:hoverInfo={() => offCardInfo(name)}
            onclick={(e) => stoppedMenu(e, name)} ondblclick={(e) => focusAgent(e, name, true)}
            oncontextmenu={(e) => { e.preventDefault(); oncontext(pointOf(e), name); }}
            use:longpress={{ onlongpress: (pt) => oncontext(pt, name) }}>
            <span class="avatar-slot">
              {#if backendIcon(backend)}<img class="ava dim" src={backendIcon(backend)} alt={backend} />{:else}<span class="ava dim">{name.slice(0, 1).toUpperCase()}</span>{/if}
            </span>
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
    --roster-ring-size: 26px;
    --roster-ring-stroke: 2px;
    --roster-paint-height: 30px;
    --roster-card-inset: 4px;
    --roster-gap: 2px;
    --roster-control-gap: 2px;
    --roster-expanded-max: min(240px, calc(32dvh / var(--ui-zoom, 1)));
    display: grid; grid-template-columns: minmax(0, 1fr) var(--control-height);
    gap: 0; flex: 0 1 auto; min-width: 0; min-height: 0; padding: 0 14px;
    container: roster / inline-size;
  }
  .roster.compact { padding-inline: 10px; }
  .cards {
    display: flex; align-items: center; gap: var(--roster-gap); overflow-x: auto; scrollbar-width: none;
    min-width: 0; min-height: 0; padding: 2px;
  }
  .cards:not(.expanded)::-webkit-scrollbar { display: none; }
  .cards.expanded {
    flex-wrap: wrap; align-content: start;
    max-height: var(--roster-expanded-max); overflow-x: hidden; overflow-y: auto; scrollbar-width: thin;
  }
  .sk-cards { display: flex; gap: var(--roster-gap); flex: none; grid-column: 1 / -1; }
  .sk-card {
    width: calc(3 * var(--control-height)); height: var(--roster-paint-height);
    margin-block: var(--control-paint-inset); border-radius: var(--ui-radius-row);
  }
  .acard {
    --card-paint: var(--surface); --card-line: var(--border);
    position: relative;
    display: flex;
    align-items: center; flex: none; width: max-content; min-width: 0;
    min-height: calc(var(--roster-paint-height) + 2 * var(--control-paint-inset));
    border: 0; border-radius: var(--ui-radius-row); color: var(--text);
  }
  .acard::before {
    content: ''; position: absolute; inset: var(--control-paint-inset) 0;
    border-radius: inherit; pointer-events: none;
    background: var(--card-paint); box-shadow: inset 0 0 0 1px var(--card-line);
    transition: background var(--t-fast), box-shadow var(--t-fast);
  }
  .acard:hover { --card-line: var(--input-border); }
  .acard.sel { --card-paint: var(--accent-bg); --card-line: var(--accent-line); }
  .acard.filtered::after {
    content: ''; position: absolute; inset: var(--control-paint-inset) 0; border: 1px dashed var(--text2);
    border-radius: inherit; pointer-events: none;
  }
  .ctx-ring {
    position: absolute; left: var(--roster-card-inset); top: calc(50% - var(--roster-ring-size) / 2);
    width: var(--roster-ring-size); height: var(--roster-ring-size); border-radius: 50%;
    background: conic-gradient(var(--ctx-color) var(--ctx-amount), var(--border) 0);
    mask: radial-gradient(farthest-side, transparent calc(100% - var(--roster-ring-stroke)), var(--control-overlay-dark) 0);
    pointer-events: none;
  }
  @media (prefers-reduced-motion: reduce) { .acard::before { transition: none; } }
  .cards.expanded .acard { max-width: 100%; }
  .agent-select {
    position: relative;
    display: flex; align-items: center; gap: var(--roster-control-gap);
    min-height: var(--control-height); min-width: var(--control-height);
    border: 0; border-radius: var(--ui-radius-row); background: transparent; color: inherit;
    padding: 0 var(--roster-card-inset); text-align: left; cursor: pointer; font-size: var(--fs-ui);
    -webkit-tap-highlight-color: transparent;
  }
  .agent-select:focus-visible { outline-color: var(--accent-ink); outline-offset: 0; }
  .a-name { font-family: var(--font-display); font-size: var(--fs-ui); font-weight: 600; white-space: nowrap; }
  .cards.expanded .a-name {
    min-width: 0; white-space: normal; overflow-wrap: anywhere;
    padding-block: var(--control-paint-inset);
  }
  /* The dot lives INSIDE the name's line, 5px after its last glyph, with
     vertical-align: middle — by definition the box's midpoint on the baseline
     plus half the x-height, i.e. centred on the lowercase letters whatever
     the font's ascent/descent. As a flex sibling it was centred on the LINE
     box, ~1px off and font-dependent, 1–2px from the name (owner, 2026-09-13:
     "状态小点稍微有点挨得近了，而且上下不居中"; board #195). */
  .ac-top { display: inline-flex; vertical-align: middle; margin-inline-start: 5px; }
  .agent-marks { display: inline-flex; flex-direction: column; align-items: center; justify-content: center; gap: 3px; min-width: 1em; flex: none; }
  .agent-marks.unmarked { display: none; }
  .agent-mention { color: var(--accent-ink); font-family: var(--font-mono); font-size: var(--fs-meta); font-weight: 600; }
  .agent-stop { display: flex; align-items: center; flex: none; }
  @media (any-pointer: coarse) {
    .roster { --roster-paint-height: 34px; }
  }
  .avatar-slot { width: var(--roster-ring-size); height: var(--roster-ring-size); display: grid; place-items: center; flex: none; }
  .ava { width: var(--roster-avatar-size); height: var(--roster-avatar-size); flex: none; border-radius: 50%; object-fit: contain; display: grid; place-items: center; }
  .ctx-value { width: 4ch; flex: none; text-align: right; font: var(--fs-meta)/1 var(--font-mono); color: var(--text2); white-space: nowrap; }
  .unread { width: 7px; height: 7px; border-radius: 50%; background: var(--status-danger); flex: none; }
  .off { color: var(--text2); }
  .ava.dim { background: var(--surface2); color: var(--text3); }
  img.ava.dim { background: none !important; filter: grayscale(1); opacity: 0.55; }
  .roster-add { display: flex; align-items: center; flex: none; min-height: var(--control-height); }
  .roster-toggle { display: flex; align-self: end; align-items: center; height: calc(var(--roster-paint-height) + 2 * var(--control-paint-inset) + 4px); }
</style>
