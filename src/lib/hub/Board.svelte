<script lang="ts">
  /* The project task board (owner, 2026-08-29: "引入一个新的看板功能…人类有一个
     看板页面，能写任务issue，agent也可以读任务，修改任务状态，在看板上记录信息
     状态"). This is the HUMAN's half; agents read and update the same issues
     through `tmm board`. Four fixed columns — the status vocabulary is shared
     with the CLI (`projects::BOARD_STATUSES`), so a free-text status here
     would fork the language. Lives in the Hub drawer as a partition, like the
     terminal and Files. */
  import { boardList, boardGet, boardSave, boardNote, boardDelete, type BoardIssue } from '../core/ws.ts';
  import { t } from '../core/i18n.svelte.ts';
  import Icon from '../ui/Icon.svelte';
  import Select from '../ui/Select.svelte';

  let { session, visible = true }: { session: string; visible?: boolean } = $props();

  const STATUSES = ['todo', 'doing', 'review', 'done'];

  let issues = $state<BoardIssue[]>([]);
  let ready = $state(false);
  let err = $state('');
  // view: the list, one issue (with its note thread), or the new-issue form.
  let sel = $state<BoardIssue | null>(null);
  let creating = $state(false);
  let nTitle = $state('');
  let nBody = $state('');
  let noteText = $state('');
  let busy = $state(false);

  async function load() {
    try {
      const r = await boardList(session);
      issues = r.issues;
      ready = true;
      err = '';
    } catch (e) {
      // A failed poll keeps the last board — "could not ask" ≠ "empty".
      err = String((e as Error)?.message ?? e);
    }
  }

  // Poll while visible: agents move cards from their panes, and the human
  // should see it without touching anything. Same verdict rule as the rooms —
  // nothing renders as "empty" before the first answer.
  $effect(() => {
    if (!visible || !session) return;
    load();
    const iv = setInterval(load, 8000);
    return () => clearInterval(iv);
  });
  // Switching projects resets the view to the new board's list.
  $effect(() => {
    void session;
    sel = null; creating = false; ready = false; issues = []; noteText = '';
  });

  async function openIssue(id: number) {
    try { sel = await boardGet(session, id); err = ''; } catch (e) { err = String((e as Error)?.message ?? e); }
  }
  async function refreshSel() {
    if (sel) await openIssue(sel.id);
    await load();
  }
  async function move(id: number, status: string) {
    busy = true;
    try { await boardSave(session, { id, status }); await refreshSel(); } catch (e) { err = String((e as Error)?.message ?? e); }
    busy = false;
  }
  async function createIssue() {
    if (!nTitle.trim() || busy) return;
    busy = true;
    try {
      await boardSave(session, { title: nTitle.trim(), body: nBody.trim() });
      nTitle = ''; nBody = ''; creating = false;
      await load();
    } catch (e) { err = String((e as Error)?.message ?? e); }
    busy = false;
  }
  async function addNote() {
    if (!sel || !noteText.trim() || busy) return;
    busy = true;
    try { await boardNote(session, sel.id, noteText.trim()); noteText = ''; await refreshSel(); } catch (e) { err = String((e as Error)?.message ?? e); }
    busy = false;
  }
  async function removeIssue() {
    if (!sel || busy) return;
    busy = true;
    try { await boardDelete(session, sel.id); sel = null; await load(); } catch (e) { err = String((e as Error)?.message ?? e); }
    busy = false;
  }

  // Esc peels the board's own layers (detail → list, form → list) before the
  // drawer's close sees it — same territory rule as the files partition.
  function onKey(e: KeyboardEvent) {
    if (e.key !== 'Escape') return;
    if (sel || creating) { sel = null; creating = false; e.stopPropagation(); }
  }

  const statusLabel = (s: string) => t(`boardStatus_${s}`);
  const col = (s: string) => issues.filter((i) => i.status === s);
  const noteCount = (i: BoardIssue) => (typeof i.notes === 'number' ? i.notes : i.notes.length);
  const ago = (ts: number) => {
    const d = Math.max(0, Date.now() / 1000 - ts);
    return d < 3600 ? `${Math.max(1, Math.round(d / 60))}m` : d < 86400 ? `${Math.round(d / 3600)}h` : `${Math.round(d / 86400)}d`;
  };
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="board" onkeydowncapture={onKey}>
  {#if !ready && !issues.length}
    <div class="empty">…</div>
  {:else if sel}
    <!-- ── one issue: the note thread is the issue's own record ── -->
    <div class="detail">
      <div class="d-head">
        <button class="icon-btn" title={t('back')} aria-label={t('back')} onclick={() => (sel = null)}>
          <Icon name="arrow-left" size={14} />
        </button>
        <span class="d-id">#{sel.id}</span>
        <span class="d-title">{sel.title}</span>
        <span class="spacer"></span>
        <button class="icon-btn" title={t('boardDeleteIssue')} aria-label={t('boardDeleteIssue')} onclick={removeIssue}>
          <Icon name="trash" size={14} />
        </button>
      </div>
      <div class="d-meta">
        <Select value={sel.status} options={STATUSES.map((s) => ({ value: s, label: statusLabel(s) }))}
          onchange={(v: string) => move(sel!.id, v)} />
        {#if sel.assignee}<span class="meta-bit">@{sel.assignee}</span>{/if}
        {#if sel.created_by}<span class="meta-bit">{t('boardOpenedBy')} {sel.created_by}</span>{/if}
      </div>
      {#if sel.body}
        <div class="d-body">{sel.body}</div>
      {/if}
      <div class="notes">
        {#if Array.isArray(sel.notes)}
          {#each sel.notes as n}
            <div class="note">
              <span class="n-author">{n.author}</span>
              <span class="n-body">{n.body}</span>
              <span class="n-at">{ago(n.at)}</span>
            </div>
          {/each}
        {/if}
      </div>
      <div class="note-add">
        <input placeholder={t('boardNotePh')} bind:value={noteText}
          onkeydown={(e) => e.key === 'Enter' && addNote()} />
        <button class="icon-btn go" title={t('save')} aria-label={t('save')} disabled={!noteText.trim() || busy} onclick={addNote}>
          <Icon name="check" size={14} />
        </button>
      </div>
    </div>
  {:else if creating}
    <!-- ── new issue: title is the one required field ── -->
    <div class="detail">
      <div class="d-head">
        <button class="icon-btn" title={t('cancel')} aria-label={t('cancel')} onclick={() => (creating = false)}>
          <Icon name="arrow-left" size={14} />
        </button>
        <span class="d-title">{t('boardNew')}</span>
        <span class="spacer"></span>
        <button class="icon-btn go" title={t('create')} aria-label={t('create')} disabled={!nTitle.trim() || busy} onclick={createIssue}>
          <Icon name="check" size={14} />
        </button>
      </div>
      <!-- svelte-ignore a11y_autofocus -->
      <input class="n-title" placeholder={t('boardTitlePh')} bind:value={nTitle} autofocus
        onkeydown={(e) => e.key === 'Enter' && createIssue()} />
      <textarea class="n-body" rows="5" placeholder={t('boardBodyPh')} bind:value={nBody}></textarea>
    </div>
  {:else}
    <!-- ── the board: four fixed columns, cards in movement order ── -->
    <div class="cols">
      {#each STATUSES as s (s)}
        <div class="colm">
          <div class="col-h">{statusLabel(s)}<span class="col-n">{col(s).length}</span></div>
          {#each col(s) as i (i.id)}
            <button class="card" onclick={() => openIssue(i.id)}>
              <span class="c-title">{i.title}</span>
              <span class="c-meta">
                #{i.id}
                {#if i.assignee}· @{i.assignee}{/if}
                {#if noteCount(i)}· {noteCount(i)} <Icon name="chat" size={10} />{/if}
                · {ago(i.updated_at)}
              </span>
            </button>
          {/each}
        </div>
      {/each}
    </div>
    {#if !issues.length && ready}
      <div class="empty">{t('boardEmpty')}</div>
    {/if}
    <button class="new-issue" onclick={() => (creating = true)}>
      <Icon name="plus" size={13} /> {t('boardNew')}
    </button>
  {/if}
  {#if err}<div class="err">{err}</div>{/if}
</div>

<style>
  .board {
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    padding: 10px;
    gap: 8px;
  }
  .empty { color: var(--text3); font-size: var(--fs-ui); padding: 18px 6px; }
  .err { color: var(--status-danger); font-size: var(--fs-meta); }

  /* Columns: wide drawer shows them side by side, a narrow one stacks them —
     same content, no second layout species. */
  .cols {
    display: grid;
    grid-template-columns: repeat(auto-fit, minmax(170px, 1fr));
    gap: 10px;
    align-items: start;
  }
  .col-h {
    font-size: var(--fs-meta);
    font-weight: 600;
    color: var(--text2);
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding: 2px 2px 6px;
    display: flex; gap: 6px; align-items: baseline;
  }
  .col-n { color: var(--text3); font-weight: 400; }
  .colm { display: flex; flex-direction: column; gap: 6px; min-width: 0; }

  .card {
    display: flex; flex-direction: column; gap: 3px;
    text-align: left;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--ui-radius-row);
    padding: 8px 10px;
    cursor: pointer;
    transition: background var(--t-fast), border-color var(--t-fast);
    min-width: 0;
  }
  .card:hover { background: var(--surface2); }
  .c-title { font-size: var(--fs-ui); color: var(--text); font-weight: 600; overflow-wrap: anywhere; }
  .c-meta { font-size: var(--fs-micro); color: var(--text3); display: flex; gap: 4px; align-items: center; flex-wrap: wrap; }

  .new-issue {
    align-self: flex-start;
    display: inline-flex; align-items: center; gap: 5px;
    background: none; border: none;
    color: var(--text3);
    font-size: var(--fs-ui);
    padding: 6px 8px;
    border-radius: var(--ui-radius-control);
    cursor: pointer;
    transition: background var(--t-fast), color var(--t-fast);
  }
  .new-issue:hover { background: var(--surface2); color: var(--accent); }

  /* ── detail / new form ── */
  .detail { display: flex; flex-direction: column; gap: 8px; min-height: 0; }
  .d-head { display: flex; align-items: center; gap: 6px; }
  .d-id { font-family: var(--ui-font-mono, monospace); font-size: var(--fs-meta); color: var(--text3); }
  .d-title { font-size: var(--fs-ui); font-weight: 600; color: var(--text); overflow-wrap: anywhere; }
  .spacer { flex: 1; }
  .d-meta { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  .meta-bit { font-size: var(--fs-meta); color: var(--text3); }
  .d-body {
    font-size: var(--fs-ui); color: var(--text);
    white-space: pre-wrap; overflow-wrap: anywhere;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--ui-radius-row);
    padding: 8px 10px;
  }
  .notes { display: flex; flex-direction: column; gap: 4px; }
  .note { display: flex; gap: 6px; align-items: baseline; font-size: var(--fs-meta); min-width: 0; }
  .n-author { color: var(--accent); font-weight: 650; flex: none; }
  .n-body { color: var(--text); overflow-wrap: anywhere; }
  .n-at { color: var(--text3); font-size: var(--fs-micro); margin-left: auto; flex: none; }
  .note-add { display: flex; gap: 6px; align-items: center; }
  .note-add input, .n-title, .n-body {
    flex: 1;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--ui-radius-control);
    color: var(--text);
    font-size: var(--fs-ui);
    padding: 7px 10px;
    font-family: inherit;
  }
  .n-body { resize: vertical; }
  .note-add input:focus, .n-title:focus, .n-body:focus { outline: none; border-color: var(--accent); }
</style>
