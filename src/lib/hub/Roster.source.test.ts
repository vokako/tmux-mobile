import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./Roster.svelte', import.meta.url), 'utf8');
const rule = (selector: string) =>
  source.match(new RegExp(`(?:^|\\n)\\s*${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`, 'u'))?.[1] ?? '';

test('context meters surround equal circular avatars with room inside the card (#180)', () => {
  assert.match(source, /style:--ctx-color=\{ctxColor\(a\.vitals\.context_pct\)\}/u);
  assert.match(rule('.acard'), /position: relative/u);
  const bar = rule('.ctx-ring');
  assert.match(bar, /position: absolute/u);
  assert.match(bar, /width: var\(--roster-ring-size\); height: var\(--roster-ring-size\)/u);
  assert.match(rule('.roster'), /--roster-ring-size: 26px/u);
  assert.match(rule('.roster'), /--roster-paint-height: 30px/u);
  assert.match(source, /\.roster \{ --roster-paint-height: 34px; \}/u);
  assert.match(bar, /pointer-events: none/u);
  assert.match(bar, /conic-gradient/u);
  assert.doesNotMatch(bar, /transition/u, 'colour thresholds never pass through intermediate hues');
  assert.match(rule('.ava'), /border-radius: 50%/u);
  assert.doesNotMatch(source, /ac-bar|roster-meter-height/u, 'edge bar removed whole');
});

test('everyone leaves the roster whole for the sole Composer command (#180)', async () => {
  assert.doesNotMatch(source, /acard all|acard\.all|all-ava|allPending|broadcast-glyph/u);
  const css = await readFile(new URL('../../app.css', import.meta.url), 'utf8');
  assert.doesNotMatch(css, /\.hub-root \.acard\.all/u, 'the obsolete corner exception leaves with the old card');
});

test('card paint keeps its inset and the single measured scrolling edge (#180)', () => {
  assert.match(rule('.acard::before'), /inset: var\(--control-paint-inset\) 0/u);
  assert.match(rule('.acard::before'), /pointer-events: none/u);
  assert.match(rule('.roster'), /padding: 0 14px/u);
  assert.match(rule('.roster'), /gap: 0/u);
  assert.match(source, /class="cards edge-fade"[^>]*use:scrollEdges=\{!expanded\}/u);
});

test('actual busy Stop is resident; no invisible action slots widen the card (#180)', () => {
  assert.doesNotMatch(source, /agent-watch|card-quick|onwatch/u);
  assert.doesNotMatch(rule('.agent-stop'), /opacity: 0|position: absolute/u);
  assert.doesNotMatch(rule('.acard'), /grid-template-columns/u);
  assert.match(source, /icon="stop" variant="warn" iconOnly/u);
});

test('one controlled roster replaces the delayed tap menu whole (#168)', () => {
  assert.match(source, /import \{ ALL_TARGET \} from '\.\/hub-composer\.ts'/u);
  assert.match(source, /const ranked = \$derived\(sortAgentsForRoster\(managedAgents\)\)/u,
    'owner 07:09 replaces team adjacency with turn-edge recency');
  assert.match(source, /onselect: setRecipient = \(\) => \{\}/u);
  assert.match(source, /setRecipient\(recipient === name \? '' : name\)/u);
  assert.doesNotMatch(source, /selectTarget\(ALL_TARGET\)/u);
  assert.match(source, /onclick=\{\(e\) => clickAgent\(e, a\.name\)\}/u);
  assert.doesNotMatch(source.slice(source.indexOf('} = $props();')), /\brecipient\s*=(?!=)/u,
    'selection emits intent without locally committing recipient');
  assert.doesNotMatch(source, /menuFor|bind:cardsEl|cardTimer|cardDbl|cardClick|toggleAgentMenu|menuAnchor|menuPos|vitalsFor|menuAgent|a-menu|am-who|am-vitals|groupRoster|tgroup/u);
  // Owner 08:41 (#173) adds Watch as a direct intent, not a local action menu.
  assert.doesNotMatch(source, /onstart|onrestart|onaction|onconfigure|setTimeout|addEventListener|popstate|history\./u);
});

test('Stop consumes parent busy and pending sets through the shared command', () => {
  assert.match(source, /busyNames = \[\], interrupting = \[\]/u);
  assert.match(source, /const renderedStops = \$derived\(pressing \? pressStops : busyNames\)/u);
  assert.match(source, /\{#if renderedStops\.includes\(a\.name\)\}/u);
  assert.match(source, /disabled=\{pending \|\| !busyNames\.includes\(a\.name\)\}/u);
  assert.doesNotMatch(source, /interrupting\.includes\(ALL_TARGET\)/u,
    'pending contains captured member names, never a second all-job sentinel');
  assert.match(source, /interrupting\.includes\(a\.name\)/u);
  assert.equal([...source.matchAll(/icon="stop" variant="warn" iconOnly/gu)].length, 1);
  assert.equal([...source.matchAll(/e\.stopPropagation\(\); interrupt\(/gu)].length, 1);
  assert.doesNotMatch(source, /hubAgentStop|hubAgentInterrupt|busyTargetsFor|\.state\s*===\s*'(?:running|working|waiting|blocked)'/u,
    'the parent owns target membership and dispatch, not a second local busy classifier');
});

test('body mentions use chipExtras, separate from selected state and Stop', () => {
  assert.match(source, /const extras = \$derived\(chipExtras\(composerText, recipient, managedNames\)\)/u);
  assert.match(source, /extras\.includes\(ALL_TARGET\)/u);
  assert.match(source, /extras\.includes\(a\.name\)/u);
  assert.match(source, /class="agent-mention"[^>]*>@<\/span>/u);
  assert.doesNotMatch(source, /mentionTokens|new RegExp|\.split\(['"]@|name="check"/u);
  assert.doesNotMatch(rule('.agent-mention'), /border[^;\n]*dashed/u,
    '#173 permits a reading-filter outline, never a mention outline');
});

test('native selection owns hover and context; stopped slots never select or resume', () => {
  assert.match(source, /<button type="button" class="agent-select"\s+aria-pressed=\{recipient === a\.name\}/u);
  assert.match(source, /use:hoverInfo=\{\(\) => cardInfo\(a\)\}/u);
  assert.match(source, /use:hoverInfo=\{\(\) => offCardInfo\(name\)\}/u);
  assert.match(source, /oncontextmenu=\{\(e\) => \{ e\.preventDefault\(\); oncontext\(pointOf\(e\), a\.name\); \}\}/u);
  assert.match(source, /use:longpress=\{\{ onlongpress: \(pt\) => oncontext\(pt, a\.name\) \}\}/u);
  const off = source.slice(source.indexOf('class="acard off"'), source.indexOf('{/each}', source.indexOf('class="acard off"')));
  assert.match(off, /onclick=\{\(e\) => stoppedMenu\(e, name\)\}/u);
  assert.match(off, /disabled=\{acting\}/u);
  assert.doesNotMatch(off, /selectTarget|interrupt\(|startAgent|a-start|aria-pressed/u);
  assert.doesNotMatch(source, /role="button"|onkeydown|position:\s*fixed/u);
  for (const fact of ['modelLabel(a.vitals.model)', 'fmtElapsed(a.since, tick)', 'stateTone(a.state)', "t('hubHoverTarget')", "t('hubHoverPath')", "t('hubToDmLong')", "t('hubToAlsoHint')"]) {
    assert.ok(source.includes(fact), `retained hover fact: ${fact}`);
  }
});

test('double-click focuses live agents without delay and uses the existing filter intent (#173)', () => {
  assert.match(source, /if \(!coarsePointer\(\) && event\.detail > 1\) return;/u);
  assert.match(source, /if \(!stopped && recipient !== name\) setRecipient\(name\);/u);
  assert.match(source, /onfilter\(name\);/u);
  assert.match(source, /anchor: anchorOf\(trigger\), align: 'left', trigger, keepTriggerClear: true/u);
  assert.match(source, /class:filtered=\{filterAgent === a\.name\}/u);
  assert.match(rule('.acard.filtered::after'), /border: 1px dashed var\(--text2\)/u);
  assert.match(rule('.acard.filtered::after'), /inset: var\(--control-paint-inset\) 0/u,
    'the compact filter border must stay outside the avatar instead of crossing its lower pixels');
  assert.doesNotMatch(source, /setTimeout|clearTimeout/u);
});

test('identity, readiness and motion retain their existing authorities', () => {
  assert.match(source, /\{#if selected\}\s*<div class="roster"/u);
  assert.match(source, /\{#if !roomReady\}\s*<div class="skel-wrap sk-cards" aria-hidden="true">/u);
  assert.match(source, /class:reveal=\{justLoaded\}/u);
  assert.match(source, /class:appear-pop=\{!!rosterBase && !rosterBase\.has\(a\.name\)\}/u);
  assert.match(source, /class:live-dot=\{stateIsLive\(a\.state\)\} style:background=\{stateDotColor\(a\.state\)\}/u);
  assert.doesNotMatch(source, /<span>\{stateLabel\(a\.state\)\}<\/span>|agent-state|stateNeedsYou/u,
    'owner 07:09 retains state wording in accessible/hover facts only');
  assert.match(source, /\{#each orderedAgents as a \(a\.name\)\}/u);
  assert.equal([...source.matchAll(/animate:flip=\{\{ duration: moveMs\(\) \}\}/gu)].length, 2);
  assert.doesNotMatch(source, /^\s*\.st \{/mu, 'the shared Hub dot box is not copied');
  assert.match(source, /const slotBackend = \(name\) => \(selectedRow\?\.slots \?\? \[\]\)\.find\(\(s\) => s\.window_name === name\)\?\.command;/u);
  assert.match(source, /img class="ava dim" src=\{backendIcon\(backend\)\}/u);
  assert.match(rule('img.ava.dim'), /filter: grayscale\(1\); opacity: 0\.55/u);
  assert.match(source, /onclick=\{onadd\}/u);
});

test('density lives in local tokens; full names and native targets do not shrink', () => {
  const roster = rule('.roster');
  assert.match(roster, /--roster-avatar-size: 20px/u);
  assert.match(roster, /--roster-expanded-max: min\(240px, calc\(32dvh \/ var\(--ui-zoom, 1\)\)\)/u);
  assert.match(rule('.agent-select'), /min-height: var\(--control-height\)/u);
  assert.match(rule('.acard'), /width: max-content/u, 'only actual content sets card width');
  assert.match(rule('.cards'), /overflow-x: auto/u);
  assert.match(rule('.cards.expanded'), /overflow-y: auto/u);
  assert.match(rule('.cards.expanded'), /max-height: var\(--roster-expanded-max\)/u);
  assert.match(rule('.cards.expanded'), /flex-wrap: wrap/u);
  assert.doesNotMatch(rule('.cards.expanded .acard'), /width: auto/u);
  assert.match(rule('.cards.expanded .a-name'), /overflow-wrap: anywhere/u);
  assert.match(rule('.cards.expanded .a-name'), /padding-block: var\(--control-paint-inset\)/u);
  assert.match(rule('.agent-marks.unmarked'), /display: none/u);
  assert.doesNotMatch(source, /\.agent-marks:empty/u,
    'Svelte leaves conditional whitespace; CSS :empty kept 12.5px reserved and split ordinary phone names');
  assert.match(rule('.a-name'), /white-space: nowrap/u);
  assert.doesNotMatch(rule('.a-name'), /max-width|ellipsis|overflow: hidden/u);
  assert.doesNotMatch(source, /data-density|URLSearchParams|location\.search/u);
});

test('one in-flow disclosure controls one list without remounting its cards', () => {
  assert.equal([...source.matchAll(/class="cards edge-fade"/gu)].length, 1);
  assert.match(source, /icon="chevron-up" variant="icon"/u);
  assert.match(source, /\{expanded\} controls=\{cardsId\} disabled=\{!roomReady\} onclick=\{onexpand\}/u);
  assert.match(source, /const holdOrder = \$derived\(hovering \|\| focused \|\| pressing\)/u);
  assert.match(source, /onpointerdown=\{beginPress\} onpointerup=\{clearPress\} onlostpointercapture=\{clearPress\}/u);
  assert.doesNotMatch(source, /requestAnimationFrame|cancelAnimationFrame|setTimeout/u,
    'order releases on input lifecycle events, not a timer');
  assert.match(source, /focused = !!cardsEl\?\.contains\(document\.activeElement\)/u,
    'removed controls can lose focus without emitting focusout');
  assert.doesNotMatch(rule('.cards') + rule('.cards.expanded') + rule('.roster'), /transition:[^;\n]*(?:height|width)/u,
    'list geometry never animates; the absolute context fill may change width (#173)');
  assert.doesNotMatch(source, /@keyframes|position: fixed/u);
});

test('idle cards keep no padding for absent commands (#180)', () => {
  assert.doesNotMatch(source, /padding-right: calc|grid-column: 1 \/ 3/u);
  assert.match(rule('.agent-select'), /padding: 0 var\(--roster-card-inset\)/u);
});
