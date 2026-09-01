import test from 'node:test';
import assert from 'node:assert/strict';
import { draftOf, draftDirty, draftValid, draftPatch, rebaseDraft, issueRef, ISSUE_REF_CHARS } from './board.ts';

// Since board #15 a draft carries all four editable fields; these helpers
// build the full shape from the short form the assertions speak.
const d = (over: Partial<{ title: string; body: string; status: string; assignee: string }>) => ({
  title: '', body: '', status: '', assignee: '', ...over,
});

test('an issue draft saves explicitly, cancels cleanly, and patches only what changed (board #11)', () => {
  const issue = { title: 'fix login', body: 'step 2 breaks' };
  const clean = draftOf(issue);
  assert.deepEqual(clean, d({ title: 'fix login', body: 'step 2 breaks' }));
  assert.ok(!draftDirty(clean, issue), 'a fresh draft is not dirty');
  assert.equal(draftPatch(clean, issue), null, 'nothing changed, nothing to save');

  // Editing one field patches ONE field — the untouched body must not ride
  // along and overwrite a concurrent edit (the server COALESCEs).
  const retitled = { ...clean, title: 'fix login flow' };
  assert.ok(draftDirty(retitled, issue));
  assert.deepEqual(draftPatch(retitled, issue), { title: 'fix login flow' });

  const rebodied = { ...clean, body: 'root cause: session cookie' };
  assert.deepEqual(draftPatch(rebodied, issue), { body: 'root cause: session cookie' });

  // Both changed → both sent; the title is trimmed on the way out.
  const both = d({ title: '  new title ', body: 'b' });
  assert.deepEqual(draftPatch(both, issue), { title: 'new title', body: 'b' });

  // A blank title is FINE while a body holds (board #31: title optional) —
  // the patch clears it verbatim; blanking BOTH is contentless and invalid.
  const blank = { ...clean, title: '   ' };
  assert.ok(draftValid(blank), 'body still says something');
  assert.deepEqual(draftPatch(blank, issue), { title: '' });
  const contentless = { ...clean, title: '   ', body: ' ' };
  assert.ok(!draftValid(contentless));
  assert.equal(draftPatch(contentless, issue), null);

  // Cancel = draftOf(sel) again: identity with the stored issue.
  assert.ok(!draftDirty(draftOf(issue), issue));

  // Whitespace typed IS an edit until removed (dirty), but body-only spaces
  // still patch verbatim — the body is prose, not an id.
  const spaced = { ...clean, body: 'step 2 breaks ' };
  assert.ok(draftDirty(spaced, issue));
  assert.deepEqual(draftPatch(spaced, issue), { body: 'step 2 breaks ' });
});

test('status and assignee are DRAFT fields too — a stray tap changes nothing until ✓ (board #15)', () => {
  const issue = { title: 'T', body: 'B', status: 'todo', assignee: '' };
  const clean = draftOf(issue);
  assert.deepEqual(clean, d({ title: 'T', body: 'B', status: 'todo' }));
  assert.ok(!draftDirty(clean, issue));

  // Sliding the status marks the draft dirty and patches ONE field.
  const moved = { ...clean, status: 'doing' };
  assert.ok(draftDirty(moved, issue), 'a status change is an edit awaiting the ✓');
  assert.deepEqual(draftPatch(moved, issue), { status: 'doing' });

  // Picking an assignee likewise — the dispatch happens at SAVE time.
  const assigned = { ...clean, assignee: 'builder' };
  assert.deepEqual(draftPatch(assigned, issue), { assignee: 'builder' });

  // Cancel restores both: identity again.
  assert.ok(!draftDirty(draftOf(issue), issue));

  // The rebase covers the new fields: an agent's concurrent take (status +
  // assignee moved server-side) flows into an untouched draft…
  const server = d({ title: 'T', body: 'B', status: 'doing', assignee: 'builder-2' });
  const flowed = rebaseDraft(clean, clean, server);
  assert.deepEqual(flowed.draft, server);
  assert.equal(draftPatch(flowed.draft, flowed.base), null);

  // …while the user's own pending status keeps priority on a clash, and the
  // patch declares exactly that field.
  const clash = rebaseDraft({ ...clean, status: 'review' }, clean, server);
  assert.equal(clash.draft.status, 'review', 'the touched status keeps the user\u2019s pick');
  assert.equal(clash.draft.assignee, 'builder-2', 'the untouched assignee follows the server');
  assert.deepEqual(draftPatch(clash.draft, clash.base), { status: 'review' });
});

test('rebase: the agent\u2019s concurrent edit survives a title-only save (#11 review)', () => {
  // The lead's exact scenario: user opens {A, B}, edits ONLY the title;
  // an agent writes a new body B' server-side; a refetch (after a move or
  // note) brings it in.
  const base = d({ title: 'A', body: 'B' });
  const draft = d({ title: 'A2', body: 'B' });
  const server = d({ title: 'A', body: 'B-agent' });
  const r = rebaseDraft(draft, base, server);
  assert.equal(r.draft.body, 'B-agent', 'the untouched body follows the server');
  assert.equal(r.draft.title, 'A2', 'the touched title keeps the user\u2019s edit');
  assert.deepEqual(r.base, server, 'the base catches up to the server');
  // The save then ships ONLY the touched field — the agent's body is safe.
  assert.deepEqual(draftPatch(r.draft, r.base), { title: 'A2' });

  // Both sides touched the same field: the user's copy wins locally and the
  // patch declares it — overwriting is now a choice, not an accident.
  const clash = rebaseDraft(d({ title: 'A2', body: 'B' }), base, d({ title: 'A3', body: 'B' }));
  assert.equal(clash.draft.title, 'A2');
  assert.deepEqual(draftPatch(clash.draft, clash.base), { title: 'A2' });

  // Nothing touched: draft and base both become the server copy — clean.
  const clean = rebaseDraft(base, base, server);
  assert.deepEqual(clean.draft, server);
  assert.equal(draftPatch(clean.draft, clean.base), null);

  // After a successful save the server carries the saved text: the rebase
  // normalizes the draft to clean without an explicit reset.
  const saved = rebaseDraft(d({ title: 'A2', body: 'B' }), base, d({ title: 'A2', body: 'B' }));
  assert.equal(draftPatch(saved.draft, saved.base), null);
});

test('issueRef: one fallback names every titleless issue (board #31)', () => {
  // A real title wins, trimmed.
  assert.equal(issueRef({ id: 7, title: '  Fix login  ', body: 'whatever' }), 'Fix login');
  // No title → the body, whitespace squashed to one line.
  assert.equal(issueRef({ id: 7, title: '', body: 'the flow\n  breaks   at step 2' }), 'the flow breaks at step 2');
  // Long bodies cut on a CODE POINT boundary with the … marker (no split
  // surrogate: 𝓍 is astral).
  const long = '𝓍标题可空'.repeat(20);
  const r = issueRef({ id: 7, title: ' ', body: long });
  assert.equal([...r].length, ISSUE_REF_CHARS + 1, 'cut + one … char');
  assert.ok(r.endsWith('…'));
  assert.ok(r.startsWith('𝓍标题可空'));
  // Exactly at the budget: complete, no marker.
  const exact = 'x'.repeat(ISSUE_REF_CHARS);
  assert.equal(issueRef({ id: 7, body: exact }), exact);
  // Legacy all-empty → the id; missing fields never throw.
  assert.equal(issueRef({ id: 7, title: '', body: '   ' }), '#7');
  assert.equal(issueRef(null), '#0');
});

// ── Sidebar counts (board #39) ──────────────────────────────────────────────

test('countsOf speaks the server dialect: zero-filled vocabulary, explicit total, null for empty (board #39)', async () => {
  const { countsOf, BOARD_STATUSES } = await import('./board.ts');
  // One todo, two done: every OTHER status is PRESENT as 0 — the client
  // never guesses the vocabulary — and total is a field, not a sum.
  const c = countsOf([{ status: 'todo' }, { status: 'done' }, { status: 'done' }]);
  assert.deepEqual(c, { todo: 1, doing: 0, review: 0, done: 2, total: 3 });
  for (const s of BOARD_STATUSES) assert.ok(c && typeof c[s] === 'number', `${s} always present`);
  // EMPTY is null, never an all-zeros row — absence is the hide signal,
  // mirroring hub_board_counts (a zero row would make emptiness two checks).
  assert.equal(countsOf([]), null);
  // A foreign status counts toward total only, exactly like the server.
  assert.deepEqual(countsOf([{ status: 'weird' }]), { todo: 0, doing: 0, review: 0, done: 0, total: 1 });
});

test('applyCounts makes create/delete IMMEDIATE: first issue appears, last issue removes the key (board #39)', async () => {
  const { applyCounts } = await import('./board.ts');
  const before = { other: { todo: 1, doing: 0, review: 0, done: 0, total: 1 } };
  // Creating the FIRST issue: the session's key appears at once — the
  // sidebar must not wait out the 20 s poll.
  const created = applyCounts(before, 'mine', [{ status: 'todo' }]);
  assert.deepEqual(created['mine'], { todo: 1, doing: 0, review: 0, done: 0, total: 1 });
  assert.deepEqual(created['other'], before['other'], 'other boards untouched');
  // Deleting the LAST issue: the key is REMOVED (absence = hide), never
  // left as zeros.
  const gone = applyCounts(created, 'mine', []);
  assert.ok(!('mine' in gone), 'empty board leaves the map');
  assert.ok('other' in gone, 'other boards survive');
  // Immutable: $state consumers see a NEW map, the old one is unchanged.
  assert.ok(!('mine' in before), 'input map never mutated');
  assert.notEqual(created, before);
});

test('visibleBoards hides empty boards and keeps the given order (board #39)', async () => {
  const { visibleBoards } = await import('./board.ts');
  const rows = [
    { project: { session: 'a', name: 'A' } },
    { project: { session: 'b', name: 'B' } },
    { project: { session: 'c', name: 'C' } },
  ];
  const counts = {
    a: { todo: 0, doing: 0, review: 0, done: 1, total: 1 },
    c: { todo: 2, doing: 0, review: 0, done: 0, total: 2 },
  };
  // b has NO key (empty board) → hidden; order of the rest is the caller's
  // (sortRows already ordered by conversation).
  assert.deepEqual(visibleBoards(rows, counts).map((r) => r.project.session), ['a', 'c']);
  // A defensive zero row hides too: the check is total>0, not key-exists.
  assert.deepEqual(visibleBoards(rows, { b: { todo: 0, doing: 0, review: 0, done: 0, total: 0 } }), []);
  assert.deepEqual(visibleBoards([], counts), []);
});

test('boardTitle answers from the FULL list — a hidden empty board still has its name (board #39)', async () => {
  const { boardTitle } = await import('./board.ts');
  const all = [
    { project: { session: 'shown', name: 'Shown' } },
    { project: { session: 'empty', name: 'Empty but named' } },
  ];
  // The current board is empty and filtered OUT of the sidebar, yet the
  // page-head names it — that is where the first issue gets created.
  assert.equal(boardTitle(all, 'empty'), 'Empty but named');
  assert.equal(boardTitle(all, 'shown'), 'Shown');
  // Unknown session → null, and the component falls back (session string,
  // then the generic page name) exactly like before.
  assert.equal(boardTitle(all, 'gone'), null);
  assert.equal(boardTitle([], 'x'), null);
});
