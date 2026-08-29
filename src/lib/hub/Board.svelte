<script lang="ts">
  /* The project task board (owner, 2026-08-29: "引入一个新的看板功能…人类有一个
     看板页面，能写任务issue，agent也可以读任务，修改任务状态，在看板上记录信息
     状态"). This is the HUMAN's half; agents read and update the same issues
     through `tmm board`. Four fixed columns — the status vocabulary is shared
     with the CLI (`projects::BOARD_STATUSES`), so a free-text status here
     would fork the language. Lives in the Hub drawer as a partition, like the
     terminal and Files. */
  import { boardList, boardGet, boardSave, boardNote, boardDelete, projectList, hubAgents, hubPost, type BoardIssue, type HubAgent } from '../core/ws.ts';
  import type { ProjectRow } from '../projects/projects.ts';
  import { t } from '../core/i18n.svelte.ts';
  import Icon from '../ui/Icon.svelte';
  import Select from '../ui/Select.svelte';
  import SideHandle from '../ui/SideHandle.svelte';
  import ConfirmDialog from '../ui/ConfirmDialog.svelte';
  import { draftOf, draftDirty, draftValid, draftPatch } from './board.ts';
  import { scrollFade } from '../core/scrollFade.ts';

  let { session = '', visible = true, onGoBack = null }: { session?: string; visible?: boolean; onGoBack?: ((fn: () => boolean) => void) | null } = $props();

  // Every project has its OWN board (issues are session-scoped like the chat
  // room), so the page carries the shared project sidebar (owner, 2026-08-29:
  // "board是不是也有一个侧边栏，我可以选择不同的项目"). The prop is the
  // FOLLOW default — the last-touched session, same as Files — and a pick
  // here overrides it until the prop moves again.
  let projects = $state<ProjectRow[]>([]);
  let cur = $state('');
  let picked = $state(false);      // compact: a picked project drills into its board
  // A dirty draft blocks the silent follow: the prop moving under an open
  // edit would reset the view and lose it with no confirm (board #11).
  $effect(() => { if (session && (!picked || !cur) && !dirty) cur = session; });
  async function loadProjects() {
    try {
      const r = await projectList();
      projects = r.projects;
      if (!cur && projects.length) cur = projects[0]?.project.session ?? '';
    } catch { /* keep the last list — "could not ask" is not "there is nobody" */ }
  }
  $effect(() => {
    if (!visible) return;
    loadProjects();
    const iv = setInterval(loadProjects, 20000);
    return () => clearInterval(iv);
  });
  function pick(s: string) {
    guard(() => {
      if (s !== cur) { cur = s; }
      picked = true;
    });
  }

  // The back gesture peels detail/form → list before leaving the page —
  // the same contract every page registers via onGoBack (Files defined it).
  $effect(() => {
    onGoBack?.(() => {
      if (pendingDiscard) { pendingDiscard = null; return true; }
      if (sel && dirty) { pendingDiscard = () => { sel = null; }; return true; }
      if (sel || creating) { sel = null; creating = false; return true; }
      if (picked) { picked = false; return true; }  // compact: back to the project list
      return false;
    });
  });

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
  // ── The DRAFT (board #11): opening an issue edits a COPY; only the explicit
  // Save persists, Cancel restores, and every path that would drop unsaved
  // edits (back, Esc, sidebar switch, the phone's back gesture) goes through
  // the app's confirm dialect instead of losing them silently.
  let draft = $state({ title: '', body: '' });
  let pendingDiscard = $state<null | (() => void)>(null);
  const dirty = $derived(sel ? draftDirty(draft, sel) : false);
  /** Run now, or park behind the discard confirm when the draft is dirty. */
  function guard(action: () => void) {
    if (dirty) pendingDiscard = action;
    else action();
  }
  let nAssignee = $state('');

  async function load() {
    try {
      const r = await boardList(cur);
      issues = r.issues;
      ready = true;
      err = '';
    } catch (e) {
      // A failed poll keeps the last board — "could not ask" ≠ "empty".
      err = String((e as Error)?.message ?? e);
    }
    // The assignee picker offers the project's MANAGED agents (the only ones
    // an assignment can be typed into). A failed read keeps the last roster.
    try {
      const a = await hubAgents(cur);
      agents = a.agents.filter((x) => x.managed);
    } catch { /* keep */ }
  }
  let agents = $state<HubAgent[]>([]);

  // Poll while visible: agents move cards from their panes, and the human
  // should see it without touching anything. Same verdict rule as the rooms —
  // nothing renders as "empty" before the first answer.
  $effect(() => {
    if (!visible || !cur) return;
    load();
    const iv = setInterval(load, 8000);
    return () => clearInterval(iv);
  });
  // Switching projects resets the view to the new board's list.
  $effect(() => {
    void cur;
    sel = null; creating = false; ready = false; issues = []; noteText = ''; nAssignee = ''; pendingDiscard = null;
  });

  async function openIssue(id: number) {
    try {
      sel = await boardGet(cur, id);
      draft = draftOf(sel);
      err = '';
    } catch (e) { err = String((e as Error)?.message ?? e); }
  }
  /** Re-fetch the open issue WITHOUT resetting the draft — a status/assignee
   * save or a note must not clobber title/body edits in progress. */
  async function refetchSel() {
    if (sel) {
      try { sel = await boardGet(cur, sel.id); err = ''; } catch (e) { err = String((e as Error)?.message ?? e); }
    }
    await load();
  }
  // Assigning DOES something (owner, 2026-08-29: "Assign 给某个 Agent 去做"):
  // besides the field, a non-empty assignment posts an @message — hub_post's
  // delivery types it into that agent's pane, so the agent actually starts.
  // ONE function carries that semantics; the detail Select and the create
  // dialog both route through it (board #11: create-with-assignee must
  // dispatch, never just write a label).
  async function dispatchAssign(id: number, name: string) {
    await boardSave(cur, { id, assignee: name });
    if (name) {
      await hubPost(cur, `@${name} ${t('boardAssignMsg').replace('{id}', String(id))}`);
    }
  }
  async function assign(id: number, name: string) {
    busy = true;
    try {
      await dispatchAssign(id, name);
      await refetchSel();
    } catch (e) { err = String((e as Error)?.message ?? e); }
    busy = false;
  }
  /** Explicit save of the draft's changed fields; a failure KEEPS the draft. */
  async function saveDraft() {
    if (!sel || busy) return;
    const patch = draftPatch(draft, sel);
    if (!patch) return;
    busy = true;
    try {
      await boardSave(cur, { id: sel.id, ...patch });
      await refetchSel();
      draft = draftOf(sel);
    } catch (e) { err = String((e as Error)?.message ?? e); }
    busy = false;
  }
  function cancelDraft() {
    draft = draftOf(sel);
  }
  async function move(id: number, status: string) {
    busy = true;
    try { await boardSave(cur, { id, status }); await refetchSel(); } catch (e) { err = String((e as Error)?.message ?? e); }
    busy = false;
  }
  async function createIssue() {
    if (!nTitle.trim() || busy) return;
    busy = true;
    try {
      const r = await boardSave(cur, { title: nTitle.trim(), body: nBody.trim() });
      // Create-with-assignee reuses the ONE dispatch semantics: the field is
      // saved AND the assignment lands in the agent's pane (board #11).
      if (nAssignee && r?.id) await dispatchAssign(r.id, nAssignee);
      nTitle = ''; nBody = ''; nAssignee = ''; creating = false;
      await load();
    } catch (e) { err = String((e as Error)?.message ?? e); }
    busy = false;
  }
  async function addNote() {
    if (!sel || !noteText.trim() || busy) return;
    busy = true;
    try { await boardNote(cur, sel.id, noteText.trim()); noteText = ''; await refetchSel(); } catch (e) { err = String((e as Error)?.message ?? e); }
    busy = false;
  }
  async function removeIssue() {
    if (!sel || busy) return;
    busy = true;
    try { await boardDelete(cur, sel.id); sel = null; await load(); } catch (e) { err = String((e as Error)?.message ?? e); }
    busy = false;
  }

  // Esc peels the board's own layers (detail → list, form → list) before the
  // drawer's close sees it — same territory rule as the files partition.
  function onKey(e: KeyboardEvent) {
    if (e.key !== 'Escape') return;
    if (pendingDiscard) return; // the ConfirmDialog's own capture handler closes itself
    if (sel && dirty) { pendingDiscard = () => { sel = null; }; e.stopPropagation(); return; }
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
<div class="board-root" class:picked>
  <aside class="sidebar">
    <SideHandle />
    <div class="side-scroll subtle-scroll" use:scrollFade>
      <div class="side-h">{t('hubProjects')}</div>
      {#each projects as p (p.project.session)}
        <button class="side-row" class:open={cur === p.project.session} onclick={() => pick(p.project.session)}>
          <span class="r-name">{p.project.name}</span>
          {#if !p.live}<span class="r-dim">○</span>{/if}
        </button>
      {/each}
    </div>
  </aside>
  <div class="board" onkeydowncapture={onKey}>
  <div class="head">
    <h1>{t('board')}</h1>
    {#if cur}<span class="h-session">{cur}</span>{/if}
  </div>
  {#if !ready && !issues.length}
    <div class="empty">…</div>
  {:else if sel}
    <!-- ── one issue: the note thread is the issue's own record ── -->
    <div class="detail">
      <div class="d-head">
        <button class="icon-btn" title={t('back')} aria-label={t('back')} onclick={() => guard(() => (sel = null))}>
          <Icon name="arrow-left" size={14} />
        </button>
        <span class="d-id">#{sel.id}</span>
        <span class="spacer"></span>
        {#if dirty}
          <!-- The draft's own verbs, only while there is a draft to speak of:
               cancel restores the stored issue, save persists the changed
               fields — and is the ONLY thing that does (board #11). -->
          <button class="icon-btn" title={t('cancel')} aria-label={t('cancel')} disabled={busy} onclick={cancelDraft}>
            <Icon name="undo" size={14} />
          </button>
          <button class="icon-btn go" title={t('save')} aria-label={t('save')} disabled={busy || !draftValid(draft)} onclick={saveDraft}>
            <Icon name="check" size={14} />
          </button>
        {/if}
        <button class="icon-btn" title={t('boardDeleteIssue')} aria-label={t('boardDeleteIssue')} disabled={busy} onclick={removeIssue}>
          <Icon name="trash" size={14} />
        </button>
      </div>
      <!-- Layout hierarchy (board #11): the title is ONE compact line, the
           body is the big field — visibly larger, growable, never same-size
           siblings again. -->
      <input class="d-title-input" bind:value={draft.title} placeholder={t('boardTitlePh')} />
      <div class="d-meta">
        <Select value={sel.status} options={STATUSES.map((s) => ({ value: s, label: statusLabel(s) }))}
          onchange={(v: string) => move(sel!.id, v)} />
        <Select value={sel.assignee} dense
          options={[{ value: '', label: t('boardUnassigned') }, ...agents.map((a) => ({ value: a.name, label: `@${a.name}` }))]}
          onchange={(v: string) => assign(sel!.id, v)} />
        {#if sel.created_by}<span class="meta-bit">{t('boardOpenedBy')} {sel.created_by}</span>{/if}
      </div>
      <textarea class="d-body-edit" bind:value={draft.body} placeholder={t('boardBodyPh')} rows="8"></textarea>
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
      <textarea class="n-body d-body-edit" rows="8" placeholder={t('boardBodyPh')} bind:value={nBody}></textarea>
      <!-- Assign at birth: the same dispatch as the detail picker — the agent
           is briefed the moment the issue exists (board #11). -->
      <div class="d-meta">
        <Select value={nAssignee} dense
          options={[{ value: '', label: t('boardUnassigned') }, ...agents.map((a) => ({ value: a.name, label: `@${a.name}` }))]}
          onchange={(v: string) => (nAssignee = v)} />
      </div>
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
              {#if i.body}<span class="c-body">{i.body}</span>{/if}
              <span class="c-meta">
                #{i.id}
                {#if i.created_by}· {t('boardBy')} {i.created_by}{/if}
                {#if i.assignee}· <span class="c-assignee">@{i.assignee}</span>{/if}
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
  <ConfirmDialog open={!!pendingDiscard} danger={false}
    title={t('confirmDiscardTitle')} note={t('boardDiscardNote')}
    confirmLabel={t('confirmDiscard')} cancelLabel={t('cancel')}
    onconfirm={() => { const go = pendingDiscard; pendingDiscard = null; draft = draftOf(sel); go?.(); }}
    oncancel={() => (pendingDiscard = null)} />
</div>

<style>
  /* Page skeleton (ui-unification §1): the shared sidebar + a main column.
     Compact is the same drill-down every page speaks: the list is the first
     screen, a picked project takes it (the back gesture peels it off). */
  .board-root { height: 100%; display: grid; grid-template-columns: var(--sidebar-w) minmax(0, 1fr); min-height: 0; background: var(--bg); }
  .sidebar { position: relative; background: var(--bg2); border-right: 1px solid var(--border); display: flex; flex-direction: column; min-height: 0; }
  .side-scroll { flex: 1; overflow-y: auto; min-height: 0; }
  .r-name { flex: 1; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .r-dim { color: var(--text3); font-size: var(--fs-micro); }
  @media (max-width: 760px) {
    .board-root { grid-template-columns: minmax(0, 1fr); }
    .sidebar { border-right: none; }
    .board-root.picked .sidebar { display: none; }
    .board-root:not(.picked) .board { display: none; }
  }
  .board {
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow-y: auto;
    padding: 14px clamp(10px, 3vw, 28px);
    gap: 10px;
    max-width: 1100px;
    margin: 0 auto;
    width: 100%;
    box-sizing: border-box;
    min-width: 0;
  }
  .head { display: flex; align-items: baseline; gap: 10px; }
  .head h1 { font-size: var(--fs-title); font-weight: 600; color: var(--text); margin: 0; }
  .h-session { font-family: ui-monospace, Menlo, monospace; font-size: var(--fs-meta); color: var(--text3); }
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
  /* The body PREVIEW (owner, 2026-08-29): short text shows whole, long text
     clamps — one mechanism, the clamp; the card is already the door to the
     detail view, so a clamped preview needs no separate "more" control. */
  .c-body {
    font-size: var(--fs-meta);
    color: var(--text2);
    white-space: pre-line;
    overflow-wrap: anywhere;
    display: -webkit-box;
    -webkit-line-clamp: 4;
    line-clamp: 4;
    -webkit-box-orient: vertical;
    overflow: hidden;
  }
  .c-meta { font-size: var(--fs-micro); color: var(--text3); display: flex; gap: 4px; align-items: center; flex-wrap: wrap; }
  .c-assignee { color: var(--accent); }

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
  /* The hierarchy (board #11): one compact title LINE, then the body as the
     visibly bigger field — it is the content, so it gets the space. */
  .d-title-input {
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--ui-radius-control);
    color: var(--text);
    font-size: var(--fs-ui);
    font-weight: 600;
    padding: 7px 10px;
  }
  .d-title-input:focus { outline: none; border-color: var(--accent); }
  .d-body-edit {
    font-size: var(--fs-ui); color: var(--text);
    font-family: inherit;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--ui-radius-row);
    padding: 8px 10px;
    min-height: 200px;
    resize: vertical;
    flex: none;
  }
  .d-body-edit:focus { outline: none; border-color: var(--accent); }
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
