import test from 'node:test';
import assert from 'node:assert/strict';

test('note-acts gen: a stale Copy timeout can never touch a later context (board #46 review)', async () => {
  const { MESSAGE_ACTS_IDLE, messageActsSet, messageActsCopyLanded, messageActsExpired } = await import('./message-actions.ts');
  // The REAL copy order: the attempt's gen is captured at the TAP (before
  // the clipboard await), the stamp lands only if the state is still that
  // context, and the dismiss timer is armed only on a landing.

  // Blocker 1 replayed: Copy on A lands, switch to B and Copy there within
  // the beat — A's timeout fires holding ITS gen and must be a no-op, B's
  // own timeout puts the row away.
  let s = messageActsSet(MESSAGE_ACTS_IDLE, 0);   // open A
  let attempt = s.gen;                       // A taps Copy
  s = messageActsCopyLanded(s, attempt);        // clipboard resolves in place
  const timerA = s.gen;
  s = messageActsSet(s, 1);                     // switch to B (within 1.5s)
  attempt = s.gen;                           // B taps Copy
  s = messageActsCopyLanded(s, attempt);        // lands
  const timerB = s.gen;
  const afterStaleA = messageActsExpired(s, timerA);
  assert.equal(afterStaleA, s, 'A\u2019s stale timeout changes NOTHING — B keeps its row and its Copied beat');
  assert.deepEqual(messageActsExpired(afterStaleA, timerB), { open: -1, copied: false, gen: timerB }, 'B\u2019s own timeout closes B');

  // Blocker 2 replayed, real await order: A taps Copy, the clipboard promise
  // is STILL PENDING while the user switches issue and opens note B — then
  // A's resolve arrives. It must not stamp Copied on B (nor arm a timer:
  // the same-reference return is the caller's no-arm signal).
  let c = messageActsSet(MESSAGE_ACTS_IDLE, 0);    // open A
  const pendingAttempt = c.gen;              // A taps Copy — promise pending
  c = messageActsSet(c, -1);                    // issue switched: context reset
  c = messageActsSet(c, 0);                     // user opens B's row in the new issue
  const resolved = messageActsCopyLanded(c, pendingAttempt);
  assert.equal(resolved, c, 'the deferred resolve is orphaned — B is never marked Copied by A\u2019s copy');
  assert.equal(resolved.copied, false, 'and no Copied beat begins');

  // The undisturbed happy path still lands and expires; gen only moves FORWARD.
  let h = messageActsSet(MESSAGE_ACTS_IDLE, 3);
  const hAttempt = h.gen;
  h = messageActsCopyLanded(h, hAttempt);
  assert.equal(h.copied, true, 'an in-place copy lands');
  assert.equal(messageActsExpired(h, h.gen).open, -1, 'an undisturbed beat closes its own row');
  assert.ok(messageActsSet(h, -1).gen > h.gen, 'every transition bumps the gen — monotonic, never reused');
});

test('message keys and copy failures use the same attempt identity across A-B-A (#167)', async () => {
  const { MESSAGE_ACTS_IDLE, messageActsSet, messageActsCopyLanded, messageActsCopyFailed, messageActsExpired } = await import('./message-actions.ts');
  let state = messageActsSet(MESSAGE_ACTS_IDLE, 'message-a');
  const old = state.gen;
  state = messageActsSet(state, 'message-b');
  state = messageActsSet(state, 'message-a');
  assert.equal(messageActsCopyLanded(state, old), state);
  assert.equal(messageActsCopyFailed(state, old, 'stale'), state);
  state = messageActsCopyFailed(state, state.gen, 'Copy failed');
  assert.equal(state.open, 'message-a');
  assert.equal(state.copied, false);
  assert.equal(state.error, 'Copy failed');
  assert.equal(messageActsExpired(state, old), state);
  state = messageActsSet(state, 'message-a');
  assert.equal(state.error, undefined, 'retry begins a new attempt');
  state = messageActsCopyLanded(state, state.gen);
  assert.equal(state.copied, true);
  assert.equal(state.error, undefined);
});
