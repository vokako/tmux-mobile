import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./Roster.svelte', import.meta.url), 'utf8');
const rule = (selector: string) =>
  source.match(new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`, 'u'))?.[1] ?? '';

test('one controlled roster replaces the delayed tap menu whole (#168)', () => {
  assert.match(source, /import \{ ALL_TARGET \} from '\.\/hub-composer\.ts'/u);
  assert.match(source, /const rosterGroups = \$derived\(groupRoster\(managedAgents\)\)/u);
  assert.match(source, /onselect: setRecipient = \(\) => \{\}/u);
  assert.match(source, /setRecipient\(recipient === name \? '' : name\)/u);
  assert.match(source, /onclick=\{\(\) => selectTarget\(ALL_TARGET\)\}/u);
  assert.match(source, /onclick=\{\(\) => selectTarget\(a\.name\)\}/u);
  assert.doesNotMatch(source.slice(source.indexOf('} = $props();')), /\brecipient\s*=(?!=)/u,
    'selection emits intent without locally committing recipient');
  assert.doesNotMatch(source, /menuFor|cardsEl|cardTimer|cardDbl|cardClick|toggleAgentMenu|menuAnchor|menuPos|vitalsFor|menuAgent|a-menu|am-who|am-vitals/u);
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
  assert.match(source, /class:needs=\{stateNeedsYou\(a\.state\)\}/u);
  assert.match(rule('.agent-state.needs'), /var\(--status-warn\)/u);
  assert.doesNotMatch(rule('.agent-state.needs'), /animation/u);
  assert.doesNotMatch(source, /^\s*\.st \{/mu, 'the shared Hub dot box is not copied');
  assert.match(source, /const slotBackend = \(name\) => \(selectedRow\?\.slots \?\? \[\]\)\.find\(\(s\) => s\.window_name === name\)\?\.command;/u);
  assert.match(source, /img class="ava dim" src=\{backendIcon\(backend\)\}/u);
  assert.match(rule('img.ava.dim'), /filter: grayscale\(1\); opacity: 0\.55/u);
  assert.match(source, /onclick=\{onadd\}/u);
});

test('density lives in local tokens; full names and native targets do not shrink', () => {
  const roster = rule('.roster');
  assert.match(roster, /--roster-facts-flow: column/u);
  assert.match(roster, /--roster-card-width: 180px/u);
  assert.match(roster, /--roster-select-height: 54px/u);
  assert.match(source, /@media \(any-pointer: coarse\)[\s\S]*--roster-card-width: 184px/u);
  assert.match(rule('.agent-facts'), /flex-direction: var\(--roster-facts-flow\)/u);
  assert.match(rule('.agent-select'), /min-height: max\(var\(--control-height\), var\(--roster-select-height\)\)/u);
  assert.match(rule('.agent-stop'), /flex: none/u);
  assert.match(rule('.acard'), /min-width: var\(--roster-card-width\)/u);
  assert.match(rule('.cards'), /overflow-x: auto/u);
  assert.match(rule('.a-name'), /white-space: nowrap/u);
  assert.doesNotMatch(rule('.a-name'), /max-width|ellipsis|overflow: hidden/u);
  assert.doesNotMatch(source, /data-density|URLSearchParams|location\.search/u);
});
