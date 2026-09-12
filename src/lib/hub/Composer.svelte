<script>
  import { tick as settled } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import CommandButton from '../ui/CommandButton.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { slashCommand, commandPalette, readlineEdit, pastedFiles, textIsThePaste } from './hub.ts';
  import { ALL_TARGET, paletteBackendFor } from './hub-composer.ts';

  let {
    selected = '', compact = false, recipient = '', composerText = $bindable(''),
    roomReady = false, allMenuOpen = false, onall = (_event) => {},
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

  /** Natural textarea growth is the only height calculation; the controls
   * share its row and the whole shell includes any attachment rows. */
  function growComposer() {
    const el = composerEl;
    if (!el) return;
    const row = el.parentElement;
    const previousMin = row?.style.minHeight ?? '';
    // Measuring a shorter textarea must not transiently expand the Feed and
    // clamp its scrollTop. Keep the row in place until the real height is ready.
    if (row) row.style.minHeight = `${row.offsetHeight}px`;
    try {
      el.style.height = 'auto';
      const maxH = parseFloat(getComputedStyle(el).maxHeight) || Infinity;
      const overflowing = el.scrollHeight > maxH + 1;
      el.style.overflowY = overflowing ? 'auto' : 'hidden';
      el.style.height = `${Math.min(el.scrollHeight, maxH)}px`;
    } finally {
      if (row) row.style.minHeight = previousMin;
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
    growComposer();
  });
  $effect(() => {
    const available = shellEl?.parentElement?.parentElement;
    if (!composerEl || !actionsEl || !available) return;
    let measured = '';
    // The chat column supplies space; commands supply the text inset. Observing
    // the textarea itself would observe our own height writes and form a loop.
    const observer = new ResizeObserver(() => {
      const style = getComputedStyle(composerEl);
      const next = `${composerEl.clientWidth}:${style.maxHeight}:${style.minHeight}`;
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
    onkeydown={onComposerKey}
    onpaste={onComposerPaste}
    onfocus={onfocus}
  ></textarea>
  <div class="composer-actions" bind:this={actionsEl}>
    <span class="all-choice">
      <CommandButton variant="icon" icon="collab" label={t('hubEveryone')} pressed={recipient === ALL_TARGET}
        hasPopup={recipient === ALL_TARGET ? 'menu' : undefined}
        expanded={recipient === ALL_TARGET ? allMenuOpen : undefined}
        disabled={!selected || !roomReady} onclick={onall} />
    </span>
    <CommandButton variant="icon" icon="plus" label={t('hubAttach')} disabled={!selected || attaching}
      pending={attaching} onclick={() => fileEl?.click()} />
    <CommandButton variant="primary" iconOnly icon="send-up" label={t('hubSend')}
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
    padding: 0 12px 10px;
  }
  .compose-shell {
    position: relative; border: 1px solid var(--border); border-radius: 16px;
    background: var(--bubble-in); padding: var(--tool-inset-block) var(--tool-inset-inline);
  }
  .compose-shell:focus-within { border-color: var(--accent-line); }
  .compose-shell.cmd { border-color: color-mix(in srgb, var(--accent) 45%, transparent); background: color-mix(in srgb, var(--accent) 6%, var(--bubble-in)); }
  .compose-shell.cmd .c-input { font-family: var(--font-mono); }
  .compose-line { display: flex; align-items: flex-end; gap: var(--tool-gap); min-width: 0; }
  .c-input {
    display: block; flex: 1; min-width: 0; width: 100%; box-sizing: border-box; min-height: var(--control-height);
    max-height: calc(30vh / var(--ui-zoom, 1)); padding: max(2px, calc((var(--control-height) - 1.5em) / 2)) 0;
    border: 0; outline: none; background: transparent; color: var(--text);
    font: var(--fs-body)/1.5 var(--font-ui); resize: none; overflow-y: hidden;
  }
  .c-input::placeholder { color: var(--text3); }
  .composer-actions { display: flex; align-items: center; flex: none; gap: var(--tool-gap); }
  .all-choice { display: flex; flex: none; }
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
    flex: none; padding: 0; border: 0; border-radius: 50%; background: transparent; color: inherit;
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
  :global(.hub-root.compact) .composer { padding: 0 9px 8px; }
  :global(.hub-root.compact) .c-input { max-height: calc(28vh / var(--ui-zoom, 1)); }
</style>
