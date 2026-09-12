import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./Composer.svelte', import.meta.url), 'utf8');
const rule = (selector: string) =>
  source.match(new RegExp(`(?:^|\\n)\\s*${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`, 'u'))?.[1] ?? '';

test('All, attachment and Send are a measured last-line signature (#186)', () => {
  assert.match(source, /class="compose-line"/u);
  assert.match(rule('.compose-line'), /position: relative/u);
  assert.doesNotMatch(rule('.compose-line'), /display: flex/u);
  assert.match(rule('.composer-actions'), /position: absolute/u);
  assert.match(rule('.c-input'), /width: 100%/u);
  assert.match(rule('.c-input'), /min-width: 0/u);
  assert.match(source, /class="all-choice"/u);
  assert.ok(source.indexOf("label={t('hubEveryone')}") < source.indexOf("label={t('hubAttach')}"));
  assert.doesNotMatch(source, /justify-content: space-between/u, 'no separate footer distributes the controls');
  assert.match(source, /signatureLayout\(/u);
});

test('composer measurement observes available geometry, not the textarea it resizes (#180)', () => {
  assert.doesNotMatch(source, /observer\.observe\(composerEl\)/u);
  assert.match(source, /observer\.observe\(available\)/u);
  assert.match(source, /observer\.observe\(actionsEl\)/u);
  assert.match(source, /style\.maxHeight/u, 'a height-only viewport change refreshes the scroll ceiling');
  const observer = source.slice(source.indexOf("let measured = ''"), source.indexOf('observer.observe(available)'));
  assert.match(observer, /actionsEl\.offsetWidth/u, '#186: action-only changes cannot be deduplicated away');
  assert.match(observer, /actionsEl\.offsetHeight/u);
  assert.match(observer, /columnGap/u);
});

test('natural textarea measurement cannot transiently expand the Feed (#180)', () => {
  const grow = source.slice(source.indexOf('function growComposer()'), source.indexOf('let lastShellH'));
  assert.match(grow, /row\.style\.minHeight = `\$\{row\.offsetHeight\}px`/u);
  assert.ok(grow.indexOf('row.style.minHeight = `${row.offsetHeight}px`') < grow.indexOf("el.style.height = 'auto'"));
  assert.match(grow, /finally \{\s*row\.style\.minHeight = previousMin;/u);
  assert.ok(grow.indexOf('row.style.minHeight = previousMin') < grow.indexOf('const shellH'));
});

test('retired recipient and send-arm mechanisms stay removed when measurement returns (#186)', () => {
  assert.doesNotMatch(source, /recipientOpen|toChipW|toExtras|toChipInfo|toMenuH|managedAgents|managedNames|setRecipient/u);
  assert.doesNotMatch(source, /closeRecipient|recipientChanged|intArm|intTimer|intTargets|intWho|recipientBusy|armInterrupt|fireInterrupt/u);
  assert.doesNotMatch(source, /SEND_ZONE|lastLineCollides|c-mirror|ss-ring|stop-spin|int-pill/u);
  assert.doesNotMatch(source, /setTimeout|clearTimeout/u,
    'the keyboard-only sequence expires by elapsed time, not a hidden send-button timer');
});

test('Composer retains its stacking context around the one remaining palette (#168)', () => {
  const composer = /\n  \.composer \{([^}]*)\}/u.exec(source)?.[1] ?? '';
  assert.match(composer, /position:\s*relative/u);
  assert.match(composer, /z-index: 15/u, 'Feed.source pins the feed below this context');
  assert.match(rule('.cmd-menu'), /z-index: 14/u);
  assert.match(source, /:global\(\.hub-root\.compact\) \.composer/u,
    'the compact ancestor crosses the component boundary without another wrapper');
  assert.doesNotMatch(source, /^\s*\.(st|note-dot|live-dot) \{/mu, 'shared atoms are not copied');
});

test('Composer owns local UI state, while transport and capture ordering stay outside (#133)', () => {
  assert.match(source, /composerText = \$bindable\(''\)/u, 'one explicit draft binding');
  assert.doesNotMatch(source, /feedEl|cardsEl|hubPrefs|core\/ws|window\.addEventListener|popstate|pushState/u,
    '#186 font-face completion is a measurement signal, not an input-capture listener');
  assert.match(source, /const paletteBackend = \$derived\(paletteBackendFor\(composerText, recipient, agents\)\);/u);
  assert.match(source, /onmodels: modelsList/u, 'models use the explicit transport command');
  assert.match(source, /if \(shellH !== lastShellH\) \{\s*lastShellH = shellH;\s*onheightchange\(\);/u,
    'height intent fires only when the shell really changed');
  assert.match(source, /onfocus=\{onfocus\}/u);
  assert.match(source, /export function caret\(\) \{ return composerEl\?\.selectionStart; \}/u);
  assert.match(source, /export function focus\(\) \{ composerEl\?\.focus\(\); \}/u);
  assert.match(source, /export function hasTransient\(\) \{ return !!palette; \}/u);
});

test('Composer registers only palette Back and preserves the capture boundary (#168)', () => {
  assert.equal([...source.matchAll(/registerBack\('/gu)].length, 1);
  assert.match(source, /return registerBack\('palette', \(\) => \{ if \(palette\) \{ paletteOff = true; return true; \} return false; \}\);/u);
  assert.match(source, /if \(palette && !t\?\.closest\?\.\('\.cmd-menu, \.compose-shell'\)\) paletteOff = true;/u);
  const escape = source.slice(source.indexOf('export function dismissEscape'), source.indexOf('let composerEl'));
  assert.match(escape, /ctrlCTapAt = null;/u);
  assert.match(escape, /if \(palette\) \{ paletteOff = true; e\.preventDefault\(\); e\.stopPropagation\(\); \}/u);
});

test('Composer attachment rendering and button gates retain the coordinator verdicts (#133)', () => {
  assert.match(source, /disabled=\{!selected \|\| attaching \|\| failed \|\| !sendable\}/u);
  assert.match(source, /\{#each pending as a, i \(a\.key\)\}\s*\n\s*\{#if a\.error\}/u);
  const err = rule('.pend-chip.err');
  assert.match(err, /var\(--status-danger\)/u);
  assert.doesNotMatch(err, /#[0-9a-f]{3,8}\b/iu);
  assert.match(source, /onclick=\{\(\) => removeAttachment\(i\)\}/u);
  assert.match(source, /onpreview\(a\.thumb\)/u, 'the preview opens the original local thumbnail URL');
});

test('the remaining command palette keeps its measured upward placement (#168)', () => {
  assert.match(source, /<div class="cmd-menu pop-layer" class:ready=\{cmdMenuH > 0\} style:--pop-origin="bottom left"[^>]*bind:clientHeight=\{cmdMenuH\}/u);
  // A closed menu forgets its height, so the NEXT opening is measured (and
  // animated) again instead of appearing already `.ready`.
  assert.match(source, /if \(!palette\?\.items\.length\) cmdMenuH = 0;/u, 'the palette re-measures per opening');
  assert.match(rule('.cmd-menu'), /position: absolute; bottom: calc\(100% \+ 6px\)/u, 'the palette keeps its own placement');
});

test('the signature mirror reads the actual prose or command font (#186)', () => {
  // The look mirrors send()'s own branch (slashCommand + a target), so the
  // capsule never promises a command that send() would deliver as prose.
  assert.match(source, /class:cmd=\{composerIsCmd\}/u);
  assert.match(source, /const composerIsCmd = \$derived/u);
  assert.match(rule('.compose-shell.cmd .c-input'), /font-family: var\(--font-mono\)/u);
  // And the height re-measures when the font flips, not just when text changes.
  assert.match(source, /void composerIsCmd;/u);
  assert.match(source, /getComputedStyle\(el\)/u);
  assert.match(source, /measureText\.style\.setProperty\(property, style\.getPropertyValue\(property\)\)/u);
  assert.match(source, /void fonts\.custom; void uiFont\.custom/u);
  assert.match(source, /fontSet\.ready\.then\(remeasure\)/u);
  assert.match(source, /fontSet\.addEventListener\('loadingdone', remeasure\)/u);
  assert.match(source, /fontSet\.removeEventListener\('loadingdone', remeasure\)/u);
  assert.match(source, /onDestroy\(\(\) => measureRoot\?\.remove\(\)\)/u);
});

test('paste and the + button stage attachments through ONE pipeline (board #25)', () => {
  // The composer textarea accepts pasted images/files: onpaste routes through
  // pastedFiles() (files win over co-riding text) into the SAME stageFiles()
  // the file picker uses — a second upload path would drift (token insertion,
  // re-encode, .tmm/uploads layout) the moment either one changed.
  assert.match(source, /class="c-input"[^>]*onpaste=\{onComposerPaste\}/su,
    'the composer textarea must wire onpaste');
  const handler = /function onComposerPaste\(e\) \{([\s\S]*?)\n  \}/u.exec(source)?.[1] ?? '';
  assert.match(handler, /pastedFiles\(e\.clipboardData\)/u, 'files come from the pure extractor');
  assert.match(handler, /preventDefault/u, 'a file paste suppresses the default text insertion');
  // Office/browser pastes ship a PNG rendering beside the words; the words are
  // the paste. The decision is the pure textIsThePaste and it runs BEFORE the
  // default insertion is suppressed (owner, 2026-09-08: "从 ppt 上粘贴过来的文字，
  // 总是被粘贴为了一个图片").
  assert.match(handler, /if \(textIsThePaste\(e\.clipboardData\?\.getData\('text\/plain'\), files\)\) return;[\s\S]*preventDefault/u,
    'text beside an image-only set wins, decided before preventDefault');
  assert.match(handler, /stageFiles\(files\)/u, 'staging is the shared pipeline');
  const picker = /async function onPickFiles\(e\) \{([\s\S]*?)\n  \}/u.exec(source)?.[1] ?? '';
  assert.match(picker, /stageFiles\(files\)/u, 'the + button goes through the same pipeline');
  assert.doesNotMatch(picker, /fsUpload|encodeImage/u, 'the picker holds no upload logic of its own');
});

test('the composer scrollbar follows measured overflow and the placeholder names its destination (#168)', () => {
  // hidden → auto → hidden: the base CSS state is hidden (an empty composer
  // never shows a track), growComposer flips it in its ONE measurement — the
  // same `scrollHeight > maxH + 1` verdict that drives the padding — so a
  // shrink or the post-send reset (growComposer re-runs on composerText)
  // lands back on hidden immediately.
  assert.match(source, /naturalHeight: el\.scrollHeight/u, 'natural height comes from the native field');
  assert.match(source, /el\.style\.overflowY = layout\.overflow \? 'auto' : 'hidden';/u, 'the measured layout owns the scroll verdict');
  assert.match(source, /resize: none; overflow-y: hidden;/u, 'the base state is hidden');
  // Not hidden PERMANENTLY: a long message must really scroll — the .c-input
  // block declares overflow-y exactly once (the hidden base; auto comes only
  // from the JS toggle), and no masking the scrollbar.
  const cInput = source.match(/\n  \.c-input \{[^}]*\}/su)?.[0] ?? '';
  assert.equal([...cInput.matchAll(/overflow-y/g)].length, 1, 'one overflow-y in .c-input, the hidden base');
  assert.ok(!/\.c-input[^}]*scrollbar-width:\s*none/su.test(source), 'the real scrollbar is never masked away');

  // Parent-owned wording can change; these are the existing destination keys,
  // now carried by the textarea rather than an inline recipient control.
  assert.match(source, /recipient === ALL_TARGET \? t\('hubComposerAll'\) : recipient \? t\('hubComposerDm'\)\.replace\('\{name\}', recipient\) : t\('hubComposerRoom'\)/u);
  assert.doesNotMatch(source, /t\('hubTo(?:All|Room)Hint'\)/u,
    'destination labels need no explanatory subtitle');
});

test('measured signature actions reuse shared commands without hardcoded avoidance (#186)', async () => {
  assert.match(source, /import CommandButton from '\.\.\/ui\/CommandButton\.svelte';/u);
  assert.match(source, /<div class="composer-actions" bind:this=\{actionsEl\}>/u);
  assert.match(rule('.composer-actions'), /display: flex/u);
  assert.match(rule('.composer-actions'), /position: absolute; right: 0; bottom: 0/u);
  assert.match(rule('.compose-shell'), /border-radius: 16px/u);
  const appCss = await readFile(new URL('../../app.css', import.meta.url), 'utf8');
  assert.doesNotMatch(appCss, /\.compose-shell\b/u, 'native round corners do not opt into the shared squircle list');
  assert.doesNotMatch(source, /corner-shape:/u, 'this property stays in its shared owner; the Svelte CSS service does not support it yet');
  assert.doesNotMatch(source, /\.send-btn|\.attach-btn|backdrop-filter|SEND_ZONE/u);
  assert.match(source, /if \(empty\) el\.style\.paddingRight = `\$\{controlsWidth \+ gap\}px`/u,
    'only an empty placeholder gives width to the controls, never a typed line');
  assert.match(source, /row\.style\.paddingBottom = layout\.reserved \? `\$\{layout\.reserved\}px` : ''/u);
  assert.match(rule('.compose-line :global(.composer-measure)'), /height: 0/u);
  assert.match(rule('.compose-line :global(.composer-measure)'), /overflow: hidden; visibility: hidden; pointer-events: none/u,
    'the mirror neither paints, handles input nor adds scrollable space');
});

test('double Ctrl+C has only a timestamp and asks the parent about the current recipient (#168)', () => {
  assert.match(source, /interruptible = false/u);
  assert.match(source, /let ctrlCTapAt = null;/u);
  assert.match(source, /if \(e\.repeat\) return;/u);
  assert.match(source, /e\.keyCode !== 229/u);
  assert.match(source, /!e\.metaKey && !e\.altKey/u);
  assert.match(source, /composerEl\?\.selectionStart !== composerEl\?\.selectionEnd/u);
  assert.match(source, /selection && !selection\.isCollapsed/u);
  assert.match(source, /!selected \|\| !recipient \|\| !interruptible/u);
  assert.match(source, /const now = performance\.now\(\);/u);
  assert.match(source, /ctrlCTapAt !== null && now - ctrlCTapAt <= 3000/u);
  assert.match(source, /ctrlCTapAt = null;\s*void oninterrupt\(recipient\);/u);
  assert.equal([...source.matchAll(/oninterrupt\(/gu)].length, 1, 'one keyboard dispatch site, never a send-button path');
  assert.match(source, /void selected; void recipient; void composerText; void interruptible; ctrlCTapAt = null;/u);
});
