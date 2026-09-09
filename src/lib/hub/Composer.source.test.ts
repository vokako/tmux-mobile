import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';
import test from 'node:test';

const source = await readFile(new URL('./Composer.svelte', import.meta.url), 'utf8');
const rule = (selector: string) =>
  source.match(new RegExp(`${selector.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}\\s*\\{([^}]*)\\}`, 'u'))?.[1] ?? '';

test('Composer retains its stacking context and orders its menus inside it (#133)', () => {
  const composer = /\n  \.composer \{([^}]*)\}/u.exec(source)?.[1] ?? '';
  assert.match(composer, /position:\s*relative/u);
  assert.match(composer, /z-index: 15/u, 'Feed.source pins the feed below this context');
  const z = (css: string) => Number(/z-index:\s*(\d+)/u.exec(css)?.[1] ?? NaN);
  assert.ok(z(rule('.cmd-menu')) > z(rule('.to-menu')));
  assert.match(source, /:global\(\.hub-root\.compact\) \.composer/u,
    'the compact ancestor crosses the component boundary without another wrapper');
  assert.doesNotMatch(source, /^\s*\.(st|note-dot|live-dot) \{/mu, 'shared atoms are not copied');
});

test('Composer owns local UI state, while transport and capture ordering stay outside (#133)', () => {
  assert.match(source, /composerText = \$bindable\(''\)/u, 'one explicit draft binding');
  assert.doesNotMatch(source, /feedEl|cardsEl|hubPrefs|core\/ws|addEventListener|popstate|pushState/u);
  assert.match(source, /const paletteBackend = \$derived\(paletteBackendFor\(composerText, recipient, agents\)\);/u);
  assert.match(source, /onmodels: modelsList/u, 'models use the explicit transport command');
  assert.match(source, /if \(shellH !== lastShellH\) \{\s*lastShellH = shellH;\s*onheightchange\(\);/u,
    'height intent fires only when the shell really changed');
  assert.match(source, /onfocus=\{onfocus\}/u);
  assert.match(source, /export function caret\(\) \{ return composerEl\?\.selectionStart; \}/u);
  assert.match(source, /export function focus\(\) \{ composerEl\?\.focus\(\); \}/u);
  const chip = /<button class="to-chip"[\s\S]*?>/u.exec(source)?.[0] ?? '';
  assert.match(chip, /use:hoverInfo=\{toChipInfo\}/u);
  assert.doesNotMatch(chip, /\stitle=/u);
});

test('Composer registers the three original Back guards and preserves capture territories (#133)', () => {
  const guards = {
    recipient: 'if (recipientOpen) { recipientOpen = false; return true; }',
    palette: 'if (palette) { paletteOff = true; return true; }',
    interrupt: 'if (intArm) { intArm = false; return true; }',
  };
  for (const [name, guard] of Object.entries(guards)) {
    assert.ok(source.includes(`registerBack('${name}', () => { ${guard} return false; })`));
  }
  assert.match(source, /return \(\) => \{ for \(const dispose of disposers\) dispose\(\); \};/u);
  assert.match(source, /if \(recipientOpen && !t\?\.closest\?\.\('\.to-wrap'\)\) recipientOpen = false;/u);
  assert.match(source, /if \(palette && !t\?\.closest\?\.\('\.cmd-menu, \.compose-shell'\)\) paletteOff = true;/u);
  assert.match(source, /if \(recipientOpen\) \{ recipientOpen = false; e\.stopPropagation\(\); \}/u);
});

test('Composer attachment rendering and button gates retain the coordinator verdicts (#133)', () => {
  assert.match(source, /disabled=\{!selected \|\| attaching \|\| failed \|\|/u);
  assert.match(source, /\{#each pending as a, i \(a\.key\)\}\s*\n\s*\{#if a\.error\}/u);
  const err = rule('.pend-chip.err');
  assert.match(err, /var\(--status-danger\)/u);
  assert.doesNotMatch(err, /#[0-9a-f]{3,8}\b/iu);
  assert.match(source, /onclick=\{\(\) => removeAttachment\(i\)\}/u);
  assert.match(source, /onpreview\(a\.thumb\)/u, 'the preview opens the original local thumbnail URL');
});

test('the composer\u2019s two upward menus grow from the chip like every other popover (motion.md wave 6)', () => {
  // Both are absolutely positioned above the capsule, so the corner touching
  // their trigger is the bottom-left one; the atom only touches opacity /
  // transform / pointer-events, and rests with `transform: none`, so the
  // menus' own `position: absolute; bottom: calc(100% + 6px)` still places them.

  for (const menu of ['to-menu', 'cmd-menu']) {
    const re = new RegExp(`<div class="${menu} pop-layer" class:ready=\\{(\\w+) > 0\\} style:--pop-origin="bottom left"[^>]*bind:clientHeight=\\{\\1\\}`, 'u');
    assert.match(source, re, `.${menu} is measured, then grows from bottom left`);
  }
  // A closed menu forgets its height, so the NEXT opening is measured (and
  // animated) again instead of appearing already `.ready`.
  assert.match(source, /if \(!recipientOpen\) toMenuH = 0;/u, 'the recipient menu re-measures per opening');
  assert.match(source, /if \(!palette\?\.items\.length\) cmdMenuH = 0;/u, 'the palette re-measures per opening');
  assert.match(rule('.to-menu'), /position: absolute; bottom: calc\(100% \+ 6px\)/u, 'the recipient menu keeps its own placement');
  assert.match(rule('.cmd-menu'), /position: absolute; bottom: calc\(100% \+ 6px\)/u, 'the palette keeps its own placement');
});

test('a command-shaped draft styles the composer, with the mirror in step', () => {

  // The look mirrors send()'s own branch (slashCommand + a target), so the
  // capsule never promises a command that send() would deliver as prose.
  assert.match(source, /class:cmd=\{composerIsCmd\}/u);
  assert.match(source, /const composerIsCmd = \$derived/u);
  // The metrics trap: growComposer's mirror re-lays-out the text to find the
  // last line. If the input flips to monospace and the mirror does not, the
  // send button's collision zone is measured in the wrong font.
  assert.match(
    source,
    /\.compose-shell\.cmd \.c-input, \.compose-shell\.cmd :global\(\.c-mirror\) \{ font-family: var\(--font-mono\)/u,
  );
  // And the height re-measures when the font flips, not just when text changes.
  assert.match(source, /void composerIsCmd;/u);
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

test('the composer scrollbar exists exactly while overflowing, and placeholders are short (board #34)', async () => {
  // hidden → auto → hidden: the base CSS state is hidden (an empty composer
  // never shows a track), growComposer flips it in its ONE measurement — the
  // same `scrollHeight > maxH + 1` verdict that drives the padding — so a
  // shrink or the post-send reset (growComposer re-runs on composerText)
  // lands back on hidden immediately.
  assert.match(source, /const overflowing = el\.scrollHeight > maxH \+ 1;/u, 'one verdict for scrollbar AND padding');
  assert.match(source, /el\.style\.overflowY = overflowing \? 'auto' : 'hidden';/u, 'the toggle rides that verdict');
  assert.match(source, /resize: none; overflow-y: hidden;/u, 'the base state is hidden');
  // Not hidden PERMANENTLY: a long message must really scroll — the .c-input
  // block declares overflow-y exactly once (the hidden base; auto comes only
  // from the JS toggle), and no masking the scrollbar.
  const cInput = source.match(/\n  \.c-input \{[^}]*\}/su)?.[0] ?? '';
  assert.equal([...cInput.matchAll(/overflow-y/g)].length, 1, 'one overflow-y in .c-input, the hidden base');
  assert.ok(!/\.c-input[^}]*scrollbar-width:\s*none/su.test(source), 'the real scrollbar is never masked away');

  // The placeholders name the reach; the menu labels the destinations once.
  const i18n = await readFile(new URL('../core/i18n.svelte.ts', import.meta.url), 'utf8');
  assert.equal([...i18n.matchAll(/hubComposerAll: 'Message every agent…',/g)].length, 1, 'EN all is short');
  assert.equal([...i18n.matchAll(/hubComposerRoom: 'Leave a note…',/g)].length, 1, 'EN room is short');
  assert.equal([...i18n.matchAll(/hubComposerAll: '发给所有 agent…',/g)].length, 1, 'zh all is short');
  assert.equal([...i18n.matchAll(/hubComposerRoom: '留一句话…',/g)].length, 1, 'zh room is short');
  assert.doesNotMatch(source, /t\('hubTo(?:All|Room)Hint'\)/u,
    'destination labels need no explanatory subtitle');
});
