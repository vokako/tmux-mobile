<script>
  import { onDestroy, tick as settled } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import CommandButton from '../ui/CommandButton.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { slashCommand, commandPalette, readlineEdit, pastedFiles, textIsThePaste } from './hub.ts';
  import { ALL_TARGET, paletteBackendFor, signatureLayout } from './hub-composer.ts';
  import { fonts, uiFont } from '../app/fonts.svelte.ts';

  let {
    selected = '', compact = false, recipient = '', composerText = $bindable(''),
    roomReady = false,
    agents = [], interruptible = false,
    pending = [], attaching = false, failed = false, sendable = false,
    onsend: send = () => {},
    onstage: stageFiles = async () => {}, onremove: removeAttachment = () => {},
    onmodels: modelsList = async () => ({ models: [] }), oninterrupt = async (_target) => {},
    onpreview = () => {}, onfocus = () => {}, onheightchange = () => {},
    registerBack = null,
  } = $props();

  // Hub owns transport and capture-listener ordering; the palette is the
  // composer's only transient layer. The keyboard sequence has no visible state.
  export function caret() { return composerEl?.selectionStart; }
  export function focus() { composerEl?.focus(); }
  export function hasTransient() { return !!palette; }
  export function dismissOutside(e) {
    const t = e.target;
    if (palette && !t?.closest?.('.cmd-menu, .compose-shell')) paletteOff = true;
  }
  export function dismissEscape(e) {
    ctrlCTapAt = null;
    if (palette) { paletteOff = true; e.preventDefault(); e.stopPropagation(); }
  }

  let composerEl = $state(null);
  let shellEl = $state(null);
  let actionsEl = $state(null);
  let ctrlCTapAt = null;
  let measureRoot = null;
  let measureText = null;
  let measureValue = null;

  function textBoxes(el, style) {
    if (!measureRoot) {
      const doc = el.ownerDocument;
      measureRoot = doc.createElement('div');
      measureRoot.className = 'composer-measure';
      measureRoot.setAttribute('aria-hidden', 'true');
      measureText = doc.createElement('div');
      measureValue = doc.createTextNode('');
      const end = doc.createElement('span');
      end.textContent = '\u200b'; // Preserve a final empty line, like the textarea caret.
      measureText.append(measureValue, end);
      measureRoot.append(measureText);
      el.parentElement.append(measureRoot);
    }
    // Copy computed metrics, not a guessed character width or another font stack.
    for (const property of [
      'font-family', 'font-size', 'font-weight', 'font-style', 'font-stretch', 'font-variant',
      'font-variant-alternates', 'font-variant-ligatures', 'font-variant-numeric',
      'font-variation-settings',
      'font-kerning', 'line-height', 'letter-spacing', 'word-spacing', 'text-indent',
      'text-transform', 'text-align', 'direction', 'tab-size', 'white-space',
      'overflow-wrap', 'word-break', 'line-break', 'padding-top', 'padding-right',
      'padding-bottom', 'padding-left', 'box-sizing',
    ]) measureText.style.setProperty(property, style.getPropertyValue(property));
    const width = (parseFloat(style.width) || el.offsetWidth) - (el.offsetWidth - el.clientWidth);
    measureText.style.width = `${width}px`;
    measureValue.data = el.value;
    const range = el.ownerDocument.createRange();
    range.selectNodeContents(measureText);
    const origin = measureText.getBoundingClientRect();
    // Range rectangles include CSS zoom; local differences remove page slides.
    const scale = origin.width / width || 1;
    return [...range.getClientRects()].filter(rect => rect.width > 0 && rect.height > 0).map(rect => ({
      left: (rect.left - origin.left) / scale, right: (rect.right - origin.left) / scale,
      top: (rect.top - origin.top) / scale, bottom: (rect.bottom - origin.top) / scale,
    }));
  }
  onDestroy(() => measureRoot?.remove());

  /** One measurement transaction owns natural height and the signature band. */
  function growComposer() {
    const el = composerEl;
    const row = el?.parentElement;
    if (!el || !row || !actionsEl || !el.clientWidth) return;
    const previousMin = row.style.minHeight;
    // Measuring a shorter textarea must not transiently expand the Feed and
    // clamp its scrollTop. Keep the row in place until the real height is ready.
    row.style.minHeight = `${row.offsetHeight}px`;
    try {
      row.style.paddingBottom = '';
      el.style.paddingRight = '';
      el.style.height = 'auto';
      el.style.overflowY = 'hidden';
      const style = getComputedStyle(el);
      const controlsWidth = actionsEl.offsetWidth, controlsHeight = actionsEl.offsetHeight;
      const actionsStyle = getComputedStyle(actionsEl);
      const gap = parseFloat(actionsStyle.columnGap) || 0;
      const paintInset = parseFloat(actionsStyle.getPropertyValue('--control-paint-inset')) || 0;
      const lift = parseFloat(actionsStyle.bottom) || 0;
      const inkInset = Math.max(0, ((parseFloat(style.lineHeight) || 0) - (parseFloat(style.fontSize) || 0)) / 2);
      const empty = !el.value;
      if (empty && measureValue) measureValue.data = '';
      const layout = signatureLayout({
        width: parseFloat(style.width) || el.clientWidth, naturalHeight: el.scrollHeight,
        maxHeight: parseFloat(style.maxHeight) || Infinity,
        controlsWidth, controlsHeight, gap, empty, paintInset, inkInset, lift,
        textRects: empty ? [] : textBoxes(el, style),
      });
      if (empty) el.style.paddingRight = `${controlsWidth + gap}px`;
      row.style.paddingBottom = layout.reserved ? `${layout.reserved}px` : '';
      el.style.overflowY = layout.overflow ? 'auto' : 'hidden';
      el.style.height = `${layout.inputHeight}px`;
      if (layout.overflow) textBoxes(el, getComputedStyle(el)); // Match the native scrollbar gutter too.
    } finally {
      row.style.minHeight = previousMin;
    }
    // Attachments and actions count toward the shell; unchanged height must
    // not re-park the feed on every keystroke.
    const shellH = shellEl?.offsetHeight ?? el.offsetHeight;
    if (shellH !== lastShellH) {
      lastShellH = shellH;
      onheightchange();
    }
  }
  let lastShellH = 0;
  $effect(() => {
    void composerText;
    void pending;
    void compact;
    void composerIsCmd;
    void fonts.custom; void uiFont.custom;
    growComposer();
  });
  $effect(() => {
    const fontSet = document.fonts;
    if (!fontSet) return;
    let live = true;
    const remeasure = () => { if (live) growComposer(); };
    fontSet.ready.then(remeasure);
    fontSet.addEventListener('loadingdone', remeasure);
    return () => {
      live = false;
      fontSet.removeEventListener('loadingdone', remeasure);
    };
  });
  $effect(() => {
    const available = shellEl?.parentElement?.parentElement;
    if (!composerEl || !actionsEl || !available) return;
    let measured = '';
    // The chat column supplies space; commands supply the signature box. Observing
    // the textarea itself would observe our own height writes and form a loop.
    const observer = new ResizeObserver(() => {
      const style = getComputedStyle(composerEl);
      const next = `${composerEl.clientWidth}:${style.maxHeight}:${style.minHeight}:${style.font}:`
        + `${actionsEl.offsetWidth}:${actionsEl.offsetHeight}:${getComputedStyle(actionsEl).columnGap}`;
      if (next === measured) return;
      measured = next;
      growComposer();
    });
    observer.observe(available);
    observer.observe(actionsEl);
    return () => observer.disconnect();
  });

  let fileEl = $state(null);

  async function onPickFiles(e) {
    const files = [...(e.target.files || [])];
    e.target.value = ''; // same file re-pickable
    await stageFiles(files);
  }

  /** Picker and paste retain one staging pipeline. Office image renderings
   * beside real text remain native text pastes, not duplicate attachments. */
  function onComposerPaste(e) {
    const files = pastedFiles(e.clipboardData);
    if (!files.length) return;
    if (textIsThePaste(e.clipboardData?.getData('text/plain'), files)) return;
    e.preventDefault();
    stageFiles(files);
  }

  // Match Hub's command branch: an explicit addressee or recipient must exist.
  const composerIsCmd = $derived.by(() => {
    const c = slashCommand(composerText.trim());
    return !!(c && (c.to || recipient));
  });
  const composerLabel = $derived(recipient === ALL_TARGET ? t('hubComposerAll') : recipient ? t('hubComposerDm').replace('{name}', recipient) : t('hubComposerRoom'));

  // Slash completion retains its backend cache and command/argument stages.
  let cmdModels = $state({});
  let paletteIdx = $state(0);
  let paletteOff = $state(false);
  const paletteBackend = $derived(paletteBackendFor(composerText, recipient, agents));
  const palette = $derived(paletteOff ? null : commandPalette(composerText, cmdModels[paletteBackend] ?? [], paletteBackend));
  $effect(() => { void composerText; paletteOff = false; });
  $effect(() => { void palette; paletteIdx = 0; });
  $effect(() => {
    const backend = paletteBackend;
    if (!palette || cmdModels[backend]) return;
    modelsList(backend || 'kiro').then((r) => { cmdModels = { ...cmdModels, [backend]: r.models ?? [] }; }).catch(() => {});
  });

  function acceptCompletion(item) {
    if (!palette) return;
    const head = composerText.slice(0, palette.from);
    composerText = `${head}${item.value}${palette.more ? ' ' : ''}`;
    paletteIdx = 0;
    composerEl?.focus();
    requestAnimationFrame(() => composerEl?.setSelectionRange(composerText.length, composerText.length));
  }

  // The readline kill buffer belongs to this mounted composer, not a room.
  let killBuf = '';
  let killChain = false;

  function onComposerKey(e) {
    if (e.key === 'Escape') ctrlCTapAt = null;
    if (e.ctrlKey && !e.metaKey && !e.altKey && !e.isComposing && composerEl) {
      const r = readlineEdit({
        key: e.key.toLowerCase(), text: composerEl.value,
        start: composerEl.selectionStart, end: composerEl.selectionEnd,
        kill: killBuf, killing: killChain,
      });
      if (r) {
        e.preventDefault();
        killBuf = r.kill;
        killChain = r.killing;
        composerText = r.text;
        settled().then(() => composerEl?.setSelectionRange(r.caret, r.caret));
        return;
      }
    }
    killChain = false;
    // Only two genuine empty-input Ctrl+C presses count. Native copy, including
    // whitespace and selections outside the textarea, never becomes Stop.
    if (e.ctrlKey && !e.metaKey && !e.altKey && !e.shiftKey && !e.isComposing && e.keyCode !== 229
        && e.key.toLowerCase() === 'c') {
      if (e.repeat) return;
      const selection = typeof getSelection === 'function' ? getSelection() : null;
      if (composerText || composerEl?.value || composerEl?.selectionStart !== composerEl?.selectionEnd
          || (selection && !selection.isCollapsed) || !selected || !recipient || !interruptible) {
        ctrlCTapAt = null;
        return;
      }
      e.preventDefault();
      const now = performance.now();
      if (ctrlCTapAt !== null && now - ctrlCTapAt <= 3000) {
        ctrlCTapAt = null;
        void oninterrupt(recipient);
      } else {
        ctrlCTapAt = now;
      }
      return;
    }
    // The palette still owns arrows, Tab and Enter before ordinary sending.
    if (palette?.items.length) {
      if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
        e.preventDefault();
        const step = e.key === 'ArrowDown' ? 1 : -1;
        paletteIdx = (paletteIdx + step + palette.items.length) % palette.items.length;
        return;
      }
      if (e.key === 'Tab' || (e.key === 'Enter' && !e.shiftKey && !e.isComposing)) {
        e.preventDefault();
        acceptCompletion(palette.items[paletteIdx] ?? palette.items[0]);
        return;
      }
      if (e.key === 'Escape') { e.preventDefault(); paletteOff = true; return; }
    }
    if (e.key !== 'Enter' || e.shiftKey || e.isComposing) return;
    if (compact) return;
    e.preventDefault();
    send();
  }

  $effect(() => { void selected; void recipient; void composerText; void interruptible; ctrlCTapAt = null; });

  let cmdMenuH = $state(0);
  $effect(() => { if (!palette?.items.length) cmdMenuH = 0; });

  $effect(() => {
    if (!registerBack) return;
    return registerBack('palette', () => { if (palette) { paletteOff = true; return true; } return false; });
  });
</script>

<div class="composer">
  <div class="compose-shell" class:cmd={composerIsCmd} bind:this={shellEl}>
  {#if palette?.items.length}
    <div class="cmd-menu pop-layer" class:ready={cmdMenuH > 0} style:--pop-origin="bottom left" role="listbox" tabindex="-1" bind:clientHeight={cmdMenuH}>
      {#each palette.items as it, i (it.value)}
        <button class="cmd-opt" class:cur={i === paletteIdx} role="option"
          aria-selected={i === paletteIdx}
          onpointerenter={() => (paletteIdx = i)}
          onclick={() => acceptCompletion(it)}>
          <span class="cmd-name">{it.value}</span>
          <span class="cmd-hint">{it.hint}</span>
        </button>
      {/each}
    </div>
  {/if}
  <div class="compose-line">
  <textarea class="c-input" rows="1" bind:this={composerEl} bind:value={composerText}
    aria-label={composerLabel} placeholder={composerLabel}
    oninput={growComposer}
    onkeydown={onComposerKey}
    onpaste={onComposerPaste}
    onfocus={onfocus}
  ></textarea>
  <!-- On the phone the two controls are a dense group (.compact-tools, the
       Files-head pitch): 32px each instead of 44, so the field keeps 24px more
       for text (board #228, owner: "发送区的按钮" tighter). -->
  <div class="composer-actions" class:compact-tools={compact} bind:this={actionsEl}>
    <CommandButton variant="icon" icon="plus" label={t('hubAttach')} disabled={!selected || attaching}
      pending={attaching} onclick={() => fileEl?.click()} />
    <CommandButton variant="primary" iconOnly round icon="send-up" label={t('hubSend')}
      disabled={!selected || attaching || failed || !sendable} onclick={send} />
  </div>
  </div>
  {#if pending.length}
    <div class="pend-row">
      {#each pending as a, i (a.key)}
        {#if a.error}
          <span class="pend-chip err appear-pop" title={`${a.name} — ${a.error}`}>
            <Icon name="info" size={12} />
            <span class="pend-name">{a.name}</span>
            <span class="pend-why">{a.error}</span>
            <button class="pend-x" aria-label={t('hubRemoveAttachment')}
              onclick={() => removeAttachment(i)}>
              <Icon name="x" size={11} />
            </button>
          </span>
        {:else if a.kind === 'image'}
          <span class="pend-thumb appear-pop" title={`[img:${a.n}] ${a.name}`}>
            <button class="pend-view" aria-label={a.name}
              onclick={() => onpreview(a.thumb)}>
              <img src={a.thumb} alt={a.name} />
            </button>
            <span class="pend-n">{a.n}</span>
            <button class="pend-x on-img" aria-label={t('hubRemoveAttachment')}
              onclick={() => removeAttachment(i)}>
              <Icon name="x" size={10} />
            </button>
          </span>
        {:else}
          <span class="pend-chip appear-pop" title={`[file:${a.n}] ${a.path}`}>
            <Icon name="file" size={12} />
            <span class="pend-name">{a.name}</span>
            <button class="pend-x" aria-label={t('hubRemoveAttachment')}
              onclick={() => removeAttachment(i)}>
              <Icon name="x" size={11} />
            </button>
          </span>
        {/if}
      {/each}
    </div>
  {/if}
  <input type="file" multiple hidden bind:this={fileEl} onchange={onPickFiles} />
  </div>
</div>

<style>
  .composer {
    position: relative; z-index: 15; flex: none; min-width: 0;
    padding: 0 var(--composer-inset) 10px;
  }
  /* An input's radius (--ui-radius-control), not 16px: on a 32px desktop
     shell 16px was a full semicircle and text near the edge read as squeezed
     into it (owner, 2026-09-17: "在电脑上这个输入框左边是一个半圆…感觉被挤到了半圆里面
     一样"; board #203). The inline inset is the row content inset so the first
     glyph stands clear of the corner.
     No top edge of its own (board #236): the shell is the TOOLBAR under a
     Chrome-style tab strip — the Roster draws the shared top hairline at the
     same --composer-inset and the lit tab paints over it, so the tab and the
     shell read as one surface. Top corners square where the strip meets the
     sides; the bottom keeps the control radius. */
  .compose-shell {
    position: relative; border: 1px solid var(--border); border-top: 0;
    border-radius: 0 0 var(--ui-radius-control) var(--ui-radius-control);
    background: var(--bubble-in); padding: var(--tool-inset-block) var(--menu-item-padding-x);
  }
  .compose-shell:focus-within { border-color: var(--accent-line); }
  .compose-shell.cmd { border-color: color-mix(in srgb, var(--accent) 45%, transparent); background: color-mix(in srgb, var(--accent) 6%, var(--bubble-in)); }
  .compose-shell.cmd .c-input { font-family: var(--font-mono); }
  /* The line's own height is captured HERE, before a dense group inside
     redefines --control-height (board #229): the controls centre on it. */
  .compose-line { position: relative; min-width: 0; box-sizing: border-box; --composer-line-h: var(--control-height); }
  .c-input {
    display: block; min-width: 0; width: 100%; box-sizing: border-box; min-height: var(--control-height);
    max-height: calc(30vh / var(--ui-zoom, 1)); padding: max(2px, calc((var(--control-height) - 1.5em) / 2)) 0;
    border: 0; outline: none; background: transparent; color: var(--text);
    font: var(--fs-body)/1.5 var(--font-ui); resize: none; overflow-y: hidden; overflow-x: hidden;
  }
  .c-input::placeholder { color: var(--text3); }
  /* The controls stand centred on the field's single-line height: on the
     phone the group is 32px inside a 44px line (#228), and glued to the
     bottom it sat 6px low (owner 2026-09-21: "消息发送按钮在详细框里行没有上下居中
     对齐", board #229). Desktop: 28 in 28 → 0, unchanged. growComposer reads
     this `bottom` back as `lift`, so the collision band moves with it. */
  .composer-actions {
    position: absolute; right: 0; bottom: calc((var(--composer-line-h) - var(--control-height)) / 2);
    display: flex; align-items: center; gap: var(--tool-gap);
  }
  .compose-line :global(.composer-measure) {
    position: absolute; top: 0; left: 0; width: 100%; height: 0;
    overflow: hidden; visibility: hidden; pointer-events: none;
  }
  .pend-row { display: flex; flex-wrap: wrap; gap: 6px; padding-block: 5px; }
  .pend-chip {
    display: inline-flex; align-items: center; gap: 5px; max-width: 100%; padding: 3px 7px;
    border: 1px solid var(--border); border-radius: 6px; background: var(--surface);
    color: var(--text2); font-size: var(--fs-sub);
  }
  .pend-chip.err { color: var(--status-danger); border-color: var(--status-danger); }
  .pend-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pend-why { max-width: 15em; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .pend-x {
    display: grid; place-items: center; width: var(--control-height); height: var(--control-height);
    flex: none; padding: 0; border: 0; border-radius: var(--ui-radius-control); background: transparent; color: inherit;
  }
  .pend-thumb { display: flex; align-items: center; position: relative; }
  .pend-view { padding: 0; border: 0; background: transparent; }
  .pend-view img { display: block; max-height: 48px; max-width: 100px; }
  .pend-n { position: absolute; left: 2px; top: 2px; font-size: var(--fs-micro); }
  .cmd-menu {
    position: absolute; bottom: calc(100% + 6px); left: 0; right: 0; z-index: 14;
    max-height: calc(40vh / var(--ui-zoom, 1)); overflow-y: auto; background: var(--bg);
    border: 1px solid var(--border); border-radius: var(--ui-radius-panel); padding: 5px;
  }
  .cmd-opt {
    display: flex; align-items: center; gap: 10px; width: 100%; min-height: 32px; padding: 6px 10px;
    border: 0; border-radius: var(--ui-radius-control); background: none; color: var(--text2);
    font: var(--fs-sub)/1.4 var(--font-mono); text-align: left;
  }
  .cmd-opt:hover, .cmd-opt.cur { background: var(--surface2); color: var(--text); }
  .cmd-name { flex: none; font-weight: 600; color: var(--accent); }
  .cmd-hint { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  @media (any-pointer: coarse) { .cmd-opt { min-height: 44px; } }
  :global(.hub-root.compact) .composer { padding: 0 var(--composer-inset) 8px; }
  :global(.hub-root.compact) .c-input { max-height: calc(28vh / var(--ui-zoom, 1)); }
</style>
