import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./Roster.svelte', import.meta.url), 'utf8');
const rule = (selector: string) =>
  source.match(new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`, 'u'))?.[1] ?? '';

test('one controlled roster replaces the delayed tap menu whole (#168)', () => {
  assert.match(source, /import \{ ALL_TARGET \} from '\.\/hub-composer\.ts'/u);
  assert.match(source, /const ranked = \$derived\(sortAgentsForRoster\(managedAgents\)\)/u,
    'owner 07:09 replaces team adjacency with turn-edge recency');
  assert.match(source, /onselect: setRecipient = \(\) => \{\}/u);
  assert.match(source, /setRecipient\(recipient === name \? '' : name\)/u);
  assert.match(source, /onclick=\{\(\) => selectTarget\(ALL_TARGET\)\}/u);
  assert.match(source, /onclick=\{\(\) => selectTarget\(a\.name\)\}/u);
  assert.doesNotMatch(source.slice(source.indexOf('} = $props();')), /\brecipient\s*=(?!=)/u,
    'selection emits intent without locally committing recipient');
  assert.doesNotMatch(source, /menuFor|bind:cardsEl|cardTimer|cardDbl|cardClick|toggleAgentMenu|menuAnchor|menuPos|vitalsFor|menuAgent|a-menu|am-who|am-vitals|groupRoster|tgroup/u);
  assert.doesNotMatch(source, /onfilter|onwatch|onstart|onrestart|onaction|onconfigure|ondblclick|setTimeout|addEventListener|popstate|history\./u);
});

test('Stop consumes parent busy and pending sets through the shared command', () => {
  assert.match(source, /busyNames = \[\], interrupting = \[\]/u);
  assert.match(source, /\{#if busyNames\.includes\(a\.name\)\}/u);
  assert.match(source, /\{#if busyNames\.length\}/u);
  assert.doesNotMatch(source, /interrupting\.includes\(ALL_TARGET\)/u,
    'pending contains captured member names, never a second all-job sentinel');
  assert.match(source, /busyNames\.some\(\(name\) => interrupting\.includes\(name\)\)/u);
  assert.match(source, /interrupting\.includes\(a\.name\)/u);
  assert.equal([...source.matchAll(/<CommandButton label=/gu)].length, 2);
  assert.equal([...source.matchAll(/icon="stop" variant="secondary" iconOnly/gu)].length, 2);
  assert.equal([...source.matchAll(/e\.stopPropagation\(\); interrupt\(/gu)].length, 2);
  assert.doesNotMatch(source, /hubAgentStop|hubAgentInterrupt|busyTargetsFor|\.state\s*===\s*'(?:running|working|waiting|blocked)'/u,
    'the parent owns target membership and dispatch, not a second local busy classifier');
});

test('body mentions use chipExtras, separate from selected state and Stop', () => {
  assert.match(source, /const extras = \$derived\(chipExtras\(composerText, recipient, managedNames\)\)/u);
  assert.match(source, /extras\.includes\(ALL_TARGET\)/u);
  assert.match(source, /extras\.includes\(a\.name\)/u);
  assert.match(source, /class="agent-mention"[^>]*>@<\/span>/u);
  assert.doesNotMatch(source, /mentionTokens|new RegExp|\.split\(['"]@|name="check"|border[^;\n]*dashed/u);
});

test('native selection owns hover and context; stopped slots never select or resume', () => {
  assert.match(source, /<button type="button" class="agent-select"\s+aria-pressed=\{recipient === a\.name\}/u);
  assert.match(source, /use:hoverInfo=\{\(\) => cardInfo\(a\)\}/u);
  assert.match(source, /use:hoverInfo=\{\(\) => offCardInfo\(name\)\}/u);
  assert.match(source, /oncontextmenu=\{\(e\) => \{ e\.preventDefault\(\); oncontext\(pointOf\(e\), a\.name\); \}\}/u);
  assert.match(source, /use:longpress=\{\{ onlongpress: \(pt\) => oncontext\(pt, a\.name\) \}\}/u);
  const off = source.slice(source.indexOf('class="acard off"'), source.indexOf('{/each}', source.indexOf('class="acard off"')));
  assert.match(off, /onclick=\{\(e\) => oncontext\(pointOf\(e\), name\)\}/u);
  assert.match(off, /disabled=\{acting\}/u);
  assert.doesNotMatch(off, /selectTarget|interrupt\(|startAgent|a-start|aria-pressed/u);
  assert.doesNotMatch(source, /role="button"|onkeydown|position:\s*fixed/u);
  for (const fact of ['modelLabel(a.vitals.model)', 'fmtElapsed(a.since, tick)', 'stateTone(a.state)', "t('hubHoverTarget')", "t('hubHoverPath')", "t('hubToDmLong')", "t('hubToAllLong')", "t('hubToAlsoHint')"]) {
    assert.ok(source.includes(fact), `retained hover fact: ${fact}`);
  }
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
  assert.match(rule('.acard'), /grid-template-columns: minmax\(0, 1fr\) var\(--control-height\)/u,
    'the Stop track stays reserved when a turn ends');
  assert.match(rule('.cards'), /overflow-x: auto/u);
  assert.match(rule('.cards.expanded'), /overflow-y: auto/u);
  assert.match(rule('.cards.expanded'), /max-height: var\(--roster-expanded-max\)/u);
  assert.match(source, /repeat\(2, minmax\(0, 1fr\)\)/u);
  assert.match(source, /repeat\(4, minmax\(0, 1fr\)\)/u);
  assert.match(rule('.cards.expanded .a-name'), /overflow-wrap: anywhere/u);
  assert.match(rule('.cards.expanded .agent-marks.unmarked'), /display: none/u);
  assert.doesNotMatch(source, /\.agent-marks:empty/u,
    'Svelte leaves conditional whitespace; CSS :empty kept 12.5px reserved and split ordinary phone names');
  assert.match(rule('.a-name'), /white-space: nowrap/u);
  assert.doesNotMatch(rule('.a-name'), /max-width|ellipsis|overflow: hidden/u);
  assert.doesNotMatch(source, /data-density|URLSearchParams|location\.search/u);
});

test('one in-flow disclosure controls one list without remounting its cards', () => {
  assert.equal([...source.matchAll(/class="cards"/gu)].length, 1);
  assert.match(source, /icon="chevron-up" variant="icon"/u);
  assert.match(source, /\{expanded\} controls=\{cardsId\} disabled=\{!roomReady\} onclick=\{onexpand\}/u);
  assert.match(source, /const holdOrder = \$derived\(hovering \|\| focused \|\| pressing\)/u);
  assert.match(source, /onpointerdown=\{beginPress\} onpointerup=\{clearPress\} onlostpointercapture=\{clearPress\}/u);
  assert.doesNotMatch(source, /requestAnimationFrame|cancelAnimationFrame|setTimeout/u,
    'order releases on input lifecycle events, not a timer');
  assert.match(source, /focused = !!cardsEl\?\.contains\(document\.activeElement\)/u,
    'removed controls can lose focus without emitting focusout');
  assert.doesNotMatch(source, /transition:[^;\n]*(?:height|width)|@keyframes|position: fixed/u);
});

test('idle cards keep their width but give the vacant Stop track back to selection', () => {
  const idle = rule('.acard:not(.has-stop):not(.off) .agent-select');
  assert.match(idle, /grid-column: 1 \/ -1/u);
  assert.match(idle, /padding-right: calc\(6px \+ var\(--control-height\)\)/u,
    'the hit box spans the object while the content budget stays stable');
});
