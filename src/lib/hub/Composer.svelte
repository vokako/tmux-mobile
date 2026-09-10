<script>
  import { tick as settled } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import { t } from '../core/i18n.svelte.ts';
  import { hoverInfo } from '../ui/hover.ts';
  import { chipExtras, slashCommand, commandPalette, readlineEdit, pastedFiles, textIsThePaste, stateDotColor, stateIsLive } from './hub.ts';
  import { ALL_TARGET, paletteBackendFor } from './hub-composer.ts';

  let {
    selected = '', compact = false, recipient = '', composerText = $bindable(''),
    agents = [], managedAgents = [], managedNames = [],
    pending = [], attaching = false, failed = false, sendable = false,
    onselect: setRecipient = () => {}, onsend: send = () => {},
    onstage: stageFiles = async () => {}, onremove: removeAttachment = () => {},
    onmodels: modelsList = async () => ({ models: [] }), oninterrupt = async () => {},
    onpreview = () => {}, onfocus = () => {}, onheightchange = () => {},
    registerBack = null,
  } = $props();

  // Hub owns transport and the capture-listener ordering. Only these view
  // operations cross back; no DOM references or copied popup flags escape.
  export function caret() { return composerEl?.selectionStart; }
  export function focus() { composerEl?.focus(); }
  export function closeRecipient() { recipientOpen = false; }
  export function recipientChanged() {
    recipientOpen = false;
    // An armed interrupt belongs to the recipient selected when it was armed.
    intArm = false;
  }
  export function hasTransient() { return recipientOpen || !!palette; }
  export function dismissOutside(e) {
    const t = e.target;
    if (recipientOpen && !t?.closest?.('.to-wrap')) recipientOpen = false;
    // A tap outside parks the palette until the next text change.
    if (palette && !t?.closest?.('.cmd-menu, .compose-shell')) paletteOff = true;
  }
  export function dismissEscape(e) {
    if (recipientOpen) { recipientOpen = false; e.stopPropagation(); }
  }

  let recipientOpen = $state(false);

  // Empty-composer interrupt (owner, 2026-08-24): with nothing typed, the grey
  // send button (or Ctrl+C) ARMS — it becomes a "send interrupt" button — and
  // a second activation fires. Two beats on purpose: interrupt cancels a
  // running turn, and a single stray tap on the button every message is sent
  // from should not be able to do that.
  let intArm = $state(false);
  let intTimer = 0;

  let composerEl = $state(null);

  let toChipW = $state(0);

  // Who the BODY reaches past the chip's recipient (`@bob` mid-sentence is
  // delivered too — deliver_mentions scans the whole body). Shown on the chip
  // as `+@bob` so the chip never understates where a line is going.
  const toExtras = $derived(chipExtras(composerText, recipient, managedNames));

  /** Grow to fit what is being typed, up to the CSS ceiling, then let it scroll.
   * Height has to be measured, not guessed: wrapping depends on the font, the
   * width and the text. Reset to `auto` first or the box can only ever grow. */
  /* The send button does NOT reserve a column (owner: text may run directly
     above it). The textarea is full width; a hidden mirror re-lays-out the
     value to find the LAST line's right edge, and only when that edge would
     collide with the button zone does the box gain one line of bottom
     padding — the same "share the last line, else drop below" semantics as
     the bubble's meta trailer. A textarea cannot flow around a float, so
     the mirror is the only honest way to know where the last line ends. */
  let mirrorEl = null;
  const SEND_ZONE = 74; // attach + send (30px each + gaps), from the textarea's right edge
  function lastLineCollides(el) {
    if (!el.value) return false;
    if (!mirrorEl) {
      mirrorEl = document.createElement('div');
      mirrorEl.className = 'c-mirror';
      el.parentElement?.appendChild(mirrorEl);
    }
    mirrorEl.style.width = `${el.clientWidth}px`;
    mirrorEl.style.textIndent = el.style.textIndent || '0';
    mirrorEl.textContent = el.value;
    const marker = document.createElement('span');
    marker.textContent = '\u200b';
    mirrorEl.appendChild(marker);
    return marker.offsetLeft > el.clientWidth - SEND_ZONE;
  }
  function growComposer() {
    const el = composerEl;
    if (!el) return;
    // Measure the natural height first, with no avoidance applied.
    el.style.paddingBottom = '';
    el.style.paddingRight = '';
    el.style.height = 'auto';
    const maxH = parseFloat(getComputedStyle(el).maxHeight) || Infinity;
    const overflowing = el.scrollHeight > maxH + 1;
    // The scrollbar exists exactly while there is something to scroll
    // (board #34): the base state is hidden (an empty composer must never
    // show a track), auto ONLY when the natural height exceeds max-height —
    // decided here, in the SAME measurement, so shrinking or the post-send
    // reset flips it back to hidden on the very next run. Never hide it
    // permanently and never scrollbar-width:none — a long message must
    // really scroll.
    el.style.overflowY = overflowing ? 'auto' : 'hidden';
    if (overflowing) {
      // Scrolled state: the box is at max height and the button permanently
      // overlays its bottom-right corner — EVERY line scrolls past it, so
      // all lines shorten clear of the button zone while scrolling lasts.
      el.style.paddingRight = '76px'; // clear BOTH corner buttons
    } else if (lastLineCollides(el)) {
      // Tail collision: clear the button's full height (top edge sits
      // ~30px above the textarea's bottom; 34px keeps descenders clear),
      // not just one line box — a 24px pad still left the button's top
      // strip over the glyphs (owner report).
      el.style.paddingBottom = '34px';
    }
    el.style.height = 'auto';
    el.style.height = `${el.scrollHeight}px`;
    // The composer taking space is the feed losing it — the same way the keyboard
    // does — so keep the tail parked while it grows. ONLY when it actually
    // grew: this runs per KEYSTROKE, and re-parking an unchanged tail
    // re-assigns scrollTop (fractional clamping under --ui-zoom lands on a
    // different pixel each time — the measured 2261 → 2221↔2298 oscillation)
    // and re-seeds the ask anchor, so the whole feed shivered as you typed
    // (owner, 2026-08-26: "每打一个字符…内容就会上下闪烁"). The SHELL is what
    // takes space (chips rows count, not just the textarea).
    const shellH = el.parentElement?.offsetHeight ?? el.offsetHeight;
    if (shellH !== lastShellH) {
      lastShellH = shellH;
      onheightchange();
    }
  }
  let lastShellH = 0;
  $effect(() => {
    void composerText;   // includes the reset to '' after sending
    void toChipW;        // the indent changes wrapping, so height must re-measure
    void composerIsCmd;  // the mono flip changes metrics, so height re-measures too
    growComposer();
  });


  let fileEl = $state(null);

  async function onPickFiles(e) {
    const files = [...(e.target.files || [])];
    e.target.value = ''; // same file re-pickable
    await stageFiles(files);
  }

  /** Paste is the second door into the SAME staging pipeline (board #25:
   * "chat 输入框，应该支持剪贴板粘贴图片或者文件等"): a screenshot or a copied
   * file lands exactly like the + button's pick — image re-encoded, file
   * uploaded byte-identical, chip above the box, [img:n]/[file:n] token at
   * the caret. Files WIN over text riding the same clipboard (a copied file
   * also carries its path as text — inserting it beside the staged chip
   * would say the same thing twice), so the default insertion is suppressed
   * exactly when there are files to stage; a plain text paste is untouched.
   * ONE exception, decided by `textIsThePaste`: words riding beside an
   * image-only file set are the paste and the image is a rendering of them
   * (PowerPoint/Word/Excel/Keynote/browsers all ship that PNG), so the text
   * is inserted and the picture dropped (owner, 2026-09-08). */
  function onComposerPaste(e) {
    const files = pastedFiles(e.clipboardData);
    if (!files.length) return; // text-only paste: the textarea's own business
    if (textIsThePaste(e.clipboardData?.getData('text/plain'), files)) return; // a picture OF the text: take the words
    e.preventDefault();
    stageFiles(files);
  }


  // Is what is typed going to be RUN rather than SAID? Mirrors send()'s own
  // branch exactly — slashCommand() recognises the shape, and a target must
  // exist (explicit @name, else the recipient; a room note has nobody to run
  // it and stays a message) — so the composer's look never promises a command
  // that send() would deliver as prose (owner, 2026-08-20: "如果是指令的话在输
  // 入框里样式改变一下").
  const composerIsCmd = $derived.by(() => {
    const c = slashCommand(composerText.trim());
    return !!(c && (c.to || recipient));
  });

  /** Enter sends where there is a keyboard with modifiers, and inserts a newline
   * on a touch device — where the return key is the ONLY way to get one and the
   * send button is right there. Shift+Enter is always a newline. */
  // ── Slash-command completion. Typing `/` offers the agent CLI's commands;
  // choosing one that takes an argument offers ITS values next (the model ids
  // come from the server, which asks the CLI). Two stages, one palette.
  let cmdModels = $state({});          // backend → model ids (each fetched once)
  let paletteIdx = $state(0);
  let paletteOff = $state(false);      // Escape closes it until the text changes
  // The palette speaks the ADDRESSEE's dialect: each CLI has its own command
  // table (kiro's /tangent does not exist in codex; codex's /model is a
  // picker), so the table follows the explicit @name, else the composer's
  // recipient. @all with a mixed roster gets no palette — one command line
  // cannot be right in two dialects at once (owner, 2026-08-22 对齐).
  const paletteBackend = $derived(paletteBackendFor(composerText, recipient, agents));
  const palette = $derived(paletteOff ? null : commandPalette(composerText, cmdModels[paletteBackend] ?? [], paletteBackend));
  $effect(() => { void composerText; paletteOff = false; });
  $effect(() => { void palette; paletteIdx = 0; });
  // The model list is only needed once a command wants it, and the server caches
  // it for ten minutes — so this asks at most once per backend per Hub visit.
  // kiro and grok can enumerate their models; claude and codex return null.
  $effect(() => {
    const backend = paletteBackend;
    if (!palette || cmdModels[backend]) return;
    modelsList(backend || 'kiro').then((r) => { cmdModels = { ...cmdModels, [backend]: r.models ?? [] }; }).catch(() => {});
  });

  /** Put the chosen completion in the box. `more` keeps the palette alive for the
   * argument, which is what makes a two-part command one flow. */
  function acceptCompletion(item) {
    if (!palette) return;
    const head = composerText.slice(0, palette.from);
    composerText = `${head}${item.value}${palette.more ? ' ' : ''}`;
    paletteIdx = 0;
    composerEl?.focus();
    // Put the caret at the end; assigning `value` in Svelte leaves it wherever
    // it was, which on a re-render means before the text we just inserted.
    requestAnimationFrame(() => composerEl?.setSelectionRange(composerText.length, composerText.length));
  }

  // The composer's readline set (Ctrl-A/E/U/K/W/Y/D/H/T/F/B): the kill buffer
  // and the accumulation chain live here; the arithmetic is readlineEdit()
  // (pure, in hub.ts). Ours everywhere — macOS keeps the half it had natively,
  // every other platform gains the whole set, and none of them drift apart.
  let killBuf = '';
  let killChain = false;

  function onComposerKey(e) {
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
        // The caret AFTER Svelte writes the value back — setting it before
        // would let the value update reset it to the text's end.
        settled().then(() => composerEl?.setSelectionRange(r.caret, r.caret));
        return;
      }
    }
    // Any other keystroke breaks the kill chain: Ctrl-K Ctrl-K accumulates,
    // Ctrl-K <type> Ctrl-K replaces — readline's own rule.
    killChain = false;
    // Ctrl+C twice on an EMPTY composer sends an interrupt (owner, 2026-08-24)
    // — the terminal's own cancel gesture, aimed at the addressee. Only while
    // empty: with text present (or a selection) Ctrl+C stays the browser's
    // copy, and readlineEdit deliberately lets it fall through.
    if (e.ctrlKey && !e.metaKey && !e.altKey && !e.isComposing
        && e.key.toLowerCase() === 'c' && !composerText.trim()) {
      e.preventDefault();
      armInterrupt();
      return;
    }
    if (e.key === 'Escape' && intArm) { e.preventDefault(); intArm = false; return; }
    // The palette owns the arrows, Tab and Enter while it is open — it is a
    // menu, and a menu that ignores the keyboard is a menu you have to reach for
    // the mouse to use.
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
    if (compact) return;      // let the newline through; tap send
    e.preventDefault();
    send();
  }



  // The composer's interrupt reaches whoever the composer reaches: the named
  // recipient, or every managed agent for @all. An unaddressed room note
  // interrupts NOBODY — same shape as message delivery, no third rule.
  const intTargets = $derived(
    recipient === ALL_TARGET ? managedAgents.map((a) => a.name)
    : recipient ? [recipient] : []);
  const intWho = $derived(recipient === ALL_TARGET ? '@all' : recipient ? `@${recipient}` : '');

  // The recipient's turn is OPEN (running a turn, or holding an ask) — the
  // send button wears a slow spinner around a stop square, the reader's cue
  // that "this one is mid-turn, tapping here is how you cut it" (owner,
  // 2026-08-25: the lightning bolt "看着好像不是那么容易理解"). Not for
  // idle/failed: an ended turn has nothing to interrupt.
  const BUSY_STATES = ['running', 'working', 'waiting', 'blocked'];
  const recipientBusy = $derived(
    recipient === ALL_TARGET
      ? managedAgents.some((a) => BUSY_STATES.includes(a.state))
      : managedAgents.some((a) => a.name === recipient && BUSY_STATES.includes(a.state)));

  /** First activation arms; the second (button or Ctrl+C, mixable) fires. */
  function armInterrupt() {
    if (!selected || !intTargets.length) return;
    if (intArm) { void fireInterrupt(); return; }
    intArm = true;
    clearTimeout(intTimer);
    // An armed cancel button should not lie in wait: unfired, it stands down.
    intTimer = setTimeout(() => { intArm = false; }, 3000);
  }
  async function fireInterrupt() {
    clearTimeout(intTimer);
    intArm = false;
    await oninterrupt(intTargets);
  }
  // Typing disarms — the button means "send" again the moment there is text —
  // and so does switching projects: an armed cancel must not follow the user
  // into another room.
  $effect(() => { if (composerText.trim()) intArm = false; });
  $effect(() => { void selected; intArm = false; });


  /** The composer's two upward popovers wear the same `.pop-layer` gate as the
   * fixed menus (motion.md principle 8): measured invisible, then grown from
   * the corner touching the chip (`bottom left`). The heights reset when a
   * menu closes so the NEXT opening is measured — and animated — again. */
  let toMenuH = $state(0);
  let cmdMenuH = $state(0);
  $effect(() => { if (!recipientOpen) toMenuH = 0; });
  $effect(() => { if (!palette?.items.length) cmdMenuH = 0; });


  /** The chip's meaning under the three-destination model (hub-composer.md):
   * one pane, every managed pane, or the record alone. */
  function toChipInfo() {
    const text = recipient === ALL_TARGET ? t('hubToAllLong') : recipient ? t('hubToDmLong').replace('{name}', recipient) : t('hubToRoomLong');
    const note = toExtras.length ? t('hubToAlsoHint').replace('{names}', toExtras.map((n) => '@' + n).join(', ')) : undefined;
    return { text, note };
  }

  $effect(() => {
    if (!registerBack) return;
    const disposers = [
      registerBack('recipient', () => { if (recipientOpen) { recipientOpen = false; return true; } return false; }),
      registerBack('palette', () => { if (palette) { paletteOff = true; return true; } return false; }),
      registerBack('interrupt', () => { if (intArm) { intArm = false; return true; } return false; }),
    ];
    return () => { for (const dispose of disposers) dispose(); };
  });
</script>

<div class="composer">
  <div class="compose-shell" class:cmd={composerIsCmd}>
  <!-- WHO this goes to, always visible: the lead by default, one tap to
       retarget or to broadcast. No @ typing required. Pinned to the shell's
       top-left; the textarea's FIRST line starts beside it (measured
       text-indent) and wrapped lines run full width beneath it. -->
  {#if managedAgents.length}
    <div class="to-wrap" bind:clientWidth={toChipW}>
      <!-- What the current destination MEANS is the hover card's job
           (toChipInfo — the three-destination model in one sentence,
           the body's +@ extras as its note); no native title beside it. -->
      <button class="to-chip" class:all={recipient === ALL_TARGET} class:note={!recipient}
        use:hoverInfo={toChipInfo}
        onclick={() => recipientOpen = !recipientOpen}>
        <span class="to-label">{t('hubTo')}</span>
        <span class="to-name">{recipient === ALL_TARGET ? t('hubEveryone') : recipient || t('hubRoomNote')}</span>
        <!-- The body's own @mentions, live: `to: alice +@bob`. The chip
             is the one place that says where a line goes, and it used to
             understate it (review, 2026-09-03). Accent even on the grey
             note chip — this part IS a delivery. -->
        {#if toExtras.length}<span class="to-extra appear">{toExtras.map((n) => '+@' + n).join(' ')}</span>{/if}
        <!-- ONE arrow that TURNS (motion.md principle 4): it points up while
             the menu is closed (the menu opens upward) and rotates to down
             once it is open — never two glyphs swapped. -->
        <span class="flip" class:on={recipientOpen}><Icon name="chevron-up" size={11} /></span>
      </button>
      {#if recipientOpen}
        <div class="to-menu pop-layer" class:ready={toMenuH > 0} style:--pop-origin="bottom left" bind:clientHeight={toMenuH}>
          {#each managedAgents as a (a.window)}
            <button class:sel={recipient === a.name} onclick={() => setRecipient(a.name)}>
              <span class="st" class:live-dot={stateIsLive(a.state)} style:background={stateDotColor(a.state)}></span>{a.name}
            </button>
          {/each}
          <div class="to-sep"></div>
          <!-- Broadcast and room note are destinations, not lessons. -->
          <button class:sel={recipient === ALL_TARGET} onclick={() => setRecipient(ALL_TARGET)}>
            <span class="st all-dot"></span>
            <span>{t('hubEveryone')}</span>
          </button>
          <button class:sel={!recipient} onclick={() => setRecipient('')}>
            <span class="st note-dot"></span>
            <span>{t('hubRoomNote')}</span>
          </button>
        </div>
      {/if}
    </div>
  {/if}
  {#if palette?.items.length}
    <!-- Completion for a `/command`, opening UPWARD so the on-screen
         keyboard never covers it — the same rule as the recipient menu, and
         the same popover dialect. -->
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
  <!-- A textarea, not an input: a message you are still writing has to be
       readable. It grows with the text and then scrolls, so a long one is
       never a one-line peephole. -->
  <textarea class="c-input" rows="1" bind:this={composerEl} bind:value={composerText}
    style:text-indent={managedAgents.length ? `${toChipW + 8}px` : '0'}
    placeholder={recipient === ALL_TARGET ? t('hubComposerAll') : recipient ? t('hubComposerDm').replace('{name}', recipient) : t('hubComposerRoom')}
    onkeydown={onComposerKey}
    onpaste={onComposerPaste}
    onfocus={onfocus}
  ></textarea>
  {#if pending.length}
    <div class="pend-row">
      {#each pending as a, i (a.key)}
        {#if a.error}
          <!-- A failed attachment, in the file chip's own clothes turned
               to the danger tone: name + reason, ✕ removes it. It blocks
               send while it stands — nothing leaves without what it
               showed (review, 2026-09-03). -->
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
            <!-- Tap the thumb → the in-app viewer (owner, 2026-08-27:
                 "文本输入框…显示的图片…我可以点击放大查看…在我应用内的"). -->
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
  <!-- Send lives INSIDE the capsule, bottom-right, out of the flow: it
       stopped costing the composer a whole column. Empty, it is still
       CLICKABLE (grey, muted): the first tap arms it as a "send
       interrupt" button — amber, named — and the second fires. -->
  {#if intArm}
    <div class="int-pill appear-rise" role="status">{t('hubIntArmed').replace('{who}', intWho)}</div>
  {/if}
  <input type="file" multiple hidden bind:this={fileEl} onchange={onPickFiles} />
  <button class="attach-btn" class:busy={attaching} title={t('hubAttach')} aria-label={t('hubAttach')}
    disabled={!selected || attaching} onclick={() => fileEl?.click()}>
    <svg class="plus-ring" viewBox="0 0 20 20" aria-hidden="true">
      <g fill="none" stroke="currentColor" stroke-width="1.25" stroke-linecap="round">
        <circle cx="10" cy="10" r="9.2" />
        <line x1="10" y1="6.4" x2="10" y2="13.6" />
        <line x1="6.4" y1="10" x2="13.6" y2="10" />
      </g>
    </svg>
  </button>
  <button class="send-btn" class:muted={!sendable && !intArm && !recipientBusy} class:arm={intArm}
    class:busy={recipientBusy && !sendable && !intArm}
    onclick={() => (sendable ? send() : armInterrupt())}
    title={failed ? t('hubAttachBlocked') : intArm ? t('hubIntArmed').replace('{who}', intWho) : sendable ? t('hubSend') : t('hubIntHint')}
    aria-label={failed ? t('hubAttachBlocked') : intArm ? t('hubIntArmed').replace('{who}', intWho) : sendable ? t('hubSend') : t('hubIntHint')}
    disabled={!selected || attaching || failed || (!sendable && !intTargets.length)}>
    {#if !sendable && (intArm || recipientBusy)}
      <!-- A stop square inside a slowly circling arc: the "mid-turn, tap
           to cut it" glyph every chat product speaks. Armed keeps the
           same glyph on the amber ground — same object, hotter state —
           instead of the lightning bolt nobody read as "interrupt". -->
      <svg class="stop-spin appear-pop" viewBox="0 0 20 20" width="16" height="16" aria-hidden="true">
        <circle class="ss-ring" cx="10" cy="10" r="8" fill="none" stroke="currentColor"
          stroke-width="1.6" stroke-linecap="round" stroke-dasharray="37.7 12.6" />
        <rect x="6.6" y="6.6" width="6.8" height="6.8" rx="1.7" fill="currentColor" />
      </svg>
    {:else}
      <!-- Arrow ↔ stop-square are unrelated glyphs, so they SWAP (motion.md
           principle 4) — the {#if} remounts and the newcomer pops in. -->
      <span class="send-glyph appear-pop"><Icon name="send-up" size={15} /></span>
    {/if}
  </button>
  </div>
</div>

<style>
  /* No env(safe-area-inset-bottom) here: the composer does not sit at the
     screen's bottom — the TAB BAR below it does, and it already pads for the
     gesture bar (`--sab`). Adding the inset here too stacked it twice and
     opened a wide blank band between the composer and the tabs on Android
     (owner, 2026-08-21: "消息框和下边的选项标签中间的空白有点大"). With the
     keyboard open the tabbar hides, but the keyboard covers the inset then —
     8px is the right gap in both states. */
  :global(.hub-root.compact) .composer { padding: 8px 9px; }
  :global(.hub-root.compact) .compose-shell { padding: 6px 9px; border-radius: 15px; }
  :global(.hub-root.compact) .to-chip { max-width: 110px; height: 28px; }
  :global(.hub-root.compact) .to-label { display: none; }
  :global(.hub-root.compact) .c-input { min-height: 30px; font-size: var(--fs-body); max-height: calc(40vh / var(--ui-zoom, 1)); }
  :global(.hub-root.compact) .send-btn { width: 32px; height: 32px; right: 6px; bottom: 4.5px; border-radius: var(--ui-radius-control); }
  :global(.hub-root.compact) .attach-btn { right: 44px; bottom: 12.5px; }

  :global(.hub-root.compact) .to-menu button { min-height: 44px; }
  .composer {
    display: flex; align-items: flex-end; gap: 9px; padding: 10px clamp(12px, 3vw, 28px);
    border-top: 1px solid var(--border2); background: color-mix(in srgb, var(--bg) 92%, transparent);
    box-shadow: 0 -8px 28px rgba(0,0,0,0.05);
    -webkit-backdrop-filter: blur(14px); backdrop-filter: blur(14px);
    /* ONE stacking context for the whole composer, above every layer the FEED
       makes (pinned .ask-top/.ask-bottom at 6, the action overlay at 8): its
       popovers — the recipient menu and the / palette — open UPWARD over the
       feed, and without this the .to-wrap wrapper's own level (2) CAPPED the
       menu below a pinned bubble (board #1, owner: "应该在钉住的消息图层上方…
       新弹出的选项都应该优先级高于已有的组件"). Inner levels keep ordering
       INSIDE the composer; this decides composer-vs-feed once. */
    position: relative; z-index: 15;
  }

  .compose-shell {
    flex: 1; min-width: 0; position: relative;
    padding: 6px 10px; border: 1px solid var(--input-border); border-radius: 16px;
    background: var(--bubble-in); box-shadow: 0 1px 3px rgba(0,0,0,0.10);
    transition: border-color var(--t-fast) ease, box-shadow var(--t-fast) ease;
  }
  .compose-shell:focus-within { border-color: color-mix(in srgb, var(--accent) 55%, transparent); box-shadow: 0 2px 8px rgba(0,0,0,0.12); }
  /* A line that will be RUN, not said: machine text wears the machine face —
     the same monospace the tool lane uses — plus an accent tint on the capsule,
     so "this goes to the CLI" is visible before send decides anything. The
     mirror MUST flip with it: it re-lays-out the text to find the last line,
     and measuring mono text with a proportional font misplaces the send
     button's collision zone. */
  .compose-shell.cmd { border-color: color-mix(in srgb, var(--accent) 45%, transparent); background: color-mix(in srgb, var(--accent) 6%, var(--bubble-in)); }
  .compose-shell.cmd .c-input, .compose-shell.cmd :global(.c-mirror) { font-family: var(--font-mono); }
  /* Recipient control: who this message goes to, with a menu that opens
     UPWARD so the on-screen keyboard never covers it. */
  /* Pinned to the capsule's top-left; the textarea's first line is indented
     past it and later lines reclaim the full width beneath. */
  .to-wrap { position: absolute; top: 7px; left: 8px; z-index: 2; width: max-content; }
  .to-chip {
    display: flex; align-items: center; gap: 4px; height: 26px;
    background: var(--accent-bg); color: var(--accent); border: 1px solid transparent;
    border-radius: var(--ui-radius-control); padding: 0 9px; font-size: var(--fs-sub); font-weight: 650;
    cursor: pointer; max-width: min(34vw, 220px);
    transition: background var(--t-fast), color var(--t-fast), border-color var(--t-fast);
  }
  /* Broadcast and room-note are NOT the default state, so they do not wear the
     accent: one interrupts everyone, the other reaches nobody live. */
  .to-chip.all { background: var(--surface); color: var(--status-warn); border-color: var(--status-warn); }
  .to-chip.note { background: var(--surface); color: var(--text2); border-color: var(--border); }
  .to-label { font-weight: 500; opacity: 0.7; font-size: var(--fs-meta); text-transform: uppercase; letter-spacing: 0.5px; }
  .to-name { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .to-extra { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; color: var(--accent); font-weight: 600; font-size: var(--fs-meta); font-family: var(--font-mono); }
  .to-sep { height: 1px; background: var(--border2); margin: 4px 6px; }
  .to-menu {
    position: absolute; bottom: calc(100% + 6px); left: 0; z-index: 12;
    min-width: 168px; max-height: calc(46vh / var(--ui-zoom, 1)); overflow-y: auto;
    background: var(--bg); border: 1px solid var(--border); border-radius: var(--ui-radius-panel);
    box-shadow: 0 12px 34px rgba(0,0,0,0.45); padding: 5px; display: flex; flex-direction: column; gap: 2px;
  }
  .to-menu button {
    display: flex; align-items: center; gap: 7px; min-height: 36px; width: 100%; text-align: left;
    background: none; border: none; border-radius: var(--ui-radius-control); color: var(--text2);
    padding: 6px 10px; font-size: var(--ui-font-control); cursor: pointer; font-family: var(--font-mono);
  }
  .to-menu button:hover { background: var(--surface2); color: var(--text); }
  /* The slash-command palette: the recipient menu's surface, full capsule width
     because a command list is read as rows of name + description. */
  .cmd-menu {
    position: absolute; bottom: calc(100% + 6px); left: 0; right: 0; z-index: 14;
    max-height: calc(44vh / var(--ui-zoom, 1)); overflow-y: auto; scrollbar-width: thin;
    background: var(--bg); border: 1px solid var(--border); border-radius: var(--ui-radius-panel);
    box-shadow: 0 12px 34px rgba(0,0,0,0.45); padding: 5px;
    display: flex; flex-direction: column; gap: 2px;
  }
  .cmd-opt {
    display: flex; align-items: baseline; gap: 10px; width: 100%; text-align: left;
    background: none; border: none; border-radius: var(--ui-radius-control); color: var(--text2);
    padding: 6px 10px; font-size: var(--ui-font-control); cursor: pointer;
    font-family: var(--font-mono);
  }
  /* Hover and the keyboard cursor are the SAME highlight — two would read as two
     selections. */
  .cmd-opt:hover, .cmd-opt.cur { background: var(--surface2); color: var(--text); }
  .cmd-name { flex: none; font-weight: 650; color: var(--accent); }
  .cmd-hint {
    min-width: 0; color: var(--text3); font-family: inherit; font-size: var(--fs-meta);
    overflow: hidden; text-overflow: ellipsis; white-space: nowrap;
  }
  :global(.hub-root.compact) .cmd-opt { min-height: 44px; align-items: center; }
  .to-menu button.sel { color: var(--accent); background: var(--accent-bg); }
  .all-dot { border: 1px solid var(--text3); background: none; }
  .c-input {
    display: block; width: 100%; min-height: 28px; max-height: calc(34vh / var(--ui-zoom, 1));
    padding: 5px 0 4px; background: transparent; border: none; outline: none;
    color: var(--text); font-size: var(--fs-body); line-height: 1.5;
    resize: none; overflow-y: hidden; /* growComposer flips to auto only while overflowing (board #34) */
  }
  /* growComposer's layout mirror: same metrics as .c-input, invisible.
     Created by JS, so it has no scope class — hence :global under the shell. */
  .compose-shell :global(.c-mirror) {
    position: absolute; left: 10px; top: 0; visibility: hidden; pointer-events: none;
    font-size: var(--fs-body); line-height: 1.5; white-space: pre-wrap;
    overflow-wrap: break-word; padding: 0; border: 0;
  }
  .c-input::placeholder { color: var(--text3); opacity: 0.82; }
  /* The send action: a bold up-arrow (the iMessage/ChatGPT shape — symmetric,
     so it optically centres where a diagonal plane always sat crooked) on a
     flat accent square that matches the capsule's radius. Light theme: full
     accent (a deep blue) + near-white ink. Dark theme: the accent is ELECTRIC
     CYAN (#00d4ff) — at full strength it read as a glowing block on the dark
     canvas (owner report), so the fill is toned to a 60% mix with the
     background and the ink flips to near-white, which also matches the
     recipient chip's quiet accent language. Disabled recedes into the
     surface instead of ghosting the accent. */
  /* Sized so the empty capsule centres it exactly (measured shell 43px, so
     the absolute `bottom` is measured from the PADDING box, 1px inside the
     border, so 5.5px yields symmetric 6.5px gaps). Bottom-anchored, so it
     stays put as the box grows into multiple lines. */
  .pend-row { display: flex; flex-wrap: wrap; gap: 5px; padding: 6px 88px 4px 4px; }
  .pend-chip {
    display: inline-flex; align-items: center; gap: 5px;
    height: 24px; padding: 0 4px 0 8px; max-width: 220px;
    border: 1px solid var(--border2); border-radius: var(--ui-radius-control);
    background: var(--surface2); color: var(--text2); font-size: var(--fs-micro);
  }
  .pend-name { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  /* The failed chip: same box, danger tone (token + 8% wash), room for the
     reason. The name keeps its ellipsis; the reason ellipsizes after it. */
  .pend-chip.err {
    max-width: 340px; color: var(--status-danger); border-color: var(--status-danger);
    background: color-mix(in srgb, var(--status-danger) 8%, var(--surface2));
  }
  .pend-chip.err .pend-name { flex: none; max-width: 120px; font-weight: 600; }
  .pend-why { overflow: hidden; text-overflow: ellipsis; white-space: nowrap; opacity: 0.85; }
  .pend-chip.err .pend-x { color: var(--status-danger); }
  .pend-view { display: block; width: 100%; height: 100%; padding: 0; margin: 0; border: 0; background: none; cursor: zoom-in; }
  .pend-thumb {
    position: relative; width: 44px; height: 44px; flex: none;
    border: 1px solid var(--border2); border-radius: var(--ui-radius-control);
    overflow: hidden; background: var(--surface2);
    cursor: zoom-in;
  }
  .pend-thumb img { width: 100%; height: 100%; object-fit: cover; display: block; }
  .pend-n {
    position: absolute; left: 0; bottom: 0; padding: 0 4px;
    font-size: var(--fs-micro); line-height: 14px; font-family: var(--font-mono);
    color: #fff; background: rgba(0,0,0,0.55); border-top-right-radius: 5px;
  }
  .pend-x.on-img {
    position: absolute; top: 0; right: 0; width: 15px; height: 15px;
    color: #fff; background: rgba(0,0,0,0.55); border-bottom-left-radius: 6px; border-radius: 0 0 0 6px;
  }
  .pend-x.on-img::after { inset: -6px; }
  .pend-x.on-img:hover { color: #fff; background: var(--status-danger); }
  .pend-x {
    display: grid; place-items: center; width: 16px; height: 16px; padding: 0;
    border: none; border-radius: 5px; background: none; color: var(--text3);
    cursor: pointer; position: relative;
  }
  .pend-x::after { content: ''; position: absolute; inset: -8px; }
  .pend-x:hover { color: var(--status-danger); }
  .attach-btn {
    position: absolute; right: 42px; bottom: 12.5px;
    width: 16px; height: 16px; display: grid; place-items: center;
    padding: 0; border: none; border-radius: 50%;
    background: transparent; color: color-mix(in srgb, var(--text3) 78%, transparent); cursor: pointer;
    transition: color var(--t-fast) ease;
  }
  .attach-btn .plus-ring { position: absolute; inset: 0; }
  .attach-btn::after { content: ''; position: absolute; inset: -12px; }
  /* The glyph is ONE drawing: hover lifts circle and plus together. */
  .attach-btn:hover:not(:disabled) { color: var(--accent); }
  .attach-btn:disabled { opacity: 0.55; cursor: default; }
  .attach-btn.busy { animation: attach-pulse 1s ease-in-out infinite; }
  @keyframes attach-pulse { 50% { opacity: 0.4; } }
  @media (prefers-reduced-motion: reduce) { .attach-btn.busy { animation: none; } }
  .send-btn {
    position: absolute; right: 7px; bottom: 5.5px;
    width: 30px; height: 30px; display: grid; place-items: center;
    padding: 0; border: none; border-radius: var(--ui-radius-control); cursor: pointer;
    background: var(--accent-fill);
    color: var(--accent-fill-ink);
    transition: filter var(--t-fast) ease, background var(--t-fast) ease, color var(--t-fast) ease, transform var(--t-fast) ease;
  }
  .send-btn:hover:not(:disabled) { filter: brightness(1.07); }
  .send-btn:active:not(:disabled) { transform: scale(0.93); }
  .send-btn:disabled { background: var(--surface2); color: var(--text3); cursor: default; }
  /* Empty composer: clickable but wearing the resting grey — the tap is an
     ARM, not a send, and the button must not advertise accent urgency. */
  .send-btn.muted { background: var(--surface2); color: var(--text3); }
  /* The recipient is mid-turn: same resting ground, but the glyph is the
     spinner-around-a-stop-square in accent — alive, not urgent. The button
     still ARMS first; this state only changes what it looks like at rest. */
  .send-btn.busy { background: var(--surface2); color: var(--accent); }
  /* Armed: the one attention colour — interrupt asks a person to confirm. */
  .send-btn.arm { background: var(--status-warn); color: var(--accent-fill-ink); }
  /* Deliberately unhurried (owner: "动画不用很快"): a fast spin says
     "loading", this says "a turn is open". The square stays put; only the
     arc travels. */
  .ss-ring { transform-origin: 50% 50%; animation: spin 2.2s linear infinite; }
  @media (prefers-reduced-motion: reduce) { .ss-ring { animation: none; } }
  /* What the armed button will do, in words, above it — a phone has no hover
     for the title. pointer-events off: it is a caption, not a control. */
  .int-pill {
    position: absolute; right: 6px; bottom: 44px;
    font-size: var(--fs-micro); color: var(--text2);
    background: var(--surface2); border-radius: 6px; padding: 3px 8px;
    white-space: nowrap; pointer-events: none;
  }
  /* Phone-first hit areas (contract: primary actions ≥44px): the visual box
     stays small, the tap target grows via an invisible overlay. (.to-tail
     brings its own ::before from app.css.) */
  .send-btn::after { content: ''; position: absolute; inset: -7px; }
  .send-glyph { display: inline-flex; }

</style>
