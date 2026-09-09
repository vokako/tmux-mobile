<script>
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { groupRoster } from './roster.ts';
  import { backendIcon } from '../core/agents.ts';
  import { backendColor, stateDotColor, stateIsLive, stateNeedsYou, ctxColor, fmtElapsed, modelLabel } from './hub.ts';
  import { anchorOf, menuPlacement, popOrigin, viewBox } from '../ui/placement.ts';
  import { hoverInfo } from '../ui/hover.ts';
  import { longpress } from '../ui/longpress.ts';
  import { flip } from 'svelte/animate';
  import { moveMs } from '../ui/motion.ts';

  let {
    selected = '', compact = false, managedAgents = [], stopped = [], selectedRow = null,
    recipient = '', filterAgent = '', unread = new Set(), acting = false, tick = Date.now(),
    roomReady = false, justLoaded = false, rosterBase = null,
    menuFor = $bindable(''), cardsEl = $bindable(null),
    stateLabel = (state) => state, stateTone = () => undefined,
    onselect: setRecipient = () => {}, onfilter: toggleFilter = () => {},
    onwatch: openDrawer = () => {}, onstart: startAgent = () => {},
    oninterrupt: interrupt = () => {}, onrestart: restartAgent = () => {},
    onaction: askAction = () => {}, onconfigure: openAgentConfig = null,
    onadd = () => {}, oncontext = () => {},
  } = $props();

  /** The roster in display order, same-team cards folded into ONE group at
   * the position of their first member (board #74: "视图上放到一个 group 里").
   * Solo agents are groups of one with no team. */
  const rosterGroups = $derived(groupRoster(managedAgents));


  /** A stopped slot's declared backend — the SAME face the live card wears,
   * greyed, not an anonymous letter (owner, 2026-09-05: "头像应该使用我们
   * 正常设定的 Agent 头像，并且变成灰色"). Slots carry the backend in
   * `command`, the way rowAgents' closed-project chips already read it. */
  const slotBackend = (name) => (selectedRow?.slots ?? []).find((s) => s.window_name === name)?.command;


  // The open menu's agent, as its status line reads right now.
  // The menu header's reading DROPS the model: the model belongs to the
  // agent's CONFIG, and the menu now links there instead of quoting it
  // (owner, 2026-08-25: "菜单里不应该有模型名，可以有一个跳转到模型配置的
  // 页面的选项"). Context/effort/branch stay — they are live state, not
  // configuration.
  const vitalsFor = $derived((() => {
    const v = managedAgents.find((a) => a.name === menuFor)?.vitals;
    if (!v) return '';
    const parts = [];
    if (v.context_pct != null) parts.push(`${v.context_pct}% ctx`);
    if (v.effort) parts.push(v.effort);
    if (v.branch) parts.push(v.branch);
    return parts.join(' · ');
  })());
  /** The menu's subject, for its header lines (state + elapsed). */
  const menuAgent = $derived(managedAgents.find((a) => a.name === menuFor));


  /** What the agent's own status line says, as one line. Sniffed server-side
   * from the last lines of its pane (there is no API for a CLI's live state), so
   * every field is a maybe and a missing one is simply absent — never a zero, a
   * dash, or a guess. */
  function vitalsLine(v) {
    if (!v) return '';
    const parts = [];
    if (v.model) parts.push(modelLabel(v.model));
    if (v.context_pct != null) parts.push(`${v.context_pct}% ctx`);
    if (v.effort) parts.push(v.effort);
    if (v.branch) parts.push(v.branch);
    return parts.join(' · ');
  }
  /** The card's share of the reading: what makes THIS agent different from its
   * neighbours. The MODEL only — effort belongs behind the tap, in the agent
   * menu's header line, next to context and branch (owner, 2026-08-26: "只展示
   * 模型名字就可以了 … Effort 只要在我点击额外展开时再去展示"); branch and cwd
   * are project-wide, so they too stay in the tooltip and the menu — repeating
   * them on every card would be chrome, not data. The provider prefix
   * (`openai.`, `xai.`) is dropped too — `modelLabel` — it is the route, not the
   * model, and the card has no room for it (owner, 2026-09-03). */
  function cardVitals(v) {
    return v?.model ? modelLabel(v.model) : '';
  }


  let cardTimer = null;
  let cardTimerFor = ''; // WHICH card the pending menu belongs to — a click on
                         // a DIFFERENT card within the window must still act
                         // (review of board #3: the global timer swallowed it)
  function cardClick(name, el) {
    if (cardTimer) {
      clearTimeout(cardTimer); cardTimer = null;
      if (cardTimerFor === name) return; // 2nd of a double on the SAME card — dblclick acts
      // another card's pending menu is stale; fall through and act on THIS one
    }
    if (recipient !== name) { setRecipient(name); return; }               // select first, options later
    cardTimerFor = name;
    cardTimer = setTimeout(() => { cardTimer = null; toggleAgentMenu(name, el); }, 260);
  }
  function cardDbl(name) {
    if (cardTimer) { clearTimeout(cardTimer); cardTimer = null; }
    menuFor = '';
    setRecipient(name);
    toggleFilter(name); // the parent owns the filter
  }


  let menuAnchor = $state(null);   // trigger rect, in CSS px
  let menuW = $state(0);
  let menuH = $state(0);


  /** A pointer event as a plain client point. */
  const pointOf = (e) => ({ x: e.clientX ?? 0, y: e.clientY ?? 0 });


  function toggleAgentMenu(name, trigger) {
    if (menuFor === name) { menuFor = ''; return; }
    // anchorOf divides the client rect by --ui-zoom: a rect is in visual px
    // while a fixed child's `left` is in its own zoomed px.
    menuAnchor = anchorOf(trigger);
    menuW = 0; menuH = 0;          // re-measure for this opening
    menuFor = name;
  }

  /** Under the trigger, LEFT-aligned to it (board #47: "应该和 agent 卡片
   * 左边缘对齐，而不是右边缘对齐" — the card reads name-first from its left
   * edge, and the menu expanding that card starts where the card starts,
   * the same reading the title menu chose in #32), flipped above when the
   * bottom of the viewport is closer than the menu is tall, and always
   * clamped into view. Measured size arrives one frame after mount, which
   * is why the menu stays invisible until it has one. The math is
   * `menuPlacement` in placement.ts, unit-tested there. */
  const menuPos = $derived.by(() =>
    menuAnchor ? menuPlacement(menuAnchor, { w: menuW, h: menuH }, viewBox(), 6, 8, 'left') : { x: 0, y: 0 },
  );


  function cardInfo(a) {
    const lines = [{ label: t('hubHoverState'), value: [stateLabel(a.state), a.detail].filter(Boolean).join(' · '), tone: stateTone(a.state) }];
    const model = [a.agent, a.vitals?.model ? modelLabel(a.vitals.model) : ''].filter(Boolean).join(' · ');
    if (model) lines.push({ label: t('hubHoverModel'), value: model });
    if (a.vitals?.context_pct != null) lines.push({ label: t('hubHoverCtx'), value: `${a.vitals.context_pct}%` });
    if (a.since) lines.push({ label: t('hubHoverSince'), value: fmtElapsed(a.since, tick) });
    lines.push({ label: t('hubHoverTarget'), value: `${selected}:${a.window}` });
    if (selectedRow?.project.path) lines.push({ label: t('hubHoverPath'), value: selectedRow.project.path });
    return { title: a.name, lines };
  }
  function offCardInfo(name) {
    const slot = (selectedRow?.slots ?? []).find((x) => x.window_name === name);
    const lines = [{ label: t('hubHoverState'), value: t('hubStopped') }];
    if (slot?.command) lines.push({ label: t('hubHoverModel'), value: slot.command });
    if (selectedRow?.project.path) lines.push({ label: t('hubHoverPath'), value: selectedRow.project.path });
    return { title: name, lines };
  }

</script>

{#if selected}
  <!-- The roster. Tapping an agent makes it the recipient (and this
       project's lead) — the phone gets chips, the desktop gets cards. -->
  <!-- The roster row: one scrolling strip, the add button riding INSIDE
       it as the sticky last card. It renders for every SELECTED
       project, empty roster and closed session included, because `+ agent`
       is the only way into an empty room and both gates hid it: on a
       non-empty-roster gate it vanished with the last agent, and on a
       live-session gate a CLOSED project still had none (owner, 2026-08-24,
       twice — "test 这个 project"). A closed session is not a real
       constraint either: `hub_spawn` → `projects::spawn` calls
       `tmux::ensure_session` itself, so spawning into a project that is
       down OPENS it. -->
  <div class="roster">
  <div class="cards" class:chips={compact} class:reveal={justLoaded} bind:this={cardsEl}>
    <!-- The coming roster's shape while an uncached room loads: three
         card-sized skeletons (aria-hidden; .skel-wrap hides them for
         the first 150ms). -->
    {#if !roomReady}
      <div class="skel-wrap sk-cards" aria-hidden="true">
        <span class="skel sk-card"></span><span class="skel sk-card"></span><span class="skel sk-card"></span>
      </div>
    {/if}
    <!-- ONE card markup, rendered per agent through a snippet, so a
         team GROUP (board #74) can wrap several without a second copy
         of the card. -->
    {#snippet card(a)}
      <!-- A div, not a button: the card is a menu trigger and may
           contain real controls; a button inside a button would be invalid
           HTML that the browser silently reshuffles. -->
      <!-- The card itself reaches the options without spending width on a
           redundant dots control (owner clarification, board #89). A card
           tap ALSO makes this agent the recipient: tapping a
           card means "I want to talk to this one", so the conversation
           switches without hunting for the menu's first item (owner,
           2026-08-26: "每次点击 project 里的 Agent 小卡片时 能自动帮我
           切换到跟当前 Agent 的对话"). -->
      <!-- WAITING is the state that needs the human most and it was the
           weakest signal on the card — a 6px static amber dot beside a
           running neighbour's breathing halo (review, 2026-09-03). The
           card itself now carries it: `.needs` = amber frame + wash +
           a short label, in the SAME status tokens the dot speaks
           (stateDotColor → --status-warn), and static — the breathe
           means "a turn is open", and a waiting turn is suspended. -->
      <!-- No native title: the hover card is the tooltip (motion.md
           principle 16) and shows the same facts, live, as rows. The
           aria-label keeps the one-line reading for screen readers. -->
      <div class="acard" class:sel={recipient === a.name} class:needs={stateNeedsYou(a.state)} class:appear-pop={!!rosterBase && !rosterBase.has(a.name)} role="button" tabindex="0"
        aria-label={[`${a.name} · ${stateLabel(a.state)}`, a.detail, vitalsLine(a.vitals)].filter(Boolean).join(' · ')}
        use:hoverInfo={() => cardInfo(a)}
        onclick={(e) => cardClick(a.name, e.currentTarget)}
        ondblclick={() => cardDbl(a.name)}
        oncontextmenu={(e) => { e.preventDefault(); oncontext(pointOf(e), a.name); }}
        use:longpress={{ onlongpress: (pt) => oncontext(pt, a.name) }}
        onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); cardClick(a.name, e.currentTarget); } }}>
        <div class="ac-top">
          {#if backendIcon(a.agent)}<img class="ava" src={backendIcon(a.agent)} alt={a.agent} />{:else}<span class="ava" style:background={backendColor(a.agent)}>{a.name.slice(0, 1).toUpperCase()}</span>{/if}
          <span class="a-name">{a.name}</span>
          <span class="st" class:live-dot={stateIsLive(a.state)} style:background={stateDotColor(a.state)}></span>
          {#if stateNeedsYou(a.state)}<span class="ac-needs appear">{a.state === 'blocked' ? t('hubState_blocked') : t('hubNeedsYou')}</span>{/if}
          {#if unread.has(a.name)}<span class="unread appear-pop" title={t('hubUnread')}></span>{/if}
        </div>
        <!-- What the agent's own status line says, kept ON the card rather
             than behind the menu ("这个直接常驻显示吧 可以字号小一点"). The
             MODEL only: effort shows on tap in the menu header (owner,
             2026-08-26), and the branch is the same for every agent in a
             project, so on a card it is noise. -->
        {#if cardVitals(a.vitals)}<div class="ac-vitals">{cardVitals(a.vitals)}</div>{/if}
        <!-- Context used as a thin colour-changing line at the card's own
             bottom edge ("百分比用一个细长会变颜色的进度条示意 一个细横线就
             行"): a percentage you read at a glance, costing no row and no
             vertical space. The exact number stays in the tooltip and the
             menu header, where a number is what you came for. -->
        {#if a.vitals?.context_pct != null}
          <div class="ac-bar" title={`${a.vitals.context_pct}% · ${t('hubCtxUsed')}`}>
            <i style:width="{a.vitals.context_pct}%" style:background={ctxColor(a.vitals.context_pct)}></i>
          </div>
        {/if}
      </div>
    {/snippet}
    {#snippet group(g)}
      <!-- Same-team cards sit in one labelled group: a dashed frame in
           the row dialect, the team name as a micro label. A nested
           team is a group inside the group (same snippet, recursive). -->
      <div class="tgroup" title={t('hubTeamGroup').replace('{name}', g.path)}>
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
    <!-- Stopped agents: declared by the project, no window right now.
         Starting one resumes its conversation, so it stays on the roster
         instead of vanishing from the room it belongs to. -->
    {#each stopped as name (name)}
      {@const backend = slotBackend(name)}
      <!-- A div for the same reason as the live card: it contains the
           direct Resume control. The card SURFACE is inert — restarting is
           the refresh button's job alone (owner, 2026-08-24: "已经停止的
           agent我只要点击就自动重启了 并没有点到重启的那个圆圈箭头上" —
           a card-wide click restarted agents by accident). Removing it
           lives in the menu, because a stopped agent you are done with
           has to be ejectable — the slot is what keeps `up` recreating
           it (owner, 2026-08-19). -->
      <div class="acard off" class:busy={acting} class:appear-pop={!!rosterBase && !rosterBase.has(name)} role="button" tabindex="0" aria-label={`${name} · ${t('hubStopped')}`}
        animate:flip={{ duration: moveMs() }}
        onclick={(e) => toggleAgentMenu(name, e.currentTarget)}
        onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); toggleAgentMenu(name, e.currentTarget); } }}
        oncontextmenu={(e) => { e.preventDefault(); oncontext(pointOf(e), name); }}
        use:longpress={{ onlongpress: (pt) => oncontext(pt, name) }}
        use:hoverInfo={() => offCardInfo(name)}>
        <div class="ac-top">
          {#if backendIcon(backend)}<img class="ava dim" src={backendIcon(backend)} alt={backend} />{:else}<span class="ava dim">{name.slice(0, 1).toUpperCase()}</span>{/if}
          <!-- No "stopped" word beside the name: the dimmed card already
               says it, and the word made the card wide (owner, 2026-09-03).
               It stays in the aria-label for screen readers. -->
          <span class="a-name">{name}</span>
          <button class="a-start" title={t('hubStartAgain')} aria-label={t('hubStartAgain')}
            disabled={acting}
            onclick={(e) => { e.stopPropagation(); startAgent(name); }}>
            <Icon name="refresh" size={11} />
          </button>
        </div>
      </div>
    {/each}
  <!-- Ad hoc: add an agent to a conversation already in progress. It
       lives INSIDE the strip as its last card — one region, one family
       (owner, 2026-08-25: "加agent应该放到最后，和其他agent放到一起…
       不用强行一直占一个位置") — but STICKY at the right edge, because
       the strip hides its scrollbar and a plainly-scrolling last child
       was invisible with agents present (owner, 2026-08-21: "agent 不为
       空的情况下 我都看不到'加 Agent'的按钮"). Sticky is both at once:
       it sits after the last card when everything fits, and floats at
       the edge while the strip scrolls. Icon-only when agents exist;
       the label only when the roster is empty and the button IS the
       room's entry point (a closed session is not a reason to hide it —
       the spawn opens the session on its way in). -->
    <button class="acard add" class:mini={managedAgents.length > 0 || stopped.length > 0}
      onclick={onadd} title={t('hubSpawn')} aria-label={t('hubSpawn')}>
      <Icon name="plus" size={14} />{#if !managedAgents.length && !stopped.length}<span>{t('hubSpawn')}</span>{/if}
    </button>
  </div>
  </div>
{/if}

{#if menuFor}
  <!-- Actions for one agent, as a context menu beside its chip. It is a
       FIXED layer: the roster scrolls horizontally, and a popover inside
       that scroll container would be clipped by it (which is why this was
       a full-width bar under the roster until 2026-08-19). Same popover
       dialect as the recipient menu — one menu language in this file.

       A stopped agent can resume, filter/configure its retained identity,
       or leave the project. Watch/Interrupt/Restart/Stop need a live pane. -->
  <div class="a-menu pop-layer" class:ready={menuH > 0} role="menu" tabindex="-1"
    style:left="{menuPos.x}px" style:top="{menuPos.y}px"
    style:--pop-origin={menuAnchor ? popOrigin(menuAnchor, menuPos, 'left') : undefined}
    bind:clientWidth={menuW} bind:clientHeight={menuH}>
    <div class="am-who">{menuFor}</div>
    <!-- State + running time live HERE, not on the card: the elapsed
         counter earned its glance-value in the tooltip and menu, and a
         ticking number on every card was permanent motion the roster
         did not need (owner, 2026-08-25: "运行时间…没太必要常显示，
         可以点击三个点显示就行"). -->
    {#if menuAgent}
      <div class="am-vitals">{stateLabel(menuAgent.state)}{menuAgent.since ? ` · ${fmtElapsed(menuAgent.since, tick)}` : ''}</div>
    {/if}
    {#if vitalsFor}<div class="am-vitals">{vitalsFor}</div>{/if}
    <!-- Rising order of consequence, colours saying which is which
         (owner, 2026-08-25: "停止 删除 打断等颜色不一样…停止删除应该
         靠后"): reading verbs first, then config, then amber interrupt
         (a turn cut short — the sys grammar's colour), and the red
         stop/remove close the menu. -->
    {#if stopped.includes(menuFor)}
      <button role="menuitem" disabled={acting} onclick={() => { const n = menuFor; menuFor = ''; startAgent(n); }}>
        <Icon name="refresh" size={12} />{t('hubStartAgain')}
      </button>
    {:else}
      <!-- Choosing the recipient used to be the card's own click; the
           card opens this menu now, so the verb lives here, first. -->
      <button role="menuitem" onclick={() => { const n = menuFor; menuFor = ''; setRecipient(n); }}>
        <Icon name="chat" size={12} />{t('hubTalkTo')}
      </button>
      <button role="menuitem" onclick={() => { const a = managedAgents.find((x) => x.name === menuFor); menuFor = ''; if (a) openDrawer(a); }}>
        <Icon name="terminal" size={12} />{t('hubWatch')}
      </button>
    {/if}
    <!-- The one-agent feed filter, as a verb you can SEE (review,
         2026-09-03) — the double-click stays as the shortcut. A reading
         verb, so it sits with Message/Watch, before configure. Offered
         for a stopped agent too: its history is exactly what you narrow
         to when it is gone. -->
    <button role="menuitem" onclick={() => toggleFilter(menuFor)}>
      <Icon name="search" size={12} />{filterAgent === menuFor ? t('hubFilterExit') : t('hubFilterItem')}
    </button>
    {#if openAgentConfig}
      <!-- The model's HOME is the config page — the menu links to it
           instead of quoting the model name as dead text. -->
      <button role="menuitem" onclick={() => { const n = menuFor; menuFor = ''; openAgentConfig(n); }}>
        <Icon name="gear" size={12} />{t('hubAgentConfig')}
      </button>
    {/if}
    {#if !stopped.includes(menuFor)}
      <button role="menuitem" class="warn" title={t('hubInterruptHint')} onclick={() => { const n = menuFor; menuFor = ''; interrupt(n); }}>
        <Icon name="x" size={12} />{t('hubInterrupt')}
      </button>
      <button role="menuitem" onclick={() => { const n = menuFor; menuFor = ''; restartAgent(n); }}>
        <Icon name="refresh" size={12} />{t('hubRestart')}
      </button>
      <button role="menuitem" class="danger" onclick={() => { const n = menuFor; menuFor = ''; askAction('stop', n); }}>
        <Icon name="stop" size={12} />{t('hubStop')}
      </button>
    {/if}
    <button role="menuitem" class="danger" title={t('hubRemoveHint')} onclick={() => { const n = menuFor; menuFor = ''; askAction('remove', n); }}>
      <Icon name="trash" size={12} />{t('hubRemove')}
    </button>
  </div>
{/if}

<style>

  /* The roster: one line per agent, on every screen size. It answers "who is
     here and are they busy" — anything more was a wall of cards. Metrics run
     TIGHT (owner, 2026-08-25: "感觉整体占的空间不小"): the card is a chip
     with a reading, not a panel. */
  .roster { display: flex; align-items: stretch; padding: 6px 14px; border-bottom: 1px solid var(--border2); min-width: 0; }
  .cards { display: flex; gap: 5px; overflow-x: auto; scrollbar-width: none; flex: 1 1 auto; min-width: 0; }
  /* Skeletons of the coming shape (app.css .skel/.skel-wrap): card-sized in
     the roster, bubble-sized and side-alternating in the feed. */
  .sk-cards { display: flex; gap: 5px; flex: none; }
  .sk-card { width: 96px; min-height: 30px; border-radius: var(--ui-radius-row); }
  .cards::-webkit-scrollbar { display: none; }
  /* A team group (board #74): same-team cards in one dashed frame wearing the
     row radius, the team name as a micro label above them. The frame is
     `flex: none` like a card so the strip scrolls it as one unit. */
  .tgroup { flex: none; display: flex; flex-direction: column; gap: 2px; padding: 2px 4px 4px; border: 1px dashed var(--border2); border-radius: var(--ui-radius-row); }
  .tg-label { display: inline-flex; align-items: center; gap: 3px; font-family: var(--font-mono); font-size: var(--fs-micro); color: var(--text3); padding: 0 2px; line-height: 1.4; }
  .tg-cards { display: flex; gap: 5px; }
  .acard {
    position: relative; flex: none; display: flex; flex-direction: column;
    align-items: stretch; justify-content: center; gap: 1px; overflow: hidden;
    min-height: 30px; background: var(--surface); border: 1px solid var(--border);
    border-radius: var(--ui-radius-row); padding: 3px 8px 3px 5px; cursor: pointer; text-align: left;
    font-size: var(--fs-ui); color: var(--text2);
    /* Its clothes cross-fade (motion.md .state-ctl grammar): .sel's frame and
       wash, .needs' ring, .off/.busy's opacity. A TRANSITION, never an
       animation — waiting is not in motion (see .acard.needs). */
    transition: border-color var(--t-fast), color var(--t-fast), background var(--t-fast), box-shadow var(--t-fast), opacity var(--t-fast);
    -webkit-tap-highlight-color: transparent;
  }
  /* The identity row — what the card used to be in its entirety. */
  .ac-top { display: flex; align-items: center; gap: 6px; }
  /* The sniffed reading: the smallest step on the scale, in monospace so a model
     id and a percentage do not reflow as they change. The cap must FIT a real
     `model · effort` reading (26ch holds `claude-sonnet-4.5 · medium`): at 16ch
     the effort truncated into a trailing `…` that carried no information — the
     owner read it as decoration ("模型名 后面又有一个点点点 … 好像有点多余了",
     2026-08-26). Cards size to content, so short readings stay tight. */
  .ac-vitals {
    font-size: var(--fs-micro); color: var(--meta-ink); line-height: 1.35;
    font-family: var(--font-mono); padding: 0 1px 1px 5px;
    max-width: 26ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  /* One thin horizontal line at the card's bottom edge. Absolute, so it costs no
     height and cannot push the roster taller as it appears. */
  .ac-bar {
    position: absolute; left: 0; right: 0; bottom: 0; height: 2px;
    background: var(--pill-bg); border-radius: 0 0 8px 8px; overflow: hidden;
  }
  .ac-bar > i { display: block; height: 100%; transition: width var(--t-move), background var(--t-move); }
  .acard:hover { border-color: var(--input-border); color: var(--text); }
  .acard.sel { border-color: var(--accent-line); background: var(--accent-bg); color: var(--text); }
  /* Needs a person: the card wears the dot's own amber as a frame (1px border
     + 1px inset ring — inset, because the strip clips outward shadows) and a
     wash, plus the .ac-needs word. Same --status-warn token as stateDotColor,
     no motion (see stateNeedsYou). Selection keeps its accent frame on top;
     the wash, word and dot still say waiting. */
  .acard.needs {
    border-color: var(--status-warn); color: var(--text);
    background: color-mix(in srgb, var(--status-warn) 9%, var(--surface));
    box-shadow: inset 0 0 0 1px var(--status-warn);
  }
  .acard.needs:hover { border-color: var(--status-warn); }
  .acard.needs.sel { border-color: var(--accent-line); box-shadow: inset 0 0 0 1px var(--accent-line); }
  .ac-needs {
    color: var(--status-warn); font-size: var(--fs-micro); font-weight: 650;
    text-transform: uppercase; letter-spacing: 0.4px; white-space: nowrap; flex: none;
  }
  /* The add card rides INSIDE the strip as its last member, STICKY at the
     right edge: after the last card when everything fits, floating at the
     edge while the strip scrolls (the strip hides its scrollbar, so a
     plainly-scrolling add button was invisible — owner, 2026-08-21). The
     opaque ground alone masks cards passing beneath — the left lift shadow
     read as "a strange shadow" between it and the cards, and the borderless
     icon-action grammar applies to a lone + as much as to any icon button
     (owner, 2026-08-28). */
  .acard.add {
    flex-direction: row; align-items: center; gap: 6px; color: var(--text3); padding-right: 12px;
    position: sticky; right: 0; z-index: 1; background: var(--bg);
    border-color: transparent;
    transition: border-color var(--t-fast), color var(--t-fast), background var(--t-fast);
  }
  .acard.add:hover { color: var(--accent); background: var(--surface2); }
  /* With agents present it is a small square +: it stands at the end of the
     family without holding a seat wider than it needs (owner, 2026-08-25:
     "不用强行一直占一个位置"). */
  .acard.add.mini { padding: 0 7px; }
  .acard.off { opacity: 0.55; cursor: default; }
  /* Waking on hover is fine; promising a click is not — the accent border was
     the card selling a card-wide restart it no longer has. */
  .acard.off:hover { opacity: 1; }
  /* An action is in flight: the card stops taking clicks (the handler guards
     too — this is the visible half of that). */
  .acard.busy { opacity: 0.35; pointer-events: none; }
  /* Identity layer: names wear the display face (--font-display), not mono. */
  .a-name { font-family: var(--font-display); font-weight: 600; max-width: 12ch; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .unread { width: 7px; height: 7px; border-radius: 50%; background: var(--status-danger); flex: none; }
  .ava.dim { background: var(--surface2) !important; color: var(--text3); }
  /* The stopped card's ICON avatar: identity stays, colour goes (owner,
     2026-09-05). The letter tile above keeps its surface; the img drops it —
     the live img.ava wears none either. */
  img.ava.dim { background: none !important; filter: grayscale(1); opacity: 0.55; }
  /* The stopped card's direct constructive action: a small borderless
     Resume control. The surrounding card surface opens its menu (board #89). */
  .a-start {
    display: grid; place-items: center; width: 20px; height: 22px; border-radius: 6px;
    background: none; border: none; padding: 0; cursor: pointer; color: var(--text3); flex: none;
  }
  .a-start:hover:not(:disabled) { color: var(--accent); background: var(--surface2); }
  .a-start:disabled { opacity: 0.5; cursor: default; }
  /* The agent action menu: a fixed popover, positioned in JS from the trigger's
     rect (see toggleAgentMenu). It speaks the same dialect as .to-menu — same
     surface, radius, shadow and row metrics — because this file should have ONE
     popover language, not one per feature. Invisible until measured so the
     clamp/flip cannot be seen happening. */
  .a-menu {
    position: fixed; z-index: 24; min-width: 176px; max-width: min(76vw, 280px);
    background: var(--bg); border: 1px solid var(--border); border-radius: var(--ui-radius-panel);
    box-shadow: 0 12px 34px rgba(0,0,0,0.45); padding: 5px;
    display: flex; flex-direction: column; gap: 2px;
    /* Visibility and the intro are the shared .pop-layer atom (app.css). */
  }
  .am-who {
    font-family: var(--font-display); font-weight: 600;
    font-size: var(--fs-meta); color: var(--text3);
    padding: 4px 10px 5px; border-bottom: 1px solid var(--border2); margin-bottom: 3px;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  /* Menu rows are CONTROLS: --ui-font-control (= --fs-sub) is the app's size
     for those, and it is what the action bar used before this became a menu.
     --fs-ui read as oversized for a menu (owner, 2026-08-19). */
  .a-menu button {
    display: flex; align-items: center; gap: 8px; min-height: 36px; width: 100%; text-align: left;
    background: none; border: none; border-radius: var(--ui-radius-control); color: var(--text2);
    padding: 6px 10px; font-size: var(--ui-font-control); cursor: pointer; font-family: var(--font-mono);
  }
  /* Touch contract: a menu row is a tap target, so the phone keeps 44px rows
     even though the type got smaller. */
  :global(.hub-root.compact) .a-menu button { min-height: 44px; }
  .a-menu button:hover { background: var(--surface2); color: var(--text); }
  /* Tones live on the verb itself (owner, 2026-08-25): amber = interrupt (a
     turn cut short, the sys grammar's colour), red = stop/remove. */
  .a-menu button.warn { color: var(--status-warn); }
  .a-menu button.warn:hover { background: color-mix(in srgb, var(--status-warn) 14%, transparent); color: var(--status-warn); }
  .a-menu button.danger { color: var(--status-danger); }
  .a-menu button.danger:hover { background: color-mix(in srgb, var(--status-danger) 14%, transparent); color: var(--status-danger); }
  .a-menu button:disabled { opacity: 0.45; cursor: default; background: none; }
  .a-menu button :global(svg) { flex: none; }
  .am-vitals {
    padding: 0 9px 6px; margin-top: -3px; font-size: var(--fs-meta); color: var(--text3);
    font-family: var(--font-mono); max-width: 240px;
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
</style>
