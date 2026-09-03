<script lang="ts">
  /* The project task board (owner, 2026-08-29: "引入一个新的看板功能…人类有一个
     看板页面，能写任务issue，agent也可以读任务，修改任务状态，在看板上记录信息
     状态"). This is the HUMAN's half; agents read and update the same issues
     through `tmm board`. Four fixed columns — the status vocabulary is shared
     with the CLI (`projects::BOARD_STATUSES`), so a free-text status here
     would fork the language. Lives in the Hub drawer as a partition, like the
     terminal and Files. */
  import { boardList, boardGet, boardSave, boardNote, boardDelete, boardCounts, projectList, hubAgents, hubPost, hubRooms, type BoardIssue, type BoardCountRow, type HubAgent } from '../core/ws.ts';
  import { sortRows } from '../projects/projects.ts';
  import { agoShort, boardStatusColor } from './hub.ts';
  import type { ProjectRow } from '../projects/projects.ts';
  import { t } from '../core/i18n.svelte.ts';
  import { untrack } from 'svelte';
  import Icon from '../ui/Icon.svelte';
  import Select from '../ui/Select.svelte';
  import SideHandle from '../ui/SideHandle.svelte';
  import ConfirmDialog from '../ui/ConfirmDialog.svelte';
  import { draftOf, draftDirty, draftValid, draftPatch, rebaseDraft, issueRef, countsOf, applyCounts, visibleBoards, boardTitle, assignNotes, chipCols, noteActsSet, noteActsCopyLanded, noteActsExpired, NOTE_ACTS_IDLE, type NoteActsState } from './board.ts';
  import { scrollFade } from '../core/scrollFade.ts';

  let { session = '', visible = true, onGoBack = null, issueRequest = null, embedded = false, createRequest = null, jumped = false }: { session?: string; visible?: boolean; onGoBack?: ((fn: () => boolean) => void) | null; issueRequest?: { session: string; id: number; n: number } | null; embedded?: boolean; createRequest?: { n: number } | null; jumped?: boolean } = $props();

  // Every project has its OWN board (issues are session-scoped like the chat
  // room), so the page carries the shared project sidebar (owner, 2026-08-29:
  // "board是不是也有一个侧边栏，我可以选择不同的项目"). The prop is the
  // FOLLOW default — the last-touched session, same as Files — and a pick
  // here overrides it until the prop moves again.
  let cur = $state('');
  let picked = $state(false);      // a manual pick overrides the session follow
  // The Board sheet's own condition (≤760px — the old media gate, expressed
  // where the class is applied; see app.css .side-sheet).
  let narrowVp = $state(typeof window !== 'undefined' && window.matchMedia('(max-width: 760px)').matches);
  $effect(() => {
    const mq = window.matchMedia('(max-width: 760px)');
    const fn = () => { narrowVp = mq.matches; };
    mq.addEventListener('change', fn);
    return () => mq.removeEventListener('change', fn);
  });
  let sideOpen = $state(false);    // compact: the hamburger DRAWER (same dialect
                                   // as Chat/Terminal — reopened #11, no more
                                   // second-page drilldown)
  // UNSAVED WORK blocks the silent follow — a dirty edit or typed create
  // data alike (board #11; #29 review): the prop moving under either would
  // reset the view and wipe it with no confirm, bypassing the shared
  // ConfirmDialog. The board keeps its current project until the user
  // explicitly cancels/creates/saves; the effect reads both flags, so the
  // moment they clear it follows again.
  $effect(() => { if (session && (!picked || !cur) && !dirty && !createDirty) cur = session; });
  // A jump from the feed's board line (board #13 follow-up): the request names
  // its OWN session — a manual pick may have parked this page on another
  // project, and the issue id is session-gated. The dirty-draft guard still
  // stands between the jump and an open edit.
  let issueReqSeen = $state(0);
  $effect(() => {
    const req = issueRequest;
    if (!req || req.n === issueReqSeen) return;
    issueReqSeen = req.n;
    guard(() => {
      if (req.session && req.session !== cur) { cur = req.session; picked = !embedded; }
      creating = false; sideOpen = false;
      void openIssue(req.id);
    });
  });
  // Embedded, the new-issue button lives in the DRAWER head (board #23: the
  // embedded page-head was one redundant project-name row), so creation
  // arrives as a request. The dirty-draft guard still stands.
  let createReqSeen = $state(0);
  $effect(() => {
    const req = createRequest;
    if (!req || req.n === createReqSeen) return;
    createReqSeen = req.n;
    guard(() => { sel = null; creating = true; });
  });
  let talkMap = $state<Record<string, number>>({});
  const rowTalk = (r: { project: { room?: string; session: string } }) =>
    talkMap[r.project.room || `proj:${r.project.session}`] ?? 0;
  // The FULL non-archived list vs what the sidebar shows (board #39: "如果该
  // 项目完全为空 则直接不显示该 project"): the rows are filtered by the bulk
  // counts — total>0 — but allProjects stays whole, because the CURRENT board
  // may be empty and hidden while its page-head still names it and its main
  // area still creates the first issue. DERIVED, so a local counts update
  // (create/delete below) re-filters without waiting for any poll.
  let allProjects = $state<ProjectRow[]>([]);
  let countsMap = $state<Record<string, BoardCountRow>>({});
  const projects = $derived(visibleBoards(allProjects, countsMap));
  async function loadProjects() {
    try {
      // The SAME order as the Hub/Chat sidebar (reopened #11): newest
      // conversation first — one hub_rooms read feeds sortRows, exactly the
      // recipe the Hub uses. boardCounts is the third bulk read (board #39):
      // ONE grouped RPC for every project's four column counts — a
      // per-project boardList walk here is the N+1 the server call exists
      // to prevent.
      const [r, rooms, bc] = await Promise.all([
        projectList(),
        hubRooms().catch(() => ({ rooms: {} })),
        boardCounts().catch(() => null),
      ]);
      talkMap = rooms.rooms ?? {};
      allProjects = sortRows((r.projects ?? []).filter((row) => !row.project.archived), talkMap);
      // A failed counts read keeps the last map — "could not ask" is not
      // "every board is empty" (the sidebar would blank otherwise).
      if (bc) countsMap = bc.counts ?? {};
      // No session to follow: land on the first NON-EMPTY board — the ones
      // the sidebar actually shows (board #39).
      if (!cur) cur = visibleBoards(allProjects, countsMap)[0]?.project.session ?? '';
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
      sideOpen = false; // choosing closes the drawer
    });
  }

  // The back gesture peels detail/form → list before leaving the page —
  // the same contract every page registers via onGoBack (Files defined it).
  $effect(() => {
    onGoBack?.(() => {
      if (pendingDelete) { pendingDelete = null; return true; } // back dismisses, never confirms
      if (pendingDiscard) { pendingDiscard = null; return true; }
      if (sel && dirty) { pendingDiscard = () => { sel = null; }; return true; }
      if (creating && createDirty) { pendingDiscard = () => { creating = false; }; return true; }
      if (sel || creating) { sel = null; creating = false; return true; }
      // Compact bare list: back LIFTS the project drawer — the drawer is the
      // FLOOR, exactly Hub's compact rule (back with it open falls through,
      // so it can never cycle open/close). Except when the page was JUMPED
      // INTO from the chat: then App's return slot below is the floor, and
      // back belongs to the conversation (board #47: the bottom-bar entry
      // used to fall straight to the terminal).
      if (narrowVp && !sideOpen && !jumped) { sideOpen = true; return true; }
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
  let nAssignee = $state('');
  let noteText = $state('');
  let busy = $state(false);
  // ── The DRAFT (board #11): opening an issue edits a COPY; only the explicit
  // Save persists, Cancel restores, and every path that would drop unsaved
  // edits (back, Esc, sidebar switch, the phone's back gesture) goes through
  // the app's confirm dialect instead of losing them silently.
  let draft = $state(draftOf(null));
  // The draft's BASE: the server text the user started from (or last saved/
  // rebased onto). Dirty and the save patch are measured against IT, never
  // the live issue — a concurrent refetch moves the live copy, and diffing
  // against that would ship stale fields the user never touched (#11 review).
  let draftBase = $state(draftOf(null));
  let pendingDiscard = $state<null | (() => void)>(null);
  const dirty = $derived(sel ? draftDirty(draft, draftBase) : false);
  // The create form's typed-but-uncreated data is the same kind of unsaved
  // work as a dirty edit (board #29): any exit that would drop it — cancel,
  // Escape, back, a sidebar project pick — asks through the SAME guard, and
  // a clean form navigates without a pointless dialog.
  const createDirty = $derived(creating && !!(nTitle.trim() || nBody.trim() || nAssignee));
  /** Run now, or park behind the discard confirm when unsaved work exists. */
  function guard(action: () => void) {
    if (dirty || createDirty) pendingDiscard = action;
    else action();
  }

  async function load() {
    // FROZEN at entry (lead review, board #39): every await below is a window
    // for the user to switch projects, and with live `cur` the OLD board's
    // response would paint its issues into the NEW board — and, worse since
    // the counts fold, apply the old list's counts to the new session,
    // hiding/showing the wrong project in the sidebar. So the whole call is
    // about `s`: the RPCs ask for it, and every landing re-checks identity
    // before touching state — a stale response (or a stale ERROR) is dropped
    // whole; the switch effect already reset the view, and s's own next
    // reader is gone.
    const s = cur;
    try {
      const r = await boardList(s);
      if (cur !== s) return;
      issues = r.issues;
      ready = true;
      err = '';
      // The fresh list IS this board's counts: fold them into the bulk map
      // at once (board #39: "删除最后一条立即从 sidebar 消失，创建第一条立即
      // 出现") — load() runs after every create/delete/save, so the sidebar
      // reacts NOW instead of waiting out the 20 s projects poll. applyCounts
      // removes the key when the list is empty, the same absence the server
      // speaks. Keyed by the FROZEN session — the one the issues belong to.
      countsMap = applyCounts(countsMap, s, issues);
    } catch (e) {
      if (cur !== s) return;
      // A failed poll keeps the last board — "could not ask" ≠ "empty".
      err = String((e as Error)?.message ?? e);
    }
    // The assignee picker offers the project's MANAGED agents (the only ones
    // an assignment can be typed into). A failed read keeps the last roster.
    try {
      const a = await hubAgents(s);
      if (cur !== s) return;
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
    sel = null; creating = false; ready = false; issues = []; noteText = ''; nTitle = ''; nBody = ''; nAssignee = ''; pendingDiscard = null; pendingDelete = null;
    // untrack: this effect runs on `cur` — reading acts to bump its gen
    // would ALSO subscribe the effect to acts, and writing it back loops
    // the effect to death (caught live: effect_update_depth_exceeded).
    acts = noteActsSet(untrack(() => acts), -1);
  });

  /** The editor boxes ADAPT to their content (owner, 2026-08-29: "有的框很大
   * 有空白 应该自适应"): height follows scrollHeight on input and whenever the
   * bound value changes programmatically (opening an issue swaps the draft).
   * The CSS min-height is the floor; manual resize stays available. */
  function autoGrow(el: HTMLTextAreaElement, _value: string) {
    const fit = () => { el.style.height = 'auto'; el.style.height = `${el.scrollHeight + 2}px`; };
    el.addEventListener('input', fit);
    fit();
    return {
      update: (_v: string) => fit(),
      destroy: () => el.removeEventListener('input', fit),
    };
  }

  // ── The status slider (board #15): one segmented track, slide or tap ──
  // It edits the DRAFT — nothing reaches the server until the ✓ confirms
  // ("避免手动手滑随便一点就改变了状态"). Pointer capture makes a swipe from
  // any segment sweep the track; a plain tap lands on its segment.
  let segEl = $state<HTMLElement | null>(null);
  let segDrag = $state(false);
  function segPick(e: PointerEvent) {
    if (!segEl) return;
    const r = segEl.getBoundingClientRect();
    const i = Math.min(STATUSES.length - 1, Math.max(0, Math.floor(((e.clientX - r.left) / r.width) * STATUSES.length)));
    draft.status = STATUSES[i]!;
  }
  function segDown(e: PointerEvent) {
    segDrag = true;
    segEl?.setPointerCapture?.(e.pointerId);
    segPick(e);
  }

  async function openIssue(id: number) {
    try {
      sel = await boardGet(cur, id);
      draft = draftOf(sel);
      draftBase = draftOf(sel);
      acts = noteActsSet(acts, -1); // a different issue, a fresh slate (board #46)
      err = '';
    } catch (e) { err = String((e as Error)?.message ?? e); }
  }
  /** Re-fetch the open issue and REBASE the draft three-way: untouched
   * fields follow the server (an agent's new body shows up mid-edit),
   * touched fields keep the user's text (#11 review). */
  async function refetchSel() {
    if (sel) {
      try {
        sel = await boardGet(cur, sel.id);
        const r = rebaseDraft(draft, draftBase, draftOf(sel));
        draft = r.draft;
        draftBase = r.base;
        err = '';
      } catch (e) { err = String((e as Error)?.message ?? e); }
    }
    await load();
  }
  // Assigning DOES something (owner, 2026-08-29: "Assign 给某个 Agent 去做"):
  // besides the field, a non-empty assignment posts an @message — hub_post's
  // delivery types it into that agent's pane, so the agent actually starts.
  // ONE function carries that semantics; the detail Select and the create
  // dialog both route through it (board #11: create-with-assignee must
  // dispatch, never just write a label).
  // The brief CARRIES the issue (owner, 2026-08-30): the ORIGINAL title/body
  // are the task input, so they ride in full and the agent starts without a
  // lookup. Only the note thread has a separate explicit budget below.
  async function dispatchAssign(id: number, name: string, title = '', body = '', notes: { author: string; body: string; at: number }[] = []) {
    await boardSave(cur, { id, assignee: name });
    if (name) {
      const b = body.trim() ? ` — ${body.trim()}.` : '.';
      // The {title} slot names the issue; with the body already riding in
      // {body}, a titleless issue is named by its id (not the body twice).
      // The SUBJECT leads (board #51: "前边有谁分给他一个主体名称"): who
      // assigned it, then the issue, then the notes, then the ask — the
      // reading order of a handoff. The UI's dispatch is the operator's act,
      // and 'human' is the name agents know the operator by (hubPost below
      // posts as the same identity).
      const msg = t('boardAssignMsg')
        .replaceAll('{id}', String(id))
        .replace('{who}', 'human')
        .replace('{title}', title.trim() || '#' + id)
        .replace('{body}', b);
      // The note thread rides along (board #42): the discussion under an
      // issue is context the agent must not miss, appended AFTER the rendered
      // message under its own header, chronological, authors kept, and
      // budget-capped with the `tmm board show` pointer — assignNotes owns
      // that shape. A fresh issue has no notes and appends nothing. The tmm
      // instructions come LAST (board #51: "please tmm xxxx这样的顺序") —
      // after the content, never between the issue and its thread.
      const take = t('boardAssignTake').replaceAll('{id}', String(id));
      await hubPost(cur, `@${name} ${msg}${assignNotes(id, notes)}\n${take}`);
    }
  }
  /** Explicit save of the USER's changed fields (diffed against the draft
   * base, never the live issue); a failure KEEPS the draft. The refetch's
   * rebase then normalizes: the server now carries the saved text, so base
   * catches up and the draft reads clean. */
  async function saveDraft() {
    if (!sel || busy) return;
    const patch = draftPatch(draft, draftBase);
    if (!patch) return;
    busy = true;
    try {
      // The assignee travels through dispatchAssign — the ONE carrier of
      // assignment=dispatch semantics (board #11) — so a change confirmed by
      // the ✓ briefs the agent exactly like assign-at-birth does. Everything
      // else is an ordinary field patch.
      const { assignee, ...rest } = patch;
      if (Object.keys(rest).length) await boardSave(cur, { id: sel.id, ...rest });
      // A reassign from the detail view carries the OPEN issue's note thread
      // (board #42) — sel is the boardGet copy, so its notes are the array
      // (list rows only carry a count, which assignNotes treats as none).
      if (assignee !== undefined) await dispatchAssign(sel.id, assignee, draft.title, draft.body, Array.isArray(sel.notes) ? sel.notes : []);
      // The ✓ ANSWERS the edit (board #48 v2, owner: "点击对勾应该自动回到
      // 主页面，不用停留在详情页"): a successful save leaves the detail view
      // for the refreshed board. A FAILED save takes the catch instead —
      // the detail stays open with the error and the typed draft, because
      // a form that closes on failure eats the retry (createIssue's rule).
      err = '';
      sel = null;
      await load();
    } catch (e) { err = String((e as Error)?.message ?? e); }
    busy = false;
  }
  async function createIssue() {
    // Title OR body — the same not-contentless rule the server enforces.
    if (!(nTitle.trim() || nBody.trim()) || busy) return;
    busy = true;
    let created: number | null = null;
    try {
      created = (await boardSave(cur, { title: nTitle.trim(), body: nBody.trim() }))?.id ?? null;
    } catch (e) {
      // The CREATE failed: the form stays, retry is honest.
      err = String((e as Error)?.message ?? e);
      busy = false;
      return;
    }
    // The issue EXISTS from here on: close the form unconditionally — a
    // retryable form after a successful create is how duplicate issues are
    // born (#11 review). A failed dispatch is reported instead; the issue is
    // on the board and can be assigned from its detail view.
    const wantAssign = nAssignee;
    const wantTitle = nTitle; const wantBody = nBody; // captured — the form clears before the dispatch
    nTitle = ''; nBody = ''; nAssignee = ''; creating = false;
    try {
      // Create-with-assignee reuses the ONE dispatch semantics: the field is
      // saved AND the assignment lands in the agent's pane (board #11).
      if (wantAssign && created != null) await dispatchAssign(created, wantAssign, wantTitle, wantBody);
    } catch (e) {
      err = `#${created}: ${String((e as Error)?.message ?? e)}`;
    }
    await load();
    busy = false;
  }
  async function addNote() {
    if (!sel || !noteText.trim() || busy) return;
    busy = true;
    try { await boardNote(cur, sel.id, noteText.trim()); noteText = ''; await refetchSel(); } catch (e) { err = String((e as Error)?.message ?? e); }
    busy = false;
  }
  // ── Delete is CONFIRMED, and the request is CAPTURED (board #29): the
  // dialog carries the session + issue it was opened FOR, and the executor
  // uses exactly that — a poll refetch, a selection change or a project
  // switch while the dialog stands open cannot redirect the delete. Nothing
  // destructive runs before the confirm; busy blocks a double confirm.
  let pendingDelete = $state<null | { session: string; id: number; title: string }>(null);
  function requestDelete() {
    if (!sel || busy) return;
    pendingDelete = { session: cur, id: sel.id, title: issueRef(sel) };
  }
  async function confirmDelete() {
    const cap = pendingDelete;
    if (!cap || busy) return;
    busy = true;
    try {
      await boardDelete(cap.session, cap.id);
      // Only the MATCHING view is cleaned: if the user moved to another
      // project meanwhile, that board is not ours to touch.
      if (cur === cap.session) {
        if (sel?.id === cap.id) sel = null;
        await load();
      }
    } catch (e) {
      if (cur === cap.session) err = String((e as Error)?.message ?? e);
    }
    busy = false;
    pendingDelete = null;
  }

  // ── Note actions (board #46: "点击中间 Agent 或人回复的消息…出现一个 copy
  // 按钮"): tapping a note's body reveals ONE Copy action on the bubble's
  // corner — Chat's action-row pattern, wearing Chat's own .m-acts atoms
  // from app.css (a scoped copy is dialect drift). acts.open is the single
  // source of WHICH row is open; another note's tap switches, the same
  // note's tap closes, outside/Escape/issue-switch put it away. Every
  // transition is a pure board.ts function over ONE state triple, and
  // acts.gen makes the Copy beat's timeout self-scoped (review blocker:
  // a global boolean let Copy A's stale timeout close Copy B's row).
  let acts = $state<NoteActsState>(NOTE_ACTS_IDLE);
  function toggleNoteActs(i: number) {
    // A drag-selection's tail click must not steal the selection — the note
    // text is swipe-selectable (#43); the action row is for a plain tap.
    if (typeof getSelection === 'function' && !(getSelection()?.isCollapsed ?? true)) return;
    acts = noteActsSet(acts, acts.open === i ? -1 : i);
  }
  async function copyNote(body: string) {
    // The attempt's identity, captured BEFORE the await (second blocker):
    // the clipboard write is async, and by resolve time the user may be on
    // another note or another issue — that resolve must not stamp Copied
    // onto the new context, nor arm a timer against it.
    const attempt = acts.gen;
    try {
      await navigator.clipboard.writeText(body ?? '');
      const next = noteActsCopyLanded(acts, attempt);
      if (next === acts) return; // the context moved mid-flight; the resolve is orphaned
      acts = next;
      // The Copied beat, then the row puts itself away — copying IS what the
      // row was opened for (Chat's own 1.5 s). The timeout captures ITS gen:
      // it may expire only the copy it belongs to.
      const gen = next.gen;
      setTimeout(() => { acts = noteActsExpired(acts, gen); }, 1500);
    } catch (e) { console.warn('copy failed', e); }
  }
  // Outside pointerdown / Escape close the open row — the transient-layer
  // rule every popover follows. WINDOW-level capture, active only while a
  // row is open: the note text is a div, so a click leaves focus on <body>
  // and a .bmain-scoped key handler would never hear the Escape (measured —
  // the row survived it). Open dialogs keep their own Escape.
  $effect(() => {
    if (acts.open < 0) return;
    const onDown = (e: PointerEvent) => {
      const el = e.target as HTMLElement | null;
      if (!el?.closest?.('.m-acts, .n-wrap, .n-at')) acts = noteActsSet(acts, -1);
    };
    const onEsc = (e: KeyboardEvent) => {
      if (e.key !== 'Escape' || pendingDiscard || pendingDelete) return;
      acts = noteActsSet(acts, -1); e.stopPropagation();
    };
    window.addEventListener('pointerdown', onDown, true);
    window.addEventListener('keydown', onEsc, true);
    return () => { window.removeEventListener('pointerdown', onDown, true); window.removeEventListener('keydown', onEsc, true); };
  });

  // Esc peels the board's own layers (detail → list, form → list) before the
  // drawer's close sees it — same territory rule as the files partition.
  // (The note action row peels FIRST, via its own window-capture listener
  // above — it stops propagation, so this handler never sees that press.)
  function onKey(e: KeyboardEvent) {
    if (e.key !== 'Escape') return;
    if (pendingDiscard || pendingDelete) return; // the ConfirmDialog's own capture handler closes itself
    if (sideOpen) { sideOpen = false; e.stopPropagation(); return; }
    if (sel && dirty) { pendingDiscard = () => { sel = null; }; e.stopPropagation(); return; }
    if (creating && createDirty) { pendingDiscard = () => { creating = false; }; e.stopPropagation(); return; }
    if (sel || creating) { sel = null; creating = false; e.stopPropagation(); }
  }

  const statusLabel = (s: string) => t(`boardStatus_${s}`);
  /** The SIDEBAR count chips' colours — the owner's four CATEGORICAL colours
   * in board order (2026-09-01, fourth ruling: "红 蓝 黄紫 吧，橙色和红色区
   * 分不明显" — at a 5px dot the third ruling's red/orange pair blurred, so
   * doing wears BLUE: var(--accent), the app's own blue and incidentally the
   * colour the global language already gives doing). These are counts, not
   * severities: red-todo is category paint, not alarm. All theme tokens (a
   * hex literal is only right in one theme), none of them green — the first
   * ruling (no collision with the row's green LIVE dot) holds by
   * construction. Still deliberately SCOPED (lead): the feed's "→ done"
   * badge keeps boardStatusColor's language, unknown statuses delegate to
   * it, and the chips stay non-colour-readable — every one carries its
   * label and count. */
  const COUNT_COLORS: Record<string, string> = {
    todo: 'var(--status-danger)',
    doing: 'var(--accent)',
    review: 'var(--status-warn)',
    done: 'var(--status-purple)',
  };
  const countColor = (st: string) => COUNT_COLORS[st] ?? boardStatusColor(st);
  /** Dynamic chip layout (owner, 2026-09-01: "如果一行能 放下放一行也行，甚
   * 至两行放不下，就放一列，动态适配" — layered on 不要 3+1): a hidden GHOST
   * row (the composer's mirror-div pattern) with nowrap natural-width chips
   * reports the row width and the widest chip; chipCols (pure, tested) turns
   * the two into 4, 2 or 1 equal columns, and every real row wears the
   * answer inline. The ghost carries the sidebar's WIDEST count so digit
   * width is priced in, and its labels re-render with the locale. */
  const CHIP_GAP_X = 10; // .side-wins column-gap in app.css
  let winsW = $state(0);
  let chipWs = $state<number[]>([0, 0, 0, 0]);
  const maxCount = $derived(Math.max(0, ...Object.values(countsMap).flatMap(
    (c) => STATUSES.map((st) => Number(c[st as keyof BoardCountRow]) || 0),
  )));
  const cols = $derived(chipCols(winsW, Math.max(...chipWs), CHIP_GAP_X));
  const col = (s: string) => issues.filter((i) => i.status === s);
  const noteCount = (i: BoardIssue) => (typeof i.notes === 'number' ? i.notes : i.notes.length);
  const ago = (ts: number) => {
    const d = Math.max(0, Date.now() / 1000 - ts);
    return d < 3600 ? `${Math.max(1, Math.round(d / 60))}m` : d < 86400 ? `${Math.round(d / 3600)}h` : `${Math.round(d / 86400)}d`;
  };
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div class="board-root" class:embedded>
  {#if !embedded}
  <aside class="sidebar" class:side-sheet={narrowVp} class:open={narrowVp && sideOpen}>
    <SideHandle />
    <div class="side-scroll subtle-scroll" use:scrollFade>
      <div class="side-h">{t('hubProjects')}</div>
      <!-- The Chat sidebar's two-line row, atom for atom (board #39: "board
           侧边栏的样式也要和 chat terminal 的侧边栏对齐"): dot + name + age up
           top from the SHARED .proj-row/.p-* atoms in app.css, and the quiet
           second line wears the same .side-win chips the Terminal's windows
           and Chat's agents wear — here each chip is a COLUMN: its status
           color (boardStatusColor, the one board status language) and count,
           all four in fixed order, zeros included, so every row reads the
           same shape. Only boards WITH issues are listed (visibleBoards —
           the empty ones are hidden, never zero-rows). -->
      <!-- The measurement ghost: a real row's structure at height 0, chips
           nowrap at natural width. Its binds feed chipCols; it renders the
           WIDEST count so the answer already fits every real row. -->
      <div class="side-row proj-row mirror" aria-hidden="true">
        <span class="dot off"></span>
        <span class="p-main">
          <span class="side-wins" bind:clientWidth={winsW}>
            {#each STATUSES as st, i (st)}
              <span class="side-win" bind:clientWidth={chipWs[i]}>
                <span class="side-win-dot"></span>
                <span class="side-win-name">{statusLabel(st)}</span>
                <span class="b-count">{maxCount}</span>
              </span>
            {/each}
          </span>
        </span>
      </div>
      {#each projects as p (p.project.session)}
        {@const c = countsMap[p.project.session]}
        <button class="side-row proj-row" class:open={cur === p.project.session} onclick={() => pick(p.project.session)}>
          <span class="dot" class:off={!p.live}></span>
          <span class="p-main">
            <span class="p-top">
              <span class="p-name">{p.project.name}</span>
              {#if rowTalk(p)}<span class="side-age">{agoShort(rowTalk(p), Date.now())}</span>{/if}
            </span>
            <span class="side-wins grid" style:grid-template-columns={`repeat(${cols}, minmax(0, 1fr))`}>
              {#each STATUSES as st (st)}
                <span class="side-win">
                  <span class="side-win-dot" style:background={countColor(st)}></span>
                  <span class="side-win-name">{statusLabel(st)}</span>
                  <span class="b-count">{c?.[st as keyof BoardCountRow] ?? 0}</span>
                </span>
              {/each}
            </span>
          </span>
        </button>
      {/each}
    </div>
  </aside>
  {#if sideOpen}
    <!-- svelte-ignore a11y_click_events_have_key_events -->
    <!-- svelte-ignore a11y_no_static_element_interactions -->
    <div class="side-scrim" onclick={() => (sideOpen = false)}></div>
  {/if}
  {/if}
  <div class="bmain" onkeydowncapture={onKey}>
  <!-- The bar is Chat's own .page-head dialect (board #15: "board上边那一栏
       应该和chat样式一致…按钮风格也要一致"): the shared app.css class carries
       the height, padding, border and h1 type — a scoped re-style is the
       drift that split the sidebars once. The project name appears ONCE
       ("project名称写一遍就行了" — the session chip retired).
       EMBEDDED there is no bar at all (board #23: the drawer head already
       names the project — this row repeated it and nothing else); the
       new-issue + moves up into the drawer head, arriving as createRequest. -->
  {#if !embedded}
  <div class="page-head">
    <!-- Compact: the hamburger calls the project drawer — the same dialect as
         Chat and Terminal (reopened #11), never a second-page drilldown. -->
    <button class="icon-btn side-toggle" title={t('hubProjects')} aria-label={t('hubProjects')}
      onclick={() => (sideOpen = !sideOpen)}>
      <Icon name="menu" size={16} />
    </button>
    <!-- The page names the PROJECT, not itself (board #15): the tab already
         says "Board", so the title says WHOSE board this is. -->
    <h1>{boardTitle(allProjects, cur) ?? (cur || t('board'))}</h1>
    <span class="spacer"></span>
    {#if !sel && !creating}
      <!-- New issue lives in the page head's top-right (reopened #11:
           "不应该放到最下方…手机操作更友好"). -->
      <button class="icon-btn go" title={t('boardNew')} aria-label={t('boardNew')} onclick={() => (creating = true)}>
        <Icon name="plus" size={16} />
      </button>
    {/if}
  </div>
  {/if}
  <div class="board">
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
        <!-- Verb order (board #48 v2, owner: "一般习惯对勾在最右边，防止误
             点击"): trash first, then the draft's undo + ✓ — the ✓ is the
             LAST control so the muscle-memory rightmost tap confirms, and
             undo stands between it and delete. -->
        <button class="icon-btn" title={t('boardDeleteIssue')} aria-label={t('boardDeleteIssue')} disabled={busy} onclick={requestDelete}>
          <Icon name="trash" size={14} />
        </button>
        {#if dirty}
          <!-- The draft's own verbs, only while there is a draft to speak of:
               cancel restores the stored issue, save persists the changed
               fields — and is the ONLY thing that does (board #11). -->
          <!-- Cancel ASKS (board #15: "当前状态没有保存，是否退出"): the same
               ConfirmDialog every guarded exit uses — confirming restores the
               base, so a slid status or picked assignee rolls back. -->
          <button class="icon-btn" title={t('cancel')} aria-label={t('cancel')} disabled={busy} onclick={() => guard(() => {})}>
            <Icon name="undo" size={14} />
          </button>
          <button class="icon-btn go" title={t('save')} aria-label={t('save')} disabled={busy || !draftValid(draft)} onclick={saveDraft}>
            <Icon name="check" size={14} />
          </button>
        {/if}
      </div>
      <!-- Layout hierarchy (board #11): the title is ONE compact line, the
           body is the big field — visibly larger, growable, never same-size
           siblings again. -->
      <!-- Locked text is the RECORD (board #43, backend 842f970): once an
           agent has touched the issue — or while it is assigned — the server
           refuses title/body patches, so the frontend renders the ORIGINAL
           text as semantic static prose: complete, swipe-selectable, and
           carrying NO edit affordance (the bordered box read as "tap to
           edit" — a disabled input would be the same lie). The workflow
           stays live either way: status slider, assignee, note reply. -->
      {#if sel.editable}
        <input class="d-title-input" bind:value={draft.title} placeholder={t('boardTitlePh')} />
      {:else if sel.title.trim()}
        <div class="d-title-static">{sel.title}</div>
      {/if}
      <div class="d-meta">
        <!-- The status is a SLIDER (board #15): sweep the track or tap a
             stop. It edits the draft — the head's ✓ is what saves, and
             cancel asks before losing the change. -->
        <div class="seg" role="radiogroup" tabindex="-1" aria-label={statusLabel(draft.status || sel.status)}
          bind:this={segEl}
          onpointerdown={segDown}
          onpointermove={(e) => segDrag && segPick(e)}
          onpointerup={() => (segDrag = false)}
          onpointercancel={() => (segDrag = false)}>
          {#each STATUSES as st (st)}
            <button class="seg-b" class:on={draft.status === st} role="radio" aria-checked={draft.status === st}
              onclick={() => (draft.status = st)}>{statusLabel(st)}</button>
          {/each}
        </div>
        <Select value={draft.assignee} dense
          options={[{ value: '', label: t('boardUnassigned') }, ...agents.map((a) => ({ value: a.name, label: `@${a.name}` }))]}
          onchange={(v: string) => (draft.assignee = v)} />
        {#if sel.created_by}<span class="meta-bit">{t('boardOpenedBy')} <span class="m-name">{sel.created_by}</span></span>{/if}
      </div>
      {#if sel.editable}
        <textarea class="d-body-edit" bind:value={draft.body} use:autoGrow={draft.body} placeholder={t('boardBodyPh')} rows="3"></textarea>
      {:else if sel.body.trim()}
        <!-- The full body, VERBATIM (review blocker on ebda03b): this is the
             original history, so the display layer must not rewrite it —
             trim() only decides whether the field renders at all; the output
             is sel.body untouched, leading/trailing whitespace included
             (pre-wrap makes it real). -->
        <div class="d-body-static">{sel.body}</div>
      {/if}
      <!-- The note thread as a TIMELINE (reopened #11): a header line — author
           in the accent ink, time right-aligned — and the content in its own
           box below, so ragged name lengths stop pushing the text around. -->
      <div class="notes">
        {#if Array.isArray(sel.notes)}
          {#each sel.notes as n, i}
            <div class="note">
              <div class="n-head">
                <span class="n-author">{n.author}</span>
                <!-- The accessible route to the action row (board #46): the
                     note text stays TEXT to assistive tech — like the Chat
                     bubble — so the time is a real borderless button, Chat's
                     meta-trailer pattern. -->
                <button class="n-at" aria-label={t('hubMsgActions')}
                  onclick={(e) => { e.stopPropagation(); acts = noteActsSet(acts, acts.open === i ? -1 : i); }}>{ago(n.at)}</button>
              </div>
              <!-- fit-content relative wrapper: the overlay anchors to the
                   BUBBLE's corner, never the full row's far right. -->
              <div class="n-wrap">
                <!-- svelte-ignore a11y_no_static_element_interactions, a11y_click_events_have_key_events -->
                <div class="n-text" onclick={() => toggleNoteActs(i)}>{n.body.trim()}</div>
                {#if acts.open === i}
                  <div class="m-acts">
                    <button onclick={() => copyNote(n.body)}>
                      <Icon name="copy" size={11} />{acts.copied ? t('hubCopied') : t('hubCopy')}
                    </button>
                  </div>
                {/if}
              </div>
            </div>
          {/each}
        {/if}
      </div>
      <div class="note-add">
        <!-- A textarea, not an input (board #28: "消息过长要自动帮我换行，
             现在是一直在一行里，前边都看不到了"): long text soft-wraps in
             the visible width and the box grows with it — the same shared
             autoGrow the title/body edits use, one line at rest. Enter
             sends (guarded: Shift+Enter inserts a real newline, and an IME
             composition's Enter commits the composition, never the note);
             clearing on send shrinks the box back through autoGrow's
             update. -->
        <textarea class="note-input" rows="1" placeholder={t('boardNotePh')} bind:value={noteText}
          use:autoGrow={noteText}
          onkeydown={(e) => { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); addNote(); } }}></textarea>
        <button class="icon-btn go" title={t('save')} aria-label={t('save')} disabled={!noteText.trim() || busy} onclick={addNote}>
          <Icon name="check" size={14} />
        </button>
      </div>
    </div>
  {:else if creating}
    <!-- ── new issue: title is the one required field ── -->
    <div class="detail">
      <div class="d-head">
        <button class="icon-btn" title={t('cancel')} aria-label={t('cancel')} onclick={() => guard(() => (creating = false))}>
          <Icon name="arrow-left" size={14} />
        </button>
        <span class="d-title">{t('boardNew')}</span>
        <span class="spacer"></span>
        <button class="icon-btn go" title={t('create')} aria-label={t('create')} disabled={!(nTitle.trim() || nBody.trim()) || busy} onclick={createIssue}>
          <Icon name="check" size={14} />
        </button>
      </div>
      <!-- Owner-set order (2026-08-29): the title STARTS as one line and grows
           with its text; assign comes next; the body takes whatever height is
           left and scrolls INSIDE itself. -->
      <!-- svelte-ignore a11y_autofocus -->
      <textarea class="n-title one-line" rows="1" placeholder={t('boardTitlePh')} bind:value={nTitle} autofocus
        use:autoGrow={nTitle}
        onkeydown={(e) => { if (e.key === 'Enter' && !e.shiftKey && !e.isComposing) { e.preventDefault(); createIssue(); } }}></textarea>
      <!-- Assign at birth: the same dispatch as the detail picker — the agent
           is briefed the moment the issue exists (board #11). -->
      <div class="d-meta">
        <Select value={nAssignee} dense
          options={[{ value: '', label: t('boardUnassigned') }, ...agents.map((a) => ({ value: a.name, label: `@${a.name}` }))]}
          onchange={(v: string) => (nAssignee = v)} />
      </div>
      <!-- The body is MULTI-LINE, so Enter must stay a newline — the submit
           chord is Cmd+Enter (mac) / Ctrl+Enter (elsewhere), the pair every
           chat product speaks (board #36, owner 2026-09-01: "我在 board 填写
           完 issue 描述后，可以 cmd+enter 直接提交确认"). Same createIssue as
           the ✓ — a trigger, not a second submit path — and an IME
           composition's Enter commits the candidate text, never the issue
           (the note box's precedent, board #28). -->
      <textarea class="d-body-edit fill" placeholder={t('boardBodyPh')} bind:value={nBody}
        onkeydown={(e) => { if (e.key === 'Enter' && (e.metaKey || e.ctrlKey) && !e.isComposing) { e.preventDefault(); createIssue(); } }}></textarea>
    </div>
  {:else}
    <!-- ── the board: four fixed columns, cards in movement order ── -->
    <div class="cols">
      {#each STATUSES as s (s)}
        {@const items = col(s)}
        <!-- SPARSE (0–1 cards) vs DENSE (2+) drives the 1-column layout
             (board #33): a sparse area takes header+content height only,
             dense areas flex-share what remains — the class is inert in the
             2/4-column grids (flex properties do nothing for grid items). -->
        <div class="colm" class:sparse={items.length <= 1}>
          <div class="col-h">{statusLabel(s)}<span class="col-n">{items.length}</span></div>
          <div class="col-scroll subtle-scroll">
          {#each items as i (i.id)}
            <button class="card" onclick={() => openIssue(i.id)}>
              <!-- Titleless issues (board #31) wear their body excerpt as the
                   title (issueRef); the preview line then stays EMPTY — the
                   same text twice reads as a rendering bug. -->
              <span class="c-title">{issueRef(i)}</span>
              {#if i.body && i.title?.trim()}<span class="c-body">{i.body}</span>{/if}
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
        </div>
      {/each}
    </div>
    {#if !issues.length && ready}
      <div class="empty">{t('boardEmpty')}</div>
    {/if}
  {/if}
  {#if err}<div class="err">{err}</div>{/if}
  </div>
  </div>
  <ConfirmDialog open={!!pendingDiscard} danger={false} compact={narrowVp}
    title={t('confirmDiscardTitle')} note={creating ? t('boardCreateDiscardNote') : t('boardDiscardNote')}
    confirmLabel={t('confirmDiscard')} cancelLabel={t('cancel')}
    onconfirm={() => { const go = pendingDiscard; pendingDiscard = null; draft = { ...draftBase }; nTitle = ''; nBody = ''; nAssignee = ''; go?.(); }}
    oncancel={() => (pendingDiscard = null)} />
  <!-- Deleting is the DANGER confirmation (board #29): the dialog names the
       captured issue, nothing reaches boardDelete before the confirm, and
       busy holds the button through the RPC. -->
  <ConfirmDialog open={!!pendingDelete} danger compact={narrowVp} {busy}
    title={t('boardDeleteConfirmTitle').replace('{title}', pendingDelete?.title ?? '')}
    note={t('boardDeleteConfirmNote')}
    confirmLabel={t('boardDeleteIssue')} cancelLabel={t('cancel')}
    onconfirm={confirmDelete}
    oncancel={() => (pendingDelete = null)} />
</div>

<style>
  /* Page skeleton (ui-unification §1): the shared sidebar + a main column.
     Compact is the same drill-down every page speaks: the list is the first
     screen, a picked project takes it (the back gesture peels it off). */
  .board-root { height: 100%; display: grid; grid-template-columns: var(--sidebar-w) minmax(0, 1fr); min-height: 0; background: var(--bg); }
  .sidebar { position: relative; background: var(--bg2); border-right: 1px solid var(--border); display: flex; flex-direction: column; min-height: 0; }
  /* The same 8px scroll inset the Chat sidebar wears — the row/header INSET
     is .side-h/.side-row's own 10px in app.css, but the container padding
     was Board's silent 0 and the whole list sat 8px left of Chat's (board
     #39: "我看 projects 这些写的位置都不一样"). */
  .side-scroll { flex: 1; overflow-y: auto; min-height: 0; padding: 8px; }
  /* The count on a column chip: tabular so 9→10 does not wiggle the row. */
  .b-count { font-variant-numeric: tabular-nums; }
  /* Embedded in the Hub's right drawer: one column, the drawer names the
     project — no sidebar track, no hamburger (board #13 follow-up). */
  .board-root.embedded { grid-template-columns: minmax(0, 1fr); }
  .board-root.embedded .board { padding: 10px 12px; }
  /* Compact: the sidebar is a DRAWER over the board — the sheet geometry and
     motion are the SHARED .side-sheet dialect in app.css (owner, 2026-08-30:
     one drawer for Chat/Terminal/Board), never a second page. */
  .side-toggle { display: none; flex: none; }
  @media (max-width: 760px) {
    .board-root { grid-template-columns: minmax(0, 1fr); }
    .side-toggle { display: inline-flex; }
  }
  /* The main column: the shared page-head on top, the padded board below —
     the head spans full width like Chat's, the padding belongs to the content. */
  .bmain { display: flex; flex-direction: column; height: 100%; min-width: 0; overflow: hidden; }
  .board {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
    /* The PAGE holds still; each column scrolls its own cards (reopened #11:
       "应该是分区上下滑动才对"). The detail view brings its own scroller. */
    overflow: hidden;
    padding: 14px clamp(10px, 3vw, 28px);
    gap: 10px;
    max-width: 1100px;
    margin: 0 auto;
    width: 100%;
    box-sizing: border-box;
    min-width: 0;
    /* The board is the CONTAINER the column count answers to (board #27):
       the standalone page and the Hub drawer are the same rule by
       construction, because each asks its own width — the viewport plays
       no part. inline-size only: width comes from bmain either way. */
    container-type: inline-size;
    container-name: board;
  }
  .empty { color: var(--text3); font-size: var(--fs-ui); padding: 18px 6px; }
  .err { color: var(--status-danger); font-size: var(--fs-meta); }

  /* Columns: four areas can honestly tile as 1, 2 or 4 — NEVER 3 (board #27:
     "总共只有 4 个区域，控制列数在 1 2 4 不要出现 3"): at three across, the
     fourth wraps into a lonely orphan row, which is neither the side-by-side
     reading nor the stack. auto-fit was exactly the 3-column bug — it packs
     as many 170px tracks as fit, viewport and drawer alike. The ladder asks
     the BOARD container (see .board), so the standalone page and the Hub
     drawer obey the same thresholds by construction; the steps are the same
     math auto-fit used (170px min card, 10px gap): 2×170+10 = 350,
     4×170+3×10 = 710. Stacked modes cap each area's share of the height so
     every column keeps its OWN scroller — the page still holds still. */
  /* The 1-column BASE is a column flex, not an equal-row grid (board #33:
     four unconditionally equal areas squeezed every real column to a
     quarter-screen while empty ones stood stretched): a SPARSE area (0–1
     cards) is content-sized — header plus what it actually holds — and
     DENSE areas (2+) flex-share the remaining height, each keeping its own
     scroller. All four sparse leaves the leftover blank at the BOTTOM,
     never inflated into empty areas. No fixed card heights, no JS
     measuring — the class is the whole mechanism. */
  .cols {
    display: flex;
    flex-direction: column;
    gap: 10px;
    align-items: stretch;
    flex: 1;
    min-height: 0;
  }
  .colm.sparse { flex: none; }
  .colm:not(.sparse) { flex: 1 1 0; }
  /* ≥2 columns: back to the #27 grid — equal rows, never 3 across; flex
     properties on .colm are inert here (grid items ignore them). */
  @container board (min-width: 350px) {
    .cols {
      display: grid;
      grid-template-columns: repeat(2, minmax(0, 1fr));
      grid-auto-rows: minmax(0, 1fr);
    }
  }
  @container board (min-width: 710px) {
    .cols { grid-template-columns: repeat(4, minmax(0, 1fr)); }
  }
  .col-scroll {
    overflow-y: auto;
    min-height: 0;
    display: flex; flex-direction: column; gap: 6px;
    padding-bottom: 6px;
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
  .colm { display: flex; flex-direction: column; gap: 6px; min-width: 0; min-height: 0; }

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


  /* ── detail / new form ── */
  .detail { display: flex; flex-direction: column; gap: 8px; min-height: 0; flex: 1; overflow-y: auto; }
  .d-head { display: flex; align-items: center; gap: 6px; }
  .d-id { font-family: var(--ui-font-mono, monospace); font-size: var(--fs-meta); color: var(--text3); }
  .d-title { font-size: var(--fs-ui); font-weight: 600; color: var(--text); overflow-wrap: anywhere; }
  .spacer { flex: 1; }
  .d-meta { display: flex; align-items: center; gap: 10px; flex-wrap: wrap; }
  /* The status slider (board #15): one bordered track in the control dialect,
     equal stops; the chosen stop fills accent (the app's selected-state ink).
     touch-action none so the sweep is ours, not the page scroll's. */
  .seg {
    display: inline-flex; align-items: stretch;
    border: 1px solid var(--border); border-radius: var(--ui-radius-control);
    background: var(--surface); overflow: hidden;
    touch-action: none; user-select: none;
  }
  .seg-b {
    border: none; background: none; cursor: pointer;
    font-size: var(--fs-meta); color: var(--text2);
    padding: 5px 12px; position: relative;
    transition: background var(--t-fast), color var(--t-fast);
  }
  .seg-b + .seg-b { border-left: 1px solid var(--border); }
  .seg-b.on { background: var(--accent-fill); color: var(--accent-fill-ink); }
  .seg-b:not(.on):hover { background: var(--surface2); }
  .meta-bit { font-size: var(--fs-meta); color: var(--text3); }
  .meta-bit .m-name { color: var(--accent); font-weight: 650; }
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
    /* A floor, not a size: autoGrow raises the box to its content, so a
       two-line body is a small box and a long one opens at full height
       (owner, 2026-08-29: "有的框很大 有空白 应该自适应"). */
    min-height: 72px;
    resize: vertical;
    flex: none;
  }
  .d-body-edit:focus { outline: none; border-color: var(--accent); }
  /* Locked title/body (board #43): the same type as their editable twins,
     but PROSE — no surface, no border, no focus ring, nothing that invites
     a tap. Explicitly selectable: the app shell's global user-select:none
     (app.css) only opts inputs back in, and these are no longer inputs. */
  .d-title-static {
    color: var(--text); font-size: var(--fs-ui); font-weight: 600;
    padding: 7px 0;
    user-select: text; -webkit-user-select: text; cursor: text;
  }
  .d-body-static {
    color: var(--text); font-size: var(--fs-ui);
    white-space: pre-wrap; overflow-wrap: anywhere;
    padding: 2px 0;
    user-select: text; -webkit-user-select: text; cursor: text;
  }
  /* Timeline notes (reopened #11): author + right-aligned time on the header
     line, the content in its own box below — ragged author widths no longer
     push the text around, and the inks follow the app's hierarchy (accent
     name / grey time / full-ink content). */
  .notes { display: flex; flex-direction: column; gap: 8px; }
  .note { display: flex; flex-direction: column; gap: 3px; min-width: 0; }
  .n-head { display: flex; align-items: baseline; gap: 8px; }
  .n-author { color: var(--accent); font-weight: 650; font-size: var(--fs-meta); }
  /* A real <button> since board #46 (the accessible route to Copy — the note
     text stays text), still dressed as the quiet time it always was. */
  .n-at {
    color: var(--text3); font-size: var(--fs-micro); margin-left: auto; flex: none;
    background: none; border: none; padding: 0; font-family: inherit; line-height: 1; cursor: pointer;
  }
  /* The overlay's anchor (board #46): relative + fit-content, so .m-acts
     sits on the BUBBLE's corner — anchored to the full-width row it would
     float at the far right, nowhere near a short note. */
  .n-wrap { position: relative; width: fit-content; max-width: 100%; }
  .n-text {
    color: var(--text); font-size: var(--fs-ui);
    white-space: pre-wrap; overflow-wrap: anywhere;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--ui-radius-row);
    padding: 7px 10px;
    /* Every historical note is a document to READ (board #43: "所有的历史
       消息都可以手动滑动选择"): explicit opt-out of the shell's global
       user-select:none, with the text cursor saying so. */
    user-select: text; -webkit-user-select: text; cursor: text;
    /* The box HUGS its content — a one-word note is a small chip, not a
       full-width band of blank (owner, 2026-08-29). */
    width: fit-content;
    max-width: 100%;
    box-sizing: border-box;
  }
  /* The send button rides the LAST line as the box grows (board #28) —
     flex-end, not center: centered, a five-line note strands the button in
     the middle of the text block. At one line the two are the same place. */
  .note-add { display: flex; gap: 6px; align-items: flex-end; }
  /* The create form's title: a textarea so it can WRAP as it grows (autoGrow),
     but dressed exactly like the input it replaces — one line at rest, no
     manual resize handle, and no flex stretch in the column layout. */
  .n-title.one-line { flex: none; resize: none; overflow: hidden; }
  /* The note reply is the same species (board #28): soft wrap in the visible
     width, autoGrow raises it, no scrollbar flash while it measures. */
  .note-input { resize: none; overflow: hidden; }
  /* The create form's body: takes the height the column has left and scrolls
     INSIDE itself (owner, 2026-08-29: "下边区域大一点 上下撑满 可以内部滚动"). */
  .d-body-edit.fill { flex: 1; min-height: 140px; overflow-y: auto; resize: none; }
  .note-input, .n-title {
    flex: 1;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: var(--ui-radius-control);
    color: var(--text);
    font-size: var(--fs-ui);
    padding: 7px 10px;
    font-family: inherit;
  }
  .note-input:focus, .n-title:focus { outline: none; border-color: var(--accent); }
</style>
